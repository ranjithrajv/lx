// SPDX-License-Identifier: GPL-3.0-or-later

//! Format-aware package install dispatch.
//!
//! Installs a prebuilt package using the right system tool for the host's
//! native format (deb → dpkg, rpm → rpm, arch → pacman). Used by the index
//! plugins so that `lx index install` serves rpm and arch hosts, not just deb.

use anyhow::{bail, Context, Result};
use std::path::Path;
use std::process::Command;

use crate::index::InstallFormat;

/// Verification and confirmation policy for [`install_prebuilt`]. Grouped
/// into a struct so callers cannot transpose three adjacent booleans.
#[derive(Debug, Clone, Copy, Default)]
pub struct InstallPolicy {
    /// Skip the confirmation prompt.
    pub assume_yes: bool,
    /// Skip checksum verification entirely.
    pub no_verify: bool,
    /// Proceed (with a warning) when the checksum is missing/mismatched
    /// instead of failing.
    pub allow_unverified: bool,
}

/// Install a prebuilt package at `path` in the given `format`. Prompts for
/// confirmation unless `policy.assume_yes`. Verifies checksum before install
/// when `expected_sha` is provided (non-empty). Returns `true` when the
/// package was installed, `false` when the user aborted the prompt.
pub fn install_prebuilt(
    path: &Path,
    filename: &str,
    format: InstallFormat,
    expected_sha: &str,
    policy: InstallPolicy,
) -> Result<bool> {
    if !policy.no_verify && !expected_sha.is_empty() {
        let actual = crate::lx_lib::checksum::sha256_file(path)?;
        if actual != expected_sha {
            if policy.allow_unverified {
                eprintln!("⚠ checksum mismatch for {filename}; installing anyway");
            } else {
                bail!(
                    "checksum mismatch for {filename}: expected {}, got {}",
                    &expected_sha[..16],
                    &actual[..16]
                );
            }
        } else {
            println!("✓ checksum verified for {filename}");
        }
    }

    if !policy.assume_yes
        && !confirm(
            &format!("Install {filename} via `sudo {}`?", install_cmd(format)),
            false,
        )?
    {
        println!("Aborted; package left at {}", path.display());
        return Ok(false);
    }

    match format {
        InstallFormat::Deb => install_deb(path)?,
        InstallFormat::Rpm => install_rpm(path)?,
        InstallFormat::Arch => install_arch(path)?,
        InstallFormat::Apk => install_apk(path)?,
    }
    Ok(true)
}

/// The package manager install subcommand for each format (for prompts).
fn install_cmd(format: InstallFormat) -> &'static str {
    match format {
        InstallFormat::Deb => "dpkg -i",
        InstallFormat::Rpm => "rpm -U",
        InstallFormat::Arch => "pacman -U",
        InstallFormat::Apk => "apk add",
    }
}

fn install_deb(path: &Path) -> Result<()> {
    let status = Command::new("sudo")
        .args(["dpkg", "-i"])
        .arg(path)
        .status()
        .context("failed to run `sudo dpkg -i`")?;
    if !status.success() {
        eprintln!("  dpkg reported missing dependencies; attempting `sudo apt-get install -f -y`");
        let fix = Command::new("sudo")
            .args(["apt-get", "install", "-f", "-y"])
            .status()
            .context("failed to run `sudo apt-get install -f -y`")?;
        if !fix.success() {
            bail!("failed to install {}", path.display());
        }
    }
    println!("✓ installed {}", path.display());
    Ok(())
}

fn install_rpm(path: &Path) -> Result<()> {
    let status = Command::new("sudo")
        .args(["rpm", "-U", "--replacefiles", "--replacepkgs"])
        .arg(path)
        .status()
        .context("failed to run `sudo rpm -U`")?;
    if !status.success() {
        bail!("failed to install {}", path.display());
    }
    println!("✓ installed {}", path.display());
    Ok(())
}

fn install_arch(path: &Path) -> Result<()> {
    let status = Command::new("sudo")
        .args(["pacman", "-U", "--noconfirm"])
        .arg(path)
        .status()
        .context("failed to run `sudo pacman -U`")?;
    if status.success() {
        println!("✓ installed {}", path.display());
        return Ok(());
    }

    // `pacman -U` failed. The recoverable case is a file the package wants to
    // place that already exists on disk but is owned by no package — a stray
    // binary from a `curl | sh` / Homebrew / mise install. pacman refuses to
    // overwrite those by default; retry with `--overwrite` for exactly those
    // paths. Files owned by *another* package are never handed to
    // `--overwrite`, so this cannot force-replace a file the host manager is
    // responsible for.
    let unowned = arch_unowned_existing_paths(path)?;
    if unowned.is_empty() {
        bail!("failed to install {}", path.display());
    }
    println!(
        "  {} path(s) already exist on disk unowned; retrying with `pacman -U --overwrite`",
        unowned.len()
    );
    let mut cmd = Command::new("sudo");
    cmd.args(["pacman", "-U", "--noconfirm"]);
    for p in &unowned {
        cmd.arg("--overwrite").arg(p);
    }
    cmd.arg(path);
    let status = cmd
        .status()
        .context("failed to run `sudo pacman -U --overwrite`")?;
    if !status.success() {
        bail!("failed to install {}", path.display());
    }
    println!("✓ installed {}", path.display());
    Ok(())
}

/// The paths a package file would install that already exist on disk but are
/// owned by no installed package — the `pacman -U` conflicts `--overwrite`
/// can legitimately resolve.
fn arch_unowned_existing_paths(pkg: &Path) -> Result<Vec<String>> {
    let files = arch_package_files(pkg)?;
    let existing: Vec<String> = files
        .into_iter()
        .filter(|p| Path::new(p).exists())
        .collect();
    if existing.is_empty() {
        return Ok(Vec::new());
    }
    let mut unowned = Vec::new();
    // `pacman -Qo` takes many paths but an unbounded argv would overflow
    // ARG_MAX on a large package; normalizing per chunk keeps it bounded.
    for chunk in existing.chunks(256) {
        let out = Command::new("pacman")
            .arg("-Qo")
            .args(chunk)
            .output()
            .context("failed to run `pacman -Qo`")?;
        // Exit 1 just means some path was unowned; the message is on stderr.
        unowned.extend(parse_unowned_paths(&String::from_utf8_lossy(&out.stderr)));
    }
    unowned.sort();
    unowned.dedup();
    Ok(unowned)
}

/// `pacman -Qlpq <file>` → the package's file paths.
fn arch_package_files(pkg: &Path) -> Result<Vec<String>> {
    let out = Command::new("pacman")
        .args(["-Qlpq"])
        .arg(pkg)
        .output()
        .context("failed to run `pacman -Qlpq`")?;
    if !out.status.success() {
        bail!("failed to list the files in {}", pkg.display());
    }
    Ok(parse_arch_package_files(&String::from_utf8_lossy(
        &out.stdout,
    )))
}

/// Parse `pacman -Qlpq` output (one path per line, trailing `/` on dirs).
fn parse_arch_package_files(out: &str) -> Vec<String> {
    out.lines()
        .map(|l| l.trim().trim_end_matches('/').to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

/// Parse the unowned paths out of `pacman -Qo`'s stderr
/// (`error: No package owns <path>`).
fn parse_unowned_paths(stderr: &str) -> Vec<String> {
    stderr
        .lines()
        .filter_map(|l| l.trim().strip_prefix("error: No package owns "))
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

fn install_apk(path: &Path) -> Result<()> {
    // `--allow-untrusted`: a locally built .apk is not in a signed repository,
    // the same way `dpkg -i`/`rpm -U` bypass repo signature checks.
    let status = Command::new("sudo")
        .args(["apk", "add", "--allow-untrusted"])
        .arg(path)
        .status()
        .context("failed to run `sudo apk add`")?;
    if !status.success() {
        bail!("failed to install {}", path.display());
    }
    println!("✓ installed {}", path.display());
    Ok(())
}

/// Detect the host's package architecture in a format-agnostic way.
pub fn detect_arch(format: InstallFormat) -> Result<String> {
    match format {
        InstallFormat::Deb => {
            let out = Command::new("dpkg")
                .arg("--print-architecture")
                .output()
                .context("failed to run `dpkg --print-architecture`")?;
            if out.status.success() {
                return Ok(String::from_utf8(out.stdout)?.trim().to_string());
            }
        }
        InstallFormat::Rpm => {
            let out = Command::new("rpm")
                .args(["--eval", "%_target_cpu"])
                .output()
                .context("failed to run `rpm --eval %_target_cpu`")?;
            if out.status.success() {
                let arch = String::from_utf8(out.stdout)?.trim().to_string();
                if !arch.is_empty() {
                    return Ok(arch);
                }
            }
        }
        InstallFormat::Arch => {
            // pacman has no direct arch query; use uname mapping below.
        }
        InstallFormat::Apk => {
            let out = Command::new("apk")
                .arg("--print-arch")
                .output()
                .context("failed to run `apk --print-arch`")?;
            if out.status.success() {
                let arch = String::from_utf8(out.stdout)?.trim().to_string();
                if !arch.is_empty() {
                    return Ok(arch);
                }
            }
        }
    }
    // Fallback: uname -m mapped to the format's arch naming.
    let machine = Command::new("uname")
        .arg("-m")
        .output()
        .context("failed to run `uname -m`")?;
    let m = String::from_utf8(machine.stdout)?.trim().to_string();
    Ok(map_arch(&m, format))
}

/// Map `uname -m` output to the format's architecture naming.
fn map_arch(machine: &str, format: InstallFormat) -> String {
    match format {
        InstallFormat::Deb => match machine {
            "x86_64" | "amd64" => "amd64".to_string(),
            "aarch64" | "arm64" => "arm64".to_string(),
            "armv7l" | "armv7" => "armhf".to_string(),
            "i686" | "i386" | "i586" => "i386".to_string(),
            "ppc64le" | "ppc64el" => "ppc64el".to_string(),
            "s390x" => "s390x".to_string(),
            "riscv64" => "riscv64".to_string(),
            "loongarch64" | "loong64" => "loong64".to_string(),
            other => other.to_string(),
        },
        InstallFormat::Rpm => match machine {
            "x86_64" | "amd64" => "x86_64".to_string(),
            "aarch64" | "arm64" => "aarch64".to_string(),
            "armv7l" | "armv7" => "armv7hl".to_string(),
            "i686" | "i386" | "i586" => "i686".to_string(),
            "ppc64le" | "ppc64el" => "ppc64le".to_string(),
            "s390x" => "s390x".to_string(),
            "riscv64" => "riscv64".to_string(),
            "loongarch64" | "loong64" => "loongarch64".to_string(),
            other => other.to_string(),
        },
        InstallFormat::Arch => match machine {
            "x86_64" | "amd64" => "x86_64".to_string(),
            "aarch64" | "arm64" => "aarch64".to_string(),
            other => other.to_string(),
        },
        InstallFormat::Apk => match machine {
            "x86_64" | "amd64" => "x86_64".to_string(),
            "aarch64" | "arm64" => "aarch64".to_string(),
            "armv7l" | "armv7" => "armv7".to_string(),
            "armv6l" => "armhf".to_string(),
            "i686" | "i386" | "i586" => "x86".to_string(),
            "ppc64le" | "ppc64el" => "ppc64le".to_string(),
            "s390x" => "s390x".to_string(),
            "riscv64" => "riscv64".to_string(),
            other => other.to_string(),
        },
    }
}

/// Shared confirmation prompt (mirrors `debs::confirm`).
fn confirm(prompt: &str, _default: bool) -> Result<bool> {
    use std::io::Write;
    print!("{prompt} [y/N] ");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pacman_qlpq_paths() {
        let out = "/opt/\n/opt/1Password/\n/opt/1Password/1Password-BrowserSupport\n\n";
        assert_eq!(
            parse_arch_package_files(out),
            vec![
                "/opt",
                "/opt/1Password",
                "/opt/1Password/1Password-BrowserSupport",
            ]
        );
    }

    #[test]
    fn parses_only_the_unowned_paths() {
        // `pacman -Qo` mixes owned (stdout) and unowned (stderr); only the
        // stderr lines reach this parser.
        let stderr = "error: No package owns /usr/bin/herdr\nerror: No package owns /usr/share/licenses/herdr\n";
        assert_eq!(
            parse_unowned_paths(stderr),
            vec!["/usr/bin/herdr", "/usr/share/licenses/herdr"]
        );
    }

    #[test]
    fn owned_paths_are_not_recovery_candidates() {
        assert!(parse_unowned_paths("").is_empty());
        assert!(parse_unowned_paths("/usr/bin/bash is owned by bash 5.3.15-1\n").is_empty());
    }
}
