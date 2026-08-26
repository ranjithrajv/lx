use anyhow::{anyhow, bail, Context, Result};
use clap::Args;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::PackageConfig;
use crate::discovery::{config_from_release, guess_format, match_assets};
use lpt_lib::github::Asset;

#[derive(Debug, Clone, Args)]
pub struct BuildArgs {
    /// Path to package.yaml
    #[arg(default_value = lpt_lib::constants::DEFAULT_CONFIG_FILENAME)]
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

    /// Directory to write resulting package files into.
    #[arg(long, default_value = "dist")]
    pub output: PathBuf,

    /// Package format plugin to use (deb, rpm, or arch). Overrides
    /// package.yaml's `package_format`. Defaults to the config file's value
    /// (or "deb").
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
    /// in-process -- no Docker, no dpkg-source subprocess.
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

    /// Path for the build progress JSON (default: /tmp/build_progress.json).
    #[arg(long)]
    pub progress_path: Option<PathBuf>,

    /// Keep intermediate files (downloaded assets, staging dirs).
    #[arg(long)]
    pub keep: bool,

    /// Sign built packages: path to an ASCII-armored secret key. Overrides
    /// package.yaml's `signature.key_file`. For deb, produces a detached
    /// `<pkg>.deb.sig` via gpg; for rpm, embeds the PGP signature natively
    /// (`rpm -K` verifiable). Passphrase comes from $LPT_SIGN_PASSPHRASE,
    /// falling back to $NFPM_PASSPHRASE.
    #[arg(long, value_name = "KEY_FILE")]
    pub sign_key: Option<PathBuf>,

    /// Signing key id / fingerprint (gpg --local-user for deb; ignored by
    /// rpm). Overrides package.yaml's `signature.key_id`.
    #[arg(long, value_name = "KEY_ID")]
    pub sign_key_id: Option<String>,
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
    let mut cfg = match crate::plugins::source::parse_any_url(&args.config.to_string_lossy()) {
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
            // `parse_github_url` for backward compat (though parse_any_url
            // already covers it).
            if let Some(github_repo) = parse_github_url(&args.config.to_string_lossy()) {
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
            } else {
                PackageConfig::load(&args.config)?
            }
        }
    };
    if let Some(v) = &args.version {
        cfg.version = v.clone();
    }
    // Resolve effective package format: --format overrides package.yaml.
    let effective_format = args
        .format
        .as_deref()
        .unwrap_or(&cfg.effective_package_format())
        .to_ascii_lowercase();
    let plugin_names = crate::plugins::available_names();
    if crate::plugins::get_plugin(&effective_format).is_none() {
        bail!(
            "unsupported --format '{}' (expected one of: {})",
            effective_format,
            plugin_names.join(", ")
        );
    }
    // Normalize cfg's package_format for downstream consumers (summary, etc.).
    cfg.package_format = effective_format.clone();
    // Keep plugin trait object for arch/dist filtering.
    let plugin_for_matrix: Box<dyn crate::plugins::Plugin> =
        crate::plugins::get_plugin(&effective_format).unwrap();
    println!(
        "package format: {} ({})",
        effective_format,
        plugin_for_matrix.description()
    );

    // Resolve source provider plugin (github vs gitlab) for auto-discovery.
    let effective_source = args
        .provider
        .as_deref()
        .unwrap_or(&cfg.effective_source())
        .to_ascii_lowercase();
    let source = crate::plugins::source::get_source_plugin(&effective_source).ok_or_else(|| {
        anyhow!(
            "unsupported source '{}' (expected one of: {})",
            effective_source,
            crate::plugins::source::source_available_names().join(", ")
        )
    })?;
    println!("source: {} ({})", effective_source, source.description());
    cfg.source = effective_source.clone();
    crate::plugins::source::apply_source_host(source.as_ref(), &cfg);
    // Token resolution: prefer provider-specific env, fallback to CLI token.
    let token_for_source = crate::plugins::source::resolve_source_token(source.as_ref(), token);

    // Resolve the release: pinned version in config, else a tag/version, else latest.
    let release = if !cfg.version.is_empty() {
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
        lpt_lib::optimize::effective_max_parallel(0)
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
    dists = lpt_lib::config::filter_expired_distributions(&dists, None);

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
                    // A distribution_arch_overrides entry for the arch
                    // replaces the built-in support matrix entirely.
                    let supported =
                        match cfg.distribution_arch_overrides.get(arch.as_str()) {
                            Some(o) => o.distributions.iter().any(|d| d.trim() == dist),
                            None => plugin_for_matrix.arch_supported_for_dist(arch, dist),
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
    let telemetry = lpt_lib::telemetry::Telemetry::new(args.telemetry);
    telemetry.init()?;
    telemetry.record_stage("build_initialization")?;

    let progress = if args.progress && lpt_lib::progress::stdout_is_tty() {
        Some(lpt_lib::progress::Progress::new(
            arch_assets.len(),
            &release.tag_name,
            &cfg.package_name,
            args.progress_path
                .clone()
                .unwrap_or_else(|| PathBuf::from(lpt_lib::constants::DEFAULT_PROGRESS_PATH)),
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
            source_name: effective_source.clone(),
            token: token_for_source.clone(),
        },
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
                provenance,
                package_format: effective_format.clone(),
                source: effective_source.clone(),
            },
        )?;
    }

    if args.source {
        let rel = cfg.effective_relations(&effective_format);
        let pkg = crate::source::Pkg {
            name: cfg.package_name.clone(),
            github_repo: cfg.github_repo.clone(),
            description: cfg.effective_description(),
            maintainer: cfg.effective_maintainer(),
            version: release.tag_name.clone(),
            build_version: args.build_version.clone(),
            epoch: cfg.epoch.clone(),
            license_spdx: license
                .as_ref()
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
            license: license.clone(),
        };
        match effective_format.as_str() {
            "rpm" => crate::source::generate_rpm(&args.output, &pkg)?,
            "arch" => crate::source::generate_arch(&args.output, &pkg)?,
            _ => crate::source::generate(&args.output, &pkg)?,
        }
    }
    Ok(())
}

pub(crate) fn suggest_versions_source(
    source: &dyn crate::plugins::source::SourcePlugin,
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
    source: &dyn crate::plugins::source::SourcePlugin,
    cfg: &PackageConfig,
    token: Option<&str>,
    cache_dir: Option<&Path>,
) -> Result<Option<lpt_lib::github::RepoLicense>> {
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
                license = Some(lpt_lib::github::RepoLicense {
                    spdx: "Apache-2.0 or MIT".to_string(),
                    text: Some(format!(
                        "Dual-licensed under either of:\n\n=== Apache License 2.0 ===\n\n{apache}\n\n=== MIT License ===\n\n{mit}"
                    )),
                });
            }
        }
    }

    if license.is_none() && !cfg.license_spdx.is_empty() {
        license = Some(lpt_lib::github::RepoLicense {
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
/// its legacy patterns carry a literal `v` (`pkg_v{version}_...`). lpt
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
    release: &lpt_lib::github::Release,
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
pub fn parse_github_url(s: &str) -> Option<String> {
    let host = lpt_lib::constants::DEFAULT_GITHUB_HOST;
    let rest = s
        .strip_prefix(&format!("https://{host}/"))
        .or_else(|| s.strip_prefix(&format!("http://{host}/")))?;
    let mut parts = rest.trim_end_matches('/').splitn(3, '/');
    let owner = parts.next()?;
    let repo = parts.next()?.trim_end_matches(".git");
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some(format!("{owner}/{repo}"))
}

/// Surface a resolved release's prerelease/draft status. Both fields were
/// already parsed off the GitHub API response but never consulted anywhere
/// -- someone pinning an explicit tag that happens to be an RC/beta got no
/// signal that they were about to package pre-stable software.
pub(crate) fn warn_if_prerelease_or_draft(release: &lpt_lib::github::Release) {
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

/// Source-provider inputs resolved once in `run` and threaded through every
/// worker thread `build_jobs` spawns.
struct SourceInputs {
    license: Option<lpt_lib::github::RepoLicense>,
    source_name: String,
    token: Option<String>,
}

fn build_jobs(
    args: &BuildArgs,
    cfg: &PackageConfig,
    jobs: &[ResolvedJob],
    source: SourceInputs,
    progress: Option<&lpt_lib::progress::Progress>,
    telemetry: &lpt_lib::telemetry::Telemetry,
) -> Result<Vec<serde_json::Value>> {
    let SourceInputs {
        license,
        source_name,
        token,
    } = source;
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
        let progress = progress.clone();
        let telemetry = telemetry.clone();
        let source_name = source_name.clone();
        handles.push(std::thread::spawn(move || {
            let source = crate::plugins::source::get_source_plugin(&source_name)
                .expect("unknown source plugin");
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
                    source: source.as_ref(),
                    token: token.as_deref(),
                    license: license.as_ref(),
                    pin: pin.as_ref(),
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
    Ok(std::sync::Arc::try_unwrap(provenance)
        .map(|m| m.into_inner().unwrap_or_default())
        .unwrap_or_default())
}

/// Per-worker source/verification context, resolved once per architecture
/// group and threaded through every [`build_one`] call in that group.
#[derive(Clone, Copy)]
struct BuildInputs<'a> {
    source: &'a dyn crate::plugins::source::SourcePlugin,
    token: Option<&'a str>,
    license: Option<&'a lpt_lib::github::RepoLicense>,
    pin: Option<&'a lpt_lib::checksum::PinnedMetadata>,
}

fn build_one(
    args: &BuildArgs,
    cfg: &PackageConfig,
    inputs: &BuildInputs,
    job: &ResolvedJob,
    tmp: &Path,
    downloaded: &mut std::collections::HashMap<String, PathBuf>,
    provenance: &std::sync::Mutex<Vec<serde_json::Value>>,
) -> Result<PathBuf> {
    let BuildInputs {
        source,
        token,
        license,
        pin,
    } = *inputs;
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
                    let expected = pin.and_then(|p| p.sha256_for(&job.tag, &job.asset.name));
                    let cache = lpt_lib::cache::DownloadCache::new(dir.clone())?;
                    let source_cloned = source.name().to_string();
                    let token_cloned = token.map(|s| s.to_string());
                    cache.fetch(
                        &job.asset.browser_download_url,
                        &path,
                        expected.as_deref(),
                        &|url, out| {
                            let src = crate::plugins::source::get_source_plugin(&source_cloned)
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
                        .with_context(|| {
                            format!("GET {} failed", job.asset.browser_download_url)
                        })?;
                    let mut file = std::fs::File::create(&path)?;
                    std::io::copy(&mut body, &mut file)?;
                }
            }
            let method = if args.no_verify {
                VerifyMethod::SkippedNoVerify
            } else if let Some(pin) = pin {
                if let Some(expected) = pin.sha256_for(&job.tag, &job.asset.name) {
                    lpt_lib::checksum::verify_sha256(&path, &expected)?;
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
            } else {
                verify_sidecar_or_require_flag_source(
                    source,
                    token,
                    &job.asset,
                    &path,
                    args.allow_unverified,
                )?
            };

            // Audit trail: one entry per unique download, regardless of
            // outcome, so build-summary.json's `provenance` array records
            // exactly how (or whether) every asset was verified.
            let sha256 = lpt_lib::checksum::sha256_file(&path).unwrap_or_default();
            provenance.lock().unwrap().push(serde_json::json!({
                "asset": job.asset.name,
                "url": job.asset.browser_download_url,
                "tag": job.tag,
                "method": method.as_str(),
                "sha256": sha256,
            }));

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

    // 4. Build the package via the selected plugin (deb or rpm).
    // Plugin stages the install tree and creates the archive.
    let format = cfg.effective_package_format();
    let plugin = crate::plugins::get_plugin(&format).ok_or_else(|| {
        anyhow!(
            "unsupported package format '{}' (expected deb or rpm)",
            format
        )
    })?;
    let debian_version = lpt_lib::pkgmeta::strip_upstream_prefix(if cfg.version.is_empty() {
        &job.tag
    } else {
        &cfg.version
    });
    let mtime = lpt_lib::pkgmeta::reproducible_epoch(job.published_at);
    let staging_root = tmp.join(format!(
        "{}-{}-root-{}",
        job.arch,
        job.dist,
        plugin.file_extension()
    ));
    std::fs::create_dir_all(&staging_root)?;
    // Signing for formats that embed signatures natively (rpm) is passed
    // into the plugin; deb signs post-build (detached .sig).
    let sign_key = cfg.effective_sign_key(args.sign_key.as_deref());
    let sign_passphrase = resolve_sign_passphrase();
    let ctx = crate::plugins::BuildContext {
        cfg,
        job,
        binary_dir: &binary_dir,
        staging_root: &staging_root,
        license,
        debian_version: &debian_version,
        build_version: &args.build_version,
        mtime,
        sign_key: sign_key.as_deref(),
        sign_passphrase: sign_passphrase.as_deref(),
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
            let report = lpt_lib::lintian::run(&final_path, args.lintian_pedantic, &suppress)?;
            print_lintian_report(&final_path, &report);
            if lpt_lib::lintian::should_fail(&report, args.lintian_fail_on_warnings) {
                bail!("lintian failed for {}", final_path.display());
            }
        }
    }

    // 6. Optional signing. deb: detached gpg signature next to the .deb;
    // rpm/arch: handled inside the plugin (rpm embeds natively, arch has
    // no signature story in pacman packages).
    if let Some(key) = cfg.effective_sign_key(args.sign_key.as_deref()) {
        if format == "deb" {
            let key_id = cfg.effective_sign_key_id(args.sign_key_id.as_deref());
            let req = lpt_lib::sign::SignRequest {
                key_file: &key,
                key_id: &key_id,
                passphrase: sign_passphrase.as_deref(),
            };
            let sig = lpt_lib::sign::gpg_detach_sign(&final_path, &req)?;
            println!("    ✓ signed {} -> {}", final_path.display(), sig.display());
        }
    }

    Ok(final_path)
}

/// Signing passphrase resolution shared by both formats:
/// `$LPT_SIGN_PASSPHRASE`, falling back to `$NFPM_PASSPHRASE` (nfpm parity).
fn resolve_sign_passphrase() -> Option<String> {
    for var in ["LPT_SIGN_PASSPHRASE", "NFPM_PASSPHRASE"] {
        if let Ok(v) = std::env::var(var) {
            if !v.trim().is_empty() {
                return Some(v);
            }
        }
    }
    None
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

/// How a downloaded asset's integrity was (or wasn't) established --
/// recorded per-asset into build-summary.json's `provenance` array (see
/// `build_one`) so a build can be audited after the fact.
#[derive(Clone, Copy)]
pub enum VerifyMethod {
    /// Matched a `--pinned-metadata` vet-time provenance pin.
    Pinned,
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
            VerifyMethod::Sidecar => "sidecar",
            VerifyMethod::UnverifiedAllowed => "unverified (--allow-unverified)",
            VerifyMethod::SkippedNoVerify => "skipped (--no-verify)",
        }
    }
}

/// Live sidecar checksum verification (probe-and-verify logic shared with
/// `debs.rs` via `lpt_lib::checksum::check_sidecar`). Most real-world
/// GitHub releases don't publish a checksum sidecar, so without
/// `--allow-unverified` this fails the build rather than silently
/// proceeding on an unverified download that's about to become an
/// installable, often sudo-installed .deb -- a missing sidecar used to
/// just print a warning and continue, which meant the *default* path for
/// most repos had zero integrity verification with only a console line as
/// evidence.
fn verify_sidecar_or_require_flag(
    client: &dyn lpt_lib::checksum::RawGetter,
    asset: &Asset,
    path: &Path,
    allow_unverified: bool,
) -> Result<VerifyMethod> {
    use lpt_lib::checksum::SidecarCheck;
    match lpt_lib::checksum::check_sidecar(client, &asset.browser_download_url, &asset.name, path)?
    {
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
    source: &dyn crate::plugins::source::SourcePlugin,
    token: Option<&str>,
    asset: &Asset,
    path: &Path,
    allow_unverified: bool,
) -> Result<VerifyMethod> {
    struct Getter<'a> {
        source: &'a dyn crate::plugins::source::SourcePlugin,
        token: Option<&'a str>,
    }
    impl lpt_lib::checksum::RawGetter for Getter<'_> {
        fn raw_get(&self, url: &str) -> Result<Box<dyn std::io::Read + Send>> {
            self.source.raw_get(url, self.token)
        }
    }
    verify_sidecar_or_require_flag(&Getter { source, token }, asset, path, allow_unverified)
}

pub fn extract(archive: &Path, dest: &Path, format: &str) -> Result<()> {
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
/// architecture needs QEMU emulation, and to resolve `--host`.
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
