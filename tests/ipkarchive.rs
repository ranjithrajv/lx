// SPDX-License-Identifier: GPL-3.0-or-later

use lx_lib::ipkarchive::build;
use std::path::Path;

/// Pull an ar member's bytes out of an .ipk.
fn ar_member(path: &Path, wanted: &str) -> Option<Vec<u8>> {
    let bytes = std::fs::read(path).unwrap();
    let mut archive = ar::Archive::new(bytes.as_slice());
    while let Some(entry) = archive.next_entry() {
        let mut entry = entry.unwrap();
        let name = String::from_utf8_lossy(entry.header().identifier()).to_string();
        if name == wanted {
            let mut data = Vec::new();
            std::io::copy(&mut entry, &mut data).unwrap();
            return Some(data);
        }
    }
    None
}

fn gunzip(bytes: &[u8]) -> Vec<u8> {
    use std::io::Read;
    let mut decoder = flate2::read::GzDecoder::new(bytes);
    let mut out = Vec::new();
    decoder.read_to_end(&mut out).unwrap();
    out
}

fn tar_file(tar_bytes: &[u8], name: &str) -> Option<Vec<u8>> {
    let mut archive = tar::Archive::new(tar_bytes);
    for entry in archive.entries().unwrap() {
        let mut entry = entry.unwrap();
        let path = entry.path().unwrap().to_string_lossy().to_string();
        if path.trim_start_matches("./") == name {
            let mut data = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut data).unwrap();
            return Some(data);
        }
    }
    None
}

#[test]
fn ipk_is_ar_with_control_and_data_members() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    std::fs::create_dir_all(root.join("usr/bin")).unwrap();
    std::fs::write(root.join("usr/bin/hello"), b"payload").unwrap();
    let out = tmp.path().join("hello_1.0.0-1_x86_64.ipk");

    let control = "Package: hello\nVersion: 1.0.0-1\nArchitecture: x86_64\n";
    build(&root, control.as_bytes(), 1_735_689_600, &out).unwrap();

    let bytes = std::fs::read(&out).unwrap();
    assert!(bytes.starts_with(b"!<arch>\n"), "ipk must be an ar archive");

    assert!(ar_member(&out, "debian-binary").is_some());
    let control_gz = ar_member(&out, "control.tar.gz").expect("no control.tar.gz");
    let control_tar = gunzip(&control_gz);
    let control_file = tar_file(&control_tar, "control").expect("no control member");
    let text = String::from_utf8(control_file).unwrap();
    assert!(text.contains("Package: hello"), "{text}");
    assert!(text.contains("Architecture: x86_64"), "{text}");

    let data_gz = ar_member(&out, "data.tar.gz").expect("no data.tar.gz");
    let data_tar = gunzip(&data_gz);
    assert!(
        tar_file(&data_tar, "usr/bin/hello").is_some(),
        "payload missing from data member"
    );
}
