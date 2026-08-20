use anyhow::{anyhow, bail, Context, Result};
use clap::Args;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::config::PackageConfig;
use crate::discovery::{config_from_release, guess_format, match_assets};
use lpt_lib::github::{Asset, GitHubClient};

#[derive(Debug, Clone, Args)]
pub struct BuildArgs {
    /// Path to package.yaml
    #[arg(default_value = "package.yaml")]
    pub config: PathBuf,

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
    /// `uname -m`), skipping QEMU emulation entirely. Conflicts with
    /// --architectures; pass one or the other.
    #[arg(long)]
    pub host: bool,

    /// Restrict to specific distributions (comma-separated).
    #[arg(long)]
    pub distributions: Option<String>,

    /// Directory to write resulting .deb files into.
    #[arg(long, default_value = "dist")]
    pub output: PathBuf,

    /// Skip checksum verification (not recommended).
    #[arg(long)]
    pub no_verify: bool,

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
    /// mirroring the action's build_source_packages. Runs dpkg-source in a
    /// Debian container (the host may be non-Debian).
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

    /// Show a live inline progress bar while building (action's
    /// progress.sh). Only rendered on an interactive terminal.
    #[arg(long)]
    pub progress: bool,

    /// Keep intermediate files (downloaded assets, staging dirs).
    #[arg(long)]
    pub keep: bool,
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

pub fn run(args: BuildArgs, token: Option<&str>) -> Result<()> {
    let build_start = std::time::Instant::now();
    // Dynamic parallelism (action ci-optimization.sh + resource-pool.sh
    // parity): when --max-parallel is unset (0 means auto) or exceeds what
    // the system can sustain, derive an optimal value from detected
    // resources. We need an owned value because build_jobs reads it by ref.
    let effective_parallel = lpt_lib::optimize::effective_max_parallel(args.max_parallel);
    let mut args = args;
    args.max_parallel = effective_parallel;

    // A bare GitHub URL in place of a package.yaml path triggers a fully
    // zero-config build: no manual patterns, every supported architecture,
    // and source packages included (there's no config file to opt out via,
    // so the most useful default wins).
    let mut cfg = match parse_github_url(&args.config.to_string_lossy()) {
        Some(github_repo) => {
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
        }
        None => PackageConfig::load(&args.config)?,
    };
    if let Some(v) = &args.version {
        cfg.version = v.clone();
    }

    let client =
        GitHubClient::with_cache(token.map(|s| s.to_string()), args.api_cache_dir.clone())?;
    let (owner, repo) = crate::discovery::split_repo(&cfg.github_repo)?;

    // Resolve the release: pinned version in config, else a tag/version, else latest.
    let release = if !cfg.version.is_empty() {
        match client.release_by_tag(owner, repo, &cfg.version) {
            Ok(r) => r,
            Err(e) => {
                suggest_versions(&client, owner, repo, &cfg.version);
                return Err(e);
            }
        }
    } else if cfg.has_manual_patterns() {
        client.latest_release(owner, repo)?
    } else {
        // Zero-config mode: auto-discover from the latest release.
        client.latest_release(owner, repo)?
    };

    // Fetch the upstream license once (not per-architecture), mirroring the
    // action's fetch_upstream_license + dual-license detection. Best-effort:
    // on failure, fall back to the config's license_spdx and continue.
    let license = fetch_upstream_license(&client, &cfg).unwrap_or(None);

    if args.host && args.architectures.is_some() {
        bail!("--host conflicts with --architectures; pass one or the other");
    }

    // Determine the effective build matrix.
    let mut dists = cfg.effective_distributions();
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
    if args.host {
        let detected = host_arch().ok_or_else(|| {
            anyhow!("could not detect this machine's architecture from `uname -m`; use --architectures instead")
        })?;
        println!("Building for host architecture: {detected} (--host; no QEMU needed)");
        archs = vec![detected];
    }

    // Resolve one asset per architecture.
    let arch_assets = if cfg.has_manual_patterns() {
        resolve_manual(&cfg, &release)?
    } else {
        let auto = config_from_release(&cfg.github_repo, &release)?;
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
                    if !cfg.arch_supported_for_dist(arch, dist) {
                        println!(
                            "⚠️  Skipping {dist} for {arch}: architecture not supported in this distribution"
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
        for arch in arch_assets.iter().map(|(a, _)| a.as_str()) {
            check_qemu_for(arch);
        }
        return Ok(());
    }

    // Telemetry (action's TELEMETRY_ENABLED, default false) + live progress
    // (action's progress.sh). Both best-effort.
    let telemetry = lpt_lib::telemetry::Telemetry::new(args.telemetry);
    telemetry.init()?;
    telemetry.record_stage("build_initialization")?;

    let progress = if args.progress && lpt_lib::progress::stdout_is_tty() {
        Some(lpt_lib::progress::Progress::new(
            arch_assets.len(),
            &release.tag_name,
            &cfg.package_name,
            std::path::PathBuf::from("/tmp/build_progress.json"),
            true,
        )?)
    } else {
        None
    };

    build_jobs(
        &args,
        &cfg,
        &jobs,
        license.as_ref(),
        token.map(|s| s.to_string()),
        progress.as_ref(),
        &telemetry,
    )?;

    telemetry.record_stage_complete("build_completion", "success")?;
    telemetry.finalize(build_start.elapsed().as_secs())?;

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
                distributions: dists.clone(),
                max_parallel: args.max_parallel,
                start: build_start,
                telemetry: telemetry.summary_json(),
            },
        )?;
    }

    if args.source {
        crate::source::generate(
            &args.output,
            &crate::source::Pkg {
                name: cfg.package_name.clone(),
                github_repo: cfg.github_repo.clone(),
                description: cfg.effective_description(),
                maintainer: cfg.effective_maintainer(),
                version: release.tag_name.clone(),
                build_version: args.build_version.clone(),
                license_spdx: license
                    .as_ref()
                    .map(|l| l.spdx.clone())
                    .unwrap_or_else(|| "NOASSERTION".to_string()),
            },
        )?;
    }
    Ok(())
}

/// Print the latest few releases as suggestions when the requested version
/// was not found (mirrors the action's version-miss hinting).
fn suggest_versions(client: &GitHubClient, owner: &str, repo: &str, wanted: &str) {
    eprintln!("Version '{wanted}' not found for {owner}/{repo}.");
    match client.releases(owner, repo, 5) {
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
            eprintln!(
                "  No recent releases could be listed; check https://github.com/{owner}/{repo}/releases"
            );
        }
    }
}

/// Fetch the upstream license + detect the common Rust dual
/// MIT/Apache-2.0 convention, mirroring the action's
/// `fetch_upstream_license`/`detect_multiple_license`. Best-effort.
fn fetch_upstream_license(
    client: &GitHubClient,
    cfg: &PackageConfig,
) -> Result<Option<lpt_lib::github::RepoLicense>> {
    let (owner, repo) = crate::discovery::split_repo(&cfg.github_repo)?;
    let mut license = client.repo_license(owner, repo)?;

    // Dual-license detection: if the repo root has LICENSE-APACHE and
    // LICENSE-MIT, prefer that over GitHub's single primary license.
    if let Ok(root) = client.repo_root(owner, repo) {
        let has_apache = root.iter().any(|n| n == "LICENSE-APACHE");
        let has_mit = root.iter().any(|n| n == "LICENSE-MIT");
        if has_apache && has_mit {
            let apache = client
                .repo_file_text(owner, repo, "LICENSE-APACHE")?
                .unwrap_or_default();
            let mit = client
                .repo_file_text(owner, repo, "LICENSE-MIT")?
                .unwrap_or_default();
            if !apache.is_empty() && !mit.is_empty() {
                license = Some(lpt_lib::github::RepoLicense {
                    spdx: "Apache-2.0 or MIT".to_string(),
                    text: Some(format!(
                        "Dual-licensed under either of:\n\n=== Apache License 2.0 ===\n\n{apache}\n\n=== MIT License ===\n\n{mit}"
                    )),
                });
            }
        }
    }

    if license.is_none() {
        // Fall back to the config's explicitly-declared SPDX id (if any).
        if !cfg.license_spdx.is_empty() {
            license = Some(lpt_lib::github::RepoLicense {
                spdx: cfg.license_spdx.clone(),
                text: None,
            });
        }
    }

    if let Some(l) = &license {
        println!(
            "license: {} ({})",
            l.spdx,
            if l.text.is_some() {
                "full text"
            } else {
                "no text"
            }
        );
    } else {
        println!("license: NOASSERTION (no machine-detectable license upstream)");
    }
    Ok(license)
}

/// Map each requested Debian architecture to a concrete release asset using
/// the config's pinned release_pattern.
fn resolve_manual(
    cfg: &PackageConfig,
    release: &lpt_lib::github::Release,
) -> Result<std::collections::HashMap<String, Asset>> {
    let mut out = std::collections::HashMap::new();
    for (arch, acfg) in cfg.architectures.patterns() {
        let pattern = &acfg.release_pattern;
        if pattern.is_empty() {
            continue;
        }
        let expanded = pattern.replace("{version}", &release.tag_name);
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
    let matched = match_assets(release);
    for m in matched {
        out.entry(m.arch)
            .or_insert(asset_from_name(release, &m.asset));
    }
    Ok(out)
}

/// Parse a bare `https://github.com/<owner>/<repo>` URL into `"owner/repo"`,
/// ignoring any further path (a `.git` suffix, `/releases`, a tag, etc.).
/// Returns `None` for anything that isn't a github.com URL, so callers can
/// fall through to treating the argument as a package.yaml path.
fn parse_github_url(s: &str) -> Option<String> {
    let rest = s
        .strip_prefix("https://github.com/")
        .or_else(|| s.strip_prefix("http://github.com/"))?;
    let mut parts = rest.trim_end_matches('/').splitn(3, '/');
    let owner = parts.next()?;
    let repo = parts.next()?.trim_end_matches(".git");
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some(format!("{owner}/{repo}"))
}

fn asset_from_name(release: &lpt_lib::github::Release, name: &str) -> Asset {
    release
        .assets
        .iter()
        .find(|a| a.name == name)
        .cloned()
        .unwrap_or_else(|| Asset {
            name: name.to_string(),
            size: None,
            browser_download_url: String::new(),
        })
}

fn build_jobs(
    args: &BuildArgs,
    cfg: &PackageConfig,
    jobs: &[ResolvedJob],
    license: Option<&lpt_lib::github::RepoLicense>,
    token: Option<String>,
    progress: Option<&lpt_lib::progress::Progress>,
    telemetry: &lpt_lib::telemetry::Telemetry,
) -> Result<()> {
    std::fs::create_dir_all(&args.output)?;

    let pin = match &args.pinned_metadata {
        Some(p) => Some(lpt_lib::checksum::PinnedMetadata::load(p)?),
        None => None,
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
    let license = license.cloned();
    let progress = progress.cloned();

    let workers = args.max_parallel.max(1);
    let mut handles = Vec::new();
    let failures = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

    // A simple bounded pool: each worker grabs the next arch group until
    // none remain, so runtime parallelism is capped at `workers`.
    let next = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    for _ in 0..workers {
        let groups = groups.clone();
        let next = std::sync::Arc::clone(&next);
        let failures = std::sync::Arc::clone(&failures);
        let tmp = tmp.path().to_path_buf();
        let args = args.clone();
        let cfg = cfg.clone();
        let license = license.clone();
        let token = token.clone();
        let pin = pin.clone();
        let progress = progress.clone();
        let telemetry = telemetry.clone();
        handles.push(std::thread::spawn(move || {
            let client = GitHubClient::with_cache(token, args.api_cache_dir.clone())
                .expect("failed to create GitHub client");
            loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let Some(group) = groups.get(i) else { break };
                let arch = &group[0].arch;
                check_qemu_for(arch);
                let _ = progress.as_ref().map(|p| p.set_arch(arch, "running"));
                // Fresh download map per architecture worker (distinct assets).
                let mut downloaded: std::collections::HashMap<String, PathBuf> =
                    std::collections::HashMap::new();
                let mut group_ok = true;
                for job in group {
                    let result = build_one(
                        &args,
                        &cfg,
                        &client,
                        job,
                        &tmp,
                        &mut downloaded,
                        license.as_ref(),
                        pin.as_ref(),
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
                    lpt_lib::progress::Outcome::Completed
                } else {
                    lpt_lib::progress::Outcome::Failed
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
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn build_one(
    args: &BuildArgs,
    cfg: &PackageConfig,
    client: &GitHubClient,
    job: &ResolvedJob,
    tmp: &Path,
    downloaded: &mut std::collections::HashMap<String, PathBuf>,
    license: Option<&lpt_lib::github::RepoLicense>,
    pin: Option<&lpt_lib::checksum::PinnedMetadata>,
) -> Result<PathBuf> {
    // 1. Download the asset (once per asset name).
    let asset_path = match downloaded.get(&job.asset.name) {
        Some(p) => p.clone(),
        None => {
            let path = tmp.join(&job.asset.name);
            println!(
                "  ↓ {} ({})",
                job.asset.name,
                human_size(job.asset.size.unwrap_or(0))
            );
            match &args.cache_dir {
                Some(dir) => {
                    // Expected checksum from the pin (if any) is passed into
                    // the cache so a cached entry is validated too.
                    let expected = pin.and_then(|p| p.sha256_for(&job.tag, &job.asset.name));
                    let cache = lpt_lib::cache::DownloadCache::new(dir.clone())?;
                    cache.fetch(
                        &job.asset.browser_download_url,
                        &path,
                        expected.as_deref(),
                        &|url, out| {
                            let asset_for_dl = Asset {
                                browser_download_url: url.to_string(),
                                ..job.asset.clone()
                            };
                            download(client, &asset_for_dl, out)
                        },
                    )?;
                }
                None => download(client, &job.asset, &path)?,
            }
            if !args.no_verify {
                // Prefer the vet-time provenance pin over the live sidecar,
                // mirroring the action's build.sh precedence.
                if let Some(pin) = pin {
                    if let Some(expected) = pin.sha256_for(&job.tag, &job.asset.name) {
                        lpt_lib::checksum::verify_sha256(&path, &expected)?;
                        println!("    ✓ verified against pinned sha256:{}", &expected[..12]);
                    } else {
                        eprintln!(
                            "    (no vetted pin for '{}' @ {}; falling back to live checksum)",
                            job.asset.name, job.tag
                        );
                        verify_sidecar_or_warn(client, &job.asset, &path);
                    }
                } else {
                    // No pin file supplied: try the live sidecar, else warn.
                    verify_sidecar_or_warn(client, &job.asset, &path);
                }
            }
            downloaded.insert(job.asset.name.clone(), path.clone());
            path
        }
    };

    // 2. Extract.
    let extract_dir = tmp.join(format!("{}-{}-extract", job.arch, job.dist));
    extract(&asset_path, &extract_dir, &cfg.artifact_format)?;

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

    // 4. Build the .deb via Docker, mirroring the action's Dockerfile.
    let (deb, _ctx) = build_deb_via_docker(args, cfg, job, &binary_dir, license)?;
    let final_path = args.output.join(deb.file_name().unwrap());
    std::fs::copy(&deb, &final_path)
        .with_context(|| format!("copying {} to {}", deb.display(), final_path.display()))?;

    // 5. Optional lintian validation (mirrors the action's run_lintian_check).
    if args.lintian {
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
        let report = lpt_lib::lintian::run(&final_path, args.lintian_pedantic, &suppress)?;
        print_lintian_report(&final_path, &report);
        if lpt_lib::lintian::should_fail(&report, args.lintian_fail_on_warnings) {
            bail!("lintian failed for {}", final_path.display());
        }
    }

    Ok(final_path)
}

fn print_lintian_report(deb: &Path, report: &lpt_lib::lintian::LintianReport) {
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

fn download(client: &GitHubClient, asset: &Asset, dest: &Path) -> Result<()> {
    if asset.browser_download_url.is_empty() {
        bail!("asset '{}' has no download URL", asset.name);
    }
    let resp = client
        .raw_get(&asset.browser_download_url)
        .context("asset download failed")?;
    let mut body = resp;
    let mut file = std::fs::File::create(dest)?;
    std::io::copy(&mut body, &mut file)?;
    Ok(())
}

fn verify_sidecar(client: &GitHubClient, asset: &Asset, path: &Path) -> Result<()> {
    for suffix in [".sha256", ".sha256sum"] {
        let sidecar_url = format!("{}{}", asset.browser_download_url, suffix);
        match client.raw_get(&sidecar_url) {
            Ok(mut resp) => {
                let mut text = String::new();
                resp.read_to_string(&mut text)?;
                let map = lpt_lib::checksum::parse_checksum_file(&text)?;
                let fname = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default();
                if let Some(expected) = map.get(fname) {
                    lpt_lib::checksum::verify_sha256(path, expected)?;
                    println!("    ✓ checksum verified");
                    return Ok(());
                }
            }
            Err(_) => continue,
        }
    }
    Err(anyhow!("no sidecar checksum found"))
}

/// Best-effort live sidecar verification: warn (don't fail) when no sidecar
/// exists, matching the action's optional live verification.
fn verify_sidecar_or_warn(client: &GitHubClient, asset: &Asset, path: &Path) {
    if let Err(e) = verify_sidecar(client, asset, path) {
        eprintln!(
            "    (no sidecar checksum for '{}': {e}; skipping verification)",
            asset.name
        );
    }
}

fn extract(archive: &Path, dest: &Path, format: &str) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    match format {
        "tar.gz" | "tgz" => {
            let f = std::fs::File::open(archive)?;
            let gz = flate2::read::GzDecoder::new(f);
            let mut tar = tar::Archive::new(gz);
            tar.unpack(dest)
                .with_context(|| format!("failed to extract '{}'", archive.display()))?;
        }
        "zip" => bail!("zip extraction not yet supported; use tar.gz or raw"),
        "raw" => {
            let name = archive
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("binary");
            std::fs::copy(archive, dest.join(name))?;
        }
        other => bail!("unsupported artifact_format '{other}'"),
    }
    Ok(())
}

/// Recursively copy `src`'s contents into `dst` (`dst` is created if
/// missing), preserving symlinks rather than following them -- matching the
/// action's `cp -a`. Used by `bundle: true` packaging, which needs the
/// whole extracted tree preserved (e.g. a `bin/`+`lib/` layout, including
/// any versioned-library symlinks like `libfoo.so -> libfoo.so.1`), not
/// just its top-level files.
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dest_path = dst.join(entry.file_name());
        if ty.is_symlink() {
            let target = std::fs::read_link(entry.path())?;
            std::os::unix::fs::symlink(&target, &dest_path)?;
        } else if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &dest_path)?;
        } else if ty.is_file() {
            std::fs::copy(entry.path(), &dest_path)?;
        }
    }
    Ok(())
}

fn build_deb_via_docker(
    args: &BuildArgs,
    cfg: &PackageConfig,
    job: &ResolvedJob,
    binary_dir: &Path,
    license: Option<&lpt_lib::github::RepoLicense>,
) -> Result<(PathBuf, tempfile::TempDir)> {
    check_docker()?;

    // Stage the Docker build context: output/DEBIAN/control + copyright + changelog.
    let ctx = tempfile::tempdir()?;
    let output_dir = ctx.path().join("output");
    let debian_dir = output_dir.join("DEBIAN");
    let doc_dir = output_dir
        .join("usr")
        .join("share")
        .join("doc")
        .join(&cfg.package_name);
    std::fs::create_dir_all(&debian_dir)?;
    std::fs::create_dir_all(&doc_dir)?;

    // Debian policy requires the Version field to start with a digit, but
    // upstream tags commonly carry a non-digit prefix (e.g. "v0.23.5" or
    // bun's "bun-v1.3.14"). Strip any leading run of non-digit characters,
    // mirroring the action's `sed -E 's/^[^0-9]*//'`.
    let version = if cfg.version.is_empty() {
        &job.tag
    } else {
        &cfg.version
    };
    let debian_version: String = version
        .trim_start_matches(|c: char| !c.is_ascii_digit())
        .to_string();
    let full_version = format!(
        "{debian_version}-{}+{dist}_{arch}",
        args.build_version,
        dist = job.dist,
        arch = job.arch
    );
    let deb_name = format!("{}_{}.deb", cfg.package_name, full_version);

    write_control(&debian_dir, cfg, job, &debian_version, &args.build_version)?;
    write_changelog(&output_dir, cfg, job, &debian_version, &args.build_version)?;
    write_copyright(&output_dir, cfg, license, job.published_at)?;

    // Copy the extracted binaries into the build context so the Dockerfile's
    // COPY (which must stay inside the context) can reach them. `bundle`
    // needs the whole tree (subdirectories included, e.g. a `bin/` and
    // `lib/` layout); the default flat mode only needs top-level files.
    let bin_in_ctx = ctx.path().join("binary-source");
    if cfg.bundle {
        copy_dir_recursive(binary_dir, &bin_in_ctx)?;
    } else {
        std::fs::create_dir_all(&bin_in_ctx)?;
        for entry in std::fs::read_dir(binary_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                std::fs::copy(entry.path(), bin_in_ctx.join(entry.file_name()))?;
            }
        }
    }

    let out_dir = ctx.path().join("out");
    std::fs::create_dir_all(&out_dir)?;

    let image_tag = format!("lpt-{}-{}-{}", cfg.package_name, job.dist, job.arch);
    let dockerfile = render_dockerfile(cfg, job, &full_version, &deb_name);

    let df_path = ctx.path().join("Dockerfile");
    std::fs::write(&df_path, dockerfile)?;

    let status = Command::new("docker")
        .args([
            "build",
            "-t",
            &image_tag,
            "--build-arg",
            "BINARY_SOURCE=binary-source",
            "-f",
            df_path.to_str().unwrap(),
            ".",
        ])
        .current_dir(ctx.path())
        .stdout(Stdio::null())
        .status()
        .context("failed to run docker build")?;
    if !status.success() {
        bail!("docker build failed for {}-{}", job.dist, job.arch);
    }

    // Extract the .deb from the scratch image. The container name must be
    // unique per (pid, job): parallel arch workers share the process pid and
    // would otherwise race on the same name.
    let deb_dest = out_dir.join(&deb_name);
    let container = format!("lpt-{}-{}-{}", std::process::id(), job.dist, job.arch);
    let _ = Command::new("docker")
        .args(["rm", "-f", &container])
        .status();
    let create = Command::new("docker")
        .args(["create", "--name", &container, &image_tag, "/"])
        .output()
        .context("failed to create docker container")?;
    if !create.status.success() {
        bail!(
            "docker create failed: {}",
            String::from_utf8_lossy(&create.stderr)
        );
    }
    let id = String::from_utf8_lossy(&create.stdout).trim().to_string();

    let cp = Command::new("docker")
        .args([
            "cp",
            &format!("{id}:{deb_name}"),
            deb_dest.to_str().unwrap(),
        ])
        .output()
        .context("failed to run docker cp")?;
    let _ = Command::new("docker")
        .args(["rm", "-f", &container])
        .status();
    if !cp.status.success() {
        bail!(
            "failed to copy built .deb out of container (expected '/{deb_name}'): {}",
            String::from_utf8_lossy(&cp.stderr)
        );
    }
    if !deb_dest.is_file() {
        bail!(
            "docker cp reported success but '{deb_name}' missing at {}",
            deb_dest.display()
        );
    }
    Ok((deb_dest, ctx))
}

fn check_docker() -> Result<()> {
    let status = Command::new("docker")
        .arg("version")
        .stdout(Stdio::null())
        .status();
    match status {
        Ok(s) if s.success() => Ok(()),
        _ => bail!("docker is required but not available. Install Docker, or use `lpt build --dry-run` to validate without building."),
    }
}

/// The host's Debian architecture name (`uname -m` mapped to dpkg naming),
/// or None if it can't be determined. Used both to decide whether a target
/// architecture needs QEMU emulation, and to resolve `--host`.
fn host_arch() -> Option<String> {
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

/// Warn (once per architecture) when building for an architecture other than
/// the host's: Docker needs QEMU/binfmt registration for the foreign image,
/// mirroring the action's `docker/setup-qemu-action` + binfmt diagnostics.
fn check_qemu_for(arch: &str) {
    let Some(host) = host_arch() else { return };
    if host == arch {
        return;
    }
    // binfmt_misc handlers for foreign archs are named qemu-*.
    let registered = std::fs::read_dir("/proc/sys/fs/binfmt_misc")
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .filter(|n| n.starts_with("qemu-"))
                .count()
        })
        .unwrap_or(0);
    if registered > 0 {
        return;
    }
    eprintln!(
        "  ⚠️  building for {arch} on {host}: QEMU emulation appears unregistered. \
         Foreign-arch builds may fail; run `docker run --privileged --rm tonistiigi/binfmt --install all` \
         (or the action's docker/setup-qemu-action) first."
    );
}

fn render_dockerfile(
    cfg: &PackageConfig,
    job: &ResolvedJob,
    _full_version: &str,
    deb_name: &str,
) -> String {
    let binary_rename = if cfg.binary_rename.is_empty() {
        "".to_string()
    } else {
        format!("ARG BINARY_RENAME={}\nRUN if [ -n \"$BINARY_RENAME\" ]; then count=$(find /output/usr/bin -maxdepth 1 -type f | wc -l); if [ \"$count\" = \"1\" ]; then f=$(find /output/usr/bin -maxdepth 1 -type f); mv \"$f\" \"/output/usr/bin/$BINARY_RENAME\"; fi; fi", cfg.binary_rename)
    };
    // Bundle-mode executable discovery covers two shapes, matching upstream:
    // a bin/ subdirectory (zed.app/{bin,lib,libexec,share}, an FHS-like
    // tree), and executables sitting directly at the bundle root as
    // siblings of the data directories they need (pnpm's Node
    // single-executable-application binary needs its own dist/ alongside
    // it, with no bin/lib/libexec structure at all) -- so both loops run
    // unconditionally rather than picking one shape.
    //
    // A trailing guard fails the build loudly if no executables ended up in
    // /usr/bin at all (wrong binary_path, empty archive, bundle tree with
    // no ELF anywhere), instead of silently shipping an empty package.
    let install = if cfg.bundle {
        format!(
            r#"RUN mkdir -p "/output/usr/lib/{pkg}" "/output/usr/share/doc/{pkg}" /output/DEBIAN /output/usr/bin
COPY ${{BINARY_SOURCE}}/ "/output/usr/lib/{pkg}/"
RUN for f in "/output/usr/lib/{pkg}/bin"/*; do [ -d "/output/usr/lib/{pkg}/bin" ] || break; [ -f "$f" ] || continue; file -b "$f" | grep -q "^ELF " || continue; chmod +x "$f"; ln -s "/usr/lib/{pkg}/bin/$(basename "$f")" "/output/usr/bin/$(basename "$f")"; done; for f in "/output/usr/lib/{pkg}"/*; do [ -f "$f" ] || continue; file -b "$f" | grep -q "^ELF " || continue; chmod +x "$f"; ln -s "/usr/lib/{pkg}/$(basename "$f")" "/output/usr/bin/$(basename "$f")"; done; [ -n "$(ls -A /output/usr/bin 2>/dev/null)" ] || (echo "ERROR: no executables landed in /usr/bin (bundle=true) - check binary_path/bundle config" >&2; exit 1)"#,
            pkg = cfg.package_name,
        )
    } else {
        format!(
            r#"RUN mkdir -p /output/usr/bin "/output/usr/share/doc/{pkg}" /output/DEBIAN
COPY ${{BINARY_SOURCE}}/ /tmp/binary-source/
RUN for f in /tmp/binary-source/*; do [ -f "$f" ] || continue; file -b "$f" | grep -q "^ELF " && cp "$f" /output/usr/bin/ || true; done && chmod +x /output/usr/bin/* 2>/dev/null; rm -rf /tmp/binary-source; [ -n "$(ls -A /output/usr/bin 2>/dev/null)" ] || (echo "ERROR: no executables landed in /usr/bin - check binary_path config" >&2; exit 1)"#,
            pkg = cfg.package_name,
        )
    };
    format!(
        r#"ARG DEBIAN_DIST={dist}
FROM debian:${{DEBIAN_DIST}}
ARG BINARY_SOURCE
ENV DIST={dist} SUPPORTED_ARCHITECTURES={arch}
RUN apt-get update && apt-get install -y file gzip gettext-base && rm -rf /var/lib/apt/lists/*
{install}
{binary_rename}
COPY output/DEBIAN/control /tmp/control.template
COPY output/copyright /tmp/copyright.template
COPY output/changelog.Debian /tmp/changelog.template
RUN envsubst '${{PACKAGE_NAME}} ${{FULL_VERSION}} ${{DIST}} ${{VERSION}}' < /tmp/control.template > /output/DEBIAN/control
RUN envsubst '${{PACKAGE_NAME}} ${{FULL_VERSION}} ${{DIST}} ${{VERSION}}' < /tmp/changelog.template | gzip -9 > "/output/usr/share/doc/{pkg}/changelog.Debian.gz"
RUN YEAR=$(date +%Y) envsubst '${{PACKAGE_NAME}} ${{GITHUB_REPO}} ${{YEAR}} ${{LICENSE}}' < /tmp/copyright.template > "/output/usr/share/doc/{pkg}/copyright"
RUN rm -f /tmp/control.template /tmp/changelog.template /tmp/copyright.template
RUN dpkg-deb --build /output "/{deb_name}"
FROM scratch
COPY --from=0 "/{deb_name}" /
"#,
        dist = job.dist,
        arch = job.arch,
        pkg = cfg.package_name,
        install = install,
        binary_rename = binary_rename,
        deb_name = deb_name,
    )
}

fn write_control(
    debian_dir: &Path,
    cfg: &PackageConfig,
    job: &ResolvedJob,
    version: &str,
    build_version: &str,
) -> Result<()> {
    let full_version = format!("{version}-{build_version}+{dist}", dist = job.dist);
    let maintainer = if cfg.maintainer.is_empty() {
        "latest-debs maintainers <maintainers@latest-debs.org>".to_string()
    } else {
        cfg.maintainer.clone()
    };
    let desc = if cfg.description.is_empty() {
        format!("{}, packaged from {}", cfg.package_name, cfg.github_repo)
    } else {
        cfg.description.clone()
    };
    // Mirror the action's templates/output/DEBIAN/control: Section, Priority,
    // Homepage, and an extended description line (a single-line Description
    // synopsis without a continuation paragraph trips lintian's
    // extended-description-is-empty). Depends: is appended after
    // Description, matching the action's Dockerfile which `>>`-appends it
    // to the already-rendered control file rather than templating it inline.
    let depends = if cfg.depends.trim().is_empty() {
        String::new()
    } else {
        format!("Depends: {}\n", cfg.depends.trim())
    };
    let control = format!(
        "Section: utils\nPriority: optional\nPackage: {pkg}\nVersion: {full_version}\nArchitecture: {arch}\nMaintainer: {maintainer}\nHomepage: https://github.com/{repo}\nDescription: {desc}\n Packaged from the upstream GitHub release for Debian.\n{depends}",
        pkg = cfg.package_name,
        repo = cfg.github_repo,
        arch = job.arch,
    );
    let mut f = std::fs::File::create(debian_dir.join("control"))?;
    f.write_all(control.as_bytes())?;
    Ok(())
}

fn write_changelog(
    output_dir: &Path,
    cfg: &PackageConfig,
    job: &ResolvedJob,
    version: &str,
    build_version: &str,
) -> Result<()> {
    let full_version = format!("{version}-{build_version}+{dist}", dist = job.dist);
    let changelog = format!(
        "{pkg} ({full_version}) {dist}; urgency=medium\n\n  * New upstream release {version}\n\n -- {maintainer}  {date}\n",
        pkg = cfg.package_name,
        dist = job.dist,
        version = version,
        maintainer = if cfg.maintainer.is_empty() { "latest-debs maintainers <maintainers@latest-debs.org>" } else { &cfg.maintainer },
        date = changelog_date(job.published_at),
    );
    let mut f = std::fs::File::create(output_dir.join("changelog.Debian"))?;
    f.write_all(changelog.as_bytes())?;
    Ok(())
}

/// Timestamp source for reproducible package metadata (changelog date,
/// copyright year): the `SOURCE_DATE_EPOCH` env var if set (the
/// reproducible-builds.org standard, letting operators pin an exact value),
/// else the release's own publish time, else a fixed epoch. Deliberately
/// never wall-clock "now" -- building from build time would make the same
/// release produce different package metadata depending on when it's
/// built, which is exactly what reproducible builds rule out.
fn reproducible_epoch(published_at: Option<i64>) -> i64 {
    if let Ok(v) = std::env::var("SOURCE_DATE_EPOCH") {
        if let Ok(secs) = v.trim().parse::<i64>() {
            return secs;
        }
    }
    published_at.unwrap_or(0)
}

/// RFC 2822 date (e.g. `Thu, 14 Aug 2026 09:30:00 +0000`) for the
/// changelog trailer, derived from the release's publish time (see
/// `reproducible_epoch`).
fn changelog_date(published_at: Option<i64>) -> String {
    let secs = reproducible_epoch(published_at);
    jiff::Timestamp::from_second(secs)
        .map(|t| t.strftime("%a, %d %b %Y %H:%M:%S %z").to_string())
        .unwrap_or_default()
}

fn write_copyright(
    output_dir: &Path,
    cfg: &PackageConfig,
    license: Option<&lpt_lib::github::RepoLicense>,
    published_at: Option<i64>,
) -> Result<()> {
    let secs = reproducible_epoch(published_at);
    let year = jiff::Timestamp::from_second(secs)
        .map(|t| t.strftime("%Y").to_string())
        .unwrap_or_else(|_| "1970".to_string());
    let spdx = license
        .map(|l| l.spdx.as_str())
        .filter(|s| !s.is_empty() && *s != "NOASSERTION")
        .unwrap_or_else(|| {
            if cfg.license_spdx.is_empty() {
                "NOASSERTION"
            } else {
                &cfg.license_spdx
            }
        });
    // Mirror the action's templates/output/copyright. The License body is the
    // upstream license text (or a manual-review note when undetectable).
    let body = license
        .and_then(|l| l.text.clone())
        .unwrap_or_else(|| " No machine-readable license text could be detected upstream;\n see the project's repository for licensing terms.".to_string());
    let text = format!(
        "Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/\nUpstream-Name: {pkg}\nUpstream-Contact: https://github.com/{repo}/issues\nSource: https://github.com/{repo}\n\nFiles: *\nCopyright: {year} {repo} contributors\nLicense: {spdx}\n\nFiles: debian/*\nCopyright: {year} latest-debs\nLicense: {spdx}\n\nLicense: {spdx}\n{body}\n",
        pkg = cfg.package_name,
        repo = cfg.github_repo,
    );
    let mut f = std::fs::File::create(output_dir.join("copyright"))?;
    f.write_all(text.as_bytes())?;
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changelog_date_is_deterministic_from_published_at() {
        // Same published_at must give the same changelog date no matter
        // when the test runs -- the whole point of not using
        // SystemTime::now(). Called twice to make that explicit.
        let a = changelog_date(Some(1_735_689_600)); // 2025-01-01T00:00:00Z
        let b = changelog_date(Some(1_735_689_600));
        assert_eq!(a, b);
        assert_eq!(a, "Wed, 01 Jan 2025 00:00:00 +0000");
    }

    #[test]
    fn changelog_date_falls_back_to_epoch_zero_without_published_at() {
        assert_eq!(changelog_date(None), "Thu, 01 Jan 1970 00:00:00 +0000");
    }

    #[test]
    fn reproducible_epoch_prefers_source_date_epoch_env_var() {
        // SAFETY: single assertion, cleaned up immediately; no other test
        // reads or writes SOURCE_DATE_EPOCH.
        unsafe {
            std::env::set_var("SOURCE_DATE_EPOCH", "1000000000");
        }
        let epoch = reproducible_epoch(Some(1_735_689_600));
        unsafe {
            std::env::remove_var("SOURCE_DATE_EPOCH");
        }
        assert_eq!(epoch, 1_000_000_000);
    }

    #[test]
    fn write_copyright_year_comes_from_published_at() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = PackageConfig {
            package_name: "eza".into(),
            github_repo: "eza-community/eza".into(),
            ..PackageConfig::default()
        };
        write_copyright(dir.path(), &cfg, None, Some(1_735_689_600)).unwrap();
        let text = std::fs::read_to_string(dir.path().join("copyright")).unwrap();
        assert!(text.contains("Copyright: 2025 eza-community/eza contributors"));
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
            parse_github_url("https://github.com/eza-community/eza/releases/tag/v0.24.0")
                .as_deref(),
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
            crate::config::DEFAULT_ARCHITECTURES.contains(&arch.as_str()),
            "unexpected arch: {arch}"
        );
    }

    fn job() -> ResolvedJob {
        ResolvedJob {
            dist: "trixie".into(),
            arch: "amd64".into(),
            asset: Asset {
                name: "pkg.tar.gz".into(),
                size: None,
                browser_download_url: String::new(),
            },
            tag: "v1.0.0".into(),
            published_at: Some(1_735_689_600), // 2025-01-01T00:00:00Z
        }
    }

    #[test]
    fn render_dockerfile_flat_mode_copies_loose_files() {
        let cfg = PackageConfig {
            package_name: "eza".into(),
            ..PackageConfig::default()
        };
        let out = render_dockerfile(&cfg, &job(), "1.0.0-1+trixie_amd64", "eza.deb");
        assert!(out.contains("/tmp/binary-source"));
        assert!(!out.contains("/usr/lib/eza"));
    }

    #[test]
    fn render_dockerfile_bundle_mode_symlinks_bin_into_usr_bin() {
        let cfg = PackageConfig {
            package_name: "zed".into(),
            bundle: true,
            ..PackageConfig::default()
        };
        let out = render_dockerfile(&cfg, &job(), "1.0.0-1+trixie_amd64", "zed.deb");
        assert!(out.contains("/usr/lib/zed"));
        assert!(out.contains("ln -s"));
        assert!(!out.contains("/tmp/binary-source"));
    }

    #[test]
    fn render_dockerfile_bundle_mode_also_symlinks_root_level_executables() {
        // pnpm-style bundles ship executables as siblings of their data
        // dirs at the bundle root, with no bin/ subdirectory at all.
        let cfg = PackageConfig {
            package_name: "pnpm".into(),
            bundle: true,
            ..PackageConfig::default()
        };
        let out = render_dockerfile(&cfg, &job(), "1.0.0-1+trixie_amd64", "pnpm.deb");
        assert!(out.contains(r#"for f in "/output/usr/lib/pnpm"/*"#));
    }

    #[test]
    fn render_dockerfile_fails_build_when_usr_bin_ends_up_empty() {
        for bundle in [false, true] {
            let cfg = PackageConfig {
                package_name: "x".into(),
                bundle,
                ..PackageConfig::default()
            };
            let out = render_dockerfile(&cfg, &job(), "1.0.0-1+trixie_amd64", "x.deb");
            assert!(
                out.contains("no executables landed in /usr/bin"),
                "bundle={bundle}"
            );
        }
    }

    #[test]
    fn write_control_includes_depends_when_set() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = PackageConfig {
            package_name: "pnpm".into(),
            github_repo: "pnpm/pnpm".into(),
            depends: "libatomic1, libgtk-3-0".into(),
            ..PackageConfig::default()
        };
        write_control(dir.path(), &cfg, &job(), "1.0.0", "1").unwrap();
        let text = std::fs::read_to_string(dir.path().join("control")).unwrap();
        assert!(text.contains("Depends: libatomic1, libgtk-3-0\n"));
        // Matches the action's Dockerfile, which `>>`-appends Depends after
        // the control file (including Description) is already rendered.
        assert!(text.trim_end().ends_with("Depends: libatomic1, libgtk-3-0"));
    }

    #[test]
    fn write_control_omits_depends_when_empty() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = PackageConfig {
            package_name: "eza".into(),
            github_repo: "eza-community/eza".into(),
            ..PackageConfig::default()
        };
        write_control(dir.path(), &cfg, &job(), "1.0.0", "1").unwrap();
        let text = std::fs::read_to_string(dir.path().join("control")).unwrap();
        assert!(!text.contains("Depends:"));
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
}
