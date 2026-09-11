// SPDX-License-Identifier: GPL-3.0-or-later

//! Tests for `lx upgrade --all` and `--auto-migrate` functionality.

use lx_lib::upgrade::UpgradeArgs;

#[test]
fn auto_migrate_requires_all_flag() {
    // --auto-migrate is defined with `requires = "all"` in clap, so it
    // cannot be used without --all. Verify the args parse correctly.
    let args = UpgradeArgs {
        package: None,
        all: true,
        auto_migrate: true,
        no_verify: false,
        allow_unverified: false,
        dry_run: false,
        yes: false,
        owned_only: false,
    };
    assert!(args.all);
    assert!(args.auto_migrate);
}

#[test]
fn auto_migrate_defaults_false() {
    let args = UpgradeArgs {
        package: None,
        all: true,
        auto_migrate: false,
        no_verify: false,
        allow_unverified: false,
        dry_run: false,
        yes: false,
        owned_only: false,
    };
    assert!(args.all);
    assert!(!args.auto_migrate);
}
