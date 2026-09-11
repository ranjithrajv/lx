// SPDX-License-Identifier: GPL-3.0-or-later

//! Input source plugins for language package managers.
//!
//! Each language ecosystem (npm, pip, gem, …) implements [`InputSource`].
//! The build pipeline is input-agnostic: an input plugin produces a local
//! directory of files (the "payload"), then the normal packaging pipeline
//! (format plugins) wraps it into a .deb/.rpm/.arch.
//!
//! Adding a new ecosystem is implementing `InputSource` and registering it
//! in [`all_input_sources`]. The `source:` field in package.yaml selects
//! which plugin to use (`source: npm`, `source: python`, `source: gem`).
//!
//! These are intentionally separate from [`crate::plugins::source::SourcePlugin`]
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
/// Implementors are stateless; registered once in [`all_input_sources`].
pub trait InputSource: Send + Sync {
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
    fn fetch(
        &self,
        package: &str,
        version: &str,
        cfg: &PackageConfig,
    ) -> Result<InputPayload>;
}

/// All known input source plugins, in registration order.
pub fn all_input_sources() -> Vec<Box<dyn InputSource>> {
    vec![
        Box::new(npm::NpmInputSource),
        Box::new(python::PythonInputSource),
        Box::new(gem::GemInputSource),
    ]
}

/// Look up an input source by name (case-insensitive). Returns `None` for unknown.
pub fn get_input_source(name: &str) -> Option<Box<dyn InputSource>> {
    let lower = name.to_ascii_lowercase();
    all_input_sources()
        .into_iter()
        .find(|p| p.name() == lower)
}

/// Available input source names for error messages / help text.
pub fn input_source_names() -> Vec<&'static str> {
    all_input_sources().iter().map(|p| p.name()).collect()
}
