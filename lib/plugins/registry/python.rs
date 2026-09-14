// SPDX-License-Identifier: GPL-3.0-or-later

//! Python (pip) input source plugin.
//!
//! Downloads a Python package via pip and produces a local directory of
//! files ready for packaging. Uses `pip download --no-binary :all:` for
//! sdists or `pip download` for wheels, then extracts into a staging
//! directory.

use anyhow::{Context, Result};

use crate::config::PackageConfig;
use crate::plugins::plugin::plugin_identity;
use crate::plugins::registry::{staging, RegistryPayload, RegistrySource};

pub struct PythonRegistrySource;

plugin_identity!(
    PythonRegistrySource,
    "python",
    "Python (pip) — pip download + extract"
);

impl RegistrySource for PythonRegistrySource {
    fn required_tools(&self) -> Vec<&'static str> {
        vec!["pip", "python3"]
    }

    fn fetch(&self, package: &str, version: &str, cfg: &PackageConfig) -> Result<RegistryPayload> {
        let spec = if version.trim().is_empty() {
            package.to_string()
        } else {
            format!("{}=={}", package, version)
        };

        println!("python: fetching {spec}");

        let workdir = tempfile::tempdir().context("failed to create python workdir")?;
        let download_dir = workdir.path().join("download");
        std::fs::create_dir_all(&download_dir)?;
        let dest = download_dir.to_string_lossy().to_string();

        // pip download fetches the package without installing.
        // --no-binary :all: forces sdist (source distribution) which is
        // more portable than wheels (wheels are platform-specific).
        // --no-deps: we only package this one package, not its deps.
        staging::run_tool(
            "pip",
            &[
                "download",
                &spec,
                "--no-binary",
                ":all:",
                "--no-deps",
                "-d",
                &dest,
            ],
            None,
            "pip download",
        )?;

        // Find the downloaded archive (sdist: .tar.gz, .zip, or .tar.bz2).
        let (archive_path, file_name) = staging::find_downloaded_archive(
            &download_dir,
            &[".tar.gz", ".tgz", ".zip", ".tar.bz2"],
            "pip download",
        )?;

        // Extract version from filename: "name-version.tar.gz" or "name-version.zip".
        let version = extract_sdist_version(&file_name).unwrap_or("0.0.0");

        println!("python: extracted {file_name} (version {version})");

        // Determine archive format.
        let format = if file_name.ends_with(".zip") {
            "zip"
        } else {
            "tar.gz"
        };

        // sdists nest under "<name>-<version>/".
        let files_dir =
            staging::extract_payload(workdir.path(), &archive_path, format, "python sdist")?;

        let description = staging::description_or(cfg, || read_sdist_description(&files_dir));

        Ok(RegistryPayload {
            files_dir,
            resolved_version: version.to_string(),
            description,
        })
    }
}

/// Extract version from an sdist filename like "foo-1.2.3.tar.gz".
fn extract_sdist_version(file_name: &str) -> Option<&str> {
    let stem = file_name
        .strip_suffix(".tar.gz")
        .or_else(|| file_name.strip_suffix(".tgz"))
        .or_else(|| file_name.strip_suffix(".zip"))
        .or_else(|| file_name.strip_suffix(".tar.bz2"))?;
    // Find the last "-" that separates name from version.
    // sdist naming: name-version where name may contain hyphens but
    // version starts with a digit.
    for (i, c) in stem.char_indices().rev() {
        if c == '-' && i + 1 < stem.len() && stem[i + 1..].starts_with(|c: char| c.is_ascii_digit())
        {
            return Some(&stem[i + 1..]);
        }
    }
    None
}

/// Try to read description from setup.cfg or pyproject.toml in the sdist.
fn read_sdist_description(files_dir: &std::path::Path) -> String {
    // Try pyproject.toml first (simple key-value parsing to avoid a
    // toml dependency; we only need the description field).
    let pyproject = files_dir.join("pyproject.toml");
    if let Ok(text) = std::fs::read_to_string(&pyproject) {
        let mut in_project = false;
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed == "[project]" {
                in_project = true;
                continue;
            }
            if trimmed.starts_with('[') {
                in_project = false;
                continue;
            }
            if in_project {
                if let Some((key, value)) = trimmed.split_once('=') {
                    if key.trim() == "description" {
                        return value
                            .trim()
                            .trim_matches('"')
                            .trim_matches('\'')
                            .to_string();
                    }
                }
            }
        }
    }

    // Fall back to setup.cfg.
    let setup_cfg = files_dir.join("setup.cfg");
    if let Ok(text) = std::fs::read_to_string(&setup_cfg) {
        for line in text.lines() {
            if let Some((key, value)) = line.split_once('=') {
                if key.trim() == "description" {
                    return value.trim().to_string();
                }
            }
        }
    }

    // Fall back to setup.py (look for description= in the setup() call).
    let setup_py = files_dir.join("setup.py");
    if let Ok(text) = std::fs::read_to_string(&setup_py) {
        for line in text.lines() {
            let trimmed = line.trim();
            if let Some((key, value)) = trimmed.split_once('=') {
                if key.trim() == "description" {
                    let val = value
                        .trim()
                        .trim_matches(',')
                        .trim_matches('"')
                        .trim_matches('\'');
                    return val.to_string();
                }
            }
        }
    }

    String::new()
}
