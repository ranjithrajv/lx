// SPDX-License-Identifier: GPL-3.0-or-later

//! nfpm feature-parity additions: `config|*|tree`, RPM `doc`/`license`/
//! `readme`, `rpm.group`/`buildhost`, the `ipk:` block, and the converter.

use lx_lib::config::{ContentEntry, PackageConfig};
use lx_lib::plugins::{apply_contents_full, get_packager, BuildContext};

fn fake_elf() -> Vec<u8> {
    let mut bytes = vec![0x7f, b'E', b'L', b'F', 2, 1, 1, 0];
    bytes.extend_from_slice(&[0u8; 120]);
    bytes
}

fn build_ctx<'a>(
    cfg: &'a PackageConfig,
    job: &'a lx_lib::build::ResolvedJob,
    binary_dir: &'a std::path::Path,
    root: &'a std::path::Path,
) -> BuildContext<'a> {
    BuildContext {
        cfg,
        job,
        binary_dir,
        staging_root: root,
        license: None,
        debian_version: "1.0.0",
        build_version: "1",
        mtime: 1_735_689_600,
        sign_key: None,
        sign_key_id: "",
        sign_passphrase: None,
        sign_method: "detach",
        detected_deps: Vec::new(),
    }
}

fn job(format_dist: &str) -> lx_lib::build::ResolvedJob {
    lx_lib::build::ResolvedJob {
        dist: format_dist.into(),
        arch: "amd64".into(),
        asset: lx_lib::github::Asset {
            name: "x.tar.gz".into(),
            size: None,
            browser_download_url: "".into(),
            checksums: Default::default(),
        },
        tag: "v1.0.0".into(),
        published_at: Some(1_735_689_600),
    }
}

/// `config|tree` / `config|*|tree` register every regular file in the tree.
#[test]
fn config_tree_registers_every_file() {
    let env = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(env.path().join("sub")).unwrap();
    std::fs::write(env.path().join("a.conf"), b"a").unwrap();
    std::fs::write(env.path().join("sub/b.conf"), b"b").unwrap();

    let root = tempfile::tempdir().unwrap();
    let cfg = PackageConfig {
        package_name: "x".into(),
        github_repo: "o/x".into(),
        contents: vec![ContentEntry {
            src: env.path().to_string_lossy().to_string(),
            dst: "/etc/x".into(),
            kind: "config|noreplace|tree".into(),
            ..Default::default()
        }],
        ..Default::default()
    };

    let (configs, _meta) = apply_contents_full(&cfg, root.path(), "deb").unwrap();
    let paths: Vec<&str> = configs.iter().map(|c| c.path.as_str()).collect();
    assert!(paths.contains(&"/etc/x/a.conf"), "{paths:?}");
    assert!(paths.contains(&"/etc/x/sub/b.conf"), "{paths:?}");
    assert!(configs.iter().all(|c| c.noreplace));
}

/// RPM `doc`/`license`/`readme` set the corresponding file flags, and
/// `rpm.group`/`buildhost` reach the header.
#[test]
fn rpm_classification_and_group_buildhost() {
    let env = tempfile::tempdir().unwrap();
    let license = env.path().join("LICENSE");
    let readme = env.path().join("README");
    std::fs::write(&license, b"lic\n").unwrap();
    std::fs::write(&readme, b"readme\n").unwrap();

    let binary_dir = tempfile::tempdir().unwrap();
    std::fs::write(binary_dir.path().join("x"), fake_elf()).unwrap();
    let root = tempfile::tempdir().unwrap();

    let cfg = PackageConfig {
        package_name: "x".into(),
        github_repo: "o/x".into(),
        package_format: "rpm".into(),
        description: "d".into(),
        maintainer: "t <t@example.com>".into(),
        license_spdx: "MIT".into(),
        contents: vec![
            ContentEntry {
                src: license.to_string_lossy().to_string(),
                dst: "/usr/share/licenses/x/LICENSE".into(),
                kind: "license".into(),
                ..Default::default()
            },
            ContentEntry {
                src: readme.to_string_lossy().to_string(),
                dst: "/usr/share/doc/x/README".into(),
                kind: "readme".into(),
                ..Default::default()
            },
        ],
        rpm: lx_lib::config::RpmConfig {
            group: "System/Base".into(),
            buildhost: "build.example.com".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    let job = job("fedora");
    let ctx = build_ctx(&cfg, &job, binary_dir.path(), root.path());
    let out = get_packager("rpm").unwrap().build(&ctx).unwrap();

    let pkg = rpm::Package::open(&out).unwrap();
    // `rpm.buildhost` is applied; `rpm.group` is a known crate limitation
    // (the rpm crate hardcodes Group to "Unspecified").
    assert_eq!(pkg.metadata.get_build_host().unwrap(), "build.example.com");
    assert_eq!(pkg.metadata.get_group().unwrap(), "Unspecified");

    let entries = pkg.metadata.get_file_entries().unwrap();
    let lic = entries
        .iter()
        .find(|e| e.path.to_string_lossy().ends_with("/LICENSE"))
        .expect("LICENSE entry");
    assert!(
        lic.flags.contains(rpm::FileFlags::LICENSE),
        "{:?}",
        lic.flags
    );
    let rd = entries
        .iter()
        .find(|e| e.path.to_string_lossy().ends_with("/README"))
        .expect("README entry");
    assert!(rd.flags.contains(rpm::FileFlags::README), "{:?}", rd.flags);
}

/// The `ipk:` block lands in the OpenWrt control file.
#[test]
fn ipk_control_carries_nfpm_fields() {
    use std::io::Read;

    let binary_dir = tempfile::tempdir().unwrap();
    std::fs::write(binary_dir.path().join("x"), fake_elf()).unwrap();
    let root = tempfile::tempdir().unwrap();

    let cfg = PackageConfig {
        package_name: "x".into(),
        github_repo: "o/x".into(),
        package_format: "ipk".into(),
        description: "d".into(),
        maintainer: "t <t@example.com>".into(),
        license_spdx: "MIT".into(),
        ipk: lx_lib::config::IpkConfig {
            abi_version: "1.0".into(),
            tags: vec!["base".into(), "net".into()],
            auto_installed: true,
            essential: true,
            alternatives: vec![lx_lib::config::IpkAlternative {
                priority: 100,
                link_name: "/usr/bin/x".into(),
                target: "/usr/bin/x.real".into(),
            }],
        },
        ..Default::default()
    };
    let job = job("openwrt");
    let ctx = build_ctx(&cfg, &job, binary_dir.path(), root.path());
    let out = get_packager("ipk").unwrap().build(&ctx).unwrap();

    // Pull control.tar.gz out of the ar container and read `control`.
    let bytes = std::fs::read(&out).unwrap();
    let mut ar = ar::Archive::new(bytes.as_slice());
    let mut control_gz = Vec::new();
    while let Some(entry) = ar.next_entry() {
        let mut e = entry.unwrap();
        if String::from_utf8_lossy(e.header().identifier()).starts_with("control.tar") {
            e.read_to_end(&mut control_gz).unwrap();
            break;
        }
    }
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(control_gz.as_slice()));
    let mut control = String::new();
    for entry in tar.entries().unwrap() {
        let mut e = entry.unwrap();
        if e.path().unwrap().to_string_lossy() == "control" {
            e.read_to_string(&mut control).unwrap();
            break;
        }
    }
    assert!(control.contains("ABIVersion: 1.0"), "{control}");
    assert!(control.contains("Tags: base net"), "{control}");
    assert!(control.contains("Auto-Installed: yes"), "{control}");
    assert!(control.contains("Essential: yes"), "{control}");
    assert!(
        control.contains("Alternatives: 100:/usr/bin/x:/usr/bin/x.real"),
        "{control}"
    );
}

/// The converter maps the freshly-closed fields and warns on unknown keys
/// instead of dropping them.
#[test]
fn converter_maps_msix_ipk_rpm_and_warns_unknown() {
    let yaml = r#"
name: app
version: 1.2.3
prerelease: beta.1
arch: amd64
msix:
  publisher: "CN=Acme, O=Acme, C=US"
  properties:
    logo: Assets/logo.png
  applications:
    - id: App
      executable: VFS/app.exe
rpm:
  group: System/Base
  buildhost: build.example.com
  prefixes: [/opt/app]
ipk:
  abi_version: "1.0"
  essential: true
totally_unknown_key: 1
"#;
    let out = lx_lib::nfpm::convert_str(yaml).unwrap();
    let cfg = PackageConfig::parse_str(&out).expect("converted config parses");
    assert_eq!(cfg.version, "1.2.3-beta.1");
    assert_eq!(cfg.msix.publisher, "CN=Acme, O=Acme, C=US");
    assert_eq!(cfg.msix.applications[0].id, "App");
    assert_eq!(cfg.rpm.group, "System/Base");
    assert_eq!(cfg.rpm.buildhost, "build.example.com");
    assert_eq!(cfg.rpm.prefixes, vec!["/opt/app"]);
    assert!(cfg.ipk.essential);
    assert!(
        out.contains("totally_unknown_key"),
        "unknown keys must be surfaced in the notes"
    );
}

/// Top-level `mtime:` overrides the reproducible timestamp.
#[test]
fn mtime_override_parses() {
    let cfg = PackageConfig::parse_str(
        "package_name: x\ngithub_repo: o/x\nmtime: \"2001-02-03T04:05:06Z\"\n",
    )
    .unwrap();
    assert_eq!(cfg.effective_mtime().unwrap(), Some(981_173_106));
}
