// SPDX-License-Identifier: GPL-3.0-or-later

//! `policy` — enforce the Rust-only development policy.
//!
//! `lx` ships as one Rust workspace. Developer tooling must be Rust too, so
//! this check fails if the tree gains a shell script (by extension or by
//! `#!` shebang) or a non-Rust script helper. New tooling belongs in
//! [`src/bin/xtask`](../main.rs) as a subcommand.
//!
//! GitHub Actions YAML is exempt: composite actions and workflows have no
//! shell-free form, so their inline `run:` snippets are allowed as long as
//! they stay thin wrappers over `cargo xtask` / `lx`.

use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};

const SHELL_EXTENSIONS: &[&str] = &["sh", "bash", "zsh", "ksh", "dash", "fish", "csh", "tcsh"];

/// Non-shell scripting helpers that violate the Rust-only rule (we replaced
/// `utils/coverage-gaps.py` for this reason).
const OTHER_SCRIPT_EXTENSIONS: &[&str] = &["py", "pyw", "rb", "pl", "pm"];

const SHELLS: &[&str] = &["sh", "bash", "zsh", "ksh", "dash", "fish", "csh", "tcsh"];

pub fn run() -> Result<()> {
    let output = Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(crate::repo_root())
        .output()
        .context("failed to run `git ls-files`")?;
    if !output.status.success() {
        bail!(
            "`git ls-files` failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let mut offenders = Vec::new();
    for file in output.stdout.split(|b| *b == 0).filter(|s| !s.is_empty()) {
        let rel = String::from_utf8_lossy(file).to_string();
        if let Some(reason) = violation(&rel) {
            offenders.push((rel, reason));
        }
    }

    if offenders.is_empty() {
        println!("policy: OK — no shell scripts or non-Rust tooling found");
        return Ok(());
    }

    for (path, reason) in &offenders {
        eprintln!("policy: {path}: {reason}");
    }
    eprintln!(
        "policy: the project is Rust-only. Write tooling as a `cargo xtask` \
         subcommand (src/bin/xtask/) instead of a script."
    );
    bail!("{} non-Rust script file(s) found", offenders.len());
}

fn violation(rel: &str) -> Option<&'static str> {
    let path = Path::new(rel);
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        let ext = ext.to_ascii_lowercase();
        if SHELL_EXTENSIONS.contains(&ext.as_str()) {
            return Some("shell script (by extension)");
        }
        if OTHER_SCRIPT_EXTENSIONS.contains(&ext.as_str()) {
            return Some("non-Rust script helper (by extension)");
        }
    }
    if let Some(line) = first_line(path) {
        if shebang_shell(&line).is_some() {
            return Some("shell script (by shebang)");
        }
    }
    None
}

/// The first line of `path`, or `None` if it cannot be read.
fn first_line(path: &Path) -> Option<String> {
    use std::io::Read;

    let mut file = std::fs::File::open(path).ok()?;
    let mut buf = [0u8; 256];
    let n = file.read(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf[..n]);
    text.lines().next().map(str::to_string)
}

/// The shell named by a `#!` line, if any (`#!/usr/bin/env bash` → `bash`).
fn shebang_shell(line: &str) -> Option<String> {
    let rest = line.strip_prefix("#!")?;
    for token in rest.split_whitespace().rev() {
        let base = token.rsplit('/').next().unwrap_or(token);
        if SHELLS.contains(&base) {
            return Some(base.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_detection() {
        assert!(violation("utils/old.sh").is_some());
        assert!(violation("tool.py").is_some());
        assert!(violation("lib/build.rs").is_none());
        assert!(violation("docs/README.md").is_none());
    }

    #[test]
    fn shebang_detection() {
        assert_eq!(shebang_shell("#!/bin/sh").as_deref(), Some("sh"));
        assert_eq!(
            shebang_shell("#!/usr/bin/env bash").as_deref(),
            Some("bash")
        );
        assert_eq!(
            shebang_shell("#!/usr/bin/env -S bash -e").as_deref(),
            Some("bash")
        );
        assert_eq!(shebang_shell("#!/usr/bin/python3"), None);
        assert_eq!(shebang_shell("not a shebang"), None);
    }
}
