use lx_lib::archarchive::*;

use std::os::unix::fs::PermissionsExt;

#[test]
fn pacman_arch_mapping() {
    assert_eq!(to_pacman_arch("amd64"), "x86_64");
    assert_eq!(to_pacman_arch("arm64"), "aarch64");
    assert_eq!(to_pacman_arch("armhf"), "armv7h");
}

#[test]
fn build_produces_valid_pkg_tar_zst() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("usr/bin")).unwrap();
    std::fs::write(root.path().join("usr/bin/hello"), b"payload").unwrap();
    let mut perms = std::fs::metadata(root.path().join("usr/bin/hello"))
        .unwrap()
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(root.path().join("usr/bin/hello"), perms).unwrap();

    let out = root.path().join("hello-1.0-1-x86_64.pkg.tar.zst");
    build(
        root.path(),
        &PackageMeta {
            name: "hello",
            version: "1.0",
            release: "1",
            description: "test package",
            url: "https://example.com",
            license: "MIT",
        },
        "amd64",
        1_735_689_600,
        &out,
    )
    .unwrap();

    assert!(out.exists());
    let bytes = std::fs::read(&out).unwrap();
    // zstd magic: 0x28 B5 2F FD
    assert_eq!(&bytes[0..4], &[0x28, 0xB5, 0x2F, 0xFD]);
    // Decompress and verify .PKGINFO present
    let dec = zstd::stream::read::Decoder::new(bytes.as_slice()).unwrap();
    let mut ar = tar::Archive::new(dec);
    let names: Vec<String> = ar
        .entries()
        .unwrap()
        .map(|e| e.unwrap().path().unwrap().to_string_lossy().to_string())
        .collect();
    assert!(names.contains(&".PKGINFO".to_string()), "{names:?}");
    assert!(names.contains(&".MTREE".to_string()), "{names:?}");
    assert!(names.contains(&"usr/bin/hello".to_string()), "{names:?}");
}

#[test]
fn extract_skips_metadata_and_round_trips_payload() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("usr/bin")).unwrap();
    std::fs::write(root.path().join("usr/bin/hello"), b"payload-bytes").unwrap();
    let mut perms = std::fs::metadata(root.path().join("usr/bin/hello"))
        .unwrap()
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(root.path().join("usr/bin/hello"), perms).unwrap();

    let out = root.path().join("hello-1.0-1-x86_64.pkg.tar.zst");
    build(
        root.path(),
        &PackageMeta {
            name: "hello",
            version: "1.0",
            release: "1",
            description: "test package",
            url: "https://example.com",
            license: "MIT",
        },
        "amd64",
        1_735_689_600,
        &out,
    )
    .unwrap();

    let dest = tempfile::tempdir().unwrap();
    extract(&out, dest.path()).unwrap();

    assert!(!dest.path().join(".PKGINFO").exists());
    assert!(!dest.path().join(".MTREE").exists());
    let extracted = std::fs::read(dest.path().join("usr/bin/hello")).unwrap();
    assert_eq!(extracted, b"payload-bytes");
}

#[test]
fn same_input_is_deterministic() {
    let build_once = || {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("usr/bin")).unwrap();
        std::fs::write(root.path().join("usr/bin/hello"), b"fake-elf").unwrap();
        let out = root.path().join("hello-1.0-1-x86_64.pkg.tar.zst");
        build(
            root.path(),
            &PackageMeta {
                name: "hello",
                version: "1.0",
                release: "1",
                description: "test",
                url: "https://example.com",
                license: "MIT",
            },
            "amd64",
            1_735_689_600,
            &out,
        )
        .unwrap();
        std::fs::read(&out).unwrap()
    };
    assert_eq!(build_once(), build_once());
}

#[test]
fn pkginfo_contains_required_fields() {
    let s = render_pkginfo(
        &PackageMeta {
            name: "foo",
            version: "1.2.3",
            release: "1",
            description: "desc",
            url: "https://ex.com",
            license: "MIT",
        },
        "1.2.3-1",
        "x86_64",
        0,
        123,
    );
    assert!(s.contains("pkgname = foo"));
    assert!(s.contains("pkgver = 1.2.3-1"));
    assert!(s.contains("arch = x86_64"));
    assert!(s.contains("license = MIT"));
}
