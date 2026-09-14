// SPDX-License-Identifier: GPL-3.0-or-later

use std::cmp::Ordering;
use std::collections::BTreeSet;

use lx_lib::shlibdeps::*;

fn need(soname: &str, symbols: &[(&str, &str)]) -> LibNeeded {
    LibNeeded {
        soname: soname.to_string(),
        symbols: symbols
            .iter()
            .map(|(s, v)| (s.to_string(), v.to_string()))
            .collect(),
    }
}

fn write_db(files: &[(&str, &str)]) -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let info = tmp.path().join("info");
    std::fs::create_dir_all(&info).unwrap();
    for (name, body) in files {
        std::fs::write(info.join(name), body).unwrap();
    }
    tmp
}

#[test]
fn debian_version_ordering() {
    assert_eq!(debian_version_cmp("1.0", "1.1"), Ordering::Less);
    // Numeric, not lexicographic: 1.9 < 1.10.
    assert_eq!(debian_version_cmp("1.9", "1.10"), Ordering::Less);
    assert_eq!(debian_version_cmp("1.2.3", "1.2.3"), Ordering::Equal);
    // Epoch wins over everything else.
    assert_eq!(debian_version_cmp("1:1.0", "2.0"), Ordering::Greater);
    // '~' sorts before the empty string, i.e. before a release.
    assert_eq!(debian_version_cmp("1.0~rc1", "1.0"), Ordering::Less);
    // A revision sorts after no revision.
    assert_eq!(debian_version_cmp("1.0", "1.0-1"), Ordering::Less);
}

#[test]
fn symbols_file_yields_versioned_relations() {
    let tmp = write_db(&[(
        "libfoo1.symbols",
        "libfoo.so.1 libfoo1 #MINVER#\n foo@Base 1.2.3\n bar@FOO_1.1 2.0\n",
    )]);
    let db = ShlibsDb::open(tmp.path());

    let res = resolve(
        &[need("libfoo.so.1", &[("bar", "FOO_1.1")])],
        &db,
        None,
        &BTreeSet::new(),
    );
    assert_eq!(res.relations, vec!["libfoo1 (>= 2.0)"]);
    assert!(res.unresolved.is_empty());

    let res = resolve(
        &[need("libfoo.so.1", &[("foo", "Base")])],
        &db,
        None,
        &BTreeSet::new(),
    );
    assert_eq!(res.relations, vec!["libfoo1 (>= 1.2.3)"]);

    // Highest minimum-version across all required symbols.
    let res = resolve(
        &[need("libfoo.so.1", &[("foo", "Base"), ("bar", "FOO_1.1")])],
        &db,
        None,
        &BTreeSet::new(),
    );
    assert_eq!(res.relations, vec!["libfoo1 (>= 2.0)"]);

    // Unversioned need with a `#MINVER#` header -> package, no version.
    let res = resolve(&[need("libfoo.so.1", &[])], &db, None, &BTreeSet::new());
    assert_eq!(res.relations, vec!["libfoo1"]);
}

#[test]
fn shlibs_file_is_used_when_no_symbols() {
    let tmp = write_db(&[("libbar1.shlibs", "libbar 2 libbar2 (>= 3.4.5)\n")]);
    let db = ShlibsDb::open(tmp.path());

    let res = resolve(&[need("libbar.so.2", &[])], &db, None, &BTreeSet::new());
    assert_eq!(res.relations, vec!["libbar2 (>= 3.4.5)"]);
}

#[test]
fn unresolved_exclusions_and_self_provided() {
    let tmp = write_db(&[(
        "libfoo1.symbols",
        "libfoo.so.1 libfoo1 #MINVER#\n foo@Base 1.2.3\n",
    )]);
    let db = ShlibsDb::open(tmp.path());

    // No dependency information: the fail-closed signal.
    let res = resolve(&[need("libnope.so.9", &[])], &db, None, &BTreeSet::new());
    assert!(res.relations.is_empty());
    assert_eq!(res.unresolved, vec!["libnope.so.9"]);

    // A package must not depend on itself.
    let res = resolve(
        &[need("libfoo.so.1", &[])],
        &db,
        Some("libfoo1"),
        &BTreeSet::new(),
    );
    assert!(res.relations.is_empty());
    assert!(res.resolved_names.is_empty());

    // A library the package ships itself is not an external dependency.
    let mut provided = BTreeSet::new();
    provided.insert("libfoo.so.1".to_string());
    let res = resolve(&[need("libfoo.so.1", &[])], &db, None, &provided);
    assert!(res.relations.is_empty());
    assert!(res.unresolved.is_empty());
}

#[test]
fn real_binary_reports_symbol_versions() {
    let Ok(bytes) = std::fs::read("/bin/ls") else {
        eprintln!("skipping: /bin/ls not found");
        return;
    };
    let libs = needed_libraries_verbose(&bytes).unwrap();
    if libs.is_empty() {
        eprintln!("skipping: /bin/ls appears to be statically linked");
        return;
    }
    let libc = libs
        .iter()
        .find(|l| l.soname.starts_with("libc.so"))
        .expect("libc must be among a dynamically linked binary's DT_NEEDED");
    assert!(
        !libc.symbols.is_empty(),
        "libc imports must carry symbol versions (VERNEED)"
    );
}

#[test]
fn scan_records_self_provided_sonames() {
    let tmp = tempfile::tempdir().unwrap();
    let lib = tmp.path().join("libx.so.1");
    std::fs::write(&lib, b"\x7fELFnot-really-an-elf").unwrap();

    let scan = scan_elfs(&[lib]);
    assert!(scan.provided.contains("libx.so.1"));
    assert!(scan.needs.is_empty());
}

#[test]
fn cli_errors_on_empty_directory() {
    let tmp = tempfile::tempdir().unwrap();
    assert_cmd::Command::cargo_bin("lx")
        .unwrap()
        .arg("shlibdeps")
        .arg(tmp.path())
        .assert()
        .failure();
}

/// The standalone command is fail-closed without dependency information, and
/// `--ignore-missing-info` downgrades that to a warning. Skips when the
/// probe binary has only essential libraries, so it works on any host.
#[test]
fn cli_is_fail_closed_without_dependency_info() {
    let Ok(bytes) = std::fs::read("/bin/ls") else {
        eprintln!("skipping: /bin/ls not found");
        return;
    };
    let non_essential = needed_libraries_verbose(&bytes)
        .unwrap_or_default()
        .into_iter()
        .any(|l| !lx_lib::elfdeps::is_essential_libc_soname(&l.soname));
    if !non_essential {
        eprintln!("skipping: /bin/ls has no non-essential shared libraries");
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    assert_cmd::Command::cargo_bin("lx")
        .unwrap()
        .args(["shlibdeps", "--admindir"])
        .arg(tmp.path())
        .arg("/bin/ls")
        .assert()
        .failure()
        .stderr(predicates::str::contains("no dependency information found"));

    assert_cmd::Command::cargo_bin("lx")
        .unwrap()
        .args(["shlibdeps", "--ignore-missing-info", "--admindir"])
        .arg(tmp.path())
        .arg("/bin/ls")
        .assert()
        .success();
}
