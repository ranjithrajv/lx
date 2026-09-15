// SPDX-License-Identifier: GPL-3.0-or-later

//! Functional tests for `lx capture` (`checkinstall`-style packaging from an
//! install command's `$DESTDIR`).

use assert_cmd::Command;

#[test]
fn capture_packages_an_install_command_output() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("dist");

    // The command installs a script and a config file into `$DESTDIR`.
    let script = "mkdir -p \"$DESTDIR/usr/bin\" \"$DESTDIR/etc\"; \
                  printf '#!/bin/sh\\necho hi\\n' > \"$DESTDIR/usr/bin/hello\"; \
                  chmod 755 \"$DESTDIR/usr/bin/hello\"; \
                  echo 'key=value' > \"$DESTDIR/etc/hello.conf\"";

    Command::cargo_bin("lx")
        .unwrap()
        .args([
            "capture",
            "--name",
            "hello",
            "--version",
            "1.0",
            "--format",
            "deb",
            "--output",
        ])
        .arg(&out)
        .args(["--exclude", "etc/*"])
        .arg(script)
        .assert()
        .success()
        .stdout(predicates::str::contains("excluded etc/hello.conf"));

    let debs: Vec<String> = std::fs::read_dir(&out)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".deb"))
        .collect();
    assert!(!debs.is_empty(), "expected a .deb, got {debs:?}");
}

#[test]
fn capture_requires_a_command() {
    Command::cargo_bin("lx")
        .unwrap()
        .args(["capture", "--name", "x", "--version", "1.0"])
        .assert()
        .failure();
}
