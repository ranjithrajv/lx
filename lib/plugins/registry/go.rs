// SPDX-License-Identifier: GPL-3.0-or-later

//! Go modules (pkg.go.dev) registry source plugin.
//!
//! Fetches a Go module from the module proxy and produces a local directory
//! containing a statically-linked binary ready for packaging.
//!
//! This is for Go CLI tools that publish only to the module proxy without
//! GitHub releases. The plugin downloads the module, builds a static binary
//! with CGO_ENABLED=0, and stages it.
//!
//! Example:
//!   registry_source: go
//!   github_repo: github.com/jesseduffield/lazygit  # module path
//!   version: latest

use anyhow::{bail, Context, Result};
use std::process::Command;

use crate::config::PackageConfig;
use crate::plugins::plugin::plugin_identity;
use crate::plugins::registry::{RegistryPayload, RegistrySource};

pub struct GoRegistrySource;

plugin_identity!(
    GoRegistrySource,
    "go",
    "Go modules (pkg.go.dev) — go build static binary"
);

impl RegistrySource for GoRegistrySource {
    fn required_tools(&self) -> Vec<&'static str> {
        vec!["go"]
    }

    fn fetch(&self, package: &str, version: &str, cfg: &PackageConfig) -> Result<RegistryPayload> {
        let module_path = if package.contains('/') {
            package.to_string()
        } else {
            // Simple name, assume github.com (rare but possible)
            package.to_string()
        };

        println!("go modules: fetching {module_path}");

        let workdir = tempfile::tempdir().context("failed to create go workdir")?;

        // Initialize a minimal module so we can `go get` the dependency.
        let mod_init = Command::new("go")
            .args(["mod", "init", "lx-temp"])
            .current_dir(workdir.path())
            .output()
            .context("failed to run `go mod init`")?;

        if !mod_init.status.success() {
            bail!(
                "go mod init failed: {}",
                String::from_utf8_lossy(&mod_init.stderr)
            );
        }

        // Add the dependency. `go get` downloads and adds to go.mod.
        let dep_spec = if version.trim().is_empty() {
            module_path.clone()
        } else {
            format!("{module_path}@{}", version.trim())
        };

        let go_get = Command::new("go")
            .args(["get", "-d", &dep_spec])
            .current_dir(workdir.path())
            .output()
            .context("failed to run `go get`")?;

        if !go_get.status.success() {
            bail!("go get failed: {}", String::from_utf8_lossy(&go_get.stderr));
        }

        // Find the module in the build list to get the resolved version.
        let list_output = Command::new("go")
            .args(["list", "-m", "-json", &module_path])
            .current_dir(workdir.path())
            .output()
            .context("failed to run `go list -m`")?;

        let resolved_version = if list_output.status.success() {
            let info: serde_json::Value =
                serde_json::from_slice(&list_output.stdout).unwrap_or_default();
            info.get("Version")
                .and_then(|v| v.as_str())
                .unwrap_or("0.0.0")
                .to_string()
        } else {
            version.trim().to_string()
        };

        // Build a static binary. CGO_ENABLED=0 produces a fully static binary
        // with no glibc dependency — runs on any Linux.
        let bin_name = cfg.package_name.clone();
        let bin_output = workdir.path().join("bin");
        std::fs::create_dir_all(&bin_output)?;
        let bin_path = bin_output.join(&bin_name);

        // Find the main package. Common patterns:
        //   github.com/user/repo          (root is main)
        //   github.com/user/repo/cmd/tool  (cmd/tool is main)
        let main_packages = find_main_packages(&module_path);

        if main_packages.is_empty() {
            bail!(
                "go: no main package found in {module_path} — this appears to be a library, not a CLI tool. Use forge source for projects with GitHub releases."
            );
        }

        // Build the first main package found.
        let main_pkg = &main_packages[0];
        println!("go modules: building {main_pkg}");

        let build_output = Command::new("go")
            .args([
                "build",
                "-trimpath",
                "-ldflags",
                "-s -w",
                "-o",
                &bin_path.to_string_lossy(),
                main_pkg,
            ])
            .env("CGO_ENABLED", "0")
            .current_dir(workdir.path())
            .output()
            .context("failed to run `go build`")?;

        if !build_output.status.success() {
            bail!(
                "go build failed: {}",
                String::from_utf8_lossy(&build_output.stderr)
            );
        }

        if !bin_path.exists() {
            bail!("go build produced no binary");
        }

        println!(
            "go modules: built static binary {} (version {resolved_version})",
            bin_path.display()
        );

        let description = if cfg.description.is_empty() {
            format!("Go CLI tool from {module_path}")
        } else {
            cfg.description.clone()
        };

        Ok(RegistryPayload {
            files_dir: bin_output,
            resolved_version,
            description,
        })
    }
}

/// Find main packages in the downloaded module by scanning go source files.
fn find_main_packages(module_path: &str) -> Vec<String> {
    let gomodcache = match std::env::var("GOMODCACHE") {
        Ok(p) => p,
        Err(_) => {
            // Default: ~/go/pkg/mod
            let home = std::env::var("HOME").unwrap_or_default();
            format!("{home}/go/pkg/mod")
        }
    };

    // Find the module directory in the cache.
    // Module path uses '/' as separator (e.g. github.com/user/repo).
    let mod_dir_prefix = module_path.to_string();
    let mut found_dir = None;

    if let Ok(entries) = std::fs::read_dir(&gomodcache) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            // Module dirs: github.com/user/repo@v1.2.3 or github.com/user/repo@latest
            if name.starts_with(&mod_dir_prefix) {
                found_dir = Some(entry.path());
                break;
            }
        }
    }

    let mod_dir = match found_dir {
        Some(d) => d,
        None => return Vec::new(),
    };

    // Scan for files with `package main`.
    let mut main_packages = Vec::new();
    scan_for_main_packages(&mod_dir, module_path, &mod_dir, &mut main_packages);

    // Also check common cmd/ subdirectories for the repo root pattern.
    let mut extras = Vec::new();
    for suffix in &["", "/cmd"] {
        let candidate = if suffix.is_empty() {
            module_path.to_string()
        } else {
            format!("{module_path}{suffix}")
        };
        if !main_packages.contains(&candidate) {
            // Check if any main package starts with this prefix.
            for mp in &main_packages {
                if mp.starts_with(&candidate) {
                    extras.push(candidate.clone());
                    break;
                }
            }
        }
    }
    main_packages.extend(extras);

    // Deduplicate and prefer shorter paths (root main > cmd/tool main).
    main_packages.sort_by_key(|p| p.len());
    main_packages.dedup();
    main_packages
}

/// Recursively scan a directory for Go files with `package main`.
fn scan_for_main_packages(
    dir: &std::path::Path,
    module_path: &str,
    base: &std::path::Path,
    results: &mut Vec<String>,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };

        if file_type.is_dir() {
            // Skip vendor, .git, and testdata directories.
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if name == "vendor" || name == ".git" || name == "testdata" {
                continue;
            }
            scan_for_main_packages(&path, module_path, base, results);
            continue;
        }

        if !file_type.is_file() {
            continue;
        }

        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if !name.ends_with(".go") || name.ends_with("_test.go") {
            continue;
        }

        // Quick check: does this file contain "package main"?
        if let Ok(content) = std::fs::read_to_string(&path) {
            if content.contains("package main") {
                // Compute the package path relative to the module root.
                let relative = path.strip_prefix(base).unwrap_or(&path);
                let pkg_path = relative.parent().unwrap_or(relative);
                let pkg_str = pkg_path.to_string_lossy().replace('\\', "/");

                let full_path = if pkg_str.is_empty() || pkg_str == "." {
                    module_path.to_string()
                } else {
                    format!("{module_path}/{pkg_str}")
                };

                if !results.contains(&full_path) {
                    results.push(full_path);
                }
            }
        }
    }
}
