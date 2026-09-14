// SPDX-License-Identifier: GPL-3.0-or-later

use lx_lib::plugins::repo::{
    artifacts_with_ext, get_repo_indexer, repo_indexer_names, IndexOptions,
};
use std::io::Read;
use std::path::Path;

fn opts<'a>(suite: &'a str, origin: &'a str) -> IndexOptions<'a> {
    IndexOptions {
        suite,
        origin,
        components: "main",
        sign_key: None,
        sign_key_id: "",
    }
}

fn stage(root: &Path) {
    std::fs::create_dir_all(root.join("usr/bin")).unwrap();
    std::fs::write(root.join("usr/bin/hello"), b"x").unwrap();
}

fn read_tar_member(gz_bytes: &[u8], name: &str) -> Option<String> {
    let gz = flate2::read::GzDecoder::new(gz_bytes);
    let mut tar = tar::Archive::new(gz);
    for entry in tar.entries().unwrap() {
        let mut entry = entry.unwrap();
        let path = entry.path().unwrap().to_string_lossy().to_string();
        if path == name || path.ends_with(&format!("/{name}")) {
            let mut s = String::new();
            entry.read_to_string(&mut s).unwrap();
            return Some(s);
        }
    }
    None
}

#[test]
fn registry_covers_every_package_format() {
    let names = repo_indexer_names();
    for n in ["apt", "opkg", "pacman", "apk", "rpm"] {
        assert!(names.contains(&n), "missing indexer {n}");
    }
    for f in ["deb", "ipk", "arch", "apk", "rpm"] {
        assert!(get_repo_indexer(f).is_some(), "missing format {f}");
    }
    assert!(get_repo_indexer("nope").is_none());
}

#[test]
fn opkg_writes_packages_from_ipk() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    stage(&root);
    let ipk = tmp.path().join("hello_1.0-1_x86_64.ipk");
    let control = "Package: hello\nVersion: 1.0-1\nArchitecture: x86_64\nMaintainer: t <t@e>\nDepends: libc\nDescription: test\nSection: utils\n";
    lx_lib::ipkarchive::build(&root, control.as_bytes(), 1_735_689_600, &ipk).unwrap();

    let arts = artifacts_with_ext(tmp.path(), "ipk").unwrap();
    get_repo_indexer("ipk")
        .unwrap()
        .build_index(tmp.path(), &arts, &opts("openwrt", "test"))
        .unwrap();

    let packages = std::fs::read_to_string(tmp.path().join("Packages")).unwrap();
    assert!(packages.contains("Package: hello"), "{packages}");
    assert!(
        packages.contains("Filename: hello_1.0-1_x86_64.ipk"),
        "{packages}"
    );
    assert!(packages.contains("Depends: libc"), "{packages}");
    assert!(tmp.path().join("Packages.gz").exists());
}

#[test]
fn pacman_writes_a_db_tarball_from_pkg() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    stage(&root);
    let pkg = tmp.path().join("hello-1.0-1-x86_64.pkg.tar.zst");
    let meta = lx_lib::archarchive::PackageMeta {
        name: "hello",
        version: "1.0",
        release: "1",
        description: "test pkg",
        url: "https://example.com",
        license: "MIT",
    };
    lx_lib::archarchive::build(&root, &meta, "x86_64", 1_735_689_600, &pkg, None).unwrap();

    let arts = artifacts_with_ext(tmp.path(), "zst").unwrap();
    get_repo_indexer("arch")
        .unwrap()
        .build_index(tmp.path(), &arts, &opts("core", "test"))
        .unwrap();

    let db = std::fs::read(tmp.path().join("core.db.tar.gz")).unwrap();
    let desc = read_tar_member(&db, "desc").expect("no desc member");
    assert!(desc.contains("%NAME%\nhello"), "{desc}");
    assert!(desc.contains("%VERSION%\n1.0-1"), "{desc}");
    assert!(desc.contains("%SHA256SUM%"), "{desc}");
}

#[test]
fn apk_writes_an_apkindex_tarball_from_apk() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    stage(&root);
    let apk = tmp.path().join("hello-1.0.0-r1.apk");
    let meta = lx_lib::apkarchive::PackageMeta {
        name: "hello",
        version: "1.0.0-r1",
        description: "test pkg",
        url: "https://example.com",
        license: "MIT",
        depends: &[],
        provides: &[],
        replaces: &[],
    };
    lx_lib::apkarchive::build(&root, &meta, "x86_64", 1_735_689_600, &apk).unwrap();

    let arts = artifacts_with_ext(tmp.path(), "apk").unwrap();
    get_repo_indexer("apk")
        .unwrap()
        .build_index(tmp.path(), &arts, &opts("alpine", "test"))
        .unwrap();

    let index_tar = std::fs::read(tmp.path().join("APKINDEX.tar.gz")).unwrap();
    let index = read_tar_member(&index_tar, "APKINDEX").expect("no APKINDEX member");
    assert!(index.contains("P:hello"), "{index}");
    assert!(index.contains("V:1.0.0-r1"), "{index}");
    assert!(index.contains("A:x86_64"), "{index}");
}
