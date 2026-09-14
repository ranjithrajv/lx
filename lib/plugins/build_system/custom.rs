// SPDX-License-Identifier: GPL-3.0-or-later

//! Custom build-system plugin: user-supplied build/install commands.

use anyhow::Result;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::PackageConfig;
use crate::plugins::build_system::BuildSystem;
use crate::plugins::plugin::plugin_identity;

pub struct CustomBuildSystem;

plugin_identity!(
    CustomBuildSystem,
    "custom",
    "Custom — user-supplied build_commands / install_commands"
);

impl BuildSystem for CustomBuildSystem {
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
            let mut cmd = Command::new("sh");
            cmd.args(["-c", cmd_str])
                .current_dir(src_dir)
                .env("DESTDIR", destdir.as_str());
            super::run(cmd, &format!("custom build command: {cmd_str}"))?;
        }
        println!(
            "running {} custom install command(s)",
            cfg.install_commands.len()
        );
        for cmd_str in &cfg.install_commands {
            println!("  $ {cmd_str}");
            let mut cmd = Command::new("sh");
            cmd.args(["-c", cmd_str])
                .current_dir(src_dir)
                .env("DESTDIR", destdir.as_str());
            super::run(cmd, &format!("custom install command: {cmd_str}"))?;
        }
        Ok(stage)
    }
}
