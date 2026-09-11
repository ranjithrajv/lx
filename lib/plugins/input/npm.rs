// SPDX-License-Identifier: GPL-3.0-or-later

//! npm (Node.js) input source plugin.
//!
//! Fetches an npm package and produces a local directory of files ready
//! for packaging. Uses `npm pack` to download the tarball, then extracts
//! it into a staging directory.

use anyhow::{bail, Context, Result};
use std::path::PathBuf;
use std::process::Command;

use crate::config::PackageConfig;
use crate::plugins::input::{InputPayload, InputSource};

pub struct NpmInputSource;

impl InputSource for NpmInputSource {
    fn name(&self) -> &'static str {
        "npm"
    }

    fn description(&self) -> &'static str {
        "npm (Node.js) — npm pack + extract"
    }

    fn required_tools(&self) -> Vec<&'static str> {
        vec!["npm"]
    }

    fn fetch(&self, package: &str, version: &str, cfg: &PackageConfig) -> Result<InputPayload> {
        let spec = if version.trim().is_empty() {
            package.to_string()
        } else {
            format!("{}@{}", package, version)
        };

        println!("npm: fetching {spec}");

        let workdir = tempfile::tempdir().context("failed to create npm workdir")?;

        // npm pack downloads the tarball without installing. --dry-run would
        // skip the download; we want the actual tarball.
        let pack_output = workdir.path().join("pack");
        std::fs::create_dir_all(&pack_output)?;

        let output = Command::new("npm")
            .args([
                "pack",
                &spec,
                "--pack-destination",
                &pack_output.to_string_lossy(),
            ])
            .output()
            .context("failed to run `npm pack` (is npm on PATH?)")?;

        if !output.status.success() {
            bail!(
                "npm pack failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        // npm pack produces exactly one tarball: "<name>-<version>.tgz"
        let tarball = std::fs::read_dir(&pack_output)?
            .filter_map(|e| e.ok())
            .find(|e| e.file_name().to_string_lossy().ends_with(".tgz"))
            .context("npm pack produced no tarball")?;

        let tarball_path = tarball.path();
        let file_name = tarball.file_name();
        let file_name = file_name.to_string_lossy();

        // Extract version from filename: "name-version.tgz"
        let version = file_name
            .rsplit_once('-')
            .and_then(|(_, rest)| rest.strip_suffix(".tgz"))
            .unwrap_or("0.0.0")
            .to_string();

        println!("npm: extracted {file_name} (version {version})");

        // Extract the tarball.
        let extract_dir = workdir.path().join("package");
        std::fs::create_dir_all(&extract_dir)?;
        crate::build::extract(&tarball_path, &extract_dir, "tar.gz")
            .context("failed to extract npm tarball")?;

        // npm tarballs nest everything under "package/". Use that if it
        // exists and is the only entry.
        let package_dir = extract_dir.join("package");
        let files_dir = if package_dir.is_dir() && package_dir != extract_dir {
            package_dir
        } else {
            extract_dir
        };

        // Try to read description from package.json.
        let description = read_package_description(&files_dir, cfg);

        Ok(InputPayload {
            files_dir,
            resolved_version: version,
            description,
        })
    }
}

/// Try to read the description field from package.json in the extracted dir.
fn read_package_description(files_dir: &PathBuf, cfg: &PackageConfig) -> String {
    if !cfg.description.is_empty() {
        return cfg.description.clone();
    }
    let pkg_json = files_dir.join("package.json");
    let Ok(text) = std::fs::read_to_string(&pkg_json) else {
        return String::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return String::new();
    };
    value
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}
