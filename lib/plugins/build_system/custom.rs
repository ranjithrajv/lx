// SPDX-License-Identifier: GPL-3.0-or-later

//! Custom build-system plugin: user-supplied build/install commands.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::PackageConfig;
use crate::plugins::build_system::BuildSystem;

pub struct CustomBuildSystem;

impl BuildSystem for CustomBuildSystem {
    fn name(&self) -> &'static str {
        "custom"
    }

    fn description(&self) -> &'static str {
        "Custom — user-supplied build_commands / install_commands"
    }

    // `custom` never auto-detects — it's explicit-only. Inherits the default
    // `recognize` returning false.

    fn build(&self, cfg: &PackageConfig, src_dir: &Path, workdir: &Path) -> Result<PathBuf> {
        let stage = workdir.join("stage");
        std::fs::create_dir_all(&stage)?;
        let destdir = stage.to_string_lossy().to_string();

        println!(
            "running {} custom build command(s)",
            cfg.build_commands.len()
        );
        for cmd_str in &cfg.build_commands {
            println!("  $ {cmd_str}");
            let st = Command::new("sh")
                .args(["-c", cmd_str])
                .current_dir(src_dir)
                .env("DESTDIR", destdir.as_str())
                .status()
                .with_context(|| format!("failed to run build command: {cmd_str}"))?;
            if !st.success() {
                bail!("custom build command failed: {cmd_str}");
            }
        }
        println!(
            "running {} custom install command(s)",
            cfg.install_commands.len()
        );
        for cmd_str in &cfg.install_commands {
            println!("  $ {cmd_str}");
            let st = Command::new("sh")
                .args(["-c", cmd_str])
                .current_dir(src_dir)
                .env("DESTDIR", destdir.as_str())
                .status()
                .with_context(|| format!("failed to run install command: {cmd_str}"))?;
            if !st.success() {
                bail!("custom install command failed: {cmd_str}");
            }
        }
        Ok(stage)
    }
}
