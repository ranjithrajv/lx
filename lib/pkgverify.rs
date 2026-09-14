// SPDX-License-Identifier: GPL-3.0-or-later

//! Target-format lint/verify gate for converted packages.
//!
//! `lx build` has `--lintian` for deb. This generalizes the idea across
//! formats so `lx convert --lint` can block on a malformed artifact no matter
//! which way it was converted:
//!
//! | target format | checker | command   |
//! |---------------|---------|-----------|
//! | `deb`         | lintian | `lintian` |
//! | `rpm`         | rpm     | `rpm -K`  |
//! | `arch`        | namcap  | `namcap`  |
//!
//! The checker binary must be on `PATH`: run the gate on a host that has the
//! target format's tooling. A missing tool is an error, not a silent pass.

use anyhow::{bail, Context, Result};
use std::path::Path;
use std::process::Command;

/// Outcome of one target-format check.
#[derive(Debug, Default, Clone)]
pub struct VerifyReport {
    /// The checker that produced this report (`lintian`, `rpm -K`, `namcap`).
    pub checker: String,
    pub errors: usize,
    pub warnings: usize,
    /// Diagnostic lines worth echoing (errors/warnings/info).
    pub lines: Vec<String>,
}

impl VerifyReport {
    pub fn has_issues(&self) -> bool {
        self.errors > 0 || self.warnings > 0
    }
}

/// The checker name for a target format, if a gate exists.
pub fn checker_for(format: &str) -> Option<&'static str> {
    match format {
        "deb" => Some("lintian"),
        "rpm" => Some("rpm -K"),
        "arch" => Some("namcap"),
        _ => None,
    }
}

/// Run the target format's lint/verify checker against `pkg`.
pub fn run(format: &str, pkg: &Path) -> Result<VerifyReport> {
    match format {
        "deb" => {
            let report = crate::lintian::run(pkg, false, &[])?;
            Ok(VerifyReport {
                checker: "lintian".to_string(),
                errors: report.errors,
                warnings: report.warnings,
                lines: report.lines,
            })
        }
        "rpm" => run_tool("rpm", &["-K"], pkg, "rpm -K", parse_rpm),
        "arch" => run_tool("namcap", &[], pkg, "namcap", parse_namcap),
        other => {
            bail!("no lint/verify gate for target format '{other}' (expected deb, rpm, or arch)")
        }
    }
}

/// Whether a report should fail the conversion.
pub fn should_fail(report: &VerifyReport, fail_on_warnings: bool) -> bool {
    report.errors > 0 || (fail_on_warnings && report.warnings > 0)
}

/// Run an external checker and parse its output.
///
/// Public so tests can point it at a fake program (a shell script on disk)
/// standing in for lintian/rpm/namcap.
pub fn run_tool(
    program: &str,
    args: &[&str],
    pkg: &Path,
    display: &str,
    parse: fn(&str, &str) -> VerifyReport,
) -> Result<VerifyReport> {
    let pkg_abs = pkg
        .canonicalize()
        .with_context(|| format!("resolving {}", pkg.display()))?;
    let output = Command::new(program)
        .args(args)
        .arg(&pkg_abs)
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow::anyhow!(
                    "`--lint` requires the `{program}` binary on PATH for the {display} gate; \
                     not found (run the gate on a host with the target format's tooling \
                     installed)"
                )
            } else {
                anyhow::Error::from(e).context(format!("failed to run {display}"))
            }
        })?;
    let mut report = parse(
        &String::from_utf8_lossy(&output.stdout),
        &String::from_utf8_lossy(&output.stderr),
    );
    report.checker = display.to_string();
    // A non-zero exit that produced no recognizable diagnostics is still a
    // failure; don't let an unparsed crash read as "clean".
    if !output.status.success() && !report.has_issues() {
        report.errors = 1;
        report
            .lines
            .push(format!("{display} exited with {}", output.status));
    }
    Ok(report)
}

/// `rpm -K` output: `pkg.rpm: digests OK`,
/// `pkg.rpm: digests SIGNATURES NOT OK`, or `pkg.rpm: DIGESTS FAILED`.
fn parse_rpm(stdout: &str, stderr: &str) -> VerifyReport {
    let mut report = VerifyReport::default();
    for line in stdout.lines().chain(stderr.lines()) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let upper = line.to_ascii_uppercase();
        if upper.contains("FAILED") || upper.contains("NOT OK") || upper.contains(" BAD") {
            report.errors += 1;
            report.lines.push(line.to_string());
        } else if upper.contains("NOKEY") {
            report.warnings += 1;
            report.lines.push(line.to_string());
        } else if upper.contains(": OK") || upper.contains("DIGESTS OK") {
            report.lines.push(line.to_string());
        }
    }
    report
}

/// `namcap` output: `pkgname E: message` / `pkgname W: message`.
fn parse_namcap(stdout: &str, stderr: &str) -> VerifyReport {
    let mut report = VerifyReport::default();
    for line in stdout.lines().chain(stderr.lines()) {
        let line = line.trim();
        if line.contains(" E: ") {
            report.errors += 1;
            report.lines.push(line.to_string());
        } else if line.contains(" W: ") {
            report.warnings += 1;
            report.lines.push(line.to_string());
        } else if line.contains(" I: ") {
            report.lines.push(line.to_string());
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rpm_parse_flags_failures_not_ok_and_bad() {
        let r = parse_rpm("pkg.rpm: digests FAILED\npkg.rpm: X NOT OK\n", "");
        assert_eq!(r.errors, 2);
        assert!(should_fail(&r, false));
    }

    #[test]
    fn rpm_parse_ok_and_nokey() {
        let ok = parse_rpm("pkg.rpm: digests OK\n", "");
        assert_eq!(ok.errors, 0);
        assert!(!should_fail(&ok, true));
        let nokey = parse_rpm("pkg.rpm: NOKEY\n", "");
        assert_eq!(nokey.errors, 0);
        assert_eq!(nokey.warnings, 1);
        assert!(!should_fail(&nokey, false));
        assert!(should_fail(&nokey, true));
    }

    #[test]
    fn namcap_parse_counts_errors_and_warnings() {
        let r = parse_namcap("pkg E: bad thing\npkg W: iffy\npkg I: fyi\n", "");
        assert_eq!(r.errors, 1);
        assert_eq!(r.warnings, 1);
        assert!(should_fail(&r, false));
        assert!(should_fail(&r, true));
    }

    #[test]
    fn namcap_clean_passes() {
        let r = parse_namcap("", "");
        assert!(!should_fail(&r, true));
    }

    #[test]
    fn unknown_format_has_no_gate() {
        assert!(checker_for("apk").is_none());
        assert!(run("apk", Path::new("/nonexistent")).is_err());
    }
}
