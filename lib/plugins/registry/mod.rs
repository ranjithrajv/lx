// SPDX-License-Identifier: GPL-3.0-or-later

//! Input source plugins for language package managers.
//!
//! Each language ecosystem (npm, pip, gem, …) implements [`RegistrySource`].
//! The build pipeline is input-agnostic: an input plugin produces a local
//! directory of files (the "payload"), then the normal packaging pipeline
//! (format plugins) wraps it into a .deb/.rpm/.arch.
//!
//! Adding a new ecosystem is implementing `RegistrySource` and registering it
//! in [`all_registry_sources`]. The `source:` field in package.yaml selects
//! which plugin to use (`source: npm`, `source: python`, `source: gem`).
//!
//! These are intentionally separate from [`crate::plugins::forge::ForgeSource`]
//! (forge release providers). Forge sources fetch prebuilt release assets;
//! input sources fetch from language package registries. Both produce a
//! local payload directory, but their resolution mechanisms differ.

pub mod gem;
pub mod npm;
pub mod python;

use anyhow::Result;
use std::path::PathBuf;

use crate::config::PackageConfig;

/// Output of an input source plugin: a local directory ready for packaging.
pub struct InputPayload {
    /// Directory containing the files to package. The packaging pipeline
    /// treats this like an extracted release archive (binary_dir).
    pub files_dir: PathBuf,
    /// Resolved package version (may differ from requested if "latest").
    pub resolved_version: String,
    /// Detected or configured description.
    pub description: String,
}

/// An input source plugin: fetches a package from a language registry
/// and produces a local directory of files ready for packaging.
///
/// Implementors are stateless; registered once in [`all_registry_sources`].
pub trait RegistrySource: Send + Sync {
    /// Canonical name used in `package.yaml` (`source:`).
    fn name(&self) -> &'static str;

    /// Human-readable description.
    fn description(&self) -> &'static str;

    /// Host tools this input source requires on `PATH` (checked before
    /// fetch). E.g. `vec!["npm"]` for the npm source.
    fn required_tools(&self) -> Vec<&'static str> {
        Vec::new()
    }

    /// Fetch the package and produce a local payload directory.
    ///
    /// `package` is the registry package name (npm package name, pip
    /// requirement, gem name). `version` is the requested version
    /// constraint, or empty for "latest".
    fn fetch(&self, package: &str, version: &str, cfg: &PackageConfig) -> Result<InputPayload>;
}

/// All known input source plugins, in registration order.
pub fn all_registry_sources() -> Vec<Box<dyn RegistrySource>> {
    vec![
        Box::new(npm::NpmRegistrySource),
        Box::new(python::PythonRegistrySource),
        Box::new(gem::GemRegistrySource),
    ]
}

/// Look up an input source by name (case-insensitive). Returns `None` for unknown.
pub fn get_registry_source(name: &str) -> Option<Box<dyn RegistrySource>> {
    let lower = name.to_ascii_lowercase();
    all_registry_sources().into_iter().find(|p| p.name() == lower)
}

/// Available input source names for error messages / help text.
pub fn registry_source_names() -> Vec<&'static str> {
    all_registry_sources().iter().map(|p| p.name()).collect()
}
