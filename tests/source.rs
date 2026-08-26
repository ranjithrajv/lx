use lpt_lib::source::*;
use sha2::{Digest, Sha256};

fn test_pkg(epoch: &str) -> Pkg {
    Pkg {
        name: "eza".into(),
        github_repo: "eza-community/eza".into(),
        description: "eza, packaged from eza-community/eza".into(),
        maintainer: "latest-debs maintainers <maintainers@latest-debs.org>".into(),
        version: "0.23.5".into(),
        build_version: "1".into(),
        epoch: epoch.into(),
        license_spdx: "MIT".into(),
        depends: String::new(),
        recommends: String::new(),
        suggests: String::new(),
        conflicts: String::new(),
        replaces: String::new(),
        provides: String::new(),
        breaks: String::new(),
        predepends: String::new(),
        section: String::new(),
        priority: String::new(),
        fields: std::collections::HashMap::new(),
        published_at: None,
        license: None,
    }
}

#[test]
fn dsc_lists_orig_before_debian_and_matches_dpkg_source_field_order() {
    let pkg = test_pkg("");
    let orig = DscFile::from_bytes("eza_0.23.5.orig.tar.xz".into(), b"orig-bytes");
    let debian = DscFile::from_bytes(
        "eza_0.23.5-1+bookworm.debian.tar.xz".into(),
        b"debian-bytes",
    );
    let dsc = render_dsc(&pkg, "0.23.5-1+bookworm", &orig, &debian);

    let lines: Vec<&str> = dsc.lines().collect();
    assert_eq!(lines[0], "Format: 3.0 (quilt)");
    assert_eq!(lines[1], "Source: eza");
    assert_eq!(lines[2], "Binary: eza");
    assert_eq!(lines[3], "Architecture: any");
    assert_eq!(lines[4], "Version: 0.23.5-1+bookworm");
    assert!(dsc.contains("Package-List:\n eza deb utils optional arch=any\n"));

    let checksums_idx = dsc.find("Checksums-Sha1:").unwrap();
    let orig_idx = dsc[checksums_idx..].find("orig.tar.xz").unwrap();
    let debian_idx = dsc[checksums_idx..].find("debian.tar.xz").unwrap();
    assert!(orig_idx < debian_idx, "orig must be listed before debian");
}

#[test]
fn content_version_gets_epoch_but_filenames_dont() {
    // content_version (fed to the changelog and .dsc Version: field) is
    // computed from with_epoch(); src_version (fed to every filename)
    // never is -- Debian policy excludes epoch from filenames since
    // `:` isn't filename-safe. Exercise the exact computation
    // build_source_package uses.
    let pkg = test_pkg("1");
    let src_version = "0.23.5-1+bookworm";
    let content_version = lpt_lib::pkgmeta::with_epoch(&pkg.epoch, src_version);
    assert_eq!(content_version, "1:0.23.5-1+bookworm");

    let dsc_name = format!("{}_{src_version}.dsc", pkg.name);
    assert_eq!(dsc_name, "eza_0.23.5-1+bookworm.dsc");
    assert!(!dsc_name.contains(':'));

    let dsc = render_dsc(
        &pkg,
        &content_version,
        &DscFile::from_bytes("o".into(), b"x"),
        &DscFile::from_bytes("d".into(), b"y"),
    );
    assert!(dsc.contains("Version: 1:0.23.5-1+bookworm\n"));
}

/// Build a minimal staged tree (a fake ELF under usr/bin) shared by the
/// rpm/arch source-package tests below.
fn fake_staged_tree() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("usr/bin")).unwrap();
    std::fs::write(root.path().join("usr/bin/eza"), b"fake-elf-bytes").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            root.path().join("usr/bin/eza"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
    root
}

#[test]
fn generate_rpm_writes_srpm_per_dist_and_shares_source_tarball() {
    let staged = fake_staged_tree();
    let out_dir = tempfile::tempdir().unwrap();

    for (release, filename) in [
        ("1.fedora", "eza-0.23.5-1.fedora.x86_64.rpm"),
        ("1.el9", "eza-0.23.5-1.el9.x86_64.rpm"),
    ] {
        lpt_lib::rpmarchive::build(
            staged.path(),
            &lpt_lib::rpmarchive::PackageMeta {
                name: "eza",
                version: "0.23.5",
                release,
                summary: "eza, packaged from eza-community/eza",
                description: "desc",
                license: "MIT",
            },
            "amd64",
            1_735_689_600,
            &out_dir.path().join(filename),
        )
        .unwrap();
    }

    let pkg = test_pkg("");
    generate_rpm(out_dir.path(), &pkg).unwrap();

    for name in ["eza-0.23.5-1.fedora.src.rpm", "eza-0.23.5-1.el9.src.rpm"] {
        let path = out_dir.path().join(name);
        assert!(path.exists(), "missing {name}");
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(
            &bytes[0..4],
            &[0xED, 0xAB, 0xEE, 0xDB],
            "{name}: rpm magic missing"
        );

        let srpm = rpm::Package::open(&path).unwrap();
        let names: Vec<String> = srpm
            .metadata
            .get_file_entries()
            .unwrap()
            .into_iter()
            .map(|e| e.path.to_string_lossy().to_string())
            .collect();
        assert!(names.iter().any(|n| n.ends_with("eza.spec")), "{names:?}");
        assert!(
            names.iter().any(|n| n.ends_with("eza-0.23.5.tar.xz")),
            "{names:?}"
        );
    }
}

#[test]
fn generate_arch_writes_pkgbuild_with_matching_sha256() {
    let staged = fake_staged_tree();
    let out_dir = tempfile::tempdir().unwrap();
    lpt_lib::archarchive::build(
        staged.path(),
        &lpt_lib::archarchive::PackageMeta {
            name: "eza",
            version: "0.23.5",
            release: "1.arch",
            description: "eza, packaged from eza-community/eza",
            url: "https://github.com/eza-community/eza",
            license: "MIT",
        },
        "amd64",
        1_735_689_600,
        &out_dir.path().join("eza-0.23.5-1.arch-x86_64.pkg.tar.zst"),
    )
    .unwrap();

    let pkg = test_pkg("");
    generate_arch(out_dir.path(), &pkg).unwrap();

    let pkgbuild_path = out_dir.path().join("PKGBUILD");
    assert!(pkgbuild_path.exists());
    let pkgbuild = std::fs::read_to_string(&pkgbuild_path).unwrap();
    assert!(pkgbuild.contains("pkgname=eza"));
    assert!(pkgbuild.contains("pkgver=0.23.5"));
    assert!(pkgbuild.contains("pkgrel=1.arch"));
    assert!(pkgbuild.contains("source=(\"eza-0.23.5.tar.xz\")"));

    let source_path = out_dir.path().join("eza-0.23.5.tar.xz");
    assert!(source_path.exists());
    let source_bytes = std::fs::read(&source_path).unwrap();
    let mut h = Sha256::new();
    h.update(&source_bytes);
    let expected_sha = hex::encode(h.finalize());
    assert!(
        pkgbuild.contains(&format!("sha256sums=('{expected_sha}')")),
        "{pkgbuild}"
    );
}
