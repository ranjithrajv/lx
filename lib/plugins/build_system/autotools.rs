// SPDX-License-Identifier: GPL-3.0-or-later

//! GNU Autotools build-system plugin.
//!
//! Autotools (`./configure && make && make install`) is still the dominant
//! build system for C/C++ upstreams. The plugin recognizes a `configure`
//! script, an `autogen.sh` bootstrap script, or `configure.ac`/`configure.in`,
//! bootstraps when needed, then configures/builds/installs into a DESTDIR
//! stage.
//!
//! Extra `cmake_flags:` entries are passed through as additional
//! `./configure` flags (the shared "extra build flags" field, as meson does).

use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

use crate::config::PackageConfig;
use crate::plugins::build_system::BuildSystem;
use crate::plugins::plugin::plugin_identity;

pub struct AutotoolsBuildSystem;

plugin_identity!(
    AutotoolsBuildSystem,
    "autotools",
    "GNU Autotools — configure, make, DESTDIR make install"
);

impl BuildSystem for AutotoolsBuildSystem {
    fn required_tools(&self) -> Vec<&'static str> {
        vec!["make"]
    }

    fn recognize(&self, src_dir: &Path) -> bool {
        src_dir.join("configure").is_file()
            || src_dir.join("configure.ac").is_file()
            || src_dir.join("configure.in").is_file()
            || src_dir.join("autogen.sh").is_file()
    }

    fn build(&self, cfg: &PackageConfig, src_dir: &Path, workdir: &Path) -> Result<PathBuf> {
        let stage = workdir.join("stage");
        std::fs::create_dir_all(&stage)?;

        // Bootstrap `configure` when the release tarball didn't ship one.
        if !src_dir.join("configure").is_file() {
            if src_dir.join("autogen.sh").is_file() {
                println!("bootstrapping: sh autogen.sh");
                run_in("sh", &["autogen.sh"], src_dir, cfg, "autogen.sh")?;
            } else if src_dir.join("configure.ac").is_file()
                || src_dir.join("configure.in").is_file()
            {
                println!("bootstrapping: autoreconf -fi");
                run_in("autoreconf", &["-fi"], src_dir, cfg, "autoreconf")?;
            }
        }
        if !src_dir.join("configure").is_file() {
            bail!(
                "autotools: no `configure` script (or autogen.sh/configure.ac to generate one) in {}",
                src_dir.display()
            );
        }

        // ./configure --prefix=/usr [extra flags]
        let mut args: Vec<String> = vec!["--prefix=/usr".to_string()];
        for f in &cfg.cmake_flags {
            if f.starts_with('-') {
                args.push(f.clone());
            } else {
                args.push(format!("--{f}"));
            }
        }
        println!("configuring: ./configure {}", args.join(" "));
        let mut cmd = super::build_command("./configure", cfg.sandbox);
        cmd.args(&args).current_dir(src_dir);
        super::apply_musl_env(&mut cmd, cfg);
        super::run(cmd, "autotools configure")?;

        // make -jN
        let jobs = super::available_parallelism();
        let mut cmd = super::build_command("make", cfg.sandbox);
        cmd.arg(format!("-j{jobs}")).current_dir(src_dir);
        super::run(cmd, "autotools make")?;

        // make DESTDIR=<stage> install
        let mut cmd = super::build_command("make", cfg.sandbox);
        cmd.arg(format!("DESTDIR={}", stage.to_string_lossy()))
            .arg("install")
            .current_dir(src_dir);
        super::run(cmd, "autotools make install")?;

        Ok(stage)
    }
}

/// Run a bootstrap command with the same environment treatment as the
/// configure step (so musl tooling is visible to codegen/autoconf).
fn run_in(
    program: &str,
    args: &[&str],
    dir: &Path,
    cfg: &PackageConfig,
    label: &str,
) -> Result<()> {
    let mut cmd = super::build_command(program, cfg.sandbox);
    cmd.args(args).current_dir(dir);
    super::apply_musl_env(&mut cmd, cfg);
    super::run(cmd, label)
}
