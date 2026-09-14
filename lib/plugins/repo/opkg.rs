// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{bail, Context, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::{IndexOptions, RepoIndexer};

/// OpenWrt/opkg repository: a `Packages` index plus `Packages.gz`, built
/// from each `.ipk`'s control file (same RFC-2822 field syntax as `.deb`).
pub struct OpkgIndexer;

impl RepoIndexer for OpkgIndexer {
    fn name(&self) -> &'static str {
        "opkg"
    }

    fn format(&self) -> &'static str {
        "ipk"
    }

    fn description(&self) -> &'static str {
        "opkg repository (Packages, Packages.gz)"
    }

    fn file_extension(&self) -> &'static str {
        "ipk"
    }

    fn build_index(&self, dir: &Path, artifacts: &[PathBuf], _opts: &IndexOptions) -> Result<()> {
        if artifacts.is_empty() {
            bail!("no .ipk files in '{}'", dir.display());
        }
        let mut stanzas = Vec::new();
        for ipk in artifacts {
            let ctrl = read_ipk_control(ipk)?;
            let get = |k: &str| ctrl.get(k).cloned().unwrap_or_default();
            let name = ipk.file_name().unwrap().to_string_lossy().to_string();
            let size = std::fs::metadata(ipk)?.len();
            let sha256 = lx_lib::checksum::sha256_file(ipk)?;
            let stanza = format!(
                "Package: {}\nVersion: {}\nArchitecture: {}\nMaintainer: {}\nDepends: {}\nProvides: {}\nSection: {}\nDescription: {}\nFilename: {}\nSize: {}\nSHA256: {}\n",
                get("Package"),
                get("Version"),
                get("Architecture"),
                get("Maintainer"),
                get("Depends"),
                get("Provides"),
                get("Section"),
                get("Description"),
                name,
                size,
                sha256,
            );
            stanzas.push(stanza.trim_end_matches('\n').to_string());
        }
        let packages = stanzas.join("\n\n") + "\n";
        std::fs::write(dir.join("Packages"), &packages)?;
        let gz = lx_lib::debarchive::deterministic_gzip_bytes(packages.as_bytes(), 0, 9)?;
        std::fs::write(dir.join("Packages.gz"), &gz)?;
        println!("wrote Packages, Packages.gz ({} packages)", artifacts.len());
        Ok(())
    }
}

/// Read the `control` file out of an `.ipk`'s `control.tar.gz` member.
pub fn read_ipk_control(ipk: &Path) -> Result<BTreeMap<String, String>> {
    let file = std::fs::File::open(ipk).with_context(|| format!("opening '{}'", ipk.display()))?;
    let mut archive = ar::Archive::new(file);
    while let Some(entry) = archive.next_entry() {
        let mut entry = entry?;
        let name = String::from_utf8_lossy(entry.header().identifier()).to_string();
        if !name.starts_with("control.tar") {
            continue;
        }
        let mut data = Vec::new();
        std::io::copy(&mut entry, &mut data)?;
        let text = if name.ends_with(".gz") {
            let mut out = Vec::new();
            std::io::copy(&mut flate2::read::GzDecoder::new(data.as_slice()), &mut out)?;
            out
        } else {
            data
        };
        let mut tar = tar::Archive::new(text.as_slice());
        for member in tar.entries()? {
            let mut member = member?;
            let path = member.path()?.to_string_lossy().to_string();
            if path.ends_with("control") && member.header().entry_type().is_file() {
                let mut s = String::new();
                std::io::Read::read_to_string(&mut member, &mut s)?;
                return Ok(crate::repo::parse_control(&s));
            }
        }
        bail!("control.tar has no control file in '{}'", ipk.display());
    }
    bail!("'{}' has no control.tar.* member", ipk.display())
}
