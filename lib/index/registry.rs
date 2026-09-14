// SPDX-License-Identifier: GPL-3.0-or-later
//! Index registry: the list of enabled package-index read backends,
//! persisted to `~/.config/lx/indexes.yaml`.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::plugins::package_index::{get_index_backend, PackageIndex};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Registry {
    #[serde(default)]
    pub sources: Vec<SourceEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceEntry {
    pub name: String,
    pub kind: SourceKind,
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    /// The LX community index (ranjithrajv/lx-index) — recipes + prebuilts.
    LxCommunity,
    /// Arch User Repository.
    Aur,
    /// Repology — cross-distro package metadata (read-only).
    Repology,
    /// A user-added custom index (git URL + format).
    Custom { url: String },
}

impl SourceKind {
    /// Canonical backend id for this source, as registered in
    /// [`crate::plugins::package_index`]. A `custom` source is read by the
    /// `custom` git-index backend.
    pub fn plugin_kind(&self) -> Option<&'static str> {
        match self {
            SourceKind::LxCommunity => Some("lx-community"),
            SourceKind::Aur => Some("aur"),
            SourceKind::Repology => Some("repology"),
            SourceKind::Custom { .. } => Some("custom"),
        }
    }
}

impl Registry {
    pub fn config_path() -> Result<PathBuf> {
        // Honor $XDG_CONFIG_HOME (falls back to ~/.config) so tests can
        // redirect the registry to a temp dir.
        let dir = std::env::var("XDG_CONFIG_HOME")
            .ok()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .or_else(dirs::config_dir)
            .context("could not determine config dir")?;
        Ok(dir.join("lx").join("indexes.yaml"))
    }

    pub fn load() -> Result<Self> {
        let path = Self::config_path()?;
        if !path.is_file() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(&path)?;
        serde_yaml::from_str(&text).context("parsing indexes.yaml")
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::config_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let yaml = serde_yaml::to_string(self)?;
        std::fs::write(&path, yaml)?;
        Ok(())
    }

    /// The default registry ships the LX community index, AUR, and the
    /// repology metadata source — all enabled.
    pub fn with_defaults() -> Self {
        Self {
            sources: vec![
                SourceEntry {
                    name: "lx-community".into(),
                    kind: SourceKind::LxCommunity,
                    enabled: true,
                },
                SourceEntry {
                    name: "aur".into(),
                    kind: SourceKind::Aur,
                    enabled: true,
                },
                SourceEntry {
                    name: "repology".into(),
                    kind: SourceKind::Repology,
                    enabled: true,
                },
            ],
        }
    }

    /// Ensure a registry exists on disk, seeding defaults if absent.
    pub fn ensure_exists() -> Result<Self> {
        let path = Self::config_path()?;
        if path.exists() {
            return Self::load();
        }
        let reg = Self::with_defaults();
        reg.save()?;
        Ok(reg)
    }

    pub fn add(&mut self, name: String, kind: SourceKind) -> Result<()> {
        if self.sources.iter().any(|s| s.name == name) {
            anyhow::bail!("a source named '{name}' already exists (remove it first)");
        }
        self.sources.push(SourceEntry {
            name,
            kind,
            enabled: true,
        });
        self.save()
    }

    pub fn remove(&mut self, name: &str) -> Result<()> {
        let before = self.sources.len();
        self.sources.retain(|s| s.name != name);
        if self.sources.len() == before {
            anyhow::bail!("no source named '{name}'");
        }
        self.save()
    }
}

/// Build the concrete source instances for every enabled entry by looking
/// each one up in the [`crate::plugins::package_index`] registry. A `custom`
/// entry passes its configured git URL to the backend.
pub fn active_sources(reg: &Registry) -> Vec<Box<dyn PackageIndex>> {
    reg.sources
        .iter()
        .filter(|s| s.enabled)
        .filter_map(|s| {
            let backend = get_index_backend(s.kind.plugin_kind()?)?;
            let url = match &s.kind {
                SourceKind::Custom { url } => Some(url.as_str()),
                _ => None,
            };
            Some(backend.make_with(&s.name, url))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_kind_maps_to_the_custom_backend() {
        let kind = SourceKind::Custom {
            url: "https://example.com/index.git".into(),
        };
        assert_eq!(kind.plugin_kind(), Some("custom"));

        let backend = get_index_backend("custom").expect("custom backend is registered");
        assert!(backend.capabilities.can_read());
        assert!(!backend.capabilities.can_write());

        let src = backend.make_with("my-index", Some("https://example.com/index.git"));
        assert_eq!(src.name(), "custom");
        assert_eq!(src.instance_name(), "my-index");
    }
}
