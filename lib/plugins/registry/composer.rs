// SPDX-License-Identifier: GPL-3.0-or-later

//! Composer (PHP) registry source plugin.
//!
//! Fetches a PHP package from Packagist (Composer) and produces a local
//! directory of files ready for packaging. Uses `composer install` with
//! a minimal composer.json to download and autoload the package.

use anyhow::{bail, Context, Result};
use std::path::PathBuf;
use std::process::Command;

use crate::config::PackageConfig;
use crate::plugins::registry::{RegistryPayload, RegistrySource};

pub struct ComposerRegistrySource;

impl RegistrySource for ComposerRegistrySource {
    fn name(&self) -> &'static str {
        "composer"
    }

    fn description(&self) -> &'static str {
        "Composer (PHP) — composer install + extract"
    }

    fn required_tools(&self) -> Vec<&'static str> {
        vec!["composer"]
    }

    fn fetch(&self, package: &str, version: &str, cfg: &PackageConfig) -> Result<RegistryPayload> {
        let version_constraint = if version.trim().is_empty() {
            "*".to_string()
        } else {
            version.trim().to_string()
        };

        println!("composer: fetching {package} ({version_constraint})");

        let workdir = tempfile::tempdir().context("failed to create composer workdir")?;
        let project_dir = workdir.path().join("project");
        std::fs::create_dir_all(&project_dir)?;

        // Create a minimal composer.json requiring the package.
        let composer_json = format!(
            r#"{{
    "name": "lx/temp",
    "description": "Temporary project for packaging",
    "require": {{
        "{}": "{}"
    }}
}}
"#,
            package, version_constraint
        );

        let composer_path = project_dir.join("composer.json");
        std::fs::write(&composer_path, composer_json)
            .context("failed to write composer.json")?;

        // composer install with --no-dev (production only), --no-interaction,
        // --no-scripts (avoid post-install scripts that may need network).
        let output = Command::new("composer")
            .args([
                "install",
                "--no-dev",
                "--no-interaction",
                "--no-scripts",
                "--no-progress",
                "--prefer-dist",
            ])
            .current_dir(&project_dir)
            .output()
            .context("failed to run `composer install` (is composer on PATH?)")?;

        if !output.status.success() {
            bail!(
                "composer install failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        // composer installs to <project>/vendor/<vendor>/<package>/.
        let vendor_dir = project_dir.join("vendor");
        if !vendor_dir.is_dir() {
            bail!("composer produced no vendor directory");
        }

        // Find the package in vendor (structure: vendor/vendor-name/package-name/).
        let package_dir = find_composer_package_dir(&vendor_dir, package);

        let files_dir = if let Some(dir) = package_dir {
            dir
        } else {
            // Use the whole vendor directory as fallback.
            vendor_dir
        };

        let version = extract_composer_version(&project_dir, package);

        println!("composer: staged {package} (version {version})");

        let description = cfg.description.clone();

        Ok(RegistryPayload {
            files_dir,
            resolved_version: version,
            description,
        })
    }
}

/// Find the package directory in vendor/.
/// Composer vendor structure: vendor/<vendor>/<package>/
fn find_composer_package_dir(vendor_dir: &PathBuf, package: &str) -> Option<PathBuf> {
    // The package name in composer is "vendor/package-name".
    // Try to find it by the second path component.
    let parts: Vec<&str> = package.split('/').collect();
    let pkg_name = parts.last()?;

    for entry in std::fs::read_dir(vendor_dir).ok()?.flatten() {
        if entry.file_type().ok()?.is_dir() {
            let candidate = entry.path().join(pkg_name);
            if candidate.is_dir() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Read the installed version from composer.lock.
fn extract_composer_version(project_dir: &PathBuf, package: &str) -> String {
    let lock_file = project_dir.join("composer.lock");
    if !lock_file.exists() {
        return "0.0.0".to_string();
    }

    let Ok(text) = std::fs::read_to_string(&lock_file) else {
        return "0.0.0".to_string();
    };

    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return "0.0.0".to_string();
    };

    // Check both "packages" and "packages-dev" (though we use --no-dev).
    for key in &["packages", "packages-dev"] {
        if let Some(arr) = value.get(key).and_then(|v| v.as_array()) {
            for pkg in arr {
                if let Some(name) = pkg.get("name").and_then(|n| n.as_str()) {
                    if name == package {
                        if let Some(ver) = pkg.get("version").and_then(|v| v.as_str()) {
                            // Composer versions often have "v" prefix.
                            return ver.trim_start_matches('v').to_string();
                        }
                    }
                }
            }
        }
    }

    "0.0.0".to_string()
}
