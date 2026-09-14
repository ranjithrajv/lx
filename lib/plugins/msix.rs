// SPDX-License-Identifier: GPL-3.0-or-later

//! MSIX (Windows) plugin.
//!
//! Produces an unsigned MSIX from the staged tree and the `msix:` config
//! block, using [`lx_lib::msixarchive`]. `.pfx` signing is not implemented;
//! a non-empty `msix.signature.pfx_file` is reported as unsupported instead
//! of being silently dropped.

use anyhow::{bail, Context, Result};
use std::path::PathBuf;

use super::BuildMetadata;
use super::{BuildContext, Packager};
use crate::plugins::plugin::plugin_identity;

pub struct MsixPackager;

plugin_identity!(
    MsixPackager,
    "msix",
    "MSIX (Windows) package — unsigned OPC zip (AppxManifest/BlockMap)"
);

impl Packager for MsixPackager {
    fn file_extension(&self) -> &'static str {
        "msix"
    }

    fn default_distributions(&self) -> &'static [&'static str] {
        lx_lib::constants::DEFAULT_MSIX_DISTRIBUTIONS
    }

    fn arch_supported_for_dist(&self, _arch: &str, _dist: &str) -> bool {
        true
    }

    fn build(&self, ctx: &BuildContext) -> Result<PathBuf> {
        let cfg = ctx.cfg;
        let msix = cfg.msix();

        if msix.publisher.trim().is_empty() {
            bail!("msix.publisher is required to build an MSIX package");
        }
        if msix.applications.is_empty() {
            bail!("msix.applications must list at least one application");
        }
        if !msix.signature.pfx_file.trim().is_empty() {
            bail!(
                "msix.signature.pfx_file is not supported; use signature.key_file \
                 (PKCS#8 RSA private key, PEM) plus signature.cert_file (X.509 cert, PEM) \
                 for native AppxSignature.p7x signing"
            );
        }

        // Stage the install tree and the `contents:` overlay (file_info /
        // expand / disown_subtree apply to the payload paths).
        super::stage_install_tree(cfg.config(), ctx.binary_dir, ctx.staging_root, ctx.mtime)?;
        let _ = super::apply_contents_full(cfg.config(), ctx.staging_root, "msix")?;

        let version = to_msix_version(ctx.debian_version);
        let arch = msix_arch(&msix.arch, &ctx.job.arch);
        let manifest = build_manifest(cfg.config(), &version, &arch);
        let signer = build_signer(ctx)?;

        let file_name = format!("{}_{version}_{arch}.msix", cfg.package_name());
        let out_dir = super::output_dir(ctx.staging_root)?;
        let dest = out_dir.join(&file_name);

        lx_lib::msixarchive::build(ctx.staging_root, &manifest, &dest, signer.as_ref())
            .with_context(|| format!("failed to build {}", dest.display()))?;
        Ok(dest)
    }
}

/// Build the Appx signer from `signature.key_file` (PKCS#8 RSA key) and
/// `signature.cert_file` (X.509 cert), both PEM. `None` when no key is set.
fn build_signer(ctx: &BuildContext) -> Result<Option<xcommon::Signer>> {
    let Some(key_path) = ctx.sign_key else {
        return Ok(None);
    };
    let key = std::fs::read_to_string(key_path)
        .with_context(|| format!("reading signing key '{}'", key_path.display()))?;
    let cert_path = ctx.cfg.signature().cert_file.trim();
    let combined = if cert_path.is_empty() {
        key
    } else {
        let cert = std::fs::read_to_string(cert_path)
            .with_context(|| format!("reading signing cert '{cert_path}'"))?;
        format!("{cert}\n{key}")
    };
    xcommon::Signer::new(&combined)
        .map(Some)
        .context("parsing MSIX signing material (need a PEM certificate and a PKCS#8 RSA key)")
}

fn build_manifest(
    cfg: &crate::config::PackageConfig,
    version: &str,
    arch: &str,
) -> lx_lib::msixarchive::MsixManifest {
    let msix = cfg.msix();

    let mut applications = Vec::new();
    let mut needs_full_trust = false;
    for app in &msix.applications {
        let entry_point = if app.entry_point.trim().is_empty() {
            "Windows.FullTrustApplication".to_string()
        } else {
            app.entry_point.clone()
        };
        if entry_point == "Windows.FullTrustApplication" {
            needs_full_trust = true;
        }
        let logo = if msix.properties.logo.trim().is_empty() {
            String::new()
        } else {
            msix.properties.logo.clone()
        };
        let square150 = if app.visual_elements.square150x150_logo.trim().is_empty() {
            logo.clone()
        } else {
            app.visual_elements.square150x150_logo.clone()
        };
        let square44 = if app.visual_elements.square44x44_logo.trim().is_empty() {
            logo
        } else {
            app.visual_elements.square44x44_logo.clone()
        };
        applications.push(lx_lib::msixarchive::MsixApplication {
            id: app.id.clone(),
            executable: app.executable.clone(),
            entry_point,
            display_name: app.visual_elements.display_name.clone(),
            description: app.visual_elements.description.clone(),
            background_color: app.visual_elements.background_color.clone(),
            square150x150_logo: square150,
            square44x44_logo: square44,
        });
    }

    let mut device_families: Vec<lx_lib::msixarchive::TargetDeviceFamily> = msix
        .dependencies
        .target_device_families
        .iter()
        .map(|d| lx_lib::msixarchive::TargetDeviceFamily {
            name: d.name.clone(),
            min_version: d.min_version.clone(),
            max_version_tested: d.max_version_tested.clone(),
        })
        .collect();
    if device_families.is_empty() {
        device_families.push(lx_lib::msixarchive::TargetDeviceFamily {
            name: "Windows.Desktop".to_string(),
            min_version: "10.0.17763.0".to_string(),
            max_version_tested: "10.0.22621.0".to_string(),
        });
    }

    let mut restricted = msix.capabilities.restricted.clone();
    if needs_full_trust && !restricted.iter().any(|c| c == "runFullTrust") {
        restricted.push("runFullTrust".to_string());
    }

    lx_lib::msixarchive::MsixManifest {
        name: cfg.package_name().clone(),
        version: version.to_string(),
        publisher: msix.publisher.clone(),
        arch: arch.to_string(),
        resource_id: msix.identity.resource_id.clone(),
        display_name: msix.properties.display_name.clone(),
        publisher_display_name: msix.properties.publisher_display_name.clone(),
        description: cfg.effective_description(),
        logo: msix.properties.logo.clone(),
        applications,
        device_families,
        capabilities: msix.capabilities.capabilities.clone(),
        device_capabilities: msix.capabilities.device_capabilities.clone(),
        restricted,
    }
}

/// Convert a Debian-style version into MSIX's mandatory 4-part numeric
/// `Major.Minor.Build.Revision`.
fn to_msix_version(version: &str) -> String {
    // Drop an epoch prefix (`1:2.0` -> `2.0`) and any non-version tail.
    let v = version.rsplit(':').next().unwrap_or(version);
    let mut parts = [0u64; 4];
    for (i, raw) in v.split('.').take(4).enumerate() {
        let digits: String = raw.chars().take_while(|c| c.is_ascii_digit()).collect();
        parts[i] = digits.parse().unwrap_or(0);
    }
    format!("{}.{}.{}.{}", parts[0], parts[1], parts[2], parts[3])
}

/// Map a Debian architecture to MSIX's `ProcessorArchitecture` value,
/// honoring an explicit `msix.arch` override.
fn msix_arch(override_arch: &str, debian_arch: &str) -> String {
    let o = override_arch.trim();
    if !o.is_empty() {
        return o.to_string();
    }
    match debian_arch {
        "amd64" => "x64",
        "i386" | "i686" => "x86",
        "arm64" => "arm64",
        "armhf" | "armel" | "arm" => "arm",
        _ => "neutral",
    }
    .to_string()
}
