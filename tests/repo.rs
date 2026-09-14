// SPDX-License-Identifier: GPL-3.0-or-later

use lx_lib::repo::{read_control, run, RepoArgs};

fn tiny_deb(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    let root = dir.join("root");
    std::fs::create_dir_all(root.join("usr/bin")).unwrap();
    std::fs::write(root.join("usr/bin/hello"), b"payload").unwrap();
    let control = b"Package: hello\nVersion: 1.0-1+bookworm\nArchitecture: amd64\nMaintainer: T <t@e.c>\nDepends: libc6\nDescription: hi\n";
    let deb = dir.join(name);
    lx_lib::debarchive::build(&root, control, 0, &deb).unwrap();
    deb
}

#[test]
fn read_control_round_trips_a_built_deb() {
    let dir = tempfile::tempdir().unwrap();
    let deb = tiny_deb(dir.path(), "hello_1.0-1+bookworm_amd64.deb");
    let ctrl = read_control(&deb).unwrap();
    assert_eq!(ctrl.get("Package").map(String::as_str), Some("hello"));
    assert_eq!(
        ctrl.get("Version").map(String::as_str),
        Some("1.0-1+bookworm")
    );
    assert_eq!(ctrl.get("Architecture").map(String::as_str), Some("amd64"));
    assert_eq!(ctrl.get("Depends").map(String::as_str), Some("libc6"));
}

#[test]
fn repo_command_writes_packages_and_release() {
    let dir = tempfile::tempdir().unwrap();
    tiny_deb(dir.path(), "hello_1.0-1+bookworm_amd64.deb");
    lx_lib::repo::run(lx_lib::repo::RepoArgs {
        dir: dir.path().to_path_buf(),
        format: Some("deb".to_string()),
        suite: "bookworm".to_string(),
        multi_suite: false,
        components: "main".to_string(),
        origin: "latest-debs".to_string(),
        sign_key: None,
        sign_key_id: None,
    })
    .unwrap();
    let packages = std::fs::read_to_string(dir.path().join("Packages")).unwrap();
    assert!(packages.contains("Package: hello"));
    assert!(packages.contains("SHA256:"));
    assert!(dir.path().join("Packages.gz").exists());
    let release = std::fs::read_to_string(dir.path().join("Release")).unwrap();
    assert!(release.contains("Suite: bookworm"));
    assert!(release.contains("SHA256:"));
    assert!(!dir.path().join("InRelease").exists());
}

#[test]
fn repo_command_rejects_empty_dir() {
    let dir = tempfile::tempdir().unwrap();
    assert!(lx_lib::repo::run(lx_lib::repo::RepoArgs {
        dir: dir.path().to_path_buf(),
        format: Some("deb".to_string()),
        suite: "stable".to_string(),
        multi_suite: false,
        components: "main".to_string(),
        origin: "latest-debs".to_string(),
        sign_key: None,
        sign_key_id: None,
    })
    .is_err());
}

#[test]
fn multi_suite_repo_writes_per_suite_and_top_level_release() {
    let dir = tempfile::tempdir().unwrap();
    // Create dists/bookworm/ and dists/trixie/ with .deb files each.
    let bookworm_dir = dir.path().join("dists/bookworm");
    let trixie_dir = dir.path().join("dists/trixie");
    std::fs::create_dir_all(&bookworm_dir).unwrap();
    std::fs::create_dir_all(&trixie_dir).unwrap();
    tiny_deb(&bookworm_dir, "hello_1.0-1+bookworm_amd64.deb");
    tiny_deb(&trixie_dir, "hello_1.0-1+trixie_amd64.deb");

    run(RepoArgs {
        dir: dir.path().to_path_buf(),
        format: Some("deb".to_string()),
        suite: "stable".to_string(), // ignored in multi-suite mode
        multi_suite: true,
        components: "main".to_string(),
        origin: "test-repo".to_string(),
        sign_key: None,
        sign_key_id: None,
    })
    .unwrap();

    // Per-suite Packages.
    let bw_packages = std::fs::read_to_string(bookworm_dir.join("Packages")).unwrap();
    assert!(bw_packages.contains("Package: hello"));
    assert!(bw_packages.contains("Filename: dists/bookworm/"));

    // Per-suite Release.
    let bw_release = std::fs::read_to_string(bookworm_dir.join("Release")).unwrap();
    assert!(bw_release.contains("Suite: bookworm"));

    // Top-level Release listing both suites.
    let top_release = std::fs::read_to_string(dir.path().join("Release")).unwrap();
    assert!(top_release.contains("Suites: bookworm trixie"));
    assert!(top_release.contains("dists/bookworm/Packages"));
    assert!(top_release.contains("dists/trixie/Packages"));
}

#[test]
fn multi_suite_repo_fallback_to_flat_layout() {
    let dir = tempfile::tempdir().unwrap();
    // No dists/ subdirectory — .deb files directly in root.
    tiny_deb(dir.path(), "hello_1.0-1+bookworm_amd64.deb");

    run(RepoArgs {
        dir: dir.path().to_path_buf(),
        format: Some("deb".to_string()),
        suite: "stable".to_string(),
        multi_suite: true,
        components: "main".to_string(),
        origin: "test-repo".to_string(),
        sign_key: None,
        sign_key_id: None,
    })
    .unwrap();

    // Should fall back to treating root as a single "stable" suite.
    let release = std::fs::read_to_string(dir.path().join("Release")).unwrap();
    assert!(release.contains("Suite: stable"));
}
