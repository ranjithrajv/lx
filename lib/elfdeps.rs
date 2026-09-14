// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared-library dependency scanning: read an ELF binary's `DT_NEEDED`
//! entries (the same information `ldd`/`readelf -d` report), natively via
//! the `object` crate -- no `ldd`/`objdump`/`readelf` host-tool dependency,
//! consistent with the rest of this project's subprocess-free
//! packaging pipeline.

use anyhow::{Context, Result};
use object::read::Object;
use std::sync::OnceLock;

/// The shared libraries a binary needs at runtime (`DT_NEEDED` entries, in
/// the order they appear in the ELF dynamic section). Empty for a
/// statically linked binary.
pub fn needed_libraries(bytes: &[u8]) -> Result<Vec<String>> {
    let file = object::File::parse(bytes).context("failed to parse as an ELF/object file")?;
    let mut names = Vec::new();
    for lib in file
        .import_libraries()
        .context("failed to read dynamic library dependencies")?
    {
        let lib = lib.context("failed to read a dynamic library dependency entry")?;
        names.push(String::from_utf8_lossy(lib.name()).into_owned());
    }
    Ok(names)
}

/// Well-known glibc sonames across architectures. Used as a fast-path
/// before falling back to dynamic package-manager detection.
const GLIBC_SONAMES: &[&str] = &[
    "libc.so.6",
    "libm.so.6",
    "libdl.so.2",
    "libpthread.so.0",
    "librt.so.1",
    "libresolv.so.2",
    "libutil.so.1",
    "libanl.so.1",
    "ld-linux-x86-64.so.2",
    "ld-linux-aarch64.so.1",
    "ld-linux-armhf.so.3",
    "ld-linux.so.2",
    "ld-linux-x86-64-musl.so.1",
    "ld-musl-x86_64.so.1",
    "ld-musl-aarch64.so.1",
    "ld-musl-armhf.so.1",
];

/// Cached set of package names that provide the C runtime on this host
/// (e.g. `libc6`, `glibc`, `musl`, `musl-libc`). Queried once from the
/// host's package manager, then reused for all `is_essential_libc_soname`
/// checks. Empty when no supported package manager is available.
fn libc_package_names() -> &'static std::collections::HashSet<String> {
    static CACHE: OnceLock<std::collections::HashSet<String>> = OnceLock::new();
    CACHE.get_or_init(detect_libc_packages)
}

/// Detect which installed packages provide the C runtime by querying the
/// host's package manager. Returns a set of package names (lowercased)
/// that should be treated as "essential" -- sonames owned by any of these
/// packages are omitted from `depends:`.
pub fn detect_libc_packages() -> std::collections::HashSet<String> {
    let mut pkgs = std::collections::HashSet::new();
    // dpkg: query which package owns the canonical libc soname.
    if let Ok(out) = std::process::Command::new("dpkg")
        .args(["-S", "libc.so.6"])
        .output()
    {
        if out.status.success() {
            if let Ok(text) = String::from_utf8(out.stdout) {
                if let Some(line) = text.lines().next() {
                    if let Some(name) = line.split(':').next() {
                        pkgs.insert(name.trim().to_ascii_lowercase());
                    }
                }
            }
        }
    }
    // rpm: check glibc and musl.
    for query in &["glibc", "musl", "musl-libc"] {
        if let Ok(out) = std::process::Command::new("rpm")
            .args(["-q", query])
            .output()
        {
            if out.status.success() {
                if let Ok(text) = String::from_utf8(out.stdout) {
                    let line = text.trim();
                    if !line.starts_with("not installed") {
                        // Extract just the package name from NEVRA.
                        let name = line.rsplit_once('-').map(|(n, _)| n).unwrap_or(line);
                        let name = name.rsplit_once('-').map(|(n, _)| n).unwrap_or(name);
                        let name = name.split_once(':').map(|(_, n)| n).unwrap_or(name);
                        if !name.is_empty() {
                            pkgs.insert(name.to_ascii_lowercase());
                        }
                    }
                }
            }
        }
    }
    // pacman: check glibc and musl.
    for pkg in &["glibc", "musl"] {
        if let Ok(out) = std::process::Command::new("pacman")
            .args(["-Qi", pkg])
            .output()
        {
            if out.status.success() {
                if let Ok(text) = String::from_utf8(out.stdout) {
                    for line in text.lines() {
                        if let Some(name) = line.strip_prefix("Name            : ") {
                            pkgs.insert(name.trim().to_ascii_lowercase());
                        }
                    }
                }
            }
        }
    }
    pkgs
}

/// True if `soname` is provided by the C runtime (glibc, musl, etc.) and
/// can usually be omitted from `depends:`. Checks the static glibc list
/// first (fast path), then queries the host's package manager to catch
/// musl and other libc implementations.
pub fn is_essential_libc_soname(soname: &str) -> bool {
    // Fast path: well-known glibc sonames.
    if GLIBC_SONAMES.contains(&soname) {
        return true;
    }
    // Dynamic path: check if any libc package on this host owns the soname.
    let libc_names = libc_package_names();
    if libc_names.is_empty() {
        return false;
    }
    // Use pkg_owner from scandeps to resolve the soname, then check if
    // the owning package is a known libc package. This avoids a circular
    // dependency by inlining a lightweight owner lookup.
    owner_of(soname)
        .map(|owner| libc_names.contains(&owner.to_ascii_lowercase()))
        .unwrap_or(false)
}

/// Lightweight soname-to-package lookup, independent of `scandeps::pkg_owner`
/// to avoid circular module dependencies. Tries dpkg, rpm, pacman in order.
fn owner_of(soname: &str) -> Option<String> {
    // dpkg -S
    if let Ok(out) = std::process::Command::new("dpkg")
        .args(["-S", soname])
        .output()
    {
        if out.status.success() {
            if let Ok(text) = String::from_utf8(out.stdout) {
                if let Some(pkg) = text.lines().next()?.split(':').next() {
                    let pkg = pkg.trim();
                    if !pkg.is_empty() {
                        return Some(pkg.split(':').next().unwrap_or(pkg).to_string());
                    }
                }
            }
        }
    }
    // rpm --whatprovides
    if let Ok(out) = std::process::Command::new("rpm")
        .args(["-q", "--whatprovides", soname])
        .output()
    {
        if out.status.success() {
            if let Ok(text) = String::from_utf8(out.stdout) {
                let line = text.lines().next()?.trim();
                if !line.starts_with("no package provides") && !line.is_empty() {
                    let name = line.rsplit_once('-').map(|(n, _)| n).unwrap_or(line);
                    let name = name.rsplit_once('-').map(|(n, _)| n).unwrap_or(name);
                    let name = name.split_once(':').map(|(_, n)| n).unwrap_or(name);
                    if !name.is_empty() {
                        return Some(name.to_string());
                    }
                }
            }
        }
    }
    None
}
