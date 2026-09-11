// SPDX-License-Identifier: GPL-3.0-or-later

use lx_lib::config::{DebConfig, PackageConfig};

fn empty_config() -> PackageConfig {
    PackageConfig {
        package_name: "test".into(),
        github_repo: "owner/test".into(),
        ..Default::default()
    }
}

#[test]
fn deb_extra_members_empty_when_nothing_configured() {
    let cfg = empty_config();
    let members = lx_lib::plugins::deb_extra_members(&cfg).unwrap();
    assert!(members.is_empty());
}

#[test]
fn deb_extra_members_rules_mode_0755() {
    let dir = tempfile::tempdir().unwrap();
    let rules_path = dir.path().join("rules");
    std::fs::write(&rules_path, "#!/usr/bin/make -f\n%\n\t$@\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&rules_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&rules_path, perms).unwrap();
    }

    let mut cfg = empty_config();
    cfg.deb.rules = rules_path.to_string_lossy().into();

    let members = lx_lib::plugins::deb_extra_members(&cfg).unwrap();
    let rules = members.iter().find(|m| m.name == "rules").unwrap();
    assert_eq!(rules.mode, 0o755);
    assert!(String::from_utf8_lossy(&rules.content).contains("make -f"));
}

#[test]
fn deb_extra_members_templates_mode_0644() {
    let dir = tempfile::tempdir().unwrap();
    let tmpl_path = dir.path().join("templates");
    std::fs::write(
        &tmpl_path,
        "Template: test/question\nType: string\nDefault: yes\nDescription: test\n",
    )
    .unwrap();

    let mut cfg = empty_config();
    cfg.deb.templates = tmpl_path.to_string_lossy().into();

    let members = lx_lib::plugins::deb_extra_members(&cfg).unwrap();
    let tmpl = members.iter().find(|m| m.name == "templates").unwrap();
    assert_eq!(tmpl.mode, 0o644);
    assert!(String::from_utf8_lossy(&tmpl.content).contains("Template:"));
}

#[test]
fn deb_extra_members_config_mode_0755() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config");
    std::fs::write(&config_path, "#!/bin/sh\n. /usr/share/debconf/confmodule\n").unwrap();

    let mut cfg = empty_config();
    cfg.deb.config = config_path.to_string_lossy().into();

    let members = lx_lib::plugins::deb_extra_members(&cfg).unwrap();
    let config = members.iter().find(|m| m.name == "config").unwrap();
    assert_eq!(config.mode, 0o755);
    assert!(String::from_utf8_lossy(&config.content).contains("debconf"));
}

#[test]
fn deb_extra_members_triggers_interest_and_activate() {
    let cfg = PackageConfig {
        deb: DebConfig {
            triggers_interest: vec!["some-trigger".into()],
            triggers_activate: vec!["another-trigger".into()],
            ..Default::default()
        },
        ..empty_config()
    };

    let members = lx_lib::plugins::deb_extra_members(&cfg).unwrap();
    let triggers = members.iter().find(|m| m.name == "triggers").unwrap();
    assert_eq!(triggers.mode, 0o644);
    let body = String::from_utf8_lossy(&triggers.content);
    assert!(body.contains("interest some-trigger"));
    assert!(body.contains("activate another-trigger"));
}

#[test]
fn deb_extra_members_no_triggers_when_empty() {
    let cfg = empty_config();
    let members = lx_lib::plugins::deb_extra_members(&cfg).unwrap();
    assert!(!members.iter().any(|m| m.name == "triggers"));
}

#[test]
fn deb_extra_members_all_combined() {
    let dir = tempfile::tempdir().unwrap();
    let rules_path = dir.path().join("rules");
    std::fs::write(&rules_path, "# rules\n").unwrap();
    let tmpl_path = dir.path().join("templates");
    std::fs::write(&tmpl_path, "Template: t/q\n").unwrap();
    let config_path = dir.path().join("config");
    std::fs::write(&config_path, "#!/bin/sh\n").unwrap();

    let cfg = PackageConfig {
        deb: DebConfig {
            rules: rules_path.to_string_lossy().into(),
            templates: tmpl_path.to_string_lossy().into(),
            config: config_path.to_string_lossy().into(),
            triggers_interest: vec!["trigger-a".into()],
            triggers_activate: vec!["trigger-b".into()],
            ..Default::default()
        },
        ..empty_config()
    };

    let members = lx_lib::plugins::deb_extra_members(&cfg).unwrap();
    assert_eq!(members.len(), 4);
    assert!(members.iter().any(|m| m.name == "rules"));
    assert!(members.iter().any(|m| m.name == "templates"));
    assert!(members.iter().any(|m| m.name == "config"));
    assert!(members.iter().any(|m| m.name == "triggers"));
}

#[test]
fn scripts_includes_rpm_and_arch_fields() {
    // Verify the new script fields parse from YAML.
    let cfg: PackageConfig = serde_yaml::from_str(
        r#"
package_name: test
github_repo: owner/test
scripts:
  preinstall: scripts/pre.sh
  postinstall: scripts/post.sh
  preremove: scripts/prerm.sh
  postremove: scripts/postrm.sh
  pretrans: scripts/pretrans.sh
  posttrans: scripts/posttrans.sh
  verify: scripts/verify.sh
  preupgrade: scripts/preupgrade.sh
  postupgrade: scripts/postupgrade.sh
"#,
    )
    .unwrap();
    assert_eq!(cfg.scripts.preinstall, "scripts/pre.sh");
    assert_eq!(cfg.scripts.postinstall, "scripts/post.sh");
    assert_eq!(cfg.scripts.preremove, "scripts/prerm.sh");
    assert_eq!(cfg.scripts.postremove, "scripts/postrm.sh");
    assert_eq!(cfg.scripts.pretrans, "scripts/pretrans.sh");
    assert_eq!(cfg.scripts.posttrans, "scripts/posttrans.sh");
    assert_eq!(cfg.scripts.verify, "scripts/verify.sh");
    assert_eq!(cfg.scripts.preupgrade, "scripts/preupgrade.sh");
    assert_eq!(cfg.scripts.postupgrade, "scripts/postupgrade.sh");
}

#[test]
fn deb_config_parses_triggers_and_scripts() {
    let cfg: PackageConfig = serde_yaml::from_str(
        r#"
package_name: test
github_repo: owner/test
deb:
  rules: debian/rules
  templates: debian/templates
  config: debian/config
  triggers_interest:
    - some-trigger
  triggers_activate:
    - another-trigger
"#,
    )
    .unwrap();
    assert_eq!(cfg.deb.rules, "debian/rules");
    assert_eq!(cfg.deb.templates, "debian/templates");
    assert_eq!(cfg.deb.config, "debian/config");
    assert_eq!(cfg.deb.triggers_interest, vec!["some-trigger"]);
    assert_eq!(cfg.deb.triggers_activate, vec!["another-trigger"]);
}

#[test]
fn validate_rejects_empty_trigger_entries() {
    let cfg = PackageConfig {
        deb: DebConfig {
            triggers_interest: vec!["".into()],
            ..Default::default()
        },
        ..empty_config()
    };
    assert!(cfg.validate_for_local().is_err());

    let cfg2 = PackageConfig {
        deb: DebConfig {
            triggers_activate: vec!["  ".into()],
            ..Default::default()
        },
        ..empty_config()
    };
    assert!(cfg2.validate_for_local().is_err());
}
