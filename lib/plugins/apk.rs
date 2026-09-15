// SPDX-License-Identifier: GPL-3.0-or-later

//! Alpine Linux `.apk` (apk-tools v2) plugin.
//!
//! Implements `Packager` using `lx_lib::apkarchive`. Shares the same
//! staging logic as deb/rpm/arch; emits the concatenated gzip-member apk
//! format. Because Alpine is musl-only, this is the natural home for
//! `musl: true` builds.
//!
//! apk signing needs an RSA key produced by `abuild`; unsigned packages
//! install with `apk add --allow-untrusted`.

use anyhow::{Context, Result};
use std::path::PathBuf;

use super::{BuildContext, Packager};
use crate::plugins::plugin::plugin_identity;

pub struct ApkPackager;

plugin_identity!(
    ApkPackager,
    "apk",
    "Alpine Linux package (.apk) — concatenated gzip members via lx_lib::apkarchive"
);

impl Packager for ApkPackager {
    fn file_extension(&self) -> &'static str {
        "apk"
    }

    fn default_distributions(&self) -> &'static [&'static str] {
        lx_lib::constants::DEFAULT_APK_DISTRIBUTIONS
    }

    fn arch_supported_for_dist(&self, _arch: &str, _dist: &str) -> bool {
        true
    }

    fn artifact_glob(&self, package: &str) -> String {
        format!("{package}-*.apk")
    }

    fn build(&self, ctx: &BuildContext) -> Result<PathBuf> {
        let cfg = ctx.cfg;
        let job = ctx.job;

        super::stage_install_tree(cfg, ctx.binary_dir, ctx.staging_root, ctx.mtime)?;
        let (_conffiles, file_meta) = super::apply_contents_full(cfg, ctx.staging_root, "apk")?;

        let version = ctx.debian_version.to_string();
        let revision = if ctx.build_version.trim().is_empty() {
            "1"
        } else {
            ctx.build_version
        };
        // Alpine combines version + revision into `pkgver` (`1.0.0-r1`).
        let pkgver = format!("{version}-r{revision}");
        let arch = lx_lib::apkarchive::to_alpine_arch(&job.arch);
        // Include the arch in the filename — every other format does — so
        // multi-arch builds into a flat output dir don't overwrite each other.
        // The `.PKGINFO` segment carries the canonical `arch=`, so the apk index
        // is unaffected.
        let file_name = format!("{}-{pkgver}-{arch}.apk", cfg.package_name);

        let relations = cfg.effective_relations("apk");
        let mut depends = split_list(&relations.depends);
        for dep in &ctx.detected_deps {
            if !dep.trim().is_empty() {
                depends.push(dep.trim().to_string());
            }
        }
        let provides = split_list(&relations.provides);
        let replaces = split_list(&relations.replaces);

        let out_dir = super::output_dir(ctx.staging_root)?;
        let dest = out_dir.join(&file_name);

        let url = super::resolve_homepage(cfg);
        let description = cfg.effective_description();
        let meta = lx_lib::apkarchive::PackageMeta {
            name: &cfg.package_name,
            version: &pkgver,
            description: &description,
            url: &url,
            license: &cfg.license_spdx,
            depends: &depends,
            provides: &provides,
            replaces: &replaces,
        };
        // apk install scripts go in the control segment (dotted names).
        let mut control_owned: Vec<(String, Vec<u8>, u32)> = Vec::new();
        for m in super::maintainer_script_members(ctx)? {
            let apk_name = match m.name.as_str() {
                "preinst" => ".pre-install",
                "postinst" => ".post-install",
                "prerm" => ".pre-deinstall",
                "postrm" => ".post-deinstall",
                "preupgrade" => ".pre-upgrade",
                "postupgrade" => ".post-upgrade",
                _ => continue,
            };
            control_owned.push((apk_name.to_string(), m.content, 0o755));
        }
        let control_files: Vec<lx_lib::apkarchive::ApkControlFile> = control_owned
            .iter()
            .map(|(name, content, mode)| lx_lib::apkarchive::ApkControlFile {
                name: name.as_str(),
                content,
                mode: *mode,
            })
            .collect();

        // apk v2 signing: sign the compressed control segment and prepend a
        // `.SIGN.RSA.<keyname>` segment. `sign_key` must be an RSA private
        // key (PEM); `sign_key_id`, when set, is the key name in the member.
        let member;
        let sign;
        let signer = if let Some(key) = ctx.sign_key {
            member = format!(
                ".SIGN.RSA.{}",
                lx_lib::sign::apk_key_name(key, ctx.sign_key_id)
            );
            sign = |control_gz: &[u8]| {
                lx_lib::sign::rsa_sha1_sign(control_gz, key, ctx.sign_passphrase)
            };
            Some(lx_lib::apkarchive::ApkSigner {
                member_name: &member,
                sign: &sign,
            })
        } else {
            None
        };
        lx_lib::apkarchive::build_full_with_meta(
            ctx.staging_root,
            &meta,
            arch,
            ctx.mtime,
            &dest,
            signer,
            &control_files,
            &file_meta,
        )
        .with_context(|| format!("failed to build {}", dest.display()))?;

        Ok(dest)
    }
}

/// Split a deb-style comma-separated relation string into trimmed entries.
fn split_list(s: &str) -> Vec<String> {
    s.split(',')
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .collect()
}
