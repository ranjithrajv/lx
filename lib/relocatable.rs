// SPDX-License-Identifier: GPL-3.0-or-later

//! Relocatable binaries support.
//!
//! Binaries that work from any install path (like cargo-dist). For ELF
//! binaries, this means using `$ORIGIN`-relative RPATH so shared libraries
//! are found relative to the binary location, not an absolute path.

use anyhow::{Context, Result};
use std::path::Path;

/// Make ELF binaries in `bin_dir` relocatable by setting RPATH to `$ORIGIN/../lib`.
///
/// This allows the binary to find its shared libraries regardless of where
/// it's installed. Uses `patchelf` if available, otherwise warns and skips.
pub fn make_relocatable(bin_dir: &Path) -> Result<usize> {
    let patchelf = find_patchelf();
    let mut patched = 0;

    for entry in
        std::fs::read_dir(bin_dir).with_context(|| format!("reading '{}'", bin_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if !entry.file_type()?.is_file() {
            continue;
        }
        if !is_elf(&path)? {
            continue;
        }
        if is_dynamic(&path) {
            if let Some(ref pl) = patchelf {
                if let Err(e) = set_rpath(pl, &path) {
                    eprintln!("  ⚠ patchelf failed for '{}': {}", path.display(), e);
                } else {
                    patched += 1;
                }
            }
        }
    }

    if patched > 0 {
        println!("  ✓ made {} binary(ies) relocatable", patched);
    }
    Ok(patched)
}

/// True if the file is an ELF binary.
fn is_elf(path: &Path) -> Result<bool> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut magic = [0u8; 4];
    if f.read_exact(&mut magic).is_err() {
        return Ok(false);
    }
    Ok(&magic == b"\x7fELF")
}

/// True if the ELF is dynamically linked (has INTERP segment).
fn is_dynamic(path: &Path) -> bool {
    // Quick check: does it have a PT_INTERP segment?
    // We check for the presence of "lib" in NEEDED entries.
    if let Ok(bytes) = std::fs::read(path) {
        // Check ELF header for e_type == ET_DYN (3) or ET_EXEC (2).
        if bytes.len() > 18 {
            let e_type = u16::from_le_bytes([bytes[16], bytes[17]]);
            return e_type == 2 || e_type == 3; // ET_EXEC or ET_DYN
        }
    }
    false
}

/// Find patchelf binary.
fn find_patchelf() -> Option<String> {
    for name in &["patchelf", "patchelf-stable"] {
        if std::process::Command::new(name)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return Some(name.to_string());
        }
    }
    None
}

/// Set RPATH to `$ORIGIN/../lib` on an ELF binary.
fn set_rpath(patchelf: &str, path: &Path) -> Result<()> {
    let status = std::process::Command::new(patchelf)
        .args(["--set-rpath", "$ORIGIN/../lib"])
        .arg(path)
        .status()
        .context("failed to run patchelf")?;
    if !status.success() {
        anyhow::bail!("patchelf exit code {}", status);
    }
    Ok(())
}
