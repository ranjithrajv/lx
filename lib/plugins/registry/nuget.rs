// SPDX-License-Identifier: GPL-3.0-or-later

//! NuGet (.NET) registry source plugin.
//!
//! Fetches a .NET package from NuGet and produces a local directory of
//! files ready for packaging. Uses `nuget install` to download the .nupkg
//! and extract it.
//!
//! Note: .NET packages are typically libraries. For .NET *applications*
//! with self-contained deployments, the forge source (github) with release
//! assets is usually better.

use anyhow::{bail, Context, Result};
use std::process::Command;

use crate::config::PackageConfig;
use crate::plugins::registry::{RegistryPayload, RegistrySource};

pub struct NugetRegistrySource;

impl RegistrySource for NugetRegistrySource {
    fn name(&self) -> &'static str {
        "nuget"
    }

    fn description(&self) -> &'static str {
        "NuGet (.NET) — nuget install + extract"
    }

    fn required_tools(&self) -> Vec<&'static str> {
        vec!["nuget"]
    }

    fn fetch(&self, package: &str, version: &str, cfg: &PackageConfig) -> Result<RegistryPayload> {
        let spec = package.to_string();
        let version_args: Vec<String> = if version.trim().is_empty() {
            vec![]
        } else {
            vec!["-Version".to_string(), version.trim().to_string()]
        };

        println!("nuget: fetching {spec}");

        let workdir = tempfile::tempdir().context("failed to create nuget workdir")?;
        let download_dir = workdir.path().join("download");
        std::fs::create_dir_all(&download_dir)?;

        // nuget install downloads and extracts the package.
        // -OutputDirectory: where to extract.
        // -ExcludeVersion: don't create a version subdirectory.
        // -NoCache: don't use the local cache.
        let mut args = vec![
            "install".to_string(),
            spec.clone(),
            "-OutputDirectory".to_string(),
            download_dir.to_string_lossy().to_string(),
            "-ExcludeVersion".to_string(),
        ];
        args.extend(version_args);

        let output = Command::new("nuget")
            .args(&args)
            .output()
            .context("failed to run `nuget install` (is nuget on PATH?)")?;

        if !output.status.success() {
            bail!(
                "nuget install failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        // The package is extracted to <download_dir>/<PackageName>/
        // (even with -ExcludeVersion, the directory is named after the package).
        let package_dir = download_dir.join(package);
        let files_dir = if package_dir.is_dir() {
            package_dir
        } else {
            // Fallback: find the only directory in download_dir.
            let entries: Vec<std::path::PathBuf> = std::fs::read_dir(&download_dir)?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.is_dir())
                .collect();
            if entries.len() == 1 {
                entries.into_iter().next().unwrap()
            } else {
                download_dir
            }
        };

        let version = extract_nuget_version(&files_dir, package);

        println!("nuget: staged {spec} (version {version})");

        let description = cfg.description.clone();

        Ok(RegistryPayload {
            files_dir,
            resolved_version: version,
            description,
        })
    }
}

/// Read version from the .nupkg file or .nuspec inside the extracted dir.
fn extract_nuget_version(files_dir: &std::path::Path, _package: &str) -> String {
    // Look for .nuspec file which contains metadata.
    if let Ok(entries) = std::fs::read_dir(files_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".nuspec") {
                if let Ok(text) = std::fs::read_to_string(entry.path()) {
                    // Simple XML parsing for <version> tag.
                    for line in text.lines() {
                        let trimmed = line.trim();
                        if trimmed.starts_with("<version>") && trimmed.ends_with("</version>") {
                            let ver = trimmed
                                .trim_start_matches("<version>")
                                .trim_end_matches("</version>");
                            return ver.trim().to_string();
                        }
                    }
                }
            }
        }
    }

    "0.0.0".to_string()
}
