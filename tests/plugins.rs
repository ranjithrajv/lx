// SPDX-License-Identifier: GPL-3.0-or-later

use lx_lib::plugins::*;

use lx_lib::config::PackageConfig;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

fn fake_elf_bytes() -> Vec<u8> {
    // Minimal ELF magic + padding.
    let mut b = vec![0x7f, b'E', b'L', b'F', 2, 1, 1, 0];
    b.extend(vec![0u8; 100]);
    b
}

#[test]
fn stage_install_tree_flat_mode_copies_elf_files_only() {
    let binary_dir = tempfile::tempdir().unwrap();
    std::fs::write(binary_dir.path().join("eza"), fake_elf_bytes()).unwrap();
    std::fs::write(binary_dir.path().join("README.md"), b"not an elf").unwrap();
    let root = tempfile::tempdir().unwrap();
    let cfg = PackageConfig {
        package_name: "eza".into(),
        ..PackageConfig::default()
    };

    stage_install_tree(&cfg, binary_dir.path(), root.path(), 0).unwrap();

    assert!(root.path().join("usr/bin/eza").is_file());
    assert!(!root.path().join("usr/bin/README.md").exists());
    assert!(!root.path().join("usr/lib/eza").exists());
    let mode = std::fs::metadata(root.path().join("usr/bin/eza"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o111, 0o111, "not executable: {mode:o}");
}

#[test]
fn stage_install_tree_flat_mode_installs_man_pages_and_license() {
    let binary_dir = tempfile::tempdir().unwrap();
    std::fs::write(binary_dir.path().join("eza"), fake_elf_bytes()).unwrap();
    std::fs::write(binary_dir.path().join("eza.1"), b"man page text").unwrap();
    std::fs::write(binary_dir.path().join("LICENSE"), b"MIT license text").unwrap();
    std::fs::write(binary_dir.path().join("checksums.txt"), b"unrecognized").unwrap();
    let root = tempfile::tempdir().unwrap();
    let cfg = PackageConfig {
        package_name: "eza".into(),
        ..PackageConfig::default()
    };

    stage_install_tree(&cfg, binary_dir.path(), root.path(), 1_735_689_600).unwrap();

    // Man page: installed gzip-compressed under the right section dir.
    let man_gz = root.path().join("usr/share/man/man1/eza.1.gz");
    assert!(man_gz.is_file());
    let decompressed = {
        let f = std::fs::File::open(&man_gz).unwrap();
        let mut gz = flate2::read::GzDecoder::new(f);
        let mut s = String::new();
        std::io::Read::read_to_string(&mut gz, &mut s).unwrap();
        s
    };
    assert_eq!(decompressed, "man page text");

    // License: copied as-is into usr/share/doc/<pkg>/.
    assert_eq!(
        std::fs::read_to_string(root.path().join("usr/share/doc/eza/LICENSE")).unwrap(),
        "MIT license text"
    );

    // Unrecognized top-level file: left alone, not an error.
    assert!(!root.path().join("usr/share/doc/eza/checksums.txt").exists());
    assert!(!root.path().join("usr/bin/checksums.txt").exists());
}

#[test]
fn man_section_recognizes_plain_and_gzipped_pages() {
    assert_eq!(man_section("eza.1"), Some(1));
    assert_eq!(man_section("eza.1.gz"), Some(1));
    assert_eq!(man_section("eza.9"), Some(9));
    assert_eq!(man_section("eza.0"), None); // no man section 0
    assert_eq!(man_section("eza.tar.gz"), None);
    assert_eq!(man_section("README.md"), None);
}

#[test]
fn is_license_like_matches_common_names_case_insensitively() {
    assert!(is_license_like("LICENSE"));
    assert!(is_license_like("LICENSE.md"));
    assert!(is_license_like("license.txt"));
    assert!(is_license_like("COPYING"));
    assert!(is_license_like("NOTICE"));
    assert!(!is_license_like("README.md"));
    assert!(!is_license_like("eza"));
}

#[test]
fn stage_install_tree_bundle_mode_symlinks_bin_into_usr_bin() {
    let binary_dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(binary_dir.path().join("bin")).unwrap();
    std::fs::write(binary_dir.path().join("bin/zed"), fake_elf_bytes()).unwrap();
    std::fs::create_dir_all(binary_dir.path().join("lib")).unwrap();
    std::fs::write(binary_dir.path().join("lib/libfoo.so"), fake_elf_bytes()).unwrap();
    let root = tempfile::tempdir().unwrap();
    let cfg = PackageConfig {
        package_name: "zed".into(),
        bundle: true,
        ..PackageConfig::default()
    };

    stage_install_tree(&cfg, binary_dir.path(), root.path(), 0).unwrap();

    assert!(root.path().join("usr/lib/zed/bin/zed").is_file());
    assert!(root.path().join("usr/lib/zed/lib/libfoo.so").is_file());
    let link = root.path().join("usr/bin/zed");
    assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
    assert_eq!(
        std::fs::read_link(&link).unwrap(),
        Path::new("/usr/lib/zed/bin/zed")
    );
}

#[test]
fn stage_install_tree_bundle_mode_also_symlinks_root_level_executables() {
    // pnpm-style bundles ship executables as siblings of their data
    // dirs at the bundle root, with no bin/ subdirectory at all.
    let binary_dir = tempfile::tempdir().unwrap();
    std::fs::write(binary_dir.path().join("pnpm"), fake_elf_bytes()).unwrap();
    std::fs::create_dir_all(binary_dir.path().join("dist")).unwrap();
    let root = tempfile::tempdir().unwrap();
    let cfg = PackageConfig {
        package_name: "pnpm".into(),
        bundle: true,
        ..PackageConfig::default()
    };

    stage_install_tree(&cfg, binary_dir.path(), root.path(), 0).unwrap();

    let link = root.path().join("usr/bin/pnpm");
    assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
    assert_eq!(
        std::fs::read_link(&link).unwrap(),
        Path::new("/usr/lib/pnpm/pnpm")
    );
}

#[test]
fn stage_install_tree_fails_when_usr_bin_ends_up_empty() {
    for bundle in [false, true] {
        let binary_dir = tempfile::tempdir().unwrap();
        std::fs::write(binary_dir.path().join("README.md"), b"not an elf").unwrap();
        let root = tempfile::tempdir().unwrap();
        let cfg = PackageConfig {
            package_name: "x".into(),
            bundle,
            ..PackageConfig::default()
        };
        let err = stage_install_tree(&cfg, binary_dir.path(), root.path(), 0).unwrap_err();
        assert!(
            err.to_string()
                .contains("no executables landed in /usr/bin"),
            "bundle={bundle}: {err}"
        );
    }
}

#[test]
fn copy_dir_recursive_preserves_tree() {
    let src = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(src.path().join("bin")).unwrap();
    std::fs::create_dir_all(src.path().join("lib")).unwrap();
    std::fs::write(src.path().join("bin/zed"), b"elf-ish").unwrap();
    std::fs::write(src.path().join("lib/libfoo.so"), b"lib").unwrap();

    let dst = tempfile::tempdir().unwrap();
    let dst_path = dst.path().join("out");
    copy_dir_recursive(src.path(), &dst_path).unwrap();

    assert_eq!(std::fs::read(dst_path.join("bin/zed")).unwrap(), b"elf-ish");
    assert_eq!(
        std::fs::read(dst_path.join("lib/libfoo.so")).unwrap(),
        b"lib"
    );
}

#[test]
fn copy_dir_recursive_preserves_symlinks() {
    let src = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(src.path().join("lib")).unwrap();
    std::fs::write(src.path().join("lib/libfoo.so.1"), b"lib").unwrap();
    std::os::unix::fs::symlink("libfoo.so.1", src.path().join("lib/libfoo.so")).unwrap();

    let dst = tempfile::tempdir().unwrap();
    let dst_path = dst.path().join("out");
    copy_dir_recursive(src.path(), &dst_path).unwrap();

    let link = dst_path.join("lib/libfoo.so");
    assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
    assert_eq!(std::fs::read_link(&link).unwrap(), Path::new("libfoo.so.1"));
    assert_eq!(std::fs::read(&link).unwrap(), b"lib");
}

#[test]
fn registry_contains_deb_and_rpm() {
    let names = packager_names();
    assert!(names.contains(&"deb"));
    assert!(names.contains(&"rpm"));
    assert!(names.contains(&"arch"));
    assert!(get_packager("deb").is_some());
    assert!(get_packager("rpm").is_some());
    assert!(get_packager("arch").is_some());
    assert!(get_packager("unknown").is_none());
    assert!(get_packager("DEB").is_some()); // case-insensitive
}

#[test]
fn plugin_file_extensions() {
    assert_eq!(get_packager("deb").unwrap().file_extension(), "deb");
    assert_eq!(get_packager("rpm").unwrap().file_extension(), "rpm");
    assert_eq!(
        get_packager("arch").unwrap().file_extension(),
        "pkg.tar.zst"
    );
}

#[test]
fn deb_and_rpm_plugins_build_valid_archives() {
    for format in ["deb", "rpm", "arch"] {
        let plugin = get_packager(format).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let binary_dir = tmp.path().join("binary");
        std::fs::create_dir_all(&binary_dir).unwrap();
        let bin_path = binary_dir.join("hello");
        std::fs::write(&bin_path, fake_elf_bytes()).unwrap();
        let mut perms = std::fs::metadata(&bin_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&bin_path, perms).unwrap();

        let staging_root = tmp.path().join("root");
        std::fs::create_dir_all(&staging_root).unwrap();

        let cfg = PackageConfig {
            package_name: "hello".into(),
            github_repo: "owner/hello".into(),
            description: "test".into(),
            maintainer: "t <t@example.com>".into(),
            license_spdx: "MIT".into(),
            package_format: format.into(),
            ..Default::default()
        };
        let asset = lx_lib::github::Asset {
            name: "hello.tar.gz".into(),
            size: None,
            browser_download_url: "".into(),
        };
        let job = lx_lib::build::ResolvedJob {
            dist: match format {
                "deb" => "trixie".into(),
                "arch" => "arch".into(),
                _ => "fedora".into(),
            },
            arch: "amd64".into(),
            asset,
            tag: "v1.0.0".into(),
            published_at: Some(1_735_689_600),
        };
        let ctx = BuildContext {
            cfg: &cfg,
            job: &job,
            binary_dir: &binary_dir,
            staging_root: &staging_root,
            license: None,
            debian_version: "1.0.0",
            build_version: "1",
            mtime: 1_735_689_600,
            sign_key: None,
            sign_key_id: "",
            sign_passphrase: None,
            sign_method: "detach",
        };
        let out = plugin.build(&ctx).unwrap();
        assert!(out.exists(), "plugin {format} did not produce output");
        let bytes = std::fs::read(&out).unwrap();
        if format == "deb" {
            assert!(bytes.starts_with(b"!<arch>\n"), "deb magic missing");
            assert_eq!(out.extension().unwrap(), "deb");
        } else if format == "rpm" {
            assert_eq!(&bytes[0..4], &[0xED, 0xAB, 0xEE, 0xDB], "rpm magic missing");
            assert_eq!(out.extension().unwrap(), "rpm");
        } else {
            // arch: .pkg.tar.zst is zstd-compressed tar, magic 0x28B52FFD
            assert_eq!(
                &bytes[0..4],
                &[0x28, 0xB5, 0x2F, 0xFD],
                "arch zstd magic missing"
            );
            assert!(out.to_string_lossy().ends_with(".pkg.tar.zst"));
        }
    }
}

#[test]
fn default_distributions_differ_by_format() {
    let deb = get_packager("deb").unwrap();
    let rpm = get_packager("rpm").unwrap();
    let arch = get_packager("arch").unwrap();
    assert!(deb.default_distributions().contains(&"bookworm"));
    assert!(rpm.default_distributions().contains(&"fedora"));
    assert!(arch.default_distributions().contains(&"arch"));
    assert_ne!(deb.default_distributions(), rpm.default_distributions());
    assert_ne!(deb.default_distributions(), arch.default_distributions());
}

#[test]
fn deb_with_new_control_fields_and_compression_is_valid() {
    let plugin = get_packager("deb").unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let binary_dir = tmp.path().join("binary");
    std::fs::create_dir_all(&binary_dir).unwrap();
    let bin_path = binary_dir.join("hello");
    std::fs::write(&bin_path, fake_elf_bytes()).unwrap();
    let mut perms = std::fs::metadata(&bin_path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&bin_path, perms).unwrap();

    let staging_root = tmp.path().join("root");
    std::fs::create_dir_all(&staging_root).unwrap();

    let mut fields = std::collections::HashMap::new();
    fields.insert(
        "Bugs".to_string(),
        "https://github.com/owner/hello/issues".to_string(),
    );
    fields.insert("X-Custom".to_string(), "value".to_string());
    let cfg = PackageConfig {
        package_name: "hello".into(),
        github_repo: "owner/hello".into(),
        description: "test desc".into(),
        maintainer: "t <t@example.com>".into(),
        license_spdx: "MIT".into(),
        package_format: "deb".into(),
        suggests: "bar".into(),
        predepends: "libc6 (>= 2.35)".into(),
        section: "devel".into(),
        priority: "extra".into(),
        fields,
        compression: "xz".into(),
        ..Default::default()
    };
    let asset = lx_lib::github::Asset {
        name: "hello.tar.gz".into(),
        size: None,
        browser_download_url: "".into(),
    };
    let job = lx_lib::build::ResolvedJob {
        dist: "trixie".into(),
        arch: "amd64".into(),
        asset,
        tag: "v1.0.0".into(),
        published_at: Some(1_735_689_600),
    };
    let ctx = BuildContext {
        cfg: &cfg,
        job: &job,
        binary_dir: &binary_dir,
        staging_root: &staging_root,
        license: None,
        debian_version: "1.0.0",
        build_version: "1",
        mtime: 1_735_689_600,
        sign_key: None,
        sign_key_id: "",
        sign_passphrase: None,
        sign_method: "detach",
    };
    let out = plugin.build(&ctx).unwrap();
    assert!(out.exists());
    // Check ar member names reflect xz compression
    let bytes = std::fs::read(&out).unwrap();
    let mut ar = ar::Archive::new(bytes.as_slice());
    let mut names = Vec::new();
    while let Some(e) = ar.next_entry() {
        names.push(String::from_utf8_lossy(e.unwrap().header().identifier()).to_string());
    }
    assert!(names.contains(&"data.tar.xz".to_string()), "{names:?}");
    assert!(names.contains(&"control.tar.xz".to_string()), "{names:?}");

    // If dpkg-deb is available, spot-check via dpkg-deb --info
    if let Ok(v) = std::process::Command::new("dpkg-deb")
        .arg("--version")
        .output()
    {
        if v.status.success() {
            let info = std::process::Command::new("dpkg-deb")
                .args(["--info", out.to_str().unwrap()])
                .output()
                .unwrap();
            let txt = String::from_utf8_lossy(&info.stdout);
            assert!(
                txt.contains("Section: devel"),
                "dpkg-deb info missing Section: {txt}"
            );
        }
    }
    // Also verify via direct control extraction        // Also verify via direct control extraction for determinism without dpkg
    // Extract control.tar.xz from the deb and read control file
    let deb_bytes = std::fs::read(&out).unwrap();
    let mut ar = ar::Archive::new(deb_bytes.as_slice());
    let mut control_data = Vec::new();
    while let Some(entry) = ar.next_entry() {
        let mut e = entry.unwrap();
        let name = String::from_utf8_lossy(e.header().identifier()).to_string();
        if name.starts_with("control.tar") {
            let mut data = Vec::new();
            std::io::copy(&mut e, &mut data).unwrap();
            control_data = data;
            break;
        }
    }
    assert!(!control_data.is_empty(), "no control.tar found");
    // Decompress xz
    let decoder = lzma_rust2::XzReader::new(control_data.as_slice(), true);
    let mut tar = tar::Archive::new(decoder);
    let mut control_text = String::new();
    for entry in tar.entries().unwrap() {
        let mut e = entry.unwrap();
        let path = e.path().unwrap().to_string_lossy().to_string();
        if path.ends_with("control") {
            std::io::Read::read_to_string(&mut e, &mut control_text).unwrap();
        }
    }
    assert!(
        control_text.contains("Section: devel"),
        "control={control_text}"
    );
    assert!(
        control_text.contains("Priority: extra"),
        "control={control_text}"
    );
    assert!(
        control_text.contains("Suggests: bar"),
        "control={control_text}"
    );
    assert!(
        control_text.contains("Pre-Depends: libc6 (>= 2.35)"),
        "control={control_text}"
    );
    assert!(
        control_text.contains("Bugs: https://github.com/owner/hello/issues"),
        "control={control_text}"
    );
    assert!(
        control_text.contains("X-Custom: value"),
        "control={control_text}"
    );
}

/// contents overlay + conffiles + maintainer scripts, end to end
/// through the deb plugin.
#[test]
fn deb_contents_scripts_conffiles_end_to_end() {
    let plugin = get_packager("deb").unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let binary_dir = tmp.path().join("binary");
    std::fs::create_dir_all(&binary_dir).unwrap();
    let bin_path = binary_dir.join("hello");
    std::fs::write(&bin_path, fake_elf_bytes()).unwrap();
    let mut perms = std::fs::metadata(&bin_path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&bin_path, perms).unwrap();

    // Build-environment files for the contents overlay.
    let env = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(env.path().join("completions")).unwrap();
    std::fs::write(
        env.path().join("completions/hello.bash"),
        b"# bash completion",
    )
    .unwrap();
    std::fs::write(env.path().join("hello.conf"), b"key=value\n").unwrap();
    std::fs::write(
        env.path().join("postinst.sh"),
        "#!/bin/sh\nset -e\necho hi\n",
    )
    .unwrap();

    let staging_root = tmp.path().join("root");
    std::fs::create_dir_all(&staging_root).unwrap();

    let cfg = PackageConfig {
        package_name: "hello".into(),
        github_repo: "owner/hello".into(),
        description: "test desc".into(),
        maintainer: "t <t@example.com>".into(),
        license_spdx: "MIT".into(),
        package_format: "deb".into(),
        contents: vec![
            lx_lib::config::ContentEntry {
                src: env
                    .path()
                    .join("completions/hello.bash")
                    .to_string_lossy()
                    .to_string(),
                dst: "/usr/share/bash-completion/completions/hello".into(),
                kind: String::new(),
                packager: String::new(),
            },
            lx_lib::config::ContentEntry {
                src: env.path().join("hello.conf").to_string_lossy().to_string(),
                dst: "/etc/hello/hello.conf".into(),
                kind: "config|noreplace".into(),
                packager: String::new(),
            },
            lx_lib::config::ContentEntry {
                src: String::new(),
                dst: "/var/lib/hello".into(),
                kind: "dir".into(),
                packager: String::new(),
            },
        ],
        scripts: lx_lib::config::Scripts {
            postinstall: env.path().join("postinst.sh").to_string_lossy().to_string(),
            ..Default::default()
        },
        ..Default::default()
    };
    let asset = lx_lib::github::Asset {
        name: "hello.tar.gz".into(),
        size: None,
        browser_download_url: "".into(),
    };
    let job = lx_lib::build::ResolvedJob {
        dist: "trixie".into(),
        arch: "amd64".into(),
        asset,
        tag: "v1.0.0".into(),
        published_at: Some(1_735_689_600),
    };
    let ctx = BuildContext {
        cfg: &cfg,
        job: &job,
        binary_dir: &binary_dir,
        staging_root: &staging_root,
        license: None,
        debian_version: "1.0.0",
        build_version: "1",
        mtime: 1_735_689_600,
        sign_key: None,
        sign_key_id: "",
        sign_passphrase: None,
        sign_method: "detach",
    };
    let out = plugin.build(&ctx).unwrap();
    assert!(out.exists());

    // Pull the control member set back out and inspect it.
    let deb_bytes = std::fs::read(&out).unwrap();
    let mut ar = ar::Archive::new(deb_bytes.as_slice());
    let mut control_tar = None;
    while let Some(entry) = ar.next_entry() {
        let mut e = entry.unwrap();
        let name = String::from_utf8_lossy(e.header().identifier()).to_string();
        if name.starts_with("control.tar") {
            let mut data = Vec::new();
            std::io::copy(&mut e, &mut data).unwrap();
            control_tar = Some(data);
            break;
        }
    }
    let control_tar = control_tar.expect("no control.tar");
    let decoder = flate2::read::GzDecoder::new(control_tar.as_slice());
    let mut tar = tar::Archive::new(decoder);
    let mut members: std::collections::HashMap<String, (Vec<u8>, u32)> =
        std::collections::HashMap::new();
    for entry in tar.entries().unwrap() {
        let mut e = entry.unwrap();
        let path = e.path().unwrap().to_string_lossy().to_string();
        let mode = e.header().mode().unwrap();
        let mut content = Vec::new();
        std::io::Read::read_to_end(&mut e, &mut content).unwrap();
        members.insert(path.trim_start_matches("./").to_string(), (content, mode));
    }

    // postinst staged with exec bits.
    let (postinst, mode) = members.get("postinst").expect("no postinst member");
    assert_eq!(mode & 0o111, 0o111, "postinst must be executable");
    assert!(String::from_utf8_lossy(postinst).contains("echo hi"));
    // conffiles lists the config-typed entry.
    let (conffiles, _) = members.get("conffiles").expect("no conffiles member");
    assert_eq!(
        String::from_utf8_lossy(conffiles),
        "/etc/hello/hello.conf\n"
    );

    // data payload got the overlay files — verify via full extract.
    let dest = tempfile::tempdir().unwrap();
    lx_lib::debarchive::extract(&out, dest.path()).unwrap();
    assert!(
        dest.path()
            .join("usr/share/bash-completion/completions/hello")
            .exists(),
        "completion file not staged"
    );
    assert!(dest.path().join("etc/hello/hello.conf").exists());
    assert!(dest.path().join("var/lib/hello").is_dir());
}

#[test]
fn safe_join_rejects_traversal() {
    let root = Path::new("/tmp/stage-root");
    assert!(safe_join(root, "/usr/bin/foo").is_ok());
    assert!(safe_join(root, "relative/path").is_err());
    assert!(safe_join(root, "/../escape").is_err());
    assert!(safe_join(root, "/a/../../b").is_err());
}

#[test]
fn apply_contents_respects_packager_filter() {
    use lx_lib::plugins::apply_contents;
    let env = tempfile::tempdir().unwrap();
    std::fs::write(env.path().join("deb-only"), b"deb").unwrap();
    std::fs::write(env.path().join("rpm-only"), b"rpm").unwrap();
    std::fs::write(env.path().join("all"), b"all").unwrap();

    let cfg = PackageConfig {
        package_name: "x".into(),
        github_repo: "o/x".into(),
        contents: vec![
            lx_lib::config::ContentEntry {
                src: env.path().join("deb-only").to_string_lossy().into(),
                dst: "/usr/share/deb-only".into(),
                kind: String::new(),
                packager: "deb".into(),
            },
            lx_lib::config::ContentEntry {
                src: env.path().join("rpm-only").to_string_lossy().into(),
                dst: "/usr/share/rpm-only".into(),
                kind: String::new(),
                packager: "rpm".into(),
            },
            lx_lib::config::ContentEntry {
                src: env.path().join("all").to_string_lossy().into(),
                dst: "/usr/share/all".into(),
                kind: String::new(),
                packager: String::new(),
            },
        ],
        ..Default::default()
    };
    let root = tempfile::tempdir().unwrap();
    apply_contents(&cfg, root.path(), "deb").unwrap();
    assert!(root.path().join("usr/share/deb-only").is_file());
    assert!(root.path().join("usr/share/all").is_file());
    assert!(!root.path().join("usr/share/rpm-only").exists());
}
