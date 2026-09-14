// SPDX-License-Identifier: GPL-3.0-or-later

//! `build_mode: source` — compile upstream source on the host and wrap the
//! install tree per suite.
//!
//! Bash-action parity (`src/lib/source-build.sh`), built natively: for
//! upstreams that publish no Linux binaries there is no release asset to
//! repack, so this path fetches the upstream source tag, configures +
//! compiles it with cmake on the **host**, installs to a `DESTDIR` stage,
//! computes runtime `Depends` from the staged ELFs (shlibdeps analogue via
//! `dpkg -S`), and wraps one `.deb` per suite with the shared deb plugin.
//!
//! Deliberate differences from the bash path:
//! - No containers/chroots: compile happens on the host, once, and every
//!   suite re-wraps the same tree. The host's glibc is therefore the
//!   effective symbol floor — build on the oldest suite you ship (or older).
//! - Native-arch only: every requested architecture must equal the host
//!   arch (the bash path gets this from one-native-runner-per-arch matrix
//!   cells; run lx once per native host here).
//! - `build_depends_suites` / `build_apt_sources` are accepted for config
//!   compat but not applied (container-only concepts); `build_depends`
//!   names host packages the caller/CI must have installed. With
//!   `--install-build-deps`, lx installs the missing ones (plus the build
//!   system's toolchain) via the host package manager first; without it,
//!   lx never installs packages on its own.

use anyhow::{bail, Context, Result};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use crate::build::BuildArgs;
use crate::builddeps;
use crate::config::PackageConfig;
use crate::plugins::build_system::{self, BuildSystem};

/// Run a source-mode build. `cfg` is the already-loaded config.
pub fn run(args: BuildArgs, cfg: &PackageConfig, token: Option<&str>) -> Result<()> {
    let build_start = Instant::now();
    let telemetry = lx_lib::telemetry::Telemetry::new(args.telemetry);
    telemetry.init()?;
    telemetry.record_stage("build_initialization")?;

    // Resolve the version: CLI wins, then config, then latest upstream tag.
    let version = resolve_version(args.clone(), cfg, token)?;

    // Requested architectures must all be the host arch (native only).
    let host = crate::build::host_arch()
        .ok_or_else(|| anyhow::anyhow!("could not detect host architecture from `uname -m`"))?;
    let requested = if let Some(a) = &args.architectures {
        a.split(',').map(|s| s.trim().to_string()).collect()
    } else {
        cfg.effective_architectures()
    };
    for a in &requested {
        if a != &host {
            bail!(
                "build_mode: source is native-only: requested '{a}' but host is '{host}' \
                 (run lx on a native {a} host, or pass --architectures {host})"
            );
        }
    }

    // Target format: `--format` overrides package.yaml, defaulting to deb.
    // Source mode now wraps in whatever format is selected.
    let format = crate::config::canonical_format(
        args.format
            .as_deref()
            .unwrap_or(&cfg.effective_package_format()),
    );
    // Resolve the source-capable packager once; source builds need a format
    // that can wrap a tree the build system already staged. Which formats
    // those are comes from the plugin roles themselves, not a central list.
    let packager = crate::plugins::get_source_packager(&format).ok_or_else(|| {
        let supported: Vec<&str> = crate::plugins::all_source_packagers()
            .iter()
            .map(|p| p.name())
            .collect();
        anyhow::anyhow!(
            "build_mode: source supports {} (got '{format}')",
            supported.join(", ")
        )
    })?;

    // Suites: configured distributions (or --distributions) minus skips,
    // with expired suites dropped — same pipeline as binary builds. Defaults
    // are per-format (Debian suites, RPM distros, or `arch`).
    let mut configured = cfg.effective_distributions_for(&format);
    if let Some(d) = &args.distributions {
        configured = d.split(',').map(|s| s.trim().to_string()).collect();
    }
    configured = lx_lib::config::filter_expired_distributions(&configured, None);
    let suites = cfg.source_suites(&configured);
    if suites.is_empty() {
        bail!("no suites to build (build_suites/distributions resolved empty)");
    }
    println!(
        "source build: {} {version} (format: {format}; suites: {})",
        cfg.package_name,
        suites.join(" ")
    );

    // Resolve the build system plugin: explicit `build_system:` wins, else
    // auto-detect from the source tree (after fetch), else default to cmake.
    let explicit_build_system = cfg.effective_build_system();
    let build_sys_name = if explicit_build_system == "custom" {
        Some("custom")
    } else if !cfg.build_system.trim().is_empty() {
        Some(explicit_build_system.as_str())
    } else {
        None // auto-detect after fetch
    };

    // Toolchain + build-dep sanity before any download. For an explicitly
    // configured build system, ensure (or report) its tools now; an
    // auto-detected build system is handled after resolution (below).
    if let Some(name) = build_sys_name {
        if let Some(bs) = build_system::get_build_system(name) {
            ensure_tool_deps(&args, &bs.required_tools())?;
        }
    }
    // Explicit host build dependencies (host-distro package names).
    builddeps::ensure_build_deps(&cfg.build_depends, args.install_build_deps, args.dry_run)?;

    if args.dry_run {
        println!(
            "dry run: would fetch {version} and compile for suites: {}",
            suites.join(" ")
        );
        return Ok(());
    }

    // Fetch + extract the source tag.
    let workdir = tempfile::tempdir().context("failed to create source build workdir")?;
    let (src_dir, src_url, src_sha) = fetch_source(
        cfg,
        &version,
        token,
        args.cache_dir.as_deref(),
        workdir.path(),
    )?;

    // Resolve the build system plugin (auto-detect if not explicit).
    let build_sys: Box<dyn BuildSystem> = if let Some(name) = build_sys_name {
        build_system::get_build_system(name).ok_or_else(|| {
            anyhow::anyhow!(
                "unsupported build_system '{}' (expected one of: {})",
                name,
                build_system::build_system_names().join(", ")
            )
        })?
    } else {
        match build_system::detect_build_system(&src_dir) {
            Some(bs) => {
                println!(
                    "build system: auto-detected {} ({})",
                    bs.name(),
                    bs.description()
                );
                bs
            }
            None => bail!(
                "could not auto-detect build system for {} (set build_system: in package.yaml: cmake, cargo, go, or custom)",
                cfg.github_repo
            ),
        }
    };

    // Build-system-specific tool checks: install/report the tool packages
    // first (for an auto-detected build system), then verify each tool is
    // actually on PATH.
    ensure_tool_deps(&args, &build_sys.required_tools())?;
    for tool in build_sys.required_tools() {
        require_tool(tool)?;
    }

    // Compile once on the host; every suite re-wraps the same tree.
    // prebuild steps run for every build system (patches, codegen).
    run_steps(&cfg.prebuild_steps, &src_dir, &[])?;
    let stage = build_sys.build(cfg, &src_dir, workdir.path())?;

    // shlibdeps analogue: ELF DT_NEEDED -> owning host packages.
    let depends = compute_depends(&stage, cfg, &format);
    println!("depends: {depends}");
    let mut wrap_cfg = cfg.clone();
    wrap_cfg.depends = depends.clone();

    // Wrap one package per suite using the selected format's plugin.
    std::fs::create_dir_all(&args.output)?;
    let debian_version = lx_lib::pkgmeta::strip_upstream_prefix(&version);
    let mtime = match cfg.effective_mtime()? {
        Some(m) => m,
        None => lx_lib::pkgmeta::reproducible_epoch(None),
    };
    let sign_key = cfg.effective_sign_key(args.sign_key.as_deref());
    let sign_key_id = cfg.effective_sign_key_id(args.sign_key_id.as_deref());
    let sign_method = cfg.effective_sign_method(args.sign_method.as_deref());
    let sign_passphrase = crate::build::resolve_sign_passphrase();
    let mut built: Vec<PathBuf> = Vec::new();
    for dist in &suites {
        let staging = tempfile::tempdir().context("failed to create staging dir")?;
        crate::plugins::copy_dir_recursive(&stage, staging.path())?;
        let job = crate::build::ResolvedJob {
            dist: dist.clone(),
            arch: host.clone(),
            asset: lx_lib::github::Asset {
                name: String::new(),
                browser_download_url: String::new(),
                size: None,
                checksums: Default::default(),
            },
            tag: version.clone(),
            published_at: None,
        };
        let ctx = crate::plugins::BuildContext {
            cfg: &wrap_cfg,
            job: &job,
            binary_dir: staging.path(),
            staging_root: staging.path(),
            license: None,
            debian_version: &debian_version,
            build_version: &args.build_version,
            mtime,
            sign_key: sign_key.as_deref(),
            sign_key_id: &sign_key_id,
            sign_passphrase: sign_passphrase.as_deref(),
            sign_method: &sign_method,
            detected_deps: Vec::new(),
        };
        let pkg_tmp = packager
            .archive_staged_tree(&ctx)
            .with_context(|| format!("wrapping {dist}/{host} ({format})"))?;
        let dest = args.output.join(
            pkg_tmp
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("built package has no filename"))?,
        );
        std::fs::copy(&pkg_tmp, &dest)?;
        // Post-build signing goes through the `Signer` plugin for
        // `(format, method)`: embedded backends (`rpm-pgp`, `deb-debsign`)
        // already signed while the tree was archived, detached backends
        // (`gpg-detach`) write a sibling signature now.
        if let Some(key) = &sign_key {
            let sign_type = cfg.effective_sign_type();
            let sign_ctx = crate::plugins::signer::SignContext {
                key_file: key,
                key_id: &sign_key_id,
                passphrase: sign_passphrase.as_deref(),
                sign_type: &sign_type,
                cert_file: &cfg.signature.cert_file,
            };
            if let crate::plugins::signer::PostBuild::Detached { path, .. } =
                crate::plugins::signer::apply_post_build(&format, &sign_method, &dest, &sign_ctx)?
            {
                println!("  ✓ signed {} -> {}", dest.display(), path.display());
            }
        }
        println!("  ✓ built {} ({dist})", dest.display());
        built.push(dest);
    }

    telemetry.record_stage_complete("build_completion", "success")?;
    telemetry.finalize(build_start.elapsed().as_secs())?;

    if args.sbom {
        let artifacts = lx_lib::sbom::collect_artifacts(&args.output)?;
        let materials = vec![lx_lib::sbom::Material {
            uri: src_url,
            digest: if src_sha.is_empty() {
                None
            } else {
                Some(src_sha)
            },
        }];
        lx_lib::sbom::emit(
            &args.output,
            &cfg.package_name,
            &version,
            &args.build_version,
            &artifacts,
            &materials,
        )?;
    }
    if args.save_baseline {
        telemetry.save_as_baseline()?;
    }

    if args.summary {
        crate::summary::write(
            &args.output,
            built.len(),
            &crate::summary::SummaryInputs {
                package: cfg.package_name.clone(),
                version: version.clone(),
                build_version: args.build_version.clone(),
                github_repo: cfg.github_repo.clone(),
                architectures: vec![host.clone()],
                distributions: suites.clone(),
                max_parallel: args.max_parallel,
                start: build_start,
                telemetry: telemetry.summary_json(),
                provenance: vec![],
                package_format: format.clone(),
                source: "source".to_string(),
            },
        )?;
    }

    // Emit source packages from the built binaries, best-effort: Debian
    // `.dsc` + tarballs, an RPM `.src.rpm`, or an Arch `PKGBUILD`.
    let rel = wrap_cfg.effective_relations(&format);
    let pkg = crate::source::Pkg {
        name: cfg.package_name.clone(),
        github_repo: cfg.github_repo.clone(),
        description: cfg.effective_description(),
        maintainer: cfg.effective_maintainer(),
        version: version.clone(),
        build_version: args.build_version.clone(),
        epoch: cfg.epoch.clone(),
        license_spdx: cfg.license_spdx.clone(),
        depends,
        recommends: rel.recommends,
        suggests: rel.suggests,
        conflicts: rel.conflicts,
        replaces: rel.replaces,
        provides: rel.provides,
        breaks: rel.breaks,
        predepends: rel.predepends,
        section: cfg.effective_section(),
        priority: cfg.effective_priority(),
        fields: cfg.fields.clone(),
        published_at: None,
        license: None,
    };
    let source_result = packager.generate_source_package(&args.output, &pkg);
    if let Err(e) = source_result {
        eprintln!("⚠ source package generation failed (binaries stand on their own): {e:#}");
    }

    println!("\n✓ built {} package(s) from source", built.len());
    Ok(())
}

/// CLI version > config version > latest upstream tag.
fn resolve_version(args: BuildArgs, cfg: &PackageConfig, token: Option<&str>) -> Result<String> {
    if let Some(v) = args.version.filter(|v| !v.trim().is_empty()) {
        return Ok(v);
    }
    if !cfg.version.trim().is_empty() {
        return Ok(cfg.version.clone());
    }
    let source_name = cfg.effective_forge_source();
    let source = crate::plugins::forge::get_forge_source(&source_name).ok_or_else(|| {
        anyhow::anyhow!("unsupported source '{source_name}' for version detection")
    })?;
    crate::plugins::forge::apply_forge_host(source.as_ref(), cfg);
    let tok = crate::plugins::forge::resolve_forge_token(source.as_ref(), token);
    let latest = source.latest_release(
        &cfg.github_repo,
        tok.as_deref(),
        args.api_cache_dir.as_deref(),
    )?;
    println!("version: using latest upstream tag '{}'", latest.tag_name);
    Ok(latest.tag_name)
}

pub(crate) fn require_tool(name: &str) -> Result<()> {
    if Command::new(name).arg("--version").output().is_ok() {
        Ok(())
    } else {
        bail!("build_mode: source requires '{name}' on PATH (host compile, no containers)")
    }
}

/// Ensure (or report) the host packages providing a build system's tools.
/// No-op when no recognised package manager is on `PATH` — the plain
/// [`require_tool`] check below still catches a missing binary.
fn ensure_tool_deps(args: &BuildArgs, tools: &[&str]) -> Result<()> {
    let Some(pm) = builddeps::HostPm::detect() else {
        return Ok(());
    };
    let deps = builddeps::resolve(&[], tools, pm);
    builddeps::ensure_build_deps(&deps, args.install_build_deps, args.dry_run)
}

/// Download the source tag tarball, extract it, return the source dir.
fn fetch_source(
    cfg: &PackageConfig,
    version: &str,
    token: Option<&str>,
    cache_dir: Option<&Path>,
    workdir: &Path,
) -> Result<(PathBuf, String, String)> {
    let upstream_ref = if cfg.upstream_ref.trim().is_empty() {
        version
    } else {
        cfg.upstream_ref.trim()
    };
    let url = if cfg.upstream_url.trim().is_empty() {
        format!(
            "https://github.com/{}/archive/refs/tags/{upstream_ref}.tar.gz",
            cfg.github_repo
        )
    } else {
        format!(
            "{}/archive/{upstream_ref}.tar.gz",
            cfg.upstream_url.trim_end_matches('/')
        )
    };
    println!("downloading source: {url}");
    let tarball = workdir.join("source.tar.gz");
    if let Some(dir) = cache_dir {
        let cache = lx_lib::cache::DownloadCache::new(dir.to_path_buf())?;
        cache.fetch(&url, &tarball, None, &|u, out| download_to(u, out, token))?;
    } else {
        download_to(&url, &tarball, token)?;
    }
    let src_root = workdir.join("src");
    std::fs::create_dir_all(&src_root)?;
    extract_tar_gz(&tarball, &src_root)?;
    let sha = lx_lib::checksum::sha256_file(&tarball).unwrap_or_default();
    // GitHub-style tarballs nest one top-level dir; Forgejo too. Use it
    // when it's the only entry, else the root itself.
    let entries: Vec<PathBuf> = std::fs::read_dir(&src_root)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    if entries.len() == 1 && entries[0].is_dir() {
        Ok((entries.into_iter().next().unwrap(), url, sha))
    } else {
        Ok((src_root, url, sha))
    }
}

fn download_to(url: &str, out: &Path, token: Option<&str>) -> Result<()> {
    let client = lx_lib::http::new_client()?;
    let mut req = client.get(url);
    if let Some(t) = token.filter(|t| !t.is_empty()) {
        req = req.bearer_auth(t);
    }
    let mut resp = req.send().context("failed to download source tarball")?;
    if !resp.status().is_success() {
        bail!("failed to download source tarball: HTTP {}", resp.status());
    }
    let mut f = std::fs::File::create(out)?;
    std::io::copy(&mut resp, &mut f)?;
    Ok(())
}

fn extract_tar_gz(tarball: &Path, dest: &Path) -> Result<()> {
    let f = std::fs::File::open(tarball)?;
    let gz = flate2::read::GzDecoder::new(f);
    let mut ar = tar::Archive::new(gz);
    ar.unpack(dest)
        .context("failed to extract source tarball")?;
    Ok(())
}

/// Run ordered shell steps (`sh -c`) in `dir` with extra env.
fn run_steps(steps: &[String], dir: &Path, env: &[(&str, &str)]) -> Result<()> {
    for step in steps {
        println!("  $ {step}");
        let mut cmd = Command::new("sh");
        cmd.args(["-c", step]);
        cmd.current_dir(dir);
        for (k, v) in env {
            cmd.env(k, v);
        }
        let st = cmd
            .status()
            .with_context(|| format!("failed to run step: {step}"))?;
        if !st.success() {
            bail!("prebuild step failed: {step}");
        }
    }
    Ok(())
}

/// shlibdeps analogue: staged ELFs' DT_NEEDED sonames resolved to owning
/// host packages via the host's package manager (dpkg, rpm, or pacman).
/// Essential/libc sonames are skipped (they come with every base install);
/// empty result falls back to the config's `depends:` and finally `libc6`
/// (bash falls back to `libc6` too).
fn compute_depends(stage: &Path, cfg: &PackageConfig, format: &str) -> String {
    let mut pkgs = BTreeSet::new();
    let elfs = lx_lib::scandeps::find_elf_files(stage).unwrap_or_default();
    for elf in &elfs {
        let bytes = match std::fs::read(elf) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let needed = lx_lib::elfdeps::needed_libraries(&bytes).unwrap_or_default();
        for soname in needed {
            if lx_lib::elfdeps::is_essential_libc_soname(&soname) {
                continue;
            }
            if let Some(pkg) = lx_lib::scandeps::pkg_owner(&soname) {
                pkgs.insert(pkg.to_string());
            }
        }
    }
    if pkgs.is_empty() {
        if cfg.musl {
            // Musl-static binary: no glibc dependency at all.
            return String::new();
        }
        if !cfg.depends.trim().is_empty() {
            return cfg.depends.trim().to_string();
        }
        // Format-appropriate C-runtime fallback (matches the host-package
        // naming each format expects).
        return if format == "deb" {
            "libc6".to_string()
        } else {
            "glibc".to_string()
        };
    }
    pkgs.into_iter().collect::<Vec<_>>().join(", ")
}
