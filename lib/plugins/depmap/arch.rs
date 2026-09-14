// SPDX-License-Identifier: GPL-3.0-or-later

use super::DependencyMapper;

/// Arch Linux (pacman) dependency syntax.
pub struct ArchDeps;

impl DependencyMapper for ArchDeps {
    fn name(&self) -> &'static str {
        "pacman"
    }

    fn format(&self) -> &'static str {
        "arch"
    }

    fn description(&self) -> &'static str {
        "Arch package names and `name>=version` syntax"
    }

    fn repo_family(&self) -> Option<&'static str> {
        Some("arch")
    }

    fn map(&self, ecosystem: &str, dep_name: &str) -> Option<String> {
        crate::depmap::hardcoded_or_repology(ecosystem, dep_name, "arch")
    }

    fn constraint(&self, raw: &str) -> Option<String> {
        crate::depmap::normalize_version(raw, "arch")
    }
}
