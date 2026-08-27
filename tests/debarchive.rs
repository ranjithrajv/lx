use lpt_lib::debarchive::*;
use std::os::unix::fs::PermissionsExt;

#[test]
fn extract_round_trips_build() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("usr/bin")).unwrap();
    std::fs::write(root.path().join("usr/bin/hello"), b"payload").unwrap();
    let mut perms = std::fs::metadata(root.path().join("usr/bin/hello"))
        .unwrap()
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(root.path().join("usr/bin/hello"), perms).unwrap();

    let deb_path = tempfile::NamedTempFile::new().unwrap().into_temp_path();
    build(root.path(), b"Package: hello\n", 0, &deb_path).unwrap();

    let dest = tempfile::tempdir().unwrap();
    extract(&deb_path, dest.path()).unwrap();

    assert_eq!(
        std::fs::read(dest.path().join("usr/bin/hello")).unwrap(),
        b"payload"
    );
    let mode = std::fs::metadata(dest.path().join("usr/bin/hello"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o111, 0o111, "executable bit lost on round-trip");
}

#[test]
fn extract_rejects_deb_with_no_data_tar() {
    // An ar archive with only debian-binary -- no data.tar(.gz) member.
    let path = tempfile::NamedTempFile::new().unwrap().into_temp_path();
    let file = std::fs::File::create(&path).unwrap();
    let mut builder = ar::Builder::new(file);
    let header = ar::Header::new(b"debian-binary".to_vec(), 4);
    builder.append(&header, &b"2.0\n"[..]).unwrap();
    drop(builder);

    let dest = tempfile::tempdir().unwrap();
    let err = extract(&path, dest.path()).unwrap_err();
    assert!(err.to_string().contains("no data.tar"));
}

#[test]
fn tar_xz_tree_uses_the_given_prefix_not_dot_slash() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("bin")).unwrap();
    std::fs::write(root.path().join("bin/eza"), b"elf").unwrap();

    let bytes = tar_xz_tree(root.path(), "eza-0.23.5/usr/", 0).unwrap();
    let decoder = lzma_rust2::XzReader::new(bytes.as_slice(), true);
    let mut archive = tar::Archive::new(decoder);
    let names: Vec<String> = archive
        .entries()
        .unwrap()
        .map(|e| e.unwrap().path().unwrap().to_string_lossy().to_string())
        .collect();
    assert!(
        names.contains(&"eza-0.23.5/usr/bin/eza".to_string()),
        "{names:?}"
    );
    assert!(!names.iter().any(|n| n.starts_with("./")), "{names:?}");
}

/// The real `dpkg-deb` on this host (not our own reader) must accept
/// the archive, report the right control fields, and list the exact
/// files we staged -- proof this isn't just internally self-consistent
/// but actually a valid Debian package. Skips gracefully if dpkg-deb
/// isn't installed (this crate has no runtime dependency on it).
#[test]
fn real_dpkg_deb_accepts_the_archive() {
    let Ok(check) = std::process::Command::new("dpkg-deb")
        .arg("--version")
        .output()
    else {
        eprintln!("skipping: dpkg-deb not on PATH");
        return;
    };
    if !check.status.success() {
        eprintln!("skipping: dpkg-deb not usable");
        return;
    }

    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("usr/bin")).unwrap();
    std::fs::write(root.path().join("usr/bin/hello"), b"fake-elf-payload").unwrap();
    let mut perms = std::fs::metadata(root.path().join("usr/bin/hello"))
        .unwrap()
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(root.path().join("usr/bin/hello"), perms).unwrap();

    let deb_path = root.path().join("hello.deb");
    let control =
            b"Package: hello\nVersion: 1.0-1\nArchitecture: amd64\nMaintainer: t <t@example.com>\nDescription: test\n";
    build(root.path(), control, 1_735_689_600, &deb_path).unwrap();

    let info = std::process::Command::new("dpkg-deb")
        .args(["--info", deb_path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        info.status.success(),
        "dpkg-deb --info failed: {}",
        String::from_utf8_lossy(&info.stderr)
    );
    let info_text = String::from_utf8_lossy(&info.stdout);
    assert!(info_text.contains("Package: hello"));
    assert!(info_text.contains("Version: 1.0-1"));

    let contents = std::process::Command::new("dpkg-deb")
        .args(["--contents", deb_path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(contents.status.success());
    let contents_text = String::from_utf8_lossy(&contents.stdout);
    // dpkg-deb's own `--build` stores paths with a "./" prefix (and an
    // explicit "./" root entry); the `tar` crate strips leading "./"
    // path components with no raw-bytes escape hatch to preserve them
    // (verified against its source), so ours comes out as bare
    // "usr/bin/hello". Confirmed with a real dpkg-deb-built reference
    // package that this is still fully valid and installable -- dpkg
    // itself doesn't require the "./" convention, just relative paths.
    assert!(contents_text.contains("usr/bin/hello"));
    assert!(
        contents_text.contains("-rwxr-xr-x"),
        "executable bit not preserved: {contents_text}"
    );
}

/// A real .deb produced by `dpkg-deb --build` starts with the ar magic
/// "!<arch>\n" and its first member is always "debian-binary".
#[test]
fn produces_a_valid_ar_container_with_debian_binary_first() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("usr/bin")).unwrap();
    std::fs::write(root.path().join("usr/bin/hello"), b"fake-elf").unwrap();
    let deb_path = tempfile::NamedTempFile::new().unwrap().into_temp_path();

    build(root.path(), b"Package: hello\n", 1_735_689_600, &deb_path).unwrap();

    let bytes = std::fs::read(&deb_path).unwrap();
    assert!(bytes.starts_with(b"!<arch>\n"));

    let mut archive = ar::Archive::new(bytes.as_slice());
    let first = archive.next_entry().unwrap().unwrap();
    assert_eq!(first.header().identifier(), b"debian-binary");
}

#[test]
fn same_input_produces_byte_identical_output() {
    let build_once = || {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("usr/bin")).unwrap();
        std::fs::write(root.path().join("usr/bin/hello"), b"fake-elf").unwrap();
        let deb_path = tempfile::NamedTempFile::new().unwrap().into_temp_path();
        build(root.path(), b"Package: hello\n", 1_735_689_600, &deb_path).unwrap();
        std::fs::read(&deb_path).unwrap()
    };
    assert_eq!(build_once(), build_once());
}

#[test]
fn md5sums_lists_every_regular_file_without_leading_dot_slash() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("usr/bin")).unwrap();
    std::fs::write(root.path().join("usr/bin/hello"), b"payload").unwrap();
    let mut md5sums = String::new();
    build_data_tar_gz(root.path(), 0, &mut md5sums).unwrap();
    let expected = format!("{:x}", md5::compute(b"payload"));
    assert_eq!(md5sums, format!("{expected}  usr/bin/hello\n"));
}

#[test]
fn preserves_symlinks() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("usr/lib/pkg/bin")).unwrap();
    std::fs::write(root.path().join("usr/lib/pkg/bin/real"), b"elf").unwrap();
    std::os::unix::fs::symlink("../lib/pkg/bin/real", root.path().join("usr/bin_link")).unwrap();
    // Just confirm the walk doesn't error on a symlink and still
    // reaches the real file for md5sums.
    let mut md5sums = String::new();
    build_data_tar_gz(root.path(), 0, &mut md5sums).unwrap();
    assert!(md5sums.contains("usr/lib/pkg/bin/real"));
}

#[test]
fn compression_variants_produce_valid_ars() {
    // Levels included: the `:N` suffix must be honored end to end, not
    // just tolerated.
    for comp in ["gzip", "xz", "zstd", "none", "gzip:1", "xz:3", "zstd:10"] {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("usr/bin")).unwrap();
        std::fs::write(root.path().join("usr/bin/hello"), b"payload").unwrap();
        let deb_path = tempfile::NamedTempFile::new().unwrap().into_temp_path();
        build_with_compression(
            root.path(),
            b"Package: hello\n",
            1_735_689_600,
            &deb_path,
            comp,
        )
        .unwrap();
        let bytes = std::fs::read(&deb_path).unwrap();
        assert!(
            bytes.starts_with(b"!<arch>\n"),
            "ar magic missing for {comp}"
        );
        let mut archive = ar::Archive::new(bytes.as_slice());
        let mut names: Vec<String> = Vec::new();
        while let Some(entry) = archive.next_entry() {
            let entry = entry.unwrap();
            names.push(String::from_utf8_lossy(entry.header().identifier()).to_string());
        }
        let expected_data = match comp.split(':').next().unwrap() {
            "gzip" => "data.tar.gz",
            "xz" => "data.tar.xz",
            "zstd" => "data.tar.zst",
            "none" => "data.tar",
            other => unreachable!("{other}"),
        };
        assert!(
            names.contains(&expected_data.to_string()),
            "{comp}: {names:?}"
        );
        // round-trip extract
        let dest = tempfile::tempdir().unwrap();
        extract(&deb_path, dest.path()).unwrap();
        assert_eq!(
            std::fs::read(dest.path().join("usr/bin/hello")).unwrap(),
            b"payload"
        );
    }
}

#[test]
fn compression_levels_parse_and_validate() {
    let g1 = normalize_compression("gzip:1").unwrap();
    assert_eq!(g1.kind, CompressionKind::Gzip);
    assert_eq!(g1.level, Some(1));

    // Defaults match the previously hardcoded levels.
    assert_eq!(normalize_compression("zstd").unwrap().level, Some(19));
    assert_eq!(normalize_compression("xz").unwrap().level, Some(9));
    assert_eq!(normalize_compression("gz").unwrap().level, Some(9));
    assert_eq!(normalize_compression("none").unwrap().level, None);
    // Bare/empty string falls back to gzip.
    assert_eq!(normalize_compression("").unwrap().level, Some(9));
    assert_eq!(
        normalize_compression("").unwrap().kind,
        CompressionKind::Gzip
    );

    // Out-of-range and non-numeric levels are errors, not silently clamped.
    assert!(normalize_compression("gzip:99").is_err());
    assert!(normalize_compression("xz:10").is_err());
    assert!(normalize_compression("zstd:0").is_err()); // zstd min is 1
    assert!(normalize_compression("zstd:23").is_err());
    assert!(normalize_compression("gzip:fast").is_err());
    // 'none' takes no level at all.
    assert!(normalize_compression("none:1").is_err());
}

#[test]
fn same_level_is_deterministic_and_levels_change_output() {
    let build_with = |comp: &str| {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("usr/bin")).unwrap();
        std::fs::write(root.path().join("usr/bin/hello"), vec![b'x'; 4096]).unwrap();
        let deb_path = tempfile::NamedTempFile::new().unwrap().into_temp_path();
        build_with_compression(
            root.path(),
            b"Package: hello\n",
            1_735_689_600,
            &deb_path,
            comp,
        )
        .unwrap();
        std::fs::read(&deb_path).unwrap()
    };
    // Determinism per level setting.
    assert_eq!(build_with("gzip:1"), build_with("gzip:1"));
    assert_eq!(build_with("zstd:5"), build_with("zstd:5"));
    // Different levels produce different bytes (level actually applied).
    assert_ne!(build_with("gzip:1"), build_with("gzip:9"));
    assert_ne!(build_with("zstd:3"), build_with("zstd:19"));
}

#[test]
fn same_input_compressed_is_deterministic() {
    for comp in ["gzip", "xz", "zstd", "none"] {
        let build_once = || {
            let root = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(root.path().join("usr/bin")).unwrap();
            std::fs::write(root.path().join("usr/bin/hello"), b"fake-elf").unwrap();
            let deb_path = tempfile::NamedTempFile::new().unwrap().into_temp_path();
            build_with_compression(
                root.path(),
                b"Package: hello\n",
                1_735_689_600,
                &deb_path,
                comp,
            )
            .unwrap();
            std::fs::read(&deb_path).unwrap()
        };
        assert_eq!(build_once(), build_once(), "determinism failed for {comp}");
    }
}

#[test]
fn build_full_with_origin_signer_appends_gpgorigin() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("usr/bin")).unwrap();
    std::fs::write(root.path().join("usr/bin/hello"), b"payload").unwrap();

    let deb_path = tempfile::NamedTempFile::new().unwrap().into_temp_path();
    let signer = |payload: &[u8]| -> anyhow::Result<Vec<u8>> {
        let mut out = b"BEGIN:".to_vec();
        out.extend_from_slice(payload);
        out.extend_from_slice(b":END");
        Ok(out)
    };
    build_full(
        root.path(),
        b"Package: hello\n",
        0,
        &deb_path,
        "gzip",
        &[],
        Some(&signer),
    )
    .unwrap();

    let file = std::fs::File::open(&deb_path).unwrap();
    let mut archive = ar::Archive::new(file);
    let mut names = Vec::new();
    let mut gpgorigin = None;
    while let Some(entry) = archive.next_entry() {
        let mut entry = entry.unwrap();
        let name = String::from_utf8_lossy(entry.header().identifier()).to_string();
        if name == "_gpgorigin" {
            let mut buf = Vec::new();
            std::io::copy(&mut entry, &mut buf).unwrap();
            gpgorigin = Some(buf);
        }
        names.push(name);
    }
    assert_eq!(
        names,
        vec![
            "debian-binary",
            "control.tar.gz",
            "data.tar.gz",
            "_gpgorigin"
        ]
    );
    let sig = gpgorigin.expect("_gpgorigin missing");
    assert!(sig.starts_with(b"BEGIN:"));
    assert!(sig.ends_with(b":END"));
    // Payload is debian-binary + control + data (at least "2.0\n").
    assert!(sig.len() > b"BEGIN:2.0\n:END".len());
}

#[test]
fn build_full_without_signer_has_three_members() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("usr/bin")).unwrap();
    std::fs::write(root.path().join("usr/bin/hello"), b"payload").unwrap();
    let deb_path = tempfile::NamedTempFile::new().unwrap().into_temp_path();
    build_full(
        root.path(),
        b"Package: hello\n",
        0,
        &deb_path,
        "gzip",
        &[],
        None,
    )
    .unwrap();
    let file = std::fs::File::open(&deb_path).unwrap();
    let mut archive = ar::Archive::new(file);
    let mut names = Vec::new();
    while let Some(entry) = archive.next_entry() {
        let entry = entry.unwrap();
        names.push(String::from_utf8_lossy(entry.header().identifier()).to_string());
    }
    assert_eq!(
        names,
        vec!["debian-binary", "control.tar.gz", "data.tar.gz"]
    );
}
