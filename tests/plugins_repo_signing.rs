// SPDX-License-Identifier: GPL-3.0-or-later

//! Index signing for the non-apt repository formats (and a corrected apt
//! `InRelease`). Each test generates a throwaway gpg key and skips cleanly
//! when gpg (or, for rpm, the `rpm` CLI) isn't available.

use lx_lib::plugins::package_index::{get_index_backend, IndexOptions};
use lx_lib::repo::{run, RepoArgs};
use std::path::{Path, PathBuf};

/// Export an armored throwaway secret key to `<out_dir>/key.asc`, or `None`
/// when gpg / quick key generation isn't available here.
fn make_key(out_dir: &Path) -> Option<PathBuf> {
    std::process::Command::new("gpg")
        .arg("--version")
        .output()
        .ok()?;
    let home = tempfile::tempdir().ok()?;
    let gen = std::process::Command::new("gpg")
        .env("GNUPGHOME", home.path())
        .args([
            "--batch",
            "--pinentry-mode",
            "loopback",
            "--passphrase",
            "",
            "--quick-gen-key",
            "lx-idx <lx@example.invalid>",
            "default",
            "default",
            "never",
        ])
        .output()
        .ok()?;
    if !gen.status.success() {
        return None;
    }
    let fpr = lx_lib::sign::key_id_from_home(home.path()).ok()?;
    let export = std::process::Command::new("gpg")
        .env("GNUPGHOME", home.path())
        .args(["--batch", "--armor", "--export-secret-keys", &fpr])
        .output()
        .ok()?;
    if !export.status.success() {
        return None;
    }
    let key = out_dir.join("key.asc");
    std::fs::write(&key, &export.stdout).ok()?;
    Some(key)
}

fn args(dir: &Path, format: &str, key: Option<PathBuf>) -> RepoArgs {
    RepoArgs {
        dir: dir.to_path_buf(),
        format: format.to_string(),
        suite: "stable".to_string(),
        multi_suite: false,
        components: "main".to_string(),
        origin: "test".to_string(),
        sign_key: key,
        sign_key_id: None,
    }
}

fn stage(root: &Path) {
    std::fs::create_dir_all(root.join("usr/bin")).unwrap();
    std::fs::write(root.join("usr/bin/hello"), b"x").unwrap();
}

fn tiny_deb(dir: &Path) -> PathBuf {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    stage(&root);
    let control = "Package: hello\nVersion: 1.0-1\nArchitecture: amd64\nMaintainer: t <t@e>\nDepends: libc6\nDescription: test\nSection: utils\n";
    let deb = dir.join("hello_1.0-1_amd64.deb");
    lx_lib::debarchive::build(&root, control.as_bytes(), 1_735_689_600, &deb).unwrap();
    deb
}

fn tiny_ipk(dir: &Path) -> PathBuf {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    stage(&root);
    let control = "Package: hello\nVersion: 1.0-1\nArchitecture: x86_64\nMaintainer: t <t@e>\nDepends: libc\nDescription: test\nSection: utils\n";
    let ipk = dir.join("hello_1.0-1_x86_64.ipk");
    lx_lib::ipkarchive::build(&root, control.as_bytes(), 1_735_689_600, &ipk).unwrap();
    ipk
}

fn tiny_pkg(dir: &Path) -> PathBuf {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    stage(&root);
    let pkg = dir.join("hello-1.0-1-x86_64.pkg.tar.zst");
    let meta = lx_lib::archarchive::PackageMeta {
        name: "hello",
        version: "1.0",
        release: "1",
        description: "test",
        url: "https://example.com",
        license: "MIT",
    };
    lx_lib::archarchive::build(&root, &meta, "x86_64", 1_735_689_600, &pkg, None).unwrap();
    pkg
}

#[test]
fn apt_writes_inrelease_and_release_gpg() {
    let tmp = tempfile::tempdir().unwrap();
    tiny_deb(tmp.path());
    let Some(key) = make_key(tmp.path()) else {
        eprintln!("skipping: gpg unavailable");
        return;
    };
    run(args(tmp.path(), "deb", Some(key))).unwrap();
    let inrelease = std::fs::read_to_string(tmp.path().join("InRelease")).unwrap();
    assert!(
        inrelease.contains("BEGIN PGP SIGNED MESSAGE"),
        "InRelease must be inline-clearsigned:\n{inrelease}"
    );
    assert!(tmp.path().join("Release.gpg").exists());
}

#[test]
fn opkg_writes_packages_sig() {
    let tmp = tempfile::tempdir().unwrap();
    tiny_ipk(tmp.path());
    let Some(key) = make_key(tmp.path()) else {
        eprintln!("skipping: gpg unavailable");
        return;
    };
    run(args(tmp.path(), "ipk", Some(key))).unwrap();
    let sig = tmp.path().join("Packages.sig");
    assert!(sig.exists(), "opkg index signature missing");
    assert!(!std::fs::read(&sig).unwrap().is_empty());
}

#[test]
fn pacman_writes_db_sig() {
    let tmp = tempfile::tempdir().unwrap();
    tiny_pkg(tmp.path());
    let Some(key) = make_key(tmp.path()) else {
        eprintln!("skipping: gpg unavailable");
        return;
    };
    run(args(tmp.path(), "arch", Some(key))).unwrap();
    let sig = tmp.path().join("stable.db.tar.gz.sig");
    assert!(sig.exists(), "pacman db signature missing");
    assert!(!std::fs::read(&sig).unwrap().is_empty());
}

#[test]
fn rpm_writes_repomd_asc() {
    // Test the rpm index signer directly (no `rpm` CLI needed): it signs an
    // existing `repodata/repomd.xml` into `repomd.xml.asc`.
    let tmp = tempfile::tempdir().unwrap();
    let repodata = tmp.path().join("repodata");
    std::fs::create_dir_all(&repodata).unwrap();
    std::fs::write(repodata.join("repomd.xml"), b"<repomd/>").unwrap();
    let Some(key) = make_key(tmp.path()) else {
        eprintln!("skipping: gpg unavailable");
        return;
    };
    let opts = IndexOptions {
        suite: "stable",
        origin: "test",
        components: "main",
        sign_key: Some(&key),
        sign_key_id: "",
    };
    get_index_backend("rpm")
        .unwrap()
        .make("rpm")
        .sign_index(tmp.path(), &opts)
        .unwrap();
    let asc = std::fs::read_to_string(repodata.join("repomd.xml.asc")).unwrap();
    assert!(asc.contains("BEGIN PGP SIGNATURE"), "{asc}");
}

#[test]
fn apk_index_signing_is_reported_unsupported() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    stage(&root);
    let apk = tmp.path().join("hello-1.0.0-r1.apk");
    let meta = lx_lib::apkarchive::PackageMeta {
        name: "hello",
        version: "1.0.0-r1",
        description: "test",
        url: "https://example.com",
        license: "MIT",
        depends: &[],
        provides: &[],
        replaces: &[],
    };
    lx_lib::apkarchive::build(&root, &meta, "x86_64", 1_735_689_600, &apk).unwrap();
    let Some(key) = make_key(tmp.path()) else {
        eprintln!("skipping: gpg unavailable");
        return;
    };
    // Succeeds, but produces no signature (Alpine needs an RSA key).
    run(args(tmp.path(), "apk", Some(key))).unwrap();
    assert!(tmp.path().join("APKINDEX.tar.gz").exists());
    assert!(!tmp.path().join("APKINDEX.tar.gz.sig").exists());
}

#[test]
fn no_key_means_no_signatures() {
    let tmp = tempfile::tempdir().unwrap();
    tiny_ipk(tmp.path());
    run(args(tmp.path(), "ipk", None)).unwrap();
    assert!(!tmp.path().join("Packages.sig").exists());
}
