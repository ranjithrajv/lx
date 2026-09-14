// SPDX-License-Identifier: GPL-3.0-or-later

//! Plain GNU Make build-system plugin.
//!
//! Handles projects that ship a top-level `Makefile` without a configure
//! step. Recognized last among the dedicated build systems (after cmake,
//! cargo, go, meson, autotools) so it only claims trees nothing more
//! specific matched, then runs `make` + `make install` with
//! `PREFIX=/usr`/`DESTDIR=<stage>`.
//!
//! Extra `cmake_flags:` entries are passed through as additional make
//! arguments (variable overrides like `PREFIX=/usr` or targets).

use anyhow::Result;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::PackageConfig;
use crate::plugins::build_system::BuildSystem;
use crate::plugins::plugin::plugin_identity;

pub struct MakeBuildSystem;

plugin_identity!(
    MakeBuildSystem,
    "make",
    "GNU Make — make + DESTDIR make install (no configure)"
);

impl BuildSystem for MakeBuildSystem {
    fn required_tools(&self) -> Vec<&'static str> {
        vec!["make"]
    }

    fn recognize(&self, src_dir: &Path) -> bool {
        src_dir.join("Makefile").is_file()
            || src_dir.join("makefile").is_file()
            || src_dir.join("GNUmakefile").is_file()
    }

    fn build(&self, cfg: &PackageConfig, src_dir: &Path, workdir: &Path) -> Result<PathBuf> {
        let stage = workdir.join("stage");
        std::fs::create_dir_all(&stage)?;

        let jobs = super::available_parallelism();
        let mut cmd = Command::new("make");
        cmd.arg(format!("-j{jobs}")).arg("PREFIX=/usr");
        for f in &cfg.cmake_flags {
            cmd.arg(f);
        }
        super::apply_musl_env(&mut cmd, cfg);
        println!(
            "building: make -j{jobs} PREFIX=/usr {}",
            cfg.cmake_flags.join(" ")
        );
        cmd.current_dir(src_dir);
        super::run(cmd, "make")?;

        let mut cmd = Command::new("make");
        cmd.arg(format!("DESTDIR={}", stage.to_string_lossy()))
            .arg("PREFIX=/usr");
        for f in &cfg.cmake_flags {
            cmd.arg(f);
        }
        cmd.arg("install");
        super::apply_musl_env(&mut cmd, cfg);
        cmd.current_dir(src_dir);
        super::run(cmd, "make install")?;

        Ok(stage)
    }
}
