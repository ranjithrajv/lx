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

/// Build a real `.rpm` in-process (no `rpm`/`rpm2cpio`/`cpio` on PATH) so the
/// conversion test exercises the native RPM reader.
fn tiny_rpm(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    use lx_lib::rpmarchive::{self, BuildOptions, PackageMeta, RpmRelations};
    let root = dir.join("rpmroot");
    std::fs::create_dir_all(root.join("usr/bin")).unwrap();
    std::fs::write(root.join("usr/bin/hello"), b"payload").unwrap();
    let rpm = dir.join(name);
    let opts = BuildOptions {
        relations: RpmRelations {
            requires: vec![rpm::Dependency::greater_eq("glibc", "2.17")],
            recommends: vec![rpm::Dependency::any("bash")],
            conflicts: vec![rpm::Dependency::any("old-pkg")],
            obsoletes: vec![rpm::Dependency::any("legacy")],
            provides: vec![rpm::Dependency::any("webserver")],
            ..Default::default()
        },
        epoch: Some(2),
        ..Default::default()
    };
    rpmarchive::build_with_options(
        &root,
        &PackageMeta {
            name: "hello",
            version: "1.0",
            release: "1",
            summary: "hi",
            description: "hi",
            license: "MIT",
            vendor: None,
            packager: None,
        },
        "amd64",
        1_735_689_600,
        &rpm,
        &opts,
        &lx_lib::filemeta::FileMetaMap::new(),
    )
    .unwrap();
    rpm
}

#[test]
fn convert_rpm_to_deb_in_process_carries_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let rpm = tiny_rpm(dir.path(), "hello-1.0-1.x86_64.rpm");

    run(ConvertArgs {
        input: rpm,
        to: Some("deb".to_string()),
        output: dir.path().join("out"),
        package_name: None,
        version: None,
        arch: None,
        distribution: None,
        build_version: "1".to_string(),
        dry_run: false,
    })
    .unwrap();

    let deb = std::fs::read_dir(dir.path().join("out"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().map(|x| x == "deb").unwrap_or(false))
        .expect("a .deb was produced");
    let ctrl = lx_lib::repo::read_control(&deb).unwrap();

    // Arch normalized RPM→deb.
    assert_eq!(ctrl.get("Architecture").map(String::as_str), Some("amd64"));
    // Epoch carried into the deb Version.
    assert!(
        ctrl.get("Version").unwrap().starts_with("2:"),
        "epoch carried: {:?}",
        ctrl.get("Version")
    );
    // Relation syntax rewritten RPM→deb, and non-Depends relations carried.
    assert!(
        ctrl.get("Depends").unwrap().contains("glibc (>= 2.17)"),
        "dep syntax converted: {:?}",
        ctrl.get("Depends")
    );
    assert!(ctrl.get("Recommends").unwrap().contains("bash"));
    assert!(ctrl.get("Conflicts").unwrap().contains("old-pkg"));
    assert!(ctrl.get("Replaces").unwrap().contains("legacy"));
    // Virtual Provides carried; the RPM self-provide and sonames dropped.
    assert_eq!(
        ctrl.get("Provides").map(String::as_str),
        Some("webserver"),
        "provides carried + filtered: {:?}",
        ctrl.get("Provides")
    );
}

#[test]
fn convert_deb_to_rpm_carries_conffiles() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("debroot");
    std::fs::create_dir_all(root.join("usr/bin")).unwrap();
    std::fs::create_dir_all(root.join("etc")).unwrap();
    std::fs::write(root.join("usr/bin/hello"), b"payload").unwrap();
    std::fs::write(root.join("etc/hello.conf"), b"a=1").unwrap();
    let control = b"Package: hello\nVersion: 1.0-1+bookworm\nArchitecture: amd64\nMaintainer: T <t@e.c>\nConffiles:\n /etc/hello.conf abc123\nDescription: hi\n";
    let deb = dir.path().join("hello_1.0-1+bookworm_amd64.deb");
    lx_lib::debarchive::build(&root, control, 0, &deb).unwrap();

    run(ConvertArgs {
        input: deb,
        to: Some("rpm".to_string()),
        output: dir.path().join("out"),
        package_name: None,
        version: None,
        arch: None,
        distribution: None,
        build_version: "1".to_string(),
        dry_run: false,
    })
    .unwrap();

    let rpm = std::fs::read_dir(dir.path().join("out"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().map(|x| x == "rpm").unwrap_or(false))
        .expect("a .rpm was produced");
    let pkg = rpm::Package::open(&rpm).unwrap();
    let entries = pkg.metadata.get_file_entries().unwrap();
    let flags = entries
        .iter()
        .find(|e| e.path.to_string_lossy() == "/etc/hello.conf")
        .expect("conffile present in the rpm payload")
        .flags;
    assert!(
        flags.contains(rpm::FileFlags::CONFIG),
        "deb conffile became an rpm %config: {flags:?}"
    );
}
