use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// One entry of `contents:` — an extra file/dir/symlink layered into the
/// staged install tree, mirroring nfpm's `contents: [{src, dst, type}]`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentEntry {
    /// Source path in the build environment (relative paths resolve against
    /// the current working directory). Not required for `type: dir` or
    /// `type: symlink` (whose target is package-internal, per nfpm).
    #[serde(default)]
    pub src: String,
    /// Absolute destination path inside the package (must start with `/`).
    pub dst: String,
    /// Entry type: ""/"file" (copy), "config"/"config|noreplace"/
    /// "config|missingok" (copy + register as a deb conffile), "tree"
    /// (recursive directory copy), "symlink", "dir", or "ghost" (RPM-only;
    /// a no-op for other formats, matching nfpm).
    #[serde(default, rename = "type")]
    pub kind: String,
}

/// Per-format relation-field overrides (`overrides: {deb: {depends: ...}}`),
/// mirroring nfpm. An overridden field *replaces* the top-level value for
/// that format — including replacing it with an empty string to clear it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FormatOverrides {
    #[serde(default)]
    pub depends: Option<String>,
    #[serde(default)]
    pub recommends: Option<String>,
    #[serde(default)]
    pub suggests: Option<String>,
    #[serde(default)]
    pub conflicts: Option<String>,
    #[serde(default)]
    pub replaces: Option<String>,
    #[serde(default)]
    pub provides: Option<String>,
    #[serde(default)]
    pub breaks: Option<String>,
    #[serde(default)]
    pub predepends: Option<String>,
}

/// Maintainer scripts (`scripts:`), mirroring nfpm's pre/post-install and
/// pre/post-remove names. Paths are in the build environment; for deb they
/// become `DEBIAN/{preinst,postinst,prerm,postrm}` (mode 0755); for rpm
/// they map to `%pre`/`%post`/`%preun`/`%postun` scriptlets.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scripts {
    #[serde(default)]
    pub preinstall: String,
    #[serde(default)]
    pub postinstall: String,
    #[serde(default)]
    pub preremove: String,
    #[serde(default)]
    pub postremove: String,
}

/// Package signing configuration (`signature:`).
///
/// - **rpm**: the armored secret key is loaded natively and the signature
///   is embedded in the `.rpm` header (verifiable via `rpm -K`).
/// - **deb**: `gpg --detach-sign` produces a `<file>.sig` next to each
///   built `.deb` (asset-level verification; apt repository publishing
///   additionally signs the Release index as usual).
///
/// Passphrase resolution (both formats): `$LPT_SIGN_PASSPHRASE`, falling
/// back to `$NFPM_PASSPHRASE` for nfpm parity.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignatureConfig {
    /// Path to an ASCII-armored secret key.
    #[serde(default)]
    pub key_file: String,
    /// Optional key id / fingerprint (passed to gpg as `--local-user`;
    /// ignored by the rpm crate, which uses the key file's primary/subkey).
    #[serde(default)]
    pub key_id: String,
}

/// A single Debian package definition, mirroring package.yaml.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageConfig {
    /// Name of the Debian package.
    pub package_name: String,
    /// Source repo in "owner/repo" form (GitHub or GitLab, per `source`).
    /// Kept as `github_repo` for backward compat; `repo` is alias.
    #[serde(alias = "repo")]
    #[serde(alias = "gitlab_repo")]
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
    /// Per-architecture release asset patterns, or a plain list restricting
    /// auto-discovery to a named subset. Omit entirely for full
    /// auto-discovery.
    #[serde(default)]
    pub architectures: ArchSpec,
    /// Path to the binary within the extracted archive.
    #[serde(default)]
    pub binary_path: String,
    /// Rename the installed binary to this command name.
    #[serde(default)]
    pub binary_rename: String,
    /// Install the whole `binary_path` tree under `/usr/lib/<package_name>/`
    /// and symlink its `bin/` executables into `/usr/bin`, instead of
    /// flattening loose ELF files into `/usr/bin`. Needed for apps with
    /// `$ORIGIN`-relative RPATH that must keep their directory layout intact
    /// (e.g. zed-industries/zed).
    #[serde(default)]
    pub bundle: bool,
    /// Comma-separated `Depends:` line for binaries needing a runtime
    /// library not present on a bare Debian install (e.g. "libatomic1").
    #[serde(default)]
    pub depends: String,
    /// Comma-separated `Recommends:` line (strongly-associated but
    /// non-essential packages apt installs by default alongside this one).
    #[serde(default)]
    pub recommends: String,
    /// Comma-separated `Conflicts:` line (packages that cannot be
    /// installed at the same time as this one).
    #[serde(default)]
    pub conflicts: String,
    /// Comma-separated `Replaces:` line (packages/files this package
    /// takes over from, e.g. a distro's own older build of the same tool).
    #[serde(default)]
    pub replaces: String,
    /// Comma-separated `Provides:` line (virtual packages this package
    /// satisfies).
    #[serde(default)]
    pub provides: String,
    /// Comma-separated `Breaks:` line (packages this version is known to
    /// break, without literally conflicting with them).
    #[serde(default)]
    pub breaks: String,
    /// Comma-separated `Suggests:` line (packages that are suggested alongside
    /// this one, not installed by default).
    #[serde(default)]
    pub suggests: String,
    /// Comma-separated `Pre-Depends:` line (packages that must be fully
    /// installed before this package is unpacked).
    #[serde(default)]
    pub predepends: String,
    /// Debian Section field (e.g. "utils", "devel"). Defaults to "utils".
    #[serde(default)]
    pub section: String,
    /// Debian Priority field (e.g. "optional", "extra"). Defaults to "optional".
    #[serde(default)]
    pub priority: String,
    /// Additional arbitrary control fields (e.g. Bugs, Homepage extras).
    /// Mirrors nfpm's `deb.fields`.
    #[serde(default)]
    pub fields: HashMap<String, String>,
    /// Compression for the deb's control.tar and data.tar members: "gzip"
    /// (default), "xz", "zstd", or "none". Mirrors nfpm's `deb.compression`.
    #[serde(default)]
    pub compression: String,
    /// Extra files/dirs/symlinks layered into the staged install tree after
    /// the release payload (shell completions, systemd units, desktop
    /// files, ...). See [`ContentEntry`].
    #[serde(default)]
    pub contents: Vec<ContentEntry>,
    /// Per-format relation-field overrides keyed by package format
    /// ("deb"/"rpm"/"arch"). See [`FormatOverrides`].
    #[serde(default)]
    pub overrides: HashMap<String, FormatOverrides>,
    /// Maintainer scripts. See [`Scripts`].
    #[serde(default)]
    pub scripts: Scripts,
    /// Package signing. See [`SignatureConfig`].
    #[serde(default)]
    pub signature: SignatureConfig,
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
    /// Debian epoch (e.g. "1"), for the rare case upstream renumbers
    /// versions downward. Prefixed onto the control file's Version field
    /// as `<epoch>:<version>`; never part of the .deb filename itself
    /// (Debian policy excludes it there since `:` isn't filename-safe).
    #[serde(default)]
    pub epoch: String,
    /// Package format plugin to use: "deb" (default) or "rpm".
    #[serde(default = "default_package_format")]
    pub package_format: String,
    /// Source provider plugin for auto-discovery: "github" (default) or "gitlab".
    #[serde(default = "default_source")]
    #[serde(alias = "source_provider")]
    pub source: String,
    /// Optional GitLab host for self-hosted instances (e.g. "gitlab.example.com").
    /// Only used when `source = "gitlab"`.
    #[serde(default)]
    pub gitlab_host: Option<String>,
    /// Optional Gitea host for self-hosted instances (e.g. "gitea.example.com").
    #[serde(default)]
    pub gitea_host: Option<String>,
    /// Optional Forgejo host for self-hosted instances (e.g. "codeberg.org").
    #[serde(default)]
    pub forgejo_host: Option<String>,
    /// Optional Bitbucket host for self-hosted (e.g. "bitbucket.example.com").
    /// Only used when `source = "bitbucket"`.
    #[serde(default)]
    pub bitbucket_host: Option<String>,
    /// Optional Gerrit host (e.g. "gerrit.example.com" or
    /// "review.gerrithub.io"). Only used when `source = "gerrit"`.
    /// Mirrors `gitlab_host`/`gitea_host`/etc.; without it, the
    /// `GERRIT_HOST` env var is consulted.
    #[serde(default)]
    pub gerrit_host: Option<String>,

    // ---- Legacy debian-multiarch-builder keys ---------------------------
    // The bash action's templates and zero-config wizard emitted these key
    // names; `deny_unknown_fields` would hard-reject every config written
    // against it, so they are parsed and folded into their modern lpt
    // equivalents by [`PackageConfig::apply_legacy_compat`]. Modern keys win
    // whenever both are set.
    //
    /// Legacy alias of `description`.
    #[serde(default)]
    pub summary: String,
    /// Legacy alias of `license_spdx`.
    #[serde(default)]
    pub license: String,
    /// Recorded as an extra `Vendor:` control field (the action never read
    /// this back either, but accepting it keeps its templates loadable).
    #[serde(default)]
    pub vendor: String,
    /// Legacy list form of `depends:` ("dependencies: [libc6, ...]").
    #[serde(default)]
    pub dependencies: Vec<String>,
    /// Legacy single-pattern asset name with `{version}`/`{arch}`/
    /// `{package_name}` placeholders. When no modern `architectures:` block
    /// is present it is expanded into per-arch release patterns using
    /// `architecture_map` (falling back to the Debian arch names).
    #[serde(default)]
    pub download_pattern: String,
    /// Maps Debian architecture -> upstream release-artifact name for
    /// `download_pattern` expansion ({arch} substitution).
    #[serde(default)]
    pub architecture_map: HashMap<String, String>,
    /// Per-architecture distribution allow-list overriding the built-in
    /// support matrix (`distribution_arch_overrides: { riscv64: {
    /// distributions: [trixie, forky, sid] } }`). Documented upstream but
    /// implemented only here.
    #[serde(default)]
    pub distribution_arch_overrides: HashMap<String, DistributionArchOverride>,
    /// Default max concurrent builds when --max-parallel is not passed
    /// (action's `parallel_builds.architectures.max_concurrent` /
    /// template-level `max_parallel`).
    #[serde(default)]
    pub max_parallel: usize,
    /// `parallel_builds: false` pins the build to a single worker
    /// regardless of --max-parallel auto-tuning.
    #[serde(default)]
    pub parallel_builds: Option<bool>,
}

/// An entry of `distribution_arch_overrides`: the distributions an
/// architecture is allowed to build for, replacing the built-in matrix for
/// that architecture entirely.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DistributionArchOverride {
    #[serde(default)]
    pub distributions: Vec<String>,
}

fn default_package_format() -> String {
    "deb".to_string()
}

fn default_source() -> String {
    "github".to_string()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchConfig {
    /// Exact asset filename, with optional `{version}` placeholder.
    #[serde(default)]
    pub release_pattern: String,
}

/// `architectures:` accepts two shapes:
/// - a plain list (`[amd64, arm64, armhf]`) restricting auto-discovery to a
///   named subset, with no pinned patterns;
/// - a map (`{ amd64: { release_pattern: "..." } }`) pinning exact assets.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ArchSpec {
    List(Vec<String>),
    Map(HashMap<String, ArchConfig>),
}

impl Default for ArchSpec {
    fn default() -> Self {
        ArchSpec::Map(HashMap::new())
    }
}

impl ArchSpec {
    pub fn is_empty(&self) -> bool {
        match self {
            ArchSpec::List(v) => v.is_empty(),
            ArchSpec::Map(m) => m.is_empty(),
        }
    }

    /// Architecture names in this spec, in either form.
    pub fn names(&self) -> Vec<String> {
        match self {
            ArchSpec::List(v) => v.clone(),
            ArchSpec::Map(m) => m.keys().cloned().collect(),
        }
    }

    /// Pinned patterns, if this is the map form (empty otherwise -- a plain
    /// list only restricts which architectures auto-discovery considers).
    pub fn patterns(&self) -> HashMap<String, ArchConfig> {
        match self {
            ArchSpec::List(_) => HashMap::new(),
            ArchSpec::Map(m) => m.clone(),
        }
    }

    /// Record a discovered pattern for `arch` (used by zero-config
    /// auto-discovery to build a config from scratch). Converts a list form
    /// to a map first, though callers only ever do this on a fresh default.
    pub fn set_pattern(&mut self, arch: String, acfg: ArchConfig) {
        if !matches!(self, ArchSpec::Map(_)) {
            *self = ArchSpec::Map(HashMap::new());
        }
        if let ArchSpec::Map(m) = self {
            m.insert(arch, acfg);
        }
    }
}

impl PackageConfig {
    /// Load and parse a package.yaml file.
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read config file '{}'", path.display()))?;
        Self::parse_str(&text).with_context(|| "failed to parse package.yaml")
    }

    /// Parse from a string, applying legacy debian-multiarch-builder key
    /// compatibility, then structural validation. Split out so callers
    /// holding YAML in memory get the identical pipeline.
    pub fn parse_str(text: &str) -> Result<Self> {
        let mut config: PackageConfig =
            serde_yaml::from_str(text).with_context(|| "failed to parse package.yaml")?;
        config.apply_legacy_compat();
        config.validate()?;
        Ok(config)
    }

    /// Fold legacy debian-multiarch-builder keys into their modern
    /// equivalents. Modern fields win whenever both are set. Pure (no I/O,
    /// no env), so it is unit-testable.
    pub fn apply_legacy_compat(&mut self) {
        if self.description.is_empty() {
            self.description = std::mem::take(&mut self.summary);
        }
        if self.license_spdx.is_empty() {
            self.license_spdx = std::mem::take(&mut self.license);
        }
        if !self.vendor.trim().is_empty() && !self.fields.contains_key("Vendor") {
            self.fields
                .insert("Vendor".to_string(), self.vendor.trim().to_string());
        }
        if self.depends.is_empty() && !self.dependencies.is_empty() {
            self.depends = self.dependencies.join(", ");
        }

        // Expand a legacy download_pattern into per-arch release patterns,
        // but only when no modern architectures block exists — with one
        // present, it wins outright and the pattern is ignored (the action
        // never consumed download_pattern at runtime either).
        if self.architectures.is_empty() && !self.download_pattern.is_empty() {
            let arch_names: Vec<String> = if !self.architecture_map.is_empty() {
                let mut names: Vec<String> = self.architecture_map.keys().cloned().collect();
                names.sort();
                names
            } else if self.download_pattern.contains("{arch}") {
                lpt_lib::constants::DEFAULT_ARCHITECTURES
                    .iter()
                    .map(|s| s.to_string())
                    .collect()
            } else {
                // No {arch} placeholder and no map (e.g. hugo's
                // single-asset sample): leave auto-discovery to decide at
                // build time, exactly as the action did.
                Vec::new()
            };
            if !arch_names.is_empty() {
                let base = self
                    .download_pattern
                    .replace("{package_name}", self.package_name.trim());
                for arch in arch_names {
                    let upstream = self
                        .architecture_map
                        .get(&arch)
                        .map(|s| s.as_str())
                        .unwrap_or(arch.as_str());
                    let pattern = base.replace("{arch}", upstream);
                    self.architectures.set_pattern(
                        arch.clone(),
                        ArchConfig {
                            release_pattern: pattern,
                        },
                    );
                }
            }
        }
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
        if let ArchSpec::List(names) = &self.architectures {
            if names.iter().any(|n| n.trim().is_empty()) {
                bail!("architectures list must not contain empty entries");
            }
        }
        if !self.package_format.trim().is_empty() {
            match self.package_format.trim().to_ascii_lowercase().as_str() {
                "deb" | "rpm" | "arch" => {}
                other => bail!("unsupported package_format '{other}' (expected deb, rpm, or arch)"),
            }
        }
        if !self.source.trim().is_empty() {
            match self.source.trim().to_ascii_lowercase().as_str() {
                "github" | "github-sync" | "gitlab" | "gitea" | "forgejo" | "bitbucket" | "gerrit" => {}
                other => bail!(
                    "unsupported source '{other}' (expected github, github-sync, gitlab, gitea, forgejo, bitbucket, or gerrit)"
                ),
            }
        }
        if !self.compression.trim().is_empty() {
            let c = self.compression.trim().to_ascii_lowercase();
            // Accept "gzip", "xz", "zstd", "none" optionally with ":level"
            // suffix like nfpm. Base names are checked here; the level is
            // range-checked per algorithm by `normalize_compression` (the
            // same parser the archiver uses), so a typo fails validation
            // instead of mid-build.
            let base = c.split(':').next().unwrap_or("").trim();
            match base {
                "gzip" | "gz" | "xz" | "zstd" | "none" => {}
                other => {
                    bail!("unsupported compression '{other}' (expected gzip, xz, zstd, or none)")
                }
            }
            if let Some(level) = c.split_once(':').map(|(_, l)| l.trim()) {
                if !level.is_empty() && level.parse::<u32>().is_err() {
                    bail!("invalid compression level '{level}' (expected an integer)");
                }
            }
        }
        for key in self.overrides.keys() {
            let k = key.trim().to_ascii_lowercase();
            if !matches!(k.as_str(), "deb" | "rpm" | "arch") {
                bail!("unsupported overrides format '{key}' (expected deb, rpm, or arch)");
            }
        }
        for entry in &self.contents {
            if !entry.dst.starts_with('/') {
                bail!(
                    "contents entry dst must be an absolute path starting with '/', got '{}'",
                    entry.dst
                );
            }
            if entry.dst.contains("..") {
                bail!(
                    "contents entry dst must not contain '..' components: '{}'",
                    entry.dst
                );
            }
            match entry.kind.as_str() {
                "" | "file" => {
                    if entry.src.trim().is_empty() {
                        bail!("contents entry requires src for type '{}'", display_kind(&entry.kind));
                    }
                }
                "config" | "config|noreplace" | "config|missingok" | "tree" | "symlink" => {
                    if entry.src.trim().is_empty() {
                        bail!("contents entry requires src for type '{}'", display_kind(&entry.kind));
                    }
                }
                // dir: no src needed; ghost: RPM-only, staged as a no-op.
                "dir" | "ghost" => {}
                other => bail!(
                    "unsupported contents type '{other}' (expected file, config, config|noreplace, config|missingok, tree, symlink, dir, or ghost)"
                ),
            }
        }
        for (arch, o) in &self.distribution_arch_overrides {
            if arch.trim().is_empty() {
                bail!("distribution_arch_overrides must not use an empty architecture key");
            }
            if o.distributions.is_empty() || o.distributions.iter().any(|d| d.trim().is_empty()) {
                bail!(
                    "distribution_arch_overrides entry '{arch}' must list at least one non-empty distribution"
                );
            }
        }
        Ok(())
    }

    /// Effective source provider, normalized to lowercase ("github" or "gitlab").
    pub fn effective_source(&self) -> String {
        if self.source.trim().is_empty() {
            "github".to_string()
        } else {
            self.source.trim().to_ascii_lowercase()
        }
    }

    /// Effective package format, normalized to lowercase ("deb", "rpm", or "arch").
    pub fn effective_package_format(&self) -> String {
        if self.package_format.trim().is_empty() {
            "deb".to_string()
        } else {
            self.package_format.trim().to_ascii_lowercase()
        }
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
    /// generic maintainer identity). When `maintainer:` is unset, the
    /// `LPT_MAINTAINER` env var wins over the built-in latest-debs
    /// default, so downstream users of lpt can attribute packages to
    /// themselves without editing every package.yaml.
    pub fn effective_maintainer(&self) -> String {
        if self.maintainer.is_empty() {
            resolve_default_maintainer(std::env::var(lpt_lib::constants::MAINTAINER_ENV_VAR).ok())
        } else {
            self.maintainer.clone()
        }
    }

    /// Whether the config pins release patterns explicitly (deterministic)
    /// as opposed to relying on auto-discovery. A plain `architectures:`
    /// list restricts the subset considered but pins nothing.
    pub fn has_manual_patterns(&self) -> bool {
        self.architectures
            .patterns()
            .values()
            .any(|a| !a.release_pattern.is_empty())
    }

    /// Resolve the effective distributions for an explicit format, applying
    /// built-in rules: empty -> all supported suites for that format (used
    /// when --format overrides the config file).
    pub fn effective_distributions_for(&self, format: &str) -> Vec<String> {
        if !self.debian_distributions.is_empty() {
            return self.debian_distributions.clone();
        }
        let defaults: &[&str] = match format.to_ascii_lowercase().as_str() {
            "rpm" => lpt_lib::constants::DEFAULT_RPM_DISTRIBUTIONS,
            "arch" => lpt_lib::constants::DEFAULT_ARCH_DISTRIBUTIONS,
            _ => lpt_lib::constants::DEFAULT_DEBIAN_DISTRIBUTIONS,
        };
        defaults.iter().map(|s| s.to_string()).collect()
    }

    /// Resolve the effective architectures to build.
    pub fn effective_architectures(&self) -> Vec<String> {
        if self.architectures.is_empty() {
            DEFAULT_ARCHITECTURES
                .iter()
                .map(|s| s.to_string())
                .collect()
        } else {
            self.architectures.names()
        }
    }

    /// Effective Section (defaults to "utils").
    pub fn effective_section(&self) -> String {
        if self.section.trim().is_empty() {
            lpt_lib::constants::DEFAULT_SECTION.to_string()
        } else {
            self.section.trim().to_string()
        }
    }

    /// Effective Priority (defaults to "optional").
    pub fn effective_priority(&self) -> String {
        if self.priority.trim().is_empty() {
            lpt_lib::constants::DEFAULT_PRIORITY.to_string()
        } else {
            self.priority.trim().to_string()
        }
    }

    /// Effective compression (defaults to "gzip").
    pub fn effective_compression(&self) -> String {
        if self.compression.trim().is_empty() {
            "gzip".to_string()
        } else {
            let c = self.compression.trim().to_ascii_lowercase();
            let base = c.split(':').next().unwrap_or("gzip").trim();
            match base {
                "gz" => "gzip".to_string(),
                other => other.to_string(),
            }
        }
    }

    /// Relation fields for a specific package format, after applying
    /// `overrides.<format>`. An override present for a field replaces the
    /// top-level value for that format (including with an empty string to
    /// clear it); absent overrides fall through to the top-level value.
    pub fn effective_relations(&self, format: &str) -> lpt_lib::pkgmeta::Relations {
        let mut r = lpt_lib::pkgmeta::Relations {
            depends: self.depends.clone(),
            recommends: self.recommends.clone(),
            suggests: self.suggests.clone(),
            conflicts: self.conflicts.clone(),
            replaces: self.replaces.clone(),
            provides: self.provides.clone(),
            breaks: self.breaks.clone(),
            predepends: self.predepends.clone(),
        };
        if let Some(o) = self.overrides.get(&format.trim().to_ascii_lowercase()) {
            if let Some(v) = &o.depends {
                r.depends = v.clone();
            }
            if let Some(v) = &o.recommends {
                r.recommends = v.clone();
            }
            if let Some(v) = &o.suggests {
                r.suggests = v.clone();
            }
            if let Some(v) = &o.conflicts {
                r.conflicts = v.clone();
            }
            if let Some(v) = &o.replaces {
                r.replaces = v.clone();
            }
            if let Some(v) = &o.provides {
                r.provides = v.clone();
            }
            if let Some(v) = &o.breaks {
                r.breaks = v.clone();
            }
            if let Some(v) = &o.predepends {
                r.predepends = v.clone();
            }
        }
        r
    }

    /// Effective signing key file: CLI flag wins over package.yaml.
    pub fn effective_sign_key(&self, cli_key: Option<&Path>) -> Option<std::path::PathBuf> {
        if let Some(k) = cli_key {
            if !k.as_os_str().is_empty() {
                return Some(k.to_path_buf());
            }
        }
        if self.signature.key_file.trim().is_empty() {
            None
        } else {
            Some(std::path::PathBuf::from(self.signature.key_file.trim()))
        }
    }

    /// Effective signing key id: CLI flag wins over package.yaml.
    pub fn effective_sign_key_id(&self, cli_id: Option<&str>) -> String {
        if let Some(id) = cli_id {
            if !id.trim().is_empty() {
                return id.trim().to_string();
            }
        }
        self.signature.key_id.trim().to_string()
    }

    /// Whether an architecture is supported in a given Debian distribution.
    /// A `distribution_arch_overrides` entry for the architecture replaces
    /// the built-in matrix entirely; otherwise the built-in rules mirror
    /// the action's `is_arch_supported_for_dist` (data/system.yaml).
    pub fn arch_supported_for_dist(&self, arch: &str, dist: &str) -> bool {
        if let Some(o) = self.distribution_arch_overrides.get(arch) {
            return o.distributions.iter().any(|d| d.trim() == dist);
        }
        if UNIVERSAL_ARCHS.contains(&arch) {
            return true;
        }
        // Distribution-restricted architectures: i386/armel lose support after
        // Trixie; riscv64/loong64 only gain it in newer suites.
        match arch {
            "i386" | "armel" => matches!(dist, "bullseye" | "bookworm" | "trixie"),
            "riscv64" => matches!(dist, "trixie" | "forky" | "sid"),
            "loong64" => matches!(dist, "forky" | "sid"),
            _ => false,
        }
    }
}

/// Drop suites whose LTS support has ended, mirroring the action's
/// `filter_expired_distributions`: once Debian stops supporting a suite,
/// packages stop being built for it even when a package.yaml explicitly
/// lists it. Suites without a recorded end date always pass. `today` is
/// injectable for deterministic tests; production callers pass `None`.
pub fn filter_expired_distributions(
    dists: &[String],
    today: Option<jiff::civil::Date>,
) -> Vec<String> {
    let today = today.unwrap_or_else(|| {
        jiff::Timestamp::now()
            .to_zoned(jiff::tz::TimeZone::UTC)
            .date()
    });
    let mut kept = Vec::with_capacity(dists.len());
    for dist in dists {
        let expired = lpt_lib::constants::DISTRIBUTION_LTS_ENDS
            .iter()
            .find(|(d, _)| *d == dist.as_str())
            .and_then(|(_, ends)| ends.parse::<jiff::civil::Date>().ok())
            .is_some_and(|ends| ends < today);
        if expired {
            println!("⚠️  Skipping {dist}: Debian LTS support for this suite has ended");
        } else {
            kept.push(dist.clone());
        }
    }
    kept
}

pub use lpt_lib::constants::DEFAULT_ARCHITECTURES;
pub use lpt_lib::constants::UNIVERSAL_ARCHS;

/// Human-readable name for a contents entry type (empty -> "file").
fn display_kind(kind: &str) -> &str {
    if kind.is_empty() {
        "file"
    } else {
        kind
    }
}

/// Maintainer precedence when `maintainer:` is unset: `$LPT_MAINTAINER`
/// (non-empty), else the built-in latest-debs default. Split out as a pure
/// function so the precedence is testable without mutating process env.
pub fn resolve_default_maintainer(env_value: Option<String>) -> String {
    env_value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| lpt_lib::constants::DEFAULT_MAINTAINER.to_string())
}

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
