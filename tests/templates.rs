// SPDX-License-Identifier: GPL-3.0-or-later

//! Every bundled starter template (ported verbatim from
//! debian-multiarch-builder's templates/) must parse through the same
//! pipeline a real package.yaml takes, including legacy-key compat.

use lx_lib::config::PackageConfig;
use lx_lib::wizard::EMBEDDED_TEMPLATES;

#[test]
fn every_embedded_template_parses() {
    assert_eq!(EMBEDDED_TEMPLATES.len(), 12);
    for (name, body) in EMBEDDED_TEMPLATES {
        let cfg = PackageConfig::parse_str(body)
            .unwrap_or_else(|e| panic!("template '{name}' failed to parse: {e:#}"));
        assert!(
            !cfg.package_name.is_empty(),
            "template {name} lost package_name"
        );
        assert!(
            cfg.github_repo.contains('/'),
            "template {name} lost github_repo"
        );
    }
}

#[test]
fn rust_eza_expands_patterns_from_legacy_keys() {
    let body = EMBEDDED_TEMPLATES
        .iter()
        .find(|(n, _)| *n == "rust/eza")
        .expect("rust/eza bundled")
        .1;
    let cfg = PackageConfig::parse_str(body).unwrap();
    // summary/license/vendor folded; pattern expanded via architecture_map.
    assert!(cfg.has_manual_patterns());
    let patterns = cfg.architectures.patterns();
    assert_eq!(
        patterns["amd64"].release_pattern,
        "eza_v{version}_x86_64-unknown-linux-gnu.tar.gz"
    );
}

#[test]
fn neovim_dependencies_map_to_depends() {
    let body = EMBEDDED_TEMPLATES
        .iter()
        .find(|(n, _)| *n == "c/neovim")
        .expect("c/neovim bundled")
        .1;
    let cfg = PackageConfig::parse_str(body).unwrap();
    assert!(cfg.depends.contains("libncursesw6"));
    assert!(cfg.depends.contains(", "));
}
