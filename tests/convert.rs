// SPDX-License-Identifier: GPL-3.0-or-later

use lx_lib::convert::{run, ConvertArgs};

fn tiny_deb(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    let root = dir.join("root");
    std::fs::create_dir_all(root.join("usr/bin")).unwrap();
    std::fs::write(root.join("usr/bin/hello"), b"payload").unwrap();
    let control = b"Package: hello\nVersion: 1.0-1+bookworm\nArchitecture: amd64\nMaintainer: T <t@e.c>\nDepends: libc6\nDescription: hi\n";
    let deb = dir.join(name);
    lx_lib::debarchive::build(&root, control, 0, &deb).unwrap();
    deb
}

#[test]
fn convert_deb_to_deb_is_noop_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let deb = tiny_deb(dir.path(), "hello_1.0-1+bookworm_amd64.deb");

    let result = run(ConvertArgs {
        input: deb,
        to: Some("deb".to_string()),
        output: dir.path().to_path_buf(),
        package_name: None,
        version: None,
        arch: None,
        distribution: None,
        build_version: "1".to_string(),
        dry_run: false,
    });
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("nothing to convert"));
}

#[test]
fn convert_deb_dry_run_reports_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let deb = tiny_deb(dir.path(), "hello_1.0-1+bookworm_amd64.deb");

    // Dry run should succeed and not create output.
    run(ConvertArgs {
        input: deb,
        to: Some("deb".to_string()),
        output: dir.path().to_path_buf(),
        package_name: None,
        version: None,
        arch: None,
        distribution: None,
        build_version: "1".to_string(),
        dry_run: true,
    })
    .unwrap_err(); // deb→deb is rejected even in dry-run
}

#[test]
fn convert_rejects_nonexistent_input() {
    let dir = tempfile::tempdir().unwrap();
    let result = run(ConvertArgs {
        input: dir.path().join("nonexistent.deb"),
        to: Some("rpm".to_string()),
        output: dir.path().to_path_buf(),
        package_name: None,
        version: None,
        arch: None,
        distribution: None,
        build_version: "1".to_string(),
        dry_run: false,
    });
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("does not exist"));
}

#[test]
fn convert_rejects_unknown_target_format() {
    let dir = tempfile::tempdir().unwrap();
    let deb = tiny_deb(dir.path(), "hello_1.0-1+bookworm_amd64.deb");

    let result = run(ConvertArgs {
        input: deb,
        to: Some("apk".to_string()),
        output: dir.path().to_path_buf(),
        package_name: None,
        version: None,
        arch: None,
        distribution: None,
        build_version: "1".to_string(),
        dry_run: false,
    });
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("unsupported target format"));
}

/// Build a real `.rpm` in-process (no `rpm`/`rpm2cpio`/`cpio` on PATH) so the
/// conversion test exercises the native RPM reader.
fn tiny_rpm(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    use lx_lib::rpmarchive::{self, BuildOptions, PackageMeta, RpmRelations};
    let root = dir.join("rpmroot");
    std::fs::create_dir_all(root.join("usr/bin")).unwrap();
    std::fs::write(root.join("usr/bin/hello"), b"payload").unwrap();
    let rpm = dir.join(name);
    let opts = BuildOptions {
        relations: RpmRelations {
            requires: vec![rpm::Dependency::greater_eq("glibc", "2.17")],
            recommends: vec![rpm::Dependency::any("bash")],
            conflicts: vec![rpm::Dependency::any("old-pkg")],
            obsoletes: vec![rpm::Dependency::any("legacy")],
            provides: vec![rpm::Dependency::any("webserver")],
            ..Default::default()
        },
        epoch: Some(2),
        ..Default::default()
    };
    rpmarchive::build_with_options(
        &root,
        &PackageMeta {
            name: "hello",
            version: "1.0",
            release: "1",
            summary: "hi",
            description: "hi",
            license: "MIT",
            vendor: None,
            packager: None,
        },
        "amd64",
        1_735_689_600,
        &rpm,
        &opts,
        &lx_lib::filemeta::FileMetaMap::new(),
    )
    .unwrap();
    rpm
}

#[test]
fn convert_rpm_to_deb_in_process_carries_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let rpm = tiny_rpm(dir.path(), "hello-1.0-1.x86_64.rpm");

    run(ConvertArgs {
        input: rpm,
        to: Some("deb".to_string()),
        output: dir.path().join("out"),
        package_name: None,
        version: None,
        arch: None,
        distribution: None,
        build_version: "1".to_string(),
        dry_run: false,
    })
    .unwrap();

    let deb = std::fs::read_dir(dir.path().join("out"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().map(|x| x == "deb").unwrap_or(false))
        .expect("a .deb was produced");
    let ctrl = lx_lib::repo::read_control(&deb).unwrap();

    // Arch normalized RPM→deb.
    assert_eq!(ctrl.get("Architecture").map(String::as_str), Some("amd64"));
    // Epoch carried into the deb Version.
    assert!(
        ctrl.get("Version").unwrap().starts_with("2:"),
        "epoch carried: {:?}",
        ctrl.get("Version")
    );
    // Relation syntax rewritten RPM→deb, and non-Depends relations carried.
    assert!(
        ctrl.get("Depends").unwrap().contains("glibc (>= 2.17)"),
        "dep syntax converted: {:?}",
        ctrl.get("Depends")
    );
    assert!(ctrl.get("Recommends").unwrap().contains("bash"));
    assert!(ctrl.get("Conflicts").unwrap().contains("old-pkg"));
    assert!(ctrl.get("Replaces").unwrap().contains("legacy"));
    // Virtual Provides carried; the RPM self-provide and sonames dropped.
    assert_eq!(
        ctrl.get("Provides").map(String::as_str),
        Some("webserver"),
        "provides carried + filtered: {:?}",
        ctrl.get("Provides")
    );
}

#[test]
fn convert_deb_to_rpm_carries_trigger_conditions() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("debroot");
    std::fs::create_dir_all(root.join("usr/bin")).unwrap();
    std::fs::write(root.join("usr/bin/hello"), b"payload").unwrap();
    let control = b"Package: hello\nVersion: 1.0-1+bookworm\nArchitecture: amd64\nMaintainer: T <t@e.c>\nDescription: hi\n";
    let deb = dir.path().join("hello_1.0-1+bookworm_amd64.deb");
    lx_lib::debarchive::build_full(
        &root,
        control,
        0,
        &deb,
        "gzip",
        &[lx_lib::debarchive::ControlMember {
            name: "triggers".into(),
            content: b"interest cups\n".to_vec(),
            mode: 0o644,
        }],
        None,
    )
    .unwrap();

    run(ConvertArgs {
        input: deb,
        to: Some("rpm".to_string()),
        output: dir.path().join("out"),
        package_name: None,
        version: None,
        arch: None,
        distribution: None,
        build_version: "1".to_string(),
        dry_run: false,
    })
    .unwrap();

    let rpm = std::fs::read_dir(dir.path().join("out"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().map(|x| x == "rpm").unwrap_or(false))
        .expect("a .rpm was produced");
    let pkg = rpm::Package::open(&rpm).unwrap();
    let requires = pkg.metadata.get_requires().unwrap();
    assert!(
        requires
            .iter()
            .any(|d| d.name == "cups" && d.flags.contains(rpm::DependencyFlags::TRIGGERIN)),
        "deb `interest cups` became an rpm %triggerin condition: {requires:?}"
    );
}

#[test]
fn convert_deb_to_rpm_carries_conffiles() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("debroot");
    std::fs::create_dir_all(root.join("usr/bin")).unwrap();
    std::fs::create_dir_all(root.join("etc")).unwrap();
    std::fs::write(root.join("usr/bin/hello"), b"payload").unwrap();
    std::fs::write(root.join("etc/hello.conf"), b"a=1").unwrap();
    let control = b"Package: hello\nVersion: 1.0-1+bookworm\nArchitecture: amd64\nMaintainer: T <t@e.c>\nConffiles:\n /etc/hello.conf abc123\nDescription: hi\n";
    let deb = dir.path().join("hello_1.0-1+bookworm_amd64.deb");
    lx_lib::debarchive::build(&root, control, 0, &deb).unwrap();

    run(ConvertArgs {
        input: deb,
        to: Some("rpm".to_string()),
        output: dir.path().join("out"),
        package_name: None,
        version: None,
        arch: None,
        distribution: None,
        build_version: "1".to_string(),
        dry_run: false,
    })
    .unwrap();

    let rpm = std::fs::read_dir(dir.path().join("out"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().map(|x| x == "rpm").unwrap_or(false))
        .expect("a .rpm was produced");
    let pkg = rpm::Package::open(&rpm).unwrap();
    let entries = pkg.metadata.get_file_entries().unwrap();
    let flags = entries
        .iter()
        .find(|e| e.path.to_string_lossy() == "/etc/hello.conf")
        .expect("conffile present in the rpm payload")
        .flags;
    assert!(
        flags.contains(rpm::FileFlags::CONFIG),
        "deb conffile became an rpm %config: {flags:?}"
    );
}

// ---------------------------------------------------------------------------
// Functional regression tests: real-world payload shapes (maintainer scripts,
// standard `DEBIAN/conffiles` members, symlinks, arch `.INSTALL`/`.PKGINFO`)
// that the original unit fixtures did not cover.
// ---------------------------------------------------------------------------

fn convert_to(input: &std::path::Path, to: &str, out: &std::path::Path) {
    run(ConvertArgs {
        input: input.to_path_buf(),
        to: Some(to.to_string()),
        output: out.to_path_buf(),
        package_name: None,
        version: None,
        arch: None,
        distribution: None,
        build_version: "1".to_string(),
        dry_run: false,
    })
    .unwrap_or_else(|e| panic!("convert {} -> {to} failed: {e:#}", input.display()));
}

fn find_one(dir: &std::path::Path, ext: &str) -> std::path::PathBuf {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().map(|x| x == ext).unwrap_or(false))
        .unwrap_or_else(|| panic!("no .{ext} produced in {}", dir.display()))
}

/// Read every `control.tar.*` member of a `.deb` by name.
fn deb_control_members(deb: &std::path::Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    let file = std::fs::File::open(deb).unwrap();
    let mut archive = ar::Archive::new(file);
    while let Some(entry) = archive.next_entry() {
        let mut entry = entry.unwrap();
        let name = String::from_utf8_lossy(entry.header().identifier()).to_string();
        if !name.starts_with("control.tar") {
            continue;
        }
        let mut data = Vec::new();
        std::io::copy(&mut entry, &mut data).unwrap();
        let decoded = if name.ends_with(".gz") {
            let mut out = Vec::new();
            std::io::copy(&mut flate2::read::GzDecoder::new(data.as_slice()), &mut out).unwrap();
            out
        } else {
            data
        };
        let mut members = std::collections::BTreeMap::new();
        let mut tar = tar::Archive::new(decoded.as_slice());
        for member in tar.entries().unwrap() {
            let mut member = member.unwrap();
            let name = member
                .path()
                .unwrap()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string();
            let mut buf = Vec::new();
            std::io::Read::read_to_end(&mut member, &mut buf).unwrap();
            members.insert(name, buf);
        }
        return members;
    }
    std::collections::BTreeMap::new()
}

/// Read a `.pkg.tar.zst`'s `.PKGINFO` as text.
fn arch_pkginfo(pkg: &std::path::Path) -> String {
    let file = std::fs::File::open(pkg).unwrap();
    let decoder = zstd::stream::read::Decoder::new(file).unwrap();
    let mut archive = tar::Archive::new(decoder);
    for entry in archive.entries().unwrap() {
        let mut entry = entry.unwrap();
        if entry.path().unwrap().file_name().and_then(|n| n.to_str()) == Some(".PKGINFO") {
            let mut text = String::new();
            std::io::Read::read_to_string(&mut entry, &mut text).unwrap();
            return text;
        }
    }
    String::new()
}

fn mem(name: &str, content: &[u8], mode: u32) -> lx_lib::debarchive::ControlMember {
    lx_lib::debarchive::ControlMember {
        name: name.to_string(),
        content: content.to_vec(),
        mode,
    }
}

/// A `.deb` whose maintainer scripts live in `DEBIAN/` members (the normal
/// layout) and whose conffile is in the `DEBIAN/conffiles` member — not the
/// control field the old fixture used.
fn deb_with_scripts_and_conffiles(dir: &std::path::Path) -> std::path::PathBuf {
    let root = dir.join("root");
    std::fs::create_dir_all(root.join("usr/bin")).unwrap();
    std::fs::create_dir_all(root.join("etc/demo")).unwrap();
    std::fs::write(root.join("usr/bin/demo"), b"payload").unwrap();
    std::fs::write(root.join("etc/demo/demo.conf"), b"a=1\n").unwrap();
    let control = b"Package: demo\nVersion: 1.0-1+bookworm\nArchitecture: amd64\nMaintainer: T <t@e.c>\nDepends: libc6 (>= 2.36)\nDescription: hi\n";
    let deb = dir.join("demo_1.0-1+bookworm_amd64.deb");
    lx_lib::debarchive::build_full(
        &root,
        control,
        0,
        &deb,
        "gzip",
        &[
            mem("preinst", b"#!/bin/sh\necho pre-install\n", 0o755),
            mem("postrm", b"#!/bin/sh\necho post-remove\n", 0o755),
            mem("conffiles", b"/etc/demo/demo.conf\n", 0o644),
        ],
        None,
    )
    .unwrap();
    deb
}

#[test]
fn deb_scripts_and_conffiles_member_reach_rpm() {
    let dir = tempfile::tempdir().unwrap();
    let deb = deb_with_scripts_and_conffiles(dir.path());

    convert_to(&deb, "rpm", &dir.path().join("out"));
    let rpm = find_one(&dir.path().join("out"), "rpm");
    let pkg = rpm::Package::open(&rpm).unwrap();

    // DEBIAN/preinst (content lived in a member) must reach the %pre scriptlet.
    let pre = pkg.metadata.get_pre_install_script().unwrap();
    assert!(
        pre.script.contains("pre-install"),
        "preinst body carried: {:?}",
        pre.script
    );

    // DEBIAN/conffiles member must mark the file %config.
    let entries = pkg.metadata.get_file_entries().unwrap();
    let conf = entries
        .iter()
        .find(|e| e.path.to_string_lossy() == "/etc/demo/demo.conf")
        .expect("conffile present in the rpm payload");
    assert!(
        conf.flags.contains(rpm::FileFlags::CONFIG),
        "DEBIAN/conffiles member became an rpm %config: {:?}",
        conf.flags
    );
}

#[test]
fn deb_scripts_and_conffiles_member_reach_arch() {
    let dir = tempfile::tempdir().unwrap();
    let deb = deb_with_scripts_and_conffiles(dir.path());

    convert_to(&deb, "arch", &dir.path().join("out"));
    let arch = find_one(&dir.path().join("out"), "zst");

    // The deb conffile becomes a pacman `backup` entry (noreplace).
    let pkginfo = arch_pkginfo(&arch);
    assert!(
        pkginfo.contains("backup = etc/demo/demo.conf"),
        "conffile -> pacman backup: {pkginfo}"
    );

    // `DEBIAN/conffiles` must not be shipped as payload.
    let dest = dir.path().join("extract");
    lx_lib::archarchive::extract(&arch, &dest).unwrap();
    assert!(!dest.join(".PKGINFO").exists());
    assert!(!dest.join(".MTREE").exists());
    assert!(!dest.join(".INSTALL").exists());
}

#[test]
fn rpm_scripts_reach_deb_and_soname_requires_are_filtered() {
    use lx_lib::rpmarchive::{self, BuildOptions, PackageMeta, RpmRelations};
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("rpmroot");
    std::fs::create_dir_all(root.join("usr/bin")).unwrap();
    std::fs::write(root.join("usr/bin/hello"), b"payload").unwrap();
    let rpm = dir.path().join("hello-1.0-1.x86_64.rpm");
    rpmarchive::build_with_options(
        &root,
        &PackageMeta {
            name: "hello",
            version: "1.0",
            release: "1",
            summary: "hi",
            description: "hi",
            license: "MIT",
            vendor: None,
            packager: None,
        },
        "amd64",
        0,
        &rpm,
        &BuildOptions {
            pre_install: Some("#!/bin/sh\necho rpm-pre\n"),
            post_uninstall: Some("#!/bin/sh\necho rpm-postun\n"),
            relations: RpmRelations {
                requires: vec![
                    rpm::Dependency::greater_eq("glibc", "2.17"),
                    // Auto-generated capability that is not a valid deb name.
                    rpm::Dependency::any("libc.so.6()(64bit)"),
                ],
                ..Default::default()
            },
            ..Default::default()
        },
        &lx_lib::filemeta::FileMetaMap::new(),
    )
    .unwrap();

    convert_to(&rpm, "deb", &dir.path().join("out"));
    let deb = find_one(&dir.path().join("out"), "deb");

    let members = deb_control_members(&deb);
    assert!(
        members
            .get("preinst")
            .is_some_and(|c| String::from_utf8_lossy(c).contains("rpm-pre")),
        "rpm %pre -> deb preinst: {:?}",
        members.keys().collect::<Vec<_>>()
    );
    assert!(
        members
            .get("postrm")
            .is_some_and(|c| String::from_utf8_lossy(c).contains("rpm-postun")),
        "rpm %postun -> deb postrm"
    );

    let ctrl = lx_lib::repo::read_control(&deb).unwrap();
    let depends = ctrl.get("Depends").map(String::as_str).unwrap_or("");
    assert!(
        depends.contains("glibc (>= 2.17)"),
        "versioned require converted: {depends:?}"
    );
    assert!(
        !depends.contains(".so"),
        "soname/capability requires filtered: {depends:?}"
    );
}

#[test]
fn arch_install_conflict_and_epoch_reach_deb() {
    use lx_lib::archarchive::{self, PackageMeta, PackageRelations};
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("archroot");
    std::fs::create_dir_all(root.join("usr/bin")).unwrap();
    std::fs::write(root.join("usr/bin/demo"), b"payload").unwrap();
    let arch = dir.path().join("demo-1.0-1-x86_64.pkg.tar.zst");
    archarchive::build_with_relations(
        &root,
        &PackageMeta {
            name: "demo",
            version: "1.0",
            release: "1",
            description: "hi",
            url: "https://example.com/demo",
            license: "MIT",
        },
        &PackageRelations {
            conflicts: &["demo-old".to_string()],
            ..Default::default()
        },
        None,
        Some("2"),
        "x86_64",
        0,
        &arch,
        Some("post_install() {\n  echo arch-post-install\n}\n"),
        &lx_lib::filemeta::FileMetaMap::new(),
    )
    .unwrap();

    convert_to(&arch, "deb", &dir.path().join("out"));
    let deb = find_one(&dir.path().join("out"), "deb");

    let ctrl = lx_lib::repo::read_control(&deb).unwrap();
    assert_eq!(
        ctrl.get("Conflicts").map(String::as_str),
        Some("demo-old"),
        "pacman `conflict` carried (its PKGINFO key is singular)"
    );
    assert!(
        ctrl.get("Version").unwrap().starts_with("2:"),
        "epoch split out of pkgver: {:?}",
        ctrl.get("Version")
    );

    let members = deb_control_members(&deb);
    assert!(
        members
            .get("postinst")
            .is_some_and(|c| String::from_utf8_lossy(c).contains("arch-post-install")),
        ".INSTALL post_install -> deb postinst: {:?}",
        members.keys().collect::<Vec<_>>()
    );

    // `.PKGINFO`/`.MTREE`/`.INSTALL` are control members, not payload.
    let dest = dir.path().join("payload");
    lx_lib::debarchive::extract(&deb, &dest).unwrap();
    for control in [".PKGINFO", ".MTREE", ".INSTALL"] {
        assert!(
            !dest.join(control).exists(),
            "{control} leaked into the converted payload"
        );
    }
}

#[test]
fn convert_preserves_symlinks_in_both_directions() {
    use lx_lib::rpmarchive::{self, BuildOptions, PackageMeta};
    let dir = tempfile::tempdir().unwrap();

    // deb -> arch, with an absolute symlink in the payload.
    let root = dir.path().join("root");
    std::fs::create_dir_all(root.join("usr/bin")).unwrap();
    std::fs::create_dir_all(root.join("usr/lib")).unwrap();
    std::fs::write(root.join("usr/bin/demo"), b"payload").unwrap();
    std::os::unix::fs::symlink("/usr/bin/demo", root.join("usr/lib/demo-alias")).unwrap();
    let control = b"Package: demo\nVersion: 1.0-1+bookworm\nArchitecture: amd64\nMaintainer: T <t@e.c>\nDescription: hi\n";
    let deb = dir.path().join("demo_1.0-1+bookworm_amd64.deb");
    lx_lib::debarchive::build(&root, control, 0, &deb).unwrap();

    convert_to(&deb, "arch", &dir.path().join("out1"));
    let arch = find_one(&dir.path().join("out1"), "zst");
    let dest = dir.path().join("arch-payload");
    lx_lib::archarchive::extract(&arch, &dest).unwrap();
    let link = dest.join("usr/lib/demo-alias");
    assert!(
        std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink(),
        "deb -> arch flattened the symlink"
    );
    assert_eq!(
        std::fs::read_link(&link).unwrap().to_string_lossy(),
        "/usr/bin/demo"
    );

    // rpm -> deb, with a relative symlink in the payload.
    let rpmroot = dir.path().join("rpmroot");
    std::fs::create_dir_all(rpmroot.join("usr/bin")).unwrap();
    std::fs::create_dir_all(rpmroot.join("usr/lib")).unwrap();
    std::fs::write(rpmroot.join("usr/bin/demo"), b"payload").unwrap();
    std::os::unix::fs::symlink("../bin/demo", rpmroot.join("usr/lib/demo-link")).unwrap();
    let rpm = dir.path().join("demo-1.0-1.x86_64.rpm");
    rpmarchive::build_with_options(
        &rpmroot,
        &PackageMeta {
            name: "demo",
            version: "1.0",
            release: "1",
            summary: "hi",
            description: "hi",
            license: "MIT",
            vendor: None,
            packager: None,
        },
        "amd64",
        0,
        &rpm,
        &BuildOptions::default(),
        &lx_lib::filemeta::FileMetaMap::new(),
    )
    .unwrap();

    convert_to(&rpm, "deb", &dir.path().join("out2"));
    let deb2 = find_one(&dir.path().join("out2"), "deb");
    let dest2 = dir.path().join("deb-payload");
    lx_lib::debarchive::extract(&deb2, &dest2).unwrap();
    let link2 = dest2.join("usr/lib/demo-link");
    assert!(
        std::fs::symlink_metadata(&link2)
            .unwrap()
            .file_type()
            .is_symlink(),
        "rpm -> deb flattened the symlink"
    );
    assert_eq!(
        std::fs::read_link(&link2).unwrap().to_string_lossy(),
        "../bin/demo"
    );
}
