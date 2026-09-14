// SPDX-License-Identifier: GPL-3.0-or-later

//! Commands whose input defaults to `package.yaml` (`build`, `publish`,
//! `validate`, `deps scan`) show their own help when invoked bare with no such
//! file, instead of a bare "failed to read config file" error.

use predicates::prelude::*;

fn lx() -> assert_cmd::Command {
    assert_cmd::Command::cargo_bin("lx").unwrap()
}

/// Each of these, run with no `package.yaml` present, must succeed and print
/// its own usage line (proving the right subcommand's help is rendered).
#[test]
fn bare_config_consumers_show_their_help() {
    for (args, usage) in [
        (&["build"][..], "Usage: lx build"),
        (&["publish"][..], "Usage: lx publish"),
        (&["validate"][..], "Usage: lx validate"),
        (&["deps", "scan"][..], "Usage: lx deps scan"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        lx().current_dir(dir.path())
            .args(args)
            .assert()
            .success()
            .stdout(predicate::str::contains(usage))
            .stdout(predicate::str::contains("Path to package.yaml"));
    }
}

#[test]
fn bare_build_help_mentions_its_flags() {
    let dir = tempfile::tempdir().unwrap();
    lx().current_dir(dir.path())
        .arg("build")
        .assert()
        .success()
        .stdout(predicate::str::contains("--from-dir"));
}

#[test]
fn explicit_missing_config_still_errors() {
    let dir = tempfile::tempdir().unwrap();
    for command in ["build", "publish", "validate"] {
        lx().current_dir(dir.path())
            .args([command, "missing.yaml"])
            .assert()
            .failure()
            .stdout(predicate::str::contains("Usage: lx").not())
            .stderr(predicate::str::contains("failed to read config file"));
    }
}

#[test]
fn an_existing_config_is_still_loaded() {
    let dir = tempfile::tempdir().unwrap();
    // Malformed on purpose: the point is that an existing config is loaded
    // (and errors on its own merits), never silently replaced by the help.
    std::fs::write(dir.path().join("package.yaml"), "package_name: [").unwrap();
    lx().current_dir(dir.path())
        .arg("build")
        .assert()
        .failure()
        .stdout(predicate::str::contains("Usage: lx build").not())
        .stderr(predicate::str::contains("failed to parse package.yaml"));
}
