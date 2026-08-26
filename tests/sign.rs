use lpt_lib::sign::*;
use std::path::{Path, PathBuf};

#[test]
fn sig_path_is_sibling_dot_sig() {
    assert_eq!(
        sig_path_for(Path::new("/out/eza_1.0_amd64.deb")),
        PathBuf::from("/out/eza_1.0_amd64.deb.sig")
    );
}

/// Full round trip with a throwaway key: skip when gpg is unavailable
/// or quick key generation isn't supported by the local gpg version.
#[test]
fn gpg_detach_sign_round_trip_with_generated_key() {
    let Ok(_) = std::process::Command::new("gpg").arg("--version").output() else {
        eprintln!("skipping: gpg not on PATH");
        return;
    };
    let home = match tempfile::tempdir() {
        Ok(h) => h,
        Err(_) => return,
    };
    let gen = std::process::Command::new("gpg")
        .env("GNUPGHOME", home.path())
        .args([
            "--batch",
            "--pinentry-mode",
            "loopback",
            "--passphrase",
            "",
            "--quick-gen-key",
            "lpt-test <lpt@example.invalid>",
            "default",
            "default",
            "never",
        ])
        .output();
    let Ok(gen) = gen else { return };
    if !gen.status.success() {
        eprintln!("skipping: quick-gen-key unsupported here");
        return;
    }
    let fpr = match key_id_from_home(home.path()) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("skipping: {e:#}");
            return;
        }
    };

    let dir = tempfile::tempdir().unwrap();
    let artifact = dir.path().join("hello.deb");
    std::fs::write(&artifact, b"payload").unwrap();
    // Export the secret key so we exercise the temp-GNUPGHOME import path.
    let export = std::process::Command::new("gpg")
        .env("GNUPGHOME", home.path())
        .args(["--batch", "--armor", "--export-secret-keys", &fpr])
        .output()
        .unwrap();
    assert!(export.status.success());
    let key_file = dir.path().join("key.asc");
    std::fs::write(&key_file, &export.stdout).unwrap();

    let req = SignRequest {
        key_file: &key_file,
        key_id: "",
        passphrase: None,
    };
    let sig = gpg_detach_sign(&artifact, &req).unwrap();
    assert!(sig.exists());
    assert!(sig.to_string_lossy().ends_with(".sig"));
    assert!(!std::fs::read(&sig).unwrap().is_empty());
}
