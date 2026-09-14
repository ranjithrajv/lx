# Packager Architecture

Part of the [lx docs](../README.md).

**Date:** 2026-08-24 (updated 2026-08-26: +5 Source; 2026-09-11: +BuildSystem; 2026-09-11: +RegistrySource; 2026-09-11: +5 RegistrySource; 2026-09-11: clarified ForgeSource vs RegistrySource; 2026-09-14: +5 Packagers/Sources/BuildSystems; 2026-09-14: +4 cross-cutting dimensions; 2026-09-14: +IndexSource; 2026-09-14: merged RepoIndexer + IndexSource into PackageIndex)
**Status:** Implemented — 8 independent plugin dimensions:
* **5× Packager:** `deb` + `rpm` + `arch` + `apk` + `ipk`
* **9× ForgeSource:** `github` + `gitlab` + `gitea` + `forgejo` + `bitbucket` + `gerrit` + `gitee` + `sourceforge` + `custom`
* **7× BuildSystem:** `cmake` + `cargo` + `go` + `meson` + `autotools` + `make` + `custom`
* **11× RegistrySource:** `npm` + `python` + `gem` + `cargo` + `go` + `hex` + `dart` + `nuget` + `maven` + `composer` + `cpan`
* **6× ArtifactFormat:** `tar.gz` + `tar.xz` + `tar.zst` + `tar` + `zip` + `raw`
* **6× Signer:** `gpg-detach` + `rpm-pgp` + `deb-debsign` + `apk-rsa` + `msix-p7x` + `pkg-xar`
* **5× DependencyMapper:** `debian` + `rpm` + `pacman` + `alpine` + `openwrt`
* **9× PackageIndex:** 5 write (`apt` + `opkg` + `pacman` + `apk` + `rpm`) + 4 read (`lx-community` + `aur` + `repology` + `custom`)

**Per-plugin detail:** the trait definitions and the individual packagers,
build systems, sources, and cross-cutting plugins live in
[plugin-catalog.md](plugin-catalog.md). This page is the dimensions
overview and wiring.

`lx` builds Linux packages from many kinds of upstream. The original
implementation only produced Debian `.deb`s from GitHub. To support RPM/Arch,
multiple forges, source compilation, language package managers, and
pluggable package indexes without forking the core pipeline, the build (and
the `lx index` fan-out) was refactored into a **format-agnostic core +
multi-dimensional pluggable system**.

This matches `goreleaser/nfpm`'s Go `Packager` → `deb/`, `rpm/`, `apk/` at
repo root, but adds more dimensions:

* **Packager plugins** (`lib/plugins/{deb,rpm,arch,apk,ipk}.rs` + `lib/{deb,rpm,arch,apk,ipk}archive.rs`) → produce installable artifact
* **ForgeSource plugins** (`lib/plugins/forge/{github,gitlab,gitea,forgejo,bitbucket,gerrit,gitee,sourceforge,custom}.rs` + `lib/{github,gitlab,gitea,forgejo,bitbucket,gerrit,gitee,sourceforge}.rs`) → discover **what** is available (releases, assets, versions); `custom` is an explicit-only direct-URL provider
* **BuildSystem plugins** (`lib/plugins/build_system/{cmake,cargo,go,meson,autotools,make,custom}.rs`) → compile source tree into install tree
* **RegistrySource plugins** (`lib/plugins/registry/{npm,python,gem,cargo,go,hex,dart,nuget,maven,composer,cpan}.rs`) → fetch **specific files** from language package registries

All are stateless, registered statically, and share the same
explicit-registry pattern.

## How the dimensions relate

```
 ┌─────────────────────────────────────────────────────────────────────────┐
 │                          lx build                                       │
 │                                                                         │
 │  ┌──────────────┐    ┌──────────────┐    ┌──────────────┐               │
 │  │  RegistrySource  │ or │   Source     │ →  │  BuildSystem │ (source mode) │
 │  │  (registry)   │    │  (forge)     │    │  (compile)   │               │
 │  └──────┬───────┘    └──────┬───────┘    └──────┬───────┘               │
 │         │                   │                   │                        │
 │         └─────────┬─────────┴─────────┬─────────┘                        │
 │                   ▼                   ▼                                  │
 │            ┌─────────────┐    ┌─────────────┐                           │
 │            │ payload dir │    │  (download  │                           │
 │            │ (extracted) │    │   assets)   │                           │
 │            └──────┬──────┘    └──────┬──────┘                           │
 │                   └─────────┬─────────┘                                  │
 │                             ▼                                            │
 │                      ┌──────────────┐                                    │
 │                      │   Package    │                                    │
 │                      │ (deb/rpm/arch)│                                   │
 │                      └──────────────┘                                    │
 └─────────────────────────────────────────────────────────────────────────┘
```

**Source vs RegistrySource — why two plugin types?**

Both answer "where does the package come from?" but they operate at
different levels of abstraction and return different things:

| | ForgeSource | RegistrySource |
|---|---|---|
| **Question it answers** | "What releases/assets exist?" | "Give me these files." |
| **Returns** | `Release` — metadata + asset list (per-architecture) | `RegistryPayload` — a local directory of files |
| **Trait surface** | 11 methods (discovery: `parse_url`, `latest_release`, `repo_license`, `releases`, …) | 4 methods (fetch: `name`, `required_tools`, `fetch`) |
| **Selection** | `--source` flag / zero-config URL sniffing | `registry_source:` field (explicit) |
| **Package identity** | `github_repo` = "owner/repo" | `github_repo` = bare package name |
| **Versioning** | Tags, semver, "latest release" | Registry version constraints |
| **Multi-arch** | Yes — each asset maps to an architecture | No — single payload (one arch at a time) |

They are **not merged** because:

1. **Different return types.** `Source` returns metadata (the pipeline
   still needs to download and match assets per-arch). `RegistrySource`
   returns files ready to package. Merging forces an enum wrapper and
   a `match` at every call site.
2. **Different trait surfaces.** A merged trait has 11 methods where 7
   are `unimplemented!()` for inputs, or 4 methods where 7 forge
   capabilities are lost. Neither is acceptable.
3. **Different selection mechanisms.** Sources are discovered via URL
   sniffing (`parse_any_url`); inputs are explicitly configured.
4. **Different identity semantics.** The same `github_repo` field means
   "owner/repo" for sources and "package name" for inputs — merging
   forces implicit interpretation.

The pipeline routes them separately: `Source` → download assets → package;
`RegistrySource` → `fetch()` → `run_local()` (skip asset download, the
files are already on disk). Both feed into the same `Package` plugins.

---

## 1. Plugin Types

`lib/plugins/mod.rs`, `lib/plugins/forge/mod.rs`, `lib/plugins/build_system/mod.rs`, `lib/plugins/registry/mod.rs`

The four core dimensions, each with its own trait and registry (the
cross-cutting dimensions added later are in §12):

```rust
// Identity — lib/plugins/plugin.rs; supertrait of every dimension
pub trait Plugin: Send + Sync {
    fn name(&self) -> &'static str;        // canonical id, e.g. "deb" | "github"
    fn description(&self) -> &'static str; // help / error text
}

// Packager — lib/plugins/mod.rs
pub trait Packager: Plugin {
    fn file_extension(&self) -> &'static str;    // "deb" | "rpm" | "pkg.tar.zst"
    fn default_distributions(&self) -> &'static [&'static str];
    fn arch_supported_for_dist(&self, arch: &str, dist: &str) -> bool;
    fn build(&self, ctx: &BuildContext) -> Result<PathBuf>;
    // Provided — a format overrides only what differs:
    fn artifact_glob(&self, package: &str) -> String;             // summary filename pattern
    fn supports_source_build(&self) -> bool;                      // default false
    fn archive_staged_tree(&self, ctx: &BuildContext) -> Result<PathBuf>;
    fn generate_source_package(&self, out: &Path, pkg: &Pkg) -> Result<()>;
}

// ForgeSource — lib/plugins/forge/mod.rs
pub trait ForgeSource: Plugin {
    fn parse_url(&self, url: &str) -> Option<String>;
    fn latest_release(&self, repo: &str, token: Option<&str>, cache_dir: Option<&Path>) -> Result<Release>;
    fn release_by_tag(&self, repo: &str, tag: &str, token: Option<&str>, cache_dir: Option<&Path>) -> Result<Release>;
    fn releases(&self, repo: &str, per_page: u8, token: Option<&str>, cache_dir: Option<&Path>) -> Result<Vec<ReleaseMeta>>;
    fn repo_license(&self, repo: &str, token: Option<&str>, cache_dir: Option<&Path>) -> Result<Option<RepoLicense>>;
    fn repo_root(&self, repo: &str, token: Option<&str>, cache_dir: Option<&Path>) -> Result<Vec<String>>;
    fn repo_file_text(&self, repo: &str, path: &str, token: Option<&str>, cache_dir: Option<&Path>) -> Result<Option<String>>;
    fn raw_get(&self, url: &str, token: Option<&str>) -> Result<Box<dyn Read + Send>>;
}

// BuildSystem — lib/plugins/build_system/mod.rs
pub trait BuildSystem: Plugin {
    fn recognize(&self, src_dir: &Path) -> bool;  // default false; auto-detect from source tree
    fn build(&self, cfg: &PackageConfig, src_dir: &Path, workdir: &Path) -> Result<PathBuf>;
    fn required_tools(&self) -> Vec<&'static str>; // default none; checked before build
}

// RegistrySource — lib/plugins/registry/mod.rs
pub trait RegistrySource: Plugin {
    fn required_tools(&self) -> Vec<&'static str>; // checked before fetch (e.g. ["npm"])
    fn fetch(&self, package: &str, version: &str, cfg: &PackageConfig) -> Result<RegistryPayload>;
}

pub struct RegistryPayload {
    pub files_dir: PathBuf,          // extracted files ready for packaging
    pub resolved_version: String,    // actual version fetched
    pub description: String,         // detected from package metadata
}
```

Each dimension has its own registry (`all_plugins()`, `all_forge_sources()`, `all_build_systems()`, `all_registry_sources()`), lookup (`get_*`), and auto-detection where applicable (`detect_build_system()` for BuildSystem, `parse_any_url()` for Source). Adding a new plugin is implementing the trait + one registration line — no core pipeline edits.

## 9. Wiring

* `lib/config.rs` `source: String` (`#[serde(default)]` `"github"`, `alias = "source_provider"`) validated **through the plugin registry** (the accepted values are `github|gitlab|gitea|forgejo|bitbucket|gerrit|gitee|sourceforge|custom`), `gitlab_host/gitea_host/forgejo_host/bitbucket_host/gerrit_host/gitee_host: Option<String>`. `build_system: String` (`#[serde(default)]` `"cmake"`) validated through `all_build_systems()` (`cmake|cargo|go|meson|autotools|make|custom`). `registry_source: String` (`#[serde(default)]` `""`, `alias = "registry_source"`) (`npm|python|gem|cargo|go|hex|dart|nuget|maven|composer|cpan`) resolved by `get_registry_source` at build time.
* `lib/build.rs` resolves `effective_source` → `get_forge_source`, prints `source: …`, sets `cfg.source` + provider host env vars, `resolve_source_token` (provider-specific `*_TOKEN` env > `cli --token`), then all release/license/download/sidecar paths use `source.*(repo, token, cache_dir)`. When `registry_source` is non-empty, resolves the input plugin → `fetch()` → routes through `run_local()` with the fetched payload.
* `lib/sourcebuild.rs` resolves the build system plugin: explicit `build_system:` → `get_build_system()`, else `detect_build_system(src_dir)`, else error. Runs `prebuild_steps`, checks `required_tools()`, then calls `build_sys.build()`.
* `lib/discovery.rs` (used by `lx init --from`), `lib/validate.rs`, `lib/scandeps.rs`, `lib/wizard.rs` all go through `get_forge_source`.
* `lib/summary.rs` glob switches `*_*.deb` / `-*.rpm` / `-*.pkg.tar.*` / `-*.apk` / `*_*.ipk` and JSON includes `package_format`.
* `lib/source.rs` generates deb source packages (`generate`), RPM source packages (`generate_rpm`), and Arch `PKGBUILD`s (`generate_arch`).
* `lib/checksum.rs` generic over `RawGetter`; `lib/debs.rs` `download`/`verify_sidecar_or_require_flag` also generic (`&dyn RawGetter`).

## 10. Adding a New Packager

**Package (e.g. `apk`):**

1. `lib/<format>archive.rs` – `pub fn build(root, name, version, …) -> Result<()>` doing deterministic archive (see `archarchive.rs` for pattern).
2. `lib/plugins/<format>.rs` – `impl Packager` + `plugin_identity!`; `build` calls `stage_install_tree` + `lx_lib::<format>archive::build`. Override `default_distributions`, and `artifact_glob` when the filename is not `{package}_*.{extension}` (rpm/arch/apk/osxpkg do).
3. Register in `lib/plugins/mod.rs` `all_packagers()` — one line. Nothing in `lib/config.rs` or `lib/summary.rs`: format validation, default distributions, and the summary glob all resolve through the registry. Override `supports_source_build` + `archive_staged_tree` + `generate_source_package` only if the format can wrap a source-build tree.
4. Add tests in `lib/plugins/mod.rs` (registry + `build_valid_archives` magic check).

**Source (e.g. `myforge`):**

1. `lib/<provider>.rs` – `struct Client { http, base_url, token, cache_dir }` with `latest_release/release_by_tag/releases/repo_license/repo_root/repo_file_text/raw_get` (see `lib/gitlab.rs` for REST mapping and `lib/gitea.rs` for Gitea/Forgejo, `lib/bitbucket.rs` for downloads-as-release).
2. Add `DEFAULT_<PROVIDER>_HOST/API_URL` to `lib/constants.rs` and `homepage_for_<provider>()` helper.
3. Implement `lx_lib::checksum::RawGetter for Client`.
4. `lib/plugins/forge/<provider>.rs` – `impl ForgeSource` delegating to `lib::<provider>::Client` (see `lib/plugins/forge/gitlab.rs`/`gitea.rs`).
5. Register in `lib/plugins/forge/mod.rs` + `parse_url` for `https://{host}/owner/repo`.
6. Add `*_host: Option<String>` to `lib/config.rs` + `resolve_source_token` in `lib/build.rs` + `host` env handling (the `source:` value itself validates through the registry).
7. Tests: `lib/<provider>::tests` (release mapping, `parse_*_url`), `lib/plugins/forge/tests` (`parse_any_url` dispatch, registry gains the new name).

**BuildSystem (e.g. `meson`):**

1. `lib/plugins/build_system/<name>.rs` – `impl BuildSystem` + `plugin_identity!`, with `recognize(src_dir)` (detect the project file, e.g. `meson.build`), `build(cfg, src_dir, workdir)` (compile + stage DESTDIR-style tree), and `required_tools()` (e.g. `["meson", "ninja"]`).
2. Register in `lib/plugins/build_system/mod.rs` `all_build_systems()`. `lib/config.rs` validation resolves the name through this registry, so there is no list to edit.
3. Tests: `lib/plugins/build_system/tests` (registry gains the new name, `recognize` detection, `build` produces expected tree).

No core pipeline changes – `sourcebuild.rs` is build-system-agnostic; it only calls `build_sys.build()`.

**RegistrySource (e.g. `cpan`):**

1. `lib/plugins/registry/<name>.rs` – `impl RegistrySource` + `plugin_identity!`, with `required_tools()` (e.g. `["cpan"]`) and `fetch(package, version, cfg)` which downloads/extracts the package and returns an `InputPayload { files_dir, resolved_version, description }`.
2. Register in `lib/plugins/registry/mod.rs` `all_registry_sources()`. `lib/config.rs` does not hold a `registry_source` list.
3. Tests: `lib/plugins/registry/tests` (registry gains the new name, `fetch` produces expected payload).

No core pipeline changes – `build.rs` routes any non-empty `registry_source` through `run_local()` after `fetch()`.

## 11. Relation to nfpm

| nfpm | lx |
|---|---|
| Go `Packager` interface, 6 impls, `contents:` DSL + `overrides` | Rust `Packager` + `ForgeSource` + `BuildSystem` + `RegistrySource` + `ArtifactFormat` + `Signer` + `DependencyMapper` + `PackageIndex` traits — 5 + 9 + 7 + 11 + 6 + 4 + 5 + 9 impls, shared `stage_install_tree` + `match_assets` + `RawGetter` |
| General-purpose: you supply files; `arch`/`overrides` per packager | Opinionated: we fetch releases (github/gitlab/gitea/forgejo/bitbucket/gerrit), verify, auto-install ancillaries; `source` selects provider, `package_format` selects packager, `build_system` selects compiler |
| Signing per format (`deb.signature/rpm.signature`) | `Signer` plugins: detached `gpg-detach` for any format + embedded `rpm-pgp`/`deb-debsign`/`apk-rsa`/`msix-p7x` + post-build `pkg-xar`; `check_sidecar` provider-agnostic via `RawGetter` |
| No source-build concept | `build_mode: source` with pluggable build systems (cmake, cargo, go, meson, autotools, make, custom) — compiles on host, wraps per-suite |

See `docs/decisions/2026-08-20-nfpm-adoptions.md` and `README.md` for the `nfpm`-inspired `relations`/`ancillaries`/`epoch` already shared.

