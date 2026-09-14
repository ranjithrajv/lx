// SPDX-License-Identifier: GPL-3.0-or-later

//! Artifact-format plugins: how an upstream release asset is unpacked into
//! a payload directory before packaging.
//!
//! This is the fifth plugin dimension. It answers a question distinct from
//! the other four: not *where* a payload comes from (`ForgeSource` /
//! `RegistrySource`), not *how it is compiled* (`BuildSystem`), and not
//! *what it becomes* (`Packager`) — but **how an archive on disk is turned
//! into files**.
//!
//! Selection is by `artifact_format:` in package.yaml (or auto-detected
//! from the asset filename via [`detect_artifact_format`]). Adding a format
//! is implementing [`ArtifactFormat`] and registering it in
//! [`all_artifact_formats`].

pub mod plain_tar;
pub mod raw;
pub mod tar_gz;
pub mod tar_xz;
pub mod tar_zst;
pub mod zip;

use anyhow::Result;
use std::path::Path;

use crate::plugins::plugin::{Plugin, PluginSet};

/// A plugin that extracts one archive format into a directory.
///
/// Implementors are stateless; registered once in [`all_artifact_formats`].
pub trait ArtifactFormat: Plugin {
    /// Alternative names accepted for `artifact_format:` (e.g. `tgz`).
    fn aliases(&self) -> &'static [&'static str] {
        &[]
    }

    /// True when `file_name` (lowercased) looks like this format. Used for
    /// zero-config auto-detection, so order in the registry matters: put
    /// more specific formats before catch-all formats like `raw`.
    fn recognizes(&self, file_name: &str) -> bool;

    /// Extract `archive` into `dest` (which already exists).
    fn extract(&self, archive: &Path, dest: &Path) -> Result<()>;
}

/// All known artifact formats, in detection order (specific → catch-all).
pub fn all_artifact_formats() -> Vec<Box<dyn ArtifactFormat>> {
    vec![
        Box::new(tar_gz::TarGz),
        Box::new(tar_xz::TarXz),
        Box::new(tar_zst::TarZst),
        Box::new(plain_tar::PlainTar),
        Box::new(zip::Zip),
        // `raw` matches anything, so it must be last.
        Box::new(raw::Raw),
    ]
}

/// Look up a format by canonical name or alias (case-insensitive).
pub fn get_artifact_format(name: &str) -> Option<Box<dyn ArtifactFormat>> {
    let lower = name.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return None;
    }
    PluginSet::new(all_artifact_formats())
        .take_first(|f| f.name() == lower || f.aliases().iter().any(|a| *a == lower))
}

/// Auto-detect the format from a file name. Falls back to `raw` for
/// anything unrecognized (matching the previous `guess_format` behavior).
pub fn detect_artifact_format(file_name: &str) -> &'static str {
    let lower = file_name.to_ascii_lowercase();
    PluginSet::new(all_artifact_formats())
        .first(|f| f.recognizes(&lower))
        .map(|f| f.name())
        .unwrap_or("raw")
}

/// Available format names (canonical only) for error messages / help.
pub fn artifact_format_names() -> Vec<&'static str> {
    PluginSet::new(all_artifact_formats()).names()
}

/// Extract `archive` into `dest` using the named format.
pub fn extract(archive: &Path, dest: &Path, format: &str) -> Result<()> {
    let plugin = get_artifact_format(format).ok_or_else(|| {
        anyhow::anyhow!(
            "unsupported artifact_format '{format}' (expected one of: {})",
            artifact_format_names().join(", ")
        )
    })?;
    std::fs::create_dir_all(dest)?;
    plugin.extract(archive, dest)
}
