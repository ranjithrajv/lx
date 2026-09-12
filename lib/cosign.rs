// SPDX-License-Identifier: GPL-3.0-or-later

//! Cosign keyless signing for supply-chain security.
//!
//! Cosign (Sigstore) provides keyless signing using OIDC identities and
//! transparency logs. This module wraps the `cosign sign-blob` command to
//! produce Sigstore signatures alongside built packages.
//!
//! Unlike GPG signing (which requires key management), cosign uses ephemeral
//! keys signed by an OIDC token (e.g., GitHub Actions' OIDC). Signatures are
//! recorded in the Rekor transparency log for auditability.

use anyhow::{bail, Context, Result};
use std::path::Path;

/// Sign a file with cosign keyless signing (Sigstore).
///
/// Requires:
/// - `cosign` binary on PATH
/// - OIDC token available (e.g., in GitHub Actions via `id-token: write`)
///
/// Produces `<file>.sig` (signature) and `<file>.certificate` (certificate).
pub fn cosign_sign_blob(path: &Path) -> Result<Vec<PathBuf>> {
    let path_str = path.to_string_lossy();
    let mut generated = Vec::new();

    // Check cosign is available.
    require_cosign()?;

    // Sign the file. cosign sign-blob produces a .sig file.
    let sig_path = path.with_extension(format!(
        "{}.sig",
        path.extension().unwrap_or_default().to_string_lossy()
    ));
    let sig_str = sig_path.to_string_lossy().to_string();

    let output = std::process::Command::new("cosign")
        .args([
            "sign-blob",
            "--yes",
            "--output-signature",
            &sig_str,
            &path_str,
        ])
        .output()
        .context("failed to run `cosign sign-blob`")?;

    if !output.status.success() {
        bail!(
            "cosign sign-blob failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    generated.push(sig_path);

    println!("  ✓ cosign: signed {}", path.display());
    Ok(generated)
}

/// Verify a cosign signature.
pub fn cosign_verify_blob(path: &Path, sig_path: &Path) -> Result<bool> {
    require_cosign()?;

    let output = std::process::Command::new("cosign")
        .args([
            "verify-blob",
            "--signature",
            &sig_path.to_string_lossy(),
            &path.to_string_lossy(),
        ])
        .output()
        .context("failed to run `cosign verify-blob`")?;

    Ok(output.status.success())
}

/// True if cosign is available on PATH.
pub fn cosign_available() -> bool {
    std::process::Command::new("cosign")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn require_cosign() -> Result<()> {
    if !cosign_available() {
        bail!("cosign not found on PATH (install from https://sigstore.dev)");
    }
    Ok(())
}

use std::path::PathBuf;
