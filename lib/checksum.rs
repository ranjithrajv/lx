// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{anyhow, Context, Result};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;

/// Abstraction for fetching a URL as a streaming reader (used for sidecar probes).
/// Implemented for both `GitHubClient` and `GitlabClient` so `check_sidecar`
/// is source-agnostic.
pub trait RawGetter {
    fn raw_get(&self, url: &str) -> Result<Box<dyn std::io::Read + Send>>;
}

/// Compute the SHA-512 of a file in chunks (streaming, low memory).
pub fn sha512_file(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)
        .with_context(|| format!("failed to open '{}'", path.display()))?;
    let mut hasher = sha2::Sha512::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).context("failed to read file")?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Compute the SHA-256 of a file in chunks (streaming, low memory).
pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)
        .with_context(|| format!("failed to open '{}'", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).context("failed to read file")?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Parse a checksum file in common formats:
/// - "sha256  filename"
/// - "filename: sha256"
/// - "sha256  *filename"
///
/// Returns a map of filename -> hex digest.
pub fn parse_checksum_file(text: &str) -> Result<std::collections::HashMap<String, String>> {
    let mut out = std::collections::HashMap::new();
    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Coreutils `sha256sum` style: "<hash>  <name>"
        if let Some((hash, name)) = line.split_once("  ") {
            out.insert(name.trim_start_matches('*').to_string(), hash.to_string());
            continue;
        }
        // GoReleaser style: "<hash>  <name>"
        if let Some((hash, name)) = line.split_once(' ') {
            out.insert(name.trim_start_matches('*').to_string(), hash.to_string());
            continue;
        }
        return Err(anyhow!("unparseable checksum line {}: '{line}'", i + 1));
    }
    Ok(out)
}

/// Verify a downloaded file against an expected SHA-256 digest.
/// `expected` may be the full hex digest, or a prefix.
pub fn verify_sha256(path: &Path, expected: &str) -> Result<()> {
    let actual = sha256_file(path)?;
    let expected = expected.trim().to_ascii_lowercase();
    let actual = actual.to_ascii_lowercase();
    if expected.len() < 8 {
        return Err(anyhow!("checksum too short: '{expected}'"));
    }
    if !expected.starts_with(&actual[..expected.len()]) {
        return Err(anyhow!(
            "checksum mismatch for '{}':\n  expected {expected}\n  actual   {actual}",
            path.display()
        ));
    }
    Ok(())
}

/// Verify a downloaded file against an **inline** checksum the provider
/// published (GitHub/Gitea asset `digest`, SourceForge feed `md5`).
///
/// Returns the algorithm that matched, or `None` when the map has no
/// supported entry. A mismatch is always a hard `Err`.
pub fn check_inline(
    checksums: &std::collections::BTreeMap<String, String>,
    path: &Path,
) -> Result<Option<&'static str>> {
    if let Some(h) = checksums.get("sha256") {
        verify_sha256(path, h)?;
        return Ok(Some("sha256"));
    }
    if let Some(h) = checksums.get("sha512") {
        verify_sha512(path, h)?;
        return Ok(Some("sha512"));
    }
    if let Some(h) = checksums.get("md5") {
        verify_md5(path, h)?;
        return Ok(Some("md5"));
    }
    Ok(None)
}

/// Verify a downloaded file against an expected SHA-512 digest (a prefix is
/// accepted, like [`verify_sha256`]).
pub fn verify_sha512(path: &Path, expected: &str) -> Result<()> {
    let actual = sha512_file(path)?.to_ascii_lowercase();
    let expected = expected.trim().to_ascii_lowercase();
    if expected.len() < 8 {
        return Err(anyhow!("checksum too short: '{expected}'"));
    }
    if !actual.starts_with(&expected[..expected.len()]) {
        return Err(anyhow!(
            "checksum mismatch for '{}':\n  expected {expected}\n  actual   {actual}",
            path.display()
        ));
    }
    Ok(())
}

/// Verify a downloaded file against an expected MD5 digest (a prefix is
/// accepted). MD5 is what SourceForge publishes inline; it is weaker than
/// SHA-256 but still catches corruption and most tampering.
pub fn verify_md5(path: &Path, expected: &str) -> Result<()> {
    let data =
        std::fs::read(path).with_context(|| format!("failed to read '{}'", path.display()))?;
    let actual = format!("{:x}", md5::compute(&data)).to_ascii_lowercase();
    let expected = expected.trim().to_ascii_lowercase();
    if expected.len() < 8 {
        return Err(anyhow!("checksum too short: '{expected}'"));
    }
    if !actual.starts_with(&expected[..expected.len()]) {
        return Err(anyhow!(
            "checksum mismatch for '{}':\n  expected {expected}\n  actual   {actual}",
            path.display()
        ));
    }
    Ok(())
}

/// Outcome of probing for a live sidecar checksum next to a release asset.
#[derive(Debug)]
pub enum SidecarCheck {
    /// A sidecar was found, listed this asset, and its checksum matched.
    Verified,
    /// No sidecar covering this asset could be found at all (tried every
    /// known suffix). Distinct from a checksum *mismatch*, which is always
    /// a hard `Err` from `check_sidecar` and never returned as this variant
    /// -- callers that bypass verification on `NotFound` (e.g. an
    /// `--allow-unverified` flag) must never be able to accidentally bypass
    /// an actual tamper/corruption signal instead.
    NotFound,
}

/// Try every known sidecar-checksum suffix (`.sha256`, `.sha256sum`) next
/// to `asset_url`, verifying `path` against whichever one lists
/// `asset_filename`. Shared by `lx build` (`src/build.rs`) and `lx
/// install`/`lx upgrade` (`src/debs.rs`), which independently reimplemented
/// this same probe-and-verify loop before with subtly different bug
/// surfaces -- see `docs/decisions/2026-08-21-dry-solid-cleanup.md`.
///
/// Now generic over any `RawGetter` (GitHub or GitLab) so the same logic
/// works for both source providers.
pub fn check_sidecar(
    client: &dyn RawGetter,
    asset_url: &str,
    asset_filename: &str,
    path: &Path,
) -> Result<SidecarCheck> {
    for suffix in [".sha256", ".sha256sum"] {
        let sidecar_url = format!("{asset_url}{suffix}");
        let mut resp = match client.raw_get(&sidecar_url) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let mut text = String::new();
        resp.read_to_string(&mut text)?;
        let map = parse_checksum_file(&text)?;
        if let Some(expected) = map.get(asset_filename) {
            // A mismatch here propagates as `Err` immediately, unconditionally
            // -- never downgraded to `NotFound` regardless of what a caller
            // does with that variant.
            verify_sha256(path, expected)?;
            return Ok(SidecarCheck::Verified);
        }
    }
    Ok(SidecarCheck::NotFound)
}

/// Vet-time provenance pin (release-metadata.json), mirroring the action's
/// `fetch_pinned_checksum`. Two layouts are supported:
///   * modern: `{ "version": "...", "assets": { "<asset>": { "sha256": "..." } } }`
///   * legacy: `{ "version": "...", "asset": "<asset>", "sha256": "..." }`
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct PinnedMetadata {
    pub version: Option<String>,
    pub asset: Option<String>,
    pub sha256: Option<String>,
    #[serde(default)]
    pub assets: std::collections::HashMap<String, PinnedAsset>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct PinnedAsset {
    pub sha256: Option<String>,
}

impl PinnedMetadata {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read pin file '{}'", path.display()))?;
        let meta: Self = serde_json::from_str(&text)
            .with_context(|| format!("failed to parse pin file '{}'", path.display()))?;
        Ok(meta)
    }

    /// The pinned SHA-256 for `asset` @ `version`, if this pin covers it
    /// (matching the action's `fetch_pinned_checksum` semantics).
    pub fn sha256_for(&self, version: &str, asset: &str) -> Option<String> {
        if !matches!(&self.version, Some(v) if v == version) {
            return None;
        }
        if let Some(h) = self.assets.get(asset).and_then(|a| a.sha256.as_deref()) {
            if is_sha256(h) {
                return Some(h.to_ascii_lowercase());
            }
        }
        // Legacy layout.
        if let Some((a, h)) = self.asset.as_deref().zip(self.sha256.as_deref()) {
            if a == asset && is_sha256(h) {
                return Some(h.to_ascii_lowercase());
            }
        }
        None
    }
}

fn is_sha256(s: &str) -> bool {
    s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())
}
