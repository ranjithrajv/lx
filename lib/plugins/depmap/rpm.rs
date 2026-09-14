// SPDX-License-Identifier: GPL-3.0-or-later

use super::DependencyMapper;

/// RPM (Fedora/RHEL/SUSE) dependency syntax.
pub struct RpmDeps;

impl DependencyMapper for RpmDeps {
    fn name(&self) -> &'static str {
        "rpm"
    }

    fn format(&self) -> &'static str {
        "rpm"
    }

    fn description(&self) -> &'static str {
        "RPM package names and `name >= version` syntax"
    }

    fn repo_family(&self) -> Option<&'static str> {
        Some("fedora")
    }

    fn map(&self, ecosystem: &str, dep_name: &str) -> Option<String> {
        crate::depmap::hardcoded_or_repology(ecosystem, dep_name, "rpm")
    }

    fn constraint(&self, raw: &str) -> Option<String> {
        crate::depmap::normalize_version(raw, "rpm")
    }
}
