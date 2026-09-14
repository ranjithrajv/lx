// SPDX-License-Identifier: GPL-3.0-or-later

use super::DependencyMapper;
use crate::plugins::plugin::plugin_identity;

/// OpenWrt (opkg) dependency syntax. opkg uses Debian-style
/// `name (>= version)` rendering, so only the package-name translation
/// differs from the deb mapper.
pub struct OpenWrtDeps;

plugin_identity!(
    OpenWrtDeps,
    "openwrt",
    "OpenWrt (opkg) package names, Debian-style `name (>= version)` syntax"
);

impl DependencyMapper for OpenWrtDeps {
    fn format(&self) -> &'static str {
        "ipk"
    }

    fn map(&self, ecosystem: &str, dep_name: &str) -> Option<String> {
        crate::depmap::hardcoded_or_repology(ecosystem, dep_name, "deb").map(|d| deb_to_openwrt(&d))
    }

    fn constraint(&self, raw: &str) -> Option<String> {
        crate::depmap::normalize_version(raw, "deb")
    }
}

/// Best-effort Debian→OpenWrt package-name translation. Unknown names pass
/// through unchanged.
pub fn deb_to_openwrt(deb_name: &str) -> String {
    match deb_name {
        "libc6" => "libc".to_string(),
        "libssl3" => "libopenssl".to_string(),
        "libstdc++6" => "libstdcpp".to_string(),
        "libgcc-s1" => "libgcc1".to_string(),
        "zlib1g" => "zlib".to_string(),
        other => other.to_string(),
    }
}
