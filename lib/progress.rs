use anyhow::Result;
use serde_json::json;
use std::collections::HashMap;
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Build progress mirroring the action's `src/lib/progress.sh`:
/// a JSON state file (`/tmp/build_progress.json` by default) plus an inline
/// terminal progress line. Works across the parallel arch workers because
/// the state is shared and updated under a mutex.
#[derive(Clone)]
pub struct Progress {
    file: PathBuf,
    total_archs: usize,
    package: String,
    version: String,
    /// arch -> status ("running", "completed", "failed", "pending")
    states: Arc<Mutex<HashMap<String, String>>>,
    tty: bool,
    /// last-drawn progress line length, so we can clear it on the next draw
    last_len: Arc<Mutex<usize>>,
}

/// Job outcome reported by a worker for one architecture group.
#[derive(Debug, Clone, Copy)]
pub enum Outcome {
    Completed,
    Failed,
}

impl Progress {
    /// `init_progress_tracking <total_archs> <version> <package>` + writes the
    /// initial JSON file. `tty` gates the inline bar (the action only renders
    /// a dashboard on an interactive terminal).
    pub fn new(
        total_archs: usize,
        version: &str,
        package: &str,
        file: PathBuf,
        tty: bool,
    ) -> Result<Self> {
        let p = Self {
            file,
            total_archs,
            package: package.to_string(),
            version: version.to_string(),
            states: Arc::new(Mutex::new(HashMap::new())),
            tty,
            last_len: Arc::new(Mutex::new(0)),
        };
        p.write_json()?;
        Ok(p)
    }

    /// `update_arch_status <arch> <status>`: record the status and redraw.
    pub fn set_arch(&self, arch: &str, status: &str) -> Result<()> {
        self.states
            .lock()
            .unwrap()
            .insert(arch.to_string(), status.to_string());
        self.write_json()?;
        if self.tty {
            self.draw();
        }
        Ok(())
    }

    /// Record a finished architecture group and advance the bar.
    pub fn finish_arch(&self, arch: &str, outcome: Outcome) -> Result<()> {
        self.set_arch(
            arch,
            match outcome {
                Outcome::Completed => "completed",
                Outcome::Failed => "failed",
            },
        )
    }

    /// `cleanup_progress_tracking`: remove the state file and clear the line.
    pub fn cleanup(&self) -> Result<()> {
        let _ = std::fs::remove_file(&self.file);
        if self.tty {
            let mut last = self.last_len.lock().unwrap();
            if *last > 0 {
                print!("\r{:width$}\r", "", width = *last);
                std::io::stdout().flush().ok();
                *last = 0;
            }
        }
        Ok(())
    }

    fn write_json(&self) -> Result<()> {
        let states = self.states.lock().unwrap();
        let (completed, failed, running, pending) =
            states
                .values()
                .fold((0, 0, 0, 0), |(c, f, r, p), s| match s.as_str() {
                    "completed" => (c + 1, f, r, p),
                    "failed" => (c, f + 1, r, p),
                    "running" => (c, f, r + 1, p),
                    _ => (c, f, r, p + 1),
                });
        let started = completed + failed + running;
        let pending_total = self
            .total_archs
            .saturating_sub(started)
            .saturating_add(pending);
        let archs = serde_json::Value::Object(
            states
                .iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        json!({
                            "status": v,
                            "updated_at": now_epoch(),
                            "details": "",
                        }),
                    )
                })
                .collect(),
        );
        let state = json!({
            "total_archs": self.total_archs,
            "completed": completed,
            "failed": failed,
            "running": running,
            "pending": pending_total,
            "start_time": now_epoch(),
            "package": self.package,
            "version": self.version,
            "architectures": archs,
        });
        std::fs::write(&self.file, serde_json::to_string(&state)?)?;
        Ok(())
    }

    /// `display_simple_progress`-style `\r [bar] NN% (n/N) <arch status>`.
    fn draw(&self) {
        let states = self.states.lock().unwrap();
        let (completed, failed, running, _) =
            states
                .values()
                .fold((0, 0, 0, 0), |(c, f, r, p), s| match s.as_str() {
                    "completed" => (c + 1, f, r, p),
                    "failed" => (c, f + 1, r, p),
                    "running" => (c, f, r + 1, p),
                    _ => (c, f, r, p),
                });
        let done = completed + failed;
        let pct = if self.total_archs > 0 {
            (done * 100) / self.total_archs
        } else {
            100
        };
        let width = 25usize;
        let filled = (pct * width) / 100;
        let mut line = "\r  [".to_string();
        line.push_str(&"█".repeat(filled));
        line.push_str(&"░".repeat(width - filled));
        let status = if running > 0 {
            format!(" building ({running} running)")
        } else {
            String::new()
        };
        line.push_str(&format!(
            "] {pct:3}% ({done}/{}), {} failed{status}",
            self.total_archs, failed
        ));
        print!("{line}");
        let _ = std::io::stdout().flush();
        let mut last = self.last_len.lock().unwrap();
        *last = line.len();
        drop(states);
    }
}

fn now_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// True when stdout is an interactive terminal (the action's dashboard is
/// interactive-only too).
pub fn stdout_is_tty() -> bool {
    std::io::stdout().is_terminal()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_file() -> PathBuf {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        std::env::temp_dir().join(format!("lpt-progress-test-{}-{n}.json", std::process::id()))
    }

    #[test]
    fn tracks_status_and_writes_json() {
        let f = tmp_file();
        let p = Progress::new(3, "v1.0", "eza", f.clone(), false).unwrap();

        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&f).unwrap()).unwrap();
        assert_eq!(v["total_archs"], 3);
        assert_eq!(v["pending"], 3);
        assert_eq!(v["package"], "eza");

        p.set_arch("amd64", "running").unwrap();
        p.finish_arch("amd64", Outcome::Completed).unwrap();
        p.finish_arch("arm64", Outcome::Failed).unwrap();
        p.set_arch("armhf", "running").unwrap();

        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&f).unwrap()).unwrap();
        assert_eq!(v["completed"], 1);
        assert_eq!(v["failed"], 1);
        assert_eq!(v["running"], 1);
        assert_eq!(v["architectures"]["amd64"]["status"], "completed");
        assert_eq!(v["architectures"]["arm64"]["status"], "failed");

        p.cleanup().unwrap();
        assert!(!f.exists());
    }

    #[test]
    fn tty_render_clears_line() {
        let f = tmp_file();
        let p = Progress::new(1, "v1.0", "eza", f.clone(), true).unwrap();
        p.finish_arch("amd64", Outcome::Completed).unwrap();
        p.cleanup().unwrap();
        assert!(!f.exists());
    }
}
