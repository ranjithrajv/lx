// SPDX-License-Identifier: GPL-3.0-or-later

//! CMake build-system plugin.

use anyhow::Result;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::PackageConfig;
use crate::plugins::build_system::BuildSystem;
use crate::plugins::plugin::plugin_identity;

pub struct CmakeBuildSystem;

plugin_identity!(
    CmakeBuildSystem,
    "cmake",
    "CMake + Ninja — cmake configure, build, DESTDIR install"
);

impl BuildSystem for CmakeBuildSystem {
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
        super::run(cmd, "cmake configure")?;

        let mut build = Command::new("cmake");
        build.args(["--build", &build_dir.to_string_lossy()]);
        super::run(build, "cmake build")?;

        let mut install = Command::new("cmake");
        install
            .args(["--install", &build_dir.to_string_lossy()])
            .env("DESTDIR", &stage);
        super::run(install, "cmake install")?;
        Ok(stage)
    }
}
