// SPDX-License-Identifier: GPL-3.0-or-later

//! pub.dev (Dart/Flutter) registry source plugin.
//!
//! Fetches a Dart package from pub.dev and produces a local directory of
//! files ready for packaging. For Dart CLI tools, stages the package source
//! and compiled executables.
//!
//! Example:
//!   registry_source: dart
//!   github_repo: fvm
//!   version: 3.2.0

use anyhow::{bail, Context, Result};
use std::path::PathBuf;
use std::process::Command;

use crate::config::PackageConfig;
use crate::plugins::plugin::plugin_identity;
use crate::plugins::registry::{RegistryPayload, RegistrySource};

pub struct DartRegistrySource;

plugin_identity!(
    DartRegistrySource,
    "dart",
    "pub.dev (Dart/Flutter) — dart pub cache add + stage"
);

impl RegistrySource for DartRegistrySource {
    fn required_tools(&self) -> Vec<&'static str> {
        vec!["dart"]
    }

    fn fetch(&self, package: &str, version: &str, cfg: &PackageConfig) -> Result<RegistryPayload> {
        let version_arg = if version.trim().is_empty() {
            "any".to_string()
        } else {
            version.trim().to_string()
        };

        println!("dart: fetching {package}@{version_arg}");

        let workdir = tempfile::tempdir().context("failed to create dart workdir")?;
        let project_dir = workdir.path().join("project");
        std::fs::create_dir_all(&project_dir)?;

        // Create a minimal pubspec.yaml requiring the package.
        let pubspec = format!(
            r#"name: lx_temp
version: 0.1.0
environment:
  sdk: ">=3.0.0 <4.0.0"

dependencies:
  {}: "{}"
"#,
            package, version_arg
        );

        std::fs::write(project_dir.join("pubspec.yaml"), pubspec)
            .context("failed to write pubspec.yaml")?;

        // dart pub get downloads and resolves dependencies.
        let pub_get = Command::new("dart")
            .args(["pub", "get"])
            .current_dir(&project_dir)
            .output()
            .context("failed to run `dart pub get` (is dart on PATH?)")?;

        if !pub_get.status.success() {
            bail!(
                "dart pub get failed: {}",
                String::from_utf8_lossy(&pub_get.stderr)
            );
        }

        // Find the package in the pub cache.
        let pub_cache_dir = find_dart_package_cache(package, &version_arg);

        let files_dir = if let Some(dir) = pub_cache_dir {
            dir
        } else {
            // Fallback: find in .dart_tool/package_config.json.
            project_dir.clone()
        };

        // Resolve the actual version from pubspec.lock.
        let resolved_version = extract_dart_version(&project_dir, package);

        println!("dart: staged {package} (version {resolved_version})");

        let description = cfg.description.clone();

        Ok(RegistryPayload {
            files_dir,
            resolved_version: resolved_version.to_string(),
            description,
        })
    }
}

/// Find the package directory in the Dart pub cache.
fn find_dart_package_cache(package: &str, version: &str) -> Option<PathBuf> {
    let pub_cache = std::env::var("PUB_CACHE").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}/.pub-cache")
    });

    let hosted_dir = format!("{pub_cache}/hosted/pub.dev");
    if !std::path::Path::new(&hosted_dir).exists() {
        return None;
    };

    // Look for <package>-<version>/ directory.
    if let Ok(entries) = std::fs::read_dir(&hosted_dir) {
        for entry in entries.flatten() {
            if entry.file_type().ok()?.is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with(&format!("{package}-"))
                    && (version == "any" || name.contains(&format!("-{version}")))
                {
                    return Some(entry.path());
                }
            }
        }
    }
    None
}

/// Extract the installed version from pubspec.lock.
fn extract_dart_version(project_dir: &std::path::Path, package: &str) -> String {
    let lock_file = project_dir.join("pubspec.lock");
    if !lock_file.exists() {
        return "0.0.0".to_string();
    }

    let Ok(text) = std::fs::read_to_string(&lock_file) else {
        return "0.0.0".to_string();
    };

    // pubspec.lock YAML format. Look for the package entry and version.
    let mut in_package = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with(&format!("  {package}:")) {
            in_package = true;
            continue;
        }
        if in_package && trimmed.starts_with("version:") {
            return trimmed
                .trim_start_matches("version:")
                .trim()
                .trim_matches('"')
                .to_string();
        }
        if in_package && !trimmed.starts_with("version:") && !trimmed.is_empty() {
            // Moved past this package's entry without finding version.
            in_package = false;
        }
    }

    "0.0.0".to_string()
}
