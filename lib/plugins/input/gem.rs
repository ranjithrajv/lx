// SPDX-License-Identifier: GPL-3.0-or-later

//! Ruby gem input source plugin.
//!
//! Fetches a Ruby gem and produces a local directory of files ready for
//! packaging. Uses `gem fetch` to download the .gem file, then extracts
//! it (gems are tar archives with data.tar.gz + metadata.gz).

use anyhow::{bail, Context, Result};
use std::path::PathBuf;
use std::process::Command;

use crate::config::PackageConfig;
use crate::plugins::input::{InputPayload, InputSource};

pub struct GemInputSource;

impl InputSource for GemInputSource {
    fn name(&self) -> &'static str {
        "gem"
    }

    fn description(&self) -> &'static str {
        "Ruby gem — gem fetch + extract"
    }

    fn required_tools(&self) -> Vec<&'static str> {
        vec!["gem"]
    }

    fn fetch(&self, package: &str, version: &str, cfg: &PackageConfig) -> Result<InputPayload> {
        let mut spec = package.to_string();
        if !version.trim().is_empty() {
            spec.push('(');
            spec.push('~');
            spec.push('>');
            spec.push(' ');
            spec.push_str(version.trim());
            spec.push(')');
        }

        println!("gem: fetching {spec}");

        let workdir = tempfile::tempdir().context("failed to create gem workdir")?;
        let download_dir = workdir.path().join("download");
        std::fs::create_dir_all(&download_dir)?;

        let args: Vec<&str> = if version.trim().is_empty() {
            vec!["fetch", package]
        } else {
            vec!["fetch", package, "--version", version.trim()]
        };

        let output = Command::new("gem")
            .args(&args)
            .current_dir(&download_dir)
            .output()
            .context("failed to run `gem fetch` (is gem on PATH?)")?;

        if !output.status.success() {
            bail!(
                "gem fetch failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        // Find the downloaded .gem file.
        let gem_file = std::fs::read_dir(&download_dir)?
            .filter_map(|e| e.ok())
            .find(|e| e.file_name().to_string_lossy().ends_with(".gem"))
            .context("gem fetch produced no .gem file")?;

        let gem_path = gem_file.path();
        let file_name = gem_file.file_name().to_string_lossy().to_string();

        // Extract version from filename: "name-version.gem" or "name-version-platform.gem".
        let version = extract_gem_version(&file_name).unwrap_or("0.0.0");
        println!("gem: extracted {file_name} (version {version})");

        // Extract the .gem (it's a tar archive).
        let extract_dir = workdir.path().join("package");
        std::fs::create_dir_all(&extract_dir)?;
        crate::build::extract(&gem_path, &extract_dir, "tar")
            .context("failed to extract gem archive")?;

        // The gem structure after extraction:
        //   data.tar.gz    — the actual files
        //   metadata.gz    — gem specification
        // We need to extract data.tar.gz into the final directory.
        let data_tgz = extract_dir.join("data.tar.gz");
        if data_tgz.exists() {
            let files_dir = workdir.path().join("files");
            std::fs::create_dir_all(&files_dir)?;
            crate::build::extract(&data_tgz, &files_dir, "tar.gz")
                .context("failed to extract gem data.tar.gz")?;

            let description = read_gem_description(&extract_dir, &files_dir, cfg);

            Ok(InputPayload {
                files_dir,
                resolved_version: version.to_string(),
                description,
            })
        } else {
            // Some gems have a flat structure.
            let description = read_gem_description(&extract_dir, &extract_dir, cfg);
            Ok(InputPayload {
                files_dir: extract_dir,
                resolved_version: version.to_string(),
                description,
            })
        }
    }
}

/// Extract version from gem filename: "foo-1.2.3.gem" or "foo-1.2.3-x86_64-linux.gem".
fn extract_gem_version(file_name: &str) -> Option<&str> {
    let stem = file_name.strip_suffix(".gem")?;
    // Find first "-" followed by a digit (version start).
    for (i, c) in stem.char_indices() {
        if c == '-' && i + 1 < stem.len() && stem[i + 1..].starts_with(|c: char| c.is_ascii_digit())
        {
            // Version may end at the next "-" (platform) or end of string.
            let rest = &stem[i + 1..];
            if let Some(end) = rest.find('-') {
                return Some(&rest[..end]);
            }
            return Some(rest);
        }
    }
    None
}

/// Read gem description from metadata.gz or gemspec.
fn read_gem_description(
    extract_dir: &std::path::Path,
    _files_dir: &std::path::Path,
    cfg: &PackageConfig,
) -> String {
    if !cfg.description.is_empty() {
        return cfg.description.clone();
    }

    // Try to read from metadata.gz (it's a YAML-serialized gem spec).
    let metadata_gz = extract_dir.join("metadata.gz");
    if !metadata_gz.exists() {
        return String::new();
    }

    let file = match std::fs::File::open(&metadata_gz) {
        Ok(f) => f,
        Err(_) => return String::new(),
    };
    let gz = flate2::read::GzDecoder::new(file);
    let mut reader = std::io::BufReader::new(gz);
    let mut yaml_text = String::new();
    if std::io::Read::read_to_string(&mut reader, &mut yaml_text).is_err() {
        return String::new();
    }

    // The gem spec is YAML. Look for "summary:" or "description:".
    for line in yaml_text.lines() {
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim();
            if key == "summary" || key == "description" {
                return value.trim().to_string();
            }
        }
    }

    String::new()
}
