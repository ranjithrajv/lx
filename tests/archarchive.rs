// SPDX-License-Identifier: GPL-3.0-or-later

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
        None,
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
        None,
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
            None,
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

#[test]
fn debian_relations_translate_to_pacman() {
    assert_eq!(
        debian_relations_to_pacman(
            "libc6 (>= 2.34), libssl3 | libssl1.1, foo:any, bar (<< 2.0), baz [amd64]"
        ),
        vec!["libc6>=2.34", "libssl3", "foo", "bar<2.0", "baz"]
    );
    assert_eq!(
        debian_relations_to_pacman("zlib1g (>= 1:1.2.0), bash (<= 5.0)"),
        vec!["zlib1g>=1:1.2.0", "bash<=5.0"]
    );
    assert!(debian_relations_to_pacman("  ").is_empty());
}

#[test]
fn pkginfo_with_relations_emits_every_tag() {
    let depends = vec!["glibc".to_string(), "libfoo.so=1-64".to_string()];
    let optdepends = vec!["git: version control".to_string()];
    let conflicts = vec!["oldpkg".to_string()];
    let provides = vec!["virtualpkg=1.0".to_string()];
    let replaces = vec!["legacy".to_string()];
    let backup = vec!["etc/foo.conf".to_string()];
    let relations = PackageRelations {
        depends: &depends,
        optdepends: &optdepends,
        conflicts: &conflicts,
        provides: &provides,
        replaces: &replaces,
        backup: &backup,
    };
    let s = render_pkginfo_with(
        &PackageMeta {
            name: "foo",
            version: "1.0",
            release: "1",
            description: "desc",
            url: "https://ex.com",
            license: "MIT",
        },
        &relations,
        Some("Jane <jane@example.com>"),
        "2:1.0-1",
        "x86_64",
        0,
        10,
    );
    assert!(s.contains("pkgver = 2:1.0-1"), "{s}");
    assert!(s.contains("packager = Jane <jane@example.com>"), "{s}");
    assert!(s.contains("depend = glibc\n"), "{s}");
    assert!(s.contains("depend = libfoo.so=1-64\n"), "{s}");
    assert!(s.contains("optdepend = git: version control\n"), "{s}");
    assert!(s.contains("conflict = oldpkg\n"), "{s}");
    assert!(s.contains("provides = virtualpkg=1.0\n"), "{s}");
    assert!(s.contains("replaces = legacy\n"), "{s}");
    assert!(s.contains("backup = etc/foo.conf\n"), "{s}");
}

#[test]
fn build_with_relations_round_trips_pkginfo() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("usr/bin")).unwrap();
    std::fs::write(root.path().join("usr/bin/hello"), b"payload").unwrap();
    let depends = vec!["glibc".to_string()];
    let relations = PackageRelations {
        depends: &depends,
        ..Default::default()
    };
    let out = root.path().join("hello-1.0-1-x86_64.pkg.tar.zst");
    build_with_relations(
        root.path(),
        &PackageMeta {
            name: "hello",
            version: "1.0",
            release: "1",
            description: "test",
            url: "https://example.com",
            license: "MIT",
        },
        &relations,
        Some("Tester <t@example.com>"),
        None,
        "amd64",
        1_735_689_600,
        &out,
        None,
        &Default::default(),
    )
    .unwrap();

    let bytes = std::fs::read(&out).unwrap();
    let dec = zstd::stream::read::Decoder::new(bytes.as_slice()).unwrap();
    let mut ar = tar::Archive::new(dec);
    let mut pkginfo = String::new();
    for entry in ar.entries().unwrap() {
        let mut entry = entry.unwrap();
        if entry.path().unwrap().to_string_lossy() == ".PKGINFO" {
            use std::io::Read;
            entry.read_to_string(&mut pkginfo).unwrap();
        }
    }
    assert!(pkginfo.contains("depend = glibc\n"), "{pkginfo}");
    assert!(
        pkginfo.contains("packager = Tester <t@example.com>"),
        "{pkginfo}"
    );
}

#[test]
fn render_install_script_both_hooks() {
    let s = render_install_script("echo pre", "post").unwrap();
    assert!(s.contains("pre_upgrade()"));
    assert!(s.contains("echo pre"));
    assert!(s.contains("post_upgrade()"));
    assert!(s.contains("post"));
}

#[test]
fn render_install_script_pre_only() {
    let s = render_install_script("echo pre", "").unwrap();
    assert!(s.contains("pre_upgrade()"));
    assert!(s.contains("echo pre"));
    assert!(!s.contains("post_upgrade()"));
}

#[test]
fn render_install_script_none_returns_none() {
    assert!(render_install_script("", "").is_none());
    assert!(render_install_script("  ", "  ").is_none());
}

#[test]
fn build_with_install_script_includes_install_member() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("usr/bin")).unwrap();
    std::fs::write(root.path().join("usr/bin/hello"), b"payload").unwrap();
    let mut perms = std::fs::metadata(root.path().join("usr/bin/hello"))
        .unwrap()
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(root.path().join("usr/bin/hello"), perms).unwrap();

    let out = root.path().join("hello-1.0-1-x86_64.pkg.tar.zst");
    let script = render_install_script("echo pre-upgrade", "echo post-upgrade");
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
        script.as_deref(),
    )
    .unwrap();

    let bytes = std::fs::read(&out).unwrap();
    let dec = zstd::stream::read::Decoder::new(bytes.as_slice()).unwrap();
    let mut ar = tar::Archive::new(dec);
    let names: Vec<String> = ar
        .entries()
        .unwrap()
        .map(|e| e.unwrap().path().unwrap().to_string_lossy().to_string())
        .collect();
    assert!(names.contains(&".INSTALL".to_string()), "{names:?}");
    assert!(names.contains(&".PKGINFO".to_string()), "{names:?}");
}

#[test]
fn build_without_install_script_omits_install_member() {
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
            description: "test",
            url: "https://example.com",
            license: "MIT",
        },
        "amd64",
        1_735_689_600,
        &out,
        None,
    )
    .unwrap();

    let bytes = std::fs::read(&out).unwrap();
    let dec = zstd::stream::read::Decoder::new(bytes.as_slice()).unwrap();
    let mut ar = tar::Archive::new(dec);
    let names: Vec<String> = ar
        .entries()
        .unwrap()
        .map(|e| e.unwrap().path().unwrap().to_string_lossy().to_string())
        .collect();
    assert!(!names.contains(&".INSTALL".to_string()), "{names:?}");
    assert!(names.contains(&".PKGINFO".to_string()), "{names:?}");
}
