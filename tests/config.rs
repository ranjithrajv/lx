// SPDX-License-Identifier: GPL-3.0-or-later

use lx_lib::config::*;

use std::path::{Path, PathBuf};

#[test]
fn overlay_replaces_top_level_keys_and_keeps_the_rest() {
    let base = r#"
package_name: atuin
github_repo: atuinsh/atuin
version: "18.0.0"
depends: "libc6"
debian_distributions: [trixie]
"#;
    let mut cfg: PackageConfig = PackageConfig::parse_str(base).unwrap();

    let dir = tempfile::tempdir().unwrap();
    let overlay_path = dir.path().join("overlay.yaml");
    std::fs::write(
        &overlay_path,
        r#"
version: "19.0.0"
depends: "libc6, libssl3"
"#,
    )
    .unwrap();

    cfg.apply_overlay(&overlay_path).unwrap();

    assert_eq!(cfg.version, "19.0.0");
    assert_eq!(cfg.depends, "libc6, libssl3");
    // Untouched by the overlay.
    assert_eq!(cfg.package_name, "atuin");
    assert_eq!(cfg.github_repo, "atuinsh/atuin");
    assert_eq!(cfg.debian_distributions, vec!["trixie"]);
}

#[test]
fn overlay_onto_invalid_config_fails_validation() {
    let base = r#"
package_name: atuin
github_repo: atuinsh/atuin
"#;
    let mut cfg: PackageConfig = PackageConfig::parse_str(base).unwrap();

    let dir = tempfile::tempdir().unwrap();
    let overlay_path = dir.path().join("overlay.yaml");
    // Blanking a required field must fail validate() rather than silently apply.
    std::fs::write(&overlay_path, "github_repo: \"\"\n").unwrap();

    assert!(cfg.apply_overlay(&overlay_path).is_err());
}

#[test]
fn parses_manual_patterns() {
    let yaml = r#"
package_name: atuin
github_repo: atuinsh/atuin
artifact_format: tar.gz
description: "Magical shell history"
debian_distributions: [trixie, forky, sid]
architectures:
  amd64:
    release_pattern: "atuin-x86_64-unknown-linux-gnu.tar.gz"
  arm64:
    release_pattern: "atuin-aarch64-unknown-linux-gnu.tar.gz"
"#;
    let cfg: PackageConfig = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(cfg.package_name, "atuin");
    assert_eq!(
        cfg.effective_distributions_for(&cfg.effective_package_format()),
        vec!["trixie", "forky", "sid"]
    );
    assert!(cfg.has_manual_patterns());
    assert_eq!(
        cfg.architectures.patterns()["amd64"].release_pattern,
        "atuin-x86_64-unknown-linux-gnu.tar.gz"
    );
}

#[test]
fn parses_architectures_simple_list() {
    let yaml = r#"
package_name: eza
github_repo: eza-community/eza
architectures: [amd64, arm64, armhf]
"#;
    let cfg: PackageConfig = serde_yaml::from_str(yaml).unwrap();
    assert!(!cfg.has_manual_patterns());
    assert!(cfg.architectures.patterns().is_empty());
    let mut archs = cfg.effective_architectures();
    archs.sort();
    assert_eq!(archs, vec!["amd64", "arm64", "armhf"]);
}

#[test]
fn rejects_empty_architectures_list_entry() {
    let yaml = "package_name: x\ngithub_repo: a/b\narchitectures: [amd64, \"\"]\n";
    let cfg: PackageConfig = serde_yaml::from_str(yaml).unwrap();
    assert!(cfg.validate().is_err());
}

#[test]
fn parses_bundle_and_depends() {
    let yaml = r#"
package_name: zed
github_repo: zed-industries/zed
bundle: true
depends: "libatomic1, libgtk-3-0"
"#;
    let cfg: PackageConfig = serde_yaml::from_str(yaml).unwrap();
    assert!(cfg.bundle);
    assert_eq!(cfg.depends, "libatomic1, libgtk-3-0");
}

#[test]
fn parses_dependency_relations_and_epoch() {
    let yaml = r#"
package_name: foo
github_repo: owner/foo
recommends: "bash-completion"
conflicts: "foo-legacy"
replaces: "foo-legacy"
provides: "foo-cli"
breaks: "foo-legacy (<< 2.0)"
epoch: "1"
"#;
    let cfg: PackageConfig = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(cfg.recommends, "bash-completion");
    assert_eq!(cfg.conflicts, "foo-legacy");
    assert_eq!(cfg.replaces, "foo-legacy");
    assert_eq!(cfg.provides, "foo-cli");
    assert_eq!(cfg.breaks, "foo-legacy (<< 2.0)");
    assert_eq!(cfg.epoch, "1");
}

#[test]
fn parses_new_fields_suggests_predepends_section_priority_fields_compression() {
    let yaml = r#"
package_name: foo
github_repo: owner/foo
suggests: "bar"
predepends: "libc6 (>= 2.35)"
section: "devel"
priority: "extra"
fields:
  Bugs: "https://github.com/owner/foo/issues"
  X-Custom: "value"
compression: "xz"
"#;
    let cfg: PackageConfig = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(cfg.suggests, "bar");
    assert_eq!(cfg.predepends, "libc6 (>= 2.35)");
    assert_eq!(cfg.effective_section(), "devel");
    assert_eq!(cfg.effective_priority(), "extra");
    assert_eq!(
        cfg.fields.get("Bugs").unwrap(),
        "https://github.com/owner/foo/issues"
    );
    assert_eq!(cfg.effective_compression(), "xz");
    cfg.validate().unwrap();
}

#[test]
fn defaults_for_new_fields() {
    let yaml = "package_name: foo\ngithub_repo: owner/foo\n";
    let cfg: PackageConfig = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(cfg.effective_section(), "utils");
    assert_eq!(cfg.effective_priority(), "optional");
    assert_eq!(cfg.effective_compression(), "gzip");
    assert!(cfg.fields.is_empty());
    assert!(cfg.suggests.is_empty());
    assert!(cfg.predepends.is_empty());
}

#[test]
fn rejects_unknown_compression() {
    let yaml = "package_name: foo\ngithub_repo: owner/foo\ncompression: \"bzip2\"\n";
    let cfg: PackageConfig = serde_yaml::from_str(yaml).unwrap();
    assert!(cfg.validate().is_err());
}

#[test]
fn accepts_every_compression_alias_the_schema_and_archiver_accept() {
    // Guards the schema enum / validator / archiver agreement: any name the
    // schema lists and `debarchive::CompressionKind` parses must validate.
    for c in ["gzip", "gz", "xz", "zstd", "zst", "none"] {
        let yaml = format!("package_name: foo\ngithub_repo: owner/foo\ncompression: \"{c}\"\n");
        let cfg: PackageConfig = serde_yaml::from_str(&yaml).unwrap();
        assert!(cfg.validate().is_ok(), "compression '{c}' should validate");
    }
}

#[test]
fn rejects_non_numeric_compression_level_at_validate_time() {
    let yaml = "package_name: foo\ngithub_repo: owner/foo\ncompression: \"gzip:fast\"\n";
    let cfg: PackageConfig = serde_yaml::from_str(yaml).unwrap();
    assert!(cfg.validate().is_err());
    // Numeric levels pass validation; range enforcement stays with the
    // archiver's own parser (single source of truth for ranges).
    let yaml = "package_name: foo\ngithub_repo: owner/foo\ncompression: \"zstd:19\"\n";
    let cfg: PackageConfig = serde_yaml::from_str(yaml).unwrap();
    assert!(cfg.validate().is_ok());
}

#[test]
fn maintainer_env_overrides_builtin_default() {
    // Precedence when `maintainer:` is unset:
    // $LX_MAINTAINER > built-in latest-debs default.
    let unset = resolve_default_maintainer(std::env::var("NO_SUCH_VAR_XYZ_123").ok());
    assert_eq!(unset, lx_lib::constants::DEFAULT_MAINTAINER);
    assert_eq!(
        resolve_default_maintainer(Some("Downstream Packager <p@d.example>".into())),
        "Downstream Packager <p@d.example>"
    );
    // Whitespace-only counts as unset.
    assert_eq!(
        resolve_default_maintainer(Some("   ".into())),
        lx_lib::constants::DEFAULT_MAINTAINER
    );
}

#[test]
fn gerrit_source_and_host_parse_and_validate() {
    let yaml = "package_name: foo\ngithub_repo: owner/foo\nsource: gerrit\ngerrit_host: gerrit.example.com\n";
    let cfg: PackageConfig = serde_yaml::from_str(yaml).unwrap();
    cfg.validate().unwrap();
    assert_eq!(cfg.effective_forge_source(), "gerrit");
    assert_eq!(cfg.gerrit_host.as_deref(), Some("gerrit.example.com"));
}

#[test]
fn parses_contents_overrides_scripts_signature() {
    let yaml = r#"
package_name: foo
github_repo: owner/foo
contents:
  - src: completions/foo.bash
    dst: /usr/share/bash-completion/completions/foo
  - src: etc/foo.conf
    dst: /etc/foo.conf
    type: config|noreplace
  - src: share/
    dst: /usr/share/foo
    type: tree
  - src: /usr/lib/foo/foo.real
    dst: /usr/bin/foo
    type: symlink
  - dst: /var/lib/foo
    type: dir
overrides:
  deb:
    depends: "libatomic1"
    recommends: ""
  rpm:
    depends: "libatomic"
scripts:
  postinstall: ./scripts/postinst.sh
signature:
  key_file: keys/sign.asc
  key_id: ABC123
"#;
    let cfg: PackageConfig = serde_yaml::from_str(yaml).unwrap();
    cfg.validate().unwrap();
    assert_eq!(cfg.contents.len(), 5);
    assert_eq!(cfg.contents[1].kind, "config|noreplace");
    assert_eq!(cfg.contents[4].kind, "dir");
    assert!(cfg.scripts.postinstall.ends_with("postinst.sh"));
    assert_eq!(cfg.signature.key_file, "keys/sign.asc");

    // Overrides replace per format; arch falls through to top-level.
    let deb = cfg.effective_relations("deb");
    assert_eq!(deb.depends, "libatomic1");
    assert_eq!(deb.recommends, "", "empty override clears the field");
    let rpm = cfg.effective_relations("rpm");
    assert_eq!(rpm.depends, "libatomic");
    let arch = cfg.effective_relations("arch");
    assert_eq!(arch.depends, "");
}

#[test]
fn rejects_bad_overrides_key_and_contents() {
    let bad_key = r#"
package_name: foo
github_repo: owner/foo
overrides:
  bogus:
    depends: "x"
"#;
    let cfg: PackageConfig = serde_yaml::from_str(bad_key).unwrap();
    assert!(cfg.validate().is_err());

    // apk/ipk/msix are now valid override targets too.
    let apk_key =
        "package_name: foo\ngithub_repo: owner/foo\noverrides:\n  apk:\n    depends: \"x\"\n";
    let cfg: PackageConfig = serde_yaml::from_str(apk_key).unwrap();
    assert!(cfg.validate().is_ok());

    let msix_key =
        "package_name: foo\ngithub_repo: owner/foo\noverrides:\n  msix:\n    depends: \"x\"\n";
    let cfg: PackageConfig = serde_yaml::from_str(msix_key).unwrap();
    assert!(cfg.validate().is_ok());

    let rel_dst = "package_name: f\ngithub_repo: o/f\ncontents:\n  - src: a\n    dst: usr/bin/a\n";
    let cfg: PackageConfig = serde_yaml::from_str(rel_dst).unwrap();
    assert!(cfg.validate().is_err());

    let traversal =
        "package_name: f\ngithub_repo: o/f\ncontents:\n  - src: a\n    dst: /../etc/x\n";
    let cfg: PackageConfig = serde_yaml::from_str(traversal).unwrap();
    assert!(cfg.validate().is_err());

    let bad_type =
        "package_name: f\ngithub_repo: o/f\ncontents:\n  - src: a\n    dst: /a\n    type: bogus\n";
    let cfg: PackageConfig = serde_yaml::from_str(bad_type).unwrap();
    assert!(cfg.validate().is_err());
}

#[test]
fn effective_sign_resolution_cli_wins() {
    let mut cfg = PackageConfig::default();
    cfg.signature.key_file = "from-yaml.asc".into();
    cfg.signature.key_id = "YAMLID".into();
    assert_eq!(
        cfg.effective_sign_key(Some(Path::new("cli.asc"))),
        Some(PathBuf::from("cli.asc"))
    );
    // An empty CLI value falls through to the config file's key.
    assert_eq!(
        cfg.effective_sign_key(Some(Path::new(""))),
        Some(PathBuf::from("from-yaml.asc"))
    );
    assert_eq!(
        cfg.effective_sign_key(None),
        Some(PathBuf::from("from-yaml.asc"))
    );
    // No key configured anywhere -> None.
    let bare = PackageConfig::default();
    assert_eq!(bare.effective_sign_key(None), None);
    assert_eq!(cfg.effective_sign_key_id(Some("CLI")), "CLI");
    assert_eq!(cfg.effective_sign_key_id(None), "YAMLID");
}

#[test]
fn auto_discovery_defaults() {
    let yaml = r#"
package_name: eza
github_repo: eza-community/eza
artifact_format: tar.gz
"#;
    let cfg: PackageConfig = serde_yaml::from_str(yaml).unwrap();
    assert!(!cfg.has_manual_patterns());
    // Five suites, matching the action's
    // (.debian_distributions // ["bullseye", "bookworm", "trixie",
    // "forky", "sid"]) default.
    let dists = cfg.effective_distributions_for(&cfg.effective_package_format());
    assert_eq!(dists.len(), 5);
    assert!(dists.contains(&"bullseye".to_string()));
    assert_eq!(cfg.effective_architectures().len(), 9);
}

#[test]
fn rejects_unknown_fields() {
    let yaml = "package_name: x\ngithub_repo: a/b\nbogus_field: nope\n";
    assert!(serde_yaml::from_str::<PackageConfig>(yaml).is_err());
}

#[test]
fn arch_dist_support_matrix() {
    let cfg = PackageConfig::default();
    // Universal architectures work everywhere.
    assert!(cfg.arch_supported_for_dist("amd64", "bookworm"));
    assert!(cfg.arch_supported_for_dist("amd64", "sid"));
    assert!(cfg.arch_supported_for_dist("loong64", "bookworm")); // universal per system.yaml
                                                                 // i386/armel lose support after trixie
                                                                 // (and gained it back to bullseye).
    assert!(cfg.arch_supported_for_dist("i386", "bullseye"));
    assert!(cfg.arch_supported_for_dist("armel", "bullseye"));
    assert!(cfg.arch_supported_for_dist("i386", "bookworm"));
    assert!(cfg.arch_supported_for_dist("i386", "trixie"));
    assert!(!cfg.arch_supported_for_dist("i386", "forky"));
    assert!(!cfg.arch_supported_for_dist("armel", "sid"));
    // Unknown arch is never supported (mips never made system.yaml).
    assert!(!cfg.arch_supported_for_dist("mips64el", "bookworm"));
}

#[test]
fn distribution_arch_overrides_replace_matrix() {
    let yaml = r#"
package_name: eza
github_repo: eza-community/eza
distribution_arch_overrides:
  loong64:
    distributions: [bookworm]
"#;
    let cfg = PackageConfig::parse_str(yaml).unwrap();
    // Override replaces the built-in matrix for that arch entirely.
    assert!(cfg.arch_supported_for_dist("loong64", "bookworm"));
    assert!(!cfg.arch_supported_for_dist("loong64", "forky"));
    // Other architectures fall through to the built-in matrix.
    assert!(cfg.arch_supported_for_dist("i386", "trixie"));
    assert!(!cfg.arch_supported_for_dist("i386", "forky"));
}

#[test]
fn rejects_empty_distribution_arch_override() {
    let yaml = r#"
package_name: eza
github_repo: eza-community/eza
distribution_arch_overrides:
  armhf:
    distributions: []
"#;
    assert!(PackageConfig::parse_str(yaml).is_err());
}

#[test]
fn legacy_eza_template_parses_and_expands() {
    let yaml = r#"
package_name: eza
github_repo: eza-community/eza
summary: "A modern replacement for ls"
vendor: "Eza Community"
license: MIT
download_pattern: "eza_v{version}_{arch}-unknown-linux-gnu.tar.gz"
architecture_map:
  amd64: "x86_64"
  arm64: "aarch64"
  armhf: "armv7"
"#;
    let cfg = PackageConfig::parse_str(yaml).unwrap();
    assert_eq!(cfg.description, "A modern replacement for ls");
    assert_eq!(cfg.license_spdx, "MIT");
    assert_eq!(
        cfg.fields.get("Vendor").map(String::as_str),
        Some("Eza Community")
    );
    assert_eq!(cfg.effective_description(), cfg.description);
    let patterns = cfg.architectures.patterns();
    assert_eq!(patterns.len(), 3);
    assert_eq!(
        patterns["amd64"].release_pattern,
        "eza_v{version}_x86_64-unknown-linux-gnu.tar.gz"
    );
    assert_eq!(
        patterns["armhf"].release_pattern,
        "eza_v{version}_armv7-unknown-linux-gnu.tar.gz"
    );
    assert!(cfg.has_manual_patterns());
}

#[test]
fn legacy_placeholders_expand_package_name_and_identity_arch() {
    // No architecture_map and a {package_name}/{arch} pattern: the Debian
    // arch names are used verbatim as the upstream name.
    let yaml = r#"
package_name: myapp
github_repo: owner/repo
download_pattern: "{package_name}_v{version}_linux_{arch}.tar.gz"
"#;
    let cfg = PackageConfig::parse_str(yaml).unwrap();
    let patterns = cfg.architectures.patterns();
    assert_eq!(
        patterns.len(),
        lx_lib::constants::DEFAULT_ARCHITECTURES.len()
    );
    assert_eq!(
        patterns["amd64"].release_pattern,
        "myapp_v{version}_linux_amd64.tar.gz"
    );
    assert!(patterns["s390x"]
        .release_pattern
        .contains("_linux_s390x.tar.gz"));
}

#[test]
fn legacy_pattern_without_arch_falls_back_to_auto_discovery() {
    // The action's hugo sample: no {arch} placeholder and no map — the
    // action's runtime ignored download_pattern in this shape too, so we
    // keep full auto-discovery instead of inventing an asset split.
    let yaml = r#"
package_name: hugo
github_repo: gohugoio/hugo
download_pattern: "hugo_extended_{version}_Linux-64bit.tar.gz"
"#;
    let cfg = PackageConfig::parse_str(yaml).unwrap();
    assert!(!cfg.has_manual_patterns());
    assert!(cfg.architectures.is_empty());
}

#[test]
fn modern_fields_win_over_legacy() {
    let yaml = r#"
package_name: eza
github_repo: eza-community/eza
description: "modern description wins"
summary: "legacy summary loses"
license_spdx: Apache-2.0
license: MIT
download_pattern: "ignored.zip"
architectures:
  amd64:
    release_pattern: "modern.tar.gz"
"#;
    let cfg = PackageConfig::parse_str(yaml).unwrap();
    assert_eq!(cfg.description, "modern description wins");
    assert_eq!(cfg.license_spdx, "Apache-2.0");
    assert!(cfg.has_manual_patterns());
    assert_eq!(
        cfg.architectures.patterns()["amd64"].release_pattern,
        "modern.tar.gz"
    );
}

#[test]
fn legacy_dependencies_join_into_depends() {
    let yaml = r#"
package_name: neovim
github_repo: neovim/neovim
dependencies:
  - libncursesw6
  - libluajit-5.1-2
"#;
    let cfg = PackageConfig::parse_str(yaml).unwrap();
    assert_eq!(cfg.depends, "libncursesw6, libluajit-5.1-2");
}

#[test]
fn filter_expired_distributions_drops_bullseye_after_lts_end() {
    use lx_lib::config::filter_expired_distributions;
    let before = jiff::civil::date(2026, 8, 31);
    let after = jiff::civil::date(2026, 9, 1);
    let dists = vec![
        "bullseye".to_string(),
        "bookworm".to_string(),
        "sid".to_string(),
    ];
    // On the last LTS day itself the suite still ships (ends >= today).
    let kept = filter_expired_distributions(&dists, Some(before));
    assert_eq!(kept, vec!["bullseye", "bookworm", "sid"]);
    let kept = filter_expired_distributions(&dists, Some(after));
    assert_eq!(kept, vec!["bookworm", "sid"]);
    // Suites with no recorded end date always pass.
    let kept = filter_expired_distributions(&["forky".to_string()], Some(after));
    assert_eq!(kept, vec!["forky"]);
}

#[test]
fn loong64_aliases() {
    let map = upstream_arch_names();
    assert!(map["loong64"].contains(&"loongarch64"));
    assert!(map["loong64"].contains(&"loong64"));
}

#[test]
fn expand_env_vars_substitutes_and_defaults() {
    std::env::set_var("LX_TEST_SIGN_KEY", "/tmp/key.asc");
    let out = expand_env_vars("key: ${LX_TEST_SIGN_KEY}").unwrap();
    assert_eq!(out, "key: /tmp/key.asc");
    std::env::remove_var("LX_TEST_MISSING_XYZ");
    let out = expand_env_vars("x: ${LX_TEST_MISSING_XYZ:-fallback}").unwrap();
    assert_eq!(out, "x: fallback");
    assert!(expand_env_vars("${LX_TEST_MISSING_XYZ}")
        .unwrap_err()
        .to_string()
        .contains("not set"));
    assert_eq!(expand_env_vars("cost is $$5").unwrap(), "cost is $5");
    std::env::remove_var("LX_TEST_SIGN_KEY");
}

#[test]
fn parse_str_expands_env_in_yaml() {
    std::env::set_var("LX_TEST_PAYLOAD", "/opt/payload");
    let yaml = r#"
package_name: foo
github_repo: owner/foo
local_payload: ${LX_TEST_PAYLOAD}
signature:
  key_file: ${LX_TEST_PAYLOAD}/key.asc
  method: debsign
"#;
    let cfg = PackageConfig::parse_str(yaml).unwrap();
    assert_eq!(cfg.local_payload, "/opt/payload");
    assert_eq!(cfg.signature.key_file, "/opt/payload/key.asc");
    assert_eq!(cfg.signature.method, "debsign");
    std::env::remove_var("LX_TEST_PAYLOAD");
}

#[test]
fn signature_method_validation_and_effective() {
    let yaml = "package_name: f\ngithub_repo: o/f\nsignature:\n  method: bogus\n";
    assert!(PackageConfig::parse_str(yaml).is_err());

    let yaml = "package_name: f\ngithub_repo: o/f\nsignature:\n  method: debsign\n";
    let cfg = PackageConfig::parse_str(yaml).unwrap();
    assert_eq!(cfg.effective_sign_method(None), "debsign");
    assert_eq!(cfg.effective_sign_method(Some("detach")), "detach");
    assert_eq!(cfg.effective_sign_type(), "origin");

    let yaml = "package_name: f\ngithub_repo: o/f\nsignature:\n  type: maint\n";
    let cfg = PackageConfig::parse_str(yaml).unwrap();
    assert_eq!(cfg.effective_sign_type(), "maint");

    let yaml = "package_name: f\ngithub_repo: o/f\nsignature:\n  type: bogus\n";
    assert!(PackageConfig::parse_str(yaml).is_err());

    let bare = PackageConfig::parse_str("package_name: f\ngithub_repo: o/f\n").unwrap();
    assert_eq!(bare.effective_sign_method(None), "detach");
}

#[test]
fn contents_packager_filter_parses_and_validates() {
    let yaml = r#"
package_name: foo
github_repo: owner/foo
contents:
  - src: a
    dst: /usr/bin/a
    packager: deb
  - src: b
    dst: /usr/bin/b
    packager: rpm
"#;
    let cfg = PackageConfig::parse_str(yaml).unwrap();
    assert_eq!(cfg.contents[0].packager, "deb");
    assert_eq!(cfg.contents[1].packager, "rpm");

    let bad = "package_name: f\ngithub_repo: o/f\ncontents:\n  - src: a\n    dst: /a\n    packager: bogus\n";
    assert!(PackageConfig::parse_str(bad).is_err());

    // apk/ipk/msix are accepted contents filters now.
    for fmt in ["apk", "ipk", "msix"] {
        let yaml =
            format!("package_name: f\ngithub_repo: o/f\ncontents:\n  - src: a\n    dst: /a\n    packager: {fmt}\n");
        assert!(
            PackageConfig::parse_str(&yaml).is_ok(),
            "{fmt} should be accepted"
        );
    }
}

#[test]
fn parses_local_payload() {
    let yaml = r#"
package_name: foo
github_repo: owner/foo
local_payload: ./dist/foo.tar.gz
version: "1.2.3"
"#;
    let cfg = PackageConfig::parse_str(yaml).unwrap();
    assert_eq!(cfg.local_payload, "./dist/foo.tar.gz");
    // Existence is not checked at parse time.
    cfg.validate().unwrap();
}

#[test]
fn source_mode_accepts_cmake_and_requires_architectures() {
    let ok = r#"
package_name: quickshell
github_repo: quickshell-mirror/quickshell
build_mode: source
architectures: [amd64, arm64]
build_suites: [trixie, forky, sid]
"#;
    let cfg = PackageConfig::parse_str(ok).unwrap();
    assert!(cfg.is_source_mode());

    let no_arch = r#"
package_name: quickshell
github_repo: quickshell-mirror/quickshell
build_mode: source
"#;
    assert!(PackageConfig::parse_str(no_arch).is_err());

    // The dedicated build systems (meson/autotools/make included) are
    // accepted; an unknown one is rejected.
    for ok_system in ["meson", "autotools", "make"] {
        let yaml = format!(
            "package_name: quickshell\ngithub_repo: quickshell-mirror/quickshell\nbuild_mode: source\nbuild_system: {ok_system}\narchitectures: [amd64]\n"
        );
        assert!(
            PackageConfig::parse_str(&yaml).is_ok(),
            "build_system {ok_system} should be accepted"
        );
    }

    let qmake = r#"
package_name: quickshell
github_repo: quickshell-mirror/quickshell
build_mode: source
build_system: qmake
architectures: [amd64]
"#;
    assert!(PackageConfig::parse_str(qmake).is_err());

    let bogus = r#"
package_name: quickshell
github_repo: quickshell-mirror/quickshell
build_mode: tarball
"#;
    assert!(PackageConfig::parse_str(bogus).is_err());
}

#[test]
fn source_suites_apply_build_and_skip_lists() {
    let cfg = PackageConfig::parse_str(
        r#"
package_name: quickshell
github_repo: quickshell-mirror/quickshell
build_mode: source
architectures: [amd64]
build_suites: [trixie, forky, sid]
skip_suites: [bullseye, bookworm]
"#,
    )
    .unwrap();
    assert_eq!(
        cfg.source_suites(&["bullseye".into(), "bookworm".into()]),
        vec!["trixie".to_string(), "forky".to_string(), "sid".to_string()]
    );

    // No build_suites: configured distributions minus skip_suites.
    let cfg2 = PackageConfig::parse_str(
        r#"
package_name: quickshell
github_repo: quickshell-mirror/quickshell
build_mode: source
architectures: [amd64]
skip_suites: [bookworm]
"#,
    )
    .unwrap();
    assert_eq!(
        cfg2.source_suites(&["bookworm".into(), "trixie".into()]),
        vec!["trixie".to_string()]
    );
}

#[test]
fn oldest_suite_follows_bash_family_orders() {
    let debian = vec!["sid".to_string(), "trixie".to_string(), "forky".to_string()];
    assert_eq!(
        PackageConfig::oldest_suite(&debian).as_deref(),
        Some("trixie")
    );
    let ubuntu = vec!["noble".to_string(), "jammy".to_string()];
    assert_eq!(
        PackageConfig::oldest_suite(&ubuntu).as_deref(),
        Some("jammy")
    );
    let unknown = vec!["mystery".to_string()];
    assert_eq!(
        PackageConfig::oldest_suite(&unknown).as_deref(),
        Some("mystery")
    );
}

#[test]
fn source_mode_custom_build_requires_install_commands() {
    let custom = r#"
package_name: tiny
github_repo: o/r
build_mode: source
build_system: custom
build_commands: ["make"]
install_commands: ["make install DESTDIR=$DESTDIR"]
architectures: [amd64]
"#;
    assert!(PackageConfig::parse_str(custom).is_ok());

    let missing_install = r#"
package_name: tiny
github_repo: o/r
build_mode: source
build_system: custom
architectures: [amd64]
"#;
    assert!(PackageConfig::parse_str(missing_install).is_err());
}

#[test]
fn repo_self_packaging_config_parses() {
    // .github/lx/package.yaml is the dogfood config the release workflow
    // feeds back into `lx build`; it must stay loadable and use the musl +
    // pinned-asset keys it claims to.
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/lx/package.yaml");
    let cfg = PackageConfig::load(&path).unwrap();
    assert_eq!(cfg.package_name, "lx");
    assert_eq!(cfg.github_repo, "ranjithrajv/lx");
    assert!(cfg.musl, "self-packaging config should request musl-static");
    assert!(
        cfg.has_manual_patterns(),
        "self-packaging config should pin release assets"
    );
}
