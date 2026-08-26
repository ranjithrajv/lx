use anyhow::{bail, Context, Result};
use std::path::Path;
use std::process::Command;

/// Results of a lintian run against a single .deb.
#[derive(Debug, Default, Clone)]
pub struct LintianReport {
    pub errors: usize,
    pub warnings: usize,
    pub info: usize,
    pub lines: Vec<String>,
}

/// Run the real `lintian` binary on a built .deb, mirroring the action's
/// `run_lintian_check`.
///
/// - controlled by `--lintian` (default off),
/// - requires `lintian` on `PATH` -- no Docker fallback. lintian is a
///   large, Debian-native Perl tool with no Rust equivalent to reach for,
///   so unlike the rest of this pipeline there's nothing to reimplement;
///   the fix is to just run it directly rather than route through a
///   container to reach the exact same binary,
/// - runs `lintian --info [--pedantic] [--suppress-tags ...] <deb>`,
/// - counts `E:`/`W:`/`I:` output lines.
pub fn run(deb: &Path, pedantic: bool, suppress_tags: &[String]) -> Result<LintianReport> {
    let deb_abs = deb
        .canonicalize()
        .with_context(|| format!("resolving {}", deb.display()))?;

    let mut args = vec!["--info".to_string()];
    if pedantic {
        args.push("--pedantic".to_string());
    }
    if !suppress_tags.is_empty() {
        args.push("--suppress-tags".to_string());
        args.push(suppress_tags.join(","));
    }

    let output = Command::new("lintian")
        .args(&args)
        .arg(&deb_abs)
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow::anyhow!(
                    "--lintian requires the `lintian` binary on PATH (e.g. `apt-get install \
                     lintian` on Debian/Ubuntu); not found"
                )
            } else {
                anyhow::Error::from(e).context("failed to run lintian")
            }
        })?;
    if !output.status.success() && output.stdout.is_empty() && output.stderr.is_empty() {
        bail!(
            "lintian exited with {} and produced no output",
            output.status
        );
    }
    Ok(parse(
        &String::from_utf8_lossy(&output.stdout),
        &String::from_utf8_lossy(&output.stderr),
    ))
}

/// Determine whether a report should fail the build.
pub fn should_fail(report: &LintianReport, fail_on_warnings: bool) -> bool {
    report.errors > 0 || (fail_on_warnings && report.warnings > 0)
}

pub fn parse(stdout: &str, stderr: &str) -> LintianReport {
    let mut report = LintianReport::default();
    for line in stdout.lines().chain(stderr.lines()) {
        if let Some(rest) = line.strip_prefix("E:") {
            report.errors += 1;
            report.lines.push(format!("E:{rest}"));
        } else if let Some(rest) = line.strip_prefix("W:") {
            report.warnings += 1;
            report.lines.push(format!("W:{rest}"));
        } else if let Some(rest) = line.strip_prefix("I:") {
            report.info += 1;
            report.lines.push(format!("I:{rest}"));
        }
    }
    report
}
