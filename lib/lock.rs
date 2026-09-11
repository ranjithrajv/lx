//! `package.lock` — a committable pin of exactly which release asset each
//! architecture resolved to on a known-good build, so a build refuses to
//! silently ship a different binary than last time (upstream re-tagging or
//! re-uploading an asset in place). Mirrors Nix flake-lock / fixed-output
//! semantics without adopting flakes or a store: `lx build --update-lock`
//! writes it; any build with a `package.lock` next to the config verifies
//! every resolved (tag, asset, checksum) against it and fails on drift
//! unless `--update-lock` is passed again.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LockFile {
    /// Keyed by Debian architecture (one resolved asset per arch, matching
    /// `build::resolve_manual`'s model).
    #[serde(default)]
    pub packages: BTreeMap<String, LockEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockEntry {
    pub tag: String,
    pub asset: String,
    pub url: String,
    pub sha256: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_at: Option<i64>,
}

impl LockFile {
    /// `package.lock` lives beside the config file it locks.
    pub fn path_for(config: &Path) -> PathBuf {
        config.with_file_name("package.lock")
    }

    /// `Ok(None)` when no lock file exists yet -- not an error, since most
    /// builds don't opt into locking.
    pub fn load(path: &Path) -> Result<Option<Self>> {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Ok(None);
        };
        let lock: Self = serde_json::from_str(&text)
            .with_context(|| format!("failed to parse lock file '{}'", path.display()))?;
        Ok(Some(lock))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, format!("{json}\n"))
            .with_context(|| format!("failed to write lock file '{}'", path.display()))
    }

    pub fn entry_for(&self, arch: &str) -> Option<&LockEntry> {
        self.packages.get(arch)
    }
}
