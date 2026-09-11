use lx_lib::scandeps::*;

#[test]
fn find_elf_files_walks_nested_dirs_and_skips_non_elf() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("bin")).unwrap();
    std::fs::write(dir.path().join("bin/tool"), b"\x7fELFrest-of-file").unwrap();
    std::fs::write(dir.path().join("README.md"), b"not an elf").unwrap();

    let found = find_elf_files(dir.path()).unwrap();
    assert_eq!(found.len(), 1);
    assert!(found[0].ends_with("bin/tool"));
}

/// Regression test for a real bug found scanning a live `raw`-format
/// asset (an AppImage): the download path used to disambiguate
/// architectures via a filename prefix (`"{arch}-{asset_name}"`), but
/// `build::extract`'s "raw" branch names the extracted file after the
/// *downloaded file's own on-disk name* -- so the prefix leaked into
/// what got displayed as the scanned binary's name. Downloads now go
/// into a per-architecture subdirectory instead, keeping the asset's
/// real name intact end to end.
#[test]
fn raw_format_extraction_preserves_the_real_asset_name() {
    let tmp = tempfile::tempdir().unwrap();
    let asset_dir = tmp.path().join("amd64-download");
    std::fs::create_dir_all(&asset_dir).unwrap();
    let asset_path = asset_dir.join("nvim-linux-x86_64.appimage");
    std::fs::write(&asset_path, b"\x7fELFfake-appimage-bytes").unwrap();

    let extract_dir = tmp.path().join("amd64-scan-extract");
    lx_lib::build::extract(&asset_path, &extract_dir, "raw").unwrap();

    let found = find_elf_files(&extract_dir).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0].file_name().unwrap().to_str().unwrap(),
        "nvim-linux-x86_64.appimage",
        "extracted raw asset must keep its real name, not an internal disambiguation prefix"
    );
}

#[test]
fn declared_package_names_strips_versions_and_alternatives() {
    let names = declared_package_names("libc6 (>= 2.34), libssl3 | libssl1.1, libz1");
    assert!(names.contains("libc6"));
    assert!(names.contains("libssl3"));
    assert!(names.contains("libssl1.1"));
    assert!(names.contains("libz1"));
    assert_eq!(names.len(), 4);
}

#[test]
fn declared_package_names_of_empty_string_is_empty() {
    assert!(declared_package_names("").is_empty());
    assert!(declared_package_names("   ").is_empty());
}

#[test]
fn dpkg_owner_is_none_when_dpkg_unavailable_or_no_match() {
    // This dev environment may or may not have `dpkg`; either way, a
    // nonsense soname must never resolve to a package.
    assert!(dpkg_owner("libtotally-made-up-soname.so.999").is_none());
}

#[cfg(unix)]
#[test]
fn find_elf_files_follows_a_symlink_to_an_elf_file_without_crashing() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("real-binary"), b"\x7fELFrest").unwrap();
    std::os::unix::fs::symlink(
        dir.path().join("real-binary"),
        dir.path().join("linked-binary"),
    )
    .unwrap();
    // A symlink to a directory must not be treated as a directory to
    // recurse into (DirEntry::file_type doesn't follow symlinks) nor
    // crash `is_elf`'s File::open (which does follow symlinks and
    // would try to read a directory as a file).
    std::fs::create_dir(dir.path().join("real-dir")).unwrap();
    std::os::unix::fs::symlink(dir.path().join("real-dir"), dir.path().join("linked-dir")).unwrap();

    let found = find_elf_files(dir.path()).unwrap();
    let names: Vec<String> = found
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    assert!(names.contains(&"real-binary".to_string()));
    assert!(names.contains(&"linked-binary".to_string()));
    assert_eq!(found.len(), 2, "{names:?}");
}
