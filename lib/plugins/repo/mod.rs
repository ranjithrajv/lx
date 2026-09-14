// SPDX-License-Identifier: GPL-3.0-or-later

//! Repository-indexer plugins: turn a directory of built artifacts into a
//! servable package index for one target format.
//!
//! `lx repo` was apt-only; this is the mirror of the `Packager` dimension on
//! the *distribution* side. Each format's repository layout is different
//! (`Packages`/`Release` for apt/opkg, `<repo>.db.tar.gz` for pacman,
//! `APKINDEX.tar.gz` for apk, `repodata/` for rpm), so each is a
//! [`RepoIndexer`] selected by `lx repo --format`.
//!
//! Selection: [`get_repo_indexer(format)`] — `format` is a `package_format`
//! value (`deb`, `rpm`, `arch`, `apk`, `ipk`).

pub mod apk;
pub mod apt;
pub mod opkg;
pub mod pacman;
pub mod rpm;

use anyhow::Result;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Repository-level metadata passed from `lx repo`.
pub struct IndexOptions<'a> {
    /// Suite/repository name (`stable`, `openwrt`, `core`, …).
    pub suite: &'a str,
    /// Origin/label recorded in the index (when the format has one).
    pub origin: &'a str,
    /// Components (apt `--components`).
    pub components: &'a str,
    /// Optional signing key for the index.
    pub sign_key: Option<&'a Path>,
    pub sign_key_id: &'a str,
}

/// A package-index writer for one target format.
pub trait RepoIndexer: Send + Sync {
    /// Canonical name (`apt`, `opkg`, `pacman`, `apk`, `rpm`).
    fn name(&self) -> &'static str;

    /// The `package_format` this indexer serves.
    fn format(&self) -> &'static str;

    fn description(&self) -> &'static str;

    /// Artifact file extension it consumes (without dot).
    fn file_extension(&self) -> &'static str;

    /// Write the repository index for `artifacts` into `dir`.
    fn build_index(&self, dir: &Path, artifacts: &[PathBuf], opts: &IndexOptions) -> Result<()>;
}

/// All known indexers, in registration order.
pub fn all_repo_indexers() -> Vec<Box<dyn RepoIndexer>> {
    vec![
        Box::new(apt::AptIndexer),
        Box::new(opkg::OpkgIndexer),
        Box::new(pacman::PacmanIndexer),
        Box::new(apk::ApkIndexer),
        Box::new(rpm::RpmIndexer),
    ]
}

/// Look up an indexer by `package_format` (case-insensitive).
pub fn get_repo_indexer(format: &str) -> Option<Box<dyn RepoIndexer>> {
    let lower = format.trim().to_ascii_lowercase();
    all_repo_indexers()
        .into_iter()
        .find(|i| i.format() == lower)
}

pub fn repo_indexer_names() -> Vec<&'static str> {
    all_repo_indexers().iter().map(|i| i.name()).collect()
}

/// Artifacts in `dir` whose extension matches `ext`, sorted.
pub fn artifacts_with_ext(dir: &Path, ext: &str) -> Result<Vec<PathBuf>> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.extension().map(|e| e == ext).unwrap_or(false))
        .collect();
    out.sort();
    Ok(out)
}

/// Parse `key = value` lines (used for `.PKGINFO` and apk control metadata).
pub fn parse_key_value(text: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            map.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    map
}

/// SHA-256 hex of a file.
pub fn sha256_file(path: &Path) -> Result<String> {
    lx_lib::checksum::sha256_file(path)
}
