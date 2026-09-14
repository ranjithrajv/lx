// SPDX-License-Identifier: GPL-3.0-or-later

//! Checksum sidecar generation for built packages.
//!
//! After building packages, generates `.sha256` and `.sha512` checksum files
//! alongside each artifact. These enable integrity verification by consumers
//! and match goreleaser/cargo-dist conventions.

use anyhow::{Context, Result};
use std::path::Path;

/// A `path -> hex digest` checksum function.
type FileHasher = fn(&Path) -> Result<String>;

/// Generate checksum sidecars for all package files in `dir`.
///
/// Creates `<file>.sha256` and `<file>.sha512` for each `.deb`, `.rpm`,
/// `.pkg.tar.zst`, and `.pkg.tar.xz` file found.
pub fn generate_checksum_sidecars(dir: &Path) -> Result<Vec<String>> {
    let mut generated = Vec::new();

    for entry in
        std::fs::read_dir(dir).with_context(|| format!("reading dir '{}'", dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.is_empty() {
            continue;
        }
        // Skip existing checksum files.
        if name.ends_with(".sha256") || name.ends_with(".sha512") {
            continue;
        }
        // Only checksum package files.
        if !is_package_file(name) {
            continue;
        }

        // One loop for every algorithm, so adding SHA-1/others is one entry.
        let algorithms: [(&str, FileHasher); 2] = [
            (".sha256", lx_lib::checksum::sha256_file),
            (".sha512", lx_lib::checksum::sha512_file),
        ];
        for (suffix, hash_file) in algorithms {
            let digest =
                hash_file(&path).with_context(|| format!("checksum for '{}'", path.display()))?;
            let out = dir.join(format!("{name}{suffix}"));
            std::fs::write(&out, &digest)
                .with_context(|| format!("writing '{}'", out.display()))?;
            generated.push(out.to_string_lossy().to_string());
        }
    }

    if !generated.is_empty() {
        println!("  ✓ generated {} checksum sidecar(s)", generated.len());
    }
    Ok(generated)
}

/// True if `name` is a built package archive lx can checksum and sign.
///
/// Matches the formats the packagers emit, so `.msix`/`.pkg` get sidecars and
/// signatures too; generic archives (`.tar.gz`, `.zip`) are not packages and
/// are excluded even though they may sit in the output directory.
pub fn is_package_file(name: &str) -> bool {
    name.ends_with(".deb")
        || name.ends_with(".rpm")
        || name.ends_with(".apk")
        || name.ends_with(".ipk")
        || name.ends_with(".pkg.tar.zst")
        || name.ends_with(".pkg.tar.xz")
        || name.ends_with(".msix")
        || name.ends_with(".pkg")
}
