//! Post-build artifact signing.
//!
//! Two mechanisms, one request type:
//!
//! * **`.deb`** — either a detached binary signature next to the artifact
//!   (`<file>.sig` via `gpg --detach-sign`), or an embedded `_gpgorigin`
//!   member inside the `.deb` ar (debsign / debsigs) via
//!   [`clearsign`]. There is no Rust equivalent worth pulling in for
//!   OpenPGP signing; `gpg` is on every CI runner that matters (same
//!   trade-off as `lintian`).
//! * **`.rpm`** — the `rpm` crate embeds the PGP signature in the package
//!   header natively; [`SignRequest`] just carries the key material there
//!   (see `lib/rpmarchive.rs`).

use anyhow::{bail, Context, Result};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Everything needed to sign one artifact.
#[derive(Debug, Clone)]
pub struct SignRequest<'a> {
    /// Path to an ASCII-armored secret key.
    pub key_file: &'a Path,
    /// Optional key id / fingerprint (gpg `--local-user`; ignored by the
    /// rpm crate).
    pub key_id: &'a str,
    /// Passphrase, if the key needs one. Resolved by callers from
    /// `$LPT_SIGN_PASSPHRASE` (falling back to `$NFPM_PASSPHRASE`).
    pub passphrase: Option<&'a str>,
}

impl<'a> SignRequest<'a> {
    /// Load the armored secret key bytes.
    pub fn key_bytes(&self) -> Result<Vec<u8>> {
        std::fs::read(self.key_file)
            .with_context(|| format!("failed to read signing key '{}'", self.key_file.display()))
    }
}

/// Detached-sign `artifact` with gpg, producing `<artifact>.sig`.
/// Returns the signature path. Fails loudly if `gpg` is missing — a
/// configured-but-failed sign must never yield an unsigned release.
pub fn gpg_detach_sign(artifact: &Path, req: &SignRequest) -> Result<PathBuf> {
    let sig_path = sig_path_for(artifact);
    let mut cmd = base_gpg_cmd(req)?;
    cmd.arg("--detach-sign")
        .arg("--output")
        .arg(&sig_path)
        .arg(artifact);
    let out = cmd.output().with_context(|| {
        "failed to run `gpg` (is it installed? apt-get install gnupg / dnf install gnupg2)"
    })?;
    if !out.status.success() {
        bail!(
            "gpg detach-sign failed for {}: {}",
            artifact.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(sig_path)
}

/// Armored-detach-sign `payload` bytes (stdin → stdout), returning the
/// signature. Used to produce the `_gpgorigin` ar member for
/// `signature.method: debsign` (debsigs / nfpm-compatible). Despite the
/// historical "clearsign" name, this is `gpg --armor --detach-sign`, not
/// `gpg --clearsign`: debsigs expects a detached signature over the
/// concatenated `debian-binary` + control + data members.
pub fn clearsign(payload: &[u8], req: &SignRequest) -> Result<Vec<u8>> {
    let mut cmd = base_gpg_cmd(req)?;
    cmd.arg("--armor")
        .arg("--detach-sign")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().with_context(|| {
        "failed to run `gpg` (is it installed? apt-get install gnupg / dnf install gnupg2)"
    })?;
    {
        let stdin = child.stdin.as_mut().expect("piped stdin");
        stdin
            .write_all(payload)
            .context("failed to write payload to gpg stdin")?;
    }
    let out = child
        .wait_with_output()
        .context("gpg armored detach-sign failed")?;
    if !out.status.success() {
        bail!(
            "gpg armored detach-sign failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    if out.stdout.is_empty() {
        bail!("gpg armored detach-sign produced empty signature");
    }
    Ok(out.stdout)
}

/// `<file>.deb` -> sibling `<file>.sig` (replacing any stale signature).
pub fn sig_path_for(artifact: &Path) -> PathBuf {
    let mut p = artifact.as_os_str().to_os_string();
    p.push(".sig");
    PathBuf::from(p)
}

/// Shared gpg invocation setup: `--batch --yes --local-user …`, optional
/// passphrase, and a hermetic temp GNUPGHOME when no key id is given.
fn base_gpg_cmd(req: &SignRequest) -> Result<Command> {
    let mut cmd = Command::new("gpg");
    cmd.arg("--batch").arg("--yes").arg("--local-user");
    // Prefer an explicit id; else point gpg at the provided secret key file
    // via a temporary GNUPGHOME so we don't depend on the caller's keyring.
    if !req.key_id.trim().is_empty() {
        cmd.arg(req.key_id.trim());
    } else {
        let tmp_home = import_key_to_temp_home(req)?;
        cmd.env("GNUPGHOME", &tmp_home);
        cmd.arg(key_id_from_home(&tmp_home)?);
    }
    if let Some(pass) = req.passphrase.filter(|p| !p.is_empty()) {
        cmd.arg("--pinentry-mode")
            .arg("loopback")
            .arg("--passphrase")
            .arg(pass);
    }
    Ok(cmd)
}

/// Import the armored secret key into a throwaway GNUPGHOME so signing is
/// hermetic (never touches the user's keyring). Returns the home path.
fn import_key_to_temp_home(req: &SignRequest) -> Result<PathBuf> {
    let home = tempfile::tempdir().context("failed to create temp GNUPGHOME")?;
    let path = home.path().to_path_buf();
    std::fs::create_dir_all(path.join(".gnupg"))?;
    // Permissions: gpg refuses overly-open keyrings.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
    }
    let key = req.key_bytes()?;
    let mut child = Command::new("gpg")
        .env("GNUPGHOME", &path)
        .args(["--batch", "--import"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to run `gpg --import`")?;
    {
        child.stdin.as_mut().expect("piped stdin").write_all(&key)?;
    }
    let status = child.wait().context("gpg --import failed")?;
    if !status.success() {
        bail!("gpg --import of '{}' failed", req.key_file.display());
    }
    // Leak the tempdir deliberately: GNUPGHOME must outlive the gpg call
    // below. Scoped small (one key), cleaned by tmpwatch/reboot semantics.
    std::mem::forget(home);
    Ok(path)
}

/// First secret-key fingerprint found in a GNUPGHOME.
pub fn key_id_from_home(home: &Path) -> Result<String> {
    let out = Command::new("gpg")
        .env("GNUPGHOME", home)
        .args(["--batch", "--list-secret-keys", "--with-colons"])
        .output()
        .context("failed to list imported gpg keys")?;
    if !out.status.success() {
        bail!(
            "gpg --list-secret-keys failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        // sec:fpr lines carry the primary secret key fingerprint.
        let fields: Vec<&str> = line.split(':').collect();
        if fields.len() > 4 && fields[0] == "sec" && !fields[4].trim().is_empty() {
            return Ok(fields[4].trim().to_string());
        }
    }
    bail!(
        "no usable secret key found after importing '{}' (does the file contain a secret, not public, key?)",
        home.display()
    );
}
