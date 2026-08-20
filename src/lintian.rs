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

const LINTIAN_IMAGE: &str = "lpt-lintian";

/// Run lintian on a built .deb inside a Debian container (the host may be
/// non-Debian, e.g. Arch), mirroring the action's `run_lintian_check`.
///
/// - controlled by `--lintian` (default off),
/// - builds a cached `lpt-lintian` image (debian + lintian) on first use,
/// - runs `lintian --info [--pedantic] [--suppress-tags ...] <deb>`,
/// - counts `E:`/`W:`/`I:` output lines.
pub fn run(deb: &Path, pedantic: bool, suppress_tags: &[String]) -> Result<LintianReport> {
    check_docker()?;
    ensure_lintian_image()?;

    let deb_abs = deb
        .canonicalize()
        .with_context(|| format!("resolving {}", deb.display()))?;

    let mut cmd = Command::new("docker");
    cmd.args([
        "run",
        "--rm",
        "-v",
        &format!("{}:/tmp/pkg.deb", deb_abs.display()),
        LINTIAN_IMAGE,
        "lintian",
        "--info",
    ]);
    if pedantic {
        cmd.arg("--pedantic");
    }
    if !suppress_tags.is_empty() {
        cmd.arg("--suppress-tags").arg(suppress_tags.join(","));
    }
    cmd.arg("/tmp/pkg.deb");

    let output = cmd.output().context("failed to run lintian container")?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    Ok(parse(&stdout, &stderr))
}

/// Determine whether a report should fail the build.
pub fn should_fail(report: &LintianReport, fail_on_warnings: bool) -> bool {
    report.errors > 0 || (fail_on_warnings && report.warnings > 0)
}

fn check_docker() -> Result<()> {
    let status = Command::new("docker")
        .arg("version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .context("failed to run docker")?;
    if !status.success() {
        bail!("docker is required to run lintian (the host is non-Debian)");
    }
    Ok(())
}

/// Build a cached image with lintian installed, if not already present.
fn ensure_lintian_image() -> Result<()> {
    let inspect = Command::new("docker")
        .args(["image", "inspect", LINTIAN_IMAGE])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .context("failed to inspect lintian image")?;
    if inspect.success() {
        return Ok(());
    }

    let dir = tempfile::tempdir().context("failed to create temp dir")?;
    let df = dir.path().join("Dockerfile");
    std::fs::write(
        &df,
        "FROM debian:bookworm\nRUN apt-get update && apt-get install -y --no-install-recommends lintian && rm -rf /var/lib/apt/lists/*\n",
    )
    .context("failed to write lintian Dockerfile")?;

    let status = Command::new("docker")
        .args([
            "build",
            "-t",
            LINTIAN_IMAGE,
            "-f",
            df.to_str().unwrap(),
            dir.path().to_str().unwrap(),
        ])
        .stdout(std::process::Stdio::null())
        .status()
        .context("failed to build lintian image")?;
    if !status.success() {
        bail!("failed to build lintian image ({LINTIAN_IMAGE})");
    }
    Ok(())
}

fn parse(stdout: &str, stderr: &str) -> LintianReport {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_severity_lines() {
        let r = parse(
            "E: eza: bad-version\nW: eza: no-homepage\nI: eza: no-php\n",
            "",
        );
        assert_eq!(r.errors, 1);
        assert_eq!(r.warnings, 1);
        assert_eq!(r.info, 1);
        assert_eq!(r.lines.len(), 3);
    }

    #[test]
    fn parse_ignores_non_severity_output() {
        let r = parse(
            "lintian check v2.114.0\n\nRunning checks...\n",
            "N: weird line",
        );
        assert_eq!(r.errors, 0);
        assert_eq!(r.warnings, 0);
        assert_eq!(r.info, 0);
    }

    #[test]
    fn fail_rules() {
        let clean = LintianReport::default();
        assert!(!should_fail(&clean, false));

        let err = LintianReport {
            errors: 1,
            ..Default::default()
        };
        assert!(should_fail(&err, false));
        assert!(should_fail(&err, true));

        let warn = LintianReport {
            warnings: 1,
            ..Default::default()
        };
        assert!(!should_fail(&warn, false));
        assert!(should_fail(&warn, true));
    }
}
