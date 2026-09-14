// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

use super::{parse_key_value, IndexOptions, RepoIndexer};

/// Arch Linux pacman repository: a `<repo>.db.tar.gz` database whose members
/// are `<pkgname>-<pkgver>/desc` files, built from each package's `.PKGINFO`.
pub struct PacmanIndexer;

impl RepoIndexer for PacmanIndexer {
    fn name(&self) -> &'static str {
        "pacman"
    }

    fn format(&self) -> &'static str {
        "arch"
    }

    fn description(&self) -> &'static str {
        "pacman repository (<repo>.db.tar.gz from .PKGINFO)"
    }

    fn file_extension(&self) -> &'static str {
        "zst"
    }

    fn build_index(&self, dir: &Path, artifacts: &[PathBuf], opts: &IndexOptions) -> Result<()> {
        if artifacts.is_empty() {
            bail!("no .pkg.tar.zst files in '{}'", dir.display());
        }
        let mut tar_bytes = Vec::new();
        let mut count = 0;
        {
            let mut builder = tar::Builder::new(&mut tar_bytes);
            for pkg in artifacts {
                let (single, depends) = read_pkginfo(pkg)?;
                let name = single.get("pkgname").cloned().unwrap_or_default();
                let version = single.get("pkgver").cloned().unwrap_or_default();
                let arch = single.get("arch").cloned().unwrap_or_default();
                let fname = pkg.file_name().unwrap().to_string_lossy().to_string();
                let size = std::fs::metadata(pkg)?.len();
                let sha256 = lx_lib::checksum::sha256_file(pkg)?;
                let installed = single.get("size").cloned().unwrap_or_else(|| "0".into());
                let desc = single.get("pkgdesc").cloned().unwrap_or_default();
                let license = single.get("license").cloned().unwrap_or_default();

                let mut body = String::new();
                body.push_str(&format!("%FILENAME%\n{fname}\n\n"));
                body.push_str(&format!("%NAME%\n{name}\n\n"));
                body.push_str(&format!("%BASE%\n{name}\n\n"));
                body.push_str(&format!("%VERSION%\n{version}\n\n"));
                body.push_str(&format!("%DESC%\n{desc}\n\n"));
                body.push_str(&format!("%CSIZE%\n{size}\n\n"));
                body.push_str(&format!("%ISIZE%\n{installed}\n\n"));
                body.push_str(&format!("%SHA256SUM%\n{sha256}\n\n"));
                body.push_str(&format!("%ARCH%\n{arch}\n\n"));
                if !license.is_empty() {
                    body.push_str(&format!("%LICENSE%\n{license}\n\n"));
                }
                if !depends.is_empty() {
                    body.push_str("%DEPENDS%\n");
                    for d in &depends {
                        body.push_str(&format!("{d}\n"));
                    }
                    body.push('\n');
                }
                let member = format!("{name}-{version}/desc");
                let mut header = tar::Header::new_gnu();
                header.set_size(body.len() as u64);
                header.set_mode(0o644);
                header.set_mtime(0);
                header.set_cksum();
                builder.append_data(&mut header, &member, body.as_bytes())?;
                count += 1;
            }
            builder.finish()?;
        }
        let db = dir.join(format!("{}.db.tar.gz", opts.suite));
        let gz = lx_lib::debarchive::deterministic_gzip_bytes(&tar_bytes, 0, 9)?;
        std::fs::write(&db, &gz)?;
        println!("wrote {} ({count} packages)", db.display());
        Ok(())
    }

    fn sign_index(&self, dir: &Path, opts: &IndexOptions) -> Result<()> {
        let Some(key) = opts.sign_key else {
            return Ok(());
        };
        // pacman verifies a detached signature over the database tarball,
        // `<repo>.db.tar.gz.sig`.
        let db = dir.join(format!("{}.db.tar.gz", opts.suite));
        let req = lx_lib::sign::SignRequest {
            key_file: key,
            key_id: opts.sign_key_id,
            passphrase: None,
        };
        let sig = lx_lib::sign::gpg_detach_sign(&db, &req)?;
        println!(
            "wrote {} (detached)",
            sig.file_name().unwrap_or_default().to_string_lossy()
        );
        Ok(())
    }
}

/// Read `.PKGINFO` from a `.pkg.tar.zst`, returning the single-valued fields
/// plus the repeated `depend` list.
fn read_pkginfo(pkg: &Path) -> Result<(std::collections::BTreeMap<String, String>, Vec<String>)> {
    let f = std::fs::File::open(pkg).with_context(|| format!("opening '{}'", pkg.display()))?;
    let dec = zstd::stream::read::Decoder::new(f)?;
    let mut tar = tar::Archive::new(dec);
    for member in tar.entries()? {
        let mut member = member?;
        let path = member.path()?.to_string_lossy().to_string();
        if path != ".PKGINFO" {
            continue;
        }
        let mut text = String::new();
        std::io::Read::read_to_string(&mut member, &mut text)?;
        let single = parse_key_value(&text);
        let depends = text
            .lines()
            .filter_map(|l| l.trim().strip_prefix("depend = ").map(str::to_string))
            .collect();
        return Ok((single, depends));
    }
    bail!("'{}' has no .PKGINFO member", pkg.display())
}
