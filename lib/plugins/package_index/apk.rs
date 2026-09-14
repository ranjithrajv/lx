// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

use super::{parse_key_value, IndexOptions, PackageIndex, WriteIndex};
use crate::plugins::plugin::plugin_identity;

/// Alpine apk repository: an `APKINDEX.tar.gz` archive containing the
/// `APKINDEX` text plus a `DESCRIPTION`, built from each `.apk`'s `.PKGINFO`.
pub struct ApkIndexer;

plugin_identity!(
    ApkIndexer,
    "apk",
    "Alpine repository (APKINDEX.tar.gz from .PKGINFO; RSA-signed index)"
);

impl PackageIndex for ApkIndexer {}

impl WriteIndex for ApkIndexer {
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
            // `C:` is the `Q1`-prefixed SHA-1 of the compressed control
            // segment (not the whole file), as apk computes it.
            let csum = lx_lib::apkarchive::control_checksum(apk)?;
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

    fn sign_index(&self, dir: &Path, opts: &IndexOptions) -> Result<()> {
        let Some(key) = opts.sign_key else {
            return Ok(());
        };
        // Alpine signs the whole `APKINDEX.tar.gz` (unlike a package, where
        // the control segment is signed) and prepends a
        // `.SIGN.RSA.<keyname>` segment.
        let index_path = dir.join("APKINDEX.tar.gz");
        let index = std::fs::read(&index_path)?;
        let sig = lx_lib::sign::rsa_sha1_sign(&index, key, None)?;
        let member = format!(
            ".SIGN.RSA.{}",
            lx_lib::sign::apk_key_name(key, opts.sign_key_id)
        );
        let segment = lx_lib::apkarchive::signature_segment(&member, &sig, 0)?;
        let mut out = Vec::with_capacity(segment.len() + index.len());
        out.extend_from_slice(&segment);
        out.extend_from_slice(&index);
        std::fs::write(&index_path, out)?;
        println!("wrote APKINDEX.tar.gz (signed, {member})");
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

struct ApkInfo {
    single: std::collections::BTreeMap<String, String>,
    depends: Vec<String>,
    provides: Vec<String>,
}

/// Read `.PKGINFO` from an `.apk`'s control segment. Handles signed
/// packages, whose first gzip member is the `.SIGN.*` segment.
fn read_apk_pkginfo(apk: &Path) -> Result<ApkInfo> {
    let text = lx_lib::apkarchive::read_pkginfo(apk)?;
    let single = parse_key_value(&text);
    let collect = |prefix: &str| -> Vec<String> {
        text.lines()
            .filter_map(|l| l.trim().strip_prefix(prefix).map(str::to_string))
            .collect()
    };
    Ok(ApkInfo {
        single,
        depends: collect("depend = "),
        provides: collect("provides = "),
    })
}
