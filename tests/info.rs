// SPDX-License-Identifier: GPL-3.0-or-later

use predicates::prelude::*;

#[test]
fn cli_reports_host_info() {
    assert_cmd::Command::cargo_bin("lx")
        .unwrap()
        .arg("info")
        .assert()
        .success()
        .stdout(predicate::str::contains("Host information"))
        .stdout(predicate::str::contains("Package manager"));
}

#[test]
fn cli_emits_json() {
    let out = assert_cmd::Command::cargo_bin("lx")
        .unwrap()
        .args(["info", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid JSON");
    // At minimum the detected architecture should be present on any Linux
    // host where `uname -m` works; the object must still parse elsewhere.
    assert!(value.is_object());
}
