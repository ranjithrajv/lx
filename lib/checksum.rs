use anyhow::{anyhow, Context, Result};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;

use crate::github::GitHubClient;

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

/// Verify a file against a SHA-256SUMS file already written to disk.
#[allow(dead_code)] // reserved for --checksum-file flow (action parity)
pub fn verify_from_checksum_file(file: &Path, checksum_file: &Path) -> Result<()> {
    let text = std::fs::read_to_string(checksum_file)
        .with_context(|| format!("failed to read '{}'", checksum_file.display()))?;
    let map = parse_checksum_file(&text)?;
    let fname = file
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| anyhow!("invalid filename for '{}'", file.display()))?;
    let missing = format!(
        "no checksum entry for '{fname}' in '{}'",
        checksum_file.display()
    );
    let expected = map.get(fname).ok_or_else(|| anyhow!(missing))?;
    verify_sha256(file, expected)
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
/// `asset_filename`. Shared by `lpt build` (`src/build.rs`) and `lpt
/// install`/`lpt upgrade` (`src/debs.rs`), which independently reimplemented
/// this same probe-and-verify loop before with subtly different bug
/// surfaces -- see `docs/decisions/2026-08-21-dry-solid-cleanup.md`.
pub fn check_sidecar(
    client: &GitHubClient,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_coreutils_format() {
        let text = "abc123  file.tar.gz\nabc123  *starfile.tar.gz\n# comment\n\n";
        let map = parse_checksum_file(text).unwrap();
        assert_eq!(map.get("file.tar.gz").unwrap(), "abc123");
        assert_eq!(map.get("starfile.tar.gz").unwrap(), "abc123");
    }

    #[test]
    fn parses_single_space_format() {
        let text = "abc123 file.tar.gz\nd4d444 *starfile.tar.gz\n";
        let map = parse_checksum_file(text).unwrap();
        assert_eq!(map.get("file.tar.gz").unwrap(), "abc123");
        assert_eq!(map.get("starfile.tar.gz").unwrap(), "d4d444");
    }

    #[test]
    fn rejects_unparseable_line() {
        // No space at all: neither the double- nor single-space parser matches.
        assert!(parse_checksum_file("justwords").is_err());
    }

    #[test]
    fn verifies_matching_sha() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("t.bin");
        std::fs::write(&f, b"hello world").unwrap();
        let sum = sha256_file(&f).unwrap();
        verify_sha256(&f, &sum).unwrap();
        // short prefix
        verify_sha256(&f, &sum[..12]).unwrap();
    }

    #[test]
    fn rejects_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("t.bin");
        std::fs::write(&f, b"hello world").unwrap();
        let bad = "f".repeat(64);
        assert!(verify_sha256(&f, &bad).is_err());
    }

    #[test]
    fn rejects_too_short_checksum() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("t.bin");
        std::fs::write(&f, b"hello world").unwrap();
        assert!(verify_sha256(&f, "abcd").is_err());
    }

    #[test]
    fn verify_from_checksum_file_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("tool.tar.gz");
        std::fs::write(&f, b"archive payload").unwrap();
        let sum = sha256_file(&f).unwrap();
        let sums = dir.path().join("SHA256SUMS");
        std::fs::write(&sums, format!("{sum}  tool.tar.gz\n")).unwrap();
        verify_from_checksum_file(&f, &sums).unwrap();
    }

    #[test]
    fn verify_from_checksum_file_missing_sums() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("tool.tar.gz");
        std::fs::write(&f, b"x").unwrap();
        assert!(verify_from_checksum_file(&f, &dir.path().join("nope")).is_err());
    }

    #[test]
    fn verify_from_checksum_file_missing_entry() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("tool.tar.gz");
        std::fs::write(&f, b"x").unwrap();
        let sums = dir.path().join("SHA256SUMS");
        std::fs::write(&sums, format!("{}\n", "a".repeat(64))).unwrap();
        assert!(verify_from_checksum_file(&f, &sums).is_err());
    }

    #[test]
    fn verify_from_checksum_file_bad_filename() {
        let dir = tempfile::tempdir().unwrap();
        let sums = dir.path().join("SHA256SUMS");
        std::fs::write(&sums, "".as_bytes()).unwrap();
        assert!(verify_from_checksum_file(&dir.path().join(".."), &sums).is_err());
    }

    #[test]
    fn pinned_metadata_modern_layout() {
        let h = "a".repeat(64);
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("release-metadata.json");
        std::fs::write(
            &f,
            format!(r#"{{"version":"v1.0.0","assets":{{"tool.tar.gz":{{"sha256":"{h}"}}}}}}"#),
        )
        .unwrap();
        let m = PinnedMetadata::load(&f).unwrap();
        assert_eq!(m.sha256_for("v1.0.0", "tool.tar.gz").unwrap(), h);
        // Wrong version or asset -> no pin.
        assert!(m.sha256_for("v2.0.0", "tool.tar.gz").is_none());
        assert!(m.sha256_for("v1.0.0", "other.tar.gz").is_none());
    }

    #[test]
    fn pinned_metadata_legacy_layout() {
        let h = "b".repeat(64);
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("release-metadata.json");
        std::fs::write(
            &f,
            format!(r#"{{"version":"v1.0.0","asset":"tool.tar.gz","sha256":"{h}"}}"#),
        )
        .unwrap();
        let m = PinnedMetadata::load(&f).unwrap();
        assert_eq!(m.sha256_for("v1.0.0", "tool.tar.gz").unwrap(), h);
        assert!(m.sha256_for("v1.0.0", "nope.tar.gz").is_none());
    }

    #[test]
    fn pinned_metadata_rejects_non_sha256() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("release-metadata.json");
        std::fs::write(
            &f,
            r#"{"version":"v1.0.0","asset":"tool.tar.gz","sha256":"not-a-hash"}"#,
        )
        .unwrap();
        let m = PinnedMetadata::load(&f).unwrap();
        assert!(m.sha256_for("v1.0.0", "tool.tar.gz").is_none());
    }

    #[test]
    fn pinned_metadata_modern_layout_invalid_hash_falls_through() {
        // Modern layout with a non-SHA-256 value must not return the value
        // (line coverage for the `is_sha256` guard), and must fall through
        // to the legacy check.
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("release-metadata.json");
        std::fs::write(
            &f,
            r#"{"version":"v1.0.0","assets":{"tool.tar.gz":{"sha256":"not-a-hash"}}}"#,
        )
        .unwrap();
        let m = PinnedMetadata::load(&f).unwrap();
        assert!(m.sha256_for("v1.0.0", "tool.tar.gz").is_none());
    }
}

/// Live, network-dependent regression tests -- `#[ignore]`d by default
/// (run explicitly via `cargo test -- --ignored`), mirroring
/// `debarchive.rs`'s pattern for tests that need a real external
/// dependency (there, a real `dpkg-deb`; here, a real GitHub release).
#[cfg(test)]
mod live_tests {
    use super::*;
    use crate::github::GitHubClient;

    /// A real checksum MISMATCH must never be downgraded to `NotFound`
    /// (which callers like `--allow-unverified` treat as "nothing to
    /// check, proceed") -- confirmed against a real GitHub release with a
    /// real published sidecar, where the local file is deliberately
    /// tampered after download so it no longer matches. Regression test
    /// for a real bug found while unifying `build.rs`'s and `debs.rs`'s
    /// independent sidecar-verification implementations: `build.rs`'s
    /// previous split (`verify_sidecar` returning a generic `Result<()>`,
    /// wrapped by a `match ... Err(e) if allow_unverified` in the caller)
    /// couldn't distinguish "no sidecar found" from "sidecar found but
    /// mismatched" -- both looked like a generic `Err` to the wrapper, so
    /// `--allow-unverified` could silently swallow an actual tamper/
    /// corruption signal. `check_sidecar`'s `SidecarCheck` enum makes that
    /// conflation impossible at the type level: `NotFound` is never
    /// returned on a mismatch, only on an actual absence.
    #[test]
    #[ignore]
    fn real_mismatch_is_never_reported_as_not_found() {
        let client = GitHubClient::new(None).unwrap();
        let asset_url = "https://github.com/BurntSushi/ripgrep/releases/download/15.2.0/ripgrep-15.2.0-x86_64-unknown-linux-musl.tar.gz";
        let asset_name = "ripgrep-15.2.0-x86_64-unknown-linux-musl.tar.gz";

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(asset_name);
        let mut resp = client.raw_get(asset_url).unwrap();
        let mut file = std::fs::File::create(&path).unwrap();
        std::io::copy(&mut resp, &mut file).unwrap();
        drop(file);

        // Tamper: flip the file's content so its real SHA-256 no longer
        // matches the real published sidecar.
        std::fs::write(&path, b"tampered bytes, does not match the real sidecar").unwrap();

        let result = check_sidecar(&client, asset_url, asset_name, &path);
        eprintln!("result: {result:?}");
        assert!(
            result.is_err(),
            "a checksum mismatch must be a hard Err, never Ok(SidecarCheck::NotFound)"
        );
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("mismatch"),
            "expected a checksum-mismatch error, got: {msg}"
        );
    }
}
