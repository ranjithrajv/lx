use anyhow::Result;
use serde_json::json;
use std::path::PathBuf;

/// Minimal telemetry mirroring the action's `src/lib/telemetry.sh`
/// (`TELEMETRY_ENABLED` default false; `.telemetry/` dir; `metrics.json`,
/// `stages.log`, `failures.log`). When disabled every method is a no-op, so
/// callers need no conditionals.
#[derive(Debug, Clone)]
pub struct Telemetry {
    dir: PathBuf,
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
    #[cfg(test)]
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir() -> (tempfile::TempDir, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().to_path_buf();
        (d, p)
    }

    #[test]
    fn disabled_is_noop() {
        let (_guard, dir) = tmp_dir();
        let t = Telemetry::with_dir(false, dir);
        t.init().unwrap();
        t.record_stage("build_initialization").unwrap();
        t.record_failure("docker_build", "boom", 1).unwrap();
        t.finalize(5).unwrap();
        assert_eq!(t.summary_json(), json!({}));
        assert!(!t.dir.join("metrics.json").exists());
    }

    #[test]
    fn enabled_writes_logs_and_metrics() {
        let (_guard, dir) = tmp_dir();
        let t = Telemetry::with_dir(true, dir.clone());
        t.init().unwrap();
        t.record_stage("build_initialization").unwrap();
        t.record_stage_complete("architecture_amd64", "success")
            .unwrap();
        t.record_failure("architecture_build", "failed", 1).unwrap();
        t.finalize(7).unwrap();

        let stages = std::fs::read_to_string(dir.join("stages.log")).unwrap();
        assert!(stages.contains("Stage: build_initialization"));
        assert!(stages.contains("Stage complete: architecture_amd64 (success)"));

        let failures = std::fs::read_to_string(dir.join("failures.log")).unwrap();
        assert!(failures.contains("architecture_build - failed (exit code: 1)"));

        let s = t.summary_json();
        assert_eq!(s["telemetry_enabled"], true);
        assert_eq!(s["build_duration_seconds"], 7);
        assert_eq!(s["build_completed"], true);
        assert!(s["build_start_time"].as_u64().unwrap() > 0);
        assert!(s["build_end_time"].as_u64().unwrap() >= s["build_start_time"].as_u64().unwrap());
    }
}
