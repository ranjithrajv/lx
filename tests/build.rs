// SPDX-License-Identifier: GPL-3.0-or-later

use lx_lib::build::*;
use lx_lib::plugins::forge::github::parse_github_url;

#[test]
fn verify_method_as_str_names_every_variant() {
    assert_eq!(VerifyMethod::Pinned.as_str(), "pinned");
    assert_eq!(VerifyMethod::Sidecar.as_str(), "sidecar");
    assert_eq!(
        VerifyMethod::UnverifiedAllowed.as_str(),
        "unverified (--allow-unverified)"
    );
    assert_eq!(
        VerifyMethod::SkippedNoVerify.as_str(),
        "skipped (--no-verify)"
    );
}

#[test]
fn parse_github_url_extracts_owner_repo() {
    assert_eq!(
        parse_github_url("https://github.com/eza-community/eza").as_deref(),
        Some("eza-community/eza")
    );
}

#[test]
fn parse_github_url_ignores_trailing_path() {
    assert_eq!(
        parse_github_url("https://github.com/eza-community/eza/releases/tag/v0.24.0").as_deref(),
        Some("eza-community/eza")
    );
    assert_eq!(
        parse_github_url("https://github.com/eza-community/eza.git").as_deref(),
        Some("eza-community/eza")
    );
    assert_eq!(
        parse_github_url("https://github.com/eza-community/eza/").as_deref(),
        Some("eza-community/eza")
    );
}

#[test]
fn parse_github_url_rejects_non_github_or_incomplete() {
    assert!(parse_github_url("package.yaml").is_none());
    assert!(parse_github_url("configs/eza.yaml").is_none());
    assert!(parse_github_url("https://gitlab.com/owner/repo").is_none());
    assert!(parse_github_url("https://github.com/owner-only").is_none());
}

#[test]
fn host_arch_returns_a_known_debian_arch() {
    // Smoke test on this (Linux) dev/CI host: --host relies on
    // host_arch() resolving to one of the architectures the build
    // matrix actually knows about.
    let arch = host_arch().expect("uname -m should resolve on Linux");
    assert!(
        lx_lib::config::DEFAULT_ARCHITECTURES.contains(&arch.as_str()),
        "unexpected arch: {arch}"
    );
}

#[test]
fn version_placeholder_dedupes_literal_v_prefix() {
    use lx_lib::build::expand_version_placeholder as x;
    // Legacy pattern with literal v + v-prefixed tag -> single v.
    assert_eq!(
        x("eza_v{version}_x86_64-unknown-linux-gnu.tar.gz", "v0.23.5"),
        "eza_v0.23.5_x86_64-unknown-linux-gnu.tar.gz"
    );
    // Bare-version tag (the action's convention) keeps the literal v.
    assert_eq!(
        x("pkg_v{version}_linux.tar.gz", "1.2.3"),
        "pkg_v1.2.3_linux.tar.gz"
    );
    // Modern patterns without an adjacent v get the tag verbatim.
    assert_eq!(x("tool-{version}.tar.gz", "v9.9.9"), "tool-v9.9.9.tar.gz");
    assert_eq!(x("tool-{version}.tar.gz", "9.9.9"), "tool-9.9.9.tar.gz");
    // Case-insensitive on both sides.
    assert_eq!(x("Pkg_V{version}.zip", "V3.0"), "Pkg_V3.0.zip");
    // Multiple placeholders, each deduped.
    assert_eq!(x("a_v{version}/b-{version}", "v2"), "a_v2/b-v2");
}

#[test]
fn local_dry_run_builds_matrix_from_payload_dir() {
    let payload = tempfile::tempdir().unwrap();
    std::fs::write(payload.path().join("hello"), b"\x7fELFfake").unwrap();

    let cfg_dir = tempfile::tempdir().unwrap();
    let cfg_path = cfg_dir.path().join("package.yaml");
    std::fs::write(
        &cfg_path,
        format!(
            r#"
package_name: hello
github_repo: owner/hello
version: "1.0.0"
local_payload: {}
architectures: [amd64]
debian_distributions: [trixie]
"#,
            payload.path().display()
        ),
    )
    .unwrap();

    let args = BuildArgs {
        config: cfg_path,
        all: None,
        version: None,
        build_version: "1".into(),
        architectures: None,
        host: false,
        distributions: None,
        output: cfg_dir.path().join("dist"),
        format: None,
        provider: None,
        no_verify: false,
        allow_unverified: false,
        lintian: false,
        lintian_fail_on_warnings: false,
        lintian_pedantic: false,
        lintian_suppress: None,
        dry_run: true,
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
        sign_method: Some("debsign".into()),
        local: true,
        from_dir: None,
        from_file: None,
        package_name: None,
        prefix: None,
        overlay: None,
        update_lock: false,
        artifact_cache_dir: None,
        verify: false,
    };
    run(args, None).expect("local --dry-run should succeed");
}

/// `--all` fans a fleet manifest out into one dry-run build per listed
/// package.yaml, all sharing the payload directory's binary.
#[test]
fn all_builds_every_package_in_fleet_manifest() {
    let payload = tempfile::tempdir().unwrap();
    std::fs::write(payload.path().join("hello"), b"\x7fELFfake").unwrap();

    let fleet_dir = tempfile::tempdir().unwrap();
    let write_pkg = |name: &str| {
        let path = fleet_dir.path().join(format!("{name}.yaml"));
        std::fs::write(
            &path,
            format!(
                r#"
package_name: {name}
github_repo: owner/{name}
version: "1.0.0"
local_payload: {}
architectures: [amd64]
debian_distributions: [trixie]
"#,
                payload.path().display()
            ),
        )
        .unwrap();
        format!("{name}.yaml")
    };
    write_pkg("hello");
    write_pkg("world");

    let fleet_path = fleet_dir.path().join("packages.yaml");
    std::fs::write(&fleet_path, "packages: [hello.yaml, world.yaml]\n").unwrap();

    let args = BuildArgs {
        config: fleet_path.clone(),
        all: Some(fleet_path),
        version: None,
        build_version: "1".into(),
        architectures: None,
        host: false,
        distributions: None,
        output: fleet_dir.path().join("dist"),
        format: None,
        provider: None,
        no_verify: false,
        allow_unverified: false,
        lintian: false,
        lintian_fail_on_warnings: false,
        lintian_pedantic: false,
        lintian_suppress: None,
        dry_run: true,
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
        sign_method: Some("debsign".into()),
        local: true,
        from_dir: None,
        from_file: None,
        package_name: None,
        prefix: None,
        overlay: None,
        update_lock: false,
        artifact_cache_dir: None,
        verify: false,
    };
    run(args, None).expect("fleet dry-run should succeed for every listed package");
}

/// `BuildArgs` for a real (non-dry-run) reproducibility test: a local-payload
/// build of one deb (trixie), with output and artifact cache under `dir`.
fn verify_build_args(config: &std::path::Path, dir: &std::path::Path, verify: bool) -> BuildArgs {
    BuildArgs {
        config: config.to_path_buf(),
        all: None,
        version: None,
        build_version: "1".into(),
        architectures: None,
        host: false,
        distributions: None,
        output: dir.join("dist"),
        format: None,
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
        sign_method: Some("debsign".into()),
        local: true,
        from_dir: None,
        from_file: None,
        package_name: None,
        prefix: None,
        overlay: None,
        update_lock: false,
        artifact_cache_dir: Some(dir.join("artifact-cache")),
        verify,
    }
}

/// Cached artifacts (the key-addressed blobs, not the `.name` sidecars).
fn cached_artifacts(cache_dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(cache_dir)
        .expect("artifact cache dir should exist after a baseline build")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_none_or(|e| e != "name"))
        .collect()
}

/// Drive a real baseline + `--verify` cycle and prove `--verify` actually
/// byte-compares rather than silently re-baselining:
///   1. a baseline build must populate the artifact cache,
///   2. `--verify` on the same recipe must pass,
///   3. tampering the cached baseline must then make `--verify` fail.
///
/// Step 3 is the regression guard: before the artifact cache covered this
/// payload kind, every `--verify` run printed "no prior cached build … is now
/// the baseline" and this test passed vacuously.
fn assert_verify_actually_compares(config: &std::path::Path, dir: &std::path::Path) {
    run(verify_build_args(config, dir, false), None)
        .expect("baseline build should populate the cache");

    let cached = cached_artifacts(&dir.join("artifact-cache"));
    assert_eq!(
        cached.len(),
        1,
        "baseline must cache exactly one artifact: {cached:?}"
    );

    run(verify_build_args(config, dir, true), None).expect("rebuild should verify as reproducible");

    std::fs::write(&cached[0], b"tampered cached artifact").unwrap();
    let err = run(verify_build_args(config, dir, true), None)
        .expect_err("--verify must fail once the cached baseline no longer matches the rebuild");
    // `run` aggregates per-job failures into "N of M builds failed"; the
    // specific "not reproducible" reason is only printed. What proves the
    // byte-compare actually ran is that the pipeline also did not re-baseline:
    // a vacuous `--verify` would have overwritten the cache with the fresh
    // artifact via `cache.put` (which only runs after a successful compare).
    assert!(
        err.to_string().contains("builds failed"),
        "expected the tampered verify run to fail the build, got: {err}"
    );
    assert_eq!(
        std::fs::read(&cached[0]).unwrap(),
        b"tampered cached artifact",
        "a failed --verify must not overwrite (re-baseline) the artifact cache"
    );
}

/// `--verify` (nix build --check parity) over a **directory** payload
/// (`local_payload: <dir>`): the tree hash keys the artifact cache, so the
/// rebuild is genuinely byte-compared against the cached baseline.
#[test]
fn verify_confirms_a_deterministic_rebuild() {
    let payload = tempfile::tempdir().unwrap();
    std::fs::write(payload.path().join("hello"), b"\x7fELFfake").unwrap();

    let cfg_dir = tempfile::tempdir().unwrap();
    let cfg_path = cfg_dir.path().join("package.yaml");
    std::fs::write(
        &cfg_path,
        format!(
            r#"
package_name: hello
github_repo: owner/hello
version: "1.0.0"
local_payload: {}
architectures: [amd64]
debian_distributions: [trixie]
"#,
            payload.path().display()
        ),
    )
    .unwrap();

    assert_verify_actually_compares(&cfg_path, cfg_dir.path());
}

/// Same as above for a **file** payload (a raw binary), covering the
/// single-file digest branch of the artifact-cache key.
#[test]
fn verify_confirms_a_deterministic_rebuild_from_file_payload() {
    let payload = tempfile::tempdir().unwrap();
    let asset = payload.path().join("hello");
    std::fs::write(&asset, b"\x7fELFfake").unwrap();

    let cfg_dir = tempfile::tempdir().unwrap();
    let cfg_path = cfg_dir.path().join("package.yaml");
    std::fs::write(
        &cfg_path,
        format!(
            r#"
package_name: hello
github_repo: owner/hello
version: "1.0.0"
local_payload: {}
architectures: [amd64]
debian_distributions: [trixie]
"#,
            asset.display()
        ),
    )
    .unwrap();

    assert_verify_actually_compares(&cfg_path, cfg_dir.path());
}

// ---------------------------------------------------------------------------
// "You supply files" mode: --from-dir / --from-file (fpm-style)
// ---------------------------------------------------------------------------

/// `--from-dir` builds a package from a directory of files you supply, with
/// no package.yaml at all (metadata via CLI flags).
#[test]
fn from_dir_dry_run_builds_matrix_with_cli_metadata() {
    let payload = tempfile::tempdir().unwrap();
    std::fs::write(payload.path().join("mybinary"), b"\x7fELFfake").unwrap();
    std::fs::write(payload.path().join("README"), b"docs").unwrap();

    let out = tempfile::tempdir().unwrap();
    let args = BuildArgs {
        // No package.yaml exists; use the default config path which won't be
        // found and falls back to defaults.
        config: out.path().join("nonexistent.yaml"),
        all: None,
        version: Some("1.0.0".into()),
        build_version: "1".into(),
        architectures: Some("amd64".into()),
        host: false,
        distributions: Some("trixie".into()),
        output: out.path().join("dist"),
        format: None,
        provider: None,
        no_verify: false,
        allow_unverified: false,
        lintian: false,
        lintian_fail_on_warnings: false,
        lintian_pedantic: false,
        lintian_suppress: None,
        dry_run: true,
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
        sign_method: Some("debsign".into()),
        local: false,
        from_dir: Some(payload.path().to_path_buf()),
        from_file: None,
        package_name: Some("myapp".into()),
        prefix: None,
        overlay: None,
        update_lock: false,
        artifact_cache_dir: None,
        verify: false,
    };
    run(args, None).expect("--from-dir --dry-run should succeed");
}

/// `--from-file` builds a package from a single file you supply.
#[test]
fn from_file_dry_run_builds_matrix() {
    let payload = tempfile::tempdir().unwrap();
    let binary_path = payload.path().join("mybinary");
    std::fs::write(&binary_path, b"\x7fELFfake").unwrap();

    let out = tempfile::tempdir().unwrap();
    let args = BuildArgs {
        config: out.path().join("nonexistent.yaml"),
        all: None,
        version: Some("2.0.0".into()),
        build_version: "1".into(),
        architectures: Some("amd64".into()),
        host: false,
        distributions: Some("trixie".into()),
        output: out.path().join("dist"),
        format: None,
        provider: None,
        no_verify: false,
        allow_unverified: false,
        lintian: false,
        lintian_fail_on_warnings: false,
        lintian_pedantic: false,
        lintian_suppress: None,
        dry_run: true,
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
        sign_method: Some("debsign".into()),
        local: false,
        from_dir: None,
        from_file: Some(binary_path),
        package_name: Some("myapp".into()),
        prefix: None,
        overlay: None,
        update_lock: false,
        artifact_cache_dir: None,
        verify: false,
    };
    run(args, None).expect("--from-file --dry-run should succeed");
}

/// `--from-dir` with `--prefix` stages files at the custom path.
#[test]
fn from_dir_with_prefix_stages_at_custom_path() {
    let payload = tempfile::tempdir().unwrap();
    std::fs::write(payload.path().join("mybinary"), b"\x7fELFfake").unwrap();

    let out = tempfile::tempdir().unwrap();
    let args = BuildArgs {
        config: out.path().join("nonexistent.yaml"),
        all: None,
        version: Some("1.0.0".into()),
        build_version: "1".into(),
        architectures: Some("amd64".into()),
        host: false,
        distributions: Some("trixie".into()),
        output: out.path().join("dist"),
        format: None,
        provider: None,
        no_verify: false,
        allow_unverified: false,
        lintian: false,
        lintian_fail_on_warnings: false,
        lintian_pedantic: false,
        lintian_suppress: None,
        dry_run: true,
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
        sign_method: Some("debsign".into()),
        local: false,
        from_dir: Some(payload.path().to_path_buf()),
        from_file: None,
        package_name: Some("myapp".into()),
        prefix: Some("/usr/local/bin".into()),
        overlay: None,
        update_lock: false,
        artifact_cache_dir: None,
        verify: false,
    };
    run(args, None).expect("--from-dir --prefix --dry-run should succeed");
}

/// `--from-dir` auto-fills package_name from the directory name when neither
/// package.yaml nor `--package-name` provides one.
#[test]
fn from_dir_auto_fills_package_name_from_dir() {
    let payload = tempfile::tempdir().unwrap();
    let sub = payload.path().join("my-cool-app");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(sub.join("binary"), b"\x7fELFfake").unwrap();

    let out = tempfile::tempdir().unwrap();
    let args = BuildArgs {
        config: out.path().join("nonexistent.yaml"),
        all: None,
        version: Some("1.0.0".into()),
        build_version: "1".into(),
        architectures: Some("amd64".into()),
        host: false,
        distributions: Some("trixie".into()),
        output: out.path().join("dist"),
        format: None,
        provider: None,
        no_verify: false,
        allow_unverified: false,
        lintian: false,
        lintian_fail_on_warnings: false,
        lintian_pedantic: false,
        lintian_suppress: None,
        dry_run: true,
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
        sign_method: Some("debsign".into()),
        local: false,
        from_dir: Some(sub),
        from_file: None,
        package_name: None,
        prefix: None,
        overlay: None,
        update_lock: false,
        artifact_cache_dir: None,
        verify: false,
    };
    run(args, None).expect("--from-dir with auto-name should succeed");
}

/// `--from-dir` with an invalid prefix (relative path) fails validation.
#[test]
fn from_dir_rejects_relative_prefix() {
    let payload = tempfile::tempdir().unwrap();
    std::fs::write(payload.path().join("binary"), b"\x7fELFfake").unwrap();

    let out = tempfile::tempdir().unwrap();
    let args = BuildArgs {
        config: out.path().join("nonexistent.yaml"),
        all: None,
        version: Some("1.0.0".into()),
        build_version: "1".into(),
        architectures: Some("amd64".into()),
        host: false,
        distributions: Some("trixie".into()),
        output: out.path().join("dist"),
        format: None,
        provider: None,
        no_verify: false,
        allow_unverified: false,
        lintian: false,
        lintian_fail_on_warnings: false,
        lintian_pedantic: false,
        lintian_suppress: None,
        dry_run: true,
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
        sign_method: Some("debsign".into()),
        local: false,
        from_dir: Some(payload.path().to_path_buf()),
        from_file: None,
        package_name: Some("myapp".into()),
        prefix: Some("usr/local/bin".into()), // relative — invalid
        overlay: None,
        update_lock: false,
        artifact_cache_dir: None,
        verify: false,
    };
    let err = run(args, None).expect_err("relative prefix should fail validation");
    assert!(
        err.to_string().contains("absolute path"),
        "expected prefix error, got: {err}"
    );
}

/// Regression: a real (non-dry-run) `--from-dir` build must package locally
/// and never fall through to the forge-source lookup. Before the fix, the
/// worker thread resolved `source_name` to an unknown forge plugin and
/// panicked with "unknown source plugin" for every `--from-dir`/`--from-file`
/// build (only the `--dry-run` tests covered the path, and they return
/// before the workers start).
#[test]
fn from_dir_builds_a_deb_without_a_forge_source() {
    let payload = tempfile::tempdir().unwrap();
    // A real ELF keeps the staging/dep-scan path honest; /bin/true is small
    // and present on every Linux CI image, with the test binary as a fallback.
    let dest = payload.path().join("mybinary");
    if std::fs::copy("/bin/true", &dest).is_err() {
        std::fs::copy(std::env::current_exe().unwrap(), &dest).unwrap();
    }

    let out = tempfile::tempdir().unwrap();
    let args = BuildArgs {
        config: out.path().join("nonexistent.yaml"),
        all: None,
        version: Some("1.0.0".into()),
        build_version: "1".into(),
        architectures: Some("amd64".into()),
        host: false,
        distributions: Some("trixie".into()),
        output: out.path().join("dist"),
        format: Some("deb".into()),
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
    };
    run(args, None).expect("--from-dir build should package locally");

    let built = std::fs::read_dir(out.path().join("dist"))
        .expect("output dir should exist")
        .filter_map(Result::ok)
        .any(|e| e.path().extension().is_some_and(|ext| ext == "deb"));
    assert!(built, "expected a .deb in the output directory");
}
