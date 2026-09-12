// SPDX-License-Identifier: GPL-3.0-or-later

//! Build-system plugins for `lx` source builds.
//!
//! Each build system (cmake, cargo, go, …) implements [`BuildSystem`]. The
//! source-build pipeline is build-system-agnostic: it fetches the source tag,
//! then delegates staging-the-install-tree to the selected plugin. Adding a
//! new ecosystem is implementing `BuildSystem` and registering it in
//! [`all_build_systems`].
//!
//! [`crate::sourcebuild`] resolves which plugin to use (`build_system:` in
//! package.yaml) and calls [`build`] to produce the install tree.

pub mod cargo;
pub mod cmake;
pub mod custom;
pub mod go;
pub mod meson;

use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::config::PackageConfig;

/// A build-system plugin: compiles a source tree and stages the install tree.
///
/// Implementors are stateless; registered once in [`all_build_systems`].
pub trait BuildSystem: Send + Sync {
    /// Canonical name used in `package.yaml` (`build_system:`).
    fn name(&self) -> &'static str;

    /// Human-readable description.
    fn description(&self) -> &'static str;

    /// Whether this build system recognizes the source tree. Plugins
    /// override this to detect their project files (e.g. `Cargo.toml`,
    /// `go.mod`, `CMakeLists.txt`). `custom` never auto-detects (returns
    /// false). Used by [`detect_build_system`] for zero-config builds.
    fn recognize(&self, _src_dir: &Path) -> bool {
        false
    }

    /// Build the project and stage its install tree under `workdir`.
    ///
    /// Implementors compile `src_dir` (the extracted source) and populate a
    /// `DESTDIR`-style install tree somewhere under `workdir`, then return its
    /// root. [`crate::sourcebuild`] wraps that tree per-suite.
    fn build(&self, cfg: &PackageConfig, src_dir: &Path, workdir: &Path) -> Result<PathBuf>;

    /// Host tools this build system requires on `PATH` (checked by
    /// [`crate::sourcebuild`] before building). Default: none (the caller
    /// is responsible for tooling).
    fn required_tools(&self) -> Vec<&'static str> {
        Vec::new()
    }
}

/// All known build systems, in registration order.
pub fn all_build_systems() -> Vec<Box<dyn BuildSystem>> {
    vec![
        Box::new(cmake::CmakeBuildSystem),
        Box::new(cargo::CargoBuildSystem),
        Box::new(go::GoBuildSystem),
        Box::new(meson::MesonBuildSystem),
        Box::new(custom::CustomBuildSystem),
    ]
}

/// Look up a build system by name (case-insensitive). Returns `None` for unknown.
pub fn get_build_system(name: &str) -> Option<Box<dyn BuildSystem>> {
    let lower = name.to_ascii_lowercase();
    all_build_systems().into_iter().find(|b| b.name() == lower)
}

/// Available build-system names for error messages / help text.
pub fn packager_names() -> Vec<&'static str> {
    all_build_systems().iter().map(|b| b.name()).collect()
}

/// Auto-detect the build system for a source tree. The first plugin whose
/// [`BuildSystem::recognize`] returns true wins. `custom` never auto-detects.
pub fn detect_build_system(src_dir: &Path) -> Option<Box<dyn BuildSystem>> {
    all_build_systems()
        .into_iter()
        .find(|b| b.recognize(src_dir))
}
