// SPDX-License-Identifier: GPL-3.0-or-later

//! Index signing for every repository format (and a corrected apt
//! `InRelease`), plus Alpine apk package signing. Tests generate throwaway
//! gpg / openssl keys and skip cleanly when those tools are unavailable.

use lx_lib::plugins::package_index::{artifacts_with_ext, get_index_backend, IndexOptions};
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
        format: Some(format.to_string()),
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
fn apk_index_is_rsa_signed() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    stage(&root);
    lx_lib::apkarchive::build(
        &root,
        &apk_meta(),
        "x86_64",
        1_735_689_600,
        &tmp.path().join("hello-1.0.0-r1.apk"),
    )
    .unwrap();

    let arts = artifacts_with_ext(tmp.path(), "apk").unwrap();
    let indexer = get_index_backend("apk").unwrap().make("apk");
    let base = IndexOptions {
        suite: "alpine",
        origin: "test",
        components: "main",
        sign_key: None,
        sign_key_id: "",
    };
    indexer.build_index(tmp.path(), &arts, &base).unwrap();
    let unsigned = std::fs::read(tmp.path().join("APKINDEX.tar.gz")).unwrap();

    let Some((priv_key, pub_key)) = openssl_keypair(tmp.path()) else {
        eprintln!("skipping: openssl unavailable");
        return;
    };
    assert_eq!(lx_lib::sign::apk_key_name(&priv_key, ""), "key.rsa.pub");

    let with_key = IndexOptions {
        suite: "alpine",
        origin: "test",
        components: "main",
        sign_key: Some(&priv_key),
        sign_key_id: "",
    };
    indexer.sign_index(tmp.path(), &with_key).unwrap();
    let signed = std::fs::read(tmp.path().join("APKINDEX.tar.gz")).unwrap();
    assert!(
        signed.len() > unsigned.len(),
        "signature segment not prepended"
    );

    let sig = extract_sign_member(&signed).expect("no .SIGN member in signed index");
    assert!(openssl_verify(&pub_key, &sig, &unsigned, tmp.path()));
}

#[test]
fn apk_package_is_rsa_signed() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    stage(&root);
    let Some((priv_key, pub_key)) = openssl_keypair(tmp.path()) else {
        eprintln!("skipping: openssl unavailable");
        return;
    };

    let member = format!(".SIGN.RSA.{}", lx_lib::sign::apk_key_name(&priv_key, ""));
    let captured = std::sync::Mutex::new(Vec::new());
    let sign = |control_gz: &[u8]| {
        *captured.lock().unwrap() = control_gz.to_vec();
        lx_lib::sign::rsa_sha1_sign(control_gz, &priv_key, None)
    };
    let apk = tmp.path().join("hello-1.0.0-r1.apk");
    let signer = lx_lib::apkarchive::ApkSigner {
        member_name: &member,
        sign: &sign,
    };
    lx_lib::apkarchive::build_with_signature(
        &root,
        &apk_meta(),
        "x86_64",
        1_735_689_600,
        &apk,
        Some(signer),
    )
    .unwrap();

    let signed = std::fs::read(&apk).unwrap();
    let sig = extract_sign_member(&signed).expect("no .SIGN member in signed apk");
    let control = captured.lock().unwrap().clone();
    assert!(!control.is_empty());
    assert!(openssl_verify(&pub_key, &sig, &control, tmp.path()));

    // An unsigned package carries no signature member.
    let plain = tmp.path().join("plain.apk");
    lx_lib::apkarchive::build(&root, &apk_meta(), "x86_64", 1_735_689_600, &plain).unwrap();
    assert!(extract_sign_member(&std::fs::read(&plain).unwrap()).is_none());
}

/// Verify a signed apk with a real `apk` binary when one is provided via
/// `LX_APK_STATIC` (e.g. Alpine's `apk-tools-static`). Skipped otherwise,
/// so normal runs don't need network or Alpine tooling.
#[test]
fn apk_verifies_with_real_apk_static() {
    let Some(apk_static) = std::env::var_os("LX_APK_STATIC") else {
        eprintln!("skipping: set LX_APK_STATIC=<path to apk.static> to run");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    stage(&root);
    let Some((priv_key, pub_key)) = openssl_keypair(tmp.path()) else {
        eprintln!("skipping: openssl unavailable");
        return;
    };

    let key_name = lx_lib::sign::apk_key_name(&priv_key, "");
    let member = format!(".SIGN.RSA.{key_name}");
    let sign = |cgz: &[u8]| lx_lib::sign::rsa_sha1_sign(cgz, &priv_key, None);
    let apk = tmp.path().join("hello-1.0.0-r1.apk");
    let signer = lx_lib::apkarchive::ApkSigner {
        member_name: &member,
        sign: &sign,
    };
    lx_lib::apkarchive::build_with_signature(
        &root,
        &apk_meta(),
        "x86_64",
        1_735_689_600,
        &apk,
        Some(signer),
    )
    .unwrap();

    // apk looks the signing key up as `<keys-dir>/<keyname>`.
    let keys = tmp.path().join("keys");
    std::fs::create_dir_all(&keys).unwrap();
    std::fs::copy(&pub_key, keys.join(&key_name)).unwrap();

    let out = std::process::Command::new(&apk_static)
        .arg("--keys-dir")
        .arg(&keys)
        .arg("verify")
        .arg(&apk)
        .output()
        .expect("failed to run apk.static");
    assert!(
        out.status.success(),
        "apk verify failed ({}):\n{}{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    // A signed APKINDEX is consumed by `apk update` against a local repo
    // laid out as <repo>/<arch>/APKINDEX.tar.gz (+ the package).
    let arch_dir = tmp.path().join("repo/x86_64");
    std::fs::create_dir_all(&arch_dir).unwrap();
    let repo_apk = arch_dir.join(apk.file_name().unwrap());
    std::fs::copy(&apk, &repo_apk).unwrap();
    let indexer = get_index_backend("apk").unwrap().make("apk");
    indexer
        .build_index(
            &arch_dir,
            std::slice::from_ref(&repo_apk),
            &IndexOptions {
                suite: "alpine",
                origin: "test",
                components: "main",
                sign_key: None,
                sign_key_id: "",
            },
        )
        .unwrap();
    indexer
        .sign_index(
            &arch_dir,
            &IndexOptions {
                suite: "alpine",
                origin: "test",
                components: "main",
                sign_key: Some(&priv_key),
                sign_key_id: "",
            },
        )
        .unwrap();

    let root = tmp.path().join("apkroot");
    std::fs::create_dir_all(&root).unwrap();
    // Initialize the (empty) apk database so `update` can write its cache.
    let _ = std::process::Command::new(&apk_static)
        .arg("--root")
        .arg(&root)
        .arg("--arch")
        .arg("x86_64")
        .arg("--no-network")
        .arg("add")
        .arg("--initdb")
        .output();
    let out = std::process::Command::new(&apk_static)
        .arg("--root")
        .arg(&root)
        .arg("--arch")
        .arg("x86_64")
        .arg("--keys-dir")
        .arg(&keys)
        .arg("--repositories-file")
        .arg("/dev/null")
        .arg("--repository")
        .arg(tmp.path().join("repo"))
        .arg("--no-network")
        .arg("update")
        .output()
        .expect("failed to run apk.static");
    assert!(
        out.status.success(),
        "apk update failed ({}):\n{}{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn apk_meta() -> lx_lib::apkarchive::PackageMeta<'static> {
    lx_lib::apkarchive::PackageMeta {
        name: "hello",
        version: "1.0.0-r1",
        description: "test",
        url: "https://example.com",
        license: "MIT",
        depends: &[],
        provides: &[],
        replaces: &[],
    }
}

/// Generate a 2048-bit RSA keypair with openssl; `(private, public)`.
fn openssl_keypair(dir: &Path) -> Option<(PathBuf, PathBuf)> {
    if std::process::Command::new("openssl")
        .arg("version")
        .output()
        .is_err()
    {
        return None;
    }
    let priv_key = dir.join("key.rsa");
    let pub_key = dir.join("key.rsa.pub");
    let g = std::process::Command::new("openssl")
        .args(["genrsa", "-out"])
        .arg(&priv_key)
        .arg("2048")
        .output()
        .ok()?;
    if !g.status.success() {
        return None;
    }
    let p = std::process::Command::new("openssl")
        .args(["rsa", "-in"])
        .arg(&priv_key)
        .args(["-pubout", "-out"])
        .arg(&pub_key)
        .output()
        .ok()?;
    if !p.status.success() {
        return None;
    }
    Some((priv_key, pub_key))
}

fn openssl_verify(pub_key: &Path, sig: &[u8], data: &[u8], dir: &Path) -> bool {
    let sigf = dir.join("verify.sig");
    let dataf = dir.join("verify.data");
    if std::fs::write(&sigf, sig).is_err() || std::fs::write(&dataf, data).is_err() {
        return false;
    }
    std::process::Command::new("openssl")
        .args(["dgst", "-sha1", "-verify"])
        .arg(pub_key)
        .arg("-signature")
        .arg(&sigf)
        .arg(&dataf)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Read the first `.SIGN.*` member out of a concatenated apk gzip stream.
fn extract_sign_member(bytes: &[u8]) -> Option<Vec<u8>> {
    use std::io::Read;
    let gz = flate2::read::MultiGzDecoder::new(bytes);
    let mut tar = tar::Archive::new(gz);
    for entry in tar.entries().ok()? {
        let mut entry = entry.ok()?;
        let name = entry.path().ok()?.to_string_lossy().to_string();
        if name.starts_with(".SIGN.") {
            let mut v = Vec::new();
            entry.read_to_end(&mut v).ok()?;
            return Some(v);
        }
    }
    None
}

#[test]
fn no_key_means_no_signatures() {
    let tmp = tempfile::tempdir().unwrap();
    tiny_ipk(tmp.path());
    run(args(tmp.path(), "ipk", None)).unwrap();
    assert!(!tmp.path().join("Packages.sig").exists());
}
