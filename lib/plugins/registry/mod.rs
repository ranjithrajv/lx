// SPDX-License-Identifier: GPL-3.0-or-later

//! Registry source plugins for language package managers.
//!
//! Each language ecosystem (npm, pip, gem, cargo, nuget, maven, composer, …)
//! implements [`RegistrySource`]. The build pipeline is registry-agnostic: a
//! plugin produces a local directory of files (the "payload"), then the normal
//! packaging pipeline wraps it into a .deb/.rpm/.arch.
//!
//! Adding a new ecosystem is implementing `RegistrySource` and registering it
//! in [`all_registry_sources`]. The `registry_source:` field in package.yaml
//! selects which plugin to use.
//!
//! These are intentionally separate from [`crate::plugins::forge::ForgeSource`]
//! (forge release providers). Forge sources discover *what releases and assets
//! exist*; registry sources fetch *specific files* from language package
//! registries. Both produce a local payload, but their resolution mechanisms
//! differ. See `docs/architecture/plugins.md` for the full comparison.

pub mod cargo;
pub mod composer;
pub mod cpan;
pub mod dart;
pub mod gem;
pub mod go;
pub mod hex;
pub mod maven;
pub mod npm;
pub mod nuget;
pub mod python;

use anyhow::Result;
use std::path::PathBuf;

use crate::config::PackageConfig;

/// Output of a registry source plugin: a local directory ready for packaging.
pub struct RegistryPayload {
    /// Directory containing the files to package. The packaging pipeline
    /// treats this like an extracted release archive (binary_dir).
    pub files_dir: PathBuf,
    /// Resolved package version (may differ from requested if "latest").
    pub resolved_version: String,
    /// Detected or configured description.
    pub description: String,
}

/// A registry source plugin: fetches a package from a language registry
/// and produces a local directory of files ready for packaging.
///
/// Implementors are stateless; registered once in [`all_registry_sources`].
pub trait RegistrySource: Send + Sync {
    /// Canonical name used in `package.yaml` (`registry_source:`).
    fn name(&self) -> &'static str;

    /// Human-readable description.
    fn description(&self) -> &'static str;

    /// Host tools this registry source requires on `PATH` (checked before
    /// fetch). E.g. `vec!["npm"]` for the npm source.
    fn required_tools(&self) -> Vec<&'static str> {
        Vec::new()
    }

    /// Fetch the package and produce a local payload directory.
    ///
    /// `package` is the registry package name (npm package name, pip
    /// requirement, Maven coordinate, composer package, Go module path).
    /// `version` is the requested version constraint, or empty for "latest".
    fn fetch(&self, package: &str, version: &str, cfg: &PackageConfig) -> Result<RegistryPayload>;
}

/// All known registry source plugins, in registration order.
pub fn all_registry_sources() -> Vec<Box<dyn RegistrySource>> {
    vec![
        Box::new(cargo::CargoRegistrySource),
        Box::new(composer::ComposerRegistrySource),
        Box::new(cpan::CpanRegistrySource),
        Box::new(dart::DartRegistrySource),
        Box::new(gem::GemRegistrySource),
        Box::new(go::GoRegistrySource),
        Box::new(hex::HexRegistrySource),
        Box::new(maven::MavenRegistrySource),
        Box::new(npm::NpmRegistrySource),
        Box::new(nuget::NugetRegistrySource),
        Box::new(python::PythonRegistrySource),
    ]
}

/// Look up a registry source by name (case-insensitive). Returns `None` for unknown.
pub fn get_registry_source(name: &str) -> Option<Box<dyn RegistrySource>> {
    let lower = name.to_ascii_lowercase();
    all_registry_sources()
        .into_iter()
        .find(|p| p.name() == lower)
}

/// Available registry source names for error messages / help text.
pub fn registry_source_names() -> Vec<&'static str> {
    all_registry_sources().iter().map(|p| p.name()).collect()
}
