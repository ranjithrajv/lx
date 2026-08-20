//! Shared plumbing for `install`/`upgrade`/`remove`/`list`: everything that
//! talks to the `latest-debs` GitHub org or to dpkg on the host, factored
//! out so it isn't duplicated across those four subcommands.

use anyhow::{bail, Context, Result};
use std::io::Read;
use std::path::Path;
use std::process::Command;

use lpt_lib::github::{Asset, GitHubClient, Release};

/// GitHub org hosting pre-built `.deb`s, named
/// `{package}_{version}+{dist}_{arch}.deb` per `build.rs`'s naming.
pub const LATEST_DEBS_ORG: &str = "latest-debs";

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
/// was not found, mirroring `build.rs`'s `suggest_versions`.
pub fn suggest_versions(client: &GitHubClient, package: &str, wanted: &str) {
    eprintln!("Version '{wanted}' not found for {LATEST_DEBS_ORG}/{package}.");
    match client.releases(LATEST_DEBS_ORG, package, 5) {
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
                 https://github.com/{LATEST_DEBS_ORG}/{package}/releases"
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

/// True if `candidate` is a newer Debian version than `installed`, per
/// dpkg's own version-comparison rules.
pub fn is_newer(installed: &str, candidate: &str) -> Result<bool> {
    let status = Command::new("dpkg")
        .args(["--compare-versions", candidate, "gt", installed])
        .status()
        .context("failed to run `dpkg --compare-versions`; is dpkg installed?")?;
    Ok(status.success())
}

pub fn download(client: &GitHubClient, asset: &Asset, dest: &Path) -> Result<()> {
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
/// to the asset. A checksum that doesn't match is always a hard error,
/// since this asset is about to be installed as root. A missing sidecar
/// also fails the install unless `allow_unverified` is set (`--allow-
/// unverified`) -- matching `build.rs`'s fail-closed default: most
/// real-world GitHub releases don't publish a sidecar, so silently
/// proceeding here would mean the *default* install path had zero
/// integrity verification with only a console line as evidence.
pub fn verify_sidecar_or_require_flag(
    client: &GitHubClient,
    asset: &Asset,
    path: &Path,
    allow_unverified: bool,
) -> Result<()> {
    for suffix in [".sha256", ".sha256sum"] {
        let sidecar_url = format!("{}{}", asset.browser_download_url, suffix);
        let mut resp = match client.raw_get(&sidecar_url) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let mut text = String::new();
        resp.read_to_string(&mut text)?;
        let map = lpt_lib::checksum::parse_checksum_file(&text)?;
        let fname = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if let Some(expected) = map.get(fname) {
            lpt_lib::checksum::verify_sha256(path, expected)?;
            println!("    ✓ checksum verified");
            return Ok(());
        }
    }
    if allow_unverified {
        eprintln!(
            "    ⚠ (no sidecar checksum found for '{}'; proceeding unverified per \
             --allow-unverified)",
            asset.name
        );
        return Ok(());
    }
    Err(anyhow::anyhow!(
        "no checksum verification available for '{}': no sidecar checksum found. Pass \
         --allow-unverified to install anyway.",
        asset.name
    ))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(name: &str) -> Asset {
        Asset {
            name: name.to_string(),
            size: None,
            browser_download_url: format!("https://github.com/x/y/releases/download/v1/{name}"),
        }
    }

    fn release(assets: Vec<Asset>) -> Release {
        Release {
            tag_name: "v1.0.0".into(),
            name: None,
            prerelease: false,
            draft: false,
            html_url: "https://github.com/x/y/releases".into(),
            assets,
            published_at: None,
        }
    }

    #[test]
    fn finds_matching_asset() {
        let r = release(vec![
            asset("eza_0.24.0-1+trixie_amd64.deb"),
            asset("eza_0.24.0-1+trixie_arm64.deb"),
            asset("eza_0.24.0-1+bookworm_amd64.deb"),
        ]);
        let hit = find_asset(&r, "eza", "amd64", "trixie").unwrap();
        assert_eq!(hit.name, "eza_0.24.0-1+trixie_amd64.deb");
    }

    #[test]
    fn no_match_for_wrong_arch() {
        let r = release(vec![asset("eza_0.24.0-1+trixie_arm64.deb")]);
        assert!(find_asset(&r, "eza", "amd64", "trixie").is_none());
    }

    #[test]
    fn no_cross_package_match() {
        let r = release(vec![asset("eza-extras_0.24.0-1+trixie_amd64.deb")]);
        assert!(find_asset(&r, "eza", "amd64", "trixie").is_none());
    }

    #[test]
    fn extracts_control_version() {
        let v = control_version("eza_0.24.0-1+trixie_amd64.deb", "eza", "amd64").unwrap();
        assert_eq!(v, "0.24.0-1+trixie");
    }

    #[test]
    fn control_version_none_on_mismatch() {
        assert!(control_version("eza_0.24.0-1+trixie_amd64.deb", "eza", "arm64").is_none());
    }
}
