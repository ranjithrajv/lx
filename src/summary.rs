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
    );
    Ok(())
}

/// Print the "viral badge" markdown block, mirroring the action's
/// `generate_viral_badge` in src/lib/summary.sh. Printed after every
/// summary so users can paste the badge into their README.
fn viral_badge(success_rate: u64, build_time: u64, packages: u64, arch_count: usize) {
    println!(
        r#"
---
🚀 Built with **debian-multiarch-builder**
[![Built with debian-multiarch-builder](https://img.shields.io/badge/built%20with-debian--multiarch--builder-blue?logo=github)](https://github.com/ranjithrajv/debian-multiarch-builder)

**Build Stats:**
- ⚡ Success Rate: {success_rate}%
- ⏱️  Build Time: {build_time}s
- 📦 Packages: {packages}
- 🏗️  Architectures: {arch_count}

→ Try it free: `./build.sh --setup` or `./build.sh --zc owner/repo version 1`"#
    );
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
}
