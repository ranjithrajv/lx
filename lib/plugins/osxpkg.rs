// SPDX-License-Identifier: GPL-3.0-or-later

//! macOS flat package (`.pkg`) plugin, mirroring fpm's `osxpkg` output.
//!
//! fpm shells out to macOS's `pkgbuild`; this plugin builds the xar
//! container in-process ([`lx_lib::osxpkgarchive`]), so a package can be
//! produced on any host. The archive is unsigned and carries no `Bom`.

use anyhow::{Context, Result};
use std::path::PathBuf;

use super::{BuildContext, Packager};
use crate::plugins::plugin::plugin_identity;

pub struct OsxPkgPackager;

plugin_identity!(
    OsxPkgPackager,
    "osxpkg",
    "macOS flat package (.pkg) — xar + cpio payload (unsigned)"
);

impl Packager for OsxPkgPackager {
    fn file_extension(&self) -> &'static str {
        "pkg"
    }

    fn default_distributions(&self) -> &'static [&'static str] {
        lx_lib::constants::DEFAULT_OSX_DISTRIBUTIONS
    }

    fn arch_supported_for_dist(&self, _arch: &str, _dist: &str) -> bool {
        true
    }

    fn artifact_glob(&self, package: &str) -> String {
        format!("{package}-*.pkg")
    }

    fn build(&self, ctx: &BuildContext) -> Result<PathBuf> {
        let cfg = ctx.cfg;
        let job = ctx.job;

        super::stage_install_tree(cfg, ctx.binary_dir, ctx.staging_root, ctx.mtime)?;
        let _ = super::apply_contents_full(cfg, ctx.staging_root, "osxpkg")?;

        // Install scripts map to Scripts/preinstall + Scripts/postinstall
        // (the cross-format `*_script` fields, templated like the others).
        let mut scripts: Vec<(String, Vec<u8>)> = Vec::new();
        let mut script_names: Vec<&str> = Vec::new();
        if let Some(body) = super::render_script_body(cfg, job, &cfg.scripts.preinstall)? {
            scripts.push(("preinstall".to_string(), body.into_bytes()));
            script_names.push("preinstall");
        }
        if let Some(body) = super::render_script_body(cfg, job, &cfg.scripts.postinstall)? {
            scripts.push(("postinstall".to_string(), body.into_bytes()));
            script_names.push("postinstall");
        }

        let version = ctx.debian_version.to_string();
        let file_name = format!("{}-{version}.pkg", cfg.package_name);
        let out_dir = super::output_dir(ctx.staging_root)?;
        let dest = out_dir.join(&file_name);

        // `prefix:` doubles as the install location (fpm's
        // `--prefix`/`--install-location`); default is the filesystem root.
        let install_location = if cfg.prefix.trim().is_empty() {
            "/"
        } else {
            cfg.prefix.trim()
        };
        let info = lx_lib::osxpkgarchive::OsxPackageInfo {
            identifier: &cfg.package_name,
            version: &version,
            install_location,
            postinstall_action: "none",
            scripts: &script_names,
        };
        lx_lib::osxpkgarchive::build(ctx.staging_root, &info, &scripts, &dest, ctx.mtime)
            .with_context(|| format!("failed to build {}", dest.display()))?;
        Ok(dest)
    }
}
