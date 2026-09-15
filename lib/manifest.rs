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
    packages: BTreeMap<String, Vec<PackageEntry>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageEntry {
    /// Debian Version field (matches `dpkg-query -W -f='${Version}'`), or
    /// the `[epoch:]version-release` string for rpm/arch.
    pub version: String,
    pub arch: String,
    pub distribution: String,
    pub asset: String,
    pub tag: String,
    pub installed_at: String,
    /// Native package format this generation came from (`deb`/`rpm`/`arch`).
    /// Manifests written before this field existed default to `deb`, which
    /// is exactly what those entries were.
    #[serde(default = "default_package_format")]
    pub format: String,
}

fn default_package_format() -> String {
    "deb".to_string()
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

    /// True when no package is tracked.
    pub fn is_empty(&self) -> bool {
        self.packages.is_empty()
    }

    /// True when `package` has any recorded generation.
    pub fn contains(&self, package: &str) -> bool {
        self.packages.contains_key(package)
    }

    /// Every tracked package with its full generation history, in name order.
    pub fn history(&self) -> impl Iterator<Item = (&str, &[PackageEntry])> {
        self.packages
            .iter()
            .map(|(name, gens)| (name.as_str(), gens.as_slice()))
    }

    /// Every tracked package name, in order (for "upgrade everything").
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.packages.keys().map(String::as_str)
    }

    /// The full generation history for `package` (oldest first); `None` when
    /// the package is unknown.
    pub fn generations(&self, package: &str) -> Option<&[PackageEntry]> {
        self.packages.get(package).map(Vec::as_slice)
    }

    /// Appends a new generation for `package` (does not overwrite history).
    ///
    /// A generation indistinguishable from the current one (same
    /// version/format/arch) is refreshed in place instead of appended:
    /// repeating a same-version install — a double-record, or `lx install
    /// --reinstall` — must not grow the history with entries `rollback`
    /// cannot tell apart.
    pub fn record(&mut self, package: &str, entry: PackageEntry) {
        let gens = self.packages.entry(package.to_string()).or_default();
        if let Some(last) = gens.last_mut() {
            if last.version == entry.version
                && last.format == entry.format
                && last.arch == entry.arch
            {
                *last = entry;
                return;
            }
        }
        gens.push(entry);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(version: &str) -> PackageEntry {
        PackageEntry {
            version: version.into(),
            arch: "x86_64".into(),
            distribution: String::new(),
            asset: format!("herdr-{version}-x86_64.pkg.tar.zst"),
            tag: String::new(),
            installed_at: "t".into(),
            format: "arch".into(),
        }
    }

    #[test]
    fn repeated_same_generation_collapses_to_one() {
        let mut m = Manifest::default();
        m.record("herdr", entry("0.9.0-1.arch"));
        m.record("herdr", entry("0.9.0-1.arch"));
        assert_eq!(m.generations("herdr").unwrap().len(), 1);
    }

    #[test]
    fn distinct_versions_keep_their_history() {
        let mut m = Manifest::default();
        m.record("herdr", entry("0.9.0-1.arch"));
        m.record("herdr", entry("0.10.0-1.arch"));
        assert_eq!(m.generations("herdr").unwrap().len(), 2);
        assert_eq!(m.current("herdr").unwrap().version, "0.10.0-1.arch");
        assert_eq!(m.previous("herdr", 1).unwrap().version, "0.9.0-1.arch");
    }

    #[test]
    fn a_format_change_is_a_new_generation() {
        let mut m = Manifest::default();
        m.record("herdr", entry("0.9.0-1.arch"));
        let mut deb = entry("0.9.0-1.arch");
        deb.format = "deb".into();
        m.record("herdr", deb);
        assert_eq!(m.generations("herdr").unwrap().len(), 2);
    }
}
