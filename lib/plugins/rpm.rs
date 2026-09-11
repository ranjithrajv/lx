// SPDX-License-Identifier: GPL-3.0-or-later

//! RPM `.rpm` plugin.
//!
//! Implements the `Plugin` trait for RPM packages using `lx_lib::rpmarchive`.
//! Shares the same staging logic as the deb plugin but emits a genuine RPM
//! (ED AB EE DB magic) instead of an ar+control+data archive.

use anyhow::{Context, Result};
use std::path::PathBuf;

use super::{BuildContext, Plugin};

pub struct RpmPlugin;

impl Plugin for RpmPlugin {
    fn name(&self) -> &'static str {
        "rpm"
    }

    fn file_extension(&self) -> &'static str {
        "rpm"
    }

    fn description(&self) -> &'static str {
        "RPM package (.rpm) — genuine RPM via lx_lib::rpmarchive"
    }

    fn default_distributions(&self) -> &'static [&'static str] {
        lx_lib::constants::DEFAULT_RPM_DISTRIBUTIONS
    }

    fn arch_supported_for_dist(&self, _arch: &str, _dist: &str) -> bool {
        // RPM arch matrix is largely independent of distro in our builder;
        // filter only unknown arches.
        true
    }

    fn build(&self, ctx: &BuildContext) -> Result<PathBuf> {
        let cfg = ctx.cfg;
        let job = ctx.job;

        // Stage install tree under ctx.staging_root (same layout as deb),
        // then layer the `contents:` overlay. (RPM %config marking of
        // contents entries is not yet wired into rpmarchive; files are
        // staged as regular payloads.)
        super::stage_install_tree(cfg, ctx.binary_dir, ctx.staging_root, ctx.mtime)?;
        let _conffiles = super::apply_contents(cfg, ctx.staging_root, "rpm")?;

        // RPM metadata: name, version, release, arch, summary, description,
        // license. Version is the stripped upstream version; release encodes
        // the build_version + dist (e.g. "1.fc38" or "1.el9").
        let version = ctx.debian_version.to_string();
        // RPM release must be dot-free of '+' etc. Map "1+fedora" -> "1.fc38".
        // For generic dist strings (fedora/el9), keep as is but replace '+'.
        let release = format!("{}+{}", ctx.build_version, job.dist).replace('+', ".");
        let summary = cfg.effective_description();
        let homepage = if cfg.effective_source() == "gitlab" {
            lx_lib::constants::homepage_for_gitlab(
                &cfg.github_repo,
                cfg.gitlab_host.as_deref().unwrap_or(""),
            )
        } else {
            lx_lib::constants::homepage_for_github(&cfg.github_repo)
        };
        let description = format!(
            "{summary}\nPackaged from the upstream release ({homepage}) for RPM-based distributions.",
        );
        let license = if cfg.license_spdx.is_empty() {
            "NOASSERTION"
        } else {
            cfg.license_spdx.as_str()
        };

        // RPM filename convention: {name}-{version}-{release}.{arch}.rpm
        // Version here is stripped (no epoch); epoch would be a separate
        // RPM header tag if needed.
        let rpm_name = format!(
            "{}-{}-{}.{}.rpm",
            cfg.package_name,
            version,
            release,
            to_rpm_arch(&job.arch)
        );

        // Sibling of the staging root, not inside it -- the payload is
        // staged from the whole staging tree, so an in-tree out dir
        // would package the artifact into itself.
        let out_dir = ctx
            .staging_root
            .parent()
            .context("staging root has no parent")?
            .join("__out");
        std::fs::create_dir_all(&out_dir)?;
        let rpm_dest = out_dir.join(&rpm_name);

        let relations = cfg.effective_relations("rpm");
        // Parse RPM triggers from config. Each trigger is a
        // "package: script_path" pair. Scripts are read relative to the
        // current working directory (the build environment).
        let trigger_specs = lx_lib::rpmarchive::parse_rpm_triggers(
            &cfg.rpm.trigger_pre_install,
            rpm::DependencyFlags::TRIGGERPREIN,
            std::path::Path::new("."),
        )?;
        let mut triggers: Vec<lx_lib::rpmarchive::RpmTrigger> = Vec::new();
        let mut trigger_flags: Vec<rpm::DependencyFlags> = Vec::new();
        for (trigger, flags) in trigger_specs {
            triggers.push(trigger);
            trigger_flags.push(flags);
        }
        for (trigger, flags) in lx_lib::rpmarchive::parse_rpm_triggers(
            &cfg.rpm.trigger_post_install,
            rpm::DependencyFlags::TRIGGERIN,
            std::path::Path::new("."),
        )? {
            triggers.push(trigger);
            trigger_flags.push(flags);
        }
        for (trigger, flags) in lx_lib::rpmarchive::parse_rpm_triggers(
            &cfg.rpm.trigger_pre_uninstall,
            rpm::DependencyFlags::TRIGGERUN,
            std::path::Path::new("."),
        )? {
            triggers.push(trigger);
            trigger_flags.push(flags);
        }
        for (trigger, flags) in lx_lib::rpmarchive::parse_rpm_triggers(
            &cfg.rpm.trigger_post_uninstall,
            rpm::DependencyFlags::TRIGGERPOSTUN,
            std::path::Path::new("."),
        )? {
            triggers.push(trigger);
            trigger_flags.push(flags);
        }
        let opts = lx_lib::rpmarchive::BuildOptions {
            pre_install: Some(cfg.scripts.preinstall.trim()),
            post_install: Some(cfg.scripts.postinstall.trim()),
            pre_uninstall: Some(cfg.scripts.preremove.trim()),
            post_uninstall: Some(cfg.scripts.postremove.trim()),
            pre_trans: Some(cfg.scripts.pretrans.trim()),
            post_trans: Some(cfg.scripts.posttrans.trim()),
            verify_script: Some(cfg.scripts.verify.trim()),
            sign_key_file: ctx.sign_key,
            sign_passphrase: ctx.sign_passphrase,
            relations: lx_lib::rpmarchive::parse_rpm_relations(
                &relations.depends,
                &relations.recommends,
                &relations.suggests,
                &relations.conflicts,
                &relations.replaces,
                &relations.provides,
                &relations.breaks,
            ),
            triggers,
            trigger_flags,
        };
        let meta = lx_lib::rpmarchive::PackageMeta {
            name: &cfg.package_name,
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
        )
        .with_context(|| format!("failed to build {}", rpm_dest.display()))?;

        Ok(rpm_dest)
    }
}

fn to_rpm_arch(debian_arch: &str) -> &'static str {
    lx_lib::constants::to_rpm_arch(debian_arch)
}
