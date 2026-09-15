// SPDX-License-Identifier: GPL-3.0-or-later

//! `lx system-check` — auto-discover all packages on the host machine.
//!
//! Scans the host's package manager (dpkg/rpm/pacman) for installed
//! packages, then classifies each by its *nature*: native (installed via
//! the host PM), lx-managed (tracked in the lx manifest), snap, flatpak,
//! nix, or a `curl | sh` orphan binary. Prints a table (or JSON) with
//! each package's version and classification.

use anyhow::Result;
use clap::Args;
use colored::Colorize;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;

use crate::builddeps::HostPm;

#[derive(Debug, Clone, Args)]
pub struct SystemCheckArgs {
    /// Emit machine-readable JSON instead of the human table.
    #[arg(long)]
    pub json: bool,

    /// Only show packages matching this source type (native, lx, snap,
    /// flatpak, nix, sh).
    #[arg(long)]
    pub filter: Option<String>,

    /// Sort order: name (default), source, or version.
    #[arg(long, default_value = "name")]
    pub sort: String,

    /// Emit CSV instead of the human table (or JSON with --json).
    #[arg(long)]
    pub csv: bool,

    /// Show only non-native packages (snap, flatpak, nix, curl|sh) — hides
    /// the noise of 1500+ native packages.
    #[arg(long)]
    pub non_native: bool,

    /// Group packages by source type with sub-headers instead of a flat list.
    #[arg(long)]
    pub group: bool,

    /// Show only packages that need attention: curl|sh orphans, outdated
    /// lx-managed packages, and non-native packages with a native equivalent.
    #[arg(long)]
    pub alerts_only: bool,

    /// Skip version fetching for non-native packages (faster, but curl|sh
    /// orphans will show empty version).
    #[arg(long)]
    pub no_versions: bool,

    /// Show only the summary (counts + total sizes), no table.
    #[arg(long)]
    pub summary: bool,
}

/// The nature / origin of an installed package.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PackageSource {
    /// Installed via the host's native package manager (dpkg/rpm/pacman).
    Native,
    /// Tracked in the lx install manifest (a subset of Native).
    Lx,
    /// Installed via snap.
    Snap,
    /// Installed via flatpak.
    Flatpak,
    /// Installed via nix profile.
    Nix,
    /// Orphan binary from a `curl | sh` installer (not owned by any PM).
    Sh,
}

impl PackageSource {
    pub fn label(self) -> &'static str {
        match self {
            PackageSource::Native => "native",
            PackageSource::Lx => "lx",
            PackageSource::Snap => "snap",
            PackageSource::Flatpak => "flatpak",
            PackageSource::Nix => "nix",
            PackageSource::Sh => "curl|sh",
        }
    }

    /// ANSI color for this source type (respects NO_COLOR / non-TTY via
    /// the `colored` crate's automatic detection).
    pub fn color(self) -> colored::Color {
        match self {
            PackageSource::Native => colored::Color::Green,
            PackageSource::Lx => colored::Color::Blue,
            PackageSource::Snap => colored::Color::Yellow,
            PackageSource::Flatpak => colored::Color::Magenta,
            PackageSource::Nix => colored::Color::Cyan,
            PackageSource::Sh => colored::Color::Red,
        }
    }
}

/// One discovered package entry.
#[derive(Debug, Clone, Serialize)]
pub struct PackageEntry {
    pub name: String,
    pub version: String,
    pub source: PackageSource,
    /// Extra detail (e.g. snap revision, flatpak version, orphan path).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub detail: String,
    /// Version status for lx-managed packages: "up-to-date", "changed",
    /// or empty for non-lx packages.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub status: String,
    /// For non-native packages: the native package name if one exists
    /// (e.g. snap `firefox` → native `firefox`). Empty for native packages
    /// or when no native equivalent exists.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub native_available: String,
    /// Version of the native package available for non-native installations.
    /// Empty when no native equivalent exists or for native packages.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub native_version: String,
    /// Installed size (human-readable, e.g. "312 MB"). Empty for non-native
    /// packages or when size is unavailable.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub size: String,
    /// Migration command hint (e.g., "lx install firefox"). Empty for native
    /// packages or when no migration is possible.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub migrate: String,
}

pub fn run(args: SystemCheckArgs) -> Result<()> {
    let entries = discover_all(!args.no_versions);
    let filtered = filter_entries(entries, args.filter.as_deref());
    let non_native_filtered = if args.non_native {
        filter_non_native(filtered)
    } else {
        filtered
    };
    let alerts_filtered = if args.alerts_only {
        filter_alerts(non_native_filtered)
    } else {
        non_native_filtered
    };
    let sorted = sort_entries(alerts_filtered, Some(&args.sort));

    if args.json {
        println!("{}", serde_json::to_string_pretty(&sorted)?);
    } else if args.csv {
        print_csv(&sorted);
    } else if args.summary {
        print_summary(&sorted);
    } else if args.group {
        print_table_grouped(&sorted);
    } else {
        print_table(&sorted);
    }
    Ok(())
}

/// Discover every package from every source on the host.
pub fn discover_all(fetch_versions: bool) -> Vec<PackageEntry> {
    let mut entries = Vec::new();

    // Native packages from the host PM
    print_progress("Scanning native packages…");
    entries.extend(discover_native());
    print_progress(&format!("  native: {} found", entries.len()));

    // lx-managed packages (subset of native, but flagged + version status)
    let manifest = crate::manifest::Manifest::load().unwrap_or_default();
    for e in entries.iter_mut() {
        if let Some(gens) = manifest.generations(&e.name) {
            if let Some(current) = gens.last() {
                e.source = PackageSource::Lx;
                e.status = if current.version == e.version {
                    "up-to-date".to_string()
                } else {
                    format!("changed (manifest: {})", current.version)
                };
            }
        }
    }

    // Non-native sources
    print_progress("Scanning snap…");
    entries.extend(discover_snaps(fetch_versions));
    print_progress("Scanning flatpak…");
    entries.extend(discover_flatpaks());
    print_progress("Scanning nix…");
    entries.extend(discover_nix());
    print_progress("Scanning curl|sh orphans…");
    entries.extend(discover_sh_orphans(fetch_versions));

    // For non-native packages, check if a native package with the same name exists
    let native_versions: BTreeMap<String, String> = entries
        .iter()
        .filter(|e| e.source == PackageSource::Native || e.source == PackageSource::Lx)
        .map(|e| (e.name.clone(), e.version.clone()))
        .collect();
    let mut migrate_count = 0;
    for e in entries.iter_mut() {
        if e.source != PackageSource::Native && e.source != PackageSource::Lx {
            if let Some(version) = native_versions.get(&e.name) {
                e.native_available = e.name.clone();
                e.native_version = version.clone();
                e.migrate = format!("lx install {}", e.name);
                migrate_count += 1;
            }
        }
    }

    entries.sort_by(|a, b| a.name.cmp(&b.name));

    if migrate_count > 0 {
        print_progress(&format!(
            "  {} non-native package(s) can be migrated",
            migrate_count
        ));
    }
    entries
}

/// Print a progress message to stderr (so it doesn't interfere with stdout).
/// Suppressed when stderr is not a terminal, matching the codebase's
/// TTY-aware error styling — keeps logs and piped output clean.
fn print_progress(msg: &str) {
    use std::io::IsTerminal;
    if std::io::stderr().is_terminal() {
        eprintln!("{msg}");
    }
}

/// List all native packages installed via the host's package manager.
fn discover_native() -> Vec<PackageEntry> {
    let pm = match HostPm::detect() {
        Some(pm) => pm,
        None => return Vec::new(),
    };

    match pm {
        HostPm::Apt => list_dpkg_packages(),
        HostPm::Dnf | HostPm::Zypper => list_rpm_packages(),
        HostPm::Pacman => list_pacman_packages(),
        HostPm::Apk | HostPm::Xbps => Vec::new(),
    }
}

/// `dpkg-query -W` — all installed Debian packages (name, version, size KB).
fn list_dpkg_packages() -> Vec<PackageEntry> {
    let out = match Command::new("dpkg-query")
        .args(["-W", "-f=${Package}\t${Version}\t${Installed-Size}\n"])
        .output()
    {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };
    if !out.status.success() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let name = parts.next()?.trim();
            let version = parts.next()?.trim();
            let size_kb = parts.next().unwrap_or("0").trim();
            if name.is_empty() {
                return None;
            }
            let size = if size_kb == "0" || size_kb.is_empty() {
                String::new()
            } else {
                format_size_kb(size_kb)
            };
            Some(PackageEntry {
                name: name.to_string(),
                version: version.trim().to_string(),
                source: PackageSource::Native,
                detail: String::new(),
                status: String::new(),
                native_available: String::new(),
                native_version: String::new(),
                size,
                migrate: String::new(),
            })
        })
        .collect()
}

/// `rpm -qa` — all installed RPM packages (name, version, size bytes).
fn list_rpm_packages() -> Vec<PackageEntry> {
    let out = match Command::new("rpm")
        .args([
            "-qa",
            "--queryformat",
            "%{NAME}\t%{VERSION}-%{RELEASE}\t%{SIZE}\n",
        ])
        .output()
    {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };
    if !out.status.success() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let name = parts.next()?.trim();
            let version = parts.next()?.trim();
            let size_bytes = parts.next().unwrap_or("0").trim();
            if name.is_empty() {
                return None;
            }
            let size = if size_bytes == "0" || size_bytes.is_empty() {
                String::new()
            } else {
                format_size_bytes(size_bytes)
            };
            Some(PackageEntry {
                name: name.to_string(),
                version: version.trim().to_string(),
                source: PackageSource::Native,
                detail: String::new(),
                status: String::new(),
                native_available: String::new(),
                native_version: String::new(),
                size,
                migrate: String::new(),
            })
        })
        .collect()
}

/// `pacman -Q` — all installed Arch packages. Size is parsed from
/// `pacman -Qi` output (Installed Size field) in a single batch.
fn list_pacman_packages() -> Vec<PackageEntry> {
    let out = match Command::new("pacman").args(["-Q"]).output() {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };
    if !out.status.success() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let entries: Vec<PackageEntry> = text
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(2, ' ');
            let name = parts.next()?.trim();
            let version = parts.next()?.trim();
            if name.is_empty() {
                return None;
            }
            Some(PackageEntry {
                name: name.to_string(),
                version: version.to_string(),
                source: PackageSource::Native,
                detail: String::new(),
                status: String::new(),
                native_available: String::new(),
                native_version: String::new(),
                size: String::new(),
                migrate: String::new(),
            })
        })
        .collect();

    // Batch-fetch sizes from pacman -Qi (Installed Size field)
    if let Some(size_map) = pacman_sizes() {
        return entries
            .into_iter()
            .map(|mut e| {
                if let Some(s) = size_map.get(&e.name) {
                    e.size = s.clone();
                }
                e
            })
            .collect();
    }
    entries
}

/// Parse `pacman -Qi` output to get installed sizes for all packages.
/// Returns a map of package name -> human-readable size string.
fn pacman_sizes() -> Option<BTreeMap<String, String>> {
    let out = Command::new("pacman").args(["-Qi"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut map = BTreeMap::new();
    let mut current_name = String::new();
    for line in text.lines() {
        if line.starts_with("Name") {
            current_name = line.split_once(':')?.1.trim().to_string();
        } else if line.starts_with("Installed Size") && !current_name.is_empty() {
            let size = line.split_once(':')?.1.trim().to_string();
            if !size.is_empty() && size != "None" {
                map.insert(current_name.clone(), size);
            }
        }
    }
    Some(map)
}

/// Discover snap packages (excluding runtimes/bases). Size is fetched via
/// `du -sh /snap/<name>` for each package.
fn discover_snaps(_fetch_versions: bool) -> Vec<PackageEntry> {
    let out = match Command::new("snap").args(["list"]).output() {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };
    if !out.status.success() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&out.stdout);
    crate::go_native::parse_snap_list(&text)
        .into_iter()
        .map(|(name, version)| {
            let snap_dir = std::path::PathBuf::from("/snap").join(&name);
            let size = du_size(&snap_dir);
            PackageEntry {
                name,
                version,
                source: PackageSource::Snap,
                detail: String::new(),
                status: String::new(),
                native_available: String::new(),
                native_version: String::new(),
                size,
                migrate: String::new(),
            }
        })
        .collect()
}

/// Discover flatpak apps. Size is fetched via `du -sh` on the app install
/// directory (tries user location first, then system).
fn discover_flatpaks() -> Vec<PackageEntry> {
    let out = match Command::new("flatpak")
        .args(["list", "--app", "--columns=application,version"])
        .output()
    {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };
    if !out.status.success() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&out.stdout);
    crate::go_native::parse_flatpak_list(&text)
        .into_iter()
        .map(|(id, version)| {
            let home = std::env::var("HOME").unwrap_or_default();
            let user_dir = std::path::PathBuf::from(&home)
                .join(".local/share/flatpak/app")
                .join(&id);
            let system_dir = std::path::PathBuf::from("/var/lib/flatpak/app").join(&id);
            let size = {
                let s = du_size(&user_dir);
                if s.is_empty() {
                    du_size(&system_dir)
                } else {
                    s
                }
            };
            PackageEntry {
                name: id,
                version,
                source: PackageSource::Flatpak,
                detail: String::new(),
                status: String::new(),
                native_available: String::new(),
                native_version: String::new(),
                size,
                migrate: String::new(),
            }
        })
        .collect()
}

/// Discover nix profile packages. Size is fetched via `du -sh` on the
/// store path (`nix profile list --json` gives the store path).
fn discover_nix() -> Vec<PackageEntry> {
    // Try JSON output first (nix 2.17+): gives store paths for sizing
    let store_paths: Option<BTreeMap<String, String>> = Command::new("nix")
        .args(["profile", "list", "--json"])
        .output()
        .ok()
        .and_then(|o| {
            if !o.status.success() {
                return None;
            }
            let text = String::from_utf8_lossy(&o.stdout);
            serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .map(|v| {
                    let mut map = BTreeMap::new();
                    if let Some(installed) = v.get("elements").and_then(|e| e.as_array()) {
                        for elem in installed {
                            let name = elem
                                .get("attrPath")
                                .and_then(|s| s.as_str())
                                .and_then(|p| p.rsplit('.').next())
                                .unwrap_or("")
                                .to_string();
                            let store_path = elem
                                .get("storePaths")
                                .and_then(|arr| arr.as_array())
                                .and_then(|arr| arr.first())
                                .and_then(|s| s.as_str())
                                .unwrap_or("")
                                .to_string();
                            if !name.is_empty() && !store_path.is_empty() {
                                map.insert(name, store_path);
                            }
                        }
                    }
                    map
                })
        });

    let out = match Command::new("nix").args(["profile", "list"]).output() {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };
    if !out.status.success() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&out.stdout);
    crate::go_native::parse_nix_profile(&text)
        .into_iter()
        .map(|(attr, version)| {
            let size = store_paths
                .as_ref()
                .and_then(|m| m.get(&attr))
                .map(|path| du_size(std::path::Path::new(path)))
                .unwrap_or_default();
            PackageEntry {
                name: attr,
                version,
                source: PackageSource::Nix,
                detail: String::new(),
                status: String::new(),
                native_available: String::new(),
                native_version: String::new(),
                size,
                migrate: String::new(),
            }
        })
        .collect()
}

/// Discover `curl | sh` orphan binaries (not owned by any native PM).
/// When `fetch_versions` is true, tries to get version via `--version`.
fn discover_sh_orphans(fetch_versions: bool) -> Vec<PackageEntry> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();

    let mut prefixes = vec![
        std::path::PathBuf::from("/usr/local/bin"),
        std::path::PathBuf::from("/opt"),
    ];
    if let Ok(home) = std::env::var("HOME") {
        prefixes.push(std::path::PathBuf::from(home).join(".local/bin"));
    }

    for prefix in prefixes {
        for bin in list_executables(&prefix) {
            let name = bin
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            if name.is_empty() || !seen.insert(name.clone()) {
                continue;
            }
            if native_owns(&bin) {
                continue;
            }
            let size = du_size(&bin);
            let version = if fetch_versions {
                fetch_binary_version(&bin)
            } else {
                String::new()
            };
            out.push(PackageEntry {
                name,
                version,
                source: PackageSource::Sh,
                detail: bin.display().to_string(),
                status: String::new(),
                native_available: String::new(),
                native_version: String::new(),
                size,
                migrate: String::new(),
            });
        }
    }
    out
}

/// List executable files under `dir` (one level for `bin` subdirs).
fn list_executables(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_symlink() || path.is_file() {
            if is_executable(&path) {
                out.push(path);
            }
        } else if path.is_dir() && dir.ends_with("bin") {
            // one level (e.g. /opt/<tool>/bin)
            for sub in std::fs::read_dir(&path).into_iter().flatten().flatten() {
                if sub.path().is_file() && is_executable(&sub.path()) {
                    out.push(sub.path());
                }
            }
        }
    }
    out
}

#[cfg(unix)]
fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(_path: &std::path::Path) -> bool {
    false
}

/// Try to fetch the version of a binary by running it with --version, -v,
/// or --ver. Returns a cleaned version string, or empty on failure.
/// Times out after 1 second to avoid hanging on binaries that don't exit.
///
/// Filters out non-version lines (paths, usage info, etc.) by looking for
/// lines that contain a version-like pattern (digits, dots, "v" prefix).
fn fetch_binary_version(binary_path: &std::path::Path) -> String {
    for flag in &["--version", "-v", "--ver", "-V"] {
        let mut child = match Command::new(binary_path)
            .arg(flag)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(_) => continue,
        };
        // Wait with timeout to avoid hanging
        let status = match wait_timeout::ChildExt::wait_timeout(
            &mut child,
            std::time::Duration::from_secs(1),
        ) {
            Ok(Some(s)) => s,
            Ok(None) => {
                // Timed out — kill the child and move on
                let _ = child.kill();
                continue;
            }
            Err(_) => continue,
        };
        // Reap output after the child has exited
        let output = match child.wait_with_output() {
            Ok(o) => o,
            Err(_) => continue,
        };
        if !status.success() && output.stdout.is_empty() {
            continue;
        }
        // Combine stdout and stderr (some tools print version to stderr)
        let mut text = String::from_utf8_lossy(&output.stdout).to_string();
        if text.trim().is_empty() {
            text = String::from_utf8_lossy(&output.stderr).to_string();
        }
        // Find the first line that looks like a version string
        if let Some(line) = text.lines().map(str::trim).find(|l| is_version_line(l)) {
            // Clean up common prefixes and truncate
            let cleaned = clean_version_line(line);
            return cleaned;
        }
    }
    String::new()
}

/// Heuristic: does this line look like it contains a version string?
/// Filters out paths, usage info, and other non-version output.
fn is_version_line(line: &str) -> bool {
    if line.is_empty() {
        return false;
    }
    // Skip lines that are clearly paths or usage info
    if line.starts_with('/')
        || line.starts_with("~/")
        || line.starts_with("./")
        || line.starts_with("mise ")
        || line.starts_with("Usage:")
        || line.starts_with("usage:")
        || line.starts_with("error:")
        || line.starts_with("Error:")
    {
        return false;
    }
    // Must contain at least one digit (version numbers have digits)
    if !line.chars().any(|c| c.is_ascii_digit()) {
        return false;
    }
    // Must contain a dot or "v" prefix pattern (e.g., "v1.2.3" or "1.2.3")
    // or start with a digit
    line.contains('.')
        || line.starts_with('v')
        || line.starts_with('V')
        || line
            .chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false)
}

/// Clean up a version line: extract just the version number, stripping
/// tool names and other irrelevant text.
fn clean_version_line(line: &str) -> String {
    let line = line.trim();

    // Try to find a version pattern: digits separated by dots (e.g., 1.2.3)
    // optionally prefixed with v/V (e.g., v1.2.3)
    // This regex-like search extracts just the version number part
    if let Some(version) = extract_version_number(line) {
        return version;
    }

    // Fallback: remove common prefixes and return the rest
    let line = line
        .strip_prefix("version ")
        .or_else(|| line.strip_prefix("Version "))
        .or_else(|| line.strip_prefix("v"))
        .or_else(|| line.strip_prefix("V"))
        .unwrap_or(line);
    let line = line.trim_start();

    // Extract just the version part (up to first comma, paren, or tab)
    let version_part = line
        .split(&[',', '(', ')', '\t'][..])
        .next()
        .unwrap_or(line)
        .trim();

    // Truncate at char boundary
    if version_part.chars().count() > 20 {
        format!("{}…", version_part.chars().take(20).collect::<String>())
    } else {
        version_part.to_string()
    }
}

/// Extract a version number from a string by finding the first occurrence
/// of a pattern like "1.2.3" or "v1.2.3". Returns just the version digits.
fn extract_version_number(text: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();

    for i in 0..len {
        // Look for a digit or 'v' followed by a digit
        let start = if chars[i].is_ascii_digit() {
            Some(i)
        } else if (chars[i] == 'v' || chars[i] == 'V')
            && i + 1 < len
            && chars[i + 1].is_ascii_digit()
        {
            Some(i + 1) // skip the 'v' prefix
        } else {
            None
        };

        if let Some(s) = start {
            // Collect digits and dots
            let mut end = s;
            let mut has_dot = false;
            while end < len
                && (chars[end].is_ascii_digit() || chars[end] == '.' || chars[end] == '-')
            {
                if chars[end] == '.' {
                    has_dot = true;
                }
                // Stop at double dots or dot followed by non-digit
                if chars[end] == '.' && end + 1 < len && !chars[end + 1].is_ascii_digit() {
                    break;
                }
                end += 1;
            }
            // Must have at least one dot (e.g., "1.2") and at least 3 chars
            if has_dot && end > s + 2 {
                let version: String = chars[s..end].iter().collect();
                // Validate: should look like x.y.z or x.y
                let parts: Vec<&str> = version.split('.').collect();
                if parts.len() >= 2
                    && parts
                        .iter()
                        .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
                {
                    return Some(version);
                }
            }
        }
    }
    None
}

/// Get human-readable size of a file or directory via `du -sh`.
/// Returns empty string on failure or if path doesn't exist.
fn du_size(path: &std::path::Path) -> String {
    if !path.exists() {
        return String::new();
    }
    let out = match Command::new("du")
        .args(["-sh", &path.to_string_lossy()])
        .output()
    {
        Ok(o) => o,
        Err(_) => return String::new(),
    };
    if !out.status.success() {
        return String::new();
    }
    let text = String::from_utf8_lossy(&out.stdout);
    // `du -sh` output: "312M\t/path/to/file"
    text.split('\t')
        .next()
        .map(str::trim)
        .unwrap_or("")
        .to_string()
}

/// True when dpkg/rpm/pacman claims the path.
fn native_owns(path: &std::path::Path) -> bool {
    let s = path.display().to_string();
    for (bin, args) in [
        ("dpkg", vec!["-S", &s]),
        ("rpm", vec!["-qf", &s]),
        ("pacman", vec!["-Qo", &s]),
    ] {
        if Command::new(bin)
            .args(&args)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return true;
        }
    }
    false
}

/// Format a size in KB (dpkg's Installed-Size) to human-readable string.
fn format_size_kb(size_kb: &str) -> String {
    let kb: u64 = size_kb.parse().unwrap_or(0);
    if kb >= 1024 * 1024 {
        format!("{:.1} GB", kb as f64 / (1024.0 * 1024.0))
    } else if kb >= 1024 {
        format!("{:.1} MB", kb as f64 / 1024.0)
    } else if kb > 0 {
        format!("{:.0} KB", kb)
    } else {
        String::new()
    }
}

/// Format a size in bytes (rpm's SIZE) to human-readable string.
fn format_size_bytes(size_bytes: &str) -> String {
    let bytes: u64 = size_bytes.parse().unwrap_or(0);
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        String::new()
    }
}

/// Filter entries by source type label (case-insensitive substring match).
fn filter_entries(entries: Vec<PackageEntry>, filter: Option<&str>) -> Vec<PackageEntry> {
    let Some(f) = filter else {
        return entries;
    };
    let f = f.trim().to_ascii_lowercase();
    if f.is_empty() {
        return entries;
    }
    entries
        .into_iter()
        .filter(|e| e.source.label().contains(&f))
        .collect()
}

/// Filter to show only non-native packages (snap, flatpak, nix, curl|sh).
fn filter_non_native(entries: Vec<PackageEntry>) -> Vec<PackageEntry> {
    entries
        .into_iter()
        .filter(|e| e.source != PackageSource::Native && e.source != PackageSource::Lx)
        .collect()
}

/// Filter to show only packages that need attention:
/// - curl|sh orphans
/// - non-native packages with a native equivalent available
/// - lx-managed packages with changed version
fn filter_alerts(entries: Vec<PackageEntry>) -> Vec<PackageEntry> {
    entries
        .into_iter()
        .filter(|e| {
            e.source == PackageSource::Sh
                || !e.native_available.is_empty()
                || (!e.status.is_empty() && !e.status.starts_with("up-to-date"))
        })
        .collect()
}

/// Sort entries by the given field: "name" (default), "source", or "version".
fn sort_entries(mut entries: Vec<PackageEntry>, sort: Option<&str>) -> Vec<PackageEntry> {
    match sort.unwrap_or("name") {
        "source" => entries.sort_by(|a, b| {
            a.source
                .label()
                .cmp(b.source.label())
                .then_with(|| a.name.cmp(&b.name))
        }),
        "version" => {
            entries.sort_by(|a, b| a.version.cmp(&b.version).then_with(|| a.name.cmp(&b.name)))
        }
        _ => entries.sort_by(|a, b| a.name.cmp(&b.name)),
    }
    entries
}

/// Print a one-line summary of package counts per source type,
/// plus the number of non-native packages with a native equivalent.
fn print_summary(entries: &[PackageEntry]) {
    let mut counts: BTreeMap<PackageSource, usize> = BTreeMap::new();
    let mut total_sizes: BTreeMap<PackageSource, u64> = BTreeMap::new();
    for e in entries {
        *counts.entry(e.source).or_insert(0) += 1;
        total_sizes
            .entry(e.source)
            .and_modify(|s| *s += parse_size_bytes(&e.size))
            .or_insert_with(|| parse_size_bytes(&e.size));
    }
    let parts: Vec<String> = counts
        .iter()
        .map(|(src, n): (&PackageSource, &usize)| {
            let size_str = format_bytes_human(*total_sizes.get(src).unwrap_or(&0));
            format!(
                "{} {} ({})",
                n,
                src.label().color(src.color()),
                size_str.dimmed()
            )
        })
        .collect();
    println!("{}", parts.join(", "));

    // Show migration opportunities: non-native packages with a native equivalent
    let migratable = entries
        .iter()
        .filter(|e| !e.native_available.is_empty())
        .count();
    if migratable > 0 {
        let msg = format!(
            "{} non-native package(s) have a native equivalent available",
            migratable
        );
        println!("{}", msg.yellow());
    }
}

/// Parse a human-readable size string (e.g., "329M", "132.34 KiB", "4.0K") to bytes.
fn parse_size_bytes(size_str: &str) -> u64 {
    let size_str = size_str.trim();
    if size_str.is_empty() {
        return 0;
    }
    // Extract the numeric part and the unit
    let (num_part, unit_part) = size_str
        .find(|c: char| !c.is_ascii_digit() && c != '.' && c != '-')
        .map(|i| size_str.split_at(i))
        .unwrap_or((size_str, ""));

    let num: f64 = num_part.trim().parse().unwrap_or(0.0);
    let unit = unit_part.trim().to_uppercase();

    let multiplier = match unit.as_str() {
        "B" | "" => 1u64,
        "K" | "KB" | "KIB" => 1024,
        "M" | "MB" | "MIB" => 1024 * 1024,
        "G" | "GB" | "GIB" => 1024 * 1024 * 1024,
        "T" | "TB" | "TIB" => 1024 * 1024 * 1024 * 1024,
        _ => 1,
    };
    (num * multiplier as f64) as u64
}

/// Format a byte count to a human-readable size string.
fn format_bytes_human(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes > 0 {
        format!("{bytes} B")
    } else {
        "—".to_string()
    }
}

/// Print the human-readable table with colors, version status, native
/// availability, and size.
fn print_table(entries: &[PackageEntry]) {
    if entries.is_empty() {
        println!("no packages discovered");
        return;
    }

    // Summary header
    print_summary(entries);

    // Column widths
    let name_w = entries
        .iter()
        .map(|e| e.name.len())
        .max()
        .unwrap_or(10)
        .max(10);
    let ver_w = entries
        .iter()
        .map(|e| e.version.len())
        .max()
        .unwrap_or(7)
        .max(7);
    let src_w = entries
        .iter()
        .map(|e| e.source.label().len())
        .max()
        .unwrap_or(6)
        .max(6);
    let status_w = entries
        .iter()
        .map(|e| e.status.len())
        .max()
        .unwrap_or(0)
        .max(0);
    let native_w = entries
        .iter()
        .map(|e| e.native_available.len())
        .max()
        .unwrap_or(0)
        .max(0);
    let native_ver_w = entries
        .iter()
        .map(|e| e.native_version.len())
        .max()
        .unwrap_or(0)
        .max(0);
    let size_w = entries
        .iter()
        .map(|e| e.size.len())
        .max()
        .unwrap_or(0)
        .max(0);
    let migrate_w = entries
        .iter()
        .map(|e| e.migrate.len())
        .max()
        .unwrap_or(0)
        .max(0);

    let has_status = status_w > 0;
    let has_native = native_w > 0;
    let has_native_ver = native_ver_w > 0;
    let has_size = size_w > 0;
    let has_migrate = migrate_w > 0;

    // Build header
    let mut header = format!(
        "{:<name_w$} {:<ver_w$} {:<src_w$}",
        "PACKAGE".bold(),
        "VERSION".bold(),
        "SOURCE".bold(),
        name_w = name_w,
        ver_w = ver_w,
        src_w = src_w,
    );
    if has_status {
        header.push_str(&format!(
            " {:<status_w$}",
            "STATUS".bold(),
            status_w = status_w,
        ));
    }
    if has_native {
        header.push_str(&format!(
            " {:<native_w$}",
            "NATIVE".bold(),
            native_w = native_w,
        ));
    }
    if has_native_ver {
        let label = "NATIVE VER";
        header.push_str(&format!(
            " {:<native_ver_w$}",
            label.bold(),
            native_ver_w = native_ver_w,
        ));
    }
    if has_size {
        header.push_str(&format!(" {:>size_w$}", "SIZE".bold(), size_w = size_w,));
    }
    if has_migrate {
        header.push_str(&format!(
            " {:<migrate_w$}",
            "MIGRATE".bold(),
            migrate_w = migrate_w,
        ));
    }
    println!("{}", header);

    // Rows
    for e in entries {
        let src_label = e.source.label().color(e.source.color());
        let mut row = format!(
            "{:<name_w$} {:<ver_w$} {:<src_w$}",
            e.name,
            e.version,
            src_label,
            name_w = name_w,
            ver_w = ver_w,
            src_w = src_w,
        );
        if has_status {
            row.push_str(&format!(
                " {:<status_w$}",
                if e.status.is_empty() { "" } else { &e.status },
                status_w = status_w,
            ));
        }
        if has_native {
            row.push_str(&format!(
                " {:<native_w$}",
                if e.native_available.is_empty() {
                    ""
                } else {
                    &e.native_available
                },
                native_w = native_w,
            ));
        }
        if has_native_ver {
            row.push_str(&format!(
                " {:<native_ver_w$}",
                if e.native_version.is_empty() {
                    ""
                } else {
                    &e.native_version
                },
                native_ver_w = native_ver_w,
            ));
        }
        if has_size {
            row.push_str(&format!(
                " {:>size_w$}",
                if e.size.is_empty() { "" } else { &e.size },
                size_w = size_w,
            ));
        }
        if has_migrate {
            row.push_str(&format!(
                " {:<migrate_w$}",
                if e.migrate.is_empty() { "" } else { &e.migrate },
                migrate_w = migrate_w,
            ));
        }
        println!("{}", row);
    }
    println!("\n{} package(s) total", entries.len());
}

/// Print the table grouped by source type with sub-headers.
fn print_table_grouped(entries: &[PackageEntry]) {
    if entries.is_empty() {
        println!("no packages discovered");
        return;
    }

    // Summary header
    print_summary(entries);

    // Group entries by source type
    let mut groups: BTreeMap<PackageSource, Vec<&PackageEntry>> = BTreeMap::new();
    for e in entries {
        groups.entry(e.source).or_default().push(e);
    }

    for (src, group_entries) in &groups {
        // Group header
        println!(
            "── {} ({}) ──",
            src.label().color(src.color()).bold(),
            group_entries.len()
        );

        // Column widths for this group
        let name_w = group_entries
            .iter()
            .map(|e| e.name.len())
            .max()
            .unwrap_or(10)
            .max(10);
        let ver_w = group_entries
            .iter()
            .map(|e| e.version.len())
            .max()
            .unwrap_or(7)
            .max(7);
        let status_w = group_entries
            .iter()
            .map(|e| e.status.len())
            .max()
            .unwrap_or(0)
            .max(0);
        let native_w = group_entries
            .iter()
            .map(|e| e.native_available.len())
            .max()
            .unwrap_or(0)
            .max(0);
        let native_ver_w = group_entries
            .iter()
            .map(|e| e.native_version.len())
            .max()
            .unwrap_or(0)
            .max(0);
        let size_w = group_entries
            .iter()
            .map(|e| e.size.len())
            .max()
            .unwrap_or(0)
            .max(0);
        let migrate_w = group_entries
            .iter()
            .map(|e| e.migrate.len())
            .max()
            .unwrap_or(0)
            .max(0);

        let has_status = status_w > 0;
        let has_native = native_w > 0;
        let has_native_ver = native_ver_w > 0;
        let has_size = size_w > 0;
        let has_migrate = migrate_w > 0;

        // Column headers
        let mut header = format!(
            "  {:<name_w$} {:<ver_w$}",
            "PACKAGE".bold(),
            "VERSION".bold(),
            name_w = name_w,
            ver_w = ver_w,
        );
        if has_status {
            header.push_str(&format!(
                " {:<status_w$}",
                "STATUS".bold(),
                status_w = status_w
            ));
        }
        if has_native {
            header.push_str(&format!(
                " {:<native_w$}",
                "NATIVE".bold(),
                native_w = native_w
            ));
        }
        if has_native_ver {
            header.push_str(&format!(
                " {:<native_ver_w$}",
                "NATIVE VER".bold(),
                native_ver_w = native_ver_w
            ));
        }
        if has_size {
            header.push_str(&format!(" {:>size_w$}", "SIZE".bold(), size_w = size_w));
        }
        if has_migrate {
            header.push_str(&format!(
                " {:<migrate_w$}",
                "MIGRATE".bold(),
                migrate_w = migrate_w
            ));
        }
        println!("{}", header);

        // Rows
        for e in group_entries {
            let mut row = format!(
                "  {:<name_w$} {:<ver_w$}",
                e.name,
                e.version,
                name_w = name_w,
                ver_w = ver_w,
            );
            if has_status {
                row.push_str(&format!(
                    " {:<status_w$}",
                    if e.status.is_empty() { "" } else { &e.status },
                    status_w = status_w,
                ));
            }
            if has_native {
                row.push_str(&format!(
                    " {:<native_w$}",
                    if e.native_available.is_empty() {
                        ""
                    } else {
                        &e.native_available
                    },
                    native_w = native_w,
                ));
            }
            if has_native_ver {
                row.push_str(&format!(
                    " {:<native_ver_w$}",
                    if e.native_version.is_empty() {
                        ""
                    } else {
                        &e.native_version
                    },
                    native_ver_w = native_ver_w,
                ));
            }
            if has_size {
                row.push_str(&format!(
                    " {:>size_w$}",
                    if e.size.is_empty() { "" } else { &e.size },
                    size_w = size_w,
                ));
            }
            if has_migrate {
                row.push_str(&format!(
                    " {:<migrate_w$}",
                    if e.migrate.is_empty() { "" } else { &e.migrate },
                    migrate_w = migrate_w,
                ));
            }
            println!("{}", row);
        }
        println!();
    }
    println!("{} package(s) total", entries.len());
}

/// Print entries as CSV (name,version,source,detail,status,native_available,size).
fn print_csv(entries: &[PackageEntry]) {
    println!("name,version,source,detail,status,native_available,native_version,size,migrate");
    for e in entries {
        println!(
            "{},{},{},{},{},{},{},{},{}",
            csv_field(&e.name),
            csv_field(&e.version),
            e.source.label(),
            csv_field(&e.detail),
            csv_field(&e.status),
            csv_field(&e.native_available),
            csv_field(&e.native_version),
            csv_field(&e.size),
            csv_field(&e.migrate),
        );
    }
}

/// Escape a field for CSV: wrap in quotes and double any embedded quotes
/// when the field contains a comma, quote, or newline.
fn csv_field(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}
