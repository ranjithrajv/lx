// SPDX-License-Identifier: GPL-3.0-or-later

//! `nfpm.yaml` → `package.yaml` conversion (`lx init --from-nfpm`).

use lx_lib::config::PackageConfig;

const NFPM_YAML: &str = r#"
name: mytool
arch: amd64
version: v1.2.3
version_schema: semver
release: "2"
epoch: "1"
maintainer: Jane Doe <jane@example.com>
description: A tool
vendor: Acme
homepage: https://example.com/mytool
license: MIT
section: utils
priority: optional
disable_globbing: true
umask: 0o022
depends:
  - libc6 (>= 2.31)
  - libssl3
recommends:
  - ca-certificates
scripts:
  preinstall: pre.sh
  postinstall: post.sh
  preremove: preun.sh
rpm:
  compression: zstd
  packager: Acme Packaging
  scripts:
    posttrans: posttrans.sh
  group: Unspecified
deb:
  arch_variant: amd64v3
  compression: xz
  predepends:
    - bash
  breaks:
    - mytool-legacy (<< 2.0)
  fields:
    Bugs: https://example.com/bugs
  scripts:
    rules: myrules
  triggers:
    interest:
      - /usr/lib/mytool
  signature:
    key_file: key.gpg
    method: debsign
    type: origin
archlinux:
  scripts:
    postupgrade: postupgrade.sh
apk:
  scripts:
    preupgrade: preupgrade.sh
changelog: CHANGELOG.yaml
contents:
  - src: ./mytool
    dst: /usr/bin/mytool
    file_info:
      owner: root
      group: root
      mode: 0o755
      mtime: "2008-01-02T15:04:05Z"
      lang: en
  - src: ./conf/mytool.conf
    dst: /etc/mytool/mytool.conf
    type: config|noreplace
  - src: ./lib
    dst: /usr/lib/mytool
    type: tree
    expand: true
    disown_subtree:
      - /usr/lib/mytool/vendor
  - dst: /var/lib/mytool
    type: dir
overrides:
  rpm:
    depends:
      - openssl
"#;

#[test]
fn converts_nfpm_config_and_reparses_as_package_yaml() {
    let out = lx_lib::nfpm::convert_str(NFPM_YAML).expect("conversion should succeed");
    assert!(out.contains("REVIEW ME"));

    // The generated YAML must itself be a valid lx config.
    let cfg = PackageConfig::parse_str(&out).expect("converted config should parse and validate");

    assert_eq!(cfg.package_name, "mytool");
    assert_eq!(cfg.github_repo, "OWNER/mytool");
    assert_eq!(cfg.version, "v1.2.3");
    assert_eq!(cfg.build_version, "2");
    assert_eq!(cfg.epoch, "1");
    assert_eq!(cfg.license_spdx, "MIT");
    assert_eq!(cfg.vendor, "Acme");
    assert_eq!(cfg.section, "utils");
    assert!(cfg.disable_globbing);
    assert_eq!(cfg.effective_umask(), Some(0o022));
    assert_eq!(cfg.effective_architectures(), vec!["amd64".to_string()]);

    assert_eq!(cfg.depends, "libc6 (>= 2.31), libssl3");
    assert_eq!(cfg.recommends, "ca-certificates");
    assert_eq!(cfg.predepends, "bash");
    assert_eq!(cfg.breaks, "mytool-legacy (<< 2.0)");
    assert_eq!(cfg.compression, "xz");
    assert_eq!(cfg.arch_variant, "amd64v3");
    assert_eq!(cfg.packager, "Acme Packaging");
    assert_eq!(
        cfg.fields.get("Bugs").map(String::as_str),
        Some("https://example.com/bugs")
    );

    // Maintainer scripts plus the rpm/arch/apk-specific hooks.
    assert_eq!(cfg.scripts.preinstall, "pre.sh");
    assert_eq!(cfg.scripts.postinstall, "post.sh");
    assert_eq!(cfg.scripts.preremove, "preun.sh");
    assert_eq!(cfg.scripts.posttrans, "posttrans.sh");
    assert_eq!(cfg.scripts.postupgrade_script, "postupgrade.sh");
    assert_eq!(cfg.scripts.preupgrade_script, "preupgrade.sh");

    // Per-format blocks.
    assert_eq!(cfg.rpm.compression, "zstd");
    assert_eq!(cfg.deb.rules, "myrules");
    assert_eq!(cfg.deb.triggers_interest, vec!["/usr/lib/mytool"]);

    // Signing.
    assert_eq!(cfg.signature.key_file, "key.gpg");
    assert_eq!(cfg.signature.method, "debsign");
    assert_eq!(cfg.signature.sign_type, "origin");

    // Overrides.
    let rpm_override = cfg.overrides.get("rpm").expect("rpm override");
    assert_eq!(rpm_override.depends.as_deref(), Some("openssl"));

    // Contents DSL, including file_info / expand / disown_subtree.
    assert_eq!(cfg.contents.len(), 4);
    let tool = &cfg.contents[0];
    assert_eq!(tool.dst, "/usr/bin/mytool");
    assert_eq!(tool.file_info.parsed_mode().unwrap(), Some(0o755));
    assert_eq!(tool.file_info.parsed_mtime().unwrap(), Some(1_199_286_245));
    assert_eq!(tool.file_info.owner, "root");
    assert_eq!(tool.file_info.lang, "en");

    let conf = &cfg.contents[1];
    assert_eq!(conf.kind, "config|noreplace");

    let tree = &cfg.contents[2];
    assert!(tree.expand);
    assert_eq!(tree.disown_subtree, vec!["/usr/lib/mytool/vendor"]);
    assert_eq!(tree.kind, "tree");

    // Unmapped keys are surfaced, not dropped silently.
    assert!(out.contains("changelog"));
    assert!(out.contains("github_repo"));
}

#[test]
fn rejects_missing_name() {
    let err = lx_lib::nfpm::convert_str("version: 1.0.0\n").unwrap_err();
    assert!(err.to_string().contains("name"));
}
