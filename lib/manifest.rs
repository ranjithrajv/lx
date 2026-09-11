// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Tracks packages `lx install`/`upgrade` have put on this system, so
/// `upgrade`/`remove`/`list` can operate without re-deriving state from
/// dpkg's database. Lives per-user (not root-owned) under the local data
/// dir, independent of where dpkg itself records package state -- dpkg
/// remains the source of truth for whether a package is actually installed
/// (see `dpkg_installed_version`); this manifest only remembers what
/// *lx* manages and which asset/tag it came from.
///
/// Each package keeps its full generation history (oldest first, current
/// last) rather than a single snapshot, so `lx rollback` can reinstall a
/// prior version -- Nix profile generations, adapted for dpkg.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Manifest {
    #[serde(default)]
    pub packages: BTreeMap<String, Vec<PackageEntry>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageEntry {
    /// Debian Version field (matches `dpkg-query -W -f='${Version}'`).
    pub version: String,
    pub arch: String,
    pub distribution: String,
    pub asset: String,
    pub tag: String,
    pub installed_at: String,
}

impl Manifest {
    pub fn path() -> Result<PathBuf> {
        let dir = dirs::data_local_dir()
            .ok_or_else(|| anyhow::anyhow!("could not determine a local data directory"))?
            .join("lx");
        Ok(dir.join("installed.json"))
    }

    pub fn load() -> Result<Self> {
        let path = Self::path()?;
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Ok(Self::default());
        };
        serde_json::from_str(&text).with_context(|| format!("failed to parse '{}'", path.display()))
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create '{}'", parent.display()))?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, json).with_context(|| format!("failed to write '{}'", path.display()))
    }

    /// Appends a new generation for `package` (does not overwrite history).
    pub fn record(&mut self, package: &str, entry: PackageEntry) {
        self.packages
            .entry(package.to_string())
            .or_default()
            .push(entry);
    }

    pub fn forget(&mut self, package: &str) -> Option<Vec<PackageEntry>> {
        self.packages.remove(package)
    }

    /// The most recent (current) generation.
    pub fn current(&self, package: &str) -> Option<&PackageEntry> {
        self.packages.get(package).and_then(|gens| gens.last())
    }

    /// The generation `steps` back from current (`steps = 1` is the one
    /// immediately before current). `None` if there aren't that many.
    pub fn previous(&self, package: &str, steps: usize) -> Option<&PackageEntry> {
        self.packages
            .get(package)
            .and_then(|gens| gens.iter().rev().nth(steps))
    }
}
