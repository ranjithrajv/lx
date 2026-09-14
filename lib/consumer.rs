// SPDX-License-Identifier: GPL-3.0-or-later

//! Format-aware consumer layer for `install`/`upgrade`/`update`/`remove`/
//! `list`/`show`/`rollback`.
//!
//! Historically these commands spoke only Debian: the remote side was the
//! `latest-debs` GitHub org (`<package>-debian` repos, `+{dist}_{arch}.deb`
//! assets) and the local side was dpkg. This module generalizes both ends to
//! the host's native format ([`InstallFormat`]) so rpm and pacman hosts get
//! the same `lx`/`lx get` workflow instead of being sent to `dnf`/`pacman`
//! by hand, and lets the package org be overridden with `LX_INDEX_ORG`.
//!
//! The org layout follows the same per-format repo convention for each half:
//! `<package>-debian`, `<package>-rpm`, `<package>-arch`, with a
//! `{package}_{version}+{dist}_{arch}` / `{name}-{version}-{release}.{arch}`
//! asset inside depending on the packager (the per-format filename rules are
//! owned by the packager plugins; this module only *matches* them).

use anyhow::{bail, Context, Result};
use std::cmp::Ordering;
use std::path::Path;
use std::process::Command;

use crate::debs;
use crate::index::InstallFormat;
use crate::install_pkg;
use lx_lib::github::{Asset, Release};

/// Env var overriding the org that hosts lx-built packages.
pub const INDEX_ORG_ENV_VAR: &str = "LX_INDEX_ORG";

/// The org `lx install`/`upgrade`/… fetch from. `LX_INDEX_ORG` wins;
/// otherwise the built-in `latest-debs` default. Reading it through a
/// function (rather than the constant directly) is what lets a project point
/// the consumer client at *its own* org.
pub fn index_org() -> String {
    std::env::var(INDEX_ORG_ENV_VAR)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| debs::LATEST_DEBS_ORG.to_string())
}

/// GitHub repo name for `package` in the configured org. A package has one
/// repo (`<package>-debian`, a historical naming choice) that hosts all of
/// its format assets — the same convention `go-native` already relies on —
/// so the format selection happens at the asset level, not the repo level.
pub fn repo_name(package: &str) -> String {
    debs::repo_name(package)
}

/// The host distro token this format's assets are tagged with, used to pick
/// the right asset when a release carries several. Debian/Ubuntu use the
/// codename (`bookworm`), RPM uses the lx family (`fedora`, `el9`, …), Arch
/// is rolling (`arch`). `None` when it can't be determined, in which case
/// the first arch match wins.
pub fn host_dist(format: InstallFormat) -> Option<String> {
    match format {
        InstallFormat::Deb => debs::detect_dist(),
        InstallFormat::Arch => Some("arch".to_string()),
        InstallFormat::Rpm => rpm_host_dist(),
    }
}

/// Best-effort RPM distro family from `/etc/os-release`, matching lx's
/// `DEFAULT_RPM_DISTRIBUTIONS` (`fedora`, `el9`, `el8`, `opensuse`).
fn rpm_host_dist() -> Option<String> {
    let text = std::fs::read_to_string("/etc/os-release").ok()?;
    rpm_dist_from_os_release(&text)
}

/// Pure `/etc/os-release` parser behind [`rpm_host_dist`], split out so the
/// ID/version mapping is testable without a specific host.
fn rpm_dist_from_os_release(text: &str) -> Option<String> {
    let mut id = String::new();
    let mut version_id = String::new();
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("ID=") {
            id = v.trim().trim_matches('"').to_ascii_lowercase();
        } else if let Some(v) = line.strip_prefix("VERSION_ID=") {
            version_id = v.trim().trim_matches('"').to_string();
        }
    }
    if id.starts_with("opensuse") {
        return Some("opensuse".to_string());
    }
    if id == "fedora" {
        return Some("fedora".to_string());
    }
    if matches!(id.as_str(), "rhel" | "centos" | "almalinux" | "rocky") {
        let major = version_id.split('.').next().unwrap_or("");
        if !major.is_empty() {
            return Some(format!("el{major}"));
        }
    }
    None
}

/// Parse a `--format` flag value into an [`InstallFormat`]. Accepts the
/// packager name plus the native package-manager aliases a user is likely to
/// type.
pub fn parse_format(value: &str) -> Result<InstallFormat> {
    match value.trim().to_ascii_lowercase().as_str() {
        "deb" | "debian" | "dpkg" | "apt" => Ok(InstallFormat::Deb),
        "rpm" | "dnf" | "yum" | "zypper" => Ok(InstallFormat::Rpm),
        "arch" | "pacman" | "alpm" => Ok(InstallFormat::Arch),
        other => bail!("unsupported --format '{other}' (expected deb, rpm, or arch)"),
    }
}

/// Resolve a format name recorded in the manifest, falling back to the
/// host's own package manager when it is empty or unrecognized (entries
/// written before the manifest carried a format).
pub fn format_or_host(name: &str) -> InstallFormat {
    parse_format(name).unwrap_or_else(|_| crate::index::detect_host_format())
}

/// A resolved release asset plus the version string lx records for it.
#[derive(Debug, Clone)]
pub struct ResolvedAsset<'a> {
    pub asset: &'a Asset,
    /// The version used for upgrade comparison and the manifest. For deb,
    /// the dpkg `Version` content; for rpm/arch, `[epoch:]version-release`.
    pub version: String,
    /// True when this is a musl-static fallback chosen because the host's
    /// distro had no dist-specific build (deb only).
    pub musl_fallback: bool,
}

/// Find the release asset for `package`/`format`/`arch`, preferring a build
/// for `dist` and (for deb) falling back to a musl-static asset. Returns
/// `None` when the release has no matching asset.
pub fn resolve_asset<'a>(
    release: &'a Release,
    package: &str,
    format: InstallFormat,
    arch: &str,
    dist: &str,
) -> Option<ResolvedAsset<'a>> {
    if let InstallFormat::Deb = format {
        if let Some(asset) = debs::find_asset(release, package, arch, dist) {
            let version = debs::control_version(&asset.name, package, arch)?;
            return Some(ResolvedAsset {
                asset,
                version,
                musl_fallback: false,
            });
        }
        let asset = debs::find_asset_musl(release, package, arch, dist)?;
        let version = debs::control_version(&asset.name, package, arch)?;
        return Some(ResolvedAsset {
            asset,
            version,
            musl_fallback: true,
        });
    }

    // rpm/arch: `{name}-...{version-release}.{arch}.ext`. Prefer a version
    // segment carrying the host's dist token; otherwise fall back to the
    // first arch match (covers rolling distros where the token is absent).
    let (prefix, suffix) = match format {
        InstallFormat::Rpm => (
            format!("{package}-"),
            format!(".{}.rpm", lx_lib::constants::to_rpm_arch(arch)),
        ),
        InstallFormat::Arch => (
            format!("{package}-"),
            format!("-{}.pkg.tar.zst", lx_lib::constants::to_pacman_arch(arch)),
        ),
        InstallFormat::Deb => unreachable!("deb handled above"),
    };

    let mut fallback: Option<ResolvedAsset<'a>> = None;
    for asset in &release.assets {
        let Some(mid) = asset.name.strip_prefix(prefix.as_str()) else {
            continue;
        };
        let Some(version) = mid.strip_suffix(suffix.as_str()) else {
            continue;
        };
        if version.is_empty() {
            continue;
        }
        if !dist.is_empty() && version.contains(dist) {
            return Some(ResolvedAsset {
                asset,
                version: version.to_string(),
                musl_fallback: false,
            });
        }
        if fallback.is_none() {
            fallback = Some(ResolvedAsset {
                asset,
                version: version.to_string(),
                musl_fallback: false,
            });
        }
    }
    fallback
}

/// The installed version of `package` per the host manager for `format`.
pub fn installed_version(package: &str, format: InstallFormat) -> Option<String> {
    match format {
        InstallFormat::Deb => debs::dpkg_installed_version(package),
        InstallFormat::Rpm => query_trim(Command::new("rpm").args([
            "-q",
            "--queryformat",
            "%{VERSION}-%{RELEASE}",
            package,
        ])),
        InstallFormat::Arch => {
            // `pacman -Q <pkg>` prints "<name> <version>".
            let out = query_trim(Command::new("pacman").args(["-Q", package]))?;
            out.split_whitespace().nth(1).map(str::to_string)
        }
    }
}

/// True if `candidate` is a newer version than `installed` under the host
/// format's own ordering rules.
pub fn is_newer(installed: &str, candidate: &str, format: InstallFormat) -> Result<bool> {
    match format {
        InstallFormat::Deb => debs::is_newer(installed, candidate),
        InstallFormat::Rpm | InstallFormat::Arch => {
            Ok(crate::versioncmp::rpm_evr_cmp(candidate, installed) == Ordering::Greater)
        }
    }
}

/// Install an already-downloaded artifact with the host's native manager.
/// Integrity is the caller's responsibility (sidecar or provenance pin), so
/// the redundant in-process re-hash in `install_prebuilt` is skipped.
pub fn install(path: &Path, filename: &str, format: InstallFormat, yes: bool) -> Result<()> {
    install_pkg::install_prebuilt(path, filename, format, "", yes, true, false)?;
    Ok(())
}

/// Remove (or purge) an installed package with the host's native manager.
pub fn remove(package: &str, purge: bool, yes: bool, format: InstallFormat) -> Result<()> {
    match format {
        InstallFormat::Deb => debs::remove_deb(package, purge, yes),
        InstallFormat::Rpm => {
            if !yes && !debs::confirm(&format!("Run `sudo rpm -e {package}`?"), false)? {
                println!("Aborted; '{package}' left installed.");
                return Ok(());
            }
            run_sudo(&["rpm", "-e", package])?;
            println!("✓ removed {package}");
            Ok(())
        }
        InstallFormat::Arch => {
            let flag = if purge { "-Rns" } else { "-R" };
            if !yes && !debs::confirm(&format!("Run `sudo pacman {flag} {package}`?"), false)? {
                println!("Aborted; '{package}' left installed.");
                return Ok(());
            }
            run_sudo(&["pacman", flag, "--noconfirm", package])?;
            println!("✓ removed {package}");
            Ok(())
        }
    }
}

fn run_sudo(args: &[&str]) -> Result<()> {
    let status = Command::new("sudo")
        .args(args)
        .status()
        .with_context(|| format!("failed to run `sudo {}`", args.join(" ")))?;
    if !status.success() {
        bail!("`sudo {}` failed", args.join(" "));
    }
    Ok(())
}

fn query_trim(cmd: &mut Command) -> Option<String> {
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lx_lib::github::Asset;

    fn asset(name: &str) -> Asset {
        Asset {
            name: name.to_string(),
            browser_download_url: format!("https://example.invalid/{name}"),
            size: Some(1),
            checksums: Default::default(),
        }
    }

    fn release(names: &[&str]) -> Release {
        Release {
            tag_name: "v1.0.0".into(),
            prerelease: false,
            draft: false,
            html_url: "https://example.invalid/release".into(),
            assets: names.iter().map(|n| asset(n)).collect(),
            published_at: None,
            body: None,
        }
    }

    #[test]
    fn repo_name_is_one_repo_per_package() {
        assert_eq!(repo_name("eza"), "eza-debian");
    }

    #[test]
    fn parse_format_accepts_aliases() {
        assert!(matches!(parse_format("dpkg").unwrap(), InstallFormat::Deb));
        assert!(matches!(parse_format("RPM").unwrap(), InstallFormat::Rpm));
        assert!(matches!(
            parse_format("pacman").unwrap(),
            InstallFormat::Arch
        ));
        assert!(parse_format("nonsense").is_err());
    }

    #[test]
    fn resolves_rpm_asset_and_version() {
        let rel = release(&["eza-0.20.0-1.fc40.x86_64.rpm"]);
        let r = resolve_asset(&rel, "eza", InstallFormat::Rpm, "amd64", "fc40").unwrap();
        assert_eq!(r.version, "0.20.0-1.fc40");
        assert!(!r.musl_fallback);
    }

    #[test]
    fn resolves_arch_asset_and_version() {
        let rel = release(&["eza-0.20.0-1.arch-x86_64.pkg.tar.zst"]);
        let r = resolve_asset(&rel, "eza", InstallFormat::Arch, "amd64", "arch").unwrap();
        assert_eq!(r.version, "0.20.0-1.arch");
    }

    #[test]
    fn rpm_falls_back_to_any_arch_match_when_dist_absent() {
        let rel = release(&["eza-0.20.0-1.fc40.x86_64.rpm"]);
        let r = resolve_asset(&rel, "eza", InstallFormat::Rpm, "amd64", "el9").unwrap();
        assert_eq!(r.version, "0.20.0-1.fc40");
    }

    #[test]
    fn deb_musl_fallback_is_flagged() {
        let rel = release(&["eza_0.20.0-1+musl_amd64.deb"]);
        let r = resolve_asset(&rel, "eza", InstallFormat::Deb, "amd64", "bookworm").unwrap();
        // The `+musl` token is part of the Debian Version (matching the
        // control file), not stripped.
        assert_eq!(r.version, "0.20.0-1+musl");
        assert!(r.musl_fallback);
    }

    #[test]
    fn wrong_arch_does_not_match() {
        let rel = release(&["eza-0.20.0-1.fc40.aarch64.rpm"]);
        assert!(resolve_asset(&rel, "eza", InstallFormat::Rpm, "amd64", "fc40").is_none());
    }

    #[test]
    fn rpm_dist_is_mapped_from_os_release() {
        assert_eq!(
            rpm_dist_from_os_release("ID=fedora\nVERSION_ID=40\n").as_deref(),
            Some("fedora")
        );
        assert_eq!(
            rpm_dist_from_os_release("ID=\"rocky\"\nVERSION_ID=\"9.4\"\n").as_deref(),
            Some("el9")
        );
        assert_eq!(
            rpm_dist_from_os_release("ID=almalinux\nVERSION_ID=8\n").as_deref(),
            Some("el8")
        );
        assert_eq!(
            rpm_dist_from_os_release("ID=opensuse-leap\nVERSION_ID=15.6\n").as_deref(),
            Some("opensuse")
        );
        assert_eq!(
            rpm_dist_from_os_release("ID=ubuntu\nVERSION_ID=24.04\n"),
            None
        );
    }
}
