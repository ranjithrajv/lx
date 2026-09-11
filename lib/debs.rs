//! Shared plumbing for `install`/`upgrade`/`remove`/`list`: everything that
//! talks to the `latest-debs` GitHub org or to dpkg on the host, factored
//! out so it isn't duplicated across those four subcommands.

use anyhow::{bail, Context, Result};
use std::path::Path;
use std::process::Command;

use lx_lib::github::{Asset, Release};

pub use lx_lib::constants::LATEST_DEBS_ORG;

/// The `latest-debs` org repo name for a package: `<package>-debian`. The
/// user-facing `package` argument to `install`/`upgrade` is always the bare
/// upstream name (matching `build.rs`'s `package_name`, and the name
/// embedded in asset filenames via `find_asset`/`control_version`); this is
/// the one place that maps it to the org's actual repo-naming convention.
pub fn repo_name(package: &str) -> String {
    format!("{package}-debian")
}

/// Match a release asset against `build.rs`'s naming scheme:
/// `{package}_{version}-{build}+{dist}_{arch}.deb`.
pub fn find_asset<'a>(
    release: &'a Release,
    package: &str,
    arch: &str,
    dist: &str,
) -> Option<&'a Asset> {
    let prefix = format!("{package}_");
    let suffix = format!("+{dist}_{arch}.deb");
    release
        .assets
        .iter()
        .find(|a| a.name.starts_with(&prefix) && a.name.ends_with(&suffix))
}

/// Recover the Debian `Version` field embedded in an asset filename
/// (`{package}_{version}_{arch}.deb`), which matches
/// `dpkg-query -W -f='${Version}'` for a package built by this tool.
pub fn control_version(asset_name: &str, package: &str, arch: &str) -> Option<String> {
    let prefix = format!("{package}_");
    let suffix = format!("_{arch}.deb");
    asset_name
        .strip_prefix(prefix.as_str())?
        .strip_suffix(suffix.as_str())
        .map(str::to_string)
}

/// Print the latest few releases as suggestions when the requested version
/// was not found, mirroring `build.rs`'s `suggest_versions`. `package` is
/// the bare upstream name; resolved to the org's actual repo via
/// `repo_name` for both the API call and the printed URL.
pub fn suggest_versions(client: &lx_lib::github::GitHubClient, package: &str, wanted: &str) {
    let repo = repo_name(package);
    eprintln!("Version '{wanted}' not found for {LATEST_DEBS_ORG}/{repo}.");
    match client.releases(LATEST_DEBS_ORG, &repo, 5) {
        Ok(metas) if !metas.is_empty() => {
            eprintln!("  Recent releases:");
            for m in metas {
                let date = m
                    .published_at
                    .as_deref()
                    .map(|d| format!(" ({d})"))
                    .unwrap_or_default();
                eprintln!("    - {}{}", m.tag, date);
            }
        }
        _ => {
            eprintln!(
                "  No recent releases could be listed; check \
                 https://github.com/{LATEST_DEBS_ORG}/{repo}/releases"
            );
        }
    }
}

pub fn detect_dpkg_arch() -> Result<String> {
    let out = Command::new("dpkg")
        .arg("--print-architecture")
        .output()
        .context("failed to run `dpkg --print-architecture`; pass --arch explicitly")?;
    if !out.status.success() {
        bail!("`dpkg --print-architecture` failed; pass --arch explicitly");
    }
    Ok(String::from_utf8(out.stdout)?.trim().to_string())
}

/// Read VERSION_CODENAME from /etc/os-release (e.g. "trixie").
pub fn detect_dist() -> Option<String> {
    let text = std::fs::read_to_string("/etc/os-release").ok()?;
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("VERSION_CODENAME=") {
            let v = v.trim().trim_matches('"');
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// The installed dpkg Version for `package`, or None if it isn't installed
/// (or dpkg-query / dpkg isn't available).
pub fn dpkg_installed_version(package: &str) -> Option<String> {
    let out = Command::new("dpkg-query")
        .args(["-W", "-f=${Version}", package])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let v = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if v.is_empty() {
        None
    } else {
        Some(v)
    }
}

/// Full dpkg Description (short + long) for an installed `package`, or
/// None if it isn't installed. Powers full-text search like `apt search`.
pub fn dpkg_description(package: &str) -> Option<String> {
    let out = Command::new("dpkg-query")
        .args(["-W", "-f=${Description}", package])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let v = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if v.is_empty() {
        None
    } else {
        Some(v)
    }
}

/// Direct runtime dependency names for an installed package, per dpkg
/// (`Depends:` field), with version constraints and alternatives stripped
/// down to the first alternative's bare name. Empty if the package isn't
/// installed, has no Depends, or dpkg-query isn't available.
pub fn dpkg_depends(package: &str) -> Vec<String> {
    let Ok(out) = Command::new("dpkg-query")
        .args(["-W", "-f=${Depends}", package])
        .output()
    else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    let Ok(field) = String::from_utf8(out.stdout) else {
        return Vec::new();
    };
    parse_depends_field(&field)
}

/// Pure parser for a dpkg `Depends:` field, factored out of
/// [`dpkg_depends`] so it's testable without a live dpkg database.
pub fn parse_depends_field(field: &str) -> Vec<String> {
    field
        .split(',')
        .filter_map(|dep| {
            // "foo (>= 1.2) | bar" -> "foo"
            let first_alt = dep.split('|').next().unwrap_or("").trim();
            let name = first_alt.split_whitespace().next()?;
            if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            }
        })
        .collect()
}

/// True if `candidate` is a newer Debian version than `installed`, per
/// dpkg's own version-comparison rules.
pub fn is_newer(installed: &str, candidate: &str) -> Result<bool> {
    let status = Command::new("dpkg")
        .args(["--compare-versions", candidate, "gt", installed])
        .status()
        .context("failed to run `dpkg --compare-versions`; is dpkg installed?")?;
    Ok(status.success())
}

pub fn download(
    client: &dyn lx_lib::checksum::RawGetter,
    asset: &Asset,
    dest: &Path,
) -> Result<()> {
    if asset.browser_download_url.is_empty() {
        bail!("asset '{}' has no download URL", asset.name);
    }
    let mut body = client
        .raw_get(&asset.browser_download_url)
        .context("asset download failed")?;
    let mut file = std::fs::File::create(dest)?;
    std::io::copy(&mut body, &mut file)?;
    Ok(())
}

/// Verify against a `.sha256`/`.sha256sum` sidecar file if one exists next
/// to the asset (probe-and-verify logic shared with `build.rs` via
/// `lx_lib::checksum::check_sidecar`). A checksum that doesn't match is
/// always a hard error, since this asset is about to be installed as root.
/// A missing sidecar also fails the install unless `allow_unverified` is
/// set (`--allow-unverified`) -- matching `build.rs`'s fail-closed default:
/// most real-world GitHub releases don't publish a sidecar, so silently
/// proceeding here would mean the *default* install path had zero
/// integrity verification with only a console line as evidence.
pub fn verify_sidecar_or_require_flag(
    client: &dyn lx_lib::checksum::RawGetter,
    asset: &Asset,
    path: &Path,
    allow_unverified: bool,
) -> Result<()> {
    use lx_lib::checksum::SidecarCheck;
    match lx_lib::checksum::check_sidecar(client, &asset.browser_download_url, &asset.name, path)? {
        SidecarCheck::Verified => {
            println!("    ✓ checksum verified");
            Ok(())
        }
        SidecarCheck::NotFound if allow_unverified => {
            eprintln!(
                "    ⚠ (no sidecar checksum found for '{}'; proceeding unverified per \
                 --allow-unverified)",
                asset.name
            );
            Ok(())
        }
        SidecarCheck::NotFound => Err(anyhow::anyhow!(
            "no checksum verification available for '{}': no sidecar checksum found. Pass \
             --allow-unverified to install anyway.",
            asset.name
        )),
    }
}

/// `sudo dpkg -i <path>`, falling back to `sudo apt-get install -f -y` when
/// dpkg reports missing dependencies. Prompts for confirmation unless `yes`.
pub fn install_deb(path: &Path, yes: bool) -> Result<()> {
    if !yes && !confirm(&format!("Install {} via `sudo dpkg -i`?", path.display()))? {
        println!("Aborted; package left at {}", path.display());
        return Ok(());
    }

    let status = Command::new("sudo")
        .args(["dpkg", "-i"])
        .arg(path)
        .status()
        .context("failed to run `sudo dpkg -i`")?;
    if !status.success() {
        eprintln!("  dpkg reported missing dependencies; attempting `sudo apt-get install -f -y`");
        let fix = Command::new("sudo")
            .args(["apt-get", "install", "-f", "-y"])
            .status()
            .context("failed to run `sudo apt-get install -f -y`")?;
        if !fix.success() {
            bail!("failed to install {}", path.display());
        }
    }
    println!("✓ installed {}", path.display());
    Ok(())
}

/// `sudo dpkg -r` (or `-P` to purge) a package. Prompts for confirmation
/// unless `yes`.
pub fn remove_deb(package: &str, purge: bool, yes: bool) -> Result<()> {
    let verb = if purge { "purge" } else { "remove" };
    if !yes
        && !confirm(&format!(
            "Run `sudo dpkg -{}` for '{package}'?",
            if purge { "P" } else { "r" }
        ))?
    {
        println!("Aborted; '{package}' left installed.");
        return Ok(());
    }

    let flag = if purge { "-P" } else { "-r" };
    let status = Command::new("sudo")
        .args(["dpkg", flag, package])
        .status()
        .with_context(|| format!("failed to run `sudo dpkg {flag} {package}`"))?;
    if !status.success() {
        bail!("failed to {verb} '{package}'");
    }
    println!("✓ {verb}d {package}");
    Ok(())
}

fn confirm(prompt: &str) -> Result<bool> {
    use std::io::Write;
    print!("{prompt} [y/N] ");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

pub fn human_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

/// Current time as RFC3339 UTC, mirroring `summary.rs`'s `rfc3339`.
pub fn now_rfc3339() -> String {
    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    jiff::Timestamp::from_second(epoch as i64)
        .map(|t| t.strftime("%Y-%m-%dT%H:%M:%S%z").to_string())
        .unwrap_or_default()
}
