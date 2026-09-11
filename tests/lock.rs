use lx_lib::lock::{LockEntry, LockFile};
use std::collections::BTreeMap;

#[test]
fn missing_lock_file_loads_as_none() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("package.lock");
    assert!(LockFile::load(&path).unwrap().is_none());
}

#[test]
fn round_trips_through_json_and_finds_entries_by_arch() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("package.lock");

    let mut packages = BTreeMap::new();
    packages.insert(
        "amd64".to_string(),
        LockEntry {
            tag: "v1.0.0".into(),
            asset: "tool_v1.0.0_amd64.tar.gz".into(),
            url: "https://example.com/tool_v1.0.0_amd64.tar.gz".into(),
            sha256: "a".repeat(64),
            source: "github".into(),
            published_at: Some(1_700_000_000),
        },
    );
    let lock = LockFile { packages };
    lock.save(&path).unwrap();

    let loaded = LockFile::load(&path).unwrap().expect("lock should exist");
    let entry = loaded.entry_for("amd64").expect("amd64 entry present");
    assert_eq!(entry.tag, "v1.0.0");
    assert_eq!(entry.sha256, "a".repeat(64));
    assert!(loaded.entry_for("arm64").is_none());
}

#[test]
fn path_for_lives_beside_the_config() {
    let config = std::path::Path::new("/tmp/pkgs/eza/package.yaml");
    assert_eq!(
        LockFile::path_for(config),
        std::path::PathBuf::from("/tmp/pkgs/eza/package.lock")
    );
}
