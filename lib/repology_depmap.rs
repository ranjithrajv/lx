// SPDX-License-Identifier: GPL-3.0-or-later

//! Repology integration for dependency resolution.
//!
//! Repology maps upstream project names to distribution-specific package
//! names across 200+ distributions. This module uses Repology as an online
//! fallback for dependency mapping — when our hardcoded depmap doesn't know
//! about a dependency, Repology can often tell us what it's called in the
//! target distribution.

/// Look up the distribution package name for an upstream project.
///
/// Queries Repology's API for the project, then finds the best match for
/// the target distribution family (debian, ubuntu, fedora, arch).
///
/// Returns None if the project isn't found or the distro has no package.
pub fn lookup_distro_package(upstream_name: &str, distro_family: &str) -> Option<String> {
    let source = crate::index::repology::RepologySource::new("repology");
    let project = source.lookup_project(upstream_name).ok().flatten()?;
    let repo_name = repo_name_for_family(distro_family)?;

    // Find the package for this distro.
    for pkg in &project.packages {
        if pkg.repo == repo_name {
            return Some(pkg.binname.clone().unwrap_or_else(|| {
                pkg.srcname
                    .clone()
                    .unwrap_or_else(|| upstream_name.to_string())
            }));
        }
    }

    // Fallback: try matching by srcname pattern.
    for pkg in &project.packages {
        if pkg.repo.starts_with(distro_family) {
            return Some(pkg.binname.clone().unwrap_or_else(|| {
                pkg.srcname
                    .clone()
                    .unwrap_or_else(|| upstream_name.to_string())
            }));
        }
    }

    None
}

/// Map a distro family name to Repology's repo naming convention.
///
/// Repology uses names like "debian_12", "ubuntu_24.04", "fedora_42", "arch".
/// We match by prefix since the exact version may vary.
fn repo_name_for_family(family: &str) -> Option<String> {
    match family.to_lowercase().as_str() {
        "debian" | "ubuntu" => {
            // Try to detect the specific version.
            if let Some(dist) = crate::debs::detect_dist() {
                if family == "ubuntu" {
                    // Read VERSION_ID from /etc/os-release.
                    if let Ok(text) = std::fs::read_to_string("/etc/os-release") {
                        for line in text.lines() {
                            if let Some(v) = line.strip_prefix("VERSION_ID=") {
                                return Some(format!("ubuntu_{}", v.trim_matches('"')));
                            }
                        }
                    }
                    Some("ubuntu_24.04".to_string())
                } else {
                    Some(format!("debian_{dist}"))
                }
            } else {
                Some(format!("debian_{dist}", dist = "12"))
            }
        }
        "fedora" => Some("fedora_42".to_string()),
        "arch" => Some("arch".to_string()),
        "opensuse" => Some("opensuse_tumbleweed".to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_name_for_debian() {
        let repo = repo_name_for_family("debian");
        assert!(repo.is_some());
        let repo = repo.unwrap();
        assert!(repo.starts_with("debian_"));
    }

    #[test]
    fn repo_name_for_arch() {
        assert_eq!(repo_name_for_family("arch"), Some("arch".to_string()));
    }

    #[test]
    fn repo_name_for_unknown() {
        assert_eq!(repo_name_for_family("plan9"), None);
    }
}
