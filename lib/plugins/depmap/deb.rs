// SPDX-License-Identifier: GPL-3.0-or-later

use super::DependencyMapper;
use crate::plugins::plugin::plugin_identity;

/// Debian/Ubuntu dependency syntax. The shared ecosystem→Debian tables in
/// `depmap.rs` are this mapper's native form, so it is the reference impl.
pub struct DebianDeps;

plugin_identity!(
    DebianDeps,
    "debian",
    "Debian/Ubuntu package names and `name (>= version)` syntax"
);

impl DependencyMapper for DebianDeps {
    fn format(&self) -> &'static str {
        "deb"
    }

    fn repo_family(&self) -> Option<&'static str> {
        Some("debian")
    }

    fn map(&self, ecosystem: &str, dep_name: &str) -> Option<String> {
        crate::depmap::hardcoded_or_repology(ecosystem, dep_name, "deb")
    }

    fn constraint(&self, raw: &str) -> Option<String> {
        crate::depmap::normalize_version(raw, "deb")
    }
}
