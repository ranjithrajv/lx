// SPDX-License-Identifier: GPL-3.0-or-later

use lx_lib::config::PackageConfig;
use lx_lib::pkgmeta::normalize_version;

#[test]
fn normalize_version_semver_strips_v_prefix() {
    assert_eq!(normalize_version("v1.2.3", "semver"), "1.2.3");
    assert_eq!(normalize_version("V1.2.3", "semver"), "1.2.3");
    assert_eq!(normalize_version("v0.23.5", "semver"), "0.23.5");
}

#[test]
fn normalize_version_semver_strips_non_digit_prefix() {
    assert_eq!(normalize_version("bun-v1.3.14", "semver"), "1.3.14");
    assert_eq!(normalize_version("release-2.0.0", "semver"), "2.0.0");
}

#[test]
fn normalize_version_semver_accepts_partial() {
    assert_eq!(normalize_version("1.2", "semver"), "1.2");
    assert_eq!(normalize_version("v1.2", "semver"), "1.2");
}

#[test]
fn normalize_version_none_returns_as_is() {
    assert_eq!(normalize_version("v1.2.3", "none"), "v1.2.3");
    assert_eq!(normalize_version("bun-v1.3.14", "none"), "bun-v1.3.14");
    assert_eq!(normalize_version("  v1.0  ", "none"), "v1.0");
}

#[test]
fn normalize_version_default_is_semver() {
    // Empty schema falls through to the semver arm.
    assert_eq!(normalize_version("v1.2.3", ""), "1.2.3");
}

#[test]
fn effective_arch_variant() {
    let mut cfg = PackageConfig {
        package_name: "test".into(),
        github_repo: "owner/test".into(),
        arch_variant: "amd64v3".into(),
        ..Default::default()
    };
    assert_eq!(cfg.effective_arch_variant(), "amd64v3");
    cfg.arch_variant.clear();
    assert_eq!(cfg.effective_arch_variant(), "");
}

#[test]
fn effective_umask_parses_octal() {
    let mut cfg = PackageConfig {
        package_name: "test".into(),
        github_repo: "owner/test".into(),
        ..Default::default()
    };
    assert_eq!(cfg.effective_umask(), None);
    cfg.umask = "0o002".into();
    assert_eq!(cfg.effective_umask(), Some(0o002));
    cfg.umask = "002".into();
    assert_eq!(cfg.effective_umask(), Some(0o002));
    cfg.umask = "0o022".into();
    assert_eq!(cfg.effective_umask(), Some(0o022));
}

#[test]
fn effective_umask_invalid_returns_none() {
    let mut cfg = PackageConfig {
        package_name: "test".into(),
        github_repo: "owner/test".into(),
        umask: "not-octal".into(),
        ..Default::default()
    };
    assert_eq!(cfg.effective_umask(), None);
    cfg.umask = "0o999".into();
    assert_eq!(cfg.effective_umask(), None);
}

#[test]
fn effective_packager_falls_back_to_maintainer() {
    let mut cfg = PackageConfig {
        package_name: "test".into(),
        github_repo: "owner/test".into(),
        maintainer: "Jane <jane@example.com>".into(),
        ..Default::default()
    };
    assert_eq!(cfg.effective_packager(), "Jane <jane@example.com>");
    cfg.packager = "GoReleaser <dev@goreleaser.com>".into();
    assert_eq!(cfg.effective_packager(), "GoReleaser <dev@goreleaser.com>");
}

#[test]
fn effective_version_schema_default() {
    let mut cfg = PackageConfig {
        package_name: "test".into(),
        github_repo: "owner/test".into(),
        ..Default::default()
    };
    assert_eq!(cfg.effective_version_schema(), "semver");
    cfg.version_schema = "none".into();
    assert_eq!(cfg.effective_version_schema(), "none");
    cfg.version_schema = "  NONE  ".into();
    assert_eq!(cfg.effective_version_schema(), "none");
}

#[test]
fn validate_rejects_invalid_version_schema() {
    let cfg = PackageConfig {
        package_name: "test".into(),
        github_repo: "owner/test".into(),
        version_schema: "invalid".into(),
        ..Default::default()
    };
    assert!(cfg.validate_for_local().is_err());
}

#[test]
fn validate_rejects_invalid_umask() {
    let cfg = PackageConfig {
        package_name: "test".into(),
        github_repo: "owner/test".into(),
        umask: "abc".into(),
        ..Default::default()
    };
    assert!(cfg.validate_for_local().is_err());
}
