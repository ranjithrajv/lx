// SPDX-License-Identifier: GPL-3.0-or-later

use lx_lib::apkarchive::{build, render_pkginfo, PackageMeta};
use std::path::Path;

fn make_stage(tmp: &Path) -> std::path::PathBuf {
    let root = tmp.join("root");
    std::fs::create_dir_all(root.join("usr/bin")).unwrap();
    std::fs::write(root.join("usr/bin/hello"), b"#!/bin/sh\necho hi\n").unwrap();
    std::fs::create_dir_all(root.join("usr/share/doc/hello")).unwrap();
    std::fs::write(root.join("usr/share/doc/hello/LICENSE"), b"MIT").unwrap();
    root
}

fn gunzip_first_member(bytes: &[u8]) -> Vec<u8> {
    use std::io::Read;
    let mut decoder = flate2::read::GzDecoder::new(bytes);
    let mut out = Vec::new();
    decoder.read_to_end(&mut out).unwrap();
    out
}

fn read_tar_member(tar_bytes: &[u8], name: &str) -> Option<Vec<u8>> {
    let mut archive = tar::Archive::new(tar_bytes);
    for entry in archive.entries().unwrap() {
        let mut entry = entry.unwrap();
        let path = entry.path().unwrap().to_string_lossy().to_string();
        if path == name {
            let mut data = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut data).unwrap();
            return Some(data);
        }
    }
    None
}

#[test]
fn apk_control_member_carries_pkginfo_and_datahash() {
    let tmp = tempfile::tempdir().unwrap();
    let root = make_stage(tmp.path());
    let out = tmp.path().join("hello-1.0.0-r1.apk");

    let depends = vec!["musl".to_string(), "busybox".to_string()];
    let provides = vec!["hello-cli".to_string()];
    let replaces = vec!["hello-old".to_string()];
    let meta = PackageMeta {
        name: "hello",
        version: "1.0.0-r1",
        description: "test package",
        url: "https://example.com/hello",
        license: "MIT",
        depends: &depends,
        provides: &provides,
        replaces: &replaces,
    };
    build(&root, &meta, "x86_64", 1_735_689_600, &out).unwrap();

    let bytes = std::fs::read(&out).unwrap();
    assert_eq!(
        &bytes[0..2],
        &[0x1F, 0x8B],
        "apk must start with a gzip member"
    );

    // First gzip member is the control tar holding .PKGINFO.
    let control = gunzip_first_member(&bytes);
    let pkginfo = read_tar_member(&control, ".PKGINFO").expect("no .PKGINFO in control member");
    let text = String::from_utf8(pkginfo).unwrap();
    assert!(text.contains("pkgname = hello"), "{text}");
    assert!(text.contains("pkgver = 1.0.0-r1"), "{text}");
    assert!(text.contains("arch = x86_64"), "{text}");
    assert!(text.contains("depend = musl"), "{text}");
    assert!(text.contains("depend = busybox"), "{text}");
    assert!(text.contains("provides = hello-cli"), "{text}");
    assert!(text.contains("replaces = hello-old"), "{text}");
    // datahash pins the compressed data member's SHA-256.
    let datahash = text
        .lines()
        .find_map(|l| l.strip_prefix("datahash = "))
        .expect("no datahash");
    assert_eq!(datahash.len(), 64, "datahash must be a sha256 hex digest");
}

#[test]
fn apk_build_is_deterministic() {
    let tmp = tempfile::tempdir().unwrap();
    let root = make_stage(tmp.path());
    let meta = PackageMeta {
        name: "hello",
        version: "1.0.0-r1",
        description: "test",
        url: "https://example.com",
        license: "MIT",
        depends: &[],
        provides: &[],
        replaces: &[],
    };
    let a = tmp.path().join("a.apk");
    let b = tmp.path().join("b.apk");
    build(&root, &meta, "x86_64", 1_735_689_600, &a).unwrap();
    build(&root, &meta, "x86_64", 1_735_689_600, &b).unwrap();
    assert_eq!(std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap());
}

#[test]
fn render_pkginfo_defaults_and_fields() {
    let text = render_pkginfo(
        &PackageMeta {
            name: "foo",
            version: "2.0.0-r3",
            description: "",
            url: "",
            license: "",
            depends: &[],
            provides: &[],
            replaces: &[],
        },
        "aarch64",
        0,
        42,
        "deadbeef",
    );
    assert!(text.contains("pkgname = foo"));
    assert!(text.contains("pkgdesc = No description"));
    assert!(text.contains("license = unknown"));
    assert!(text.contains("arch = aarch64"));
    assert!(text.contains("size = 42"));
    assert!(text.contains("datahash = deadbeef"));
    // Empty depends/provides/replaces emit no lines.
    assert!(!text.contains("depend = "));
}
