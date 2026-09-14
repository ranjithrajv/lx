// SPDX-License-Identifier: GPL-3.0-or-later

//! Host build-dependency solving and installation.
//!
//! `lx`'s source builds compile on the host, so they need host toolchains.
//! Historically `build_depends:` named packages the caller/CI had to install
//! and `lx` only reported what was missing. This module adds the missing
//! half: detect the host package manager, work out which declared build
//! dependencies (and which of the selected build system's `required_tools()`)
//! are absent, and install them on request (`--install-build-deps`).
//!
//! `build_depends:` stays in **host-distro package names** — `lx` does not
//! attempt cross-distro renaming. The build system's tool names, which are
//! distro-independent, *are* mapped to the right package per host
//! ([`host_package`]).

use anyhow::{bail, Context, Result};
use std::collections::BTreeSet;
use std::process::Command;

/// The host's package manager. Behaviour differs enough between them (query
/// and install commands) that a closed enum + `match` is clearer than a trait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostPm {
    Apt,
    Dnf,
    Zypper,
    Pacman,
    Apk,
    Xbps,
}

impl HostPm {
    /// Detect the host package manager from the install tools on `PATH`.
    /// Ordered so a Debian host with a foreign `dpkg`/`pacman` still resolves
    /// to the tool that actually installs packages.
    pub fn detect() -> Option<Self> {
        const ORDER: [(&str, HostPm); 7] = [
            ("apt-get", HostPm::Apt),
            ("dnf", HostPm::Dnf),
            ("yum", HostPm::Dnf),
            ("zypper", HostPm::Zypper),
            ("pacman", HostPm::Pacman),
            ("apk", HostPm::Apk),
            ("xbps-install", HostPm::Xbps),
        ];
        ORDER.iter().find(|(bin, _)| exists(bin)).map(|(_, pm)| *pm)
    }

    pub fn name(&self) -> &'static str {
        match self {
            HostPm::Apt => "apt",
            HostPm::Dnf => "dnf",
            HostPm::Zypper => "zypper",
            HostPm::Pacman => "pacman",
            HostPm::Apk => "apk",
            HostPm::Xbps => "xbps",
        }
    }

    /// The install program to invoke (for Dnf this resolves to `dnf` or the
    /// older `yum`).
    pub fn install_program(&self) -> &'static str {
        match self {
            HostPm::Apt => "apt-get",
            HostPm::Dnf => {
                if exists("dnf") {
                    "dnf"
                } else {
                    "yum"
                }
            }
            HostPm::Zypper => "zypper",
            HostPm::Pacman => "pacman",
            HostPm::Apk => "apk",
            HostPm::Xbps => "xbps-install",
        }
    }

    /// Whether `pkg` is installed (a real, local query, not a PATH guess).
    pub fn is_installed(&self, pkg: &str) -> bool {
        let ok = |cmd: &str, args: &[&str]| {
            Command::new(cmd)
                .args(args)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        };
        match self {
            // `dpkg -s` succeeds for known-but-removed packages too, so check
            // the actual status line.
            HostPm::Apt => Command::new("dpkg")
                .args(["-s", pkg])
                .output()
                .map(|o| {
                    o.status.success()
                        && String::from_utf8_lossy(&o.stdout)
                            .contains("Status: install ok installed")
                })
                .unwrap_or(false),
            HostPm::Dnf | HostPm::Zypper => ok("rpm", &["-q", pkg]),
            HostPm::Pacman => ok("pacman", &["-Q", pkg]),
            HostPm::Apk => ok("apk", &["info", "-e", pkg]),
            HostPm::Xbps => ok("xbps-query", &[pkg]),
        }
    }
}

/// Map a distro-independent build-tool name (from a [`BuildSystem`]'s
/// `required_tools()`) to the host's package name. `None` for tools with no
/// known mapping — the caller then relies on the plain `PATH` check.
///
/// [`BuildSystem`]: crate::plugins::build_system::BuildSystem
pub fn host_package(pm: HostPm, tool: &str) -> Option<&'static str> {
    Some(match (tool, pm) {
        ("cmake", _) => "cmake",
        ("ninja", HostPm::Apt | HostPm::Dnf | HostPm::Zypper) => "ninja-build",
        ("ninja", _) => "ninja",
        ("cargo", HostPm::Pacman) => "rust",
        ("cargo", _) => "cargo",
        ("rustc", HostPm::Pacman) => "rust",
        ("rustc", _) => "rustc",
        ("go", HostPm::Apt) => "golang-go",
        ("go", HostPm::Dnf | HostPm::Zypper | HostPm::Apk) => "golang",
        ("go", HostPm::Pacman | HostPm::Xbps) => "go",
        ("pkg-config", HostPm::Pacman | HostPm::Apk) => "pkgconf",
        ("pkg-config", _) => "pkg-config",
        ("musl-gcc", HostPm::Apt) => "musl-tools",
        ("musl-gcc", HostPm::Pacman) => "musl",
        ("meson", _) => "meson",
        ("make", _) => "make",
        ("autoconf", _) => "autoconf",
        ("automake", _) => "automake",
        ("libtool", _) => "libtool",
        _ => return None,
    })
}

/// The full, deduplicated, sorted set of host packages to ensure: explicit
/// `build_depends` entries verbatim, plus the build system's tools mapped
/// through [`host_package`].
pub fn resolve(explicit: &[String], tools: &[&str], pm: HostPm) -> Vec<String> {
    let mut out = BTreeSet::new();
    for dep in explicit {
        let dep = dep.trim();
        if !dep.is_empty() {
            out.insert(dep.to_string());
        }
    }
    for tool in tools {
        if let Some(pkg) = host_package(pm, tool) {
            out.insert(pkg.to_string());
        }
    }
    out.into_iter().collect()
}

/// The subset of `pkgs` not installed on the host.
pub fn missing(pm: HostPm, pkgs: &[String]) -> Vec<String> {
    pkgs.iter()
        .filter(|pkg| !pm.is_installed(pkg))
        .cloned()
        .collect()
}

/// The command that would install `pkgs`, with a `sudo` prefix when
/// requested.
pub fn install_command(pm: HostPm, pkgs: &[String], sudo: bool) -> (String, Vec<String>) {
    let mut args: Vec<String> = match pm {
        HostPm::Apt => vec!["install".into(), "-y".into()],
        HostPm::Dnf => vec!["install".into(), "-y".into()],
        HostPm::Zypper => vec!["--non-interactive".into(), "install".into()],
        HostPm::Pacman => vec!["-S".into(), "--needed".into(), "--noconfirm".into()],
        HostPm::Apk => vec!["add".into()],
        HostPm::Xbps => vec!["-y".into()],
    };
    args.extend(pkgs.iter().cloned());
    if sudo {
        let mut full = vec![pm.install_program().to_string()];
        full.extend(args);
        ("sudo".to_string(), full)
    } else {
        (pm.install_program().to_string(), args)
    }
}

/// Ensure `deps` are installed on the host.
///
/// With `install` false this only *reports* — bailing with the exact command
/// to run, matching the historical `build_depends` behavior. With `install`
/// true it runs the host package manager (via `sudo` unless already root);
/// `dry_run` prints the command instead of running it.
pub fn ensure_build_deps(deps: &[String], install: bool, dry_run: bool) -> Result<()> {
    if deps.is_empty() {
        return Ok(());
    }
    let Some(pm) = HostPm::detect() else {
        bail!(
            "missing host build dependencies: {} (no supported package manager on PATH; install them manually)",
            deps.join(" ")
        );
    };
    let missing = missing(pm, deps);
    if missing.is_empty() {
        return Ok(());
    }

    let root = unsafe { libc::geteuid() } == 0;
    let sudo = !root && exists("sudo");
    let (program, args) = install_command(pm, &missing, sudo);
    let display = format!("{program} {}", args.join(" "));

    if !install {
        bail!(
            "missing host build dependencies: {} (install them: {display}; or pass --install-build-deps)",
            missing.join(" ")
        );
    }
    if dry_run {
        println!("would install build dependencies: {display}");
        return Ok(());
    }
    if !root && !sudo {
        bail!(
            "installing build dependencies needs root: run `{display}` as root (or install sudo)"
        );
    }

    println!(
        "installing {} host build dependenc{} via {}: {}",
        missing.len(),
        if missing.len() == 1 { "y" } else { "ies" },
        pm.name(),
        missing.join(" ")
    );
    let status = Command::new(&program)
        .args(&args)
        .status()
        .with_context(|| format!("failed to run `{display}`"))?;
    if !status.success() {
        bail!("`{display}` failed");
    }
    Ok(())
}

/// Whether `name` exists on `PATH` (cheap directory scan, never executes it).
fn exists(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(name).is_file()))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_package_maps_distro_differences() {
        assert_eq!(host_package(HostPm::Apt, "cmake"), Some("cmake"));
        // ninja is `ninja-build` on apt/rpm, `ninja` on Arch.
        assert_eq!(host_package(HostPm::Apt, "ninja"), Some("ninja-build"));
        assert_eq!(host_package(HostPm::Pacman, "ninja"), Some("ninja"));
        // Rust ships as `rust` on Arch, `cargo` elsewhere.
        assert_eq!(host_package(HostPm::Pacman, "cargo"), Some("rust"));
        assert_eq!(host_package(HostPm::Apt, "cargo"), Some("cargo"));
        // Go's package name varies by family.
        assert_eq!(host_package(HostPm::Apt, "go"), Some("golang-go"));
        assert_eq!(host_package(HostPm::Dnf, "go"), Some("golang"));
        assert_eq!(host_package(HostPm::Pacman, "go"), Some("go"));
        // Unknown tools have no mapping.
        assert_eq!(host_package(HostPm::Apt, "definitely-not-a-tool"), None);
    }

    #[test]
    fn resolve_dedups_and_keeps_explicit_verbatim() {
        let explicit = vec!["libssl-dev".to_string(), "cmake".to_string()];
        let tools = ["cmake", "ninja", "unknown-tool"];
        let got = resolve(&explicit, &tools, HostPm::Apt);
        assert_eq!(got, vec!["cmake", "libssl-dev", "ninja-build"]);
    }

    #[test]
    fn install_command_shapes() {
        let pkgs = vec!["cmake".to_string(), "ninja".to_string()];
        let (prog, args) = install_command(HostPm::Apt, &pkgs, true);
        assert_eq!(prog, "sudo");
        assert_eq!(args, vec!["apt-get", "install", "-y", "cmake", "ninja"]);

        let (prog, args) = install_command(HostPm::Pacman, &pkgs, false);
        assert_eq!(prog, "pacman");
        assert_eq!(
            args,
            vec!["-S", "--needed", "--noconfirm", "cmake", "ninja"]
        );

        let (prog, args) = install_command(HostPm::Xbps, &pkgs, false);
        assert_eq!(prog, "xbps-install");
        assert_eq!(args, vec!["-y", "cmake", "ninja"]);

        let (prog, args) = install_command(HostPm::Apk, &pkgs, false);
        assert_eq!(prog, "apk");
        assert_eq!(args, vec!["add", "cmake", "ninja"]);
    }

    #[test]
    fn detect_resolves_to_a_known_manager_on_this_host() {
        let pm = HostPm::detect().expect("expected at least one supported package manager");
        assert!(!pm.name().is_empty());
        // A nonsense package is never installed.
        assert!(missing(pm, &["definitely-not-a-real-package-zzz".to_string()]).len() == 1);
    }
}
