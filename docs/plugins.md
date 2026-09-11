# Plugin Architecture

**Date:** 2026-08-24 (updated 2026-08-26: plugin types + 5 Source; 2026-09-11: + BuildSystem dimension)  
**Status:** Implemented — 3 dimensions:
* **3× Package:** `deb` + `rpm` + `arch`
* **6× Source:** `github` + `gitlab` + `gitea` + `forgejo` + `bitbucket` + `gerrit`
* **4× BuildSystem:** `cmake` + `cargo` + `go` + `custom`

`lx` builds Linux packages by repackaging release binaries or compiling source. The original implementation only produced Debian `.deb`s from GitHub. To support RPM/Arch, GitLab, and multiple build systems without forking the core pipeline, the build was refactored into a **format-agnostic core + three-dimensional pluggable system**.

This matches `goreleaser/nfpm`'s Go `Packager` → `deb/`, `rpm/`, `apk/` at repo root, but adds two more dimensions:

* **Package plugins** (`lib/plugins/{deb,rpm,arch}.rs` + `lib/{deb,rpm,arch}archive.rs`) → produce installable artifact
* **Source plugins** (`lib/plugins/source/{github,gitlab}.rs` + `lib/{github,gitlab}.rs`) → discover releases/assets
* **BuildSystem plugins** (`lib/plugins/build_system/{cmake,cargo,go,custom}.rs`) → compile source tree into install tree

All are stateless, registered statically, and share the same explicit-registry pattern.

---

## 1. Plugin Types

`lib/plugins/mod.rs`, `lib/plugins/source/mod.rs`, `lib/plugins/build_system/mod.rs`

Three independent plugin dimensions, each with its own trait and registry:

```rust
// Package — lib/plugins/mod.rs
pub trait Plugin: Send + Sync {
    fn name(&self) -> &'static str;              // "deb" | "rpm" | "arch"
    fn file_extension(&self) -> &'static str;    // "deb" | "rpm" | "pkg.tar.zst"
    fn description(&self) -> &'static str;
    fn default_distributions(&self) -> &'static [&'static str];
    fn arch_supported_for_dist(&self, arch: &str, dist: &str) -> bool;
    fn build(&self, ctx: &BuildContext) -> Result<PathBuf>;
}

// Source — lib/plugins/source/mod.rs
pub trait SourcePlugin: Send + Sync {
    fn name(&self) -> &'static str; // "github" | "gitlab" | …
    fn description(&self) -> &'static str;
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
pub trait BuildSystem: Send + Sync {
    fn name(&self) -> &'static str;              // "cmake" | "cargo" | "go" | "custom"
    fn description(&self) -> &'static str;
    fn recognize(&self, src_dir: &Path) -> bool; // auto-detect from source tree
    fn build(&self, cfg: &PackageConfig, src_dir: &Path, workdir: &Path) -> Result<PathBuf>;
    fn required_tools(&self) -> Vec<&'static str>; // checked before build
}
```

Each dimension has its own registry (`all_plugins()`, `all_source_plugins()`, `all_build_systems()`), lookup (`get_*`), and auto-detection where applicable (`detect_build_system()` for BuildSystem, `parse_any_url()` for Source). Adding a new plugin is implementing the trait + one registration line — no core pipeline edits.

## 2. Package Trait

`lib/plugins/mod.rs`

```rust
pub trait Plugin: Send + Sync {
    fn name(&self) -> &'static str;              // "deb" | "rpm" | "arch"
    fn file_extension(&self) -> &'static str;    // "deb" | "rpm" | "pkg.tar.zst"
    fn description(&self) -> &'static str;
    fn default_distributions(&self) -> &'static [&'static str];
    fn arch_supported_for_dist(&self, arch: &str, dist: &str) -> bool;
    fn build(&self, ctx: &BuildContext) -> Result<PathBuf>;
}
```

`BuildContext` (`lib/plugins/mod.rs`) carries everything a packager needs without coupling to global state:

```rust
pub struct BuildContext<'a> {
    cfg: &'a PackageConfig,          // package_name, source, github_repo, epoch, …
    job: &'a ResolvedJob,            // dist, arch, asset, tag, published_at
    binary_dir: &'a Path,            // extracted `binary_path`
    staging_root: &'a Path,          // empty dir the plugin populates
    license: Option<&'a RepoLicense>,
    debian_version: &'a str,         // stripped upstream version
    build_version: &'a str,          // "1"
    mtime: i64,                      // SOURCE_DATE_EPOCH or published_at
}
```

Plugins are **stateless** – one instance per format, shared across threads.

## 3. Source Trait (Auto-Discovery)

`lib/plugins/source/mod.rs`

```rust
pub trait SourcePlugin: Send + Sync {
    fn name(&self) -> &'static str; // "github" | "gitlab"
    fn description(&self) -> &'static str;
    fn parse_url(&self, url: &str) -> Option<String>; // https://… → "owner/repo"
    fn latest_release(&self, repo: &str, token: Option<&str>, cache_dir: Option<&Path>) -> Result<Release>;
    fn release_by_tag(&self, repo: &str, tag: &str, token: Option<&str>, cache_dir: Option<&Path>) -> Result<Release>;
    fn releases(&self, repo: &str, per_page: u8, token: Option<&str>, cache_dir: Option<&Path>) -> Result<Vec<ReleaseMeta>>;
    fn repo_license(&self, repo: &str, token: Option<&str>, cache_dir: Option<&Path>) -> Result<Option<RepoLicense>>;
    fn repo_root(&self, repo: &str, token: Option<&str>, cache_dir: Option<&Path>) -> Result<Vec<String>>;
    fn repo_file_text(&self, repo: &str, path: &str, token: Option<&str>, cache_dir: Option<&Path>) -> Result<Option<String>>;
    fn raw_get(&self, url: &str, token: Option<&str>) -> Result<Box<dyn Read + Send>>;
}
```

`Release`/`Asset`/`RepoLicense` are provider-agnostic structs from `lib/github.rs:319` (shared by both clients). `lib/checksum.rs:8` defines `RawGetter` trait (`raw_get → Box<dyn Read>`) implemented for both `GitHubClient` and `GitlabClient`, so `check_sidecar` (`lib/checksum.rs:108`) is now generic and works for either source.

Auto-discovery (`src/discovery.rs:36` `match_assets`, `src/discovery::config_from_release`) is **source-agnostic**: it only sees `&Release` (tag + assets) and maps filenames to Debian arches via segment-run matching, OS filtering, longest-alias-first. The provider only matters for *fetching* that `Release`.

## 4. Registry

`lib/plugins/mod.rs` (package), `lib/plugins/source/mod.rs` (source), `lib/plugins/build_system/mod.rs` (build system):

```rust
// Package
pub fn all_plugins() -> Vec<Box<dyn Plugin>> {
    vec![Box::new(deb::DebPlugin), Box::new(rpm::RpmPlugin), Box::new(arch::ArchPlugin)]
}
pub fn get_plugin(name: &str) -> Option<Box<dyn Plugin>> { /* case-insensitive */ }

// Source
pub fn all_source_plugins() -> Vec<Box<dyn SourcePlugin>> {
    vec![Box::new(github::GithubSourcePlugin), Box::new(gitlab::GitlabSourcePlugin), …]
}
pub fn get_source_plugin(name: &str) -> Option<Box<dyn SourcePlugin>> { /* case-insensitive */ }
pub fn parse_any_url(url: &str) -> Option<(String,String)> { // (source, repo)
    for p in all_source_plugins() { if let Some(repo)=p.parse_url(url) { return Some((p.name().into(), repo)) } }
    None
}

// BuildSystem
pub fn all_build_systems() -> Vec<Box<dyn BuildSystem>> {
    vec![Box::new(cmake::CmakeBuildSystem), Box::new(cargo::CargoBuildSystem),
         Box::new(go::GoBuildSystem), Box::new(custom::CustomBuildSystem)]
}
pub fn get_build_system(name: &str) -> Option<Box<dyn BuildSystem>> { /* case-insensitive */ }
pub fn detect_build_system(src_dir: &Path) -> Option<Box<dyn BuildSystem>> {
    all_build_systems().into_iter().find(|b| b.recognize(src_dir))
}
```

No `dlopen`, no feature flags. Adding a format = `impl Plugin` + one line in `all_plugins()`. Adding a source = `impl SourcePlugin` + one line in `all_source_plugins()`. Adding a build system = `impl BuildSystem` + one line in `all_build_systems()`.

Selection in `lib/build.rs` / `lib/discovery.rs` / `lib/validate.rs`:

```
--format flag  >  package.yaml `package_format`  >  default "deb"   → Plugin
--source flag  >  package.yaml `source`         >  default "github" → SourcePlugin
--plus zero-config URL host sniffing: `parse_any_url("https://gitlab.com/…") → ("gitlab","owner/repo")`
```

BuildSystem selection in `lib/sourcebuild.rs`:

```
build_system: in package.yaml  >  auto-detect from source tree  >  error if none recognized
```

Auto-detection checks `recognize()` in registry order: `CMakeLists.txt` → cmake, `Cargo.toml` → cargo, `go.mod` → go. `custom` never auto-detects (explicit-only).

`cfg.effective_package_format()` and `cfg.effective_source()` normalise; `cfg.effective_distributions_for(format)` picks per-plugin defaults.

## 5. Shared Staging

`stage_install_tree()` (`lib/plugins/mod.rs`) is format-agnostic and reused by all three package plugins:

* `bundle:false` → keep only ELF files in `/usr/bin`, auto-install `*.1(.gz)` → `/usr/share/man/manN/*.gz` and `LICENSE*|COPYING*|NOTICE*` → `/usr/share/doc/<pkg>/`.
* `bundle:true` → `copy_dir_recursive` whole `binary_path` tree to `/usr/lib/<pkg>/` and `symlink_elf_executables` into `/usr/bin` (preserves `$ORIGIN` RPATH, versioned `.so` symlinks).
* `prefix` set (`--from-dir`/`--from-file` mode) → dump all files flat under `<root><prefix>` with no ELF detection or ancillary staging (fpm-style "you supply files").
* `binary_rename` and empty-`/usr/bin` guard.

This preserves the nfpm-inspired ancillary handling (`docs/decisions/2026-08-20-nfpm-adoptions.md`) centrally.

## 6. Package Plugins

### deb — `lib/plugins/deb.rs` + `lib/debarchive.rs` *(3 total: deb + rpm + arch)*

*Extension* `deb`, *defaults* `bookworm/trixie/forky/sid`, *matrix* → `PackageConfig::arch_supported_for_dist()` (universal + distro-gated `i386/armel/riscv64/loong64`).

Stages via `stage_install_tree`, renders `DEBIAN/control` (`Section/Priority/Package/Version/Architecture/Maintainer/Homepage/Description` + `Relations` `lib/pkgmeta.rs:55`), `changelog.Debian.gz`/`copyright` via `lib/pkgmeta::render_*`, then `lx_lib::debarchive::build()` (`ar` with `debian-binary`+`control.tar.gz`+`data.tar.gz`, sorted walk, normalized `mtime/uid/gid`, deterministic `gzip`).

Lint via `lx_lib::lintian::run()`.

### rpm — `lib/plugins/rpm.rs` + `lib/rpmarchive.rs`

*Extension* `rpm`, *defaults* `fedora/el9/el8/opensuse`, permissive matrix.

Stages same tree, then `lx_lib::rpmarchive::build()` (`rpm = "0.16"` `PackageBuilder`): `name/version (stripped)/license/arch (mapped `amd64→x86_64`)` + `release = "{build_version}.{dist}"` (dot-normalised), `summary/description` from `effective_description`, `source_date(mtime)` for reproducibility, `with_file()` for each payload file (sorted) + dummy file for symlinks (`FileOptions::symlink`).

Valid RPM magic `ED AB EE DB`, tested with `rpm -qip` equivalent.

### arch — `lib/plugins/arch.rs` + `lib/archarchive.rs`

*Extension* `pkg.tar.zst`, *defaults* `arch` (rolling), permissive matrix.

Stages same tree, renders `.PKGINFO` (`pkgname/pkgver/pkgdesc/url/builddate/packager/size/arch/license`), `.MTREE` (`#mtree` + `time/mode/type/size/sha256digest` per entry), then `tar` + `zstd` level 19 (deterministic). Filename `name-version-release-arch.pkg.tar.zst` (e.g. `hello-1.0-1.arch-x86_64.pkg.tar.zst`), `zstd` magic `28 B5 2F FD`, verifiable `tar tz --use-compress-program=unzstd`.

## 7. BuildSystem Plugins (Compile Source)

`lib/plugins/build_system/{cmake,cargo,go,custom}.rs` — 4 total. Used by `build_mode: source` to compile upstream source into a DESTDIR-style install tree.

Each plugin implements `BuildSystem`:
* `name()` / `description()` — identity
* `recognize(src_dir)` — auto-detect from source tree (overridden; `custom` uses the default `false`)
* `build(cfg, src_dir, workdir)` → `PathBuf` — compile and stage the install tree
* `required_tools()` — host tools checked before building (default: none)

### cmake — `lib/plugins/build_system/cmake.rs`

*Name* `cmake`, detects `CMakeLists.txt`, requires `cmake` + `ninja`.

cmake `-S <src> -B <build> -G Ninja -DCMAKE_INSTALL_PREFIX=/usr -DCMAKE_BUILD_TYPE=Release` + `cmake_flags`, then `cmake --build`, then `cmake --install` with `DESTDIR=<stage>`. Produces a full FHS install tree.

### cargo — `lib/plugins/build_system/cargo.rs`

*Name* `cargo`, detects `Cargo.toml`, requires `cargo`.

`cargo install --path . --root <stage> --locked` — builds `--release` and installs binary(ies) to `<stage>/bin/`. Verifies binaries landed (errors on library crates with no binaries). Produces a ready-to-package FHS tree.

### go — `lib/plugins/build_system/go.rs`

*Name* `go`, detects `go.mod`, requires `go`.

`go build -trimpath -ldflags "-s -w" -o <stage>/bin/<package_name> .` — single binary build. `-trimpath` strips host paths (reproducibility), `-ldflags "-s -w"` strips debug info. Produces a minimal FHS tree (`bin/<name>`).

### custom — `lib/plugins/build_system/custom.rs`

*Name* `custom`, never auto-detects (explicit-only). Runs user-supplied `build_commands` then `install_commands` (with `$DESTDIR` set to the stage dir). Universal escape hatch for build systems without a dedicated plugin (meson, make, npm, python, …).

## 8. Source Plugins (Auto-Discovery)

6 total: `github` + `gitlab` + `gitea` + `forgejo` + `bitbucket` + `gerrit`.

### github — `lib/plugins/source/github.rs` + `lib/github.rs`

*Name* `github`, *description* `GitHub Releases (api.github.com / octocrab)`.

* Wraps `lx_lib::github::GitHubClient` (`octocrab` + `tokio` + 5-min JSON cache `api_cache_dir`).
* `parse_url` → `crate::build::parse_github_url` (`https://github.com/owner/repo(.git)?(/releases/…)?`).
* `latest_release` → `GET /repos/{owner}/{repo}/releases/latest`, `release_by_tag` → `GET /repos/{owner}/{repo}/releases/tags/{tag}`.
* `raw_get`/`releases`/`repo_license`/`repo_root`/`repo_file_text` map 1:1 to `GitHubClient` methods (dual-license `LICENSE-APACHE`+`LICENSE-MIT` detection uses `repo_root` + `repo_file_text`).
* Token: `resolve_source_token("github", cli_token)` prefers `cli --token` then `GITHUB_TOKEN` env.

### gitlab — `lib/plugins/source/gitlab.rs` + `lib/gitlab.rs`

*Name* `gitlab`, *description* `GitLab Releases (gitlab.com / self-hosted, API v4)`.

* Wraps `lx_lib::gitlab::GitlabClient` (blocking `reqwest`, same 5-min cache, `GITLAB_API_URL`/`GITLAB_HOST` env, default `https://gitlab.com/api/v4`).
* `parse_url` → `parse_gitlab_url` (`https://gitlab.com/owner/repo`, custom host via `GITLAB_HOST`).
* `latest_release` → `GET /projects/{%2F-encoded}/releases?per_page=1`, `release_by_tag` → `GET /projects/{id}/releases/{tag}`.
* Mapping: `GitlabReleaseRaw {tag_name, assets:{links[]}}` → `Release {assets: links→Asset}`. `published_at: released_at||created_at → jiff`.
* Token: `GITLAB_TOKEN` env > `cli --token`.

### gitea — `lib/plugins/source/gitea.rs` + `lib/gitea.rs`

*Name* `gitea`, `Gitea Releases (codeberg.org / self-hosted, API v1)`, default `https://codeberg.org/api/v1` (`DEFAULT_GITEA_API_URL`).

* Wraps `GiteaClient` (`GITEA_API_URL`/`GITEA_HOST` env, `GITEA_TOKEN`).
* `parse_url` → `parse_gitea_url` (`https://codeberg.org/owner/repo`).
* `latest_release` → `GET /repos/{owner}/{repo}/releases?limit=1`, `release_by_tag` → `GET /repos/{owner}/{repo}/releases/tags/{tag}`.
* Assets: `GiteaReleaseRaw {tag_name, assets[]}` → `Release`.

### forgejo — `lib/plugins/source/forgejo.rs` + `lib/forgejo.rs`

*Name* `forgejo`, `Forgejo Releases (codeberg.org / self-hosted, Gitea-compatible)`, default `https://codeberg.org/api/v1` (`DEFAULT_FORGEJO_API_URL`).

* Wraps `ForgejoClient` (same `GiteaClient` logic but `FORGEJO_TOKEN`/`FORGEJO_HOST` env, falls back to `GITEA_TOKEN`/`GITEA_HOST` for compat).
* `parse_url` → `parse_forgejo_url` (`https://codeberg.org/owner/repo`, also `forgejo`-containing hosts).
* Same release mapping as Gitea.

### bitbucket — `lib/plugins/source/bitbucket.rs` + `lib/bitbucket.rs`

*Name* `bitbucket`, `Bitbucket Cloud downloads (api.bitbucket.org, downloads as pseudo-releases)`, default `https://api.bitbucket.org/2.0`.

* Wraps `BitbucketClient` (`BITBUCKET_TOKEN`/`BITBUCKET_API_URL`, `Authorization: Bearer`).
* `latest_release` → `GET /repositories/{owner}/{repo}/downloads?pagelen=100` → pseudo-`Release {tag_name: "latest", assets: values[].links.download.href}`.
* `release_by_tag` → same as `latest_release` (tag ignored, `match_assets` filters by asset name).
* `parse_url` → `parse_bitbucket_url` (`https://bitbucket.org/{workspace}/{repo}` or `https://api.bitbucket.org/2.0/repositories/{workspace}/{repo}`).

Both plugins share `lib/github::Release/Asset` types, so `match_assets`/`config_from_release`/`checksum`/`download` stay source-agnostic. `lx_lib::checksum::RawGetter` (`lib/checksum.rs:8`) is implemented for both `GitHubClient` and `GitlabClient`; `check_sidecar` (`lib/checksum.rs:108`) now takes `&dyn RawGetter`, and `src/build.rs:1141` provides `verify_sidecar_or_require_flag_source` adapter (`SourcePlugin::raw_get` → `RawGetter`).

Config `lib/config.rs`:

```yaml
source: github          # github | gitlab | gitea | forgejo | bitbucket | gerrit; alias source_provider, default github
gitlab_host: gitlab.example.com # optional self-hosted GitLab
gitea_host: gitea.example.com   # optional self-hosted Gitea
forgejo_host: codeberg.org      # optional self-hosted Forgejo
bitbucket_host: bitbucket.example.com # optional self-hosted Bitbucket
gerrit_host: review.gerrithub.io # optional self-hosted Gerrit
package_format: deb     # deb | rpm | arch
build_system: cmake     # cmake | cargo | go | custom (omit = auto-detect)
github_repo: owner/repo # alias repo / gitlab_repo / gitea_repo – provider-agnostic identifier
```

Zero-config `lx build https://gitlab.com/owner/repo` auto-sets `source=gitlab` + `github_repo=owner/repo` via `parse_any_url`. Omit `build_system:` to auto-detect from the source tree after fetch.

## 9. Wiring

* `lib/config.rs` `source: String` (`#[serde(default)]` `"github"`, `alias = "source_provider"`) validated `github|gitlab|gitea|forgejo|bitbucket`, `gitlab_host/gitea_host/forgejo_host/bitbucket_host: Option<String>`. `build_system: String` (`#[serde(default)]` `"cmake"`) validated `cmake|cargo|go|custom`.
* `lib/build.rs` resolves `effective_source` → `get_source_plugin`, prints `source: …`, sets `cfg.source` + provider host env vars, `resolve_source_token` (provider-specific `*_TOKEN` env > `cli --token`), then all release/license/download/sidecar paths use `source.*(repo, token, cache_dir)`.
* `lib/sourcebuild.rs` resolves the build system plugin: explicit `build_system:` → `get_build_system()`, else `detect_build_system(src_dir)`, else error. Runs `prebuild_steps`, checks `required_tools()`, then calls `build_sys.build()`.
* `lib/discovery.rs` (`lx discover --source …`), `lib/validate.rs`, `lib/scandeps.rs`, `lib/wizard.rs` all go through `get_source_plugin`.
* `lib/summary.rs` glob switches `*_*.deb` / `-*.rpm` / `-*.pkg.tar.*` and JSON includes `package_format`.
* `lib/source.rs` generates deb source packages (`generate`), RPM source packages (`generate_rpm`), and Arch `PKGBUILD`s (`generate_arch`).
* `lib/checksum.rs` generic over `RawGetter`; `lib/debs.rs` `download`/`verify_sidecar_or_require_flag` also generic (`&dyn RawGetter`).

## 10. Adding a New Plugin

**Package (e.g. `apk`):**

1. `lib/<format>archive.rs` – `pub fn build(root, name, version, …) -> Result<()>` doing deterministic archive (see `archarchive.rs` for pattern).
2. `lib/plugins/<format>.rs` – `impl Plugin` (name, extension, defaults, `build` calls `stage_install_tree` + `lx_lib::<format>archive::build`).
3. Register in `lib/plugins/mod.rs` + `lib/config.rs` match arm + `effective_distributions_for`.
4. `lib/summary.rs` pattern arm + `lib/build.rs` source skip if needed.
5. Add tests in `lib/plugins/mod.rs` (registry + `build_valid_archives` magic check).

**Source (e.g. `myforge`):**

1. `lib/<provider>.rs` – `struct Client { http, base_url, token, cache_dir }` with `latest_release/release_by_tag/releases/repo_license/repo_root/repo_file_text/raw_get` (see `lib/gitlab.rs` for REST mapping and `lib/gitea.rs` for Gitea/Forgejo, `lib/bitbucket.rs` for downloads-as-release).
2. Add `DEFAULT_<PROVIDER>_HOST/API_URL` to `lib/constants.rs` and `homepage_for_<provider>()` helper.
3. Implement `lx_lib::checksum::RawGetter for Client`.
4. `lib/plugins/source/<provider>.rs` – `impl SourcePlugin` delegating to `lib::<provider>::Client` (see `lib/plugins/source/gitlab.rs`/`gitea.rs`).
5. Register in `lib/plugins/source/mod.rs` + `parse_url` for `https://{host}/owner/repo`.
6. Add `source = "<provider>"` to `lib/config.rs` + `*_host: Option<String>` + `resolve_source_token` in `lib/build.rs` + `host` env handling.
7. Tests: `lib/<provider>::tests` (release mapping, `parse_*_url`), `lib/plugins/source/tests` (`parse_any_url` dispatch, registry gains the new name).

**BuildSystem (e.g. `meson`):**

1. `lib/plugins/build_system/<name>.rs` – `impl BuildSystem` with `name()`, `description()`, `recognize(src_dir)` (detect the project file, e.g. `meson.build`), `build(cfg, src_dir, workdir)` (compile + stage DESTDIR-style tree), and `required_tools()` (e.g. `["meson", "ninja"]`).
2. Register in `lib/plugins/build_system/mod.rs` `all_build_systems()`.
3. Add `build_system = "<name>"` to `lib/config.rs` validation.
4. Tests: `lib/plugins/build_system/tests` (registry gains the new name, `recognize` detection, `build` produces expected tree).

No core pipeline changes – `sourcebuild.rs` is build-system-agnostic; it only calls `build_sys.build()`.

## 11. Relation to nfpm

| nfpm | lx |
|---|---|
| Go `Packager` interface, 6 impls, `contents:` DSL + `overrides` | Rust `Plugin` (Package) + `SourcePlugin` (Source) + `BuildSystem` traits — 3 Package + 6 Source + 4 BuildSystem impls, shared `stage_install_tree` + `match_assets` + `RawGetter` |
| General-purpose: you supply files; `arch`/`overrides` per packager | Opinionated: we fetch releases (github/gitlab/gitea/forgejo/bitbucket/gerrit), verify, auto-install ancillaries; `source` selects provider, `package_format` selects packager, `build_system` selects compiler |
| Signing per format (`deb.signature/rpm.signature`) | Only `lintian` for `deb`, reproducible `SOURCE_DATE_EPOCH` for all; `check_sidecar` provider-agnostic via `RawGetter` |
| No source-build concept | `build_mode: source` with pluggable build systems (cmake, cargo, go, custom) — compiles on host, wraps per-suite |

See `docs/decisions/2026-08-20-nfpm-adoptions.md` and `README.md` for the `nfpm`-inspired `relations`/`ancillaries`/`epoch` already shared.
