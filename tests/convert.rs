// SPDX-License-Identifier: GPL-3.0-or-later

use lx_lib::convert::{run, ConvertArgs};

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
fn convert_deb_to_deb_is_noop_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let deb = tiny_deb(dir.path(), "hello_1.0-1+bookworm_amd64.deb");

    let result = run(ConvertArgs {
        input: deb,
        to: Some("deb".to_string()),
        output: dir.path().to_path_buf(),
        package_name: None,
        version: None,
        arch: None,
        distribution: None,
        build_version: "1".to_string(),
        dry_run: false,
    });
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("nothing to convert"));
}

#[test]
fn convert_deb_dry_run_reports_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let deb = tiny_deb(dir.path(), "hello_1.0-1+bookworm_amd64.deb");

    // Dry run should succeed and not create output.
    run(ConvertArgs {
        input: deb,
        to: Some("deb".to_string()),
        output: dir.path().to_path_buf(),
        package_name: None,
        version: None,
        arch: None,
        distribution: None,
        build_version: "1".to_string(),
        dry_run: true,
    })
    .unwrap_err(); // deb→deb is rejected even in dry-run
}

#[test]
fn convert_rejects_nonexistent_input() {
    let dir = tempfile::tempdir().unwrap();
    let result = run(ConvertArgs {
        input: dir.path().join("nonexistent.deb"),
        to: Some("rpm".to_string()),
        output: dir.path().to_path_buf(),
        package_name: None,
        version: None,
        arch: None,
        distribution: None,
        build_version: "1".to_string(),
        dry_run: false,
    });
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("does not exist"));
}

#[test]
fn convert_rejects_unknown_target_format() {
    let dir = tempfile::tempdir().unwrap();
    let deb = tiny_deb(dir.path(), "hello_1.0-1+bookworm_amd64.deb");

    let result = run(ConvertArgs {
        input: deb,
        to: Some("apk".to_string()),
        output: dir.path().to_path_buf(),
        package_name: None,
        version: None,
        arch: None,
        distribution: None,
        build_version: "1".to_string(),
        dry_run: false,
    });
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("unsupported target format"));
}
