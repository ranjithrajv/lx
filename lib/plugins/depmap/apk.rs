// SPDX-License-Identifier: GPL-3.0-or-later

use super::DependencyMapper;
use crate::plugins::plugin::plugin_identity;

/// Alpine Linux (apk) dependency syntax.
///
/// The shared ecosystem tables are Debian-shaped, so this mapper starts
/// from the Debian package name and translates the common libc/runtime
/// differences. Alpine renders constraints *unparenthesized*
/// (`foo>=1.2`), which is why it overrides [`DependencyMapper::render`].
pub struct AlpineDeps;

plugin_identity!(
    AlpineDeps,
    "alpine",
    "Alpine (apk) package names and `name>=version` syntax"
);

impl DependencyMapper for AlpineDeps {
    fn format(&self) -> &'static str {
        "apk"
    }

    fn map(&self, ecosystem: &str, dep_name: &str) -> Option<String> {
        crate::depmap::hardcoded_or_repology(ecosystem, dep_name, "deb").map(|d| deb_to_alpine(&d))
    }

    fn constraint(&self, raw: &str) -> Option<String> {
        crate::depmap::normalize_version(raw, "arch")
    }

    fn render(&self, name: &str, constraint: Option<&str>) -> String {
        match constraint {
            Some(c) => format!("{name}{c}"),
            None => name.to_string(),
        }
    }
}

/// Best-effort Debian→Alpine package-name translation for the shared
/// runtime/library dependencies. Unknown names pass through unchanged.
pub fn deb_to_alpine(deb_name: &str) -> String {
    match deb_name {
        "libc6" => "musl".to_string(),
        "libstdc++6" => "libstdc++".to_string(),
        "libgcc-s1" => "libgcc".to_string(),
        "zlib1g" => "zlib".to_string(),
        "libssl3" => "libssl3".to_string(),
        "libcrypto3" => "libcrypto3".to_string(),
        other => other.to_string(),
    }
}
