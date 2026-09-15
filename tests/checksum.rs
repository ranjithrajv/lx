// SPDX-License-Identifier: GPL-3.0-or-later

use lx_lib::checksum::*;

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
fn sha256_tree_is_stable_and_sensitive_to_content_and_layout() {
    let a = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(a.path().join("sub")).unwrap();
    std::fs::write(a.path().join("b.conf"), b"bee").unwrap();
    std::fs::write(a.path().join("sub/a.conf"), b"aye").unwrap();
    std::os::unix::fs::symlink("b.conf", a.path().join("link")).unwrap();

    // A second tree with identical contents, created in a different insertion
    // order, must hash the same (the digest sorts paths).
    let b = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(b.path().join("sub")).unwrap();
    std::fs::write(b.path().join("sub/a.conf"), b"aye").unwrap();
    std::os::unix::fs::symlink("b.conf", b.path().join("link")).unwrap();
    std::fs::write(b.path().join("b.conf"), b"bee").unwrap();

    let base = sha256_tree(a.path()).unwrap();
    assert_eq!(
        base,
        sha256_tree(a.path()).unwrap(),
        "digest must be stable"
    );
    assert_eq!(
        base,
        sha256_tree(b.path()).unwrap(),
        "readdir order must not affect the digest"
    );

    // Content, membership, path, and symlink target each change the digest.
    std::fs::write(a.path().join("b.conf"), b"BEE").unwrap();
    let changed = sha256_tree(a.path()).unwrap();
    assert_ne!(base, changed, "content change must change the digest");

    std::fs::write(a.path().join("new.conf"), b"new").unwrap();
    assert_ne!(
        changed,
        sha256_tree(a.path()).unwrap(),
        "adding a file must change the digest"
    );

    std::fs::remove_file(a.path().join("new.conf")).unwrap();
    let before_rename = sha256_tree(a.path()).unwrap();
    assert_eq!(
        before_rename, changed,
        "removing the added file restores it"
    );
    std::fs::rename(a.path().join("b.conf"), a.path().join("c.conf")).unwrap();
    assert_ne!(
        before_rename,
        sha256_tree(a.path()).unwrap(),
        "renaming a file must change the digest"
    );

    std::fs::remove_file(a.path().join("link")).unwrap();
    std::os::unix::fs::symlink("sub/a.conf", a.path().join("link")).unwrap();
    assert_ne!(
        sha256_tree(a.path()).unwrap(),
        sha256_tree(b.path()).unwrap(),
        "changing a symlink target must change the digest"
    );

    // Permission bits land in the packaged payload, so they must key the cache.
    use std::os::unix::fs::PermissionsExt;
    let before_mode = sha256_tree(a.path()).unwrap();
    let mut perms = std::fs::metadata(a.path().join("c.conf"))
        .unwrap()
        .permissions();
    perms.set_mode(0o600);
    std::fs::set_permissions(a.path().join("c.conf"), perms).unwrap();
    assert_ne!(
        before_mode,
        sha256_tree(a.path()).unwrap(),
        "changing a file mode must change the digest"
    );
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
    let client = lx_lib::github::GitHubClient::new(None).unwrap();
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

#[test]
fn check_inline_verifies_supported_algorithms() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("asset");
    std::fs::write(&path, b"hello world").unwrap();

    let mut m = std::collections::BTreeMap::new();
    m.insert(
        "md5".to_string(),
        format!("{:x}", md5::compute(b"hello world")),
    );
    assert_eq!(
        lx_lib::checksum::check_inline(&m, &path).unwrap(),
        Some("md5")
    );

    let mut m = std::collections::BTreeMap::new();
    m.insert(
        "sha256".to_string(),
        lx_lib::checksum::sha256_file(&path).unwrap(),
    );
    assert_eq!(
        lx_lib::checksum::check_inline(&m, &path).unwrap(),
        Some("sha256")
    );

    let mut m = std::collections::BTreeMap::new();
    m.insert(
        "sha512".to_string(),
        lx_lib::checksum::sha512_file(&path).unwrap(),
    );
    assert_eq!(
        lx_lib::checksum::check_inline(&m, &path).unwrap(),
        Some("sha512")
    );

    // An empty map has no supported entry.
    assert_eq!(
        lx_lib::checksum::check_inline(&Default::default(), &path).unwrap(),
        None
    );
}

#[test]
fn check_inline_mismatch_is_a_hard_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("asset");
    std::fs::write(&path, b"hello world").unwrap();
    let mut m = std::collections::BTreeMap::new();
    m.insert("sha256".to_string(), "00".repeat(32));
    assert!(lx_lib::checksum::check_inline(&m, &path).is_err());
}
