// SPDX-License-Identifier: GPL-3.0-or-later

//! Cargo (Rust) build-system plugin.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::PackageConfig;
use crate::plugins::build_system::BuildSystem;
use crate::plugins::plugin::plugin_identity;

pub struct CargoBuildSystem;

plugin_identity!(
    CargoBuildSystem,
    "cargo",
    "Cargo (Rust) — cargo build --release, install to DESTDIR"
);

impl BuildSystem for CargoBuildSystem {
    fn required_tools(&self) -> Vec<&'static str> {
        vec!["cargo"]
    }

    fn recognize(&self, src_dir: &Path) -> bool {
        src_dir.join("Cargo.toml").exists()
    }

    fn build(&self, cfg: &PackageConfig, src_dir: &Path, workdir: &Path) -> Result<PathBuf> {
        // cargo install --path . --root <stage> builds --release and installs
        // the binary(ies) to <stage>/bin. That's a ready-to-package FHS tree.
        let stage = workdir.join("stage");
        std::fs::create_dir_all(&stage)?;

        let mut args = vec![
            "install".to_string(),
            "--path".to_string(),
            ".".to_string(),
            "--root".to_string(),
            stage.to_string_lossy().to_string(),
            "--locked".to_string(),
        ];

        if cfg.musl {
            let target = musl_target().context("musl: cannot determine host architecture")?;
            ensure_musl_target_installed(&target)?;
            args.push("--target".to_string());
            args.push(target);
        }

        let st = Command::new("cargo")
            .args(&args)
            .current_dir(src_dir)
            .output()
            .context("failed to run `cargo install`")?;
        if !st.status.success() {
            bail!("cargo install failed (exit code {})", st.status);
        }

        // cargo install may produce no binaries (library crate) or place them
        // somewhere unexpected. Verify something landed in bin/.
        let bin_dir = stage.join("bin");
        let has_bins = bin_dir
            .read_dir()
            .ok()
            .map(|mut d| d.next().is_some())
            .unwrap_or(false);
        if !has_bins {
            bail!(
                "cargo install produced no binaries in {} — \
                 is this a binary crate? (library crates need build_system: custom)",
                bin_dir.display()
            );
        }
        Ok(stage)
    }
}

/// Map the host dpkg architecture to the Rust musl target triple.
fn musl_target() -> Result<String> {
    let arch = Command::new("dpkg")
        .arg("--print-architecture")
        .output()
        .context(
            "failed to run `dpkg --print-architecture` (install dpkg, or pass the arch manually)",
        )?;
    let arch = String::from_utf8_lossy(&arch.stdout).trim().to_string();
    match arch.as_str() {
        "amd64" => Ok("x86_64-unknown-linux-musl".to_string()),
        "arm64" => Ok("aarch64-unknown-linux-musl".to_string()),
        "armhf" => Ok("armv7-unknown-linux-musleabihf".to_string()),
        "i386" => Ok("i686-unknown-linux-musl".to_string()),
        "ppc64el" => Ok("powerpc64le-unknown-linux-musl".to_string()),
        "s390x" => Ok("s390x-unknown-linux-musl".to_string()),
        "riscv64" => Ok("riscv64gc-unknown-linux-musl".to_string()),
        other => bail!("musl: unsupported architecture '{other}' (no known Rust musl target)"),
    }
}

/// `rustup target add <target>` if not already installed.
fn ensure_musl_target_installed(target: &str) -> Result<()> {
    let installed = Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .context(
            "failed to run `rustup target list` (install rustup, or add the musl target manually)",
        )?;
    let list = String::from_utf8_lossy(&installed.stdout);
    if list.lines().any(|l| l == target) {
        return Ok(());
    }
    println!("musl: installing Rust target '{target}' via rustup");
    let st = Command::new("rustup")
        .args(["target", "add", target])
        .status()
        .with_context(|| format!("failed to install musl target '{target}' via rustup"))?;
    if !st.success() {
        bail!("rustup target add {} failed (exit code {})", target, st);
    }
    Ok(())
}
