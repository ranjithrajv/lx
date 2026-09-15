// SPDX-License-Identifier: GPL-3.0-or-later

//! `dev-setup` — install the developer tools the pre-commit hooks need.
//!
//! Idempotent: anything already on `PATH` is skipped. Replaces the former
//! `make dev-setup` shell recipe.

use std::process::Command;

use anyhow::{bail, Context, Result};

pub fn run() -> Result<()> {
    println!("==> Installing dev tools for lx...");

    install_if_missing("pre-commit", &["pip", "install", "pre-commit"])?;
    install_if_missing("cargo-deny", &["cargo", "install", "cargo-deny"])?;
    install_if_missing("typos", &["cargo", "install", "typos-cli"])?;
    install_if_missing("taplo", &["cargo", "install", "taplo-cli", "--locked"])?;
    install_if_missing(
        "markdownlint",
        &["npm", "install", "-g", "markdownlint-cli"],
    )?;

    println!();
    println!("==> Done. Now run:");
    println!("    pre-commit install");
    println!("    pre-commit install --hook-type pre-push");
    Ok(())
}

fn install_if_missing(bin: &str, install: &[&str]) -> Result<()> {
    if crate::which(bin).is_some() {
        println!("  {bin}: already installed");
        return Ok(());
    }
    println!("  {bin}: installing...");
    let (program, args) = install.split_first().expect("non-empty install command");
    let status = Command::new(program)
        .args(args)
        .status()
        .with_context(|| format!("running `{program}`"))?;
    if !status.success() {
        bail!("failed to install {bin}");
    }
    Ok(())
}
