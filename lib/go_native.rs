//! `lx go-native` — migrate snap / flatpak / nix / `curl | sh` installs
//! to native packages.
//!
//! Two-stage (plan first, apply with `--yes`):
//! 1. **detect** installed non-native packages (`snap list`, `flatpak list`,
//!    `nix profile list`, plus orphan binaries in `/usr/local/bin`,
//!    `~/.local/bin`, `/opt` that no native package manager owns);
//! 2. **map** each to an lx package name via a built-in table;
//! 3. **plan** (default): print the migration plan + `missingnative` report;
//!    **apply** (`--yes`): `lx install` each mapped package, then remove the
//!    source (`snap remove` / `flatpak uninstall` / `nix profile remove`)
//!    unless `--keep-source`. `curl | sh` orphans are never auto-deleted —
//!    the plan prints manual cleanup commands instead.

use anyhow::{Context, Result};
use clap::Args;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::debs;
use crate::install::InstallArgs;

/// Snap runtimes / base layers that are never apps (these will likely
/// never have a native equivalent and must not be proposed for migration).
const EXCLUDED_SNAPS: &[&str] = &[
    "bare",
    "core",
    "core18",
    "core20",
    "core22",
    "core24",
    "snapd",
    "gtk-common-themes",
    "gtk2-common-themes",
    "gnome-42-2204",
    "gnome-46-2404",
    "kde-frameworks-5-qt-5-15-3-core20",
    "kde-frameworks-5-96-qt-5-15-9-core20",
    "mesa-core20",
    "mesa-core22",
    "mesa-2404",
    "cups",
    "snap-store",
];

/// One row of the snap/flatpak/nix/sh → native mapping table.
struct Mapping {
    /// snap name (exact match)
    snap: &'static str,
    /// flatpak app-id fragment (suffix match, case-insensitive)
    flatpak: &'static str,
    /// nix attribute name (exact match)
    nix: &'static str,
    /// orphan binary name left by `curl | sh` installers
    binary: &'static str,
    /// substring matched against the installer URL from shell history
    sh_url: &'static str,
    /// lx / native package name
    native: &'static str,
}

/// Curated mapping table (snap/flatpak-id/nix-attr/binary/URL → native),
/// compiled in. Contributions welcome — an unmapped package is reported, never dropped
/// silently.
const MAPPINGS: &[Mapping] = &[
    Mapping {
        snap: "firefox",
        flatpak: "org.mozilla.firefox",
        nix: "firefox",
        binary: "",
        sh_url: "",
        native: "firefox",
    },
    Mapping {
        snap: "chromium",
        flatpak: "org.chromium.chromium",
        nix: "chromium",
        binary: "",
        sh_url: "",
        native: "chromium",
    },
    Mapping {
        snap: "bitwarden",
        flatpak: "com.bitwarden",
        nix: "bitwarden",
        binary: "bw",
        sh_url: "bitwarden",
        native: "bitwarden",
    },
    Mapping {
        snap: "discord",
        flatpak: "com.discordapp.discord",
        nix: "discord",
        binary: "",
        sh_url: "",
        native: "discord",
    },
    Mapping {
        snap: "slack",
        flatpak: "com.slack.slack",
        nix: "slack",
        binary: "",
        sh_url: "",
        native: "slack",
    },
    Mapping {
        snap: "spotify",
        flatpak: "com.spotify.client",
        nix: "spotify",
        binary: "",
        sh_url: "",
        native: "spotify-client",
    },
    Mapping {
        snap: "telegram-desktop",
        flatpak: "org.telegram.desktop",
        nix: "telegram-desktop",
        binary: "",
        sh_url: "",
        native: "telegram",
    },
    Mapping {
        snap: "vlc",
        flatpak: "org.videolan.vlc",
        nix: "vlc",
        binary: "",
        sh_url: "",
        native: "vlc",
    },
    Mapping {
        snap: "gimp",
        flatpak: "org.gimp.gimp",
        nix: "gimp",
        binary: "",
        sh_url: "",
        native: "gimp",
    },
    Mapping {
        snap: "krita",
        flatpak: "org.kde.krita",
        nix: "krita",
        binary: "",
        sh_url: "",
        native: "krita",
    },
    Mapping {
        snap: "obs-studio",
        flatpak: "com.obsproject.studio",
        nix: "obs-studio",
        binary: "",
        sh_url: "",
        native: "obs-studio",
    },
    Mapping {
        snap: "kdenlive",
        flatpak: "org.kde.kdenlive",
        nix: "kdenlive",
        binary: "",
        sh_url: "",
        native: "kdenlive",
    },
    Mapping {
        snap: "libreoffice",
        flatpak: "org.libreoffice.libreoffice",
        nix: "libreoffice",
        binary: "",
        sh_url: "",
        native: "libreoffice",
    },
    Mapping {
        snap: "thunderbird",
        flatpak: "org.mozilla.thunderbird",
        nix: "thunderbird",
        binary: "",
        sh_url: "",
        native: "thunderbird",
    },
    Mapping {
        snap: "keepassxc",
        flatpak: "org.keepassxc.keepassxc",
        nix: "keepassxc",
        binary: "",
        sh_url: "",
        native: "keepassxc",
    },
    Mapping {
        snap: "onlyoffice-desktopeditors",
        flatpak: "org.onlyoffice",
        nix: "onlyoffice-bin",
        binary: "",
        sh_url: "",
        native: "onlyoffice-desktopeditors",
    },
    Mapping {
        snap: "skype",
        flatpak: "com.skype.client",
        nix: "skypeforlinux",
        binary: "",
        sh_url: "",
        native: "skypeforlinux",
    },
    Mapping {
        snap: "code",
        flatpak: "com.visualstudio.code",
        nix: "vscode",
        binary: "code",
        sh_url: "",
        native: "code",
    },
    Mapping {
        snap: "",
        flatpak: "com.valvesoftware.steam",
        nix: "steam",
        binary: "",
        sh_url: "",
        native: "steam",
    },
    Mapping {
        snap: "gimp",
        flatpak: "org.inkscape.inkscape",
        nix: "inkscape",
        binary: "",
        sh_url: "",
        native: "inkscape",
    },
    Mapping {
        snap: "blender",
        flatpak: "org.blender.blender",
        nix: "blender",
        binary: "",
        sh_url: "",
        native: "blender",
    },
    Mapping {
        snap: "",
        flatpak: "",
        nix: "",
        binary: "rustup",
        sh_url: "rustup.rs",
        native: "rustup",
    },
    Mapping {
        snap: "",
        flatpak: "",
        nix: "uv",
        binary: "uv",
        sh_url: "astral.sh",
        native: "uv",
    },
    Mapping {
        snap: "",
        flatpak: "",
        nix: "bun",
        binary: "bun",
        sh_url: "oven.sh/bun",
        native: "bun",
    },
    Mapping {
        snap: "",
        flatpak: "",
        nix: "deno",
        binary: "deno",
        sh_url: "deno.land",
        native: "deno",
    },
    Mapping {
        snap: "",
        flatpak: "",
        nix: "mise",
        binary: "mise",
        sh_url: "mise.run",
        native: "mise",
    },
    Mapping {
        snap: "node",
        flatpak: "",
        nix: "nodejs",
        binary: "fnm",
        sh_url: "fnm.vercel.app",
        native: "nodejs",
    },
    Mapping {
        snap: "",
        flatpak: "",
        nix: "volta",
        binary: "volta",
        sh_url: "get.volta.sh",
        native: "volta",
    },
    Mapping {
        snap: "go",
        flatpak: "",
        nix: "go",
        binary: "go",
        sh_url: "go.dev/dl",
        native: "golang",
    },
    Mapping {
        snap: "",
        flatpak: "",
        nix: "-flyctl",
        binary: "fly",
        sh_url: "fly.io/install",
        native: "flyctl",
    },
    Mapping {
        snap: "docker",
        flatpak: "",
        nix: "docker",
        binary: "docker-compose",
        sh_url: "get.docker.com",
        native: "docker-ce",
    },
    Mapping {
        snap: "kubectl",
        flatpak: "",
        nix: "kubectl",
        binary: "kubectl",
        sh_url: "dl.k8s.io",
        native: "kubectl",
    },
    Mapping {
        snap: "helm",
        flatpak: "",
        nix: "kubernetes-helm",
        binary: "helm",
        sh_url: "raw.githubusercontent.com/helm",
        native: "helm",
    },
    Mapping {
        snap: "terraform",
        flatpak: "",
        nix: "terraform",
        binary: "terraform",
        sh_url: "releases.hashicorp.com",
        native: "terraform",
    },
    Mapping {
        snap: "",
        flatpak: "",
        nix: "starship",
        binary: "starship",
        sh_url: "starship.rs/install",
        native: "starship",
    },
    Mapping {
        snap: "",
        flatpak: "",
        nix: "zoxide",
        binary: "zoxide",
        sh_url: "",
        native: "zoxide",
    },
    Mapping {
        snap: "",
        flatpak: "",
        nix: "eza",
        binary: "eza",
        sh_url: "",
        native: "eza",
    },
    Mapping {
        snap: "",
        flatpak: "",
        nix: "ripgrep",
        binary: "rg",
        sh_url: "",
        native: "ripgrep",
    },
    Mapping {
        snap: "",
        flatpak: "",
        nix: "fd",
        binary: "fd",
        sh_url: "",
        native: "fd",
    },
    Mapping {
        snap: "",
        flatpak: "",
        nix: "bat",
        binary: "bat",
        sh_url: "",
        native: "bat",
    },
    Mapping {
        snap: "",
        flatpak: "",
        nix: "fzf",
        binary: "fzf",
        sh_url: "",
        native: "fzf",
    },
    Mapping {
        snap: "",
        flatpak: "",
        nix: "yq",
        binary: "yq",
        sh_url: "github.com/mikefarah/yq",
        native: "yq",
    },
    Mapping {
        snap: "",
        flatpak: "",
        nix: "gh",
        binary: "gh",
        sh_url: "cli.github.com",
        native: "gh",
    },
    Mapping {
        snap: "",
        flatpak: "",
        nix: "glow",
        binary: "glow",
        sh_url: "",
        native: "glow",
    },
    Mapping {
        snap: "",
        flatpak: "",
        nix: "hugo",
        binary: "hugo",
        sh_url: "",
        native: "hugo",
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Source {
    Snap,
    Flatpak,
    Nix,
    Sh,
}

impl Source {
    fn name(self) -> &'static str {
        match self {
            Source::Snap => "snap",
            Source::Flatpak => "flatpak",
            Source::Nix => "nix",
            Source::Sh => "curl|sh",
        }
    }
}

struct Detected {
    source: Source,
    /// snap name / flatpak app id / nix attr / binary path
    id: String,
    /// extra context (version, installer URL)
    detail: String,
}

struct PlanEntry {
    detected: Detected,
    native: String,
}

#[derive(Debug, Clone, Args)]
pub struct GoNativeArgs {
    /// Only consider these sources (comma-separated: snap,flatpak,nix,sh).
    /// Default: all available.
    #[arg(long)]
    pub from: Option<String>,

    /// Native format to migrate to: deb, rpm, or arch. Defaults to the host
    /// package manager (dpkg → deb, rpm → rpm, pacman → arch).
    #[arg(long)]
    pub format: Option<String>,

    /// Apply the migration (install native packages, remove sources).
    /// Without it, only the plan is printed — nothing is mutated.
    #[arg(short = 'y', long)]
    pub yes: bool,

    /// Keep the snap/flatpak/nix package installed after the native
    /// install succeeds (default: remove it).
    #[arg(long)]
    pub keep_source: bool,

    /// Also remove the source manager itself (snapd, …) when every detected
    /// package from it was migrated. Off by default; always confirms.
    #[arg(long)]
    pub remove_manager: bool,

    /// Skip the `curl | sh` orphan-binary scan (managed sources only).
    #[arg(long)]
    pub skip_sh: bool,

    /// Only consider packages whose id contains one of these substrings.
    pub filter: Vec<String>,
}

pub fn run(args: GoNativeArgs, token: Option<&str>) -> Result<()> {
    let wanted = wanted_sources(&args);
    let format = detect_format(args.format.as_deref());
    println!("native format: {format} (host default; override with --format)");

    let mut detected = Vec::new();
    if wanted.contains(&Source::Snap) {
        detected.extend(detect_snaps());
    }
    if wanted.contains(&Source::Flatpak) {
        detected.extend(detect_flatpaks());
    }
    if wanted.contains(&Source::Nix) {
        detected.extend(detect_nix());
    }
    if wanted.contains(&Source::Sh) && !args.skip_sh {
        detected.extend(detect_sh_orphans());
    }
    if !args.filter.is_empty() {
        detected.retain(|d| {
            args.filter
                .iter()
                .any(|f| d.id.to_ascii_lowercase().contains(&f.to_ascii_lowercase()))
        });
    }

    if detected.is_empty() {
        println!("nothing to migrate: no snap/flatpak/nix/curl|sh packages detected");
        return Ok(());
    }

    let sh_urls = collect_sh_urls();
    let mut plan: Vec<PlanEntry> = Vec::new();
    let mut missing: Vec<&Detected> = Vec::new();
    for d in &detected {
        match map_native(d, &sh_urls) {
            Some(native) => plan.push(PlanEntry {
                detected: Detected {
                    source: d.source,
                    id: d.id.clone(),
                    detail: d.detail.clone(),
                },
                native,
            }),
            None => missing.push(d),
        }
    }

    println!(
        "\nmigration plan ({} detected, {} mapped):",
        detected.len(),
        plan.len()
    );
    for e in &plan {
        let extra = if e.detected.detail.is_empty() {
            String::new()
        } else {
            format!(" ({})", e.detected.detail)
        };
        println!(
            "  [{}] {}{} → {}",
            e.detected.source.name(),
            e.detected.id,
            extra,
            e.native
        );
    }
    if !missing.is_empty() {
        println!("\nmissingnative (no native mapping — left alone):");
        for d in &missing {
            println!("  [{}] {}", d.source.name(), d.id);
        }
        println!("consider contributing mappings upstream");
    }

    if !args.yes {
        println!("\nplan only — nothing installed or removed. Re-run with --yes to apply.");
        return Ok(());
    }

    if format != "deb" {
        println!("\napply is currently deb-only (`lx install` backend); ");
        println!("re-run on a dpkg host, or install the mapped packages manually: ");
        for e in &plan {
            println!("  {}", e.native);
        }
        return Ok(());
    }

    let mut installed = Vec::new();
    let mut failed = Vec::new();
    for e in &plan {
        if debs::dpkg_installed_version(&e.native).is_some() {
            println!(
                "  = {} already native-installed; skipping install",
                e.native
            );
            installed.push(e);
            continue;
        }
        println!("  ↓ installing native package {}", e.native);
        let res = crate::install::run(
            InstallArgs {
                package: e.native.clone(),
                version: None,
                arch: None,
                distribution: None,
                download_only: None,
                no_verify: false,
                allow_unverified: true,
                reinstall: false,
                yes: true,
            },
            token,
        );
        match res {
            Ok(()) => installed.push(e),
            Err(err) => {
                eprintln!("  ✗ native install of {} failed: {err:#}", e.native);
                failed.push(e);
            }
        }
    }

    if !args.keep_source {
        for e in &installed {
            if let Err(err) = remove_source(&e.detected) {
                eprintln!("  ⚠ could not remove source {}: {err:#}", e.detected.id);
            }
        }
    } else {
        println!("\n--keep-source: leaving all snap/flatpak/nix packages installed");
    }

    print_sh_cleanup(&plan);
    remove_managers(&args, &installed)?;

    if !failed.is_empty() {
        anyhow::bail!("{} of {} native installs failed", failed.len(), plan.len());
    }
    println!(
        "\n✓ go-native complete: {} package(s) migrated",
        installed.len()
    );
    Ok(())
}

fn wanted_sources(args: &GoNativeArgs) -> HashSet<Source> {
    let all = [Source::Snap, Source::Flatpak, Source::Nix, Source::Sh];
    let Some(from) = args.from.as_deref() else {
        return all.into_iter().collect();
    };
    from.split(',')
        .filter_map(|s| match s.trim().to_ascii_lowercase().as_str() {
            "snap" => Some(Source::Snap),
            "flatpak" => Some(Source::Flatpak),
            "nix" => Some(Source::Nix),
            "sh" | "curl" | "curl|sh" => Some(Source::Sh),
            _ => {
                eprintln!("  ⚠ unknown --from source '{s}'; expected snap,flatpak,nix,sh");
                None
            }
        })
        .collect()
}

/// Host-native format: explicit flag wins, else probe the package managers.
fn detect_format(flag: Option<&str>) -> String {
    if let Some(f) = flag {
        return f.trim().to_ascii_lowercase();
    }
    for (bin, fmt) in [("dpkg", "deb"), ("rpm", "rpm"), ("pacman", "arch")] {
        if Command::new(bin)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return fmt.to_string();
        }
    }
    "deb".to_string()
}

fn command_lines(bin: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(bin).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

fn detect_snaps() -> Vec<Detected> {
    let Some(text) = command_lines("snap", &["list"]) else {
        return Vec::new();
    };
    parse_snap_list(&text)
        .into_iter()
        .filter(|(name, _)| !EXCLUDED_SNAPS.iter().any(|x| x.eq_ignore_ascii_case(name)))
        .map(|(name, version)| Detected {
            source: Source::Snap,
            id: name,
            detail: version,
        })
        .collect()
}

/// Parse `snap list` output (columns: Name Version Rev Tracking …).
pub fn parse_snap_list(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if i == 0 || line.trim().is_empty() {
            continue; // header
        }
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() >= 2 {
            out.push((cols[0].to_string(), cols[1].to_string()));
        }
    }
    out
}

fn detect_flatpaks() -> Vec<Detected> {
    let Some(text) = command_lines(
        "flatpak",
        &["list", "--app", "--columns=application,version"],
    ) else {
        return Vec::new();
    };
    parse_flatpak_list(&text)
        .into_iter()
        .map(|(id, version)| Detected {
            source: Source::Flatpak,
            id,
            detail: version,
        })
        .collect()
}

/// Parse `flatpak list --app --columns=application,version` (tab-separated).
pub fn parse_flatpak_list(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|line| {
            let (id, version) = match line.split_once('\t') {
                Some((a, b)) => (a.trim(), b.trim()),
                None => (line.trim(), ""),
            };
            if id.is_empty() {
                return None; // malformed row (`--app` already excludes runtimes)
            }
            Some((id.to_string(), version.to_string()))
        })
        .collect()
}

fn detect_nix() -> Vec<Detected> {
    let Some(text) = command_lines("nix", &["profile", "list"]) else {
        return Vec::new();
    };
    parse_nix_profile(&text)
        .into_iter()
        .map(|(attr, version)| Detected {
            source: Source::Nix,
            id: attr,
            detail: version,
        })
        .collect()
}

/// Parse `nix profile list` rows like
/// `0 flake:nixpkgs#hello … attr-path: legacyPackages.x86_64-linux.hello`.
pub fn parse_nix_profile(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|line| {
            let attr = line
                .split("attr-path:")
                .nth(1)?
                .trim()
                .rsplit('.')
                .next()?
                .trim()
                .to_string();
            if attr.is_empty() {
                return None;
            }
            Some((attr, String::new()))
        })
        .collect()
}

/// Orphan binaries: executables under the unmanaged prefixes that no native
/// package manager claims.
fn detect_sh_orphans() -> Vec<Detected> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut prefixes: Vec<PathBuf> = vec![PathBuf::from("/usr/local/bin"), PathBuf::from("/opt")];
    if let Ok(home) = std::env::var("HOME") {
        prefixes.push(PathBuf::from(home).join(".local/bin"));
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
            out.push(Detected {
                source: Source::Sh,
                id: bin.display().to_string(),
                detail: String::new(),
            });
        }
    }
    out
}

fn list_executables(dir: &Path) -> Vec<PathBuf> {
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
            // one level (e.g. /opt/<tool>/bin) — don't recurse the world
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
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(_path: &Path) -> bool {
    false
}

/// True when dpkg/rpm/pacman claims the path.
fn native_owns(path: &Path) -> bool {
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

/// Installer URLs scraped from shell history (`curl … | sh` lines).
fn collect_sh_urls() -> Vec<String> {
    let mut urls = Vec::new();
    let home = std::env::var("HOME").unwrap_or_default();
    for hist in [
        format!("{home}/.bash_history"),
        format!("{home}/.zsh_history"),
        format!("{home}/.local/share/fish/fish_history"),
    ] {
        let Ok(text) = std::fs::read_to_string(&hist) else {
            continue;
        };
        urls.extend(parse_sh_urls(&text));
    }
    urls
}

/// Extract URLs from `curl <url> | sh` / `wget … | sh` history lines.
pub fn parse_sh_urls(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| l.contains("| sh") || l.contains("| bash"))
        .flat_map(|line| {
            line.split_whitespace()
                .filter(|t| t.starts_with("http://") || t.starts_with("https://"))
                .map(|t| {
                    t.trim_matches(|c| c == '"' || c == '\'' || c == ';' || c == '\\')
                        .to_string()
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn map_native(d: &Detected, sh_urls: &[String]) -> Option<String> {
    let lower = d.id.to_ascii_lowercase();
    match d.source {
        Source::Snap => MAPPINGS
            .iter()
            .find(|m| !m.snap.is_empty() && m.snap.eq_ignore_ascii_case(&d.id))
            .map(|m| m.native.to_string()),
        Source::Flatpak => MAPPINGS
            .iter()
            .find(|m| !m.flatpak.is_empty() && lower.contains(&m.flatpak.to_ascii_lowercase()))
            .map(|m| m.native.to_string()),
        Source::Nix => MAPPINGS
            .iter()
            .find(|m| !m.nix.is_empty() && m.nix.eq_ignore_ascii_case(&d.id))
            .map(|m| m.native.to_string()),
        Source::Sh => {
            let base = Path::new(&d.id)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&d.id);
            // binary-name match first …
            if let Some(m) = MAPPINGS
                .iter()
                .find(|m| !m.binary.is_empty() && m.binary.eq_ignore_ascii_case(base))
            {
                return Some(m.native.to_string());
            }
            // … then installer-URL match from history.
            sh_urls
                .iter()
                .flat_map(|u| {
                    MAPPINGS.iter().filter_map(|m| {
                        if !m.sh_url.is_empty() && u.contains(m.sh_url) {
                            Some(m.native.to_string())
                        } else {
                            None
                        }
                    })
                })
                .next()
        }
    }
}

fn remove_source(d: &Detected) -> Result<()> {
    let (bin, args): (&str, Vec<&str>) = match d.source {
        Source::Snap => ("snap", vec!["remove", &d.id]),
        Source::Flatpak => ("flatpak", vec!["uninstall", "-y", &d.id]),
        Source::Nix => ("nix", vec!["profile", "remove", &d.id]),
        Source::Sh => return Ok(()), // never auto-delete orphans
    };
    let status = Command::new(bin)
        .args(&args)
        .status()
        .with_context(|| format!("failed to run `{bin}`"))?;
    if !status.success() {
        anyhow::bail!("`{} {}` exited non-zero", bin, args.join(" "));
    }
    println!("  ✓ removed source [{}] {}", d.source.name(), d.id);
    Ok(())
}

/// Manual cleanup for `curl | sh` orphans — printed, never executed.
fn print_sh_cleanup(plan: &[PlanEntry]) {
    let orphans: Vec<&PlanEntry> = plan
        .iter()
        .filter(|e| e.detected.source == Source::Sh)
        .collect();
    if orphans.is_empty() {
        return;
    }
    println!("\nmanual cleanup for curl|sh orphans (review, then run yourself):");
    for e in &orphans {
        println!("  rm {}", e.detected.id);
    }
}

/// Opt-in manager removal when everything from that manager migrated.
fn remove_managers(args: &GoNativeArgs, installed: &[&PlanEntry]) -> Result<()> {
    if !args.remove_manager || installed.is_empty() {
        return Ok(());
    }
    for (source, bin, label) in [
        (Source::Snap, "snap", "snapd"),
        (Source::Flatpak, "flatpak", "flatpak"),
        (Source::Nix, "nix", "nix"),
    ] {
        let migrated: Vec<&&PlanEntry> = installed
            .iter()
            .filter(|e| e.detected.source == source)
            .collect();
        if migrated.is_empty() {
            continue;
        }
        // Only offer when nothing unmapped remains for that manager — check
        // by re-detecting: if the manager still lists packages, skip.
        let remaining = match source {
            Source::Snap => detect_snaps().len(),
            Source::Flatpak => detect_flatpaks().len(),
            Source::Nix => detect_nix().len(),
            Source::Sh => 0,
        };
        if remaining > 0 {
            println!("  (keeping {label}: {remaining} unmigrated package(s) remain)");
            continue;
        }
        println!("\nall {label} packages migrated.");
        if !confirm(&format!("Remove {label} itself (`{bin}`)?"))? {
            continue;
        }
        let _ = Command::new("sudo")
            .args(["apt-get", "remove", "-y", label])
            .status();
    }
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

/// Test probe: map one (`source`, `id`) pair through the mapping table.
/// `source` is one of "snap", "flatpak", "nix", "sh".
pub fn go_native_mapping_probe(source: &str, id: &str, sh_urls: &[String]) -> Option<String> {
    let src = match source {
        "snap" => Source::Snap,
        "flatpak" => Source::Flatpak,
        "nix" => Source::Nix,
        _ => Source::Sh,
    };
    let d = Detected {
        source: src,
        id: id.to_string(),
        detail: String::new(),
    };
    map_native(&d, sh_urls)
}
