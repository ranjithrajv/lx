use lx_lib::repo::read_control;

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
        suite: "bookworm".to_string(),
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
        suite: "stable".to_string(),
        sign_key: None,
        sign_key_id: None,
    })
    .is_err());
}
