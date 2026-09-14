// SPDX-License-Identifier: GPL-3.0-or-later

use lx_lib::plugins::artifact::{
    artifact_format_names, detect_artifact_format, extract, get_artifact_format,
};
use std::path::Path;

fn tar_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut tar_bytes = Vec::new();
    {
        let mut b = tar::Builder::new(&mut tar_bytes);
        for (name, data) in entries {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_mtime(0);
            h.set_cksum();
            b.append_data(&mut h, name, *data).unwrap();
        }
        b.finish().unwrap();
    }
    tar_bytes
}

fn write_tar_gz(path: &Path, entries: &[(&str, &[u8])]) {
    let gz = lx_lib::debarchive::deterministic_gzip_bytes(&tar_bytes(entries), 0, 9).unwrap();
    std::fs::write(path, gz).unwrap();
}

fn write_tar_zst(path: &Path, entries: &[(&str, &[u8])]) {
    let zst = zstd::bulk::compress(&tar_bytes(entries), 1).unwrap();
    std::fs::write(path, zst).unwrap();
}

fn write_tar_xz(path: &Path, entries: &[(&str, &[u8])]) {
    use std::io::Write;
    let mut w =
        lzma_rust2::XzWriter::new(Vec::new(), lzma_rust2::XzOptions::with_preset(1)).unwrap();
    w.write_all(&tar_bytes(entries)).unwrap();
    std::fs::write(path, w.finish().unwrap()).unwrap();
}

#[test]
fn registry_lists_all_formats_and_aliases() {
    let names = artifact_format_names();
    for expected in ["tar.gz", "tar.xz", "tar.zst", "tar", "zip", "raw"] {
        assert!(names.contains(&expected), "missing format {expected}");
        assert!(get_artifact_format(expected).is_some());
    }
    assert!(get_artifact_format("tgz").is_some(), "alias tgz");
    assert!(
        get_artifact_format("TXZ").is_some(),
        "case-insensitive alias"
    );
    assert!(get_artifact_format("nope").is_none());
}

#[test]
fn detects_by_file_name() {
    assert_eq!(detect_artifact_format("eza.tar.gz"), "tar.gz");
    assert_eq!(detect_artifact_format("eza.tgz"), "tar.gz");
    assert_eq!(detect_artifact_format("eza.tar.xz"), "tar.xz");
    assert_eq!(detect_artifact_format("eza.tar.zst"), "tar.zst");
    assert_eq!(detect_artifact_format("eza.zip"), "zip");
    assert_eq!(detect_artifact_format("eza"), "raw");
    assert_eq!(detect_artifact_format("eza-linux-amd64"), "raw");
}

#[test]
fn extracts_tar_gz() {
    let tmp = tempfile::tempdir().unwrap();
    let archive = tmp.path().join("x.tar.gz");
    write_tar_gz(&archive, &[("bin/hello", b"hi"), ("README", b"readme")]);
    let dest = tmp.path().join("out");
    extract(&archive, &dest, "tar.gz").unwrap();
    assert_eq!(std::fs::read(dest.join("bin/hello")).unwrap(), b"hi");
    assert_eq!(std::fs::read(dest.join("README")).unwrap(), b"readme");
}

#[test]
fn extracts_tar_zst_and_tar_xz() {
    let tmp = tempfile::tempdir().unwrap();
    let zst = tmp.path().join("x.tar.zst");
    write_tar_zst(&zst, &[("a", b"zst")]);
    let dz = tmp.path().join("zst");
    extract(&zst, &dz, "tar.zst").unwrap();
    assert_eq!(std::fs::read(dz.join("a")).unwrap(), b"zst");

    let xz = tmp.path().join("x.tar.xz");
    write_tar_xz(&xz, &[("b", b"xz")]);
    let dx = tmp.path().join("xz");
    extract(&xz, &dx, "tar.xz").unwrap();
    assert_eq!(std::fs::read(dx.join("b")).unwrap(), b"xz");
}

#[test]
fn raw_copies_the_asset_under_its_own_name() {
    let tmp = tempfile::tempdir().unwrap();
    let asset = tmp.path().join("hello");
    std::fs::write(&asset, b"\x7fELF").unwrap();
    let dest = tmp.path().join("payload");
    extract(&asset, &dest, "raw").unwrap();
    assert_eq!(std::fs::read(dest.join("hello")).unwrap(), b"\x7fELF");
}

#[test]
fn zip_is_recognized_but_reports_an_actionable_error() {
    let tmp = tempfile::tempdir().unwrap();
    let zip = tmp.path().join("x.zip");
    std::fs::write(&zip, b"PK\x03\x04").unwrap();
    let err = extract(&zip, &tmp.path().join("out"), "zip").unwrap_err();
    assert!(
        err.to_string()
            .contains("zip extraction is not implemented"),
        "{err}"
    );
}

#[test]
fn unknown_format_errors_with_the_available_list() {
    let tmp = tempfile::tempdir().unwrap();
    let err = extract(&tmp.path().join("x"), &tmp.path().join("o"), "rar").unwrap_err();
    assert!(
        err.to_string()
            .contains("unsupported artifact_format 'rar'"),
        "{err}"
    );
}
