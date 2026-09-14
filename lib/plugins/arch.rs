// SPDX-License-Identifier: GPL-3.0-or-later

//! Arch Linux pacman `.pkg.tar.zst` plugin.
//!
//! Implements `Packager` for Arch packages using `lx_lib::archarchive`.
//! Shares the same staging logic as deb/rpm but emits a `tar.zst`
//! containing `.PKGINFO` + `.MTREE` + payload.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use super::{BuildContext, Packager};
use crate::plugins::plugin::plugin_identity;

pub struct ArchPackager;

plugin_identity!(
    ArchPackager,
    "arch",
    "Arch Linux pacman package (.pkg.tar.zst) — tar.zst via lx_lib::archarchive"
);

impl Packager for ArchPackager {
    fn file_extension(&self) -> &'static str {
        "pkg.tar.zst"
    }

    fn default_distributions(&self) -> &'static [&'static str] {
        lx_lib::constants::DEFAULT_ARCH_DISTRIBUTIONS
    }

    fn arch_supported_for_dist(&self, _arch: &str, _dist: &str) -> bool {
        true
    }

    fn build(&self, ctx: &BuildContext) -> Result<PathBuf> {
        super::stage_install_tree(ctx.cfg, ctx.binary_dir, ctx.staging_root, ctx.mtime)?;
        self.archive_staged_tree(ctx)
    }

    fn artifact_glob(&self, package: &str) -> String {
        format!("{package}-*.pkg.tar.*")
    }

    fn supports_source_build(&self) -> bool {
        true
    }

    fn archive_staged_tree(&self, ctx: &BuildContext) -> Result<PathBuf> {
        build_archive(ctx)
    }

    fn generate_source_package(&self, out_dir: &Path, pkg: &crate::source::Pkg) -> Result<()> {
        crate::source::generate_arch(out_dir, pkg)
    }
}

/// Archive an already-populated `staging_root` into a `.pkg.tar.zst`.
///
/// Shared tail of [`ArchPackager::build`]: layers `contents:`, builds
/// `.PKGINFO`/`.MTREE`, and writes the package. Source-mode builds populate
/// the staging root from a `DESTDIR` install tree instead of
/// `stage_install_tree` and reuse this directly.
fn build_archive(ctx: &BuildContext) -> Result<PathBuf> {
    let cfg = ctx.cfg;
    let job = ctx.job;

    // Layer the `contents:` overlay. Config-typed entries become the
    // pacman `backup` list (preserved on upgrade/removal).
    let (configs, file_meta) = super::apply_contents_full(cfg, ctx.staging_root, "arch")?;

    let version = ctx.debian_version.to_string();
    let release = super::format_release(ctx.build_version, &job.dist);
    let epoch = cfg.epoch.trim();
    // pacman encodes the epoch into the version string (`1:2.0-1`);
    // makepkg's filename uses the same `get_full_version`.
    let full_version = if epoch.is_empty() {
        format!("{version}-{release}")
    } else {
        format!("{epoch}:{version}-{release}")
    };
    let arch_name = to_pacman_arch(&job.arch);
    let file_name = format!(
        "{}-{}-{}.pkg.tar.zst",
        cfg.package_name, full_version, arch_name
    );

    let out_dir = super::output_dir(ctx.staging_root)?;
    let dest = out_dir.join(&file_name);

    let url = super::resolve_homepage(cfg);
    let description = cfg.effective_description();
    let license = if cfg.license_spdx.is_empty() {
        "custom:unknown"
    } else {
        cfg.license_spdx.as_str()
    };

    // Relation metadata: translate the Debian-style relation fields to
    // pacman syntax, then merge in ELF-detected dependencies.
    let rel = cfg.effective_relations("arch");
    let mut depends = lx_lib::archarchive::debian_relations_to_pacman(&rel.depends);
    for dep in &ctx.detected_deps {
        if !dep.trim().is_empty() {
            depends.push(dep.trim().to_string());
        }
    }
    depends.sort();
    depends.dedup();

    let mut optdepends = lx_lib::archarchive::debian_relations_to_pacman(&rel.recommends);
    optdepends.extend(lx_lib::archarchive::debian_relations_to_pacman(
        &rel.suggests,
    ));
    optdepends.sort();
    optdepends.dedup();

    let mut conflicts = lx_lib::archarchive::debian_relations_to_pacman(&rel.conflicts);
    conflicts.extend(lx_lib::archarchive::debian_relations_to_pacman(&rel.breaks));
    conflicts.sort();
    conflicts.dedup();

    let mut provides = lx_lib::archarchive::debian_relations_to_pacman(&rel.provides);
    provides.sort();
    provides.dedup();

    let mut replaces = lx_lib::archarchive::debian_relations_to_pacman(&rel.replaces);
    replaces.sort();
    replaces.dedup();

    // pacman `backup` paths are relative, without a leading slash.
    let backup: Vec<String> = configs
        .iter()
        .map(|c| c.path.trim_start_matches('/').to_string())
        .collect();

    let relations = lx_lib::archarchive::PackageRelations {
        depends: &depends,
        optdepends: &optdepends,
        conflicts: &conflicts,
        provides: &provides,
        replaces: &replaces,
        backup: &backup,
    };

    let meta = lx_lib::archarchive::PackageMeta {
        name: &cfg.package_name,
        version: &version,
        release: &release,
        description: &description,
        url: &url,
        license,
    };

    let packager = cfg.effective_packager();
    let packager = packager.trim();
    let packager = if packager.is_empty() {
        None
    } else {
        Some(packager)
    };

    // Upgrade hooks: `scripts.preupgrade_script`/`postupgrade_script`
    // are the documented cross-format fields (`lx convert` writes
    // those); fall back to the Arch-native `preupgrade`/`postupgrade`.
    let pre_path = pick(&cfg.scripts.preupgrade_script, &cfg.scripts.preupgrade);
    let post_path = pick(&cfg.scripts.postupgrade_script, &cfg.scripts.postupgrade);
    let pre = super::render_script_body(cfg, job, pre_path)?;
    let post = super::render_script_body(cfg, job, post_path)?;
    let install_script = lx_lib::archarchive::render_install_script(
        pre.as_deref().unwrap_or(""),
        post.as_deref().unwrap_or(""),
    );

    lx_lib::archarchive::build_with_relations(
        ctx.staging_root,
        &meta,
        &relations,
        packager,
        (!epoch.is_empty()).then_some(epoch),
        &job.arch,
        ctx.mtime,
        &dest,
        install_script.as_deref(),
        &file_meta,
    )
    .with_context(|| format!("failed to build {}", dest.display()))?;

    Ok(dest)
}

/// Prefer the cross-format `*_script` field when set, else the Arch-native
/// inline field.
fn pick<'a>(preferred: &'a str, fallback: &'a str) -> &'a str {
    if preferred.trim().is_empty() {
        fallback
    } else {
        preferred
    }
}

fn to_pacman_arch(debian_arch: &str) -> &'static str {
    lx_lib::constants::to_pacman_arch(debian_arch)
}
