//! Arch Linux pacman `.pkg.tar.zst` plugin.
//!
//! Implements `Plugin` for Arch packages using `lpt_lib::archarchive`.
//! Shares the same staging logic as deb/rpm but emits a `tar.zst`
//! containing `.PKGINFO` + `.MTREE` + payload.

use anyhow::{Context, Result};
use std::path::PathBuf;

use super::{BuildContext, Plugin};

pub struct ArchPlugin;

impl Plugin for ArchPlugin {
    fn name(&self) -> &'static str {
        "arch"
    }

    fn file_extension(&self) -> &'static str {
        "pkg.tar.zst"
    }

    fn description(&self) -> &'static str {
        "Arch Linux pacman package (.pkg.tar.zst) — tar.zst via lpt_lib::archarchive"
    }

    fn default_distributions(&self) -> &'static [&'static str] {
        lpt_lib::constants::DEFAULT_ARCH_DISTRIBUTIONS
    }

    fn arch_supported_for_dist(&self, _arch: &str, _dist: &str) -> bool {
        true
    }

    fn build(&self, ctx: &BuildContext) -> Result<PathBuf> {
        let cfg = ctx.cfg;
        let job = ctx.job;

        super::stage_install_tree(cfg, ctx.binary_dir, ctx.staging_root, ctx.mtime)?;
        // Layer the `contents:` overlay. (Arch has no conffile registry in
        // .PKGINFO; config-typed entries are staged as regular files.)
        let _conffiles = super::apply_contents(cfg, ctx.staging_root)?;

        let version = ctx.debian_version.to_string();
        // Arch's pkgver is {version}-{release}; release encodes build_version
        // + dist. For Arch we keep dist-agnostic (arch is rolling) but
        // preserve dist in release for matrix uniqueness, e.g. "1.arch".
        let release = format!("{}+{}", ctx.build_version, job.dist).replace('+', ".");
        // Filename: {name}-{version}-{release}-{arch}.pkg.tar.zst
        // Example: hello-1.0-1.arch-x86_64.pkg.tar.zst
        let arch_name = to_pacman_arch(&job.arch);
        let file_name = format!(
            "{}-{}-{}-{}.pkg.tar.zst",
            cfg.package_name, version, release, arch_name
        );

        let out_dir = ctx.staging_root.join("__out");
        std::fs::create_dir_all(&out_dir)?;
        let dest = out_dir.join(&file_name);

        let url = if cfg.effective_source() == "gitlab" {
            lpt_lib::constants::homepage_for_gitlab(
                &cfg.github_repo,
                cfg.gitlab_host.as_deref().unwrap_or(""),
            )
        } else {
            lpt_lib::constants::homepage_for_github(&cfg.github_repo)
        };
        let description = cfg.effective_description();
        let license = if cfg.license_spdx.is_empty() {
            "custom:unknown"
        } else {
            cfg.license_spdx.as_str()
        };

        let meta = lpt_lib::archarchive::PackageMeta {
            name: &cfg.package_name,
            version: &version,
            release: &release,
            description: &description,
            url: &url,
            license,
        };
        lpt_lib::archarchive::build(ctx.staging_root, &meta, &job.arch, ctx.mtime, &dest)
            .with_context(|| format!("failed to build {}", dest.display()))?;

        Ok(dest)
    }
}

fn to_pacman_arch(debian_arch: &str) -> &'static str {
    lpt_lib::constants::to_pacman_arch(debian_arch)
}
