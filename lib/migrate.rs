// SPDX-License-Identifier: GPL-3.0-or-later

//! `lx migrate` — carry the installed base across the `lpt` → `lx` rename.
//!
//! Always runs the local state migration (idempotent): moves the install
//! manifest (`<data>/lpt/installed.json` → `<data>/lx/…`) and the
//! download/API caches (`~/.cache/lpt/…` → `~/.cache/lx/…`), but only when
//! the old location exists and the new one doesn't — never clobbers.
//! With `--repo DIR`, also rewrites a packaging repo's workflows:
//! `lpt` command → `lx`, `ranjithrajv/lpt@` action ref → `ranjithrajv/lx@`,
//! `lpt-cache-` keys and `~/.cache/lpt` paths likewise.

use anyhow::{Context, Result};
use clap::Args;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Args)]
pub struct MigrateArgs {
    /// Packaging repo directory whose `.github/workflows/*.yml` files
    /// should be rewritten from `lpt` to `lx`. Omit for local-state only.
    #[arg(long)]
    pub repo: Option<PathBuf>,
}

fn data_dir(name: &str) -> Option<PathBuf> {
    dirs::data_local_dir().map(|d| d.join(name))
}

fn cache_dir(name: &str) -> Option<PathBuf> {
    dirs::cache_dir().map(|d| d.join(name))
}

fn move_if_needed(old: PathBuf, new: &Path) -> Option<String> {
    if !old.exists() || new.exists() {
        return None;
    }
    if let Some(parent) = new.parent() {
        std::fs::create_dir_all(parent).ok()?;
    }
    // Prefer rename; fall back to copy+remove across filesystems.
    if std::fs::rename(&old, new).is_err() {
        copy_recursively(&old, new).ok()?;
        std::fs::remove_dir_all(&old).ok()?;
    }
    Some(format!("{} → {}", old.display(), new.display()))
}

fn copy_recursively(src: &std::path::Path, dst: &std::path::Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let d = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_recursively(&entry.path(), &d)?;
        } else {
            std::fs::copy(entry.path(), &d)?;
        }
    }
    Ok(())
}

pub fn run(args: MigrateArgs) -> Result<()> {
    let mut moved = Vec::new();

    // Install manifest.
    if let (Some(old_base), Some(new_base)) = (data_dir("lpt"), data_dir("lx")) {
        let old = old_base.join("installed.json");
        let new = new_base.join("installed.json");
        if let Some(m) = move_if_needed(old, &new) {
            moved.push(format!("manifest: {m}"));
        }
        // Remove the emptied legacy dir (harmless if non-empty: fails silently).
        let _ = std::fs::remove_dir(&old_base);
    }
    // Download + API caches.
    if let (Some(old_base), Some(new_base)) = (cache_dir("lpt"), cache_dir("lx")) {
        for sub in ["downloads", "api"] {
            if let Some(m) = move_if_needed(old_base.join(sub), &new_base.join(sub)) {
                moved.push(format!("cache: {m}"));
            }
        }
        let _ = std::fs::remove_dir(&old_base);
    }

    if moved.is_empty() {
        println!("nothing to migrate (no legacy lpt state found)");
    } else {
        for m in &moved {
            println!("migrated {m}");
        }
    }

    if let Some(repo) = &args.repo {
        let wf = repo.join(".github").join("workflows");
        if !wf.is_dir() {
            anyhow::bail!("'{}' has no .github/workflows directory", repo.display());
        }
        let mut rewritten = 0;
        let mut files: Vec<PathBuf> = std::fs::read_dir(&wf)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.extension()
                    .map(|e| e == "yml" || e == "yaml")
                    .unwrap_or(false)
            })
            .collect();
        files.sort();
        for f in files {
            let text = std::fs::read_to_string(&f)
                .with_context(|| format!("reading '{}'", f.display()))?;
            let mut out = text.replace("ranjithrajv/lpt@", "ranjithrajv/lx@");
            out = out.replace("~/.cache/lpt", "~/.cache/lx");
            out = out.replace("lpt-cache-", "lx-cache-");
            // `lpt <subcommand>` invocations → `lx <subcommand>`, without
            // touching words like `lpt-lib` (none exist in workflows).
            out = regex_replace_word(&out, "lpt", "lx");
            if out != text {
                std::fs::write(&f, out)?;
                println!("rewrote {}", f.display());
                rewritten += 1;
            }
        }
        println!("{rewritten} workflow file(s) rewritten; review the diff, then commit.");
    }
    Ok(())
}

/// Replace whole-word `from` with `to` (word = letters/digits/`-`/`_` run).
fn regex_replace_word(text: &str, from: &str, to: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let bytes = text.as_bytes();
    while i < bytes.len() {
        if text[i..].starts_with(from) {
            let before_ok = i == 0 || !word_char(bytes[i - 1]);
            let after = i + from.len();
            let after_ok = after >= bytes.len() || !word_char(bytes[after]);
            if before_ok && after_ok {
                out.push_str(to);
                i = after;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn word_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
}
