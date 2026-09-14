// SPDX-License-Identifier: GPL-3.0-or-later

//! `osxpkg` plugin: builds a xar flat package with a cpio payload.

use lx_lib::config::{PackageConfig, Scripts};
use lx_lib::plugins::{get_packager, BuildContext};

fn fake_elf() -> Vec<u8> {
    let mut bytes = vec![0x7f, b'E', b'L', b'F', 2, 1, 1, 0];
    bytes.extend_from_slice(&[0u8; 120]);
    bytes
}

fn zlib_decode(data: &[u8]) -> Vec<u8> {
    use std::io::Read;
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(data)
        .read_to_end(&mut out)
        .unwrap();
    out
}

fn gzip_decode(data: &[u8]) -> Vec<u8> {
    use std::io::Read;
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(data)
        .read_to_end(&mut out)
        .unwrap();
    out
}

#[test]
fn osxpkg_builds_a_xar_with_cpio_payload() {
    let tmp = tempfile::tempdir().unwrap();
    let binary_dir = tmp.path().join("binary");
    std::fs::create_dir_all(&binary_dir).unwrap();
    std::fs::write(binary_dir.join("mytool"), fake_elf()).unwrap();

    let script = tmp.path().join("postinstall.sh");
    std::fs::write(&script, "#!/bin/sh\necho installed\n").unwrap();

    let root = tmp.path().join("root");
    std::fs::create_dir_all(&root).unwrap();

    let cfg = PackageConfig {
        package_name: "com.example.mytool".into(),
        github_repo: "owner/mytool".into(),
        description: "test".into(),
        maintainer: "t <t@example.com>".into(),
        license_spdx: "MIT".into(),
        package_format: "osxpkg".into(),
        version: "1.2.3".into(),
        scripts: Scripts {
            postinstall: script.to_string_lossy().to_string(),
            ..Default::default()
        },
        ..Default::default()
    };
    let job = lx_lib::build::ResolvedJob {
        dist: "macos".into(),
        arch: "amd64".into(),
        asset: lx_lib::github::Asset {
            name: "mytool.tar.gz".into(),
            size: None,
            browser_download_url: "".into(),
            checksums: Default::default(),
        },
        tag: "v1.2.3".into(),
        published_at: Some(1_735_689_600),
    };
    let ctx = BuildContext {
        cfg: &cfg,
        job: &job,
        binary_dir: &binary_dir,
        staging_root: &root,
        license: None,
        debian_version: "1.2.3",
        build_version: "1",
        mtime: 1_735_689_600,
        sign_key: None,
        sign_key_id: "",
        sign_passphrase: None,
        sign_method: "detach",
        detected_deps: Vec::new(),
    };
    let out = get_packager("osxpkg").unwrap().build(&ctx).unwrap();
    assert_eq!(out.extension().unwrap(), "pkg");
    // `pkg` is an alias for the same packager.
    assert!(get_packager("pkg").is_some());

    let bytes = std::fs::read(&out).unwrap();
    assert_eq!(&bytes[0..4], b"xar!", "xar magic missing");
    assert_eq!(u16::from_be_bytes([bytes[4], bytes[5]]), 28, "header size");

    let toc_z_len = u64::from_be_bytes(bytes[8..16].try_into().unwrap()) as usize;
    let toc = String::from_utf8(zlib_decode(&bytes[28..28 + toc_z_len])).unwrap();
    assert!(toc.contains("<name>PackageInfo</name>"), "{toc}");
    assert!(toc.contains("<name>Payload</name>"), "{toc}");
    assert!(toc.contains("application/x-gzip"), "{toc}");
    assert!(toc.contains("<name>Scripts</name>"), "{toc}");

    let heap = &bytes[28 + toc_z_len..];

    // PackageInfo metadata (stored uncompressed at its heap offset).
    let pi_pos = toc.find("<name>PackageInfo</name>").unwrap();
    let pi_rest = &toc[pi_pos..];
    let pi_off: usize = extract_tag(pi_rest, "<offset>", "</offset>")
        .parse()
        .unwrap();
    let pi_size: usize = extract_tag(pi_rest, "<size>", "</size>").parse().unwrap();
    let package_info = String::from_utf8(heap[pi_off..pi_off + pi_size].to_vec()).unwrap();
    assert!(
        package_info.contains("identifier=\"com.example.mytool\""),
        "{package_info}"
    );
    assert!(
        package_info.contains("<postinstall file=\"./postinstall\"/>"),
        "{package_info}"
    );

    // Locate the Payload member in the heap and decode its gzip + cpio.
    let payload_pos = toc.find("<name>Payload</name>").unwrap();
    let rest = &toc[payload_pos..];
    let offset: usize = extract_tag(rest, "<offset>", "</offset>").parse().unwrap();
    // `<length>` is the stored (gzipped) size; `<size>` is the extracted size.
    let size: usize = extract_tag(rest, "<length>", "</length>").parse().unwrap();
    let payload = gzip_decode(&heap[offset..offset + size]);
    assert!(payload.starts_with(b"070701"), "payload is not newc cpio");
    assert!(
        payload
            .windows(b"./usr/bin/mytool".len())
            .any(|w| w == b"./usr/bin/mytool"),
        "payload missing the staged file"
    );
}

fn extract_tag(s: &str, open: &str, close: &str) -> String {
    let start = s.find(open).unwrap() + open.len();
    let end = s[start..].find(close).unwrap() + start;
    s[start..end].to_string()
}

/// Native `.pkg` signing adds a xar `Signature` member over the archive
/// checksum. Gated on `openssl` (throwaway self-signed cert + key).
#[test]
fn osxpkg_native_signing_adds_a_xar_signature() {
    use lx_lib::plugins::signer::{signer_for, SignContext, SignOutcome};
    use std::process::{Command, Stdio};

    let openssl_ok = Command::new("openssl")
        .arg("version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !openssl_ok {
        eprintln!("skipping: openssl not available");
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    let key = tmp.path().join("key.pem");
    let cert = tmp.path().join("cert.pem");
    let status = Command::new("openssl")
        .args(["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout"])
        .arg(&key)
        .arg("-out")
        .arg(&cert)
        .args(["-days", "1", "-subj", "/CN=lx-test"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "openssl failed to mint a test cert");

    let binary_dir = tmp.path().join("binary");
    std::fs::create_dir_all(&binary_dir).unwrap();
    std::fs::write(binary_dir.join("mytool"), fake_elf()).unwrap();
    let root = tmp.path().join("root");
    std::fs::create_dir_all(&root).unwrap();

    let cfg = PackageConfig {
        package_name: "com.example.mytool".into(),
        github_repo: "owner/mytool".into(),
        description: "test".into(),
        maintainer: "t <t@example.com>".into(),
        license_spdx: "MIT".into(),
        package_format: "osxpkg".into(),
        ..Default::default()
    };
    let job = lx_lib::build::ResolvedJob {
        dist: "macos".into(),
        arch: "amd64".into(),
        asset: lx_lib::github::Asset {
            name: "mytool.tar.gz".into(),
            size: None,
            browser_download_url: "".into(),
            checksums: Default::default(),
        },
        tag: "v1.2.3".into(),
        published_at: Some(1_735_689_600),
    };
    let ctx = BuildContext {
        cfg: &cfg,
        job: &job,
        binary_dir: &binary_dir,
        staging_root: &root,
        license: None,
        debian_version: "1.2.3",
        build_version: "1",
        mtime: 1_735_689_600,
        sign_key: None,
        sign_key_id: "",
        sign_passphrase: None,
        sign_method: "detach",
        detected_deps: Vec::new(),
    };
    let out = get_packager("osxpkg").unwrap().build(&ctx).unwrap();

    let cert_str = cert.to_string_lossy().to_string();
    let sign_ctx = SignContext {
        key_file: &key,
        key_id: "",
        passphrase: None,
        sign_type: "origin",
        cert_file: &cert_str,
    };
    let signer = signer_for("osxpkg", "detach").expect("pkg signer registered");
    assert!(matches!(
        signer.sign(&out, &sign_ctx).unwrap(),
        SignOutcome::Embedded
    ));

    // The signed xar's TOC carries a `Signature` (with the embedded cert).
    let bytes = std::fs::read(&out).unwrap();
    assert_eq!(&bytes[0..4], b"xar!");
    let toc_z_len = u64::from_be_bytes(bytes[8..16].try_into().unwrap()) as usize;
    let toc = String::from_utf8(zlib_decode(&bytes[28..28 + toc_z_len])).unwrap();
    assert!(toc.contains("<signature"), "no xar signature in TOC: {toc}");
    assert!(
        toc.contains("X509Certificate"),
        "signature should embed the certificate: {toc}"
    );
}
