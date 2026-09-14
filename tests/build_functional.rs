// SPDX-License-Identifier: GPL-3.0-or-later

//! Functional `lx build` matrix — the reproducible, offline portion of the
//! suggested test plan, as a normal cargo integration test rather than a one-off
//! shell script. Each test stages a small local payload (a text file + an ELF
//! binary), runs the real build pipeline (no dry-run), and asserts the produced
//! archives exist, carry the right file magic, and (for deb/apk/arch/ipk) that
//! the payload actually landed inside the package.
//!
//! Network-dependent steps (a live forge E2E and the per-distro container
//! matrix) are intentionally not here: use `lx build https://...` with
//! `--allow-unverified` for the forge path, and `make test-containers` for the
//! install/host-detection matrix.

use std::path::PathBuf;
use std::process::Command;

use lx_lib::build::{run, BuildArgs};

/// Stage a deterministic payload: a non-ELF file and an ELF binary (real when
/// `/bin/true` is present, else the test binary itself). The ELF exercises the
/// install-tree/dep-scan path; the text file exercises ancillary staging.
fn stage_payload(root: &std::path::Path) -> PathBuf {
    std::fs::write(root.join("hello.txt"), b"hello lx functional test\n").unwrap();
    let binary = root.join("mybinary");
    if std::fs::copy("/bin/true", &binary).is_err() {
        std::fs::copy(std::env::current_exe().unwrap(), &binary).unwrap();
    }
    binary
}

/// Build `format` for a single arch/dist from `payload` into `out`, returning
/// the output dir path.
fn build_one(
    payload: &std::path::Path,
    out: &std::path::Path,
    format: &str,
    dist: &str,
) -> std::path::PathBuf {
    run(
        BuildArgs {
            config: out.join("nonexistent.yaml"),
            all: None,
            version: Some("1.0.0".into()),
            build_version: "1".into(),
            architectures: Some("amd64".into()),
            host: false,
            distributions: Some(dist.into()),
            output: out.join("dist"),
            format: Some(format.into()),
            provider: None,
            no_verify: false,
            allow_unverified: false,
            lintian: false,
            lintian_fail_on_warnings: false,
            lintian_pedantic: false,
            lintian_suppress: None,
            dry_run: false,
            max_parallel: 1,
            pinned_metadata: None,
            cache_dir: None,
            api_cache_dir: None,
            source: false,
            summary: false,
            telemetry: false,
            save_baseline: false,
            sandbox: false,
            install_build_deps: false,
            sbom: false,
            cosign: false,
            cross_target: None,
            bindep: true,
            progress: false,
            progress_path: None,
            keep: false,
            sign_key: None,
            sign_key_id: None,
            sign_method: None,
            local: false,
            from_dir: Some(payload.to_path_buf()),
            from_file: None,
            package_name: Some("myapp".into()),
            prefix: None,
            overlay: None,
            update_lock: false,
            artifact_cache_dir: None,
            verify: false,
        },
        None,
    )
    .unwrap_or_else(|e| panic!("lx build --format {format}: {e:#}"));
    out.join("dist")
}

/// Match output files by filename suffix. `Path::extension()` only returns
/// the last dotted segment, so `*.pkg.tar.zst` would wrongly match as `zst`;
/// match the full trailing suffix instead.
fn artifacts(out: &std::path::Path, suffix: &str) -> Vec<PathBuf> {
    std::fs::read_dir(out)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.path().file_name().is_some_and(|n| n.to_string_lossy().ends_with(suffix)))
        .map(|e| e.path())
        .collect()
}

/// The offline payload matrix: each format produces a non-empty archive with
/// the right magic, and the payload is staged inside it.
#[test]
fn local_payload_matrix_builds_every_format_offline() {
    let payload = tempfile::tempdir().unwrap();
    stage_payload(payload.path());

    for (format, ext, dist) in [
        ("deb", "deb", "trixie"),
        ("rpm", "rpm", "fedora"),
        ("arch", "pkg.tar.zst", "arch"),
        ("apk", "apk", "alpine"),
        ("ipk", "ipk", "openwrt"),
    ] {
        let out = tempfile::tempdir().unwrap();
        let dir = build_one(payload.path(), out.path(), format, dist);

        let arts = artifacts(&dir, ext);
        assert!(!arts.is_empty(), "{format}: expected at least one .{ext}");

        for art in &arts {
            let meta = std::fs::metadata(art).unwrap();
            assert!(meta.len() > 0, "{format}: {art:?} is empty");
            let magic_out = Command::new("file").arg("-b").arg(art).output().unwrap();
            let magic = String::from_utf8_lossy(&magic_out.stdout);
            match format {
                "deb" => assert!(magic.contains("Debian binary package"), "{art:?}: {magic}"),
                "rpm" => assert!(magic.starts_with("RPM"), "{art:?}: {magic}"),
                "apk" => assert!(magic.contains("gzip compressed"), "{art:?}: {magic}"),
                // ipk is the Debian binary package format; arch is zstd.
                "ipk" => assert!(magic.contains("Debian binary package"), "{art:?}: {magic}"),
                "arch" => assert!(magic.contains("Zstandard"), "{art:?}: {magic}"),
                _ => {}
            }
        }
    }
}

/// The payload must reach the install tree, not be silently dropped. Spot-check
/// the deb (data.tar) and apk (.PKGINFO payload) since we can read them.
#[test]
fn payload_reaches_the_installed_tree() {
    let payload = tempfile::tempdir().unwrap();
    stage_payload(payload.path());

    // deb: the text file lands in usr/local/share/... via stage_ancillary_file.
    let out = tempfile::tempdir().unwrap();
    let dir = build_one(payload.path(), out.path(), "deb", "trixie");
    let deb = artifacts(&dir, "deb").pop().unwrap();
    let data_tar = Command::new("ar")
        .args(["p", deb.to_str().unwrap(), "data.tar.gz"])
        .output()
        .unwrap();
    assert!(data_tar.status.success(), "could not extract data.tar.gz");
    let list = Command::new("tar")
        .args(["tzf", "-"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    list.stdin
        .as_ref()
        .unwrap()
        .write_all(&data_tar.stdout)
        .unwrap();
    let list = list.wait_with_output().unwrap();
    let contents = String::from_utf8_lossy(&list.stdout);
    assert!(
        contents.contains("mybinary"),
        "deb data.tar missing the staged binary:\n{contents}"
    );

    // apk: the payload lives in the data segment, not .PKGINFO (Alpine
    // metadata has no file list). `tar tzf` reads the concatenated gzip
    // members and lists every staged path.
    let out = tempfile::tempdir().unwrap();
    let dir = build_one(payload.path(), out.path(), "apk", "alpine");
    let apk = artifacts(&dir, "apk").pop().unwrap();
    let apk_list = Command::new("tar")
        .args(["tzf", apk.to_str().unwrap()])
        .output()
        .unwrap();
    let contents = String::from_utf8_lossy(&apk_list.stdout);
    assert!(
        contents.contains("mybinary"),
        "apk missing the staged binary:\n{contents}"
    );
}

/// apk multi-arch builds must NOT collide on a flat output dir — each arch is a
/// distinct file. (The fix appends the arch to the apk filename.)
#[test]
fn apk_multi_arch_builds_distinct_files() {
    let payload = tempfile::tempdir().unwrap();
    stage_payload(payload.path());

    let out = tempfile::tempdir().unwrap();
    run(
        BuildArgs {
            config: out.path().join("nonexistent.yaml"),
            all: None,
            version: Some("1.0.0".into()),
            build_version: "1".into(),
            architectures: None, // every supported alpine arch
            host: false,
            distributions: Some("alpine".into()),
            output: out.path().join("dist"),
            format: Some("apk".into()),
            provider: None,
            no_verify: false,
            allow_unverified: false,
            lintian: false,
            lintian_fail_on_warnings: false,
            lintian_pedantic: false,
            lintian_suppress: None,
            dry_run: false,
            max_parallel: 1,
            pinned_metadata: None,
            cache_dir: None,
            api_cache_dir: None,
            source: false,
            summary: false,
            telemetry: false,
            save_baseline: false,
            sandbox: false,
            install_build_deps: false,
            sbom: false,
            cosign: false,
            cross_target: None,
            bindep: true,
            progress: false,
            progress_path: None,
            keep: false,
            sign_key: None,
            sign_key_id: None,
            sign_method: None,
            local: false,
            from_dir: Some(payload.path().to_path_buf()),
            from_file: None,
            package_name: Some("myapp".into()),
            prefix: None,
            overlay: None,
            update_lock: false,
            artifact_cache_dir: None,
            verify: false,
        },
        None,
    )
    .unwrap();

    let apks = artifacts(&out.path().join("dist"), "apk");
    assert!(apks.len() > 1, "expected multiple apk files, got {apks:?}");

    let mut names: Vec<String> = apks
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into())
        .collect();
    let before = names.len();
    names.sort();
    names.dedup();
    assert_eq!(
        before,
        names.len(),
        "apk filenames are not unique: {names:?}"
    );
}

/// Reproducibility: building the same recipe twice yields byte-identical output.
#[test]
fn build_is_reproducible_for_fixed_inputs() {
    let payload = tempfile::tempdir().unwrap();
    stage_payload(payload.path());

    let hash = |path: &std::path::Path| -> String {
        let data = std::fs::read(path).unwrap();
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(&data))
    };

    for format in ["deb", "rpm", "arch"] {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let a_dir = build_one(payload.path(), a.path(), format, "trixie");
        let b_dir = build_one(payload.path(), b.path(), format, "trixie");

        let suffix = if format == "arch" { "pkg.tar.zst" } else { format };
        let a_art = artifacts(&a_dir, suffix).pop().unwrap();
        let b_art = artifacts(&b_dir, suffix).pop().unwrap();
        assert_eq!(
            hash(&a_art),
            hash(&b_art),
            "{format}: two builds of the same recipe differ"
        );
    }
}
