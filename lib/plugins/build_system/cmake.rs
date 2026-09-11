// SPDX-License-Identifier: GPL-3.0-or-later

//! CMake build-system plugin.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::PackageConfig;
use crate::plugins::build_system::BuildSystem;

pub struct CmakeBuildSystem;

impl BuildSystem for CmakeBuildSystem {
    fn name(&self) -> &'static str {
        "cmake"
    }

    fn description(&self) -> &'static str {
        "CMake + Ninja — cmake configure, build, DESTDIR install"
    }

    fn required_tools(&self) -> Vec<&'static str> {
        vec!["cmake", "ninja"]
    }

    fn recognize(&self, src_dir: &Path) -> bool {
        src_dir.join("CMakeLists.txt").exists()
    }

    fn build(&self, cfg: &PackageConfig, src_dir: &Path, workdir: &Path) -> Result<PathBuf> {
        let build_dir = workdir.join("build");
        let stage = workdir.join("stage");
        std::fs::create_dir_all(&build_dir)?;
        std::fs::create_dir_all(&stage)?;

        let mut cmd = Command::new("cmake");
        cmd.args([
            "-S",
            &src_dir.to_string_lossy(),
            "-B",
            &build_dir.to_string_lossy(),
            "-G",
            "Ninja",
        ]);
        cmd.arg("-DCMAKE_INSTALL_PREFIX=/usr");
        cmd.arg("-DCMAKE_BUILD_TYPE=Release");
        if cfg.musl {
            // musl-gcc produces a statically-linked binary with no glibc
            // dependency. Requires `musl-tools` (or equivalent) on the host.
            cmd.arg("-DCMAKE_C_COMPILER=musl-gcc");
            cmd.arg("-DCMAKE_CXX_COMPILER=musl-g++");
            cmd.arg("-DCMAKE_EXE_LINKER_FLAGS=-static");
        }
        for f in &cfg.cmake_flags {
            cmd.arg(f);
        }
        println!("configuring: cmake {}", cfg.cmake_flags.join(" "));
        let st = cmd.status().context("failed to run cmake configure")?;
        if !st.success() {
            bail!("cmake configure failed");
        }
        Command::new("cmake")
            .args(["--build", &build_dir.to_string_lossy()])
            .status()
            .context("cmake build failed")?;
        Command::new("cmake")
            .args(["--install", &build_dir.to_string_lossy()])
            .env("DESTDIR", &stage)
            .status()
            .context("cmake install failed")?;
        Ok(stage)
    }
}
