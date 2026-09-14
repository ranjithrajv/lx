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

/// Install a prebuilt package at `path` in the given `format`. Prompts for
/// confirmation unless `yes`. Verifies checksum before install when
/// `expected_sha` is provided (non-empty). Returns `true` when the package was
/// installed, `false` when the user aborted the prompt.
pub fn install_prebuilt(
    path: &Path,
    filename: &str,
    format: InstallFormat,
    expected_sha: &str,
    yes: bool,
    no_verify: bool,
    allow_unverified: bool,
) -> Result<bool> {
    if !no_verify && !expected_sha.is_empty() {
        let actual = crate::lx_lib::checksum::sha256_file(path)?;
        if actual != expected_sha {
            if allow_unverified {
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

    if !yes
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
    }
    Ok(true)
}

/// The package manager install subcommand for each format (for prompts).
fn install_cmd(format: InstallFormat) -> &'static str {
    match format {
        InstallFormat::Deb => "dpkg -i",
        InstallFormat::Rpm => "rpm -U",
        InstallFormat::Arch => "pacman -U",
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
