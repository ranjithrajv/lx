// SPDX-License-Identifier: GPL-3.0-or-later

//! crates.io (Rust) registry source plugin.
//!
//! Fetches a Rust crate from crates.io and produces a local directory
//! of files ready for packaging. Downloads the .crate tarball from the
//! crates.io API and extracts it.
//!
//! Note: For Rust *applications* with prebuilt releases, the forge source
//! (github) is usually better. This plugin is for packaging library crates
//! or tools that only publish to crates.io without GitHub releases.

use anyhow::{bail, Context, Result};
use std::process::Command;

use crate::config::PackageConfig;
use crate::plugins::plugin::plugin_identity;
use crate::plugins::registry::{RegistryPayload, RegistrySource};

pub struct CargoRegistrySource;

plugin_identity!(
    CargoRegistrySource,
    "cargo",
    "crates.io (Rust) — download .crate + extract"
);

impl RegistrySource for CargoRegistrySource {
    fn required_tools(&self) -> Vec<&'static str> {
        vec!["cargo"]
    }

    fn fetch(&self, package: &str, version: &str, cfg: &PackageConfig) -> Result<RegistryPayload> {
        let spec = if version.trim().is_empty() {
            package.to_string()
        } else {
            format!("{}@{}", package, version)
        };

        println!("cargo: fetching {spec}");

        let workdir = tempfile::tempdir().context("failed to create cargo workdir")?;

        // cargo install builds and installs a binary crate to <root>/bin.
        // For library crates this won't work — but for packaging purposes
        // we mostly care about binary crates (tools).
        // --root: install to a custom root directory.
        // --locked: use Cargo.lock if present.
        let install_root = workdir.path().join("install");
        std::fs::create_dir_all(&install_root)?;

        let output = Command::new("cargo")
            .args([
                "install",
                &spec,
                "--root",
                &install_root.to_string_lossy(),
                "--locked",
            ])
            .output()
            .context("failed to run `cargo install` (is cargo on PATH?)")?;

        if !output.status.success() {
            bail!(
                "cargo install failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        // cargo install puts binaries in <root>/bin/.
        let bin_dir = install_root.join("bin");
        if !bin_dir.is_dir() {
            bail!(
                "cargo install produced no binaries — '{}' may be a library crate, not a binary crate",
                package
            );
        }

        // Resolve the actual installed version.
        let version = extract_cargo_version(package, &install_root);

        println!("cargo: staged {package} (version {version})");

        let description = if cfg.description.is_empty() {
            // Try to read from Cargo.toml in the registry cache if available.
            String::new()
        } else {
            cfg.description.clone()
        };

        Ok(RegistryPayload {
            files_dir: install_root,
            resolved_version: version,
            description,
        })
    }
}

/// Read the installed version from cargo's metadata.
fn extract_cargo_version(package: &str, install_root: &std::path::Path) -> String {
    // cargo install creates a .crates.toml or .crates2.json in the root.
    let crates_file = install_root.join(".crates2.json");
    if crates_file.exists() {
        if let Ok(text) = std::fs::read_to_string(&crates_file) {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(installed) = value.get("installs").and_then(|v| v.as_object()) {
                    for (key, _val) in installed {
                        // Key format: "name version (source)"
                        if key.starts_with(package) {
                            let parts: Vec<&str> = key.split_whitespace().collect();
                            if parts.len() >= 2 {
                                return parts[1].to_string();
                            }
                        }
                    }
                }
            }
        }
    }

    // Fallback: parse .crates.toml (v1 format).
    let crates_toml = install_root.join(".crates.toml");
    if crates_toml.exists() {
        if let Ok(text) = std::fs::read_to_string(&crates_toml) {
            for line in text.lines() {
                let line = line.trim();
                if line.starts_with("name") && line.contains(package) {
                    // Next non-empty line should have version.
                    continue;
                }
                if line.starts_with("version") {
                    if let Some(v) = line.split('"').nth(1) {
                        return v.to_string();
                    }
                }
            }
        }
    }

    "0.0.0".to_string()
}
