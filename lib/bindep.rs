// SPDX-License-Identifier: GPL-3.0-or-later

//! Binary dependency detection via ELF analysis and distro package lookup.
//!
//! After staging the install tree, scans for ELF binaries, reads their
//! DT_NEEDED shared-library dependencies, and maps those sonames to
//! versioned system packages via the dpkg `symbols`/`shlibs` databases
//! (dpkg-shlibdeps parity), falling back to the host distribution's
//! package manager when no dpkg dependency information exists.
//!
//! This catches ACTUAL dependencies (what the binary links against) rather
//! than guessing from registry metadata. It's the difference between
//! "pillow probably needs libjpeg" and "this binary actually links libjpeg.so.8".

use anyhow::{Context, Result};
use std::path::Path;

/// Detect system dependencies for all ELF binaries in a staged install tree.
///
/// Returns a sorted, deduplicated list of system package names that the
/// binaries in `root` link against.
pub fn detect_binary_deps(root: &Path) -> Result<Vec<String>> {
    detect_binary_deps_excluding(root, None)
}

/// Like [`detect_binary_deps`], but drops any relation on `exclude_pkg` — a
/// package must never depend on itself.
///
/// Prefers versioned resolution via the host's dpkg `symbols`/`shlibs`
/// databases (dpkg-shlibdeps parity). Sonames with no dpkg dependency
/// information fall back to the package-manager lookup below, so rpm/pacman
/// hosts and libraries without dpkg metadata behave exactly as before.
pub fn detect_binary_deps_excluding(root: &Path, exclude_pkg: Option<&str>) -> Result<Vec<String>> {
    let elfs = find_elf_binaries(root)?;
    let scan = lx_lib::shlibdeps::scan_elfs(&elfs);
    if scan.needs.is_empty() {
        return Ok(Vec::new());
    }

    let db = lx_lib::shlibdeps::ShlibsDb::host();
    let res = lx_lib::shlibdeps::resolve(&scan.needs, db, exclude_pkg, &scan.provided);
    let mut names = res.resolved_names;
    let mut packages = res.relations;
    for soname in &res.unresolved {
        if let Some(pkg) = lookup_soname_package(soname) {
            if names.insert(pkg.to_ascii_lowercase()) {
                packages.push(pkg);
            }
        }
    }
    packages.sort();
    Ok(packages)
}

/// Find all ELF binaries under `dir`.
fn find_elf_binaries(dir: &Path) -> Result<Vec<std::path::PathBuf>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading '{}'", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            out.extend(find_elf_binaries(&path)?);
        } else if is_elf_binary(&path) {
            out.push(path);
        }
    }
    Ok(out)
}

/// True if `path` is an ELF binary (ET_EXEC), as opposed to a shared library
/// (ET_DYN/.so). Builds on the canonical magic-byte check in `filemeta::is_elf`.
fn is_elf_binary(path: &Path) -> bool {
    let Ok(true) = crate::filemeta::is_elf(path) else {
        return false;
    };
    // ELF type at offset 16: ET_EXEC=2, ET_DYN=3.
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    if bytes.len() < 18 {
        return false;
    }
    let e_type = u16::from_le_bytes([bytes[16], bytes[17]]);
    if e_type == 3 {
        // ET_DYN: could be a PIE executable or a shared library.
        // Filter out .so files by name.
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        return !name.contains(".so");
    }
    e_type == 2 // ET_EXEC
}

/// Map a soname (e.g., "libjpeg.so.8") to a system package name.
///
/// Uses the host distribution's package manager:
/// - Debian/Ubuntu: `dpkg -S`
/// - Fedora/RHEL: `rpm -qf`
/// - Arch: `pacman -Qo`
/// - Fallback: `ldconfig -p` + heuristic
fn lookup_soname_package(soname: &str) -> Option<String> {
    // Try dpkg first (Debian/Ubuntu).
    if let Some(pkg) = dpkg_owner(soname) {
        return Some(pkg);
    }

    // Try rpm (Fedora/RHEL).
    if let Some(pkg) = rpm_owner(soname) {
        return Some(pkg);
    }

    // Try pacman (Arch).
    if let Some(pkg) = pacman_owner(soname) {
        return Some(pkg);
    }

    // Fallback: ldcache heuristic (soname without version → libname).
    ldcache_heuristic(soname)
}

/// `dpkg -S soname` → owning package.
fn dpkg_owner(soname: &str) -> Option<String> {
    let out = std::process::Command::new("dpkg")
        .args(["-S", soname])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
    // Output: "package: path" or "package1, package2: path"
    let pkg = text.lines().next()?.split(':').next()?.trim();
    // Take the first package if multiple.
    let pkg = pkg.split(',').next()?.trim();
    if pkg.is_empty() {
        None
    } else {
        Some(pkg.to_string())
    }
}

/// `rpm -qf /path/to/lib` → owning package.
fn rpm_owner(soname: &str) -> Option<String> {
    // rpm needs a full path. Try common lib directories.
    for dir in &["/usr/lib", "/usr/lib64", "/lib", "/lib64"] {
        let path = format!("{}/{}", dir, soname);
        if std::path::Path::new(&path).exists() {
            let out = std::process::Command::new("rpm")
                .args(["-qf", &path])
                .output()
                .ok()?;
            if out.status.success() {
                let pkg = String::from_utf8(out.stdout).ok()?.trim().to_string();
                if !pkg.is_empty() && !pkg.contains("not owned") {
                    return Some(pkg);
                }
            }
        }
    }
    None
}

/// `pacman -Qo /path/to/lib` → owning package.
fn pacman_owner(soname: &str) -> Option<String> {
    for dir in &["/usr/lib", "/lib"] {
        let path = format!("{}/{}", dir, soname);
        if std::path::Path::new(&path).exists() {
            let out = std::process::Command::new("pacman")
                .args(["-Qo", &path])
                .output()
                .ok()?;
            if out.status.success() {
                // Output: "/path/to/lib is owned by package version"
                let text = String::from_utf8(out.stdout).ok()?;
                if let Some(part) = text.split("is owned by ").nth(1) {
                    let pkg = part.split_whitespace().next()?.to_string();
                    return Some(pkg);
                }
            }
        }
    }
    None
}

/// Heuristic fallback: libjpeg.so.8 → libjpeg8 or libjpeg-turbo.
fn ldcache_heuristic(soname: &str) -> Option<String> {
    // Strip .so and version numbers: libjpeg.so.8 → libjpeg.
    let base = soname
        .split(".so")
        .next()
        .unwrap_or(soname)
        .trim_end_matches('.');

    // Common patterns: libfoo.so.N → libfooN or libfoo-N.
    let version = soname.split(".so.").nth(1).unwrap_or("").trim();

    if version.is_empty() {
        return None;
    }

    // Try common Debian naming patterns.
    let candidates = [
        format!("{}{}", base, version),
        format!("{}-{}", base, version),
        format!("{}:{}", base, version), // Multiarch qualifier.
    ];

    // Check if any candidate is a known package via dpkg.
    for candidate in &candidates {
        if dpkg_owner(candidate).is_some() {
            return Some(candidate.clone());
        }
    }

    // Return the most likely candidate.
    Some(candidates[0].clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_elf_binaries_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let result = find_elf_binaries(dir.path()).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn is_elf_binary_rejects_text() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "not an elf").unwrap();
        assert!(!is_elf_binary(&path));
    }

    #[test]
    fn detect_binary_deps_empty() {
        let dir = tempfile::tempdir().unwrap();
        let result = detect_binary_deps(dir.path()).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn ldcache_heuristic_basic() {
        // libjpeg.so.8 → libjpeg8.
        let result = ldcache_heuristic("libjpeg.so.8");
        assert!(result.is_some());
        let result = result.unwrap();
        assert!(result.contains("libjpeg"));
    }

    #[test]
    fn ldcache_heuristic_no_version() {
        assert_eq!(ldcache_heuristic("libfoo"), None);
    }
}
