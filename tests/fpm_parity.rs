// SPDX-License-Identifier: GPL-3.0-or-later

use lx_lib::config::PackageConfig;

fn empty_config() -> PackageConfig {
    PackageConfig {
        package_name: "test".into(),
        github_repo: "owner/test".into(),
        ..Default::default()
    }
}

#[test]
fn scripts_parse_upgrade_fields() {
    let cfg: PackageConfig = serde_yaml::from_str(
        r#"
package_name: test
github_repo: owner/test
scripts:
  preinstall: scripts/preinst.sh
  postinstall: scripts/postinst.sh
  preupgrade_script: scripts/preupgrade.sh
  postupgrade_script: scripts/postupgrade.sh
"#,
    )
    .unwrap();
    assert_eq!(cfg.scripts.preinstall, "scripts/preinst.sh");
    assert_eq!(cfg.scripts.postinstall, "scripts/postinst.sh");
    assert_eq!(cfg.scripts.preupgrade_script, "scripts/preupgrade.sh");
    assert_eq!(cfg.scripts.postupgrade_script, "scripts/postupgrade.sh");
}

#[test]
fn rpm_config_parses_triggers() {
    let cfg: PackageConfig = serde_yaml::from_str(
        r#"
package_name: test
github_repo: owner/test
rpm:
  trigger_pre_install:
    - "bash: scripts/trigger-prein.sh"
  trigger_post_install:
    - "bash: scripts/triggerin.sh"
  trigger_pre_uninstall:
    - "bash: scripts/triggerun.sh"
  trigger_post_uninstall:
    - "bash: scripts/triggerpostun.sh"
"#,
    )
    .unwrap();
    assert_eq!(
        cfg.rpm.trigger_pre_install,
        vec!["bash: scripts/trigger-prein.sh"]
    );
    assert_eq!(
        cfg.rpm.trigger_post_install,
        vec!["bash: scripts/triggerin.sh"]
    );
    assert_eq!(
        cfg.rpm.trigger_pre_uninstall,
        vec!["bash: scripts/triggerun.sh"]
    );
    assert_eq!(
        cfg.rpm.trigger_post_uninstall,
        vec!["bash: scripts/triggerpostun.sh"]
    );
}

#[test]
fn validate_rejects_trigger_without_colon() {
    let cfg = PackageConfig {
        rpm: lx_lib::config::RpmConfig {
            trigger_post_install: vec!["invalid-no-colon".into()],
            ..Default::default()
        },
        ..empty_config()
    };
    assert!(cfg.validate_for_local().is_err());
}

#[test]
fn validate_accepts_trigger_with_colon() {
    let cfg = PackageConfig {
        rpm: lx_lib::config::RpmConfig {
            trigger_post_install: vec!["bash: scripts/trigger.sh".into()],
            ..Default::default()
        },
        ..empty_config()
    };
    assert!(cfg.validate_for_local().is_ok());
}

#[test]
fn template_scripts_field_parses() {
    let cfg: PackageConfig = serde_yaml::from_str(
        r#"
package_name: test
github_repo: owner/test
template_scripts: true
"#,
    )
    .unwrap();
    assert!(cfg.template_scripts);

    let cfg_default = empty_config();
    assert!(!cfg_default.template_scripts);
}

#[test]
fn deb_config_still_parses_trigger_await_variants() {
    let cfg: PackageConfig = serde_yaml::from_str(
        r#"
package_name: test
github_repo: owner/test
deb:
  triggers_interest:
    - some-trigger
  triggers_interest_await:
    - await-trigger
  triggers_activate_await:
    - activate-await
"#,
    )
    .unwrap();
    assert_eq!(cfg.deb.triggers_interest, vec!["some-trigger"]);
    assert_eq!(cfg.deb.triggers_interest_await, vec!["await-trigger"]);
    assert_eq!(cfg.deb.triggers_activate_await, vec!["activate-await"]);
}
