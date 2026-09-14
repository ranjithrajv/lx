// SPDX-License-Identifier: GPL-3.0-or-later

//! Public embedding API.
//!
//! `lx` is a library first and a CLI second: everything the binary does lives
//! in `lx_lib`. This module is the **stable, documented surface** for other
//! programs — primarily for packaging a payload you already have, without
//! going through `package.yaml` on disk or a forge release.
//!
//! ```no_run
//! use lx_lib::api::{package, PackageRequest};
//! use lx_lib::config::PackageConfig;
//! use std::path::Path;
//!
//! let cfg = PackageConfig {
//!     package_name: "mytool".into(),
//!     github_repo: "owner/mytool".into(),
//!     version: "1.0.0".into(),
//!     ..Default::default()
//! };
//! let out = package(PackageRequest {
//!     config: &cfg,
//!     binary_dir: Path::new("./payload"),
//!     staging_root: Path::new("/tmp/lx-stage"),
//!     output: Path::new("./dist"),
//!     dist: "trixie".into(),
//!     arch: "amd64".into(),
//!     tag: "v1.0.0".into(),
//!     published_at: None,
//!     debian_version: "1.0.0".into(),
//!     build_version: "1".into(),
//!     mtime: lx_lib::pkgmeta::reproducible_epoch(None),
//!     format: None,
//!     license: None,
//!     detected_deps: Vec::new(),
//! })
//! .expect("package built");
//! println!("built {}", out.display());
//! ```
//!
//! The lower-level modules remain public too, so an embedder can drive the
//! packagers, forge clients, repository writers, and consumer commands
//! directly. [`package`] is the opinionated convenience entry point.

use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};

pub use crate::build::ResolvedJob;
pub use crate::config::{ContentEntry, ContentFileInfo, PackageConfig};
pub use crate::filemeta::{FileMeta, FileMetaMap};
pub use lx_lib::github::RepoLicense;

/// One in-process packaging request. No network access is performed: the
/// payload must already be staged in `binary_dir`.
pub struct PackageRequest<'a> {
    /// Package metadata (name, version, relations, scripts, contents, …).
    pub config: &'a PackageConfig,
    /// Directory holding the payload to install (flat mode: ELF files →
    /// `/usr/bin`; or as configured by `bundle`/`prefix`).
    pub binary_dir: &'a Path,
    /// Empty directory the packager stages into. Created by the caller; the
    /// packager populates and archives it.
    pub staging_root: &'a Path,
    /// Directory the finished artifact is copied into.
    pub output: &'a Path,
    /// Distribution token recorded in the package (e.g. `trixie`, `fedora`).
    pub dist: String,
    /// Architecture token (e.g. `amd64`).
    pub arch: String,
    /// Upstream tag/version the payload came from.
    pub tag: String,
    /// Upstream release publish time (Unix seconds); used for reproducible
    /// metadata. `None` falls back to `SOURCE_DATE_EPOCH`/undefined.
    pub published_at: Option<i64>,
    /// Normalized Debian-style version (no `v` prefix, no epoch).
    pub debian_version: String,
    /// Build revision (e.g. `"1"`).
    pub build_version: String,
    /// Reproducible mtime stamped on archive entries.
    pub mtime: i64,
    /// Package format (`deb`/`rpm`/`arch`/`apk`/`ipk`/`msix`). `None` uses the
    /// config's `package_format`.
    pub format: Option<String>,
    /// Detected upstream license (for the copyright file).
    pub license: Option<&'a RepoLicense>,
    /// Binary dependencies detected via ELF analysis, merged into `Depends`.
    pub detected_deps: Vec<String>,
}

/// Package a pre-staged payload in-process. Returns the final artifact path.
///
/// Equivalent to one cell of the CLI's build matrix, without the forge
/// download or the CLI surface.
pub fn package(req: PackageRequest<'_>) -> Result<PathBuf> {
    let format = req
        .format
        .clone()
        .unwrap_or_else(|| req.config.effective_package_format());
    let plugin = crate::plugins::get_packager(&format).ok_or_else(|| {
        anyhow!(
            "unsupported package format '{}' (expected one of: {})",
            format,
            crate::plugins::packager_names().join(", ")
        )
    })?;

    if !req.staging_root.exists() {
        std::fs::create_dir_all(req.staging_root)?;
    }

    let job = ResolvedJob {
        dist: req.dist,
        arch: req.arch,
        asset: lx_lib::github::Asset {
            name: req.tag.clone(),
            size: None,
            browser_download_url: String::new(),
            checksums: Default::default(),
        },
        tag: req.tag,
        published_at: req.published_at,
    };

    let ctx = crate::plugins::BuildContext {
        cfg: req.config,
        job: &job,
        binary_dir: req.binary_dir,
        staging_root: req.staging_root,
        license: req.license,
        debian_version: &req.debian_version,
        build_version: &req.build_version,
        mtime: req.mtime,
        sign_key: None,
        sign_key_id: "",
        sign_passphrase: None,
        sign_method: "detach",
        detected_deps: req.detected_deps,
    };

    let built = plugin.build(&ctx)?;
    std::fs::create_dir_all(req.output)?;
    let file_name = built
        .file_name()
        .ok_or_else(|| anyhow!("packager produced a path with no file name"))?;
    let final_path = req.output.join(file_name);
    std::fs::copy(&built, &final_path).with_context(|| {
        format!(
            "copying built package {} to {}",
            built.display(),
            final_path.display()
        )
    })?;
    Ok(final_path)
}

/// Parse a `package.yaml` from a string (same pipeline as the CLI: env
/// expansion, legacy-key folding, validation).
pub fn parse_config(yaml: &str) -> Result<PackageConfig> {
    PackageConfig::parse_str(yaml)
}

/// Convert an `nfpm.yaml` document to `package.yaml` text.
pub fn convert_nfpm(yaml: &str) -> Result<String> {
    crate::nfpm::convert_str(yaml)
}

/// Read and convert an `nfpm.yaml` file to `package.yaml` text.
pub fn convert_nfpm_file(path: &Path) -> Result<String> {
    crate::nfpm::convert_file(path)
}
