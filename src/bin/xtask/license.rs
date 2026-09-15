// SPDX-License-Identifier: GPL-3.0-or-later

//! `license-check` — require the SPDX header on staged `.rs` files.
//!
//! Only staged files are checked so third-party or generated files outside
//! the commit are never flagged.

use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};

const SPDX: &str = "SPDX-License-Identifier: GPL-3.0-or-later";

pub fn run() -> Result<()> {
    let output = Command::new("git")
        .args([
            "diff",
            "--cached",
            "--name-only",
            "--diff-filter=ACM",
            "--",
            "*.rs",
        ])
        .current_dir(crate::repo_root())
        .output()
        .context("failed to run `git diff --cached`")?;
    if !output.status.success() {
        bail!(
            "`git diff --cached` failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let mut missing = Vec::new();
    for file in String::from_utf8_lossy(&output.stdout).lines() {
        if file.trim().is_empty() || !Path::new(file).is_file() {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        if !text.lines().take(5).any(|line| line.contains(SPDX)) {
            missing.push(file.to_string());
        }
    }

    if missing.is_empty() {
        return Ok(());
    }
    for file in &missing {
        eprintln!("license-check: missing '// {SPDX}' header in {file}");
    }
    eprintln!("license-check: add '// {SPDX}' near the top of the file.");
    bail!("{} file(s) missing the license header", missing.len());
}
