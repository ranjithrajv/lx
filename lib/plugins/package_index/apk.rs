// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

use super::{parse_key_value, Capabilities, IndexOptions, PackageIndex};

/// Alpine apk repository: an `APKINDEX.tar.gz` archive containing the
/// `APKINDEX` text plus a `DESCRIPTION`, built from each `.apk`'s `.PKGINFO`.
pub struct ApkIndexer;

impl PackageIndex for ApkIndexer {
    fn id(&self) -> &'static str {
        "apk"
    }

    fn description(&self) -> &'static str {
        "Alpine repository (APKINDEX.tar.gz from .PKGINFO)"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::WRITE
    }

    fn file_extension(&self) -> Option<&'static str> {
        Some("apk")
    }

    fn build_index(&self, dir: &Path, artifacts: &[PathBuf], opts: &IndexOptions) -> Result<()> {
        if artifacts.is_empty() {
            bail!("no .apk files in '{}'", dir.display());
        }
        let mut index = String::new();
        for apk in artifacts {
            let pkginfo = read_apk_pkginfo(apk)?;
            let get = |k: &str| pkginfo.single.get(k).cloned().unwrap_or_default();
            let name = get("pkgname");
            let version = get("pkgver");
            let arch = get("arch");
            let size = std::fs::metadata(apk)?.len();
            // Best-effort package checksum (sha1 of the artifact, base64),
            // `Q1` marks the APKv2 sha1 format.
            let csum = apk_checksum(apk)?;
            index.push_str(&format!("C:{csum}\n"));
            index.push_str(&format!("P:{name}\n"));
            index.push_str(&format!("V:{version}\n"));
            index.push_str(&format!("A:{arch}\n"));
            index.push_str(&format!("S:{size}\n"));
            index.push_str(&format!("I:{}\n", get("size")));
            index.push_str(&format!("T:{}\n", get("pkgdesc")));
            index.push_str(&format!("U:{}\n", get("url")));
            index.push_str(&format!("L:{}\n", get("license")));
            index.push_str(&format!("o:{}\n", get("origin")));
            if !pkginfo.depends.is_empty() {
                index.push_str(&format!("D:{}\n", pkginfo.depends.join(" ")));
            }
            if !pkginfo.provides.is_empty() {
                index.push_str(&format!("p:{}\n", pkginfo.provides.join(" ")));
            }
            index.push('\n');
        }

        let mut tar_bytes = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_bytes);
            append(&mut builder, "APKINDEX", index.as_bytes())?;
            append(
                &mut builder,
                "DESCRIPTION",
                format!("{}\n", opts.origin).as_bytes(),
            )?;
            builder.finish()?;
        }
        let out = dir.join("APKINDEX.tar.gz");
        let gz = lx_lib::debarchive::deterministic_gzip_bytes(&tar_bytes, 0, 9)?;
        std::fs::write(&out, &gz)?;
        println!("wrote {} ({} packages)", out.display(), artifacts.len());
        Ok(())
    }
}

fn append<W: std::io::Write>(
    builder: &mut tar::Builder<W>,
    name: &str,
    content: &[u8],
) -> Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_size(content.len() as u64);
    header.set_mode(0o644);
    header.set_mtime(0);
    header.set_cksum();
    builder.append_data(&mut header, name, content)?;
    Ok(())
}

fn apk_checksum(apk: &Path) -> Result<String> {
    use base64::Engine as _;
    use sha1::Digest;
    let data = std::fs::read(apk)?;
    let mut h = sha1::Sha1::new();
    h.update(&data);
    Ok(format!(
        "Q1{}",
        base64::engine::general_purpose::STANDARD.encode(h.finalize())
    ))
}

struct ApkInfo {
    single: std::collections::BTreeMap<String, String>,
    depends: Vec<String>,
    provides: Vec<String>,
}

/// Read `.PKGINFO` from the first gzip member (the control tar) of an `.apk`.
fn read_apk_pkginfo(apk: &Path) -> Result<ApkInfo> {
    use std::io::Read;
    let data = std::fs::read(apk).with_context(|| format!("reading '{}'", apk.display()))?;
    let mut gz = flate2::read::GzDecoder::new(data.as_slice());
    let mut control = Vec::new();
    // GzDecoder stops after the first member, which is the control tar.
    gz.read_to_end(&mut control)
        .with_context(|| format!("decompressing control member of '{}'", apk.display()))?;
    let mut tar = tar::Archive::new(control.as_slice());
    for member in tar.entries()? {
        let mut member = member?;
        let path = member.path()?.to_string_lossy().to_string();
        if path != ".PKGINFO" {
            continue;
        }
        let mut text = String::new();
        member.read_to_string(&mut text)?;
        let single = parse_key_value(&text);
        let collect = |prefix: &str| -> Vec<String> {
            text.lines()
                .filter_map(|l| l.trim().strip_prefix(prefix).map(str::to_string))
                .collect()
        };
        let depends = collect("depend = ");
        let provides = collect("provides = ");
        return Ok(ApkInfo {
            single,
            depends,
            provides,
        });
    }
    bail!("'{}' has no .PKGINFO member", apk.display())
}
