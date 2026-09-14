// SPDX-License-Identifier: GPL-3.0-or-later

//! OpenWrt / opkg `.ipk` plugin.
//!
//! Implements `Packager` using `lx_lib::ipkarchive`. An `.ipk` shares the
//! `.deb` `ar` container layout (`debian-binary` + `control.tar.gz` +
//! `data.tar.gz`), so staging and archiving are shared; only the control
//! dialect and filename convention differ. Used for embedded Linux targets
//! (OpenWrt, and opkg-based distros).

use anyhow::{Context, Result};
use std::path::PathBuf;

use super::{BuildContext, Packager};
use crate::plugins::plugin::plugin_identity;

pub struct IpkPackager;

plugin_identity!(
    IpkPackager,
    "ipk",
    "OpenWrt package (.ipk) — ar + control.tar.gz + data.tar.gz (opkg)"
);

impl Packager for IpkPackager {
    fn file_extension(&self) -> &'static str {
        "ipk"
    }

    fn default_distributions(&self) -> &'static [&'static str] {
        lx_lib::constants::DEFAULT_IPK_DISTRIBUTIONS
    }

    fn arch_supported_for_dist(&self, _arch: &str, _dist: &str) -> bool {
        true
    }

    fn build(&self, ctx: &BuildContext) -> Result<PathBuf> {
        let cfg = ctx.cfg;
        let job = ctx.job;

        super::stage_install_tree(cfg, ctx.binary_dir, ctx.staging_root, ctx.mtime)?;
        let (conffiles, file_meta) = super::apply_contents_full(cfg, ctx.staging_root, "ipk")?;
        let conffiles: Vec<String> = conffiles.into_iter().map(|c| c.path).collect();

        let version = ctx.debian_version.to_string();
        let build = if ctx.build_version.trim().is_empty() {
            "1"
        } else {
            ctx.build_version
        };
        let full_version = format!("{version}-{build}");
        let arch = lx_lib::constants::to_openwrt_arch(&job.arch);
        let file_name = format!("{}_{full_version}_{arch}.ipk", cfg.package_name);

        let control = render_control(cfg, ctx, &full_version, arch);

        // opkg maintainer scripts (`preinst`/`postinst`/`prerm`/`postrm`)
        // plus a `conffiles` list, all as control.tar.gz members.
        let mut extras: Vec<lx_lib::debarchive::ControlMember> =
            super::maintainer_script_members(ctx)?
                .into_iter()
                .filter(|m| matches!(m.name.as_str(), "preinst" | "postinst" | "prerm" | "postrm"))
                .collect();
        if !conffiles.is_empty() {
            extras.push(lx_lib::debarchive::ControlMember {
                name: "conffiles".to_string(),
                content: format!("{}\n", conffiles.join("\n")).into_bytes(),
                mode: 0o644,
            });
        }

        let out_dir = super::output_dir(ctx.staging_root)?;
        let dest = out_dir.join(&file_name);
        lx_lib::ipkarchive::build_with_scripts_with_meta(
            ctx.staging_root,
            control.as_bytes(),
            &extras,
            ctx.mtime,
            &dest,
            &file_meta,
        )
        .with_context(|| format!("failed to build {}", dest.display()))?;

        Ok(dest)
    }
}

/// Render the OpenWrt control file. Mirrors the deb control dialect but
/// uses opkg's field names (`Architecture`, `Installed-Size`, `License`).
fn render_control(
    cfg: &crate::config::PackageConfig,
    ctx: &BuildContext,
    full_version: &str,
    arch: &str,
) -> String {
    let relations = cfg.effective_relations("ipk");
    let mut depends = split_list(&relations.depends);
    for dep in &ctx.detected_deps {
        if !dep.trim().is_empty() {
            depends.push(dep.trim().to_string());
        }
    }
    let mut out = String::new();
    out.push_str(&format!("Package: {}\n", cfg.package_name));
    out.push_str(&format!("Version: {full_version}\n"));
    if !depends.is_empty() {
        out.push_str(&format!("Depends: {}\n", depends.join(", ")));
    }
    if !relations.provides.trim().is_empty() {
        out.push_str(&format!("Provides: {}\n", relations.provides.trim()));
    }
    if !relations.conflicts.trim().is_empty() {
        out.push_str(&format!("Conflicts: {}\n", relations.conflicts.trim()));
    }
    out.push_str(&format!("Section: {}\n", cfg.effective_section()));
    out.push_str(&format!("Priority: {}\n", cfg.effective_priority()));
    out.push_str(&format!("Maintainer: {}\n", cfg.effective_maintainer()));
    out.push_str(&format!("Architecture: {arch}\n"));
    out.push_str(&format!(
        "Installed-Size: {}\n",
        installed_size_bytes(ctx.staging_root)
    ));
    if !cfg.license_spdx.trim().is_empty() {
        out.push_str(&format!("License: {}\n", cfg.license_spdx.trim()));
    }
    // nfpm `ipk:` block parity.
    if !cfg.ipk.abi_version.trim().is_empty() {
        out.push_str(&format!("ABIVersion: {}\n", cfg.ipk.abi_version.trim()));
    }
    if !cfg.ipk.tags.is_empty() {
        out.push_str(&format!("Tags: {}\n", cfg.ipk.tags.join(" ")));
    }
    if cfg.ipk.auto_installed {
        out.push_str("Auto-Installed: yes\n");
    }
    if cfg.ipk.essential {
        out.push_str("Essential: yes\n");
    }
    if !cfg.ipk.alternatives.is_empty() {
        let alts: Vec<String> = cfg
            .ipk
            .alternatives
            .iter()
            .map(|a| format!("{}:{}:{}", a.priority, a.link_name, a.target))
            .collect();
        out.push_str(&format!("Alternatives: {}\n", alts.join(", ")));
    }
    out.push_str(&format!("Description: {}\n", cfg.effective_description()));
    out
}

fn split_list(s: &str) -> Vec<String> {
    s.split(',')
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .collect()
}

/// Sum of payload sizes in bytes (opkg's `Installed-Size` convention —
/// unlike Debian, which uses KiB).
fn installed_size_bytes(root: &std::path::Path) -> u64 {
    fn walk(dir: &std::path::Path, total: &mut u64) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let Ok(ty) = entry.file_type() else { continue };
            if ty.is_dir() {
                walk(&entry.path(), total);
            } else if ty.is_file() {
                if let Ok(meta) = entry.metadata() {
                    *total += meta.len();
                }
            }
        }
    }
    let mut total = 0u64;
    walk(root, &mut total);
    total
}
