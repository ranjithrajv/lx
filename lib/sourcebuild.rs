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
//!   names host packages the caller/CI must have installed — lx never
//!   apt-gets on its own.

use anyhow::{bail, Context, Result};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use crate::build::BuildArgs;
use crate::config::PackageConfig;

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

    // Suites: configured distributions (or --distributions) minus skips,
    // with expired suites dropped — same pipeline as binary builds.
    let mut configured =
        if !cfg.debian_distributions.is_empty() || !cfg.ubuntu_distributions.is_empty() {
            let mut out = cfg.debian_distributions.clone();
            out.extend(cfg.ubuntu_distributions.clone());
            out
        } else {
            lx_lib::constants::DEFAULT_DEBIAN_DISTRIBUTIONS
                .iter()
                .map(|s| s.to_string())
                .collect()
        };
    if let Some(d) = &args.distributions {
        configured = d.split(',').map(|s| s.trim().to_string()).collect();
    }
    configured = lx_lib::config::filter_expired_distributions(&configured, None);
    let suites = cfg.source_suites(&configured);
    if suites.is_empty() {
        bail!("no suites to build (build_suites/distributions resolved empty)");
    }
    println!(
        "source build: {} {version} (suites: {})",
        cfg.package_name,
        suites.join(" ")
    );

    // Toolchain + build-dep sanity before any download.
    require_tool("cmake")?;
    require_tool("ninja")?;
    if !cfg.build_depends.is_empty() {
        let missing = missing_host_packages(&cfg.build_depends);
        if !missing.is_empty() {
            bail!(
                "missing host build dependencies: {} (install them, e.g. apt-get install -y {})",
                missing.join(" "),
                missing.join(" ")
            );
        }
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

    // Compile once on the host; every suite re-wraps the same tree.
    let sandbox = Sandbox::new(args.sandbox);
    let stage = if cfg.effective_build_system() == "custom" {
        build_custom(cfg, &src_dir, workdir.path(), &sandbox)
    } else {
        // prebuild steps run for cmake builds too (patches, codegen).
        run_steps(&cfg.prebuild_steps, &src_dir, &[])?;
        compile_cmake(cfg, &src_dir, workdir.path(), &sandbox)
    }?;

    // shlibdeps analogue: ELF DT_NEEDED -> owning host packages.
    let depends = compute_depends(&stage, cfg);
    println!("depends: {depends}");
    let mut wrap_cfg = cfg.clone();
    wrap_cfg.depends = depends.clone();

    // Wrap one .deb per suite.
    std::fs::create_dir_all(&args.output)?;
    let debian_version = lx_lib::pkgmeta::strip_upstream_prefix(&version);
    let mtime = lx_lib::pkgmeta::reproducible_epoch(None);
    let sign_key = cfg.effective_sign_key(args.sign_key.as_deref());
    let sign_key_id = cfg.effective_sign_key_id(args.sign_key_id.as_deref());
    let sign_method = cfg.effective_sign_method(args.sign_method.as_deref());
    let mut built: Vec<PathBuf> = Vec::new();
    for dist in &suites {
        let staging = tempfile::tempdir().context("failed to create staging dir")?;
        copy_tree(&stage, staging.path())?;
        let job = crate::build::ResolvedJob {
            dist: dist.clone(),
            arch: host.clone(),
            asset: lx_lib::github::Asset {
                name: String::new(),
                browser_download_url: String::new(),
                size: None,
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
            sign_passphrase: None,
            sign_method: &sign_method,
        };
        let deb_tmp = crate::plugins::deb::archive_staged_tree(&ctx)
            .with_context(|| format!("wrapping {dist}/{host}"))?;
        let dest = args.output.join(
            deb_tmp
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("built .deb has no filename"))?,
        );
        std::fs::copy(&deb_tmp, &dest)?;
        if sign_method == "detach" {
            if let Some(key) = &sign_key {
                let req = lx_lib::sign::SignRequest {
                    key_file: key,
                    key_id: &sign_key_id,
                    passphrase: None,
                };
                lx_lib::sign::gpg_detach_sign(&dest, &req)?;
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
                package_format: "deb".to_string(),
                source: "source".to_string(),
            },
        )?;
    }

    // Emit .dsc source packages from the built .debs (bash parity:
    // run_source_build ends with build_source_packages), best-effort.
    let rel = wrap_cfg.effective_relations("deb");
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
    if let Err(e) = crate::source::generate(&args.output, &pkg) {
        eprintln!("⚠ source package generation failed (debs stand on their own): {e:#}");
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
    let source_name = cfg.effective_source();
    let source = crate::plugins::source::get_source_plugin(&source_name).ok_or_else(|| {
        anyhow::anyhow!("unsupported source '{source_name}' for version detection")
    })?;
    crate::plugins::source::apply_source_host(source.as_ref(), cfg);
    let tok = crate::plugins::source::resolve_source_token(source.as_ref(), token);
    let latest = source.latest_release(
        &cfg.github_repo,
        tok.as_deref(),
        args.api_cache_dir.as_deref(),
    )?;
    println!("version: using latest upstream tag '{}'", latest.tag_name);
    Ok(latest.tag_name)
}

fn require_tool(name: &str) -> Result<()> {
    if Command::new(name).arg("--version").output().is_ok() {
        Ok(())
    } else {
        bail!("build_mode: source requires '{name}' on PATH (host compile, no containers)")
    }
}

/// Host packages checked via `dpkg -s`; returns the missing subset.
/// Non-dpkg hosts treat every entry as missing (explicit error beats a
/// half-configured compile).
fn missing_host_packages(deps: &[String]) -> Vec<String> {
    deps.iter()
        .filter(|d| {
            Command::new("dpkg")
                .args(["-s", d])
                .output()
                .map(|o| !o.status.success())
                .unwrap_or(true)
        })
        .cloned()
        .collect()
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

/// Opt-in hermetic-ish host compile (item 8): when `--sandbox` is passed,
/// build steps run under `unshare --map-root-user --mount -n` (no network,
/// private mount ns) if the kernel permits it, falling back to a direct
/// run with a warning. Network-dependent builds (cargo/go fetching deps)
/// should skip it; vendored/offline builds get isolation for free.
pub struct Sandbox {
    pub enabled: bool,
}

impl Sandbox {
    pub fn new(enabled: bool) -> Self {
        Self { enabled }
    }

    /// Run `prog args...` in `dir` with extra env, sandboxed when enabled.
    pub fn run(&self, prog: &str, args: &[String], dir: &Path, env: &[(&str, &str)]) -> Result<()> {
        if self.enabled && unshare_works() {
            let mut full = vec![prog.to_string()];
            full.extend(args.iter().cloned());
            let mut cmd = Command::new("unshare");
            cmd.args(["--map-root-user", "--mount", "-n", "--", "env", "-i"]);
            for (k, v) in env {
                cmd.arg(format!("{k}={v}"));
            }
            // Minimal PATH inside the empty env; callers pass absolute prog
            // paths or rely on /usr/bin:/bin.
            cmd.arg("PATH=/usr/bin:/bin");
            cmd.args(full);
            cmd.current_dir(dir);
            let st = cmd.status().context("failed to run sandboxed build step")?;
            if !st.success() {
                bail!("sandboxed build step failed: {prog}");
            }
            return Ok(());
        }
        if self.enabled {
            eprintln!("⚠ --sandbox requested but unshare is unavailable; running unsandboxed");
        }
        let mut cmd = Command::new(prog);
        cmd.args(args);
        cmd.current_dir(dir);
        for (k, v) in env {
            cmd.env(k, v);
        }
        let st = cmd.status().context("failed to run build step")?;
        if !st.success() {
            bail!("build step failed: {prog}");
        }
        Ok(())
    }
}

fn unshare_works() -> bool {
    Command::new("unshare")
        .args(["--map-root-user", "--mount", "-n", "--", "true"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
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

/// `build_system: custom`: build_commands then install_commands (with
/// $DESTDIR) instead of cmake. Returns the staging tree.
fn build_custom(
    cfg: &PackageConfig,
    src_dir: &Path,
    workdir: &Path,
    sandbox: &Sandbox,
) -> Result<PathBuf> {
    let stage = workdir.join("stage");
    std::fs::create_dir_all(&stage)?;
    let destdir = stage.to_string_lossy().to_string();
    run_steps(&cfg.prebuild_steps, src_dir, &[])?;
    println!(
        "running {} custom build command(s)",
        cfg.build_commands.len()
    );
    for cmd in &cfg.build_commands {
        println!("  $ {cmd}");
        sandbox.run(
            "sh",
            &["-c".to_string(), cmd.clone()],
            src_dir,
            &[("DESTDIR", destdir.as_str())],
        )?;
    }
    println!(
        "running {} custom install command(s)",
        cfg.install_commands.len()
    );
    for cmd in &cfg.install_commands {
        println!("  $ {cmd}");
        sandbox.run(
            "sh",
            &["-c".to_string(), cmd.clone()],
            src_dir,
            &[("DESTDIR", destdir.as_str())],
        )?;
    }
    Ok(stage)
}

/// cmake configure + build + DESTDIR install. Returns the install tree.
fn compile_cmake(
    cfg: &PackageConfig,
    src_dir: &Path,
    workdir: &Path,
    sandbox: &Sandbox,
) -> Result<PathBuf> {
    let build_dir = workdir.join("build");
    let stage = workdir.join("stage");
    std::fs::create_dir_all(&build_dir)?;
    std::fs::create_dir_all(&stage)?;

    let mut cmd = Command::new("cmake");
    cmd.args([
        "-S",
        &src_dir.to_string_lossy(),
        "-B",
        &build_dir.to_string_lossy(),
        "-G",
        "Ninja",
    ]);
    cmd.arg("-DCMAKE_INSTALL_PREFIX=/usr");
    cmd.arg("-DCMAKE_BUILD_TYPE=Release");
    for f in &cfg.cmake_flags {
        cmd.arg(f);
    }
    println!("configuring: cmake {}", cfg.cmake_flags.join(" "));
    let st = cmd.status().context("failed to run cmake configure")?;
    if !st.success() {
        bail!("cmake configure failed");
    }
    sandbox
        .run(
            "cmake",
            &[
                "--build".to_string(),
                build_dir.to_string_lossy().to_string(),
            ],
            workdir,
            &[],
        )
        .context("cmake build failed")?;
    sandbox
        .run(
            "cmake",
            &[
                "--install".to_string(),
                build_dir.to_string_lossy().to_string(),
            ],
            workdir,
            &[("DESTDIR", &stage.to_string_lossy())],
        )
        .context("cmake install failed")?;
    Ok(stage)
}

/// shlibdeps analogue: staged ELFs' DT_NEEDED sonames resolved to owning
/// host packages via `dpkg -S`. Essential/libc sonames are skipped (they
/// come with every base install); empty result falls back to the config's
/// `depends:` and finally `libc6` (bash falls back to `libc6` too).
fn compute_depends(stage: &Path, cfg: &PackageConfig) -> String {
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
            if let Some(pkg) = lx_lib::scandeps::dpkg_owner(&soname) {
                // Strip any :arch qualifier dpkg -S may report.
                pkgs.insert(pkg.split(':').next().unwrap_or(&pkg).to_string());
            }
        }
    }
    if pkgs.is_empty() {
        if !cfg.depends.trim().is_empty() {
            return cfg.depends.trim().to_string();
        }
        return "libc6".to_string();
    }
    pkgs.into_iter().collect::<Vec<_>>().join(", ")
}

fn copy_tree(src: &Path, dst: &Path) -> Result<()> {
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let dest = dst.join(entry.file_name());
        let ft = entry.file_type()?;
        if ft.is_dir() {
            crate::plugins::copy_dir_recursive(&entry.path(), &dest)?;
        } else if ft.is_symlink() {
            let target = std::fs::read_link(entry.path())?;
            #[cfg(unix)]
            std::os::unix::fs::symlink(&target, &dest)?;
        } else {
            std::fs::copy(entry.path(), &dest)?;
        }
    }
    Ok(())
}
