// SPDX-License-Identifier: GPL-3.0-or-later

use super::DependencyMapper;
use crate::plugins::plugin::plugin_identity;

/// RPM (Fedora/RHEL/SUSE) dependency syntax.
pub struct RpmDeps;

plugin_identity!(
    RpmDeps,
    "rpm",
    "RPM package names and `name >= version` syntax"
);

impl DependencyMapper for RpmDeps {
    fn format(&self) -> &'static str {
        "rpm"
    }

    fn map(&self, ecosystem: &str, dep_name: &str) -> Option<String> {
        crate::depmap::hardcoded_or_repology(ecosystem, dep_name, "rpm")
    }

    fn constraint(&self, raw: &str) -> Option<String> {
        crate::depmap::normalize_version(raw, "rpm")
    }
}
