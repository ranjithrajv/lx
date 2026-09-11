// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::Result;
use std::path::Path;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Runtime inputs mirrored from the action's `generate_build_summary`
/// (src/lib/summary.sh).
pub struct SummaryInputs {
    pub package: String,
    pub version: String,
    pub build_version: String,
    pub github_repo: String,
    pub architectures: Vec<String>,
    pub distributions: Vec<String>,
    pub max_parallel: usize,
    pub start: Instant,
    /// `telemetry` object from telemetry.sh's get_telemetry_summary
    /// ({} when disabled).
    pub telemetry: serde_json::Value,
    /// One entry per unique asset download: how (or whether) its integrity
    /// was verified, so a build can be audited after the fact instead of
    /// just trusting a console log that's already scrolled away. See
    /// `build::VerifyMethod`.
    pub provenance: Vec<serde_json::Value>,
    /// Package format plugin used for this build ("deb", "rpm", or "arch").
    /// Defaults to "deb" for backward compatibility with older callers/tests.
    #[allow(dead_code)]
    pub package_format: String,
    /// Source provider plugin used for this build ("github", "gitlab",
    /// "gitea", "forgejo", "bitbucket", or "gerrit"). Defaults to "github"
    /// for backward compatibility with older callers/tests.
    #[allow(dead_code)]
    pub source: String,
}

impl Default for SummaryInputs {
    fn default() -> Self {
        Self {
            package: String::new(),
            version: String::new(),
            build_version: String::new(),
            github_repo: String::new(),
            architectures: Vec::new(),
            distributions: Vec::new(),
            max_parallel: 0,
            start: Instant::now(),
            telemetry: serde_json::Value::Null,
            provenance: Vec::new(),
            package_format: "deb".to_string(),
            source: "github".to_string(),
        }
    }
}

/// Write `build-summary.json` into `out_dir`, mirroring the action's
/// `generate_build_summary`: packages with sizes, total size, timing,
/// architecture/distribution lists, and a success rate.
pub fn write(out_dir: &Path, attempted: usize, inputs: &SummaryInputs) -> Result<()> {
    let mut packages = Vec::new();
    let mut total_size = 0u64;
    let pattern = match inputs.package_format.as_str() {
        "rpm" => format!("{}-*.rpm", inputs.package),
        "arch" => format!("{}-*.pkg.tar.*", inputs.package),
        _ => format!("{}_*.deb", inputs.package),
    };
    let matcher = glob::Pattern::new(&pattern)?;
    let mut entries: Vec<_> = std::fs::read_dir(out_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| matcher.matches(n))
        .collect();
    entries.sort();
    for name in &entries {
        let size = std::fs::metadata(out_dir.join(name))
            .map(|m| m.len())
            .unwrap_or(0);
        total_size += size;
        packages.push(serde_json::json!({ "name": name, "size": size }));
    }
    let total_packages = packages.len();

    let size_human = if total_size / (1024 * 1024) > 0 {
        format!("{} MB", total_size / (1024 * 1024))
    } else {
        format!("{} KB", total_size / 1024)
    };

    let archs = serde_json::Value::Array(
        inputs
            .architectures
            .iter()
            .map(|a| serde_json::Value::String(a.clone()))
            .collect(),
    );
    let dists = serde_json::Value::Array(
        inputs
            .distributions
            .iter()
            .map(|d| serde_json::Value::String(d.clone()))
            .collect(),
    );

    let success_rate = if attempted > 0 {
        (total_packages * 100) / attempted
    } else {
        0
    };

    let unverified_assets = inputs
        .provenance
        .iter()
        .filter(|p| p["method"] != "pinned" && p["method"] != "sidecar" && p["method"] != "locked")
        .count();

    let duration = inputs.start.elapsed().as_secs();
    let end_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let now = rfc3339(end_epoch);
    let start = rfc3339(end_epoch.saturating_sub(duration));

    let summary = serde_json::json!({
        "package": inputs.package,
        "version": inputs.version,
        "build_version": inputs.build_version,
        "full_version": format!("{}-{}", inputs.version, inputs.build_version),
        "github_repo": inputs.github_repo,
        "package_format": inputs.package_format,
        "source": inputs.source,
        "architectures": archs,
        "distributions": dists,
        "total_packages": total_packages,
        "total_size_bytes": total_size,
        "total_size_human": size_human,
        "build_duration_seconds": duration,
        "build_start": start,
        "build_end": now,
        "parallel_builds": true,
        "max_parallel": inputs.max_parallel,
        "packages": packages,
        "lintian": {},
        "telemetry": inputs.telemetry,
        "provenance": inputs.provenance,
        "unverified_assets": unverified_assets,
        "success_rate": success_rate,
    });

    let path = out_dir.join(lx_lib::constants::SUMMARY_FILENAME);
    let json = serde_json::to_string_pretty(&summary)?;
    std::fs::write(&path, json)?;
    println!("  ✓ build summary saved to {}", path.display());
    println!("     📦 Total artifact size: {size_human} ({total_packages} packages)");
    viral_badge(
        success_rate as u64,
        duration,
        total_packages as u64,
        inputs.architectures.len(),
        &inputs.architectures,
        &inputs.distributions,
    );
    Ok(())
}

/// Print a markdown badge block for pasting into the packaging repo's own
/// README, mirroring the (now-superseded) action's `generate_viral_badge`
/// in src/lib/summary.sh but pointed at `lx` itself rather than its bash
/// predecessor. Printed after every summary.
fn viral_badge(
    success_rate: u64,
    build_time: u64,
    packages: u64,
    arch_count: usize,
    architectures: &[String],
    distributions: &[String],
) {
    // Latest-release/downloads badges need to know *this* repo (the one
    // publishing the built .deb as a GitHub release), which is distinct
    // from `github_repo` (the upstream source being packaged) -- only
    // knowable when actually running as the GitHub Action, via the
    // standard `GITHUB_REPOSITORY` env var. Omitted entirely on a local
    // run rather than guessed, since there's no other reliable source for
    // where (or whether) the package gets published.
    let release_badges = std::env::var("GITHUB_REPOSITORY")
        .ok()
        .filter(|r| !r.trim().is_empty())
        .map(|repo| {
            format!(
                "[![Latest Release](https://img.shields.io/github/v/release/{repo})](https://github.com/{repo}/releases/latest) \
                 [![Downloads](https://img.shields.io/github/downloads/{repo}/total)](https://github.com/{repo}/releases)\n"
            )
        })
        .unwrap_or_default();

    let mut coverage_badges = String::new();
    if !distributions.is_empty() {
        coverage_badges.push_str(&format!(
            "![Suites](https://img.shields.io/badge/suites-{}-blue) ",
            badge_encode(distributions)
        ));
    }
    if !architectures.is_empty() {
        coverage_badges.push_str(&format!(
            "![Architectures](https://img.shields.io/badge/architectures-{}-blue)",
            badge_encode(architectures)
        ));
    }

    println!(
        r#"
---
🚀 Built with **lx**
[![Built with lx](https://img.shields.io/badge/built%20with-lx-blue?logo=github)](https://github.com/ranjithrajv/lx)
{release_badges}{coverage_badges}

**Build Stats:**
- ⚡ Success Rate: {success_rate}%
- ⏱️  Build Time: {build_time}s
- 📦 Packages: {packages}
- 🏗️  Architectures: {arch_count}

→ Try it: `lx build https://github.com/<owner>/<repo>`"#
    );
}

/// Join badge label segments the way shields.io's static-badge endpoint
/// expects: spaces and `|` percent-encoded so `bookworm | trixie` renders
/// as one readable badge message instead of breaking the URL.
pub fn badge_encode(items: &[String]) -> String {
    items
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(" | ")
        .replace(' ', "%20")
        .replace('|', "%7C")
}

/// Format a Unix epoch timestamp as RFC 3339 in UTC (the action uses
/// `date -d @<epoch> '+%Y-%m-%dT%H:%M:%S%z'`).
fn rfc3339(epoch: u64) -> String {
    jiff::Timestamp::from_second(epoch as i64)
        .map(|t| t.strftime("%Y-%m-%dT%H:%M:%S%z").to_string())
        .unwrap_or_default()
}
