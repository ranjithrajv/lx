// SPDX-License-Identifier: GPL-3.0-or-later

//! Dependency-mapper plugins: translate a language-ecosystem dependency
//! into a *target package format's* package name and version syntax.
//!
//! Mapping is format-specific — the same `libc6` becomes `glibc`/`libc6`
//! on RPM, `glibc` on Arch, `musl` on Alpine, and `libc` on OpenWrt, and
//! version operators differ (`>= 1` vs `>=1` vs bare). Rather than a growing
//! `match format` inside `depmap.rs`, each target format is a
//! [`DependencyMapper`]; the shared ecosystem→Debian tables remain the
//! common fast path the deb mapper (and the others' fallback) use.
//!
//! Selection: [`get_dependency_mapper(format)`] — `format` is a
//! `package_format` value (`deb`, `rpm`, `arch`, `apk`, `ipk`).

pub mod apk;
pub mod arch;
pub mod deb;
pub mod ipk;
pub mod rpm;

use crate::plugins::plugin::{Plugin, PluginSet};

/// A dependency-mapping backend for one target package format.
pub trait DependencyMapper: Plugin {
    /// The `package_format` this backend serves.
    fn format(&self) -> &'static str;

    /// Repology distro family for the online fallback, when one exists.
    fn repo_family(&self) -> Option<&'static str> {
        None
    }

    /// Map an ecosystem dependency name to this format's package name.
    fn map(&self, ecosystem: &str, dep_name: &str) -> Option<String>;

    /// Normalize a raw version constraint (`>= 1.2`, `^1.2`, `*`, …) into
    /// this format's operator syntax.
    fn constraint(&self, raw: &str) -> Option<String>;

    /// Render `name` + `constraint` exactly as this format's metadata
    /// expects (Debian/RPM use `name (>= 1)`; Alpine uses `name>=1`).
    fn render(&self, name: &str, constraint: Option<&str>) -> String {
        match constraint {
            Some(c) => format!("{name} ({c})"),
            None => name.to_string(),
        }
    }
}

/// All known mappers, in registration order.
pub fn all_dependency_mappers() -> Vec<Box<dyn DependencyMapper>> {
    vec![
        Box::new(deb::DebianDeps),
        Box::new(rpm::RpmDeps),
        Box::new(arch::ArchDeps),
        Box::new(apk::AlpineDeps),
        Box::new(ipk::OpenWrtDeps),
    ]
}

/// Look up a mapper by `package_format` (case-insensitive).
pub fn get_dependency_mapper(format: &str) -> Option<Box<dyn DependencyMapper>> {
    let lower = format.trim().to_ascii_lowercase();
    PluginSet::new(all_dependency_mappers()).take_first(|m| m.format() == lower)
}

/// Look up a mapper by backend name.
pub fn get_dependency_mapper_by_name(name: &str) -> Option<Box<dyn DependencyMapper>> {
    PluginSet::new(all_dependency_mappers()).take(name)
}

pub fn dependency_mapper_names() -> Vec<&'static str> {
    PluginSet::new(all_dependency_mappers()).names()
}

/// Convenience: map `dep_name` for `format`, or `None` if unmapped/unknown.
pub fn map_to_format(ecosystem: &str, dep_name: &str, format: &str) -> Option<String> {
    get_dependency_mapper(format).and_then(|m| m.map(ecosystem, dep_name))
}

/// Convenience: render one resolved dependency in the target format's syntax.
pub fn render_for_format(name: &str, constraint: Option<&str>, format: &str) -> String {
    match get_dependency_mapper(format) {
        Some(m) => m.render(name, constraint),
        None => match constraint {
            Some(c) => format!("{name} ({c})"),
            None => name.to_string(),
        },
    }
}
