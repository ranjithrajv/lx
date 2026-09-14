// SPDX-License-Identifier: GPL-3.0-or-later

//! Packager architecture for `lx`.
//!
//! Each package format (`.deb`, `.rpm`, …) is a plugin implementing the
//! [`Packager`] trait. The build pipeline is format-agnostic: it resolves
//! assets, stages the install tree via shared helpers, then delegates the
//! actual archive creation to the selected plugin.
//!
//! Registration is static and explicit — no dynamic loading — so adding a new
//! format is just implementing `Packager` and registering it in
//! `all_packagers()`.
//!
//! This module holds the plugin traits, the registry and the shared build
//! context. The staging engine lives in the `staging` submodule and the
//! `contents:`/scripts overlay in `contents`; both are re-exported here so
//! plugin files keep using `super::stage_install_tree` etc.

pub mod apk;
pub mod arch;
pub mod artifact;
pub mod build_system;
pub mod deb;
pub mod depmap;
pub mod forge;
pub mod ipk;
pub mod msix;
pub mod osxpkg;
pub mod package_index;
pub mod plugin;
pub mod registry;
pub mod rpm;
pub mod signer;

mod contents;
mod metadata;
mod staging;

pub use contents::*;
pub use metadata::BuildMetadata;
pub use staging::*;

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::config::PackageConfig;
use crate::plugins::plugin::{Plugin, PluginSet};
use lx_lib::github::RepoLicense;

/// Context passed to a plugin's build method. Contains everything the plugin
/// needs to render its control/spec metadata and archive the staged tree.
pub struct BuildContext<'a> {
    /// The package metadata this plugin may read. Narrower than the full
    /// `PackageConfig` so a format only depends on what it uses; shared
    /// helpers that need everything get it via `cfg.config()`.
    pub cfg: &'a dyn BuildMetadata,
    /// Resolved job (dist, arch, asset, tag, published_at).
    pub job: &'a crate::build::ResolvedJob,
    /// Directory containing the extracted binary tree to stage (binary_dir).
    pub binary_dir: &'a Path,
    /// Staging root that the plugin should populate (e.g. `root/usr/bin/...`).
    /// Already created; plugin stages files under it then archives it.
    pub staging_root: &'a Path,
    /// Detected upstream license, if any.
    pub license: Option<&'a RepoLicense>,
    /// Debian-style version string (upstream prefix stripped).
    pub debian_version: &'a str,
    /// Build revision (e.g. "1").
    pub build_version: &'a str,
    /// Reproducible mtime (SOURCE_DATE_EPOCH or published_at).
    pub mtime: i64,
    /// Signing key file for formats that embed signatures natively
    /// (rpm), and for deb when `sign_method` is `"debsign"`. `None` when
    /// signing is disabled. deb `detach` signs post-build instead
    /// (detached `.sig`).
    pub sign_key: Option<&'a Path>,
    /// Optional gpg `--local-user` key id / fingerprint (deb signing).
    pub sign_key_id: &'a str,
    /// Passphrase for the embedded-signature key (rpm / debsign),
    /// resolved from env.
    pub sign_passphrase: Option<&'a str>,
    /// Deb signing method: `"detach"` (post-build `.sig`) or `"debsign"`
    /// (embedded `_gpgorigin`). Ignored by rpm/arch.
    pub sign_method: &'a str,
    /// Binary dependencies detected via ELF analysis. Merged into Depends:
    /// by the format plugins (deb only; rpm/arch handle deps differently).
    pub detected_deps: Vec<String>,
}

// ---------------------------------------------------------------------------
// Shared helpers for packager plugins (DRY: deb/rpm/arch all used to duplicate these)
// ---------------------------------------------------------------------------

/// Resolve the homepage URL from config. Delegates to the selected forge
/// provider's [`homepage`](crate::plugins::forge::ForgeSource::homepage), so
/// adding a provider needs no edit here; unknown/`custom` sources fall back
/// to the GitHub-style URL.
pub fn resolve_homepage(cfg: &PackageConfig) -> String {
    crate::plugins::forge::get_forge_source(&cfg.effective_forge_source())
        .map(|source| source.homepage(cfg))
        .unwrap_or_else(|| crate::constants::homepage_for_github(&cfg.github_repo))
}

/// Compute `{build_version}+{dist}` formatted for non-debian release strings
/// (the `+` is replaced with `.` per RPM/Arch conventions).
pub fn format_release(build_version: &str, dist: &str) -> String {
    format!("{build_version}+{dist}").replace('+', ".")
}

/// Create the sibling output directory `__out` next to the staging root.
/// The archive builders tar/zst the whole staging tree, so an in-tree out
/// dir would package the artifact into itself.
pub fn output_dir(staging_root: &Path) -> anyhow::Result<PathBuf> {
    let dir = staging_root
        .parent()
        .context("staging root has no parent")?
        .join("__out");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// A package-format plugin.
///
/// Implementors are stateless; they are registered once in [`registry`].
pub trait Packager: Plugin {
    /// File extension without dot (e.g. `"deb"`, `"rpm"`).
    fn file_extension(&self) -> &'static str;

    /// Default distributions when `debian_distributions` is empty and the
    /// plugin is selected. For `deb` this is Debian suites; for `rpm` this
    /// is RPM-based distros. Consumed by
    /// [`PackageConfig::effective_distributions_for`](crate::config::PackageConfig::effective_distributions_for).
    fn default_distributions(&self) -> &'static [&'static str];

    /// Whether an architecture is supported for a given distribution.
    fn arch_supported_for_dist(&self, arch: &str, dist: &str) -> bool;

    /// Build a single package archive.
    ///
    /// The plugin stages the install tree under `ctx.staging_root` (using
    /// shared helpers like [`stage_install_tree`]) and then creates the
    /// archive at the returned `PathBuf` (typically inside a temp dir).
    fn build(&self, ctx: &BuildContext) -> Result<PathBuf>;

    /// Glob (relative to the output directory) matching the artifacts this
    /// packager wrote for `package`. Used by the build summary so the
    /// filename convention lives with the format, not in a central `match`.
    fn artifact_glob(&self, package: &str) -> String {
        format!("{package}_*.{}", self.file_extension())
    }
}

/// The source-package role, implemented only by the formats that can produce
/// one (`deb`, `rpm`, `arch`). Kept separate from [`Packager`] so a binary-only
/// format (`apk`, `ipk`, `msix`, `osxpkg`) is never handed these methods and
/// cannot be selected for a source build.
pub trait SourcePackager: Packager {
    /// Archive an already-populated `staging_root` into this format.
    ///
    /// The source-build pipeline stages via the build system, so it cannot
    /// call [`build`](Packager::build) (which stages from `binary_dir`); it
    /// calls this instead.
    fn archive_staged_tree(&self, ctx: &BuildContext) -> Result<PathBuf>;

    /// Emit this format's source package from a built binary.
    fn generate_source_package(&self, out_dir: &Path, pkg: &crate::source::Pkg) -> Result<()>;
}

/// All known plugins, in registration order.
pub fn all_packagers() -> Vec<Box<dyn Packager>> {
    vec![
        Box::new(deb::DebPackager),
        Box::new(rpm::RpmPackager),
        Box::new(arch::ArchPackager),
        Box::new(apk::ApkPackager),
        Box::new(ipk::IpkPackager),
        Box::new(msix::MsixPackager),
        Box::new(osxpkg::OsxPkgPackager),
    ]
}

/// All packagers that can produce a source package, in registration order.
/// The `--source` pipeline selects from this set rather than testing a
/// `supports_source_build()` flag.
pub fn all_source_packagers() -> Vec<Box<dyn SourcePackager>> {
    vec![
        Box::new(deb::DebPackager),
        Box::new(rpm::RpmPackager),
        Box::new(arch::ArchPackager),
    ]
}

/// Look up a plugin by name (case-insensitive, aliases resolved: `pkg` →
/// `osxpkg`). Returns `None` for unknown.
pub fn get_packager(name: &str) -> Option<Box<dyn Packager>> {
    PluginSet::new(all_packagers()).take(&crate::config::canonical_format(name))
}

/// Look up a source-capable plugin by name (case-insensitive, aliases
/// resolved). Returns `None` for formats without a source-package role.
pub fn get_source_packager(name: &str) -> Option<Box<dyn SourcePackager>> {
    PluginSet::new(all_source_packagers()).take(&crate::config::canonical_format(name))
}

/// Available plugin names for error messages / help text.
pub fn packager_names() -> Vec<&'static str> {
    PluginSet::new(all_packagers()).names()
}

/// Expand a user-supplied `--format` value into the concrete formats to
/// build. `all` expands to every registered packager (deb, rpm, arch, apk,
/// ipk); a comma-separated list is split and trimmed; a single format
/// returns `None` so the caller can pass it through unchanged (preserving
/// the existing validation and error messages).
pub fn expand_formats(value: &str) -> Option<Vec<String>> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("all") {
        return Some(packager_names().into_iter().map(str::to_string).collect());
    }
    if v.contains(',') {
        let list: Vec<String> = v
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if !list.is_empty() {
            return Some(list);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Shared control-metadata helpers (deb control rendering)
// ---------------------------------------------------------------------------

/// The extended-description continuation line shared by every rendered
/// control file (binary `DEBIAN/control`, source `debian/control`). One
/// constant so the wording can't drift between renderers again — it had
/// already split into "upstream GitHub release" vs "the upstream release"
/// variants before this was centralized.
pub(crate) const PACKAGED_FROM_LINE: &str = " Packaged from the upstream release for Debian.";

/// Render sorted extra control fields (skipping core fields already
/// rendered explicitly). Shared by the deb plugin's binary control file
/// and the source package's `debian/control`.
pub(crate) fn render_extra_fields(fields: &std::collections::HashMap<String, String>) -> String {
    if fields.is_empty() {
        return String::new();
    }
    let mut keys: Vec<_> = fields.keys().collect();
    keys.sort();
    let mut out = String::new();
    for k in keys {
        let v = &fields[k];
        if k.trim().is_empty() || v.trim().is_empty() {
            continue;
        }
        // Avoid duplicating core fields if user accidentally sets them via fields.
        let lower = k.to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "package"
                | "version"
                | "architecture"
                | "maintainer"
                | "description"
                | "section"
                | "priority"
                | "homepage"
                | "depends"
                | "recommends"
                | "suggests"
                | "conflicts"
                | "replaces"
                | "provides"
                | "breaks"
                | "pre-depends"
                | "pre_depends"
                | "predepends"
                | "source"
                | "standards-version"
                | "build-depends"
        ) {
            continue;
        }
        out.push_str(&format!("{}: {}\n", k.trim(), v.trim()));
    }
    out
}
