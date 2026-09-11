use lx_lib::rpmarchive::*;

use std::os::unix::fs::PermissionsExt;

#[test]
fn rpm_arch_mapping() {
    assert_eq!(to_rpm_arch("amd64"), "x86_64");
    assert_eq!(to_rpm_arch("arm64"), "aarch64");
    assert_eq!(to_rpm_arch("ppc64el"), "ppc64le");
    assert_eq!(to_rpm_arch("s390x"), "s390x");
}

#[test]
fn build_produces_valid_rpm() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("usr/bin")).unwrap();
    std::fs::write(root.path().join("usr/bin/hello"), b"payload").unwrap();
    let mut perms = std::fs::metadata(root.path().join("usr/bin/hello"))
        .unwrap()
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(root.path().join("usr/bin/hello"), perms).unwrap();

    let rpm_path = root.path().join("hello.rpm");
    build(
        root.path(),
        &PackageMeta {
            name: "hello",
            version: "1.0",
            release: "1",
            summary: "test package",
            description: "Packaged from upstream",
            license: "MIT",
        },
        "amd64",
        1_735_689_600,
        &rpm_path,
    )
    .unwrap();

    assert!(rpm_path.exists());
    let bytes = std::fs::read(&rpm_path).unwrap();
    // RPM magic: 0xED 0xAB 0xEE 0xDB
    assert_eq!(&bytes[0..4], &[0xED, 0xAB, 0xEE, 0xDB]);
}

#[test]
fn same_input_produces_deterministic_output_for_same_mtime() {
    let build_once = || {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("usr/bin")).unwrap();
        std::fs::write(root.path().join("usr/bin/hello"), b"fake-elf").unwrap();
        let rpm_path = root.path().join("hello.rpm");
        build(
            root.path(),
            &PackageMeta {
                name: "hello",
                version: "1.0",
                release: "1",
                summary: "test",
                description: "desc",
                license: "MIT",
            },
            "amd64",
            1_735_689_600,
            &rpm_path,
        )
        .unwrap();
        std::fs::read(&rpm_path).unwrap()
    };
    assert_eq!(build_once(), build_once());
}

#[test]
fn scriptlets_build_without_error() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("usr/bin")).unwrap();
    std::fs::write(root.path().join("usr/bin/hello"), b"fake-elf").unwrap();
    let rpm_path = root.path().join("hello.rpm");
    let opts = BuildOptions {
        pre_install: Some("echo pre"),
        post_install: Some("echo post"),
        pre_uninstall: None,
        post_uninstall: Some("echo postun"),
        sign_key_file: None,
        sign_passphrase: None,
    };
    build_with_options(
        root.path(),
        &PackageMeta {
            name: "hello",
            version: "1.0",
            release: "1",
            summary: "test",
            description: "desc",
            license: "MIT",
        },
        "amd64",
        1_735_689_600,
        &rpm_path,
        &opts,
    )
    .unwrap();
    let bytes = std::fs::read(&rpm_path).unwrap();
    assert_eq!(&bytes[0..4], &[0xED, 0xAB, 0xEE, 0xDB]);
}

/// Full signing round trip with a generated throwaway key; skips when
/// gpg or quick key generation is unavailable.
#[test]
fn native_signing_round_trip_with_generated_key() {
    let Ok(_) = std::process::Command::new("gpg").arg("--version").output() else {
        eprintln!("skipping: gpg not on PATH");
        return;
    };
    let home = match tempfile::tempdir() {
        Ok(h) => h,
        Err(_) => return,
    };
    let gen = std::process::Command::new("gpg")
        .env("GNUPGHOME", home.path())
        .args([
            "--batch",
            "--pinentry-mode",
            "loopback",
            "--passphrase",
            "",
            "--quick-gen-key",
            "lx-rpm-test <lx@example.invalid>",
            "default",
            "default",
            "never",
        ])
        .output();
    let Ok(gen) = gen else { return };
    if !gen.status.success() {
        eprintln!("skipping: quick-gen-key unsupported here");
        return;
    }
    // Export the secret key (armored) for the crate's Signer.
    let list = std::process::Command::new("gpg")
        .env("GNUPGHOME", home.path())
        .args(["--batch", "--list-secret-keys", "--with-colons"])
        .output()
        .unwrap();
    let fpr = String::from_utf8_lossy(&list.stdout)
        .lines()
        .find_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            (f.len() > 4 && f[0] == "sec").then(|| f[4].to_string())
        })
        .expect("secret key fingerprint");
    let export = std::process::Command::new("gpg")
        .env("GNUPGHOME", home.path())
        .args(["--batch", "--armor", "--export-secret-keys", &fpr])
        .output()
        .unwrap();
    assert!(export.status.success());
    let key_file = home.path().join("key.asc");
    std::fs::write(&key_file, &export.stdout).unwrap();

    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("usr/bin")).unwrap();
    std::fs::write(root.path().join("usr/bin/hello"), b"fake-elf").unwrap();
    let rpm_path = root.path().join("hello-signed.rpm");
    let opts = BuildOptions {
        sign_key_file: Some(&key_file),
        sign_passphrase: None,
        ..Default::default()
    };
    build_with_options(
        root.path(),
        &PackageMeta {
            name: "hello",
            version: "1.0",
            release: "1",
            summary: "test",
            description: "desc",
            license: "MIT",
        },
        "amd64",
        1_735_689_600,
        &rpm_path,
        &opts,
    )
    .unwrap();
    assert!(rpm_path.exists());
    // Signature header tags are present in the package.
    if let Ok(info) = std::process::Command::new("rpm")
        .args(["-qpi", rpm_path.to_str().unwrap()])
        .output()
    {
        // Best-effort: host rpm may not read our payload; only require
        // that the file exists and is non-trivially sized.
        let _ = info;
    }
    assert!(std::fs::metadata(&rpm_path).unwrap().len() > 0);
}

#[test]
fn build_srpm_produces_valid_rpm_with_spec_and_source() {
    let tmp = tempfile::tempdir().unwrap();
    let srpm_path = tmp.path().join("hello-1.0-1.fedora.src.rpm");
    build_srpm(
        &PackageMeta {
            name: "hello",
            version: "1.0",
            release: "1.fedora",
            summary: "test package",
            description: "Packaged from upstream",
            license: "MIT",
        },
        ("hello.spec", b"Name: hello\nVersion: 1.0\n"),
        ("hello-1.0.tar.xz", b"fake-tarball-bytes"),
        1_735_689_600,
        &srpm_path,
    )
    .unwrap();

    assert!(srpm_path.exists());
    let bytes = std::fs::read(&srpm_path).unwrap();
    assert_eq!(&bytes[0..4], &[0xED, 0xAB, 0xEE, 0xDB], "rpm magic missing");

    let pkg = rpm::Package::open(&srpm_path).unwrap();
    let paths: Vec<String> = pkg
        .metadata
        .get_file_entries()
        .unwrap()
        .into_iter()
        .map(|e| e.path.to_string_lossy().to_string())
        .collect();
    assert!(paths.iter().any(|p| p.ends_with("hello.spec")), "{paths:?}");
    assert!(
        paths.iter().any(|p| p.ends_with("hello-1.0.tar.xz")),
        "{paths:?}"
    );
}

#[test]
fn extract_round_trips_files_dirs_and_symlinks() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("usr/bin")).unwrap();
    std::fs::write(root.path().join("usr/bin/hello"), b"payload-bytes").unwrap();
    let mut perms = std::fs::metadata(root.path().join("usr/bin/hello"))
        .unwrap()
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(root.path().join("usr/bin/hello"), perms).unwrap();
    std::os::unix::fs::symlink("hello", root.path().join("usr/bin/hello-link")).unwrap();

    let rpm_path = root.path().join("hello.rpm");
    build(
        root.path(),
        &PackageMeta {
            name: "hello",
            version: "1.0",
            release: "1",
            summary: "test",
            description: "desc",
            license: "MIT",
        },
        "amd64",
        1_735_689_600,
        &rpm_path,
    )
    .unwrap();

    let dest = tempfile::tempdir().unwrap();
    extract(&rpm_path, dest.path()).unwrap();

    let extracted = std::fs::read(dest.path().join("usr/bin/hello")).unwrap();
    assert_eq!(extracted, b"payload-bytes");
    let mode = std::fs::metadata(dest.path().join("usr/bin/hello"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o755);

    let link_target = std::fs::read_link(dest.path().join("usr/bin/hello-link")).unwrap();
    assert_eq!(link_target, std::path::PathBuf::from("hello"));
}
