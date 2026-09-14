// SPDX-License-Identifier: GPL-3.0-or-later

use lx_lib::builddeps::*;

#[test]
fn empty_dependency_list_is_a_no_op() {
    // Must not even require a package manager.
    assert!(ensure_build_deps(&[], true, false).is_ok());
    assert!(ensure_build_deps(&[], false, false).is_ok());
}

#[test]
fn report_only_mode_bails_on_a_missing_package() {
    let deps = vec!["definitely-not-a-real-package-zzz".to_string()];
    // Report-only (no --install-build-deps) must never install: it errors.
    assert!(ensure_build_deps(&deps, false, false).is_err());
}

#[test]
fn dry_run_prints_without_installing_or_needing_root() {
    // `install = true` but `dry_run = true`: the command is printed, never
    // run, and no root/sudo is needed.
    let deps = vec!["definitely-not-a-real-package-zzz".to_string()];
    assert!(ensure_build_deps(&deps, true, true).is_ok());
}

#[test]
fn installed_packages_are_not_reinstalled() {
    let Some(pm) = HostPm::detect() else {
        eprintln!("skipping: no supported package manager");
        return;
    };
    // Pick a package that exists on any host of this family.
    let probe = "make";
    let deps = vec![probe.to_string()];
    if missing(pm, &deps).is_empty() {
        // Already installed: ensure is a no-op in either mode.
        assert!(ensure_build_deps(&deps, true, false).is_ok());
    } else {
        eprintln!("skipping: {probe} not installed on this host");
    }
}
