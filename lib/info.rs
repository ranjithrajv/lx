// SPDX-License-Identifier: GPL-3.0-or-later

//! `lx info` — auto-detect and report the host OS and package system.
//!
//! Read-only and offline: it parses `/etc/os-release`, runs `uname`, and
//! probes `PATH` for the host's package manager. Nothing is downloaded,
//! installed, or built. Useful for understanding which format `lx get` /
//! `install` / `upgrade` will pick and which distro token release assets are
//! matched against on this machine.

use anyhow::Result;
use clap::Args;
use serde::Serialize;

use crate::builddeps::HostPm;

#[derive(Debug, Clone, Args)]
pub struct InfoArgs {
    /// Emit machine-readable JSON instead of the human report.
    #[arg(long)]
    pub json: bool,
}

/// The detected host, as reported by `lx info`.
#[derive(Debug, Clone, Default, Serialize)]
pub struct HostInfo {
    /// `PRETTY_NAME` from `/etc/os-release`.
    pub os: Option<String>,
    /// `ID` (lowercased) from `/etc/os-release`, e.g. `debian`, `fedora`.
    pub os_id: Option<String>,
    /// `VERSION_ID` from `/etc/os-release`.
    pub os_version: Option<String>,
    /// `VERSION_CODENAME` from `/etc/os-release` (e.g. `trixie`).
    pub codename: Option<String>,
    /// `ID_LIKE` from `/etc/os-release`, when present.
    pub id_like: Option<String>,
    /// Kernel release (`uname -r`).
    pub kernel: Option<String>,
    /// Raw machine (`uname -m`), e.g. `x86_64`.
    pub machine: Option<String>,
    /// Host architecture in Debian naming (`amd64`, `arm64`, …).
    pub arch: Option<String>,
    /// The host's package manager (`apt`, `dnf`, `pacman`, …), if any.
    pub package_manager: Option<String>,
    /// The native package format that manager consumes (`deb`, `rpm`,
    /// `arch`, `apk`, `xbps`).
    pub package_format: Option<String>,
    /// The distro token `lx` uses to pick the right prebuilt release asset
    /// (`trixie`, `fedora`, `arch`, …), matching `lx install`/`upgrade`.
    pub asset_dist: Option<String>,
}

/// Detect the host OS + package system. Every field is best-effort: a
/// missing `/etc/os-release` or `uname` leaves the relevant fields `None`
/// rather than failing the command.
pub fn detect() -> HostInfo {
    let release = std::fs::read_to_string("/etc/os-release")
        .ok()
        .map(|text| parse_os_release(&text))
        .unwrap_or_default();

    let mut info = HostInfo {
        os: release.pretty_name,
        os_id: release.id,
        os_version: release.version_id,
        codename: release.codename,
        id_like: release.id_like,
        kernel: uname("-r"),
        machine: uname("-m"),
        arch: crate::build::host_arch(),
        ..HostInfo::default()
    };

    let pm = HostPm::detect();
    info.package_manager = pm.map(|pm| pm.name().to_string());
    info.package_format = package_format(pm).map(str::to_string);
    info.asset_dist = asset_dist(info.package_format.as_deref());
    info
}

/// Detect the host distribution name for user-facing messages (e.g., "Debian",
/// "Arch", "Fedora"). Falls back to the `PRETTY_NAME` from `/etc/os-release`,
/// then the package manager name, then "unknown".
pub fn detect_host_dist() -> String {
    // Try /etc/os-release first
    if let Ok(text) = std::fs::read_to_string("/etc/os-release") {
        if let Some(pretty) = parse_os_release(&text).pretty_name {
            // Extract just the first word (e.g., "Debian" from "Debian GNU/Linux 13 (trixie)")
            return pretty
                .split_whitespace()
                .next()
                .unwrap_or(&pretty)
                .to_string();
        }
    }
    // Fall back to package manager name
    use crate::builddeps::HostPm;
    match HostPm::detect() {
        Some(HostPm::Apt) => "Debian".to_string(),
        Some(HostPm::Dnf) => "Fedora".to_string(),
        Some(HostPm::Zypper) => "openSUSE".to_string(),
        Some(HostPm::Pacman) => "Arch".to_string(),
        Some(HostPm::Apk) => "Alpine".to_string(),
        Some(HostPm::Xbps) => "Void".to_string(),
        None => "unknown".to_string(),
    }
}

pub fn run(args: InfoArgs) -> Result<()> {
    let host = detect();
    if args.json {
        println!("{}", serde_json::to_string_pretty(&host)?);
    } else {
        print_human(&host);
    }
    Ok(())
}

fn print_human(h: &HostInfo) {
    println!("Host information");
    let row = |label: &str, value: Option<&str>| {
        if let Some(value) = value {
            println!("  {label:<17}{value}");
        }
    };
    row("OS", h.os.as_deref());
    let os_id = match (&h.os_id, &h.os_version) {
        (Some(id), Some(version)) => Some(format!("{id} {version}")),
        (Some(id), None) => Some(id.clone()),
        (None, Some(version)) => Some(version.clone()),
        (None, None) => None,
    };
    row("OS ID", os_id.as_deref());
    row("Codename", h.codename.as_deref());
    row("ID_LIKE", h.id_like.as_deref());
    row("Kernel", h.kernel.as_deref());
    row("Machine", h.machine.as_deref());
    row("Architecture", h.arch.as_deref());
    row("Package manager", h.package_manager.as_deref());
    row("Package format", h.package_format.as_deref());
    row("Asset dist", h.asset_dist.as_deref());
}

/// The native package format the host package manager consumes. `None` only
/// when no supported manager was found.
fn package_format(pm: Option<HostPm>) -> Option<&'static str> {
    Some(match pm? {
        HostPm::Apt => "deb",
        HostPm::Dnf | HostPm::Zypper => "rpm",
        HostPm::Pacman => "arch",
        HostPm::Apk => "apk",
        // Void Linux's own format; `lx` has no packager for it.
        HostPm::Xbps => "xbps",
    })
}

/// The host's native format as a `lx` packager id (`deb`, `rpm`, `arch`, or
/// `apk`) — a value `--format` / `package_format:` accepts, and the smart
/// default other commands fall back to. `None` when no supported manager was
/// detected, or the host's manager has no matching packager (Void's xbps).
pub fn native_plugin_format() -> Option<&'static str> {
    native_plugin_format_for(HostPm::detect())
}

/// Pure form of [`native_plugin_format`] over an already-detected manager,
/// split out so the xbps exclusion is testable without a Void host.
fn native_plugin_format_for(pm: Option<HostPm>) -> Option<&'static str> {
    match package_format(pm)? {
        "xbps" => None,
        other => Some(other),
    }
}

/// The distro token `lx` matches prebuilt release assets against, derived
/// from the native format exactly the way the consumer layer does. `None`
/// for formats outside the deb/rpm/arch consumer scheme.
fn asset_dist(format: Option<&str>) -> Option<String> {
    match format? {
        "deb" => crate::debs::detect_dist(),
        "rpm" => crate::consumer::host_dist(crate::index::InstallFormat::Rpm),
        "arch" => Some("arch".to_string()),
        _ => None,
    }
}

/// Parsed subset of `/etc/os-release` used by [`detect`].
#[derive(Debug, Default, PartialEq, Eq)]
struct OsRelease {
    pretty_name: Option<String>,
    id: Option<String>,
    version_id: Option<String>,
    codename: Option<String>,
    id_like: Option<String>,
}

/// Parse `/etc/os-release` (shell-style `KEY=value`, values optionally
/// quoted). Unknown keys and comments are ignored.
fn parse_os_release(text: &str) -> OsRelease {
    let mut out = OsRelease::default();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = unquote(value.trim());
        if value.is_empty() {
            continue;
        }
        match key.trim() {
            "PRETTY_NAME" => out.pretty_name = Some(value),
            "ID" => out.id = Some(value.to_ascii_lowercase()),
            "VERSION_ID" => out.version_id = Some(value),
            "VERSION_CODENAME" => out.codename = Some(value),
            "ID_LIKE" => out.id_like = Some(value),
            _ => {}
        }
    }
    out
}

fn unquote(value: &str) -> String {
    let bytes = value.as_bytes();
    if bytes.len() >= 2
        && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''))
    {
        value[1..value.len() - 1].to_string()
    } else {
        value.to_string()
    }
}

/// One line of `uname` output, trimmed; `None` when the probe fails.
fn uname(flag: &str) -> Option<String> {
    let out = std::process::Command::new("uname")
        .arg(flag)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let value = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quoted_and_bare_os_release_values() {
        let r = parse_os_release(
            "# comment\n\
             PRETTY_NAME=\"Debian GNU/Linux 13 (trixie)\"\n\
             ID=debian\n\
             VERSION_ID=\"13\"\n\
             VERSION_CODENAME=trixie\n\
             ID_LIKE=\"debian\"\n\
             HOME_URL=\"https://www.debian.org/\"\n",
        );
        assert_eq!(
            r.pretty_name.as_deref(),
            Some("Debian GNU/Linux 13 (trixie)")
        );
        assert_eq!(r.id.as_deref(), Some("debian"));
        assert_eq!(r.version_id.as_deref(), Some("13"));
        assert_eq!(r.codename.as_deref(), Some("trixie"));
        assert_eq!(r.id_like.as_deref(), Some("debian"));
    }

    #[test]
    fn os_release_id_is_lowercased_and_missing_keys_are_none() {
        let r = parse_os_release("ID=Fedora\nVERSION_ID=40\n");
        assert_eq!(r.id.as_deref(), Some("fedora"));
        assert!(r.pretty_name.is_none());
        assert!(r.codename.is_none());
    }

    #[test]
    fn package_format_maps_each_manager() {
        assert_eq!(package_format(Some(HostPm::Apt)), Some("deb"));
        assert_eq!(package_format(Some(HostPm::Dnf)), Some("rpm"));
        assert_eq!(package_format(Some(HostPm::Zypper)), Some("rpm"));
        assert_eq!(package_format(Some(HostPm::Pacman)), Some("arch"));
        assert_eq!(package_format(Some(HostPm::Apk)), Some("apk"));
        assert_eq!(package_format(Some(HostPm::Xbps)), Some("xbps"));
        assert_eq!(package_format(None), None);
    }

    #[test]
    fn plugin_format_excludes_managers_without_a_packager() {
        // xbps is the native format label, but has no lx packager, so it is
        // not offered as a default --format.
        assert_eq!(native_plugin_format_for(Some(HostPm::Xbps)), None);
        assert_eq!(native_plugin_format_for(Some(HostPm::Apt)), Some("deb"));
        assert_eq!(native_plugin_format_for(Some(HostPm::Pacman)), Some("arch"));
        assert_eq!(native_plugin_format_for(None), None);
    }
}
