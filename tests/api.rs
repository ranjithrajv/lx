// SPDX-License-Identifier: GPL-3.0-or-later

//! Public embedding API (`lx_lib::api`).

use lx_lib::api::{convert_nfpm, package, parse_config, PackageRequest};
use lx_lib::config::PackageConfig;

fn fake_elf() -> Vec<u8> {
    let mut bytes = vec![0x7f, b'E', b'L', b'F', 2, 1, 1, 0];
    bytes.extend_from_slice(&[0u8; 120]);
    bytes
}

#[test]
fn embeds_a_deb_build_from_a_payload() {
    let tmp = tempfile::tempdir().unwrap();
    let payload = tmp.path().join("payload");
    std::fs::create_dir_all(&payload).unwrap();
    std::fs::write(payload.join("mytool"), fake_elf()).unwrap();

    let staging = tmp.path().join("stage");
    let output = tmp.path().join("dist");

    let cfg = PackageConfig {
        package_name: "mytool".into(),
        github_repo: "owner/mytool".into(),
        version: "1.0.0".into(),
        description: "embedded".into(),
        maintainer: "t <t@example.com>".into(),
        license_spdx: "MIT".into(),
        package_format: "deb".into(),
        ..Default::default()
    };

    let out = package(PackageRequest {
        config: &cfg,
        binary_dir: &payload,
        staging_root: &staging,
        output: &output,
        dist: "trixie".into(),
        arch: "amd64".into(),
        tag: "v1.0.0".into(),
        published_at: Some(1_735_689_600),
        debian_version: "1.0.0".into(),
        build_version: "1".into(),
        mtime: lx_lib::pkgmeta::reproducible_epoch(None),
        format: None,
        license: None,
        detected_deps: Vec::new(),
    })
    .expect("package should build");

    assert!(out.exists());
    assert_eq!(out.extension().unwrap(), "deb");
    let bytes = std::fs::read(&out).unwrap();
    assert!(bytes.starts_with(b"!<arch>\n"), "not a .deb ar container");
}

#[test]
fn api_config_and_nfpm_helpers() {
    let cfg =
        parse_config("package_name: x\ngithub_repo: o/x\nversion: 1.0.0\nlicense_spdx: MIT\n")
            .expect("parse");
    assert_eq!(cfg.package_name, "x");

    let converted = convert_nfpm("name: x\nversion: 1.0.0\n").expect("convert");
    assert!(converted.contains("package_name: x"));
    assert!(converted.contains("REVIEW ME"));
}
