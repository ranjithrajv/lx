// SPDX-License-Identifier: GPL-3.0-or-later

//! RPM `.rpm` plugin.
//!
//! Implements the `Packager` trait for RPM packages using `lx_lib::rpmarchive`.
//! Shares the same staging logic as the deb plugin but emits a genuine RPM
//! (ED AB EE DB magic) instead of an ar+control+data archive.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use super::{BuildContext, Packager, SourcePackager};
use crate::plugins::plugin::plugin_identity;

pub struct RpmPackager;

plugin_identity!(
    RpmPackager,
    "rpm",
    "RPM package (.rpm) — genuine RPM via lx_lib::rpmarchive"
);

impl Packager for RpmPackager {
    fn file_extension(&self) -> &'static str {
        "rpm"
    }

    fn default_distributions(&self) -> &'static [&'static str] {
        lx_lib::constants::DEFAULT_RPM_DISTRIBUTIONS
    }

    fn arch_supported_for_dist(&self, _arch: &str, _dist: &str) -> bool {
        true
    }

    fn build(&self, ctx: &BuildContext) -> Result<PathBuf> {
        // Stage install tree under ctx.staging_root (same layout as deb),
        // then archive it as an rpm.
        super::stage_install_tree(
            ctx.cfg.config(),
            ctx.binary_dir,
            ctx.staging_root,
            ctx.mtime,
        )?;
        self.archive_staged_tree(ctx)
    }

    fn artifact_glob(&self, package: &str) -> String {
        format!("{package}-*.rpm")
    }
}

impl SourcePackager for RpmPackager {
    fn archive_staged_tree(&self, ctx: &BuildContext) -> Result<PathBuf> {
        build_archive(ctx)
    }

    fn generate_source_package(&self, out_dir: &Path, pkg: &crate::source::Pkg) -> Result<()> {
        crate::source::generate_rpm(out_dir, pkg)
    }
}

/// Archive an already-populated `staging_root` into an `.rpm`.
///
/// Shared tail of [`RpmPackager::build`]: layers `contents:`, builds the
/// header, and writes the RPM. Source-mode builds populate the staging root
/// from a `DESTDIR` install tree instead of `stage_install_tree` and reuse
/// this directly.
fn build_archive(ctx: &BuildContext) -> Result<PathBuf> {
    let cfg = ctx.cfg;
    let job = ctx.job;

    // Layer the `contents:` overlay. Config-typed entries become
    // `%config` / `%config(noreplace)` markers.
    let (configs, file_meta) = super::apply_contents_full(cfg.config(), ctx.staging_root, "rpm")?;

    // RPM metadata: name, version, release, arch, summary, description,
    // license. Version is the stripped upstream version; release encodes
    // the build_version + dist (e.g. "1.fc38" or "1.el9").
    let version = ctx.debian_version.to_string();
    let release = super::format_release(ctx.build_version, &job.dist);
    let summary = cfg.effective_description();
    let homepage = super::resolve_homepage(cfg.config());
    let description = format!(
        "{summary}\nPackaged from the upstream release ({homepage}) for RPM-based distributions.",
    );
    let license = if cfg.license_spdx().is_empty() {
        "NOASSERTION"
    } else {
        cfg.license_spdx()
    };

    // RPM filename convention: {name}-{version}-{release}.{arch}.rpm
    let rpm_name = format!(
        "{}-{}-{}.{}.rpm",
        cfg.package_name(),
        version,
        release,
        to_rpm_arch(&job.arch)
    );

    let out_dir = super::output_dir(ctx.staging_root)?;
    let rpm_dest = out_dir.join(&rpm_name);

    let relations = cfg.effective_relations("rpm");

    // Parse scriptlet bodies from their configured file paths (they are
    // paths in the build environment, same as deb/rpm script config),
    // applying templating when enabled. Upgrade scripts fall back to the
    // cross-format `*_script` fields (`lx convert` writes those).
    let pre_install = super::render_script_body(cfg.config(), job, &cfg.scripts().preinstall)?;
    let post_install = super::render_script_body(cfg.config(), job, &cfg.scripts().postinstall)?;
    let pre_uninstall = super::render_script_body(cfg.config(), job, &cfg.scripts().preremove)?;
    let post_uninstall = super::render_script_body(cfg.config(), job, &cfg.scripts().postremove)?;
    let pre_trans = super::render_script_body(
        cfg.config(),
        job,
        pick(&cfg.scripts().pretrans, &cfg.scripts().preupgrade_script),
    )?;
    let post_trans = super::render_script_body(
        cfg.config(),
        job,
        pick(&cfg.scripts().posttrans, &cfg.scripts().postupgrade_script),
    )?;
    let verify = super::render_script_body(cfg.config(), job, &cfg.scripts().verify)?;

    // Parse RPM triggers from config. Each trigger is a
    // "package: script_path" pair. Scripts are read relative to the
    // current working directory (the build environment).
    let mut triggers: Vec<lx_lib::rpmarchive::RpmTrigger> = Vec::new();
    let mut trigger_flags: Vec<rpm::DependencyFlags> = Vec::new();
    for (field, flag) in [
        (
            &cfg.rpm().trigger_pre_install,
            rpm::DependencyFlags::TRIGGERPREIN,
        ),
        (
            &cfg.rpm().trigger_post_install,
            rpm::DependencyFlags::TRIGGERIN,
        ),
        (
            &cfg.rpm().trigger_pre_uninstall,
            rpm::DependencyFlags::TRIGGERUN,
        ),
        (
            &cfg.rpm().trigger_post_uninstall,
            rpm::DependencyFlags::TRIGGERPOSTUN,
        ),
    ] {
        let specs = lx_lib::rpmarchive::parse_rpm_triggers(field, flag, std::path::Path::new("."))?;
        for (trigger, flags) in specs {
            triggers.push(trigger);
            trigger_flags.push(flags);
        }
    }

    // Relation fields, plus ELF-detected dependencies (`--bindep`),
    // merged into Requires to match the deb plugin's behavior.
    let mut rpm_relations = lx_lib::rpmarchive::parse_rpm_relations(
        &relations.depends,
        &relations.recommends,
        &relations.suggests,
        &relations.conflicts,
        &relations.replaces,
        &relations.provides,
        &relations.breaks,
        &relations.predepends,
    );
    for dep in &ctx.detected_deps {
        let dep = dep.trim();
        if !dep.is_empty() && !rpm_relations.requires.iter().any(|d| d.name == dep) {
            rpm_relations
                .requires
                .push(rpm::Dependency::any(dep.to_string()));
        }
    }

    let config_files: Vec<String> = configs
        .iter()
        .filter(|c| !c.noreplace)
        .map(|c| c.path.clone())
        .collect();
    let config_noreplace_files: Vec<String> = configs
        .iter()
        .filter(|c| c.noreplace)
        .map(|c| c.path.clone())
        .collect();

    let opts = lx_lib::rpmarchive::BuildOptions {
        pre_install: pre_install.as_deref(),
        post_install: post_install.as_deref(),
        pre_uninstall: pre_uninstall.as_deref(),
        post_uninstall: post_uninstall.as_deref(),
        pre_trans: pre_trans.as_deref(),
        post_trans: post_trans.as_deref(),
        verify_script: verify.as_deref(),
        sign_key_file: ctx.sign_key,
        sign_passphrase: ctx.sign_passphrase,
        relations: rpm_relations,
        triggers,
        trigger_flags,
        compression: cfg.rpm().compression.clone(),
        auto_provides: cfg.rpm().auto_provides,
        auto_requires: cfg.rpm().auto_requires,
        defines: cfg.rpm().defines.clone(),
        config_files,
        config_noreplace_files,
        epoch: cfg.epoch().trim().parse::<u32>().ok(),
        buildhost: cfg.rpm().buildhost.clone(),
    };
    // Fields accepted for nfpm parity that the in-process rpm crate cannot
    // express: report them rather than dropping them silently.
    if !cfg.rpm().group.is_empty() {
        eprintln!(
            "    ⚠ rpm.group is ignored: the rpm crate hardcodes Group to \"Unspecified\" (needs header injection)"
        );
    }
    if !cfg.rpm().prefixes.is_empty() {
        eprintln!(
            "    ⚠ rpm.prefixes is not supported by the in-process rpm builder; ignoring {:?}",
            cfg.rpm().prefixes
        );
    }
    if !cfg.rpm().requires_post.is_empty() {
        eprintln!(
            "    ⚠ rpm.requires_post has no distinct post-requires setter in the rpm crate; ignoring {:?}",
            cfg.rpm().requires_post
        );
    }
    let meta = lx_lib::rpmarchive::PackageMeta {
        name: cfg.package_name(),
        version: &version,
        release: &release,
        summary: &summary,
        description: &description,
        license,
        vendor: None,
        packager: Some(&cfg.effective_packager()),
    };
    lx_lib::rpmarchive::build_with_options(
        ctx.staging_root,
        &meta,
        &job.arch,
        ctx.mtime,
        &rpm_dest,
        &opts,
        &file_meta,
    )
    .with_context(|| format!("failed to build {}", rpm_dest.display()))?;

    Ok(rpm_dest)
}

/// Prefer `preferred` when set, else `fallback` (both are script paths).
fn pick<'a>(preferred: &'a str, fallback: &'a str) -> &'a str {
    if preferred.trim().is_empty() {
        fallback
    } else {
        preferred
    }
}

fn to_rpm_arch(debian_arch: &str) -> &'static str {
    lx_lib::constants::to_rpm_arch(debian_arch)
}
