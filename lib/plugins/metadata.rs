// SPDX-License-Identifier: GPL-3.0-or-later

//! The narrow metadata view a [`Packager`](super::Packager) receives.
//!
//! Plugins used to get the whole [`PackageConfig`] (~100 fields) through
//! [`BuildContext`](super::BuildContext), so every format depended on fields it
//! never read — an ISP violation. This trait exposes only the metadata the
//! packagers actually use; `PackageConfig` implements it.
//!
//! The shared staging/contents/templating helpers genuinely need the full
//! config, so they take `&PackageConfig` and are reached through
//! [`BuildMetadata::config`] at the call sites that invoke them.

use std::collections::HashMap;

use crate::config::{
    ContentEntry, DebConfig, IpkConfig, MsixConfig, PackageConfig, RpmConfig, Scripts,
    SignatureConfig,
};

/// The metadata surface a package-format plugin depends on.
pub trait BuildMetadata {
    /// The full config, for the shared staging/contents/templating helpers.
    fn config(&self) -> &PackageConfig;

    fn package_name(&self) -> &String {
        &self.config().package_name
    }
    fn epoch(&self) -> &String {
        &self.config().epoch
    }
    fn github_repo(&self) -> &String {
        &self.config().github_repo
    }
    fn license_spdx(&self) -> &String {
        &self.config().license_spdx
    }
    fn prefix(&self) -> &String {
        &self.config().prefix
    }
    fn binary_rename(&self) -> &String {
        &self.config().binary_rename
    }
    fn bundle(&self) -> bool {
        self.config().bundle
    }
    fn disable_globbing(&self) -> bool {
        self.config().disable_globbing
    }
    fn template_scripts(&self) -> bool {
        self.config().template_scripts
    }
    fn fields(&self) -> &HashMap<String, String> {
        &self.config().fields
    }
    fn contents(&self) -> &[ContentEntry] {
        &self.config().contents
    }
    fn scripts(&self) -> &Scripts {
        &self.config().scripts
    }
    fn deb(&self) -> &DebConfig {
        &self.config().deb
    }
    fn rpm(&self) -> &RpmConfig {
        &self.config().rpm
    }
    fn ipk(&self) -> &IpkConfig {
        &self.config().ipk
    }
    fn msix(&self) -> &MsixConfig {
        &self.config().msix
    }
    fn signature(&self) -> &SignatureConfig {
        &self.config().signature
    }

    fn effective_forge_source(&self) -> String {
        self.config().effective_forge_source()
    }
    fn effective_description(&self) -> String {
        self.config().effective_description()
    }
    fn effective_maintainer(&self) -> String {
        self.config().effective_maintainer()
    }
    fn effective_section(&self) -> String {
        self.config().effective_section()
    }
    fn effective_architecture(&self) -> String {
        self.config().effective_architecture()
    }
    fn effective_priority(&self) -> String {
        self.config().effective_priority()
    }
    fn effective_arch_variant(&self) -> String {
        self.config().effective_arch_variant()
    }
    fn effective_umask(&self) -> Option<u32> {
        self.config().effective_umask()
    }
    fn effective_packager(&self) -> String {
        self.config().effective_packager()
    }
    fn effective_compression(&self) -> String {
        self.config().effective_compression()
    }
    fn effective_relations(&self, format: &str) -> lx_lib::pkgmeta::Relations {
        self.config().effective_relations(format)
    }
    fn effective_sign_type(&self) -> String {
        self.config().effective_sign_type()
    }
}

impl BuildMetadata for PackageConfig {
    fn config(&self) -> &PackageConfig {
        self
    }
}
