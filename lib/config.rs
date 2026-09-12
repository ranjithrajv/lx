// SPDX-License-Identifier: GPL-3.0-or-later

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
    /// Optional packager filter (`deb` / `rpm` / `arch`). Empty means the
    /// entry applies to every format (nfpm's `packager:` field).
    #[serde(default)]
    pub packager: String,
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

/// Maintainer scripts (`scripts:`), mirroring nfpm's script names across
/// formats. Paths are in the build environment.
///
/// - **deb**: `preinstall`→`DEBIAN/preinst`, `postinstall`→`DEBIAN/postinst`,
///   `preremove`→`DEBIAN/prerm`, `postremove`→`DEBIAN/postrm`,
///   `preupgrade_script`→`DEBIAN/preupgrade`,
///   `postupgrade_script`→`DEBIAN/postupgrade` (all mode 0755).
/// - **rpm**: `preinstall`→`%pre`, `postinstall`→`%post`,
///   `preremove`→`%preun`, `postremove`→`%postun`, `pretrans`→`%pretrans`,
///   `posttrans`→`%posttrans`, `verify`→`%verify`. `pretrans`/`posttrans`
///   also serve as upgrade hooks (`%pretrans`/`%posttrans` run around the
///   whole transaction).
/// - **arch**: `preupgrade`→`pre_upgrade()`, `postupgrade`→`post_upgrade()`
///   inside a `.INSTALL` file.
///
/// Fields not relevant to a given format are silently ignored by that
/// format's plugin.
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
    /// RPM `%pretrans` scriptlet (transaction-level, runs before any
    /// package in the transaction is installed). Ignored by deb/arch.
    #[serde(default)]
    pub pretrans: String,
    /// RPM `%posttrans` scriptlet (runs after the entire transaction
    /// completes). Ignored by deb/arch.
    #[serde(default)]
    pub posttrans: String,
    /// RPM `%verify` scriptlet (runs when `rpm -V` verifies the package).
    /// Ignored by deb/arch.
    #[serde(default)]
    pub verify: String,
    /// Arch `pre_upgrade()` hook (inside `.INSTALL`). Ignored by deb/rpm.
    #[serde(default)]
    pub preupgrade: String,
    /// Arch `post_upgrade()` hook (inside `.INSTALL`). Ignored by deb/rpm.
    #[serde(default)]
    pub postupgrade: String,
    /// Pre-upgrade script path (deb: DEBIAN/preupgrade; rpm: %pretrans;
    /// arch: pre_upgrade()). Runs before the package is upgraded.
    #[serde(default)]
    pub preupgrade_script: String,
    /// Post-upgrade script path (deb: DEBIAN/postupgrade; rpm: %posttrans;
    /// arch: post_upgrade()). Runs after the package is upgraded.
    #[serde(default)]
    pub postupgrade_script: String,
}

/// Debian-specific configuration (`deb:`), mirroring nfpm's `deb.` block.
/// Debconf templates/config, maintainer triggers, and `rules` are all
/// deb-only concepts; rpm/arch ignore this block entirely.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DebConfig {
    /// Path to a `debian/rules` Makefile. Copied into the package as
    /// `DEBIAN/rules` (mode 0755). Ignored by rpm/arch.
    #[serde(default)]
    pub rules: String,
    /// Path to a debconf `templates` file. Copied into the package as
    /// `DEBIAN/templates` (mode 0644). Ignored by rpm/arch.
    #[serde(default)]
    pub templates: String,
    /// Path to a debconf `config` maintainer script. Copied into the
    /// package as `DEBIAN/config` (mode 0755). Ignored by rpm/arch.
    #[serde(default)]
    pub config: String,
    /// Triggers this package registers interest in (deb `interest`).
    /// Each entry becomes a `interest <name>` line in `DEBIAN/triggers`.
    /// Ignored by rpm/arch.
    #[serde(default)]
    pub triggers_interest: Vec<String>,
    /// Triggers this package activates (deb `activate`). Each entry
    /// becomes an `activate <name>` line in `DEBIAN/triggers`. Ignored
    /// by rpm/arch.
    #[serde(default)]
    pub triggers_activate: Vec<String>,
    /// Triggers this package registers interest in, waiting for the
    /// trigger to fire before continuing (deb `interest_await`).
    /// Ignored by rpm/arch.
    #[serde(default)]
    pub triggers_interest_await: Vec<String>,
    /// Triggers this package registers interest in, without waiting
    /// (deb `interest_noawait`). Ignored by rpm/arch.
    #[serde(default)]
    pub triggers_interest_noawait: Vec<String>,
    /// Triggers this package activates, waiting for the trigger to
    /// fire before continuing (deb `activate_await`). Ignored by
    /// rpm/arch.
    #[serde(default)]
    pub triggers_activate_await: Vec<String>,
    /// Triggers this package activates, without waiting (deb
    /// `activate_noawait`). Ignored by rpm/arch.
    #[serde(default)]
    pub triggers_activate_noawait: Vec<String>,
}

/// RPM-specific configuration (`rpm:`), mirroring fpm's `--rpm-trigger-*`
/// flags. RPM triggers are scripts that fire when another package is
/// installed or removed. The four trigger types map to fpm's:
/// - `pre_install` → `--rpm-trigger-before-install` (%triggerprein)
/// - `post_install` → `--rpm-trigger-after-install` (%triggerin)
/// - `pre_uninstall` → `--rpm-trigger-before-uninstall` (%triggerun)
/// - `post_uninstall` → `--rpm-trigger-after-target-uninstall` (%triggerpostun)
///
/// Each entry is a `package: script_path` pair. The package name is the
/// trigger condition (fire when this package is installed/removed), and
/// the script path is the script to run. Mirrors fpm's
/// `--rpm-trigger-after-install 'PACKAGE: FILEPATH'` syntax.
///
/// Note: the underlying `rpm` crate has limited trigger support. Trigger
/// dependencies (the package condition) are emitted, but trigger scripts
/// may require manual rpmbuild or a future rpm-crate version for full
/// support. Ignored by deb/arch.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RpmConfig {
    /// Trigger before install: `%triggerprein`. Each entry is
    /// `"package: script_path"`.
    #[serde(default)]
    pub trigger_pre_install: Vec<String>,
    /// Trigger after install: `%triggerin`. Each entry is
    /// `"package: script_path"`.
    #[serde(default)]
    pub trigger_post_install: Vec<String>,
    /// Trigger before uninstall: `%triggerun`. Each entry is
    /// `"package: script_path"`.
    #[serde(default)]
    pub trigger_pre_uninstall: Vec<String>,
    /// Trigger after uninstall: `%triggerpostun`. Each entry is
    /// `"package: script_path"`.
    #[serde(default)]
    pub trigger_post_uninstall: Vec<String>,
    /// RPM payload compression algorithm. One of: `gzip`, `xz`, `lzma`,
    /// `zstd`, `none`. Default: `gzip` (rpm crate default).
    /// Mirrors fpm's `--rpm-compression` and nfpm's `rpm.compression`.
    #[serde(default)]
    pub compression: String,
    /// Auto-generate `Provides:` for every file/shared library the package
    /// installs (rpm's `--auto-provides`). Default: true.
    #[serde(default = "crate::config::true_default")]
    pub auto_provides: bool,
    /// Auto-generate `Requires:` from shared-library dependencies detected
    /// in the payload (rpm's `--auto-requires`). Default: true.
    #[serde(default = "crate::config::true_default")]
    pub auto_requires: bool,
    /// rpmbuild-style macro definitions (e.g. `_unpackaged_files_terminate_build 0`).
    /// Each entry is a `"KEY VALUE"` string written to a macro file that
    /// rpmbuild reads. Best-effort: applied as builder lead macros when the
    /// rpm crate supports it, else documented for rpmbuild fallback.
    #[serde(default)]
    pub defines: Vec<String>,
}

/// Package signing configuration (`signature:`).
///
/// - **rpm**: the armored secret key is loaded natively and the signature
///   is embedded in the `.rpm` header (verifiable via `rpm -K`).
/// - **deb**:
///   - `method: detach` (default) — `gpg --detach-sign` produces a
///     `<file>.sig` next to each built `.deb`.
///   - `method: debsign` — armored detach-sign of the ar payload is
///     embedded as `_gpgorigin` inside the `.deb` (debsigs / nfpm).
///
/// Passphrase resolution (both formats): `$LX_SIGN_PASSPHRASE`, falling
/// back to `$NFPM_PASSPHRASE` for nfpm parity.
///
/// String fields expand `${VAR}` / `${VAR:-default}` at parse time.
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
    /// Signing method for `.deb`: `"detach"` (default) or `"debsign"`.
    /// Ignored for rpm/arch.
    #[serde(default)]
    pub method: String,
    /// Debsigner role for `method: debsign`: `"origin"` (default),
    /// `"maint"`, or `"archive"`. Becomes the ar member `_gpg{type}`
    /// (nfpm / debsigs parity). Ignored for detach / rpm / arch.
    #[serde(default, rename = "type")]
    pub sign_type: String,
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
    /// Ubuntu suites to target natively (opt-in; e.g. [noble, jammy]).
    /// Merged with `debian_distributions` for deb builds, mirroring the
    /// action's separate `ubuntu_distributions` key.
    #[serde(default)]
    pub ubuntu_distributions: Vec<String>,
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
    /// Debian arch variant (e.g. "amd64v3") for optimized builds. Appended
    /// to the `Architecture` field as `amd64v3`. Mirrors nfpm's
    /// `deb.arch_variant`. Ignored by rpm/arch.
    #[serde(default)]
    pub arch_variant: String,
    /// Version schema for parsing the upstream version string. "semver"
    /// (default) normalizes semver-like versions (strips `v` prefix,
    /// handles prerelease/metadata); "none" uses the version as-is.
    /// Mirrors nfpm's `version_schema`.
    #[serde(default = "default_version_schema")]
    pub version_schema: String,
    /// Umask applied to files without an explicit `mode` set, mirroring
    /// nfpm's `umask`. Expressed in octal (e.g. "0o002"). When set, files
    /// added to the package have their mode masked by this value. Applies
    /// to all formats.
    #[serde(default)]
    pub umask: String,
    /// Packager string identifying the organization that packaged the
    /// software (as opposed to the author). For RPM this maps to the
    /// `packager` header tag; for deb it becomes a `Packager:` control
    /// field. Mirrors nfpm's `rpm.packager`. Falls back to `maintainer`
    /// when unset.
    #[serde(default)]
    pub packager: String,
    /// Disables globbing for `contents:` entries. When true, `src`
    /// patterns are treated as literal paths. Mirrors nfpm's
    /// `disable_globbing`.
    #[serde(default)]
    pub disable_globbing: bool,
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
    /// Debian-specific configuration (debconf, triggers, rules). See [`DebConfig`].
    #[serde(default)]
    pub deb: DebConfig,
    /// RPM-specific configuration (triggers). See [`RpmConfig`].
    #[serde(default)]
    pub rpm: RpmConfig,
    /// Package signing. See [`SignatureConfig`].
    #[serde(default)]
    pub signature: SignatureConfig,
    /// Local payload path (archive or directory) for `--local` builds that
    /// skip the upstream release download. Existence is checked at build
    /// time, not parse time. Env-expanded at parse (`${VAR}` / ${VAR:-default}`).
    #[serde(default)]
    pub local_payload: String,
    /// Install prefix inside the package for `--from-dir`/`--from-file`
    /// builds (fpm-style "you supply files" mode). When non-empty, files are
    /// staged under this absolute path (e.g. "/usr/local/bin") instead of the
    /// default `/usr/bin` flat-mode layout. Must start with '/'.
    #[serde(default)]
    pub prefix: String,
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
    /// Registry source plugin for language package managers. When set, `lx`
    /// fetches the package from a language registry instead of a forge
    /// release. Values: "npm", "python", "gem", "cargo", "go", "nuget",
    /// "maven", "composer". The package to fetch is taken from `github_repo` (or
    /// `package_name` if github_repo is empty), and the version from `version`.
    #[serde(default)]
    #[serde(alias = "registry_source")]
    pub registry_source: String,
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
    // against it, so they are parsed and folded into their modern lx
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
    /// Build mode: "binary" (default) repacks an upstream release asset;
    /// "source" fetches the upstream source tag and compiles it (cmake)
    /// on the host, then wraps the install tree per suite. Mirrors the
    /// bash action's `build_mode:` (see build_mode: source template).
    #[serde(default = "default_build_mode")]
    pub build_mode: String,
    /// Source-mode build system. Only "cmake" is supported (bash parity:
    /// `build_mode: source` errors on anything else).
    #[serde(default = "default_build_system")]
    pub build_system: String,
    /// Source tarball URL root for `build_mode: source` (e.g. a Forgejo
    /// instance). Empty means GitHub `<repo>/archive/refs/tags/<ref>.tar.gz`.
    #[serde(default)]
    pub upstream_url: String,
    /// Source-mode tag to fetch. Empty means the resolved `--version`.
    #[serde(default)]
    pub upstream_ref: String,
    /// Produce a musl-static binary — no glibc dependency, runs on any
    /// Linux regardless of distro age. For source builds, adjusts each
    /// build system plugin to compile for musl (cargo: `--target
    /// x86_64-unknown-linux-musl`; go: `CGO_ENABLED=0`; cmake:
    /// `musl-gcc`). For binary repacks, prefers musl release assets
    /// (e.g. `*-linux-musl*`) over glibc variants.
    #[serde(default)]
    pub musl: bool,
    /// Extra host packages the compile needs (installed via the host
    /// package manager by the caller/CI; lx itself never apt-gets).
    #[serde(default)]
    pub build_depends: Vec<String>,
    /// Enable ERB-like templating for maintainer scripts (preinstall,
    /// postinstall, preremove, postremove, preupgrade_script,
    /// postupgrade_script). When true, script files are processed through
    /// the template engine before being staged, replacing `<%= key %>`
    /// expressions with package values.
    ///
    /// Available template variables:
    /// - `<%= name %>` — package name
    /// - `<%= version %>` — package version
    /// - `<%= maintainer %>` — maintainer string
    /// - `<%= description %>` — package description
    /// - `<%= homepage %>` — homepage URL
    /// - `<%= license %>` — SPDX license
    /// - `<%= arch %>` — target architecture
    /// - `<%= dist %>` — target distribution
    /// - `<%= iteration %>` — build version/revision
    /// - `<%= epoch %>` — epoch
    /// - `<%= vendor %>` — vendor
    /// - `<%= packager %>` — packager
    /// - `<%= prefix %>` — install prefix
    #[serde(default)]
    pub template_scripts: bool,
    /// Extra flags appended to the cmake configure step.
    #[serde(default)]
    pub cmake_flags: Vec<String>,
    /// Shell steps run in the extracted source dir after unpack and before
    /// the build-system configure (patches, codegen, `go generate`, ...).
    /// makedeb-`prepare()` spirit, kept as an ordered list rather than a
    /// full second language. Applies to every `build_mode: source` build.
    #[serde(default)]
    pub prebuild_steps: Vec<String>,
    /// Custom build commands replacing the cmake configure+build when
    /// `build_system: custom` (run in the source dir, in order). For Go,
    /// Rust, Node, Make and other ecosystems cmake cannot express.
    #[serde(default)]
    pub build_commands: Vec<String>,
    /// Custom install commands replacing `cmake --install` when
    /// `build_system: custom` (run in the source dir with `$DESTDIR` set
    /// to the staging tree; install the FHS tree there).
    #[serde(default)]
    pub install_commands: Vec<String>,
    /// Source-mode suites to build. Empty means every configured
    /// (debian + ubuntu) distribution.
    #[serde(default)]
    pub build_suites: Vec<String>,
    /// Source-mode suites to skip (applied after `build_suites`/defaults).
    #[serde(default)]
    pub skip_suites: Vec<String>,
    /// Per-suite apt overlay for compile cells
    /// (`build_depends_suites: { trixie: { from: forky, packages: [...] } }`),
    /// kept for config compat. Only meaningful with containerized builds;
    /// lx compiles on the host, so these are validated but not applied.
    /// Use `build_depends` for host packages instead.
    #[serde(default)]
    pub build_depends_suites: HashMap<String, BuildDependsSuite>,
    /// Apt source lines keyed by suite name (companion to
    /// `build_depends_suites`). Same host-build caveat as above.
    #[serde(default)]
    pub build_apt_sources: HashMap<String, Vec<String>>,
}

/// One entry of `build_depends_suites`: install `packages` from suite
/// `from` with `apt-get install -t`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildDependsSuite {
    #[serde(default)]
    pub from: String,
    #[serde(default)]
    pub packages: Vec<String>,
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

fn default_version_schema() -> String {
    "semver".to_string()
}

fn default_source() -> String {
    "github".to_string()
}

fn default_build_mode() -> String {
    "binary".to_string()
}

fn default_build_system() -> String {
    "cmake".to_string()
}

/// Serde default for boolean fields that should default to `true`.
pub fn true_default() -> bool {
    true
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

    /// Parse from a string, applying env-var expansion, legacy
    /// debian-multiarch-builder key compatibility, then structural
    /// validation. Split out so callers holding YAML in memory get the
    /// identical pipeline.
    pub fn parse_str(text: &str) -> Result<Self> {
        let expanded = expand_env_vars(text)?;
        let mut config: PackageConfig =
            serde_yaml::from_str(&expanded).with_context(|| "failed to parse package.yaml")?;
        config.apply_legacy_compat();
        config.validate()?;
        Ok(config)
    }

    /// Applies a thin delta package.yaml over this already-loaded config:
    /// top-level keys present in the overlay replace the base's, everything
    /// else is left untouched. Lets an org keep one shared base
    /// `package.yaml` and fork only what differs (version pin, `depends`,
    /// `release_pattern`) instead of duplicating the whole template.
    pub fn apply_overlay(&mut self, overlay_path: &Path) -> Result<()> {
        let text = std::fs::read_to_string(overlay_path)
            .with_context(|| format!("failed to read overlay '{}'", overlay_path.display()))?;
        let expanded = expand_env_vars(&text)?;
        let overlay_value: serde_yaml::Value = serde_yaml::from_str(&expanded)
            .with_context(|| format!("failed to parse overlay '{}'", overlay_path.display()))?;
        let base_value = serde_yaml::to_value(&*self)
            .with_context(|| "failed to re-serialize base config for overlay merge")?;
        let merged = merge_yaml_top_level(base_value, overlay_value);
        *self = serde_yaml::from_value(merged).with_context(|| {
            format!(
                "failed to apply overlay '{}' onto package.yaml",
                overlay_path.display()
            )
        })?;
        self.apply_legacy_compat();
        self.validate()
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
                lx_lib::constants::DEFAULT_ARCHITECTURES
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
        self.validate_prefix()?;

        self.validate_shared()
    }

    /// Validation for "you supply files" builds (`--from-dir`/`--from-file`):
    /// same as [`validate`] but `github_repo` is not required (there is no
    /// forge to fetch from).
    pub fn validate_for_local(&self) -> Result<()> {
        if self.package_name.trim().is_empty() {
            bail!("package_name is required (set package_name in package.yaml or pass --package-name)");
        }
        self.validate_prefix()?;

        self.validate_shared()
    }

    /// Shared validation between [`validate`] and [`validate_for_local`].
    fn validate_shared(&self) -> Result<()> {
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
                "github" | "github-sync" | "gitlab" | "gitea" | "forgejo" | "bitbucket" | "gerrit" | "custom" => {}
                other => bail!(
                    "unsupported source '{other}' (expected github, github-sync, gitlab, gitea, forgejo, bitbucket, gerrit, or custom)"
                ),
            }
            if self.source.trim().eq_ignore_ascii_case("custom")
                && self.upstream_url.trim().is_empty()
            {
                bail!("source 'custom' requires upstream_url in package.yaml (URL template with {{version}}/{{arch}} placeholders)");
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
            if !entry.packager.trim().is_empty() {
                match entry.packager.trim().to_ascii_lowercase().as_str() {
                    "deb" | "rpm" | "arch" => {}
                    other => bail!(
                        "unsupported contents packager '{other}' (expected deb, rpm, or arch)"
                    ),
                }
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
        if !self.signature.method.trim().is_empty() {
            match self.signature.method.trim().to_ascii_lowercase().as_str() {
                "detach" | "debsign" => {}
                other => {
                    bail!("unsupported signature.method '{other}' (expected detach or debsign)")
                }
            }
        }
        if !self.signature.sign_type.trim().is_empty() {
            match self
                .signature
                .sign_type
                .trim()
                .to_ascii_lowercase()
                .as_str()
            {
                "origin" | "maint" | "archive" => {}
                other => bail!(
                    "unsupported signature.type '{other}' (expected origin, maint, or archive)"
                ),
            }
        }
        match self.build_mode.trim().to_ascii_lowercase().as_str() {
            "" | "binary" | "source" => {}
            other => bail!("unsupported build_mode '{other}' (expected binary or source)"),
        }
        if self.is_source_mode() {
            match self.build_system.trim().to_ascii_lowercase().as_str() {
                "" | "cmake" | "cargo" | "go" | "meson" | "custom" => {}
                other => bail!(
                    "unsupported build_system '{other}' (expected one of: cmake, cargo, go, meson, custom)"
                ),
            }
            if self.effective_build_system() == "custom" && self.install_commands.is_empty() {
                bail!("build_system: custom requires install_commands (install the FHS tree into $DESTDIR)");
            }
            // Source mode has no release assets to auto-discover, so an
            // explicit architectures list is required (bash parity).
            if self.architectures.is_empty() {
                bail!(
                    "build_mode: source requires an explicit architectures: list in package.yaml"
                );
            }
        }
        // Deb triggers must not contain empty entries.
        for t in &self.deb.triggers_interest {
            if t.trim().is_empty() {
                bail!("deb.triggers_interest must not contain empty entries");
            }
        }
        for t in &self.deb.triggers_activate {
            if t.trim().is_empty() {
                bail!("deb.triggers_activate must not contain empty entries");
            }
        }
        // Deb trigger await/noawait variants must not contain empty entries.
        for t in &self.deb.triggers_interest_await {
            if t.trim().is_empty() {
                bail!("deb.triggers_interest_await must not contain empty entries");
            }
        }
        for t in &self.deb.triggers_interest_noawait {
            if t.trim().is_empty() {
                bail!("deb.triggers_interest_noawait must not contain empty entries");
            }
        }
        for t in &self.deb.triggers_activate_await {
            if t.trim().is_empty() {
                bail!("deb.triggers_activate_await must not contain empty entries");
            }
        }
        for t in &self.deb.triggers_activate_noawait {
            if t.trim().is_empty() {
                bail!("deb.triggers_activate_noawait must not contain empty entries");
            }
        }
        // Version schema must be "semver" or "none".
        match self.version_schema.trim().to_ascii_lowercase().as_str() {
            "" | "semver" | "none" => {}
            other => bail!("unsupported version_schema '{other}' (expected semver or none)"),
        }
        // Umask must be a valid octal value if set.
        if !self.umask.trim().is_empty() && self.effective_umask().is_none() {
            bail!(
                "invalid umask '{}' (expected octal, e.g. 0o002)",
                self.umask.trim()
            );
        }
        // RPM trigger entries must contain a ':' separator (package: script).
        let trigger_lists = [
            &self.rpm.trigger_pre_install,
            &self.rpm.trigger_post_install,
            &self.rpm.trigger_pre_uninstall,
            &self.rpm.trigger_post_uninstall,
        ];
        for list in trigger_lists {
            for entry in list {
                let entry = entry.trim();
                if entry.is_empty() {
                    continue;
                }
                if !entry.contains(':') {
                    bail!(
                        "invalid rpm trigger entry '{}' (expected 'package: script_path')",
                        entry
                    );
                }
            }
        }
        Ok(())
    }

    /// Validate the `prefix:` field (used by `--from-dir`/`--from-file`).
    fn validate_prefix(&self) -> Result<()> {
        if self.prefix.is_empty() {
            return Ok(());
        }
        if !self.prefix.starts_with('/') {
            bail!(
                "prefix must be an absolute path starting with '/', got '{}'",
                self.prefix
            );
        }
        if self.prefix.contains("..") {
            bail!("prefix must not contain '..' components: '{}'", self.prefix);
        }
        Ok(())
    }

    /// Effective registry source for language package managers ("npm", "python",
    /// "gem", "cargo", "nuget", "maven", "composer"), or empty when not using
    /// a registry source plugin.
    pub fn effective_registry_source(&self) -> String {
        let s = self.registry_source.trim();
        if s.is_empty() {
            String::new()
        } else {
            s.to_ascii_lowercase()
        }
    }

    /// Effective source provider, normalized to lowercase ("github" or "gitlab").
    pub fn effective_forge_source(&self) -> String {
        if self.source.trim().is_empty() {
            "github".to_string()
        } else {
            self.source.trim().to_ascii_lowercase()
        }
    }

    /// Effective build mode, normalized to lowercase ("binary" or "source").
    pub fn effective_build_mode(&self) -> String {
        if self.build_mode.trim().is_empty() {
            "binary".to_string()
        } else {
            self.build_mode.trim().to_ascii_lowercase()
        }
    }

    /// True for `build_mode: source` (compile upstream on the host).
    pub fn is_source_mode(&self) -> bool {
        self.effective_build_mode() == "source"
    }

    /// Effective build system, normalized to lowercase ("cmake" or "custom").
    pub fn effective_build_system(&self) -> String {
        if self.build_system.trim().is_empty() {
            "cmake".to_string()
        } else {
            self.build_system.trim().to_ascii_lowercase()
        }
    }

    /// Source-mode suites: explicit `build_suites` minus `skip_suites`
    /// when set, otherwise the configured distributions minus
    /// `skip_suites` (bash `source_build_suites` parity).
    pub fn source_suites(&self, configured: &[String]) -> Vec<String> {
        let base: Vec<String> = if self.build_suites.is_empty() {
            configured.to_vec()
        } else {
            self.build_suites.clone()
        };
        base.into_iter()
            .filter(|s| !self.skip_suites.iter().any(|skip| skip.trim() == s.trim()))
            .collect()
    }

    /// Oldest suite in a same-family list per the bash
    /// SOURCE_BUILD_DEBIAN_ORDER / SOURCE_BUILD_UBUNTU_ORDER (compile on
    /// the oldest suite so symbols are the intersection of every suite).
    /// Unknown names fall back to the first entry.
    pub fn oldest_suite(suites: &[String]) -> Option<String> {
        const DEBIAN_ORDER: &[&str] = &["bullseye", "bookworm", "trixie", "forky", "sid"];
        const UBUNTU_ORDER: &[&str] = &[
            "jammy", "noble", "oracular", "plucky", "questing", "resolute",
        ];
        let order: &[&str] = match suites.first().map(|s| s.as_str()) {
            Some(s) if lx_lib::constants::is_ubuntu_dist(s) => UBUNTU_ORDER,
            _ => DEBIAN_ORDER,
        };
        for o in order {
            if let Some(s) = suites.iter().find(|s| s.trim() == *o) {
                return Some(s.clone());
            }
        }
        suites.first().cloned()
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
    /// `LX_MAINTAINER` env var wins over the built-in latest-debs
    /// default, so downstream users of lx can attribute packages to
    /// themselves without editing every package.yaml.
    pub fn effective_maintainer(&self) -> String {
        if self.maintainer.is_empty() {
            resolve_default_maintainer(std::env::var(lx_lib::constants::MAINTAINER_ENV_VAR).ok())
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
        if !self.debian_distributions.is_empty() || !self.ubuntu_distributions.is_empty() {
            let mut out = self.debian_distributions.clone();
            out.extend(self.ubuntu_distributions.clone());
            return out;
        }
        let defaults: &[&str] = match format.to_ascii_lowercase().as_str() {
            "rpm" => lx_lib::constants::DEFAULT_RPM_DISTRIBUTIONS,
            "arch" => lx_lib::constants::DEFAULT_ARCH_DISTRIBUTIONS,
            _ => lx_lib::constants::DEFAULT_DEBIAN_DISTRIBUTIONS,
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
            lx_lib::constants::DEFAULT_SECTION.to_string()
        } else {
            self.section.trim().to_string()
        }
    }

    /// Effective Priority (defaults to "optional").
    pub fn effective_priority(&self) -> String {
        if self.priority.trim().is_empty() {
            lx_lib::constants::DEFAULT_PRIORITY.to_string()
        } else {
            self.priority.trim().to_string()
        }
    }

    /// Effective arch variant (empty when unset). The deb plugin appends
    /// this to the architecture field.
    pub fn effective_arch_variant(&self) -> String {
        self.arch_variant.trim().to_string()
    }

    /// Effective version schema, normalized to lowercase ("semver" or "none").
    pub fn effective_version_schema(&self) -> String {
        let s = self.version_schema.trim().to_ascii_lowercase();
        if s.is_empty() {
            "semver".to_string()
        } else {
            s
        }
    }

    /// Effective umask as a u32 (octal), or None when unset. Mirrors nfpm's
    /// umask semantics: when set, files without an explicit mode have
    /// their mode masked by this value.
    pub fn effective_umask(&self) -> Option<u32> {
        let s = self.umask.trim();
        if s.is_empty() {
            return None;
        }
        // Accept "0o002" or "002" octal forms.
        let stripped = if s.starts_with("0o") || s.starts_with("0O") {
            &s[2..]
        } else {
            s
        };
        u32::from_str_radix(stripped, 8).ok()
    }

    /// Effective packager (falls back to maintainer when unset). Split out
    /// as a pure function so the precedence is testable.
    pub fn effective_packager(&self) -> String {
        if self.packager.trim().is_empty() {
            self.effective_maintainer()
        } else {
            self.packager.trim().to_string()
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
    pub fn effective_relations(&self, format: &str) -> lx_lib::pkgmeta::Relations {
        let mut r = lx_lib::pkgmeta::Relations {
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

    /// Effective deb signing method: CLI flag wins over package.yaml.
    /// Empty / unset → `"detach"` (post-build `.sig`).
    pub fn effective_sign_method(&self, cli_method: Option<&str>) -> String {
        if let Some(m) = cli_method {
            if !m.trim().is_empty() {
                return m.trim().to_ascii_lowercase();
            }
        }
        let m = self.signature.method.trim();
        if m.is_empty() {
            "detach".to_string()
        } else {
            m.to_ascii_lowercase()
        }
    }

    /// Effective debsign role (`signature.type`): empty → `"origin"`.
    pub fn effective_sign_type(&self) -> String {
        let t = self.signature.sign_type.trim();
        if t.is_empty() {
            "origin".to_string()
        } else {
            t.to_ascii_lowercase()
        }
    }

    /// Whether an architecture is supported in a given Debian distribution.
    /// A `distribution_arch_overrides` entry for the architecture replaces
    /// the built-in matrix entirely; otherwise the built-in rules mirror
    /// the action's `is_arch_supported_for_dist` (data/system.yaml).
    pub fn arch_supported_for_dist(&self, arch: &str, dist: &str) -> bool {
        if let Some(o) = self.distribution_arch_overrides.get(arch) {
            return o.distributions.iter().any(|d| d.trim() == dist);
        }
        if lx_lib::constants::is_ubuntu_dist(dist) {
            return lx_lib::constants::ubuntu_archs(dist).contains(&arch);
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

/// Expand `${VAR}` and `${VAR:-default}` references in `text` using process
/// environment. Missing `VAR` without a `:-` default is an error. `$$`
/// becomes a literal `$`. Used by [`PackageConfig::parse_str`] so config
/// fields like `signature.key_file` / `local_payload` can reference CI
/// secrets without baking paths into the file.
/// Shallow overlay merge: overlay's top-level mapping keys replace the
/// base's wholesale (no recursive field-by-field merge inside e.g.
/// `architectures:`) -- "modern keys win", matching a `package.yaml`
/// override's usual intent of swapping one whole section at a time.
fn merge_yaml_top_level(base: serde_yaml::Value, overlay: serde_yaml::Value) -> serde_yaml::Value {
    match (base, overlay) {
        (serde_yaml::Value::Mapping(mut base_map), serde_yaml::Value::Mapping(overlay_map)) => {
            for (k, v) in overlay_map {
                base_map.insert(k, v);
            }
            serde_yaml::Value::Mapping(base_map)
        }
        (_, overlay) => overlay,
    }
}

pub fn expand_env_vars(text: &str) -> Result<String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(dollar) = rest.find('$') {
        out.push_str(&rest[..dollar]);
        rest = &rest[dollar..];
        if rest.starts_with("$$") {
            out.push('$');
            rest = &rest[2..];
            continue;
        }
        if let Some(inner) = rest.strip_prefix("${") {
            let Some(end) = inner.find('}') else {
                bail!("unclosed ${{...}} env reference");
            };
            let body = &inner[..end];
            let (name, default) = match body.split_once(":-") {
                Some((n, d)) => (n, Some(d)),
                None => (body, None),
            };
            if name.is_empty() || !is_env_name(name) {
                bail!("invalid env var name in '${{{body}}}'");
            }
            match std::env::var(name) {
                Ok(v) => out.push_str(&v),
                Err(_) => match default {
                    Some(d) => out.push_str(d),
                    None => bail!("environment variable '{name}' is not set"),
                },
            }
            rest = &inner[end + 1..];
            continue;
        }
        out.push('$');
        rest = &rest[1..];
    }
    out.push_str(rest);
    Ok(out)
}

fn is_env_name(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
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
        let expired = lx_lib::constants::DISTRIBUTION_LTS_ENDS
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

pub use lx_lib::constants::DEFAULT_ARCHITECTURES;
pub use lx_lib::constants::UNIVERSAL_ARCHS;

/// Human-readable name for a contents entry type (empty -> "file").
fn display_kind(kind: &str) -> &str {
    if kind.is_empty() {
        "file"
    } else {
        kind
    }
}

/// Maintainer precedence when `maintainer:` is unset: `$LX_MAINTAINER`
/// (non-empty), else the built-in latest-debs default. Split out as a pure
/// function so the precedence is testable without mutating process env.
pub fn resolve_default_maintainer(env_value: Option<String>) -> String {
    env_value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| lx_lib::constants::DEFAULT_MAINTAINER.to_string())
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
