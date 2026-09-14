// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared plumbing for tool-backed [`RegistrySource`] plugins.
//!
//! Most registry sources drive an ecosystem CLI to download a package, then
//! turn the downloaded artifact into a payload directory. The mechanics are
//! identical every time — only the command line, the archive extensions, and
//! the metadata parser differ. This module holds the common pieces so each
//! plugin is reduced to its ecosystem-specific bits:
//!
//! * [`run_tool`] — run an external command, failing with its stderr.
//! * [`find_downloaded_archive`] — pick the downloaded file by extension.
//! * [`extract_payload`] — extract it and unwrap a lone top-level directory.
//! * [`description_or`] — the "configured description wins" rule.
//!
//! Sources whose tool *builds* rather than *downloads* (`cargo install`,
//! `cpanm`, `go build`, `dart pub cache`) still use [`run_tool`]; only their
//! staging differs.
//!
//! [`RegistrySource`]: super::RegistrySource

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

use crate::config::PackageConfig;

/// Run `program args…` (optionally in `current_dir`) and fail with the tool's
/// stderr when it exits non-zero. `what` names the step for error messages
/// (e.g. `"npm pack"`).
pub fn run_tool(
    program: &str,
    args: &[&str],
    current_dir: Option<&Path>,
    what: &str,
) -> Result<std::process::Output> {
    let mut cmd = std::process::Command::new(program);
    cmd.args(args);
    if let Some(dir) = current_dir {
        cmd.current_dir(dir);
    }
    let output = cmd
        .output()
        .with_context(|| format!("failed to run `{program}` (is it on PATH?)"))?;
    if !output.status.success() {
        bail!("{what} failed: {}", String::from_utf8_lossy(&output.stderr));
    }
    Ok(output)
}

/// Find the first entry in `dir` whose name ends with one of `extensions`,
/// returning its path and file name. `what` names the download step.
pub fn find_downloaded_archive(
    dir: &Path,
    extensions: &[&str],
    what: &str,
) -> Result<(PathBuf, String)> {
    let entry = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .find(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            extensions.iter().any(|ext| name.ends_with(ext))
        })
        .with_context(|| format!("{what} produced no matching archive"))?;
    Ok((
        entry.path(),
        entry.file_name().to_string_lossy().to_string(),
    ))
}

/// Extract `archive` (as `format`) under `workdir/package`, returning the
/// payload root: the lone top-level directory when the archive nests under a
/// single `<name>-<version>/`, else the extraction directory itself.
pub fn extract_payload(
    workdir: &Path,
    archive: &Path,
    format: &str,
    what: &str,
) -> Result<PathBuf> {
    let extract_dir = workdir.join("package");
    std::fs::create_dir_all(&extract_dir)?;
    crate::build::extract(archive, &extract_dir, format)
        .with_context(|| format!("failed to extract {what}"))?;
    Ok(unwrap_lone_dir(&extract_dir))
}

/// If `dir` holds exactly one top-level directory, return it; otherwise
/// return `dir`. Release tarballs conventionally nest under one.
pub fn unwrap_lone_dir(dir: &Path) -> PathBuf {
    let dirs: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    match dirs.as_slice() {
        [only] => only.clone(),
        _ => dir.to_path_buf(),
    }
}

/// The description to record: the configured one when set, else `detect()`.
pub fn description_or(cfg: &PackageConfig, detect: impl FnOnce() -> String) -> String {
    if cfg.description.is_empty() {
        detect()
    } else {
        cfg.description.clone()
    }
}
