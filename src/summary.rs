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
}

/// Write `build-summary.json` into `out_dir`, mirroring the action's
/// `generate_build_summary`: packages with sizes, total size, timing,
/// architecture/distribution lists, and a success rate.
pub fn write(out_dir: &Path, attempted: usize, inputs: &SummaryInputs) -> Result<()> {
    let mut packages = Vec::new();
    let mut total_size = 0u64;
    let pattern = format!("{}_*.deb", inputs.package);
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
        .filter(|p| p["method"] != "pinned" && p["method"] != "sidecar")
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

    let path = out_dir.join("build-summary.json");
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
/// in src/lib/summary.sh but pointed at `lpt` itself rather than its bash
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
🚀 Built with **lpt**
[![Built with lpt](https://img.shields.io/badge/built%20with-lpt-blue?logo=github)](https://github.com/ranjithrajv/lpt)
{release_badges}{coverage_badges}

**Build Stats:**
- ⚡ Success Rate: {success_rate}%
- ⏱️  Build Time: {build_time}s
- 📦 Packages: {packages}
- 🏗️  Architectures: {arch_count}

→ Try it: `lpt build https://github.com/<owner>/<repo>`"#
    );
}

/// Join badge label segments the way shields.io's static-badge endpoint
/// expects: spaces and `|` percent-encoded so `bookworm | trixie` renders
/// as one readable badge message instead of breaking the URL.
fn badge_encode(items: &[String]) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn badge_encode_percent_encodes_spaces_and_pipes() {
        assert_eq!(
            badge_encode(&["bookworm".into(), "trixie".into(), "sid".into()]),
            "bookworm%20%7C%20trixie%20%7C%20sid"
        );
        assert_eq!(badge_encode(&["amd64".into()]), "amd64");
    }

    #[test]
    fn glob_match_basic() {
        let pattern = glob::Pattern::new("eza_*.deb").unwrap();
        assert!(pattern.matches("eza_0.23.5-1+bookworm_amd64.deb"));
        assert!(!pattern.matches("eza_0.23.5-1+bookworm_amd64.dsc"));
        assert!(!pattern.matches("ezaa_0.1.deb"));
    }

    #[test]
    fn summary_lists_built_debs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        std::fs::write(path.join("eza_1.0-1+bookworm_amd64.deb"), vec![0u8; 2048]).unwrap();
        std::fs::write(path.join("eza_1.0-1+bookworm_arm64.deb"), vec![0u8; 4096]).unwrap();
        std::fs::write(path.join("eza_1.0-1+bookworm.dsc"), "x").unwrap();

        let inputs = SummaryInputs {
            package: "eza".into(),
            version: "v1.0".into(),
            build_version: "1".into(),
            github_repo: "eza-community/eza".into(),
            architectures: vec!["amd64".into(), "arm64".into()],
            distributions: vec!["bookworm".into()],
            max_parallel: 2,
            start: Instant::now(),
            telemetry: serde_json::json!({ "build_duration_seconds": 7 }),
            provenance: vec![],
        };
        write(path, 2, &inputs).unwrap();

        let text = std::fs::read_to_string(path.join("build-summary.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["total_packages"], 2);
        assert_eq!(v["total_size_bytes"], 6144);
        assert_eq!(v["total_size_human"], "6 KB");
        assert_eq!(v["success_rate"], 100);
        assert_eq!(v["package"], "eza");
        assert_eq!(v["full_version"], "v1.0-1");
        assert_eq!(v["packages"].as_array().unwrap().len(), 2);
        assert_eq!(v["telemetry"]["build_duration_seconds"], 7);
    }

    #[test]
    fn provenance_is_embedded_and_unverified_assets_are_counted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        std::fs::write(path.join("eza_1.0-1+bookworm_amd64.deb"), b"x").unwrap();

        let inputs = SummaryInputs {
            package: "eza".into(),
            version: "v1.0".into(),
            build_version: "1".into(),
            github_repo: "eza-community/eza".into(),
            architectures: vec!["amd64".into()],
            distributions: vec!["bookworm".into()],
            max_parallel: 1,
            start: Instant::now(),
            telemetry: serde_json::json!({}),
            provenance: vec![
                serde_json::json!({"asset": "a", "method": "pinned", "sha256": "aa"}),
                serde_json::json!({"asset": "b", "method": "sidecar", "sha256": "bb"}),
                serde_json::json!({
                    "asset": "c",
                    "method": "unverified (--allow-unverified)",
                    "sha256": "cc"
                }),
            ],
        };
        write(path, 1, &inputs).unwrap();

        let text = std::fs::read_to_string(path.join("build-summary.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["provenance"].as_array().unwrap().len(), 3);
        assert_eq!(v["provenance"][0]["asset"], "a");
        // Only the pinned and sidecar entries count as verified.
        assert_eq!(v["unverified_assets"], 1);
    }
}
