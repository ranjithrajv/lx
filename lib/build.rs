// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{anyhow, bail, Context, Result};
use clap::Args;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::PackageConfig;
use crate::discovery::guess_format;
use lx_lib::github::Asset;

#[derive(Debug, Clone, Args)]
pub struct BuildArgs {
    /// Path to package.yaml
    #[arg(default_value = lx_lib::constants::DEFAULT_CONFIG_FILENAME)]
    pub config: PathBuf,

    /// Build every package.yaml listed in a fleet manifest instead of a
    /// single config (YAML: `packages: [path, ...]`, paths relative to the
    /// manifest's own directory). Every other flag applies to each build.
    #[arg(long, value_name = "PACKAGES_YAML", conflicts_with = "config")]
    pub all: Option<PathBuf>,

    /// Version of the software to build (overrides any version in config).
    #[arg(short = 'v', long)]
    pub version: Option<String>,

    /// Debian build version/revision (defaults to "1").
    #[arg(long, default_value = "1")]
    pub build_version: String,

    /// Restrict to specific architectures (comma-separated).
    #[arg(long)]
    pub architectures: Option<String>,

    /// Build only for this machine's own architecture (auto-detected via
    /// `uname -m`), host-native. Conflicts with
    /// --architectures; pass one or the other.
    #[arg(long)]
    pub host: bool,

    /// Restrict to specific distributions (comma-separated).
    #[arg(long)]
    pub distributions: Option<String>,

    /// Directory to write resulting package files into.
    #[arg(long, default_value = "dist")]
    pub output: PathBuf,

    /// Package format plugin to use: deb, rpm, arch, apk, or ipk. `all`
    /// builds every registered format; a comma-separated list builds each.
    /// Overrides package.yaml's `package_format`. Defaults to the config
    /// file's value (or "deb").
    #[arg(long)]
    pub format: Option<String>,

    /// Source provider plugin for auto-discovery (github or gitlab).
    /// Overrides `package.yaml`'s `source`. Defaults to the config's value
    /// (or "github").
    #[arg(long, value_name = "PROVIDER")]
    pub provider: Option<String>,

    /// Skip checksum verification (not recommended).
    #[arg(long)]
    pub no_verify: bool,

    /// Proceed when an asset has no pinned or sidecar checksum to verify
    /// against, instead of failing the build. Most GitHub releases don't
    /// publish a checksum sidecar, so without either this or
    /// --pinned-metadata, the default is to refuse to build from an
    /// unverified download rather than silently warn and continue.
    #[arg(long)]
    pub allow_unverified: bool,

    /// Run lintian on each built .deb (fails on errors by default).
    #[arg(long)]
    pub lintian: bool,

    /// Fail the build on lintian warnings too (implies --lintian).
    #[arg(long)]
    pub lintian_fail_on_warnings: bool,

    /// Enable pedantic lintian checks (implies --lintian).
    #[arg(long)]
    pub lintian_pedantic: bool,

    /// Comma-separated lintian tags to suppress (implies --lintian).
    #[arg(long)]
    pub lintian_suppress: Option<String>,

    /// Print what would be built and exit.
    #[arg(long)]
    pub dry_run: bool,

    /// Maximum concurrent architecture builds. Default (0) auto-tunes from
    /// system resources, mirroring the action's ci-optimization.sh
    /// (resource-based dynamic parallelism). Distributions for an
    /// architecture are always built sequentially within its worker.
    #[arg(long, default_value_t = 0)]
    pub max_parallel: usize,

    /// Path to release-metadata.json (the action's vet-time provenance pin).
    /// When present, every downloaded asset is verified against the pinned
    /// SHA-256 for that asset + version instead of the live sidecar file.
    #[arg(long)]
    pub pinned_metadata: Option<PathBuf>,

    /// Persistent download cache directory (default: disabled). Reuses
    /// downloads across runs for 24h, keyed by url + expected checksum.
    #[arg(long)]
    pub cache_dir: Option<PathBuf>,

    /// JSON API cache directory (mirrors the action's /tmp/github_api_cache).
    /// GitHub release/license responses are cached for 5 minutes to avoid
    /// rate limits on repeated runs. Best-effort.
    #[arg(long)]
    pub api_cache_dir: Option<PathBuf>,

    /// Also generate Debian source packages (3.0 quilt: .dsc +
    /// .debian.tar.xz + .orig.tar.xz) for each distribution built,
    /// mirroring the action's build_source_packages. Built natively
    /// in-process -- no dpkg-source subprocess.
    #[arg(long)]
    pub source: bool,

    /// Write a build-summary.json into the output directory with the built
    /// packages, sizes, timing, and success rate (action's
    /// generate_build_summary parity).
    #[arg(long)]
    pub summary: bool,

    /// Enable minimal telemetry (action's TELEMETRY_ENABLED): writes
    /// .telemetry/metrics.json + stages.log/failures.log and populates the
    /// `telemetry` object in build-summary.json.
    #[arg(long)]
    pub telemetry: bool,

    /// Save the current build metrics as performance baseline
    /// (.telemetry/baseline.json) for regression detection on future
    /// builds (action's save-baseline / save_as_baseline parity).
    #[arg(long)]
    pub save_baseline: bool,

    /// Show a live inline progress bar while building (action's
    /// progress.sh). Only rendered on an interactive terminal.
    #[arg(long)]
    pub progress: bool,

    /// Path for the build progress JSON (default: /tmp/build_progress.json).
    #[arg(long)]
    pub progress_path: Option<PathBuf>,

    /// Keep intermediate files (downloaded assets, staging dirs).
    #[arg(long)]
    pub keep: bool,

    /// Sign built packages: path to an ASCII-armored secret key. Overrides
    /// package.yaml's `signature.key_file`. For deb with method `detach`
    /// (default), produces a detached `<pkg>.deb.sig` via gpg; with method
    /// `debsign`, embeds `_gpgorigin` inside the `.deb`. For rpm, embeds
    /// the PGP signature natively (`rpm -K` verifiable). Passphrase comes
    /// from $LX_SIGN_PASSPHRASE, falling back to $NFPM_PASSPHRASE.
    #[arg(long, value_name = "KEY_FILE")]
    pub sign_key: Option<PathBuf>,

    /// Signing key id / fingerprint (gpg --local-user for deb; ignored by
    /// rpm). Overrides package.yaml's `signature.key_id`.
    #[arg(long, value_name = "KEY_ID")]
    pub sign_key_id: Option<String>,

    /// Deb signing method: `detach` (sibling `.sig`, default) or `debsign`
    /// (embedded `_gpgorigin`). Overrides package.yaml's
    /// `signature.method`. Ignored for rpm/arch.
    #[arg(long, value_name = "METHOD")]
    pub sign_method: Option<String>,

    /// Build from a local payload (skip upstream download). Uses
    /// `local_payload` from package.yaml (archive or directory). Path
    /// existence is checked at build time.
    #[arg(long)]
    pub local: bool,

    /// "You supply files" mode (fpm-style): build a package from a directory
    /// of files you supply, with no forge release fetch. Requires
    /// `--package-name` and `--version` if not given in package.yaml. The
    /// directory's files are staged into the package (ELF binaries →
    /// /usr/bin, or `--prefix`). Conflicts with `--local`.
    #[arg(long, value_name = "PATH", conflicts_with = "local")]
    pub from_dir: Option<PathBuf>,

    /// Like `--from-dir` but for a single file. The file is installed to
    /// /usr/bin (or `--prefix`). Conflicts with `--from-dir`.
    #[arg(long, value_name = "PATH", conflicts_with = "from_dir")]
    pub from_file: Option<PathBuf>,

    /// Package name for `--from-dir`/`--from-file` builds (overrides
    /// package.yaml). Required if no package.yaml is present.
    #[arg(long, value_name = "NAME")]
    pub package_name: Option<String>,

    /// Install prefix inside the package for `--from-dir`/`--from-file`
    /// (e.g. "/usr/local/bin", "/opt/myapp"). Files are staged under this
    /// absolute path instead of the default /usr/bin. Must start with '/'.
    #[arg(long, value_name = "PATH")]
    pub prefix: Option<String>,

    /// Apply a delta package.yaml over the base config before building: its
    /// top-level keys replace the base's. Lets an org share one base
    /// package.yaml and fork only what differs per target instead of
    /// duplicating the whole template.
    #[arg(long)]
    pub overlay: Option<PathBuf>,

    /// Write/refresh `package.lock` next to the config after a successful
    /// build, pinning each architecture's resolved tag/asset/checksum. When
    /// a `package.lock` already exists and this flag is absent, every
    /// resolved asset is instead verified against it and the build fails on
    /// drift (a different tag or asset than what's pinned).
    #[arg(long)]
    pub update_lock: bool,

    /// Cache built package artifacts, keyed by config + build flags + asset
    /// checksum + format + distribution + architecture. An identical
    /// rebuild copies the cached file instead of re-running the packaging
    /// pipeline. Skipped for jobs that produce a detached signature (the
    /// cache doesn't track the sibling `.sig` file).
    #[arg(long)]
    pub artifact_cache_dir: Option<PathBuf>,

    /// build_mode: source only — run compile steps under `unshare -n`
    /// (no network, private mounts) when the kernel permits, else run
    /// unsandboxed with a warning. Binary repacks never execute anything
    /// and ignore this flag.
    #[arg(long)]
    pub sandbox: bool,

    /// build_mode: source only — install the missing host build
    /// dependencies (`build_depends:` plus the build system's toolchain)
    /// with the host package manager before compiling, instead of only
    /// reporting them. Uses `sudo` unless already root; combine with
    /// `--dry-run` to print the install command without running it.
    #[arg(long)]
    pub install_build_deps: bool,

    /// Emit supply-chain attestations into the output dir:
    /// `<pkg>_<ver>.spdx.json` (SPDX 2.3 SBOM) and `<pkg>_<ver>.slsa.json`
    /// (SLSA v1-style provenance over built artifacts + upstream materials).
    #[arg(long)]
    pub sbom: bool,

    /// Reproducibility check (nix build --check parity): force a real
    /// rebuild even when the artifact cache already has this recipe's key,
    /// then byte-compare the fresh output against the cached one and fail
    /// if they differ. Requires --artifact-cache-dir. The first build for
    /// a given recipe has nothing to compare against yet and just
    /// populates the cache as a baseline.
    #[arg(long, requires = "artifact_cache_dir")]
    pub verify: bool,

    /// Sign built packages with cosign (Sigstore keyless signing).
    /// Requires cosign on PATH and an OIDC token (e.g., in GitHub Actions).
    #[arg(long)]
    pub cosign: bool,

    /// Cross-compile target architecture (e.g., "arm64", "riscv64").
    /// Builds for this architecture even on a different host. Uses musl
    /// static linking when the build system supports it.
    #[arg(long, value_name = "ARCH")]
    pub cross_target: Option<String>,

    /// Detect binary dependencies via ELF analysis + distro package lookup.
    /// Maps shared-library links (DT_NEEDED) to system packages. Default: true.
    #[arg(long, default_value_t = true)]
    pub bindep: bool,
}

#[derive(Clone, Debug)]
pub struct ResolvedJob {
    pub dist: String,
    pub arch: String,
    pub asset: Asset,
    pub tag: String,
    /// The release's own publish time (Unix epoch seconds), used instead of
    /// wall-clock build time for reproducible package metadata.
    pub published_at: Option<i64>,
}

/// Fleet manifest for `lx build --all`: a flat list of package.yaml paths,
/// relative to the manifest's own directory.
#[derive(Debug, serde::Deserialize)]
struct FleetManifest {
    packages: Vec<PathBuf>,
}

fn run_all(fleet_path: &Path, args: &BuildArgs, token: Option<&str>) -> Result<()> {
    let text = std::fs::read_to_string(fleet_path)
        .with_context(|| format!("failed to read fleet manifest '{}'", fleet_path.display()))?;
    let fleet: FleetManifest = serde_yaml::from_str(&text)
        .with_context(|| format!("failed to parse fleet manifest '{}'", fleet_path.display()))?;
    let base_dir = fleet_path.parent().unwrap_or_else(|| Path::new("."));

    let mut failed = Vec::new();
    for rel in &fleet.packages {
        let config = base_dir.join(rel);
        println!("=== {} ===", config.display());
        let mut job_args = args.clone();
        job_args.all = None;
        job_args.config = config.clone();
        if let Err(e) = run(job_args, token) {
            eprintln!("✗ {}: {e:#}", config.display());
            failed.push(config);
        }
    }

    if failed.is_empty() {
        println!(
            "\n✓ built {} package(s) from {}",
            fleet.packages.len(),
            fleet_path.display()
        );
        Ok(())
    } else {
        bail!(
            "{} of {} package(s) failed to build: {}",
            failed.len(),
            fleet.packages.len(),
            failed
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
}

pub fn run(args: BuildArgs, token: Option<&str>) -> Result<()> {
    if let Some(fleet_path) = args.all.clone() {
        return run_all(&fleet_path, &args, token);
    }
    let build_start = std::time::Instant::now();
    // Parallelism precedence (debian-multiarch-builder parity): an explicit
    // CLI --max-parallel wins over package.yaml's max_parallel; when neither
    // is set, auto-tune from detected resources (ci-optimization.sh +
    // resource-pool.sh parity). package.yaml `parallel_builds: false` pins
    // sequential. We need owned args because build_jobs reads it by ref.
    let cli_parallel = args.max_parallel;
    let mut args = args;

    // A bare GitHub/GitLab URL in place of a package.yaml path triggers a fully
    // zero-config build: no manual patterns, every supported architecture,
    // and source packages included (there's no config file to opt out via,
    // so the most useful default wins).
    let mut cfg = match crate::plugins::forge::parse_any_forge_url(&args.config.to_string_lossy()) {
        Some((source, repo)) => {
            println!("Zero-config build from {source}:{repo} (no package.yaml)");
            args.source = true;
            let cfg = PackageConfig {
                package_name: repo.split('/').next_back().unwrap_or(&repo).to_string(),
                github_repo: repo.clone(),
                source: source.clone(),
                ..PackageConfig::default()
            };
            cfg.validate()?;
            cfg
        }
        None => {
            // Fallback for plain package.yaml path; also handle legacy
            // `parse_github_url` for backward compat (though parse_any_forge_url
            // already covers it).
            if let Some(github_repo) =
                crate::plugins::forge::github::parse_github_url(&args.config.to_string_lossy())
            {
                println!("Zero-config build from {github_repo} (no package.yaml)");
                args.source = true;
                let cfg = PackageConfig {
                    package_name: github_repo
                        .split('/')
                        .next_back()
                        .unwrap_or(&github_repo)
                        .to_string(),
                    github_repo,
                    ..PackageConfig::default()
                };
                cfg.validate()?;
                cfg
            } else if args.from_dir.is_some() || args.from_file.is_some() {
                // "You supply files" mode: package.yaml is optional. Load it
                // as a base if it exists, else start from defaults.
                PackageConfig::load(&args.config).unwrap_or_default()
            } else {
                PackageConfig::load(&args.config)?
            }
        }
    };
    if let Some(overlay) = &args.overlay {
        cfg.apply_overlay(overlay)
            .with_context(|| format!("applying overlay '{}'", overlay.display()))?;
    }
    if let Some(v) = &args.version {
        cfg.version = v.clone();
    }
    // "You supply files" mode (fpm-style): a directory or file you supply is
    // the payload. Apply CLI overrides, point local_payload at it, and route
    // through run_local with relaxed validation (no github_repo needed).
    if args.from_dir.is_some() || args.from_file.is_some() {
        let path = args
            .from_dir
            .as_ref()
            .or(args.from_file.as_ref())
            .unwrap()
            .clone();
        if let Some(name) = &args.package_name {
            cfg.package_name = name.clone();
        }
        if let Some(prefix) = &args.prefix {
            cfg.prefix = prefix.clone();
        }
        if cfg.package_name.trim().is_empty() {
            // Default the package name to the source's file/dir name.
            cfg.package_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("package")
                .to_string();
        }
        cfg.local_payload = path.to_string_lossy().to_string();
        // Directory or single file: treat as already-extracted raw payload.
        cfg.artifact_format = "raw".to_string();
        cfg.validate_for_local()?;
        // Fall through to the local routing below (effective_format etc.
        // still need resolving first).
    }
    // Source-mode builds compile upstream on the host instead of repacking
    // release assets (bash `build_mode: source` parity, natively).
    if cfg.is_source_mode() {
        return crate::sourcebuild::run(args, &cfg, token);
    }

    // Resolve effective package format: --format overrides package.yaml.
    let effective_format = crate::config::canonical_format(
        args.format
            .as_deref()
            .unwrap_or(&cfg.effective_package_format()),
    );
    let plugin_names = crate::plugins::packager_names();
    if crate::plugins::get_packager(&effective_format).is_none() {
        bail!(
            "unsupported --format '{}' (expected one of: {})",
            effective_format,
            plugin_names.join(", ")
        );
    }
    // Normalize cfg's package_format for downstream consumers (summary, etc.).
    cfg.package_format = effective_format.clone();
    let plugin_desc = crate::plugins::get_packager(&effective_format)
        .unwrap()
        .description();
    println!("package format: {effective_format} ({plugin_desc})");

    // Input source plugins (language package managers): npm, python, gem.
    // These fetch from language registries and produce a local payload
    // directory, then flow through the normal packaging pipeline via run_local.
    let registry_source_name = cfg.effective_registry_source();
    if !registry_source_name.is_empty() {
        let input_plugin = crate::plugins::registry::get_registry_source(&registry_source_name)
            .ok_or_else(|| {
                anyhow!(
                    "unsupported input source '{}' (expected one of: {})",
                    registry_source_name,
                    crate::plugins::registry::registry_source_names().join(", ")
                )
            })?;

        // Check required tools.
        for tool in input_plugin.required_tools() {
            crate::sourcebuild::require_tool(tool)?;
        }

        // The package name for the registry is github_repo (there's no
        // separate "npm_package" field — github_repo doubles as the
        // external package identifier for non-forge sources).
        let package_id = if cfg.github_repo.trim().is_empty() {
            &cfg.package_name
        } else {
            &cfg.github_repo
        };

        println!(
            "input source: {} ({}) — {}",
            registry_source_name,
            package_id,
            input_plugin.description()
        );

        let payload = input_plugin.fetch(package_id, &cfg.version, &cfg)?;

        // Update config with resolved values.
        if cfg.version.trim().is_empty() && !payload.resolved_version.is_empty() {
            println!("  resolved version: {}", payload.resolved_version);
            cfg.version = payload.resolved_version;
        }
        if cfg.description.is_empty() && !payload.description.is_empty() {
            cfg.description = payload.description;
        }

        // Feature 5: Auto-map registry dependencies to system packages.
        // If the user didn't specify depends:, try to infer them from the
        // fetched package's dependency files.
        if cfg.depends.trim().is_empty() {
            if let Some(mapped) = lx_lib::depmap::infer_deps_from_dir(
                &registry_source_name,
                &payload.files_dir,
                &effective_format,
            ) {
                println!("  mapped depends: {mapped}");
                cfg.depends = mapped;
            }
        }

        // Apply package naming conventions if user didn't set package_name.
        if cfg.package_name.trim().is_empty() || cfg.package_name == "package" {
            let conventional = lx_lib::pkgname::conventional_name(
                &registry_source_name,
                package_id,
                &effective_format,
                None,
            );
            println!("  conventional name: {conventional}");
            cfg.package_name = conventional;
        }

        // Auto-detect architecture (all vs any) for registry packages.
        if cfg.architecture.trim().is_empty() || cfg.architecture == "auto" {
            let has_ext = lx_lib::pkgname::has_compiled_extensions(&payload.files_dir);
            let detected = lx_lib::pkgname::detect_architecture(&registry_source_name, has_ext);
            println!("  architecture: {detected}");
            cfg.architecture = detected.to_string();
        }

        // Route through local packaging with the fetched payload directory.
        cfg.local_payload = payload.files_dir.to_string_lossy().to_string();
        cfg.artifact_format = "raw".to_string();
        return run_local(args, cfg, &effective_format, build_start);
    }

    // Resolve source provider plugin (github vs gitlab) for auto-discovery.
    // Local builds skip the network entirely.
    let effective_forge_source = args
        .provider
        .as_deref()
        .unwrap_or(&cfg.effective_forge_source())
        .to_ascii_lowercase();
    cfg.source = effective_forge_source.clone();

    let sign_method = cfg.effective_sign_method(args.sign_method.as_deref());
    match sign_method.as_str() {
        "detach" | "debsign" => {}
        other => bail!("unsupported --sign-method '{other}' (expected detach or debsign)"),
    }

    if args.local || args.from_dir.is_some() || args.from_file.is_some() {
        if args.from_dir.is_some() || args.from_file.is_some() {
            println!("source: files you supplied (no forge fetch)");
        } else {
            println!("source: local (skipping upstream download)");
        }
        return run_local(args, cfg, &effective_format, build_start);
    }

    let source =
        crate::plugins::forge::get_forge_source(&effective_forge_source).ok_or_else(|| {
            anyhow!(
                "unsupported source '{}' (expected one of: {})",
                effective_forge_source,
                crate::plugins::forge::forge_source_names().join(", ")
            )
        })?;
    println!(
        "source: {} ({})",
        effective_forge_source,
        source.description()
    );
    crate::plugins::forge::apply_forge_host(source.as_ref(), &cfg);
    // Token resolution: prefer provider-specific env, fallback to CLI token.
    let token_for_source = crate::plugins::forge::resolve_forge_token(source.as_ref(), token);

    // Resolve the release: pinned version in config, else a tag/version, else latest.
    // `source: custom` has no forge API: the version (config or --version)
    // is expanded into the upstream_url template, one URL per architecture.
    let release = if effective_forge_source == "custom" {
        let version = cfg.version.clone();
        if version.is_empty() {
            bail!("source 'custom' requires version: in package.yaml or --version (upstream_url template is expanded with it)");
        }
        let template = cfg.upstream_url.trim().to_string();
        let mut custom_archs = cfg.effective_architectures();
        if let Some(a) = &args.architectures {
            custom_archs = a
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
        }
        if custom_archs.is_empty() {
            bail!("source 'custom' requires a non-empty architectures: list in package.yaml (or --architectures)");
        }
        crate::plugins::forge::custom::synthetic_release(
            &template,
            &version,
            &cfg.package_name,
            &custom_archs,
        )
    } else if !cfg.version.is_empty() {
        match source.release_by_tag(
            &cfg.github_repo,
            &cfg.version,
            token_for_source.as_deref(),
            args.api_cache_dir.as_deref(),
        ) {
            Ok(r) => r,
            Err(e) => {
                suggest_versions_source(
                    source.as_ref(),
                    &cfg.github_repo,
                    &cfg.version,
                    token_for_source.as_deref(),
                    args.api_cache_dir.as_deref(),
                );
                return Err(e);
            }
        }
    } else if cfg.has_manual_patterns() {
        source.latest_release(
            &cfg.github_repo,
            token_for_source.as_deref(),
            args.api_cache_dir.as_deref(),
        )?
    } else {
        // Zero-config mode: auto-discover from the latest release.
        source.latest_release(
            &cfg.github_repo,
            token_for_source.as_deref(),
            args.api_cache_dir.as_deref(),
        )?
    };
    warn_if_prerelease_or_draft(&release);

    // Resolve package.yaml's parallelism knobs now that the config is
    // loaded (CLI --max-parallel > config max_parallel > auto-tune;
    // parallel_builds: false forces a single worker).
    args.max_parallel = if cli_parallel > 0 {
        cli_parallel
    } else if cfg.parallel_builds == Some(false) {
        1
    } else if cfg.max_parallel > 0 {
        cfg.max_parallel
    } else {
        lx_lib::optimize::effective_max_parallel(0)
    };

    // Fetch the upstream license once (not per-architecture), mirroring the
    // action's fetch_upstream_license + dual-license detection. Best-effort:
    // on failure, fall back to the config's license_spdx and continue.
    let license = fetch_upstream_license_source(
        source.as_ref(),
        &cfg,
        token_for_source.as_deref(),
        args.api_cache_dir.as_deref(),
    )
    .unwrap_or(None);

    if args.host && args.architectures.is_some() {
        bail!("--host conflicts with --architectures; pass one or the other");
    }

    // Determine the effective build matrix (distributions depend on format).
    let mut dists = cfg.effective_distributions_for(&effective_format);
    let mut archs = cfg.effective_architectures();
    if let Some(d) = &args.distributions {
        dists = d
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
    }
    if let Some(a) = &args.architectures {
        archs = a
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
    }

    // Drop suites whose Debian LTS support has ended (the action's
    // filter_expired_distributions; e.g. bullseye ends 2026-08-31). Applies
    // to the config default and an explicit --distributions alike.
    dists = lx_lib::config::filter_expired_distributions(&dists, None);

    if args.host {
        let detected = host_arch().ok_or_else(|| {
            anyhow!("could not detect this machine's architecture from `uname -m`; use --architectures instead")
        })?;
        println!("Building for host architecture: {detected} (--host; native)");
        archs = vec![detected];
    }

    // Feature 3: Cross-compilation target. Overrides the architecture list
    // and enables musl static linking for reproducible multi-arch builds.
    if let Some(ref target) = args.cross_target {
        println!("Cross-compiling for: {target}");
        archs = vec![target.clone()];
        if !cfg.musl {
            cfg.musl = true;
            println!("  enabled musl-static for cross-compilation");
        }
    }

    // Resolve one asset per architecture. `source: custom` assets are already
    // per-arch (one expanded URL each), keyed by position — no pattern
    // matching, since there is no release listing to match against.
    let arch_assets = if effective_forge_source == "custom" {
        // Re-expand the template against the final arch list (post --host /
        // --architectures reshaping) so the URLs always match the archs
        // actually being built, regardless of what the synthetic release
        // was first built with.
        let version = release.tag_name.clone();
        let template = cfg.upstream_url.trim();
        let mut map = std::collections::HashMap::new();
        for arch in &archs {
            let (name, url) = crate::plugins::forge::custom::expand_for_arch(
                template,
                &version,
                arch,
                &cfg.package_name,
            );
            map.insert(
                arch.clone(),
                lx_lib::github::Asset {
                    name,
                    size: None,
                    browser_download_url: url,
                    checksums: Default::default(),
                },
            );
        }
        map
    } else if cfg.has_manual_patterns() {
        resolve_manual(&cfg, &release)?
    } else {
        let auto =
            crate::discovery::config_from_release_with_musl(&cfg.github_repo, &release, cfg.musl)?;
        resolve_manual(&auto, &release)?
    };

    let arch_assets: Vec<(String, Asset)> = archs
        .into_iter()
        .filter_map(|a| {
            if let Some(asset) = arch_assets.get(&a).cloned() {
                return Some((a, asset));
            }
            println!(
                "⚠️  Architecture '{a}' skipped: no release assets available for version {}",
                release.tag_name
            );
            None
        })
        .collect();

    if arch_assets.is_empty() {
        bail!("no release assets matched any requested architecture");
    }

    // Zero-config (no artifact_format in package.yaml, or none at all in
    // URL mode) previously left this empty all the way to `extract()`,
    // which would then reject it outright -- config_from_release's own
    // guess only ever landed on a throwaway `auto` config above, never on
    // `cfg` itself. Guess it here from whatever got resolved.
    if cfg.artifact_format.is_empty() {
        cfg.artifact_format = guess_format(&arch_assets[0].1.name).to_string();
    }

    let jobs: Vec<ResolvedJob> = dists
        .iter()
        .flat_map(|dist| {
            arch_assets
                .iter()
                .filter_map(|(arch, asset)| {
                    // A distribution_arch_overrides entry for the arch
                    // replaces the built-in support matrix entirely.
                    let supported =
                        match cfg.distribution_arch_overrides.get(arch.as_str()) {
                            Some(o) => o.distributions.iter().any(|d| d.trim() == dist),
                            // Arch/dist matrix only applies to deb; rpm/arch are distro-agnostic.
                            None => effective_format != "deb"
                                || cfg.arch_supported_for_dist(arch, dist),
                        };
                    if !supported {
                        println!(
                            "⚠️  Skipping {dist} for {arch}: architecture not supported in this distribution (format: {})",
                            effective_format
                        );
                        return None;
                    }
                    Some(ResolvedJob {
                        dist: dist.clone(),
                        arch: arch.clone(),
                        asset: asset.clone(),
                        tag: release.tag_name.clone(),
                        published_at: release.published_at,
                    })
                })
                .collect::<Vec<_>>()
        })
        .collect();

    if args.dry_run {
        println!("Would build {} jobs:", jobs.len());
        for j in &jobs {
            println!("  {:<8} {:<8} asset={}", j.dist, j.arch, j.asset.name);
        }
        return Ok(());
    }

    // Telemetry (action's TELEMETRY_ENABLED, default false) + live progress
    // (action's progress.sh). Both best-effort.
    let telemetry = lx_lib::telemetry::Telemetry::new(args.telemetry);
    telemetry.init()?;
    telemetry.record_stage("build_initialization")?;

    let progress = if args.progress && lx_lib::progress::stdout_is_tty() {
        Some(lx_lib::progress::Progress::new(
            arch_assets.len(),
            &release.tag_name,
            &cfg.package_name,
            args.progress_path
                .clone()
                .unwrap_or_else(|| PathBuf::from(lx_lib::constants::DEFAULT_PROGRESS_PATH)),
            true,
        )?)
    } else {
        None
    };

    let provenance = build_jobs(
        &args,
        &cfg,
        &jobs,
        SourceInputs {
            license: license.clone(),
            source_name: effective_forge_source.clone(),
            token: token_for_source.clone(),
        },
        progress.as_ref(),
        &telemetry,
    )?;

    telemetry.record_stage_complete("build_completion", "success")?;
    telemetry.finalize(build_start.elapsed().as_secs())?;
    if args.save_baseline {
        telemetry.save_as_baseline()?;
    } else if let Some(w) = telemetry.check_regression() {
        eprintln!("⚠️  Performance regression detected: {w}");
    }

    emit_post_build_artifacts(PostBuildInputs {
        args: &args,
        cfg: &cfg,
        release: &release,
        jobs: &jobs,
        provenance: &provenance,
        dists: &dists,
        arch_assets: &arch_assets,
        format: &effective_format,
        forge_source: &effective_forge_source,
        license: license.as_ref(),
        build_start,
        telemetry: &telemetry,
    })?;
    Ok(())
}

/// Everything the post-build artifact phase needs, grouped so `run` stays a
/// short pipeline and this phase can change independently of build setup.
struct PostBuildInputs<'a> {
    args: &'a BuildArgs,
    cfg: &'a PackageConfig,
    release: &'a lx_lib::github::Release,
    jobs: &'a [ResolvedJob],
    provenance: &'a [crate::summary::ProvenanceEntry],
    dists: &'a [String],
    arch_assets: &'a [(String, Asset)],
    format: &'a str,
    forge_source: &'a str,
    license: Option<&'a lx_lib::github::RepoLicense>,
    build_start: std::time::Instant,
    telemetry: &'a lx_lib::telemetry::Telemetry,
}

/// Emit everything derived from a completed build: the lock file, summary,
/// SBOM, source package, checksum sidecars, shell installer, and cosign
/// signatures.
fn emit_post_build_artifacts(input: PostBuildInputs<'_>) -> Result<()> {
    let PostBuildInputs {
        args,
        cfg,
        release,
        jobs,
        provenance,
        dists,
        arch_assets,
        format,
        forge_source,
        license,
        build_start,
        telemetry,
    } = input;

    if args.update_lock {
        write_lock_file(&args.config, forge_source, jobs, provenance)?;
    }

    if args.summary {
        crate::summary::write(
            &args.output,
            jobs.len(),
            &crate::summary::SummaryInputs {
                package: cfg.package_name.clone(),
                version: release.tag_name.clone(),
                build_version: args.build_version.clone(),
                github_repo: cfg.github_repo.clone(),
                architectures: arch_assets.iter().map(|(a, _)| a.clone()).collect(),
                distributions: dists.to_vec(),
                max_parallel: args.max_parallel,
                start: build_start,
                telemetry: telemetry.summary_json(),
                provenance: provenance.to_vec(),
                package_format: format.to_string(),
                source: forge_source.to_string(),
            },
        )?;
    }

    if args.sbom {
        let artifacts = lx_lib::sbom::collect_artifacts(&args.output)?;
        let mut seen = std::collections::BTreeSet::new();
        let materials: Vec<lx_lib::sbom::Material> = provenance
            .iter()
            .filter_map(|p| {
                if p.url.is_empty() || !seen.insert(p.url.clone()) {
                    return None;
                }
                let digest = (!p.sha256.is_empty()).then(|| p.sha256.clone());
                Some(lx_lib::sbom::Material {
                    uri: p.url.clone(),
                    digest,
                })
            })
            .collect();
        lx_lib::sbom::emit(
            &args.output,
            &cfg.package_name,
            &release.tag_name,
            &args.build_version,
            &artifacts,
            &materials,
        )?;
    }

    if args.source {
        let rel = cfg.effective_relations(format);
        let pkg = crate::source::Pkg {
            name: cfg.package_name.clone(),
            github_repo: cfg.github_repo.clone(),
            description: cfg.effective_description(),
            maintainer: cfg.effective_maintainer(),
            version: release.tag_name.clone(),
            build_version: args.build_version.clone(),
            epoch: cfg.epoch.clone(),
            license_spdx: license
                .map(|l| l.spdx.clone())
                .unwrap_or_else(|| "NOASSERTION".to_string()),
            depends: rel.depends,
            recommends: rel.recommends,
            suggests: rel.suggests,
            conflicts: rel.conflicts,
            replaces: rel.replaces,
            provides: rel.provides,
            breaks: rel.breaks,
            predepends: rel.predepends,
            section: cfg.section.clone(),
            priority: cfg.priority.clone(),
            fields: cfg.fields.clone(),
            published_at: release.published_at,
            license: license.cloned(),
        };
        crate::plugins::get_source_packager(format)
            .ok_or_else(|| anyhow::anyhow!("'{format}' has no source-package format"))?
            .generate_source_package(&args.output, &pkg)?;
    }

    // Checksum sidecars (.sha256, .sha512) for integrity verification.
    lx_lib::checksum_sidecar::generate_checksum_sidecars(&args.output)?;

    // Shell installer script (curl | sh).
    lx_lib::shell_installer::generate_shell_installer(
        &args.output,
        &cfg.package_name,
        &release.tag_name,
    )?;

    // Cosign signing (Sigstore keyless).
    if args.cosign {
        for entry in std::fs::read_dir(&args.output)? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) == Some("sig") {
                continue;
            }
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if lx_lib::checksum_sidecar::is_package_file(&name) {
                lx_lib::cosign::cosign_sign_blob(&path)?;
            }
        }
    }

    Ok(())
}

/// Rebuilds `package.lock` from this build's provenance (one entry per
/// architecture -- distributions sharing an arch share its asset) and
/// writes it beside the config. Called only for `--update-lock`.
fn write_lock_file(
    config: &Path,
    source_name: &str,
    jobs: &[ResolvedJob],
    provenance: &[crate::summary::ProvenanceEntry],
) -> Result<()> {
    let mut lock = lx_lib::lock::LockFile::default();
    for p in provenance {
        if p.arch.is_empty() {
            continue;
        }
        let published_at = jobs
            .iter()
            .find(|j| j.arch == p.arch)
            .and_then(|j| j.published_at);
        lock.packages.insert(
            p.arch.clone(),
            lx_lib::lock::LockEntry {
                tag: p.tag.clone(),
                asset: p.asset.clone(),
                url: p.url.clone(),
                sha256: p.sha256.clone(),
                source: source_name.to_string(),
                published_at,
            },
        );
    }
    let lock_path = lx_lib::lock::LockFile::path_for(config);
    lock.save(&lock_path)?;
    println!("  ✓ package.lock updated at {}", lock_path.display());
    Ok(())
}

/// `--local` path: package a local archive/directory without hitting the
/// upstream forge. Requires `local_payload` in package.yaml (and usually
/// `version:` / `--version`).
fn run_local(
    mut args: BuildArgs,
    mut cfg: PackageConfig,
    effective_format: &str,
    build_start: std::time::Instant,
) -> Result<()> {
    let payload = cfg.local_payload.trim();
    if payload.is_empty() {
        bail!("--local requires local_payload in package.yaml (path to an archive or directory)");
    }
    let payload_path = PathBuf::from(payload);
    if !payload_path.exists() {
        bail!("local_payload '{}' does not exist", payload_path.display());
    }

    let version = if !cfg.version.is_empty() {
        cfg.version.clone()
    } else if let Some(v) = &args.version {
        v.clone()
    } else {
        bail!("--local requires version: in package.yaml or --version");
    };

    let cli_parallel = args.max_parallel;
    args.max_parallel = if cli_parallel > 0 {
        cli_parallel
    } else if cfg.parallel_builds == Some(false) {
        1
    } else if cfg.max_parallel > 0 {
        cfg.max_parallel
    } else {
        lx_lib::optimize::effective_max_parallel(0)
    };

    if args.host && args.architectures.is_some() {
        bail!("--host conflicts with --architectures; pass one or the other");
    }

    let mut dists = cfg.effective_distributions_for(effective_format);
    let mut archs = cfg.effective_architectures();
    if let Some(d) = &args.distributions {
        dists = d
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
    }
    if let Some(a) = &args.architectures {
        archs = a
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
    }
    dists = lx_lib::config::filter_expired_distributions(&dists, None);

    if args.host {
        let detected = host_arch().ok_or_else(|| {
            anyhow!("could not detect this machine's architecture from `uname -m`; use --architectures instead")
        })?;
        println!("Building for host architecture: {detected} (--host; native)");
        archs = vec![detected];
    }

    if cfg.artifact_format.is_empty() {
        if payload_path.is_dir() {
            // Directory payload: treat as already-extracted; extract() is
            // skipped in build_one for dirs.
            cfg.artifact_format = "raw".to_string();
        } else {
            let name = payload_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            cfg.artifact_format = guess_format(name).to_string();
        }
    }

    let asset_name = payload_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("local-payload")
        .to_string();
    let fake_asset = Asset {
        name: asset_name,
        size: payload_path.metadata().ok().map(|m| m.len()),
        browser_download_url: String::new(),
        checksums: Default::default(),
    };

    let mut jobs: Vec<ResolvedJob> = Vec::new();
    for dist in &dists {
        for arch in &archs {
            let supported = match cfg.distribution_arch_overrides.get(arch.as_str()) {
                Some(o) => o.distributions.iter().any(|d| d.trim() == dist.as_str()),
                // Arch/dist matrix only applies to deb; rpm/arch are distro-agnostic.
                None => effective_format != "deb" || cfg.arch_supported_for_dist(arch, dist),
            };
            if !supported {
                println!(
                    "⚠️  Skipping {dist} for {arch}: architecture not supported in this distribution (format: {effective_format})"
                );
                continue;
            }
            jobs.push(ResolvedJob {
                dist: dist.clone(),
                arch: arch.clone(),
                asset: fake_asset.clone(),
                tag: version.clone(),
                published_at: None,
            });
        }
    }

    if jobs.is_empty() {
        bail!("no local build jobs matched the requested architecture/distribution matrix");
    }

    if args.dry_run {
        println!(
            "Would build {} local jobs from {}:",
            jobs.len(),
            payload_path.display()
        );
        for j in &jobs {
            println!("  {:<8} {:<8}", j.dist, j.arch);
        }
        return Ok(());
    }

    let telemetry = lx_lib::telemetry::Telemetry::new(args.telemetry);
    telemetry.init()?;
    telemetry.record_stage("build_initialization")?;

    let progress = if args.progress && lx_lib::progress::stdout_is_tty() {
        Some(lx_lib::progress::Progress::new(
            archs.len(),
            &version,
            &cfg.package_name,
            args.progress_path
                .clone()
                .unwrap_or_else(|| PathBuf::from(lx_lib::constants::DEFAULT_PROGRESS_PATH)),
            true,
        )?)
    } else {
        None
    };

    let provenance = build_jobs(
        &args,
        &cfg,
        &jobs,
        SourceInputs {
            license: if cfg.license_spdx.is_empty() {
                None
            } else {
                Some(lx_lib::github::RepoLicense {
                    spdx: cfg.license_spdx.clone(),
                    text: None,
                })
            },
            source_name: String::new(), // unused when args.local
            token: None,
        },
        progress.as_ref(),
        &telemetry,
    )?;

    telemetry.record_stage_complete("build_completion", "success")?;
    telemetry.finalize(build_start.elapsed().as_secs())?;
    if args.save_baseline {
        telemetry.save_as_baseline()?;
    } else if let Some(w) = telemetry.check_regression() {
        eprintln!("⚠️  Performance regression detected: {w}");
    }

    if args.summary {
        crate::summary::write(
            &args.output,
            jobs.len(),
            &crate::summary::SummaryInputs {
                package: cfg.package_name.clone(),
                version: version.clone(),
                build_version: args.build_version.clone(),
                github_repo: cfg.github_repo.clone(),
                architectures: archs.clone(),
                distributions: dists.clone(),
                max_parallel: args.max_parallel,
                start: build_start,
                telemetry: telemetry.summary_json(),
                provenance,
                package_format: effective_format.to_string(),
                source: "local".to_string(),
            },
        )?;
    }

    if args.source {
        let rel = cfg.effective_relations(effective_format);
        let pkg = crate::source::Pkg {
            name: cfg.package_name.clone(),
            github_repo: cfg.github_repo.clone(),
            description: cfg.effective_description(),
            maintainer: cfg.effective_maintainer(),
            version: version.clone(),
            build_version: args.build_version.clone(),
            epoch: cfg.epoch.clone(),
            license_spdx: if cfg.license_spdx.is_empty() {
                "NOASSERTION".to_string()
            } else {
                cfg.license_spdx.clone()
            },
            depends: rel.depends,
            recommends: rel.recommends,
            suggests: rel.suggests,
            conflicts: rel.conflicts,
            replaces: rel.replaces,
            provides: rel.provides,
            breaks: rel.breaks,
            predepends: rel.predepends,
            section: cfg.section.clone(),
            priority: cfg.priority.clone(),
            fields: cfg.fields.clone(),
            published_at: None,
            license: if cfg.license_spdx.is_empty() {
                None
            } else {
                Some(lx_lib::github::RepoLicense {
                    spdx: cfg.license_spdx.clone(),
                    text: None,
                })
            },
        };
        crate::plugins::get_source_packager(effective_format)
            .ok_or_else(|| anyhow::anyhow!("'{effective_format}' has no source-package format"))?
            .generate_source_package(&args.output, &pkg)?;
    }
    Ok(())
}

pub(crate) fn suggest_versions_source(
    source: &dyn crate::plugins::forge::ForgeSource,
    repo: &str,
    wanted: &str,
    token: Option<&str>,
    cache_dir: Option<&Path>,
) {
    eprintln!(
        "Version '{wanted}' not found for {repo} (source: {}).",
        source.name()
    );
    match source.releases(repo, 5, token, cache_dir) {
        Ok(metas) if !metas.is_empty() => {
            eprintln!("  Recent releases:");
            for m in metas {
                let date = m
                    .published_at
                    .as_deref()
                    .map(|d| format!(" ({d})"))
                    .unwrap_or_default();
                eprintln!("    - {}{}", m.tag, date);
            }
        }
        _ => {
            eprintln!("  No recent releases could be listed; check {repo} releases");
        }
    }
}

/// Fetch the upstream license via source plugin (github/gitlab agnostic).
fn fetch_upstream_license_source(
    source: &dyn crate::plugins::forge::ForgeSource,
    cfg: &PackageConfig,
    token: Option<&str>,
    cache_dir: Option<&Path>,
) -> Result<Option<lx_lib::github::RepoLicense>> {
    let mut license = source.repo_license(&cfg.github_repo, token, cache_dir)?;

    // Dual-license detection (only meaningful for GitHub where LICENSE-* files exist;
    // for GitLab it will be empty and harmless).
    if let Ok(root) = source.repo_root(&cfg.github_repo, token, cache_dir) {
        let has_apache = root.iter().any(|n| n == "LICENSE-APACHE");
        let has_mit = root.iter().any(|n| n == "LICENSE-MIT");
        if has_apache && has_mit {
            let apache = source
                .repo_file_text(&cfg.github_repo, "LICENSE-APACHE", token, cache_dir)?
                .unwrap_or_default();
            let mit = source
                .repo_file_text(&cfg.github_repo, "LICENSE-MIT", token, cache_dir)?
                .unwrap_or_default();
            if !apache.is_empty() && !mit.is_empty() {
                license = Some(lx_lib::github::RepoLicense {
                    spdx: "Apache-2.0 or MIT".to_string(),
                    text: Some(format!(
                        "Dual-licensed under either of:\n\n=== Apache License 2.0 ===\n\n{apache}\n\n=== MIT License ===\n\n{mit}"
                    )),
                });
            }
        }
    }

    if license.is_none() && !cfg.license_spdx.is_empty() {
        license = Some(lx_lib::github::RepoLicense {
            spdx: cfg.license_spdx.clone(),
            text: None,
        });
    }

    if let Some(l) = &license {
        println!(
            "license: {} ({}) [source: {}]",
            l.spdx,
            if l.text.is_some() {
                "full text"
            } else {
                "no text"
            },
            source.name()
        );
    } else {
        println!("license: NOASSERTION (no machine-detectable license upstream)");
    }
    Ok(license)
}

/// `{version}` placeholder substitution with `v`-prefix dedup. The bash
/// action's convention was bare versions (`./build.sh cfg 0.18.0 1`), so
/// its legacy patterns carry a literal `v` (`pkg_v{version}_...`). lx
/// always substitutes the full tag (`v0.23.5`), which would produce
/// `pkg_vv0.23.5`. When a literal `v`/`V` directly precedes the
/// placeholder, the tag's own prefix is donated so both styles resolve to
/// exactly one `v`. Patterns without an adjacent `v` get the tag verbatim.
pub fn expand_version_placeholder(pattern: &str, tag: &str) -> String {
    let mut out = String::with_capacity(pattern.len() + tag.len());
    let mut rest = pattern;
    while let Some(i) = rest.find("{version}") {
        let (before, after) = rest.split_at(i);
        let after = &after["{version}".len()..];
        out.push_str(before);
        let mut donated: &str = tag;
        let preceded_by_v = matches!(
            before.chars().next_back().map(|c| c.to_ascii_lowercase()),
            Some('v')
        );
        let tag_v = donated
            .strip_prefix('v')
            .or_else(|| donated.strip_prefix('V'));
        if preceded_by_v {
            if let Some(stripped) = tag_v {
                donated = stripped;
            }
        }
        out.push_str(donated);
        rest = after;
    }
    out.push_str(rest);
    out
}

/// Map each requested Debian architecture to a concrete release asset using
/// the config's pinned release_pattern.
pub(crate) fn resolve_manual(
    cfg: &PackageConfig,
    release: &lx_lib::github::Release,
) -> Result<std::collections::HashMap<String, Asset>> {
    let mut out = std::collections::HashMap::new();
    for (arch, acfg) in cfg.architectures.patterns() {
        let pattern = &acfg.release_pattern;
        if pattern.is_empty() {
            continue;
        }
        let expanded = expand_version_placeholder(pattern, &release.tag_name);
        let asset = release
            .assets
            .iter()
            .find(|a| a.name == expanded)
            .cloned()
            .ok_or_else(|| {
                anyhow!(
                    "asset '{expanded}' (pattern '{pattern}' for arch '{arch}') not found in release '{}'. Available: {}",
                    release.tag_name,
                    release.assets.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", ")
                )
            })?;
        out.insert(arch.clone(), asset);
    }
    // Auto-discovery fallback for any archs without a pattern.
    let matched = crate::discovery::match_assets_with_musl(release, cfg.musl);
    for m in matched {
        out.entry(m.arch)
            .or_insert(asset_from_name(release, &m.asset));
    }
    Ok(out)
}

/// Surface a resolved release's prerelease/draft status. Both fields were
/// already parsed off the GitHub API response but never consulted anywhere
/// -- someone pinning an explicit tag that happens to be an RC/beta got no
/// signal that they were about to package pre-stable software.
pub(crate) fn warn_if_prerelease_or_draft(release: &lx_lib::github::Release) {
    if release.draft {
        println!(
            "  ⚠️  '{}' is a draft release -- not yet publicly published",
            release.tag_name
        );
    }
    if release.prerelease {
        println!(
            "  ⚠️  '{}' is marked as a pre-release (not yet considered stable upstream)",
            release.tag_name
        );
    }
}

fn asset_from_name(release: &lx_lib::github::Release, name: &str) -> Asset {
    release
        .assets
        .iter()
        .find(|a| a.name == name)
        .cloned()
        .unwrap_or_else(|| Asset {
            name: name.to_string(),
            size: None,
            browser_download_url: String::new(),
            checksums: Default::default(),
        })
}

/// Source-provider inputs resolved once in `run` and threaded through every
/// worker thread `build_jobs` spawns.
struct SourceInputs {
    license: Option<lx_lib::github::RepoLicense>,
    source_name: String,
    token: Option<String>,
}

fn build_jobs(
    args: &BuildArgs,
    cfg: &PackageConfig,
    jobs: &[ResolvedJob],
    source: SourceInputs,
    progress: Option<&lx_lib::progress::Progress>,
    telemetry: &lx_lib::telemetry::Telemetry,
) -> Result<Vec<crate::summary::ProvenanceEntry>> {
    let SourceInputs {
        license,
        source_name,
        token,
    } = source;
    std::fs::create_dir_all(&args.output)?;

    let pin = match &args.pinned_metadata {
        Some(p) => Some(lx_lib::checksum::PinnedMetadata::load(p)?),
        None => None,
    };

    // `--update-lock` means "regenerate the lock", not "verify against it" --
    // treat this run as unpinned so it falls back to live checksum
    // verification like a first-ever build would.
    let lock_path = lx_lib::lock::LockFile::path_for(&args.config);
    let lock = if args.update_lock {
        None
    } else {
        lx_lib::lock::LockFile::load(&lock_path)?
    };

    // Group jobs by architecture: each worker builds one architecture's
    // distributions sequentially, mirroring the action's parallel-by-arch
    // orchestration (GNU parallel + MAX_PARALLEL).
    let mut by_arch: std::collections::HashMap<String, Vec<ResolvedJob>> =
        std::collections::HashMap::new();
    for job in jobs {
        by_arch
            .entry(job.arch.clone())
            .or_default()
            .push(job.clone());
    }
    let groups: Vec<Vec<ResolvedJob>> = by_arch.into_values().collect();

    let tmp = tempfile::tempdir().context("failed to create temp dir")?;
    let progress = progress.cloned();

    let workers = args.max_parallel.max(1);
    let mut handles = Vec::new();
    let failures = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    // Shared across every worker thread -- one entry per unique asset
    // download, regardless of which architecture/distribution triggered it.
    let provenance = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));

    // A simple bounded pool: each worker grabs the next arch group until
    // none remain, so runtime parallelism is capped at `workers`.
    let next = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    for _ in 0..workers {
        let groups = groups.clone();
        let next = std::sync::Arc::clone(&next);
        let failures = std::sync::Arc::clone(&failures);
        let provenance = std::sync::Arc::clone(&provenance);
        let tmp = tmp.path().to_path_buf();
        let args = args.clone();
        let cfg = cfg.clone();
        let license = license.clone();
        let token = token.clone();
        let pin = pin.clone();
        let lock = lock.clone();
        let progress = progress.clone();
        let telemetry = telemetry.clone();
        let source_name = source_name.clone();
        handles.push(std::thread::spawn(move || {
            let source = if args.local || args.from_dir.is_some() || args.from_file.is_some() {
                None
            } else {
                Some(
                    crate::plugins::forge::get_forge_source(&source_name)
                        .expect("unknown source plugin"),
                )
            };
            loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let Some(group) = groups.get(i) else { break };
                let arch = &group[0].arch;
                let _ = progress.as_ref().map(|p| p.set_arch(arch, "running"));
                // Fresh download map per architecture worker (distinct assets).
                let mut downloaded: std::collections::HashMap<String, PathBuf> =
                    std::collections::HashMap::new();
                let mut group_ok = true;
                let build_inputs = BuildInputs {
                    source: source.as_deref(),
                    token: token.as_deref(),
                    license: license.as_ref(),
                    pin: pin.as_ref(),
                    lock: lock.as_ref(),
                };
                for job in group {
                    let result = build_one(
                        &args,
                        &cfg,
                        &build_inputs,
                        job,
                        &tmp,
                        &mut downloaded,
                        &provenance,
                    );
                    match result {
                        Ok(deb) => println!("  ✓ built {} ({})", deb.display(), job.dist),
                        Err(e) => {
                            eprintln!("  ✗ {}/{}: {e:#}", job.arch, job.dist);
                            failures.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            let _ = telemetry.record_failure(
                                "architecture_build",
                                &format!("Failed to build architecture {arch}"),
                                1,
                            );
                            group_ok = false;
                        }
                    }
                }
                let outcome = if group_ok {
                    lx_lib::progress::Outcome::Completed
                } else {
                    lx_lib::progress::Outcome::Failed
                };
                let _ = progress.as_ref().map(|p| p.finish_arch(arch, outcome));
                let _ = telemetry.record_stage_complete(
                    &format!("architecture_{arch}"),
                    if group_ok { "success" } else { "failure" },
                );
            }
        }));
    }
    for h in handles {
        h.join().map_err(|_| anyhow!("build worker panicked"))?;
    }

    if let Some(p) = progress {
        let _ = p.cleanup();
    }

    if !args.keep {
        let _ = tmp.close();
    } else {
        eprintln!("keeping temp files in {}", tmp.path().display());
        std::mem::forget(tmp);
    }

    let failures = failures.load(std::sync::atomic::Ordering::SeqCst);
    if failures > 0 {
        bail!("{failures} of {} builds failed", jobs.len());
    }
    Ok(std::sync::Arc::try_unwrap(provenance)
        .map(|m| m.into_inner().unwrap_or_default())
        .unwrap_or_default())
}

/// Per-worker source/verification context, resolved once per architecture
/// group and threaded through every [`build_one`] call in that group.
#[derive(Clone, Copy)]
struct BuildInputs<'a> {
    source: Option<&'a dyn crate::plugins::forge::ForgeSource>,
    token: Option<&'a str>,
    license: Option<&'a lx_lib::github::RepoLicense>,
    pin: Option<&'a lx_lib::checksum::PinnedMetadata>,
    lock: Option<&'a lx_lib::lock::LockFile>,
}

/// Resolve the payload for `job`: a local path, an already-downloaded asset,
/// or a fresh download through the source plugin, then verify it per the
/// pinned / lock / sidecar / allow-unverified / no-verify policy and record a
/// provenance entry. Returns the path on disk.
#[allow(clippy::too_many_arguments)]
fn resolve_asset(
    args: &BuildArgs,
    cfg: &PackageConfig,
    source: Option<&dyn crate::plugins::forge::ForgeSource>,
    token: Option<&str>,
    pin: Option<&lx_lib::checksum::PinnedMetadata>,
    lock: Option<&lx_lib::lock::LockFile>,
    job: &ResolvedJob,
    tmp: &Path,
    downloaded: &mut std::collections::HashMap<String, PathBuf>,
    provenance: &std::sync::Mutex<Vec<crate::summary::ProvenanceEntry>>,
) -> Result<PathBuf> {
    if args.local || args.from_dir.is_some() || args.from_file.is_some() {
        let p = PathBuf::from(cfg.local_payload.trim());
        if !p.exists() {
            bail!("local_payload '{}' does not exist", p.display());
        }
        return Ok(p);
    }
    let source = source.expect("source plugin required for non-local builds");
    if let Some(p) = downloaded.get(&job.asset.name) {
        return Ok(p.clone());
    }

    let path = tmp.join(&job.asset.name);
    println!(
        "  ↓ {} ({})",
        job.asset.name,
        human_size(job.asset.size.unwrap_or(0))
    );
    match &args.cache_dir {
        Some(dir) => {
            let expected = pin.and_then(|p| p.sha256_for(&job.tag, &job.asset.name));
            let cache = lx_lib::cache::DownloadCache::new(dir.clone())?;
            let source_cloned = source.name().to_string();
            let token_cloned = token.map(|s| s.to_string());
            cache.fetch(
                &job.asset.browser_download_url,
                &path,
                expected.as_deref(),
                &|url, out| {
                    let src = crate::plugins::forge::get_forge_source(&source_cloned)
                        .expect("unknown source");
                    let mut body = src
                        .raw_get(url, token_cloned.as_deref())
                        .with_context(|| format!("GET {url} failed"))?;
                    let mut file = std::fs::File::create(out)?;
                    std::io::copy(&mut body, &mut file)?;
                    Ok(())
                },
            )?;
        }
        None => {
            let mut body = source
                .raw_get(&job.asset.browser_download_url, token)
                .with_context(|| format!("GET {} failed", job.asset.browser_download_url))?;
            let mut file = std::fs::File::create(&path)?;
            std::io::copy(&mut body, &mut file)?;
        }
    }

    let method = if args.no_verify {
        VerifyMethod::SkippedNoVerify
    } else if let Some(pin) = pin {
        if let Some(expected) = pin.sha256_for(&job.tag, &job.asset.name) {
            lx_lib::checksum::verify_sha256(&path, &expected)?;
            println!("    ✓ verified against pinned sha256:{}", &expected[..12]);
            VerifyMethod::Pinned
        } else {
            eprintln!(
                "    (no vetted pin for '{}' @ {}; falling back to live checksum)",
                job.asset.name, job.tag
            );
            verify_sidecar_or_require_flag_source(
                source,
                token,
                &job.asset,
                &path,
                args.allow_unverified,
            )?
        }
    } else if let Some(lock) = lock {
        match lock.entry_for(&job.arch) {
            Some(entry) if entry.tag == job.tag && entry.asset == job.asset.name => {
                lx_lib::checksum::verify_sha256(&path, &entry.sha256)?;
                println!(
                    "    ✓ verified against package.lock sha256:{}",
                    &entry.sha256[..entry.sha256.len().min(12)]
                );
                VerifyMethod::Locked
            }
            Some(entry) => bail!(
                "package.lock drift for arch '{}': locked {}/{}, resolved {}/{} -- \
                 pass --update-lock to accept the new asset",
                job.arch,
                entry.tag,
                entry.asset,
                job.tag,
                job.asset.name
            ),
            None => {
                eprintln!(
                    "    (no package.lock entry for arch '{}'; falling back to live checksum)",
                    job.arch
                );
                verify_sidecar_or_require_flag_source(
                    source,
                    token,
                    &job.asset,
                    &path,
                    args.allow_unverified,
                )?
            }
        }
    } else {
        verify_sidecar_or_require_flag_source(
            source,
            token,
            &job.asset,
            &path,
            args.allow_unverified,
        )?
    };

    // Audit trail: one entry per unique download, regardless of outcome, so
    // build-summary.json's `provenance` array records exactly how (or whether)
    // every asset was verified.
    let sha256 = lx_lib::checksum::sha256_file(&path).unwrap_or_default();
    provenance
        .lock()
        .unwrap()
        .push(crate::summary::ProvenanceEntry {
            asset: job.asset.name.clone(),
            url: job.asset.browser_download_url.clone(),
            tag: job.tag.clone(),
            arch: job.arch.clone(),
            method: method.as_str().to_string(),
            sha256,
        });

    downloaded.insert(job.asset.name.clone(), path.clone());
    Ok(path)
}

fn build_one(
    args: &BuildArgs,
    cfg: &PackageConfig,
    inputs: &BuildInputs,
    job: &ResolvedJob,
    tmp: &Path,
    downloaded: &mut std::collections::HashMap<String, PathBuf>,
    provenance: &std::sync::Mutex<Vec<crate::summary::ProvenanceEntry>>,
) -> Result<PathBuf> {
    let BuildInputs {
        source,
        token,
        license,
        pin,
        lock,
    } = *inputs;

    // 1. Resolve the payload: local path, or download the asset once per name.
    // 1. Resolve the payload: local path, or download the asset once per name.
    let asset_path = resolve_asset(
        args, cfg, source, token, pin, lock, job, tmp, downloaded, provenance,
    )?;

    // Resolved once here (rather than at step 4) so the artifact-cache key
    // below can include everything that affects the output bytes.
    let format = cfg.effective_package_format();
    let sign_key = cfg.effective_sign_key(args.sign_key.as_deref());
    let sign_key_id = cfg.effective_sign_key_id(args.sign_key_id.as_deref());
    let sign_method = cfg.effective_sign_method(args.sign_method.as_deref());

    // Input-keyed artifact cache: key = recipe (config + every build/sign/
    // lintian flag that affects the output bytes or validation) + asset
    // digest + format + dist + arch. Skipped for directory payloads (no
    // single file to digest) and for detached signing (the cache doesn't
    // track the sibling `.sig`).
    let artifact_cache_key =
        if let (Some(dir), true) = (&args.artifact_cache_dir, asset_path.is_file()) {
            if sign_key.is_some() && sign_method == "detach" {
                None
            } else {
                let asset_sha256 = lx_lib::checksum::sha256_file(&asset_path).ok();
                asset_sha256.map(|digest| {
                    let recipe = serde_json::json!({
                        "cfg": cfg,
                        "build_version": args.build_version,
                        "sign_key": sign_key,
                        "sign_key_id": sign_key_id,
                        "sign_method": sign_method,
                        "lintian": args.lintian,
                        "lintian_fail_on_warnings": args.lintian_fail_on_warnings,
                        "lintian_pedantic": args.lintian_pedantic,
                        "lintian_suppress": args.lintian_suppress,
                        "digest": digest,
                        "format": format,
                        "dist": job.dist,
                        "arch": job.arch,
                    });
                    let key = lx_lib::cache::ArtifactCache::key(&recipe.to_string());
                    (dir.clone(), key)
                })
            }
        } else {
            None
        };
    // --verify forces a real rebuild even on a cache hit, so it can
    // byte-compare the fresh output against what's cached below; stash the
    // pre-existing cached file here rather than returning early.
    let mut verify_against: Option<PathBuf> = None;
    if let Some((dir, key)) = &artifact_cache_key {
        let cache = lx_lib::cache::ArtifactCache::new(dir.clone())?;
        if let Some(cached) = cache.get(key) {
            if args.verify {
                verify_against = Some(cached);
            } else if let Some(name) = cache.name_for(key) {
                let final_path = args.output.join(&name);
                std::fs::copy(&cached, &final_path).with_context(|| {
                    format!("copying cached artifact to {}", final_path.display())
                })?;
                println!(
                    "    (cache) reusing built artifact {}",
                    final_path.display()
                );
                return Ok(final_path);
            }
        }
    }

    // 2. Extract (or use directory payload as-is).
    let extract_dir = if asset_path.is_dir() {
        asset_path.clone()
    } else {
        let extract_dir = tmp.join(format!("{}-{}-extract", job.arch, job.dist));
        extract(&asset_path, &extract_dir, &cfg.artifact_format)?;
        extract_dir
    };

    // 3. Locate the binary to package.
    let binary_dir = if cfg.binary_path.is_empty() {
        extract_dir.clone()
    } else {
        extract_dir.join(&cfg.binary_path)
    };
    if !binary_dir.is_dir() {
        bail!(
            "binary_path '{}' does not exist in archive",
            cfg.binary_path
        );
    }

    // 3b. Scan ELF dependencies and auto-fill or verify depends:.
    // Only for binary repacks (not source builds, which handle this in
    // sourcebuild::compute_depends). Skipped for musl-static builds.
    let (scanned_depends, scanned_sonames) =
        lx_lib::scandeps::compute_depends_from_dir(&binary_dir, &cfg.depends, cfg.musl);
    let mut modified_cfg;
    let effective_cfg = if !cfg.musl && format != "source" {
        if cfg.depends.trim().is_empty() && scanned_depends != "libc6" {
            // Auto-fill: source had no deps, scanning found real needs.
            modified_cfg = cfg.clone();
            modified_cfg.depends = scanned_depends.clone();
            println!(
                "    ℹ auto-filled depends from ELF scanning: {}",
                scanned_depends
            );
            &modified_cfg
        } else if !cfg.depends.is_empty() {
            // Verify: source had deps — warn on mismatches.
            let (missing, unnecessary) =
                lx_lib::scandeps::diff_deps(&scanned_sonames, &cfg.depends);
            if !missing.is_empty() {
                println!(
                    "    ⚠ ELF needs packages not in depends: {}",
                    missing.join(", ")
                );
            }
            if !unnecessary.is_empty() {
                println!(
                    "    ℹ declared but not in ELF needs: {}",
                    unnecessary.join(", ")
                );
            }
            cfg
        } else {
            cfg
        }
    } else {
        cfg
    };

    // 4. Build the package via the selected plugin (deb or rpm).
    // Packager stages the install tree and creates the archive.
    let plugin = crate::plugins::get_packager(&format).ok_or_else(|| {
        anyhow!(
            "unsupported package format '{}' (expected one of: {})",
            format,
            crate::plugins::packager_names().join(", ")
        )
    })?;
    let raw_version = if cfg.version.is_empty() {
        job.tag.clone()
    } else {
        cfg.version.clone()
    };
    let debian_version =
        lx_lib::pkgmeta::normalize_version(&raw_version, &cfg.effective_version_schema());
    let mtime = match cfg.effective_mtime()? {
        Some(m) => m,
        None => lx_lib::pkgmeta::reproducible_epoch(job.published_at),
    };
    let staging_root = tmp.join(format!(
        "{}-{}-root-{}",
        job.arch,
        job.dist,
        plugin.file_extension()
    ));
    std::fs::create_dir_all(&staging_root)?;
    // Signing: rpm embeds natively; deb debsign embeds via the plugin;
    // deb detach signs post-build (.sig).
    let sign_passphrase = resolve_sign_passphrase();

    // Detect binary dependencies from the binaries being packaged.
    // Done before staging so the format plugin can include them in metadata.
    let detected_deps = if args.bindep {
        let deps = crate::bindep::detect_binary_deps_excluding(
            &binary_dir,
            Some(&effective_cfg.package_name),
        )
        .unwrap_or_default();
        if !deps.is_empty() {
            println!("  detected binary deps: {}", deps.join(", "));
        }
        deps
    } else {
        Vec::new()
    };

    let ctx = crate::plugins::BuildContext {
        cfg: effective_cfg,
        job,
        binary_dir: &binary_dir,
        staging_root: &staging_root,
        license,
        debian_version: &debian_version,
        build_version: &args.build_version,
        mtime,
        sign_key: sign_key.as_deref(),
        sign_key_id: &sign_key_id,
        sign_passphrase: sign_passphrase.as_deref(),
        sign_method: &sign_method,
        detected_deps,
    };
    let built = plugin.build(&ctx)?;
    let final_path = args.output.join(built.file_name().unwrap());
    std::fs::copy(&built, &final_path)
        .with_context(|| format!("copying {} to {}", built.display(), final_path.display()))?;

    // 5. Optional lintian validation (deb only; rpm has no lintian).
    if args.lintian {
        if format != "deb" {
            eprintln!("    ⚠ --lintian is only applicable to deb packages; skipping for {format}");
        } else {
            let suppress: Vec<String> = args
                .lintian_suppress
                .as_deref()
                .map(|s| {
                    s.split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            let report = lx_lib::lintian::run(&final_path, args.lintian_pedantic, &suppress)?;
            print_lintian_report(&final_path, &report);
            if lx_lib::lintian::should_fail(&report, args.lintian_fail_on_warnings) {
                bail!("lintian failed for {}", final_path.display());
            }
        }
    }

    // 6. Post-build signing. The `Signer` plugin for (format, method)
    // decides how: detached backends (`gpg-detach`) write a sibling
    // signature now; embedded backends (`rpm-pgp`, `deb-debsign`) already
    // signed the artifact while the packager built it.
    if let Some(key) = cfg.effective_sign_key(args.sign_key.as_deref()) {
        let sign_type = cfg.effective_sign_type();
        let ctx = crate::plugins::signer::SignContext {
            key_file: &key,
            key_id: &sign_key_id,
            passphrase: sign_passphrase.as_deref(),
            sign_type: &sign_type,
            cert_file: &cfg.signature.cert_file,
        };
        match crate::plugins::signer::apply_post_build(&format, &sign_method, &final_path, &ctx)? {
            crate::plugins::signer::PostBuild::Embedded(name) => {
                println!(
                    "    ✓ signed {} ({}: embedded by the packager)",
                    final_path.display(),
                    name
                );
            }
            crate::plugins::signer::PostBuild::Detached { signer, path } => {
                println!(
                    "    ✓ signed {} -> {} ({})",
                    final_path.display(),
                    path.display(),
                    signer
                );
            }
            crate::plugins::signer::PostBuild::Unsupported => {
                eprintln!(
                    "    ⚠ no signer supports format '{format}' method '{sign_method}'; artifact left unsigned"
                );
            }
        }
    }

    if args.verify {
        match &verify_against {
            Some(cached) => {
                let prior = lx_lib::checksum::sha256_file(cached)?;
                let fresh = lx_lib::checksum::sha256_file(&final_path)?;
                if prior != fresh {
                    bail!(
                        "--verify: build is not reproducible for {} (cached {prior}, rebuilt {fresh})",
                        final_path.display()
                    );
                }
                println!("    ✓ verified reproducible: {}", final_path.display());
            }
            None => println!(
                "    (verify) no prior cached build for this recipe; {} is now the baseline",
                final_path.display()
            ),
        }
    }

    if let Some((dir, key)) = &artifact_cache_key {
        let cache = lx_lib::cache::ArtifactCache::new(dir.clone())?;
        let name = final_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        cache.put(key, &final_path, name)?;
    }

    Ok(final_path)
}

/// Signing passphrase resolution shared by both formats:
/// `$LX_SIGN_PASSPHRASE`, falling back to `$NFPM_PASSPHRASE` (nfpm parity).
pub(crate) fn resolve_sign_passphrase() -> Option<String> {
    for var in ["LX_SIGN_PASSPHRASE", "NFPM_PASSPHRASE"] {
        if let Ok(v) = std::env::var(var) {
            if !v.trim().is_empty() {
                return Some(v);
            }
        }
    }
    None
}

fn print_lintian_report(deb: &Path, report: &lx_lib::lintian::LintianReport) {
    let name = deb
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    if report.errors > 0 {
        println!(
            "    ❌ lintian {name}: {} error(s), {} warning(s), {} info",
            report.errors, report.warnings, report.info
        );
    } else if report.warnings > 0 {
        println!(
            "    ⚠ lintian {name}: {} warning(s), {} info",
            report.warnings, report.info
        );
    } else if report.info > 0 {
        println!(
            "    ℹ lintian {name}: {} informational message(s)",
            report.info
        );
    } else {
        println!("    ✅ lintian {name}: no issues");
    }
    for line in &report.lines {
        println!("      {line}");
    }
}

/// How a downloaded asset's integrity was (or wasn't) established --
/// recorded per-asset into build-summary.json's `provenance` array (see
/// `build_one`) so a build can be audited after the fact.
#[derive(Clone, Copy)]
pub enum VerifyMethod {
    /// Matched a `--pinned-metadata` vet-time provenance pin.
    Pinned,
    /// Matched a `package.lock` entry for this architecture.
    Locked,
    /// Matched an inline checksum the provider published (GitHub/Gitea asset
    /// `digest`, SourceForge feed hash).
    Inline,
    /// Matched a live `.sha256`/`.sha256sum` sidecar next to the asset.
    Sidecar,
    /// No pin and no sidecar existed; proceeded anyway because
    /// `--allow-unverified` was passed.
    UnverifiedAllowed,
    /// Verification was skipped entirely via `--no-verify`.
    SkippedNoVerify,
}

impl VerifyMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            VerifyMethod::Pinned => "pinned",
            VerifyMethod::Locked => "locked",
            VerifyMethod::Inline => "inline",
            VerifyMethod::Sidecar => "sidecar",
            VerifyMethod::UnverifiedAllowed => "unverified (--allow-unverified)",
            VerifyMethod::SkippedNoVerify => "skipped (--no-verify)",
        }
    }
}

/// Live sidecar checksum verification (probe-and-verify logic shared with
/// `debs.rs` via `lx_lib::checksum::check_sidecar`). Most real-world
/// GitHub releases don't publish a checksum sidecar, so without
/// `--allow-unverified` this fails the build rather than silently
/// proceeding on an unverified download that's about to become an
/// installable, often sudo-installed .deb -- a missing sidecar used to
/// just print a warning and continue, which meant the *default* path for
/// most repos had zero integrity verification with only a console line as
/// evidence.
fn verify_sidecar_or_require_flag(
    client: &dyn lx_lib::checksum::RawGetter,
    asset: &Asset,
    path: &Path,
    allow_unverified: bool,
) -> Result<VerifyMethod> {
    use lx_lib::checksum::SidecarCheck;
    // Provider-published inline checksums first (no network round-trip), then
    // a live sidecar probe.
    if let Some(algo) = lx_lib::checksum::check_inline(&asset.checksums, path)? {
        println!("    ✓ checksum verified ({algo})");
        return Ok(VerifyMethod::Inline);
    }
    match lx_lib::checksum::check_sidecar(client, &asset.browser_download_url, &asset.name, path)? {
        SidecarCheck::Verified => {
            println!("    ✓ checksum verified");
            Ok(VerifyMethod::Sidecar)
        }
        SidecarCheck::NotFound if allow_unverified => {
            eprintln!(
                "    ⚠ (no sidecar checksum for '{}': no sidecar checksum found; proceeding \
                 unverified per --allow-unverified)",
                asset.name
            );
            Ok(VerifyMethod::UnverifiedAllowed)
        }
        SidecarCheck::NotFound => Err(anyhow!(
            "no checksum verification available for '{}': no sidecar checksum found. Pass \
             --allow-unverified to build anyway, or supply --pinned-metadata or a checksum \
             sidecar.",
            asset.name
        )),
    }
}

fn verify_sidecar_or_require_flag_source(
    source: &dyn crate::plugins::forge::ForgeSource,
    token: Option<&str>,
    asset: &Asset,
    path: &Path,
    allow_unverified: bool,
) -> Result<VerifyMethod> {
    struct Getter<'a> {
        source: &'a dyn crate::plugins::forge::ForgeSource,
        token: Option<&'a str>,
    }
    impl lx_lib::checksum::RawGetter for Getter<'_> {
        fn raw_get(&self, url: &str) -> Result<Box<dyn std::io::Read + Send>> {
            self.source.raw_get(url, self.token)
        }
    }
    verify_sidecar_or_require_flag(&Getter { source, token }, asset, path, allow_unverified)
}

pub fn extract(archive: &Path, dest: &Path, format: &str) -> Result<()> {
    crate::plugins::artifact::extract(archive, dest, format)
}

pub(crate) fn is_elf(path: &Path) -> Result<bool> {
    let mut f = std::fs::File::open(path)?;
    let mut magic = [0u8; 4];
    if f.read_exact(&mut magic).is_err() {
        return Ok(false);
    }
    Ok(&magic == b"\x7fELF")
}

/// The host's Debian architecture name (`uname -m` mapped to dpkg naming),
/// or None if it can't be determined. Used both to decide whether a target
/// architecture needs emulation, and to resolve `--host`.
pub fn host_arch() -> Option<String> {
    let out = Command::new("uname").arg("-m").output().ok()?;
    let machine = String::from_utf8(out.stdout).ok()?.trim().to_string();
    Some(match machine.as_str() {
        "x86_64" | "amd64" => "amd64".to_string(),
        "aarch64" | "arm64" => "arm64".to_string(),
        "armv7l" | "armhf" | "armv7" => "armhf".to_string(),
        "ppc64le" | "ppc64el" => "ppc64el".to_string(),
        "s390x" => "s390x".to_string(),
        "riscv64" => "riscv64".to_string(),
        "loongarch64" | "loong64" => "loong64".to_string(),
        "i686" | "i386" | "i586" => "i386".to_string(),
        other => other.to_string(),
    })
}

fn human_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

/// Write a failure record into `./failed-build-logs/` (bash action parity:
/// that dir is uploaded as a workflow artifact on failure). Captures the
/// error plus any `.telemetry/` logs present. Best-effort; never fails.
pub fn write_failed_build_log(error: &str) {
    let dir = std::path::Path::new("failed-build-logs");
    let _ = std::fs::create_dir_all(dir);
    let ts = jiff::Timestamp::now()
        .strftime("%Y-%m-%d %H:%M:%S")
        .to_string();
    let _ = std::fs::write(
        dir.join("build-failure.log"),
        format!("{ts} - Build failure: {error}\n"),
    );
    for name in ["stages.log", "failures.log", "metrics.json"] {
        let src = std::path::Path::new(".telemetry").join(name);
        if src.exists() {
            let _ = std::fs::copy(&src, dir.join(name));
        }
    }
}
