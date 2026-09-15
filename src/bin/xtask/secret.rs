// SPDX-License-Identifier: GPL-3.0-or-later

//! `secret-scan` — block high-confidence credentials in the staged diff.
//!
//! Deliberately narrow: a handful of patterns for the credential types this
//! tool actually touches (`GITHUB_TOKEN`, cloud keys that might leak into a
//! pasted example or test fixture). Not a replacement for a real scanner —
//! near-zero false positives matter more for a pre-commit hook.

use std::process::Command;

use anyhow::{bail, Context, Result};
use regex::Regex;

const PATTERNS: &[&str] = &[
    r"ghp_[A-Za-z0-9]{36}",          // GitHub personal access token
    r"gh[oust]_[A-Za-z0-9]{36}",     // GitHub OAuth/user/server/refresh token
    r"github_pat_[A-Za-z0-9_]{22,}", // GitHub fine-grained PAT
    r"AKIA[0-9A-Z]{16}",             // AWS access key ID
    r"xox[baprs]-[A-Za-z0-9-]{10,}", // Slack token
    r"-----BEGIN (RSA|OPENSSH|EC|PGP|DSA) PRIVATE KEY-----",
];

pub fn run() -> Result<()> {
    // Only the staged diff, and only added lines (below). The xtask tree is
    // excluded so the pattern literals in this file can never self-match.
    let output = Command::new("git")
        .args([
            "diff",
            "--cached",
            "-U0",
            "--",
            ".",
            ":(exclude)src/bin/xtask/**",
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

    let regexes: Vec<Regex> = PATTERNS
        .iter()
        .map(|pattern| Regex::new(pattern).expect("valid secret pattern"))
        .collect();

    let diff = String::from_utf8_lossy(&output.stdout);
    let mut found = false;
    for line in diff.lines() {
        if !line.starts_with('+') || line.starts_with("+++") {
            continue;
        }
        for (pattern, regex) in PATTERNS.iter().zip(&regexes) {
            if regex.is_match(line) {
                println!("secret-scan: possible credential matching /{pattern}/ in staged changes");
                found = true;
            }
        }
    }

    if found {
        eprintln!("secret-scan: refusing to commit. If this is a false positive (e.g. a");
        eprintln!("  test fixture), rename the pattern to break the match or use");
        eprintln!("  'git commit --no-verify' deliberately.");
        bail!("possible credentials in staged changes");
    }
    Ok(())
}
