// SPDX-License-Identifier: GPL-3.0-or-later

//! Hex (Elixir/Erlang) registry source plugin.
//!
//! Fetches an Elixir package from hex.pm and produces a local directory of
//! files ready for packaging. Elixir projects use Mix as their build tool.
//!
//! For projects with prebuilt releases (mix release), the plugin stages the
//! release contents. For library-only packages, it stages the compiled BEAM
//! bytecode and source.
//!
//! Example:
//!   registry_source: hex
//!   github_repo: livebook
//!   version: 0.14.0

use anyhow::{bail, Context, Result};
use std::path::PathBuf;
use std::process::Command;

use crate::config::PackageConfig;
use crate::plugins::plugin::plugin_identity;
use crate::plugins::registry::{RegistryPayload, RegistrySource};

pub struct HexRegistrySource;

plugin_identity!(
    HexRegistrySource,
    "hex",
    "Hex (Elixir/Erlang) — mix deps.get + stage"
);

impl RegistrySource for HexRegistrySource {
    fn required_tools(&self) -> Vec<&'static str> {
        vec!["mix", "elixir"]
    }

    fn fetch(&self, package: &str, version: &str, cfg: &PackageConfig) -> Result<RegistryPayload> {
        let version_arg = if version.trim().is_empty() {
            "==".to_string()
        } else {
            format!("=={}", version.trim())
        };

        println!("hex: fetching {package} ({version_arg})");

        let workdir = tempfile::tempdir().context("failed to create hex workdir")?;
        let project_dir = workdir.path().join("project");
        std::fs::create_dir_all(&project_dir)?;

        // Create a minimal Mix project requiring the package.
        let mix_exs = format!(
            r#"defmodule LxTemp.MixProject do
  use Mix.Project

  def project do
    [
      app: :lx_temp,
      version: "0.1.0",
      elixir: "~> 1.15",
      deps: deps()
    ]
  end

  defp deps do
    [{{:{}, "{}"}}]
  end
end
"#,
            package.replace("-", "_"),
            version_arg
        );

        std::fs::write(project_dir.join("mix.exs"), mix_exs).context("failed to write mix.exs")?;

        // mix deps.get downloads and compiles dependencies.
        let deps_get = Command::new("mix")
            .args(["deps.get"])
            .current_dir(&project_dir)
            .output()
            .context("failed to run `mix deps.get` (is mix/elixir on PATH?)")?;

        if !deps_get.status.success() {
            bail!(
                "mix deps.get failed: {}",
                String::from_utf8_lossy(&deps_get.stderr)
            );
        }

        // Find the downloaded package in the deps directory.
        let deps_dir = project_dir.join("deps");
        if !deps_dir.is_dir() {
            bail!("mix produced no deps directory");
        };

        // The package directory name may differ from the package name
        // (e.g., "livebook" → "livebook", but some packages use
        // different directory names). Try exact match first, then fuzzy.
        let package_dir = find_hex_package_dir(&deps_dir, package);

        let files_dir = if let Some(dir) = package_dir {
            dir
        } else {
            deps_dir
        };

        // Resolve version from mix.lock or mix.exs.
        let resolved_version = extract_hex_version(&project_dir, package);

        println!("hex: staged {package} (version {resolved_version})");

        let description = cfg.description.clone();

        Ok(RegistryPayload {
            files_dir,
            resolved_version: resolved_version.to_string(),
            description,
        })
    }
}

/// Find the package directory in deps/.
fn find_hex_package_dir(deps_dir: &PathBuf, package: &str) -> Option<PathBuf> {
    if let Ok(entries) = std::fs::read_dir(deps_dir) {
        for entry in entries.flatten() {
            if entry.file_type().ok()?.is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name == package || name.replace('_', "-") == package {
                    return Some(entry.path());
                }
            }
        }
    }
    None
}

/// Extract the installed version from mix.lock.
fn extract_hex_version(project_dir: &std::path::Path, package: &str) -> String {
    let lock_file = project_dir.join("mix.lock");
    if !lock_file.exists() {
        return "0.0.0".to_string();
    }

    let Ok(text) = std::fs::read_to_string(&lock_file) else {
        return "0.0.0".to_string();
    };

    // mix.lock format: "package": {:hex, :"package", "version", ...}
    for line in text.lines() {
        if line.contains(package) {
            // Parse the version from the lock line.
            let parts: Vec<&str> = line.split('"').collect();
            for (i, part) in parts.iter().enumerate() {
                if *part == package && i + 2 < parts.len() {
                    return parts[i + 2].trim().to_string();
                }
            }
        }
    }

    "0.0.0".to_string()
}
