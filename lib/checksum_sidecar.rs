// SPDX-License-Identifier: GPL-3.0-or-later

//! Checksum sidecar generation for built packages.
//!
//! After building packages, generates `.sha256` and `.sha512` checksum files
//! alongside each artifact. These enable integrity verification by consumers
//! and match goreleaser/cargo-dist conventions.

use anyhow::{Context, Result};
use std::path::Path;

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

        // SHA-256.
        let sha256 = lx_lib::checksum::sha256_file(&path)
            .with_context(|| format!("checksum for '{}'", path.display()))?;
        let sha256_file = dir.join(format!("{name}.sha256"));
        std::fs::write(&sha256_file, &sha256)
            .with_context(|| format!("writing '{}'", sha256_file.display()))?;
        generated.push(sha256_file.to_string_lossy().to_string());

        // SHA-512.
        let sha512 = lx_lib::checksum::sha512_file(&path)
            .with_context(|| format!("checksum for '{}'", path.display()))?;
        let sha512_file = dir.join(format!("{name}.sha512"));
        std::fs::write(&sha512_file, &sha512)
            .with_context(|| format!("writing '{}'", sha512_file.display()))?;
        generated.push(sha512_file.to_string_lossy().to_string());
    }

    if !generated.is_empty() {
        println!("  ✓ generated {} checksum sidecar(s)", generated.len());
    }
    Ok(generated)
}

/// True if `name` looks like a package archive file.
pub fn is_package_file(name: &str) -> bool {
    name.ends_with(".deb")
        || name.ends_with(".rpm")
        || name.ends_with(".pkg.tar.zst")
        || name.ends_with(".pkg.tar.xz")
        || name.ends_with(".tar.gz")
        || name.ends_with(".zip")
}
