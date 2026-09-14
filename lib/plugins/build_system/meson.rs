// SPDX-License-Identifier: GPL-3.0-or-later

//! Meson build-system plugin.
//!
//! Meson is the standard build system for GNOME, systemd-adjacent, and many
//! C/C++ projects. It uses Ninja as its backend and supports DESTDIR installs.
//!
//! The plugin recognizes `meson.build` and invokes:
//!   meson setup <builddir> && meson compile -C <builddir> && meson install -C <builddir> --destdir <stage>

use anyhow::Result;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::PackageConfig;
use crate::plugins::build_system::BuildSystem;
use crate::plugins::plugin::plugin_identity;

pub struct MesonBuildSystem;

plugin_identity!(
    MesonBuildSystem,
    "meson",
    "Meson + Ninja — meson setup, compile, DESTDIR install"
);

impl BuildSystem for MesonBuildSystem {
    fn required_tools(&self) -> Vec<&'static str> {
        vec!["meson", "ninja"]
    }

    fn recognize(&self, src_dir: &Path) -> bool {
        src_dir.join("meson.build").exists()
    }

    fn build(&self, cfg: &PackageConfig, src_dir: &Path, workdir: &Path) -> Result<PathBuf> {
        let build_dir = workdir.join("build");
        let stage = workdir.join("stage");
        std::fs::create_dir_all(&build_dir)?;
        std::fs::create_dir_all(&stage)?;

        // meson setup configures the build directory.
        let mut cmd = Command::new("meson");
        cmd.args([
            "setup",
            &build_dir.to_string_lossy(),
            &src_dir.to_string_lossy(),
        ]);
        cmd.arg("--prefix=/usr");
        cmd.arg("--buildtype=release");
        cmd.arg("--strip");
        if cfg.musl {
            // Cross-compile to musl for a static binary. Requires a
            // cross-file or the musl-gcc wrapper on the host.
            cmd.arg("--cross-file=musl");
        }
        for f in &cfg.cmake_flags {
            // Reuse cmake_flags as generic extra configure flags.
            cmd.arg(format!("-D{f}"));
        }
        println!("configuring: meson setup {}", cfg.cmake_flags.join(" "));
        super::run(cmd, "meson setup")?;

        // meson compile builds the project.
        let mut compile = Command::new("meson");
        compile.args(["compile", "-C", &build_dir.to_string_lossy()]);
        super::run(compile, "meson compile")?;

        // meson install stages into DESTDIR.
        let mut install = Command::new("meson");
        install
            .args(["install", "-C", &build_dir.to_string_lossy(), "--destdir"])
            .arg(&*stage.to_string_lossy());
        super::run(install, "meson install")?;

        Ok(stage)
    }
}
