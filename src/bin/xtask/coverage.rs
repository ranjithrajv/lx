// SPDX-License-Identifier: GPL-3.0-or-later

//! `coverage` — run llvm-cov, report uncovered functions, gate on lines.
//!
//! Requires `cargo-llvm-cov` and builds the in-repo `utils/covscan` helper on
//! first use. The gate exits non-zero when overall line coverage drops below
//! `--min` (default 40%).

use std::fs::File;
use std::process::Command;

use anyhow::{bail, Context, Result};

pub fn run(min: u64) -> Result<()> {
    let root = crate::repo_root();
    let covscan_bin = root.join("utils/covscan/target/debug/covscan");
    if !covscan_bin.is_file() {
        run_command(
            Command::new("cargo")
                .arg("build")
                .arg("--manifest-path")
                .arg(root.join("utils/covscan/Cargo.toml"))
                .current_dir(&root),
        )?;
    }

    let tmp = tempfile::tempdir().context("creating coverage temp dir")?;
    let cov_json = tmp.path().join("cov.json");
    let fns_json = tmp.path().join("fns.json");

    // `cargo llvm-cov --json` writes the report to stdout; `--fail-under-lines`
    // makes it exit non-zero when the gate is missed.
    let min_arg = min.to_string();
    run_command(
        Command::new("cargo")
            .args([
                "llvm-cov",
                "--all-targets",
                "--fail-under-lines",
                &min_arg,
                "--json",
            ])
            .current_dir(&root)
            .stdout(File::create(&cov_json)?),
    )?;

    run_command(
        Command::new(&covscan_bin)
            .args(["src", "lib"])
            .current_dir(&root)
            .stdout(File::create(&fns_json)?),
    )?;

    crate::gaps::report(&fns_json, &cov_json)
}

fn run_command(cmd: &mut Command) -> Result<()> {
    let status = cmd
        .status()
        .with_context(|| format!("failed to run {cmd:?}"))?;
    if !status.success() {
        bail!("command failed ({status}): {cmd:?}");
    }
    Ok(())
}
