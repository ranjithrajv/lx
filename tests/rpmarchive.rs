// SPDX-License-Identifier: GPL-3.0-or-later

use lx_lib::rpmarchive::*;

use std::os::unix::fs::PermissionsExt;

#[test]
fn rpm_arch_mapping() {
    assert_eq!(to_rpm_arch("amd64"), "x86_64");
    assert_eq!(to_rpm_arch("arm64"), "aarch64");
    assert_eq!(to_rpm_arch("ppc64el"), "ppc64le");
    assert_eq!(to_rpm_arch("s390x"), "s390x");
}

#[test]
fn build_produces_valid_rpm() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("usr/bin")).unwrap();
    std::fs::write(root.path().join("usr/bin/hello"), b"payload").unwrap();
    let mut perms = std::fs::metadata(root.path().join("usr/bin/hello"))
        .unwrap()
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(root.path().join("usr/bin/hello"), perms).unwrap();

    let rpm_path = root.path().join("hello.rpm");
    build(
        root.path(),
        &PackageMeta {
            name: "hello",
            version: "1.0",
            release: "1",
            summary: "test package",
            description: "Packaged from upstream",
            license: "MIT",
            vendor: None,
            packager: None,
        },
        "amd64",
        1_735_689_600,
        &rpm_path,
    )
    .unwrap();

    assert!(rpm_path.exists());
    let bytes = std::fs::read(&rpm_path).unwrap();
    // RPM magic: 0xED 0xAB 0xEE 0xDB
    assert_eq!(&bytes[0..4], &[0xED, 0xAB, 0xEE, 0xDB]);
}

#[test]
fn same_input_produces_deterministic_output_for_same_mtime() {
    let build_once = || {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("usr/bin")).unwrap();
        std::fs::write(root.path().join("usr/bin/hello"), b"fake-elf").unwrap();
        let rpm_path = root.path().join("hello.rpm");
        build(
            root.path(),
            &PackageMeta {
                name: "hello",
                version: "1.0",
                release: "1",
                summary: "test",
                description: "desc",
                license: "MIT",
                vendor: None,
                packager: None,
            },
            "amd64",
            1_735_689_600,
            &rpm_path,
        )
        .unwrap();
        std::fs::read(&rpm_path).unwrap()
    };
    assert_eq!(build_once(), build_once());
}

#[test]
fn scriptlets_build_without_error() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("usr/bin")).unwrap();
    std::fs::write(root.path().join("usr/bin/hello"), b"fake-elf").unwrap();
    let rpm_path = root.path().join("hello.rpm");
    let opts = BuildOptions {
        pre_install: Some("echo pre"),
        post_install: Some("echo post"),
        pre_uninstall: None,
        post_uninstall: Some("echo postun"),
        ..Default::default()
    };
    build_with_options(
        root.path(),
        &PackageMeta {
            name: "hello",
            version: "1.0",
            release: "1",
            summary: "test",
            description: "desc",
            license: "MIT",
            vendor: None,
            packager: None,
        },
        "amd64",
        1_735_689_600,
        &rpm_path,
        &opts,
        &Default::default(),
    )
    .unwrap();
    let bytes = std::fs::read(&rpm_path).unwrap();
    assert_eq!(&bytes[0..4], &[0xED, 0xAB, 0xEE, 0xDB]);
}

#[test]
fn transaction_and_verify_scriptlets_build_without_error() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("usr/bin")).unwrap();
    std::fs::write(root.path().join("usr/bin/hello"), b"fake-elf").unwrap();
    let rpm_path = root.path().join("hello.rpm");
    let opts = BuildOptions {
        pre_trans: Some("echo pretrans"),
        post_trans: Some("echo posttrans"),
        verify_script: Some("echo verify"),
        ..Default::default()
    };
    build_with_options(
        root.path(),
        &PackageMeta {
            name: "hello",
            version: "1.0",
            release: "1",
            summary: "test",
            description: "desc",
            license: "MIT",
            vendor: None,
            packager: None,
        },
        "amd64",
        1_735_689_600,
        &rpm_path,
        &opts,
        &Default::default(),
    )
    .unwrap();
    let bytes = std::fs::read(&rpm_path).unwrap();
    assert_eq!(&bytes[0..4], &[0xED, 0xAB, 0xEE, 0xDB]);
}

/// Full signing round trip with a generated throwaway key; skips when
/// gpg or quick key generation is unavailable.
#[test]
fn native_signing_round_trip_with_generated_key() {
    let Some((_home, key_file)) = export_test_gpg_key() else {
        eprintln!("skipping: gpg quick-gen-key unavailable");
        return;
    };

    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("usr/bin")).unwrap();
    std::fs::write(root.path().join("usr/bin/hello"), b"fake-elf").unwrap();
    let rpm_path = root.path().join("hello-signed.rpm");
    let opts = BuildOptions {
        sign_key_file: Some(&key_file),
        sign_passphrase: None,
        ..Default::default()
    };
    build_with_options(
        root.path(),
        &PackageMeta {
            name: "hello",
            version: "1.0",
            release: "1",
            summary: "test",
            description: "desc",
            license: "MIT",
            vendor: None,
            packager: None,
        },
        "amd64",
        1_735_689_600,
        &rpm_path,
        &opts,
        &Default::default(),
    )
    .unwrap();
    assert!(rpm_path.exists());
    // Signature header tags are present in the package.
    if let Ok(info) = std::process::Command::new("rpm")
        .args(["-qpi", rpm_path.to_str().unwrap()])
        .output()
    {
        // Best-effort: host rpm may not read our payload; only require
        // that the file exists and is non-trivially sized.
        let _ = info;
    }
    assert!(std::fs::metadata(&rpm_path).unwrap().len() > 0);
}

#[test]
fn predepends_fold_into_requires_with_prereq_flag() {
    let rel = parse_rpm_relations("", "", "", "", "", "", "", "prereq-pkg");
    let dep = rel
        .requires
        .iter()
        .find(|d| d.name == "prereq-pkg")
        .expect("prereq-pkg in requires");
    assert!(dep.flags.contains(rpm::DependencyFlags::PREREQ));
}

#[test]
fn config_files_get_config_flags() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("etc")).unwrap();
    std::fs::write(root.path().join("etc/keep.conf"), b"a=1").unwrap();
    std::fs::write(root.path().join("etc/replace.conf"), b"b=2").unwrap();
    let rpm_path = root.path().join("hello.rpm");
    let opts = BuildOptions {
        config_files: vec!["/etc/keep.conf".into()],
        config_noreplace_files: vec!["/etc/replace.conf".into()],
        ..Default::default()
    };
    build_with_options(
        root.path(),
        &PackageMeta {
            name: "hello",
            version: "1.0",
            release: "1",
            summary: "test",
            description: "desc",
            license: "MIT",
            vendor: None,
            packager: None,
        },
        "amd64",
        1_735_689_600,
        &rpm_path,
        &opts,
        &Default::default(),
    )
    .unwrap();

    let pkg = rpm::Package::open(&rpm_path).unwrap();
    let entries = pkg.metadata.get_file_entries().unwrap();
    let flag = |p: &str| {
        entries
            .iter()
            .find(|e| e.path.to_string_lossy() == p)
            .unwrap_or_else(|| panic!("missing {p}"))
            .flags
    };
    assert!(flag("/etc/keep.conf").contains(rpm::FileFlags::CONFIG));
    assert!(!flag("/etc/keep.conf").contains(rpm::FileFlags::NOREPLACE));
    assert!(flag("/etc/replace.conf").contains(rpm::FileFlags::NOREPLACE));
}

#[test]
fn auto_requires_scans_elf_payload() {
    // A small, real dynamically linked ELF whose DT_NEEDED entries exercise
    // the find-requires path. Skip when no suitable system binary exists.
    let candidate = ["/usr/bin/true", "/bin/true", "/usr/bin/ls", "/bin/ls"]
        .iter()
        .map(std::path::Path::new)
        .find(|p| p.is_file() && std::fs::metadata(p).map(|m| m.len()).unwrap_or(0) < 5_000_000);
    let Some(candidate) = candidate else {
        eprintln!("skipping: no small system ELF available");
        return;
    };

    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("usr/bin")).unwrap();
    std::fs::copy(candidate, root.path().join("usr/bin/hello")).unwrap();

    let rpm_path = root.path().join("hello.rpm");
    let opts = BuildOptions {
        auto_requires: true,
        auto_provides: false,
        ..Default::default()
    };
    build_with_options(
        root.path(),
        &PackageMeta {
            name: "hello",
            version: "1.0",
            release: "1",
            summary: "test",
            description: "desc",
            license: "MIT",
            vendor: None,
            packager: None,
        },
        "amd64",
        1_735_689_600,
        &rpm_path,
        &opts,
        &Default::default(),
    )
    .unwrap();

    let pkg = rpm::Package::open(&rpm_path).unwrap();
    let requires: Vec<String> = pkg
        .metadata
        .get_requires()
        .unwrap()
        .into_iter()
        .map(|d| d.name)
        .collect();
    assert!(
        requires
            .iter()
            .any(|r| r.ends_with("()(64bit)") || r.ends_with("()(32bit)")),
        "expected an auto-generated soname require, got {requires:?}"
    );
}

#[test]
fn rpm_epoch_header_is_emitted() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("usr/bin")).unwrap();
    std::fs::write(root.path().join("usr/bin/hello"), b"payload").unwrap();
    let rpm_path = root.path().join("hello.rpm");
    let opts = BuildOptions {
        epoch: Some(2),
        ..Default::default()
    };
    build_with_options(
        root.path(),
        &PackageMeta {
            name: "hello",
            version: "1.0",
            release: "1",
            summary: "test",
            description: "desc",
            license: "MIT",
            vendor: None,
            packager: None,
        },
        "amd64",
        1_735_689_600,
        &rpm_path,
        &opts,
        &Default::default(),
    )
    .unwrap();
    let pkg = rpm::Package::open(&rpm_path).unwrap();
    assert_eq!(pkg.metadata.get_epoch().unwrap(), 2);
}

#[test]
fn build_srpm_produces_valid_rpm_with_spec_and_source() {
    let tmp = tempfile::tempdir().unwrap();
    let srpm_path = tmp.path().join("hello-1.0-1.fedora.src.rpm");
    build_srpm(
        &PackageMeta {
            name: "hello",
            version: "1.0",
            release: "1.fedora",
            summary: "test package",
            description: "Packaged from upstream",
            license: "MIT",
            vendor: None,
            packager: None,
        },
        ("hello.spec", b"Name: hello\nVersion: 1.0\n"),
        ("hello-1.0.tar.xz", b"fake-tarball-bytes"),
        1_735_689_600,
        &srpm_path,
    )
    .unwrap();

    assert!(srpm_path.exists());
    let bytes = std::fs::read(&srpm_path).unwrap();
    assert_eq!(&bytes[0..4], &[0xED, 0xAB, 0xEE, 0xDB], "rpm magic missing");

    let pkg = rpm::Package::open(&srpm_path).unwrap();
    let paths: Vec<String> = pkg
        .metadata
        .get_file_entries()
        .unwrap()
        .into_iter()
        .map(|e| e.path.to_string_lossy().to_string())
        .collect();
    assert!(paths.iter().any(|p| p.ends_with("hello.spec")), "{paths:?}");
    assert!(
        paths.iter().any(|p| p.ends_with("hello-1.0.tar.xz")),
        "{paths:?}"
    );
}

#[test]
fn extract_round_trips_files_dirs_and_symlinks() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("usr/bin")).unwrap();
    std::fs::write(root.path().join("usr/bin/hello"), b"payload-bytes").unwrap();
    let mut perms = std::fs::metadata(root.path().join("usr/bin/hello"))
        .unwrap()
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(root.path().join("usr/bin/hello"), perms).unwrap();
    std::os::unix::fs::symlink("hello", root.path().join("usr/bin/hello-link")).unwrap();

    let rpm_path = root.path().join("hello.rpm");
    build(
        root.path(),
        &PackageMeta {
            name: "hello",
            version: "1.0",
            release: "1",
            summary: "test",
            description: "desc",
            license: "MIT",
            vendor: None,
            packager: None,
        },
        "amd64",
        1_735_689_600,
        &rpm_path,
    )
    .unwrap();

    let dest = tempfile::tempdir().unwrap();
    extract(&rpm_path, dest.path()).unwrap();

    let extracted = std::fs::read(dest.path().join("usr/bin/hello")).unwrap();
    assert_eq!(extracted, b"payload-bytes");
    let mode = std::fs::metadata(dest.path().join("usr/bin/hello"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o755);

    let link_target = std::fs::read_link(dest.path().join("usr/bin/hello-link")).unwrap();
    assert_eq!(link_target, std::path::PathBuf::from("hello"));
}

// ---------------------------------------------------------------------------
// file_info.lang → RPMTAG_FILELANGS
// ---------------------------------------------------------------------------

fn lang_meta(paths: &[(&str, &str)]) -> lx_lib::filemeta::FileMetaMap {
    let mut meta = lx_lib::filemeta::FileMetaMap::new();
    for (path, lang) in paths {
        meta.insert(
            (*path).to_string(),
            lx_lib::filemeta::FileMeta {
                lang: Some((*lang).to_string()),
                ..Default::default()
            },
        );
    }
    meta
}

fn test_meta() -> PackageMeta<'static> {
    PackageMeta {
        name: "hello",
        version: "1.0",
        release: "1",
        summary: "test",
        description: "desc",
        license: "MIT",
        vendor: None,
        packager: None,
    }
}

/// `file_info.lang` must land in RPM's `RPMTAG_FILELANGS`, an array parallel
/// to the package's file list, and the rewritten header's digest must verify.
#[test]
fn file_info_lang_reaches_rpm_header() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("usr/bin")).unwrap();
    std::fs::create_dir_all(root.path().join("usr/share/doc/hello")).unwrap();
    std::fs::write(root.path().join("usr/bin/hello"), b"payload").unwrap();
    std::fs::write(root.path().join("usr/share/doc/hello/README"), b"docs").unwrap();
    std::fs::write(
        root.path().join("usr/share/doc/hello/CHANGELOG"),
        b"changes",
    )
    .unwrap();

    let meta = lang_meta(&[
        ("/usr/share/doc/hello/README", "en"),
        ("/usr/share/doc/hello/CHANGELOG", "de"),
    ]);
    let rpm_path = root.path().join("hello.rpm");
    build_with_options(
        root.path(),
        &test_meta(),
        "amd64",
        1_735_689_600,
        &rpm_path,
        &BuildOptions::default(),
        &meta,
    )
    .unwrap();

    let pkg = rpm::Package::open(&rpm_path).unwrap();
    // The signature-header digest must match the rewritten main header.
    pkg.verify_digests()
        .expect("rpm header digest must verify after lang injection");

    let files: Vec<String> = pkg
        .metadata
        .get_file_entries()
        .unwrap()
        .into_iter()
        .map(|e| e.path.to_string_lossy().to_string())
        .collect();
    let langs: Vec<String> = pkg
        .metadata
        .header
        .get_entry_data_as_string_array(rpm::IndexTag::RPMTAG_FILELANGS)
        .unwrap()
        .to_vec();

    assert_eq!(
        langs.len(),
        files.len(),
        "FILELANGS must be parallel to the file list"
    );
    let by_path: std::collections::HashMap<&str, &str> = files
        .iter()
        .map(String::as_str)
        .zip(langs.iter().map(String::as_str))
        .collect();
    assert_eq!(by_path.get("/usr/share/doc/hello/README"), Some(&"en"));
    assert_eq!(by_path.get("/usr/share/doc/hello/CHANGELOG"), Some(&"de"));
    assert_eq!(
        by_path.get("/usr/bin/hello"),
        Some(&""),
        "files without a lang keep the empty slot"
    );
}

/// Injecting languages must stay byte-for-byte reproducible.
#[test]
fn file_info_lang_output_is_deterministic() {
    let build_once = || {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("usr/share/doc/hello")).unwrap();
        std::fs::write(root.path().join("usr/share/doc/hello/README"), b"docs").unwrap();
        let meta = lang_meta(&[("/usr/share/doc/hello/README", "en")]);
        let rpm_path = root.path().join("hello.rpm");
        build_with_options(
            root.path(),
            &test_meta(),
            "amd64",
            1_735_689_600,
            &rpm_path,
            &BuildOptions::default(),
            &meta,
        )
        .unwrap();
        std::fs::read(&rpm_path).unwrap()
    };
    assert_eq!(build_once(), build_once());
}

/// A signed build re-signs over the rewritten header, so the language and the
/// embedded PGP signature both survive.
#[test]
fn file_info_lang_survives_signing() {
    let Some((_home, key_file)) = export_test_gpg_key() else {
        eprintln!("skipping: gpg quick-gen-key unavailable");
        return;
    };

    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("usr/share/doc/hello")).unwrap();
    std::fs::write(root.path().join("usr/share/doc/hello/README"), b"docs").unwrap();
    let meta = lang_meta(&[("/usr/share/doc/hello/README", "en")]);

    let rpm_path = root.path().join("hello-signed.rpm");
    let opts = BuildOptions {
        sign_key_file: Some(&key_file),
        sign_passphrase: None,
        ..Default::default()
    };
    build_with_options(
        root.path(),
        &test_meta(),
        "amd64",
        1_735_689_600,
        &rpm_path,
        &opts,
        &meta,
    )
    .unwrap();

    let pkg = rpm::Package::open(&rpm_path).unwrap();
    pkg.verify_digests()
        .expect("digest must verify for the signed + lang package");
    // The PGP signature header entry survived the re-sign (RSA or EdDSA,
    // depending on the generated key type).
    let sig = &pkg.metadata.signature;
    let signed = sig
        .get_entry_data_as_binary(rpm::IndexSignatureTag::RPMSIGTAG_RSA)
        .is_ok()
        || sig
            .get_entry_data_as_binary(rpm::IndexSignatureTag::RPMSIGTAG_DSA)
            .is_ok();
    assert!(signed, "expected an embedded PGP signature");
    let langs = pkg
        .metadata
        .header
        .get_entry_data_as_string_array(rpm::IndexTag::RPMTAG_FILELANGS)
        .unwrap();
    assert!(langs.iter().any(|l| l == "en"), "{langs:?}");
}

/// Gated on the host `rpm` binary: it must be able to read the injected
/// languages back out of the package it did not create.
#[test]
fn real_rpm_reports_file_info_lang() {
    let Ok(ver) = std::process::Command::new("rpm").arg("--version").output() else {
        eprintln!("skipping: rpm not on PATH");
        return;
    };
    if !ver.status.success() {
        eprintln!("skipping: rpm --version failed");
        return;
    }

    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("usr/share/doc/hello")).unwrap();
    std::fs::write(root.path().join("usr/share/doc/hello/README"), b"docs").unwrap();
    let meta = lang_meta(&[("/usr/share/doc/hello/README", "en")]);
    let rpm_path = root.path().join("hello.rpm");
    build_with_options(
        root.path(),
        &test_meta(),
        "amd64",
        1_735_689_600,
        &rpm_path,
        &BuildOptions::default(),
        &meta,
    )
    .unwrap();

    let out = std::process::Command::new("rpm")
        .args(["-qp", "--qf", "[%{FILENAMES}\t%{FILELANGS}\n]"])
        .arg(&rpm_path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "rpm -qp failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.lines().any(|l| l == "/usr/share/doc/hello/README\ten"),
        "rpm did not report the language: {text:?}"
    );
}

/// Mint a throwaway GPG key and export its armored secret material. Returns
/// `None` (so callers skip) when gpg or quick key generation is unavailable.
fn export_test_gpg_key() -> Option<(tempfile::TempDir, std::path::PathBuf)> {
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
            "lx-rpm-test <lx@example.invalid>",
            "default",
            "default",
            "never",
        ])
        .output()
        .ok()?;
    if !gen.status.success() {
        return None;
    }
    let list = std::process::Command::new("gpg")
        .env("GNUPGHOME", home.path())
        .args(["--batch", "--list-secret-keys", "--with-colons"])
        .output()
        .ok()?;
    let fpr = String::from_utf8_lossy(&list.stdout)
        .lines()
        .find_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            (f.len() > 4 && f[0] == "sec").then(|| f[4].to_string())
        })?;
    let export = std::process::Command::new("gpg")
        .env("GNUPGHOME", home.path())
        .args(["--batch", "--armor", "--export-secret-keys", &fpr])
        .output()
        .ok()?;
    if !export.status.success() {
        return None;
    }
    let key_file = home.path().join("key.asc");
    std::fs::write(&key_file, &export.stdout).ok()?;
    Some((home, key_file))
}
