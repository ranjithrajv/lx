// SPDX-License-Identifier: GPL-3.0-or-later

//! CLI regrouping guards: the new groups exist, the moved names stay hidden
//! but functional, and the top-level help no longer advertises them.

use predicates::prelude::*;

fn lx() -> assert_cmd::Command {
    assert_cmd::Command::cargo_bin("lx").unwrap()
}

#[test]
fn deps_group_exposes_scan_and_resolve() {
    lx().args(["deps", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("scan"))
        .stdout(predicate::str::contains("resolve"));
}

#[test]
fn migrate_group_exposes_lpt_and_native() {
    lx().args(["migrate", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("lpt"))
        .stdout(predicate::str::contains("native"));
}

#[test]
fn schema_is_canonical_with_aliases() {
    for name in ["schema", "json-schema", "jsonschema"] {
        lx().args([name, "--help"]).assert().success();
    }
}

#[test]
fn get_includes_rollback() {
    lx().args(["get", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("rollback"));
}

#[test]
fn moved_names_still_work_as_hidden_shims() {
    for name in [
        "scan-deps",
        "shlibdeps",
        "go-native",
        "reinstall",
        "discover",
    ] {
        lx().args([name, "--help"]).assert().success();
    }
}

#[test]
fn init_exposes_from_scaffold() {
    lx().args(["init", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--from <REPO>"))
        .stdout(predicate::str::contains("--from-aur"));
}

#[test]
fn top_level_help_does_not_advertise_moved_names() {
    lx().arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("deps"))
        .stdout(predicate::str::contains("migrate"))
        .stdout(predicate::str::contains("schema"))
        // Command-list entries are indented two spaces; match those so the
        // descriptions (e.g. "dpkg-shlibdeps parity") don't false-positive.
        .stdout(predicate::str::contains("  go-native").not())
        .stdout(predicate::str::contains("  scan-deps").not())
        .stdout(predicate::str::contains("  shlibdeps").not())
        .stdout(predicate::str::contains("  discover").not());
}
