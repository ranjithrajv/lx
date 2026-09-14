// SPDX-License-Identifier: GPL-3.0-or-later

use lx_lib::plugins::build_system::{all_build_systems, detect_build_system, get_build_system};

#[test]
fn registry_contains_autotools_and_make() {
    let names = all_build_systems()
        .iter()
        .map(|b| b.name())
        .collect::<Vec<_>>();
    for expected in [
        "cmake",
        "cargo",
        "go",
        "meson",
        "autotools",
        "make",
        "custom",
    ] {
        assert!(names.contains(&expected), "missing build system {expected}");
        assert!(get_build_system(expected).is_some());
    }
    assert!(get_build_system("AUTOTOOLS").is_some(), "case-insensitive");
    assert!(get_build_system("nope").is_none());
}

#[test]
fn detects_autotools_from_configure() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("configure"), b"#!/bin/sh\n").unwrap();
    assert_eq!(detect_build_system(tmp.path()).unwrap().name(), "autotools");
}

#[test]
fn detects_autotools_from_configure_ac_and_autogen() {
    for marker in ["configure.ac", "configure.in", "autogen.sh"] {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(marker), b"").unwrap();
        assert_eq!(
            detect_build_system(tmp.path()).unwrap().name(),
            "autotools",
            "marker {marker}"
        );
    }
}

#[test]
fn detects_plain_makefile_as_make() {
    for marker in ["Makefile", "makefile", "GNUmakefile"] {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(marker), b"all:\n\ttrue\n").unwrap();
        assert_eq!(detect_build_system(tmp.path()).unwrap().name(), "make");
    }
}

#[test]
fn more_specific_systems_win_over_make() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("CMakeLists.txt"), b"project(x)\n").unwrap();
    std::fs::write(tmp.path().join("Makefile"), b"all:\n\ttrue\n").unwrap();
    assert_eq!(detect_build_system(tmp.path()).unwrap().name(), "cmake");
}

#[test]
fn empty_tree_detects_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(detect_build_system(tmp.path()).is_none());
}
