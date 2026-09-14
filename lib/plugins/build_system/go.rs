// SPDX-License-Identifier: GPL-3.0-or-later

//! Go build-system plugin.

use anyhow::Result;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::PackageConfig;
use crate::plugins::build_system::BuildSystem;
use crate::plugins::plugin::plugin_identity;

pub struct GoBuildSystem;

plugin_identity!(
    GoBuildSystem,
    "go",
    "Go — go build single binary, install to DESTDIR"
);

impl BuildSystem for GoBuildSystem {
    fn required_tools(&self) -> Vec<&'static str> {
        vec!["go"]
    }

    fn recognize(&self, src_dir: &Path) -> bool {
        src_dir.join("go.mod").exists()
    }

    fn build(&self, cfg: &PackageConfig, src_dir: &Path, workdir: &Path) -> Result<PathBuf> {
        let stage = workdir.join("stage");
        let bin_dir = stage.join("bin");
        std::fs::create_dir_all(&bin_dir)?;

        // Build the package in the current directory. -trimpath strips host
        // paths from the binary (reproducibility); -ldflags "-s -w" strips
        // debug info for a smaller binary.
        let mut cmd = Command::new("go");
        cmd.args([
            "build",
            "-trimpath",
            "-ldflags",
            "-s -w",
            "-o",
            &bin_dir.join(&cfg.package_name).to_string_lossy(),
            ".",
        ]);
        if cfg.musl {
            // CGO_ENABLED=0 produces a statically-linked binary with no
            // glibc dependency — runs on any Linux regardless of distro age.
            cmd.env("CGO_ENABLED", "0");
        }
        cmd.current_dir(src_dir);
        super::run(cmd, "go build")?;
        Ok(stage)
    }
}
