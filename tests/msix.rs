// SPDX-License-Identifier: GPL-3.0-or-later

//! MSIX plugin: builds an unsigned OPC container and rejects unsupported
//! signing.

use lx_lib::config::{MsixApplication, MsixConfig, MsixProperties, PackageConfig};
use lx_lib::plugins::{get_packager, BuildContext};

fn fake_elf() -> Vec<u8> {
    let mut bytes = vec![0x7f, b'E', b'L', b'F', 2, 1, 1, 0];
    bytes.extend_from_slice(&[0u8; 120]);
    bytes
}

fn build_msix(
    cfg: &PackageConfig,
    root: &std::path::Path,
    binary_dir: &std::path::Path,
) -> Vec<u8> {
    let job = lx_lib::build::ResolvedJob {
        dist: "windows".into(),
        arch: "amd64".into(),
        asset: lx_lib::github::Asset {
            name: "hello.zip".into(),
            size: None,
            browser_download_url: "".into(),
            checksums: Default::default(),
        },
        tag: "v1.0.0".into(),
        published_at: Some(1_735_689_600),
    };
    let key_buf = std::path::PathBuf::from(&cfg.signature.key_file);
    let sign_key = if cfg.signature.key_file.trim().is_empty() {
        None
    } else {
        Some(key_buf.as_path())
    };
    let ctx = BuildContext {
        cfg,
        job: &job,
        binary_dir,
        staging_root: root,
        license: None,
        debian_version: "1.2.3",
        build_version: "1",
        mtime: 1_735_689_600,
        sign_key,
        sign_key_id: "",
        sign_passphrase: None,
        sign_method: "detach",
        detected_deps: Vec::new(),
    };
    let out = get_packager("msix").unwrap().build(&ctx).unwrap();
    assert_eq!(out.extension().unwrap(), "msix");
    std::fs::read(&out).unwrap()
}

fn base_config() -> PackageConfig {
    PackageConfig {
        package_name: "hello".into(),
        github_repo: "owner/hello".into(),
        description: "A test app".into(),
        maintainer: "t <t@example.com>".into(),
        license_spdx: "MIT".into(),
        package_format: "msix".into(),
        msix: MsixConfig {
            publisher: "CN=Acme, O=Acme, C=US".into(),
            properties: MsixProperties {
                logo: "Assets/Square150x150Logo.png".into(),
                ..Default::default()
            },
            applications: vec![MsixApplication {
                id: "Hello".into(),
                executable: "VFS/usr/bin/hello".into(),
                ..Default::default()
            }],
            ..Default::default()
        },
        ..Default::default()
    }
}

#[test]
fn msix_plugin_builds_opc_container() {
    let tmp = tempfile::tempdir().unwrap();
    let binary_dir = tmp.path().join("binary");
    std::fs::create_dir_all(&binary_dir).unwrap();
    std::fs::write(binary_dir.join("hello"), fake_elf()).unwrap();

    let root = tmp.path().join("root");
    std::fs::create_dir_all(&root).unwrap();

    let cfg = base_config();
    let bytes = build_msix(&cfg, &root, &binary_dir);

    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("valid zip");
    let mut manifest = String::new();
    let mut names = Vec::new();
    for i in 0..zip.len() {
        let mut file = zip.by_index(i).unwrap();
        names.push(file.name().to_string());
        if file.name() == "AppxManifest.xml" {
            std::io::Read::read_to_string(&mut file, &mut manifest).unwrap();
        }
    }

    assert!(
        names.iter().any(|n| n == "[Content_Types].xml"),
        "{names:?}"
    );
    assert!(names.iter().any(|n| n == "AppxManifest.xml"), "{names:?}");
    assert!(names.iter().any(|n| n == "AppxBlockMap.xml"), "{names:?}");

    // Manifest identity + version normalization (1.2.3 -> 1.2.3.0).
    assert!(manifest.contains("Name=\"hello\""), "{manifest}");
    assert!(manifest.contains("Version=\"1.2.3.0\""), "{manifest}");
    assert!(
        manifest.contains("ProcessorArchitecture=\"x64\""),
        "{manifest}"
    );
    assert!(
        manifest.contains("Publisher=\"CN=Acme, O=Acme, C=US\""),
        "{manifest}"
    );
    // Full-trust entry point auto-adds the restricted capability.
    assert!(manifest.contains("runFullTrust"), "{manifest}");
}

#[test]
fn msix_rejects_unsupported_signing() {
    let tmp = tempfile::tempdir().unwrap();
    let binary_dir = tmp.path().join("binary");
    std::fs::create_dir_all(&binary_dir).unwrap();
    std::fs::write(binary_dir.join("hello"), fake_elf()).unwrap();
    let root = tmp.path().join("root");
    std::fs::create_dir_all(&root).unwrap();

    let mut cfg = base_config();
    cfg.msix.signature.pfx_file = "cert.pfx".into();
    let job = lx_lib::build::ResolvedJob {
        dist: "windows".into(),
        arch: "amd64".into(),
        asset: lx_lib::github::Asset {
            name: "hello.zip".into(),
            size: None,
            browser_download_url: "".into(),
            checksums: Default::default(),
        },
        tag: "v1.0.0".into(),
        published_at: Some(1_735_689_600),
    };
    let ctx = BuildContext {
        cfg: &cfg,
        job: &job,
        binary_dir: &binary_dir,
        staging_root: &root,
        license: None,
        debian_version: "1.0.0",
        build_version: "1",
        mtime: 1_735_689_600,
        sign_key: None,
        sign_key_id: "",
        sign_passphrase: None,
        sign_method: "detach",
        detected_deps: Vec::new(),
    };
    let err = get_packager("msix").unwrap().build(&ctx).unwrap_err();
    assert!(err.to_string().contains("signing"), "{err}");
}

#[test]
fn msix_config_parses() {
    let yaml = r#"
package_name: hello
github_repo: owner/hello
package_format: msix
msix:
  publisher: "CN=Acme, O=Acme, C=US"
  properties:
    display_name: Hello
    logo: Assets/logo.png
  applications:
    - id: Hello
      executable: VFS/usr/bin/hello
      visual_elements:
        display_name: Hello
        description: A test app
  capabilities:
    restricted: [runFullTrust]
"#;
    let cfg = PackageConfig::parse_str(yaml).expect("msix config should parse");
    assert_eq!(cfg.msix.publisher, "CN=Acme, O=Acme, C=US");
    assert_eq!(cfg.msix.applications.len(), 1);
    assert_eq!(cfg.msix.applications[0].id, "Hello");
}

/// MSIX native signing embeds a parseable `AppxSignature.p7x`. Gated on
/// `openssl` (used only to mint a throwaway self-signed cert + key).
#[test]
fn msix_native_signing_embeds_appx_signature() {
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
    std::fs::write(binary_dir.join("hello"), fake_elf()).unwrap();
    let root = tmp.path().join("root");
    std::fs::create_dir_all(&root).unwrap();

    let mut cfg = base_config();
    cfg.signature.key_file = key.to_string_lossy().to_string();
    cfg.signature.cert_file = cert.to_string_lossy().to_string();
    let bytes = build_msix(&cfg, &root, &binary_dir);

    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes.clone())).expect("valid zip");
    assert!(
        zip.by_name("AppxSignature.p7x").is_ok(),
        "AppxSignature.p7x member missing"
    );
    drop(zip);

    // The same crate that produced the format must be able to read it back.
    let signed_path = tmp.path().join("signed.msix");
    std::fs::write(&signed_path, &bytes).unwrap();
    msix::p7x::read_p7x(&signed_path).expect("p7x should decode as PKCS#7 SignedData");
}
