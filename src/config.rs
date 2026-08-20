use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// A single Debian package definition, mirroring package.yaml.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageConfig {
    /// Name of the Debian package.
    pub package_name: String,
    /// GitHub repo in "owner/repo" form.
    pub github_repo: String,
    /// Archive format: tar.gz, tgz, zip, or raw.
    #[serde(default)]
    pub artifact_format: String,
    /// Human-readable description.
    #[serde(default)]
    pub description: String,
    /// Debian distributions to target. Defaults to all supported.
    #[serde(default)]
    pub debian_distributions: Vec<String>,
    /// Per-architecture release asset patterns. Omit for auto-discovery.
    #[serde(default)]
    pub architectures: HashMap<String, ArchConfig>,
    /// Path to the binary within the extracted archive.
    #[serde(default)]
    pub binary_path: String,
    /// Rename the installed binary to this command name.
    #[serde(default)]
    pub binary_rename: String,
    /// SPDX license identifier.
    #[serde(default)]
    pub license_spdx: String,
    /// Maintainer string for the package.
    #[serde(default)]
    pub maintainer: String,
    /// Version of the upstream software being packaged.
    #[serde(default)]
    pub version: String,
    /// Debian build version number (revision).
    #[serde(default)]
    pub build_version: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchConfig {
    /// Exact asset filename, with optional `{version}` placeholder.
    #[serde(default)]
    pub release_pattern: String,
}

impl PackageConfig {
    /// Load and parse a package.yaml file.
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read config file '{}'", path.display()))?;
        let config: PackageConfig =
            serde_yaml::from_str(&text).with_context(|| "failed to parse package.yaml")?;
        config.validate()?;
        Ok(config)
    }

    /// Structural validation independent of the network.
    pub fn validate(&self) -> Result<()> {
        if self.package_name.trim().is_empty() {
            bail!("package_name is required");
        }
        if self.github_repo.trim().is_empty() {
            bail!("github_repo is required");
        }
        if !self.github_repo.contains('/') {
            bail!(
                "github_repo must be in 'owner/repo' form, got '{}'",
                self.github_repo
            );
        }
        if !self.artifact_format.is_empty() {
            match self.artifact_format.as_str() {
                "tar.gz" | "tgz" | "zip" | "raw" => {}
                other => bail!(
                    "unsupported artifact_format '{other}' (expected tar.gz, tgz, zip, or raw)"
                ),
            }
        }
        Ok(())
    }

    /// Description fallback mirroring the action's config.sh:
    /// `${PACKAGE_NAME}, packaged from ${GITHUB_REPO}`.
    pub fn effective_description(&self) -> String {
        if self.description.is_empty() {
            format!("{}, packaged from {}", self.package_name, self.github_repo)
        } else {
            self.description.clone()
        }
    }

    /// Maintainer fallback (the action defaults to the repo owner / a
    /// generic maintainer identity).
    pub fn effective_maintainer(&self) -> String {
        if self.maintainer.is_empty() {
            "latest-debs maintainers <maintainers@latest-debs.org>".to_string()
        } else {
            self.maintainer.clone()
        }
    }

    /// Whether the config pins release patterns explicitly (deterministic)
    /// as opposed to relying on auto-discovery.
    pub fn has_manual_patterns(&self) -> bool {
        !self.architectures.is_empty()
            && self
                .architectures
                .values()
                .any(|a| !a.release_pattern.is_empty())
    }

    /// Resolve the effective distributions, applying built-in rules:
    /// - empty -> all supported suites
    pub fn effective_distributions(&self) -> Vec<String> {
        if self.debian_distributions.is_empty() {
            DEFAULT_DISTRIBUTIONS
                .iter()
                .map(|s| s.to_string())
                .collect()
        } else {
            self.debian_distributions.clone()
        }
    }

    /// Resolve the effective architectures to build.
    pub fn effective_architectures(&self) -> Vec<String> {
        if self.architectures.is_empty() {
            DEFAULT_ARCHITECTURES
                .iter()
                .map(|s| s.to_string())
                .collect()
        } else {
            self.architectures.keys().cloned().collect()
        }
    }

    /// Whether an architecture is supported in a given Debian distribution,
    /// mirroring the action's `is_arch_supported_for_dist` (data/system.yaml).
    pub fn arch_supported_for_dist(&self, arch: &str, dist: &str) -> bool {
        if UNIVERSAL_ARCHS.contains(&arch) {
            return true;
        }
        // Distribution-restricted architectures: i386/armel lose support after
        // Trixie; riscv64/loong64 only gain it in newer suites.
        match arch {
            "i386" | "armel" => matches!(dist, "bookworm" | "trixie"),
            "riscv64" => matches!(dist, "trixie" | "forky" | "sid"),
            "loong64" => matches!(dist, "forky" | "sid"),
            _ => false,
        }
    }
}

/// Architectures supported in every Debian distribution (system.yaml).
const UNIVERSAL_ARCHS: &[&str] = &[
    "amd64", "arm64", "armhf", "ppc64el", "s390x", "riscv64", "loong64",
];

/// All Debian suites the tool knows how to build for.
pub const DEFAULT_DISTRIBUTIONS: [&str; 4] = ["bookworm", "trixie", "forky", "sid"];

/// All Debian architectures the tool can target.
pub const DEFAULT_ARCHITECTURES: [&str; 9] = [
    "amd64", "arm64", "armel", "armhf", "i386", "ppc64el", "s390x", "riscv64", "loong64",
];

/// Architecture aliases commonly used by upstream projects, keyed by
/// the canonical Debian architecture.
pub fn upstream_arch_names() -> &'static HashMap<&'static str, &'static [&'static str]> {
    use std::sync::OnceLock;
    static MAP: OnceLock<HashMap<&'static str, &'static [&'static str]>> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut m = HashMap::new();
        m.insert("amd64", &["x86_64", "amd64"][..]);
        m.insert("arm64", &["aarch64", "arm64"][..]);
        m.insert("armel", &["arm", "armeabi"][..]);
        m.insert(
            "armhf",
            &["armv7", "armhf", "gnueabihf", "eabihf", "arm-gnueabihf"][..],
        );
        m.insert("i386", &["i386", "i686", "x86"][..]);
        m.insert("ppc64el", &["powerpc64le", "ppc64le"][..]);
        m.insert("s390x", &["s390x"][..]);
        m.insert("riscv64", &["riscv64", "riscv64gc"][..]);
        m.insert("loong64", &["loongarch64", "loong64"][..]);
        m
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_manual_patterns() {
        let yaml = r#"
package_name: atuin
github_repo: atuinsh/atuin
artifact_format: tar.gz
description: "Magical shell history"
debian_distributions: [trixie, forky, sid]
architectures:
  amd64:
    release_pattern: "atuin-x86_64-unknown-linux-gnu.tar.gz"
  arm64:
    release_pattern: "atuin-aarch64-unknown-linux-gnu.tar.gz"
"#;
        let cfg: PackageConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.package_name, "atuin");
        assert_eq!(
            cfg.effective_distributions(),
            vec!["trixie", "forky", "sid"]
        );
        assert!(cfg.has_manual_patterns());
        assert_eq!(
            cfg.architectures["amd64"].release_pattern,
            "atuin-x86_64-unknown-linux-gnu.tar.gz"
        );
    }

    #[test]
    fn auto_discovery_defaults() {
        let yaml = r#"
package_name: eza
github_repo: eza-community/eza
artifact_format: tar.gz
"#;
        let cfg: PackageConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(!cfg.has_manual_patterns());
        assert_eq!(cfg.effective_distributions().len(), 4);
        assert_eq!(cfg.effective_architectures().len(), 9);
    }

    #[test]
    fn rejects_unknown_fields() {
        let yaml = "package_name: x\ngithub_repo: a/b\nbogus_field: nope\n";
        assert!(serde_yaml::from_str::<PackageConfig>(yaml).is_err());
    }

    #[test]
    fn arch_dist_support_matrix() {
        let cfg = PackageConfig::default();
        // Universal architectures work everywhere.
        assert!(cfg.arch_supported_for_dist("amd64", "bookworm"));
        assert!(cfg.arch_supported_for_dist("amd64", "sid"));
        assert!(cfg.arch_supported_for_dist("loong64", "bookworm")); // universal per system.yaml
                                                                     // i386/armel lose support after trixie.
        assert!(cfg.arch_supported_for_dist("i386", "bookworm"));
        assert!(cfg.arch_supported_for_dist("i386", "trixie"));
        assert!(!cfg.arch_supported_for_dist("i386", "forky"));
        assert!(!cfg.arch_supported_for_dist("armel", "sid"));
        // Unknown arch is never supported.
        assert!(!cfg.arch_supported_for_dist("mips64el", "bookworm"));
    }

    #[test]
    fn loong64_aliases() {
        let map = upstream_arch_names();
        assert!(map["loong64"].contains(&"loongarch64"));
        assert!(map["loong64"].contains(&"loong64"));
    }
}
