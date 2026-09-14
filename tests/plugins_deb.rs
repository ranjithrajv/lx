// SPDX-License-Identifier: GPL-3.0-or-later

use lx_lib::plugins::deb::*;

use lx_lib::config::PackageConfig;
use lx_lib::github::Asset;

fn job() -> lx_lib::build::ResolvedJob {
    lx_lib::build::ResolvedJob {
        dist: "trixie".into(),
        arch: "amd64".into(),
        asset: Asset {
            name: "pkg.tar.gz".into(),
            size: None,
            browser_download_url: String::new(),
            checksums: Default::default(),
        },
        tag: "v1.0.0".into(),
        published_at: Some(1_735_689_600), // 2025-01-01T00:00:00Z
    }
}

#[test]
fn write_copyright_year_comes_from_published_at() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = PackageConfig {
        package_name: "eza".into(),
        github_repo: "eza-community/eza".into(),
        ..PackageConfig::default()
    };
    write_copyright(dir.path(), &cfg, None, Some(1_735_689_600)).unwrap();
    let text = std::fs::read_to_string(dir.path().join("copyright")).unwrap();
    assert!(text.contains("Copyright: 2025 eza-community/eza contributors"));
}

#[test]
fn render_control_includes_depends_when_set() {
    let cfg = PackageConfig {
        package_name: "pnpm".into(),
        github_repo: "pnpm/pnpm".into(),
        depends: "libatomic1, libgtk-3-0".into(),
        ..PackageConfig::default()
    };
    let text = render_control(&cfg, &job(), "1.0.0", "1", &[]);
    assert!(text.contains("Depends: libatomic1, libgtk-3-0\n"));
    // Matches the bash action's image build, which `>>`-appended Depends
    // after the control file (including Description) was rendered.
    assert!(text.trim_end().ends_with("Depends: libatomic1, libgtk-3-0"));
}

#[test]
fn render_control_omits_depends_when_empty() {
    let cfg = PackageConfig {
        package_name: "eza".into(),
        github_repo: "eza-community/eza".into(),
        ..PackageConfig::default()
    };
    let text = render_control(&cfg, &job(), "1.0.0", "1", &[]);
    assert!(!text.contains("Depends:"));
}

#[test]
fn render_control_includes_all_relation_fields_when_set() {
    let cfg = PackageConfig {
        package_name: "foo".into(),
        github_repo: "owner/foo".into(),
        depends: "libatomic1".into(),
        recommends: "bash-completion".into(),
        conflicts: "foo-legacy".into(),
        replaces: "foo-legacy".into(),
        provides: "foo-cli".into(),
        breaks: "foo-legacy (<< 2.0)".into(),
        ..PackageConfig::default()
    };
    let text = render_control(&cfg, &job(), "1.0.0", "1", &[]);
    assert!(text.contains("Depends: libatomic1\n"));
    assert!(text.contains("Recommends: bash-completion\n"));
    assert!(text.contains("Conflicts: foo-legacy\n"));
    assert!(text.contains("Replaces: foo-legacy\n"));
    assert!(text.contains("Provides: foo-cli\n"));
    assert!(text.contains("Breaks: foo-legacy (<< 2.0)\n"));
}

#[test]
fn render_control_prefixes_version_with_epoch_but_not_filename() {
    let cfg = PackageConfig {
        package_name: "foo".into(),
        github_repo: "owner/foo".into(),
        epoch: "1".into(),
        ..PackageConfig::default()
    };
    let text = render_control(&cfg, &job(), "2.0.0", "1", &[]);
    assert!(text.contains("Version: 1:2.0.0-1+trixie\n"));
}

#[test]
fn render_control_omits_epoch_prefix_when_unset() {
    let cfg = PackageConfig {
        package_name: "foo".into(),
        github_repo: "owner/foo".into(),
        ..PackageConfig::default()
    };
    let text = render_control(&cfg, &job(), "2.0.0", "1", &[]);
    // Exactly "Version: 2.0.0-..." -- no epoch prefix sneaking in
    // before the version number itself.
    assert!(text.contains("Version: 2.0.0-1+trixie\n"));
}

#[test]
fn render_changelog_includes_epoch_in_version() {
    let cfg = PackageConfig {
        package_name: "foo".into(),
        github_repo: "owner/foo".into(),
        epoch: "1".into(),
        ..PackageConfig::default()
    };
    let text = render_changelog(&cfg, &job(), "2.0.0", "1");
    assert!(text.starts_with("foo (1:2.0.0-1+trixie) trixie;"));
}
