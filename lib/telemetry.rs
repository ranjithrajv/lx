// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::Result;
use serde_json::json;
use std::path::PathBuf;

/// Minimal telemetry mirroring the action's `src/lib/telemetry.sh`
/// (`TELEMETRY_ENABLED` default false; `.telemetry/` dir; `metrics.json`,
/// `stages.log`, `failures.log`). When disabled every method is a no-op, so
/// callers need no conditionals.
#[derive(Debug, Clone)]
pub struct Telemetry {
    pub dir: PathBuf,
    enabled: bool,
    start_epoch: u64,
}

impl Telemetry {
    /// `dir` defaults to `.telemetry` in the current directory (the action's
    /// `TELEMETRY_DIR`).
    pub fn new(enabled: bool) -> Self {
        Self {
            dir: PathBuf::from(".telemetry"),
            enabled,
            start_epoch: now_epoch(),
        }
    }

    /// Test seam: telemetry rooted at an arbitrary directory.
    pub fn with_dir(enabled: bool, dir: PathBuf) -> Self {
        Self {
            dir,
            enabled,
            start_epoch: now_epoch(),
        }
    }

    /// `init_telemetry`: create the directory and seed `metrics.json`.
    pub fn init(&self) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        std::fs::create_dir_all(&self.dir)?;
        self.write_metrics(json!({
            "build_start_time": self.start_epoch,
            "telemetry_enabled": true,
        }))?;
        Ok(())
    }

    /// `record_build_stage <stage>`: append a line to `stages.log`.
    pub fn record_stage(&self, stage: &str) -> Result<()> {
        self.append_log("stages.log", &format!("{} - Stage: {stage}", timestamp()))
    }

    /// `record_build_stage_complete <stage> <status>`: append a line.
    pub fn record_stage_complete(&self, stage: &str, status: &str) -> Result<()> {
        self.append_log(
            "stages.log",
            &format!("{} - Stage complete: {stage} ({status})", timestamp()),
        )
    }

    /// `record_build_failure <category> <details> <exit_code>`: append to
    /// `failures.log`.
    pub fn record_failure(&self, category: &str, details: &str, exit_code: u32) -> Result<()> {
        self.append_log(
            "failures.log",
            &format!("Build failure recorded: {category} - {details} (exit code: {exit_code})"),
        )
    }

    /// `finalize_telemetry`: append completion fields to `metrics.json`.
    pub fn finalize(&self, duration_secs: u64) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        let end = now_epoch();
        // Re-read current metrics so we don't clobber init-time fields.
        let mut metrics = self
            .read_metrics()
            .unwrap_or_else(|| json!({ "telemetry_enabled": true }));
        metrics["build_end_time"] = json!(end);
        metrics["build_duration"] = json!(duration_secs);
        metrics["build_completed"] = json!(true);
        self.write_metrics(metrics)?;
        Ok(())
    }

    /// `get_telemetry_summary`: the `telemetry` object embedded in
    /// `build-summary.json`. Returns `{}` when disabled (or missing data).
    pub fn summary_json(&self) -> serde_json::Value {
        if !self.enabled {
            return json!({});
        }
        match self.read_metrics() {
            Some(m) => json!({
                "build_start_time": m["build_start_time"],
                "build_end_time": m["build_end_time"],
                "build_duration_seconds": m["build_duration"],
                "build_completed": m["build_completed"],
                "telemetry_enabled": true,
            }),
            None => json!({}),
        }
    }

    /// `save_as_baseline`: copy metrics.json to baseline.json for future
    /// regression comparison (mirrors the action's `save_as_baseline`).
    /// No-op when telemetry is disabled or no metrics exist yet.
    pub fn save_as_baseline(&self) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        let src = self.dir.join("metrics.json");
        if !src.exists() {
            return Ok(());
        }
        let dst = self.dir.join("baseline.json");
        std::fs::copy(src, dst)?;
        Ok(())
    }

    /// Compare the current build duration against baseline.json; returns a
    /// warning string when duration regressed >20% (mirrors the action's
    /// `check_performance_regressions`, duration half — lx tracks no
    /// memory metric). `None` when no baseline or no regression.
    pub fn check_regression(&self) -> Option<String> {
        if !self.enabled {
            return None;
        }
        let cur: serde_json::Value = self.read_metrics()?;
        let base: serde_json::Value = std::fs::read_to_string(self.dir.join("baseline.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())?;
        let cur_d = cur.get("build_duration")?.as_u64()?;
        let base_d = base.get("build_duration")?.as_u64()?;
        if base_d > 0 && cur_d > base_d * 120 / 100 {
            let pct = (cur_d - base_d) * 100 / base_d;
            Some(format!(
                "Build duration increased by {pct}% ({base_d}s -> {cur_d}s)"
            ))
        } else {
            None
        }
    }

    fn append_log(&self, file: &str, line: &str) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        let path = self.dir.join(file);
        if !path.exists() {
            std::fs::create_dir_all(&self.dir)?;
        }
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        writeln!(f, "{line}")?;
        Ok(())
    }

    fn write_metrics(&self, metrics: serde_json::Value) -> Result<()> {
        std::fs::write(
            self.dir.join("metrics.json"),
            serde_json::to_string_pretty(&metrics)?,
        )?;
        Ok(())
    }

    fn read_metrics(&self) -> Option<serde_json::Value> {
        std::fs::read_to_string(self.dir.join("metrics.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
    }
}

fn now_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// `date '+%Y-%m-%d %H:%M:%S'` (UTC) for log lines.
fn timestamp() -> String {
    let epoch = now_epoch();
    jiff::Timestamp::from_second(epoch as i64)
        .map(|t| t.strftime("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_default()
}
