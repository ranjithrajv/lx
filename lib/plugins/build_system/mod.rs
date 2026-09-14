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

pub mod autotools;
pub mod cargo;
pub mod cmake;
pub mod custom;
pub mod go;
pub mod make;
pub mod meson;

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::PackageConfig;
use crate::plugins::plugin::{Plugin, PluginSet};

/// A build-system plugin: compiles a source tree and stages the install tree.
///
/// Implementors are stateless; registered once in [`all_build_systems`].
pub trait BuildSystem: Plugin {
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
        Box::new(autotools::AutotoolsBuildSystem),
        // `make` is deliberately last among auto-detecting systems: it
        // matches any tree with a Makefile, so the more specific systems
        // get first claim.
        Box::new(make::MakeBuildSystem),
        Box::new(custom::CustomBuildSystem),
    ]
}

/// Look up a build system by name (case-insensitive). Returns `None` for unknown.
pub fn get_build_system(name: &str) -> Option<Box<dyn BuildSystem>> {
    PluginSet::new(all_build_systems()).take(name)
}

/// Available build-system names for error messages / help text.
pub fn build_system_names() -> Vec<&'static str> {
    PluginSet::new(all_build_systems()).names()
}

/// Auto-detect the build system for a source tree. The first plugin whose
/// [`BuildSystem::recognize`] returns true wins. `custom` never auto-detects.
pub fn detect_build_system(src_dir: &Path) -> Option<Box<dyn BuildSystem>> {
    PluginSet::new(all_build_systems()).take_first(|b| b.recognize(src_dir))
}

// ---------------------------------------------------------------------------
// Shared helpers for build-system plugins
// ---------------------------------------------------------------------------

/// Run a configured build command, streaming its output, and fail with
/// `label` on a non-zero exit. Build systems use `status()` (rather than
/// capturing output) so the tool's progress is visible to the user.
pub(crate) fn run(mut cmd: Command, label: &str) -> Result<()> {
    let status = cmd
        .status()
        .with_context(|| format!("failed to run {label}"))?;
    if !status.success() {
        match status.code() {
            Some(code) => bail!("{label} failed (exit code {code})"),
            None => bail!("{label} failed (terminated by signal)"),
        }
    }
    Ok(())
}

/// Apply the musl static-linking compiler environment for `musl: true`.
pub(crate) fn apply_musl_env(cmd: &mut Command, cfg: &PackageConfig) {
    if cfg.musl {
        cmd.env("CC", "musl-gcc")
            .env("CXX", "musl-g++")
            .env("LDFLAGS", "-static");
    }
}

/// Available parallelism for `-jN` builds, defaulting to 1.
pub(crate) fn available_parallelism() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_accepts_a_zero_exit() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "exit 0"]);
        run(cmd, "test command").unwrap();
    }

    #[test]
    fn run_reports_a_nonzero_exit() {
        // Regression: build/install steps must fail the build, not be ignored.
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "exit 3"]);
        let err = run(cmd, "test command").unwrap_err().to_string();
        assert!(err.contains("test command failed"), "{err}");
        assert!(err.contains('3'), "{err}");
    }
}
