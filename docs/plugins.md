# Plugin Architecture

**Date:** 2026-08-24 (updated 2026-08-26: plugin types + 5 Source)  
**Status:** Implemented (3× Package: `deb` + `rpm` + `arch`; 6× Source: `github` + `gitlab` + `gitea` + `forgejo` + `bitbucket` + `gerrit`)

`lx` builds Linux packages by repackaging release binaries. The original implementation only produced Debian `.deb`s from GitHub. To support RPM/Arch **and** GitLab without forking the core pipeline, the build was refactored into a **format-agnostic core + two-dimensional pluggable system**.

This matches `goreleaser/nfpm`'s Go `Packager` → `deb/`, `rpm/`, `apk/` at repo root, but adds a second dimension for source providers:

* **Package plugins** (`src/plugins/{deb,rpm,arch}.rs` + `lib/{deb,rpm,arch}archive.rs`) → produce installable artifact
* **Source plugins** (`src/plugins/source/{github,gitlab}.rs` + `lib/{github,gitlab}.rs`) → discover releases/assets

Both are stateless, registered statically, and share the same `PluginType` discriminator.

---

## 1. Plugin Types

`src/plugins/mod.rs:16`, `src/plugins/source/mod.rs:16`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginType {
    Package, // deb, rpm, arch
    Source,  // github, gitlab
}

pub trait Plugin: Send + Sync { // Package
    fn plugin_type(&self) -> PluginType { PluginType::Package }
    fn name(&self) -> &'static str;
    // ...
}
pub trait SourcePlugin: Send + Sync {
    fn plugin_type(&self) -> PluginType { PluginType::Source }
    fn name(&self) -> &'static str; // "github" | "gitlab"
    fn parse_url(&self, url: &str) -> Option<String>;
    fn latest_release(&self, repo: &str, token: Option<&str>, cache_dir: Option<&Path>) -> Result<Release>;
    // ...
}
```

`PluginType` lets callers list/filter by dimension (`all_plugins()` vs `all_source_plugins()`) and keeps `docs/` and `lx --help` organized. Adding a new dimension (e.g. `Publisher`) would be a new enum variant, not a new registry.

## 2. Package Trait

`src/plugins/mod.rs:46`

```rust
pub trait Plugin: Send + Sync {
    fn name(&self) -> &'static str;              // "deb" | "rpm" | "arch"
    fn file_extension(&self) -> &'static str;    // "deb" | "rpm" | "pkg.tar.zst"
    fn description(&self) -> &'static str;
    fn default_distributions(&self) -> &'static [&'static str];
    fn arch_supported_for_dist(&self, arch: &str, dist: &str) -> bool;
    fn build(&self, ctx: &BuildContext) -> Result<PathBuf>;
    fn lint(&self, _path: &Path, ...) -> Result<()> { Ok(()) }
}
```

`BuildContext` (`src/plugins/mod.rs:23`) carries everything a packager needs without coupling to global state:

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

`src/plugins/source/mod.rs:29`

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

`src/plugins/mod.rs:82` (package) and `src/plugins/source/mod.rs:129` (source):

```rust
// Package
pub fn all_plugins() -> Vec<Box<dyn Plugin>> {
    vec![Box::new(deb::DebPlugin), Box::new(rpm::RpmPlugin), Box::new(arch::ArchPlugin)]
}
pub fn get_plugin(name: &str) -> Option<Box<dyn Plugin>> { /* case-insensitive */ }

// Source
pub fn all_source_plugins() -> Vec<Box<dyn SourcePlugin>> {
    vec![Box::new(github::GithubSourcePlugin), Box::new(gitlab::GitlabSourcePlugin)]
}
pub fn get_source_plugin(name: &str) -> Option<Box<dyn SourcePlugin>> { /* case-insensitive */ }
pub fn parse_any_url(url: &str) -> Option<(String,String)> { // (source, repo)
    for p in all_source_plugins() { if let Some(repo)=p.parse_url(url) { return Some((p.name().into(), repo)) } }
    None
}
```

No `dlopen`, no feature flags. Adding a format = `impl Plugin` + one line in `all_plugins()` + `src/config.rs:193` allow. Adding a source = `impl SourcePlugin` + one line in `all_source_plugins()` + `src/config.rs:source` allow.

Selection in `src/build.rs:184` / `src/discovery.rs:132` / `src/validate.rs:42`:

```
--format flag  >  package.yaml `package_format`  >  default "deb"   → Plugin
--source flag  >  package.yaml `source`         >  default "github" → SourcePlugin
--plus zero-config URL host sniffing: `parse_any_url("https://gitlab.com/…") → ("gitlab","owner/repo")`
```

`cfg.effective_package_format()` (`src/config.rs:202`) and `cfg.effective_source()` (`src/config.rs:203`) normalise; `cfg.effective_distributions_for(format)` picks per-plugin defaults.

## 5. Shared Staging

`stage_install_tree()` (`src/plugins/mod.rs:106`) is format-agnostic and reused by all three package plugins:

* `bundle:false` → keep only ELF files in `/usr/bin`, auto-install `*.1(.gz)` → `/usr/share/man/manN/*.gz` and `LICENSE*|COPYING*|NOTICE*` → `/usr/share/doc/<pkg>/`.
* `bundle:true` → `copy_dir_recursive` whole `binary_path` tree to `/usr/lib/<pkg>/` and `symlink_elf_executables` into `/usr/bin` (preserves `$ORIGIN` RPATH, versioned `.so` symlinks).
* `binary_rename` and empty-`/usr/bin` guard.

This preserves the nfpm-inspired ancillary handling (`docs/decisions/2026-08-20-nfpm-adoptions.md`) centrally.

## 6. Package Plugins

### deb — `src/plugins/deb.rs:11` + `lib/debarchive.rs:1`

*Extension* `deb`, *defaults* `bookworm/trixie/forky/sid`, *matrix* → `PackageConfig::arch_supported_for_dist()` (universal + distro-gated `i386/armel/riscv64/loong64`).

Stages via `stage_install_tree`, renders `DEBIAN/control` (`Section/Priority/Package/Version/Architecture/Maintainer/Homepage/Description` + `Relations` `lib/pkgmeta.rs:55`), `changelog.Debian.gz`/`copyright` via `lib/pkgmeta::render_*`, then `lx_lib::debarchive::build()` (`ar` with `debian-binary`+`control.tar.gz`+`data.tar.gz`, sorted walk, normalized `mtime/uid/gid`, deterministic `gzip`).

Lint via `lx_lib::lintian::run()`.

### rpm — `src/plugins/rpm.rs:7` + `lib/rpmarchive.rs:1`

*Extension* `rpm`, *defaults* `fedora/el9/el8/opensuse`, permissive matrix.

Stages same tree, then `lx_lib::rpmarchive::build()` (`rpm = "0.16"` `PackageBuilder`): `name/version (stripped)/license/arch (mapped `amd64→x86_64`)` + `release = "{build_version}.{dist}"` (dot-normalised), `summary/description` from `effective_description`, `source_date(mtime)` for reproducibility, `with_file()` for each payload file (sorted) + dummy file for symlinks (`FileOptions::symlink`).

Valid RPM magic `ED AB EE DB`, tested with `rpm -qip` equivalent.

### arch — `src/plugins/arch.rs:7` + `lib/archarchive.rs:1`

*Extension* `pkg.tar.zst`, *defaults* `arch` (rolling), permissive matrix.

Stages same tree, renders `.PKGINFO` (`pkgname/pkgver/pkgdesc/url/builddate/packager/size/arch/license`), `.MTREE` (`#mtree` + `time/mode/type/size/sha256digest` per entry), then `tar` + `zstd` level 19 (deterministic). Filename `name-version-release-arch.pkg.tar.zst` (e.g. `hello-1.0-1.arch-x86_64.pkg.tar.zst`), `zstd` magic `28 B5 2F FD`, verifiable `tar tz --use-compress-program=unzstd`.

## 7. Source Plugins (Auto-Discovery)

### github — `src/plugins/source/github.rs:7` + `lib/github.rs:12`

*Name* `github`, *description* `GitHub Releases (api.github.com / octocrab)`.

* Wraps `lx_lib::github::GitHubClient` (`octocrab` + `tokio` + 5-min JSON cache `api_cache_dir`).
* `parse_url` → `crate::build::parse_github_url` (`https://github.com/owner/repo(.git)?(/releases/…)?`).
* `latest_release` → `GET /repos/{owner}/{repo}/releases/latest`, `release_by_tag` → `GET /repos/{owner}/{repo}/releases/tags/{tag}`.
* `raw_get`/`releases`/`repo_license`/`repo_root`/`repo_file_text` map 1:1 to `GitHubClient` methods (dual-license `LICENSE-APACHE`+`LICENSE-MIT` detection uses `repo_root` + `repo_file_text`).
* Token: `resolve_source_token("github", cli_token)` prefers `cli --token` then `GITHUB_TOKEN` env.

### gitlab — `src/plugins/source/gitlab.rs:7` + `lib/gitlab.rs:1`

*Name* `gitlab`, *description* `GitLab Releases (gitlab.com / self-hosted, API v4)`.

* Wraps `lx_lib::gitlab::GitlabClient` (blocking `reqwest`, same 5-min cache, `GITLAB_API_URL`/`GITLAB_HOST` env, default `https://gitlab.com/api/v4`).
* `parse_url` → `parse_gitlab_url` (`https://gitlab.com/owner/repo`, custom host via `GITLAB_HOST`).
* `latest_release` → `GET /projects/{%2F-encoded}/releases?per_page=1`, `release_by_tag` → `GET /projects/{id}/releases/{tag}`.
* Mapping: `GitlabReleaseRaw {tag_name, assets:{links[]}}` → `Release {assets: links→Asset}`. `published_at: released_at||created_at → jiff`.
* Token: `GITLAB_TOKEN` env > `cli --token`.

### gitea — `src/plugins/source/gitea.rs:7` + `lib/gitea.rs:1`

*Name* `gitea`, `Gitea Releases (codeberg.org / self-hosted, API v1)`, default `https://codeberg.org/api/v1` (`DEFAULT_GITEA_API_URL`).

* Wraps `GiteaClient` (`GITEA_API_URL`/`GITEA_HOST` env, `GITEA_TOKEN`).
* `parse_url` → `parse_gitea_url` (`https://codeberg.org/owner/repo`).
* `latest_release` → `GET /repos/{owner}/{repo}/releases?limit=1`, `release_by_tag` → `GET /repos/{owner}/{repo}/releases/tags/{tag}`.
* Assets: `GiteaReleaseRaw {tag_name, assets[]}` → `Release`.

### forgejo — `src/plugins/source/forgejo.rs:7` + `lib/forgejo.rs:1`

*Name* `forgejo`, `Forgejo Releases (codeberg.org / self-hosted, Gitea-compatible)`, default `https://codeberg.org/api/v1` (`DEFAULT_FORGEJO_API_URL`).

* Wraps `ForgejoClient` (same `GiteaClient` logic but `FORGEJO_TOKEN`/`FORGEJO_HOST` env, falls back to `GITEA_TOKEN`/`GITEA_HOST` for compat).
* `parse_url` → `parse_forgejo_url` (`https://codeberg.org/owner/repo`, also `forgejo`-containing hosts).
* Same release mapping as Gitea.

### bitbucket — `src/plugins/source/bitbucket.rs:7` + `lib/bitbucket.rs:1`

*Name* `bitbucket`, `Bitbucket Cloud downloads (api.bitbucket.org, downloads as pseudo-releases)`, default `https://api.bitbucket.org/2.0`.

* Wraps `BitbucketClient` (`BITBUCKET_TOKEN`/`BITBUCKET_API_URL`, `Authorization: Bearer`).
* `latest_release` → `GET /repositories/{owner}/{repo}/downloads?pagelen=100` → pseudo-`Release {tag_name: "latest", assets: values[].links.download.href}`.
* `release_by_tag` → same as `latest_release` (tag ignored, `match_assets` filters by asset name).
* `parse_url` → `parse_bitbucket_url` (`https://bitbucket.org/{workspace}/{repo}` or `https://api.bitbucket.org/2.0/repositories/{workspace}/{repo}`).

Both plugins share `lib/github::Release/Asset` types, so `match_assets`/`config_from_release`/`checksum`/`download` stay source-agnostic. `lx_lib::checksum::RawGetter` (`lib/checksum.rs:8`) is implemented for both `GitHubClient` and `GitlabClient`; `check_sidecar` (`lib/checksum.rs:108`) now takes `&dyn RawGetter`, and `src/build.rs:1141` provides `verify_sidecar_or_require_flag_source` adapter (`SourcePlugin::raw_get` → `RawGetter`).

Config `src/config.rs:83`:

```yaml
source: github          # github | gitlab | gitea | forgejo | bitbucket | gerrit; alias source_provider, default github
gitlab_host: gitlab.example.com # optional self-hosted GitLab
gitea_host: gitea.example.com   # optional self-hosted Gitea
forgejo_host: codeberg.org      # optional self-hosted Forgejo
bitbucket_host: bitbucket.example.com # optional self-hosted Bitbucket
gerrit_host: review.gerrithub.io # optional self-hosted Gerrit
package_format: deb     # deb | rpm | arch
github_repo: owner/repo # alias repo / gitlab_repo / gitea_repo – provider-agnostic identifier
```

Zero-config `lx build https://gitlab.com/owner/repo` auto-sets `source=gitlab` + `github_repo=owner/repo` via `parse_any_url` (`src/plugins/source/mod.rs:152`, `src/build.rs:161`, `src/scandeps.rs:36`, `src/discovery.rs:132`).

## 8. Wiring

* `src/config.rs:83` `source: String` (`#[serde(default)]` `"github"`, `alias = "source_provider"`) validated `github|gitlab|gitea|forgejo|bitbucket`, `gitlab_host/gitea_host/forgejo_host/bitbucket_host: Option<String>`.
* `src/build.rs:229` resolves `effective_source` → `get_source_plugin`, prints `source: …`, sets `cfg.source` + `GITLAB_HOST/GITEA_HOST/FORGEJO_HOST/BITBUCKET_HOST` env, `resolve_source_token` (provider-specific `*_TOKEN` env > `cli --token`), then all release/license/download/sidecar paths use `source.*(repo, token, cache_dir)`. `build_jobs` (`src/build.rs:747`) now takes `source_name: String` + `token: Option<String>` and each thread re-looks-up the plugin; `build_one` (`src/build.rs:881`) takes `&dyn SourcePlugin` + `token`.
* `src/discovery.rs:132` (`lx discover --source github|gitlab|gitea|forgejo|bitbucket|gerrit`), `src/validate.rs:42`, `src/scandeps.rs:35`, `src/wizard.rs:35` all go through `get_source_plugin` (`wizard` prompts `Source provider (github/gitlab/gitea/forgejo/bitbucket/gerrit)` and optional host; `validate`/`scandeps` set `GERRIT_HOST` from `cfg.gerrit_host` same as the other providers).
* `src/summary.rs:51` glob switches `*_*.deb` / `-*.rpm` / `-*.pkg.tar.*` and JSON includes `package_format`; `source` is not yet in summary (provider-agnostic).
* `src/source.rs` generates deb source packages (`generate`), RPM source packages (`generate_rpm` -- `.src.rpm` via `lx_lib::rpmarchive::build_srpm`), and Arch `PKGBUILD`s (`generate_arch` -- no compiled archive; a real Arch source package *is* a `PKGBUILD` text file). All three re-extract the upstream payload from an already-built binary artifact (`lx_lib::rpmarchive::extract` / `lx_lib::archarchive::extract` for rpm/arch) rather than reusing the live build's staging dir, matching the deb path's existing approach.
* `lib/checksum.rs:108` generic over `RawGetter`; `src/debs.rs:137` `download`/`verify_sidecar_or_require_flag` also generic (`&dyn RawGetter`) – `lx install`'s `latest-debs` org stays GitHub-specific but now benefits from the same checksum abstraction.

## 9. Adding a New Plugin

**Package (e.g. `apk`):**

1. `lib/<format>archive.rs` – `pub fn build(root, name, version, …) -> Result<()>` doing deterministic archive (see `archarchive.rs` for pattern).
2. `src/plugins/<format>.rs` – `impl Plugin` (name, extension, defaults, `build` calls `stage_install_tree` + `lx_lib::<format>archive::build`).
3. Register in `src/plugins/mod.rs:82` and `src/config.rs:193` match arm + `effective_distributions_for`.
4. `src/summary.rs:55` pattern arm + `src/build.rs:384` source skip if needed.
5. Add tests in `src/plugins/mod.rs:282` (registry + `build_valid_archives` magic check).

**Source (e.g. `myforge`):**

1. `lib/<provider>.rs` – `struct Client { http, base_url, token, cache_dir }` with `latest_release/release_by_tag/releases/repo_license/repo_root/repo_file_text/raw_get` (see `lib/gitlab.rs` for REST mapping and `lib/gitea.rs` for Gitea/Forgejo, `lib/bitbucket.rs` for downloads-as-release).
2. Add `DEFAULT_<PROVIDER>_HOST/API_URL` to `lib/constants.rs:5` and `homepage_for_<provider>()` helper.
3. Implement `lx_lib::checksum::RawGetter for Client`.
4. `src/plugins/source/<provider>.rs` – `impl SourcePlugin` delegating to `lib::<provider>::Client` (see `src/plugins/source/gitlab.rs`/`gitea.rs`).
5. Register in `src/plugins/source/mod.rs:129` + `parse_url` for `https://{host}/owner/repo`.
6. Add `source = "<provider>"` to `src/config.rs:193` + `*_host: Option<String>` + `resolve_source_token` in `src/build.rs:520` + `host` env handling in `src/build.rs:251`/`src/validate.rs`/`src/scandeps.rs`/`src/wizard.rs`.
7. Tests: `lib/<provider>::tests` (release mapping, `parse_*_url`), `src/plugins/source/tests` (`parse_any_url` dispatch, registry gains the new name — see `registry_contains_github_and_gitlab` for the pattern).

No core pipeline changes – parallel `build_jobs()` grouping by `arch` remains agnostic to both dimensions.

## 10. Relation to nfpm

| nfpm | lx |
|---|---|
| Go `Packager` interface, 6 impls, `contents:` DSL + `overrides` | Rust `Plugin` (Package) + `SourcePlugin` (Source) traits, 3 Package + 5 Source impls, shared `stage_install_tree` + `match_assets` + `RawGetter` |
| General-purpose: you supply files; `arch`/`overrides` per packager | Opinionated: we fetch releases (github/gitlab/gitea/forgejo/bitbucket/gerrit), verify, auto-install ancillaries; `source` selects provider (`--provider`/`package.yaml:source`), `package_format` selects packager |
| Signing per format (`deb.signature/rpm.signature`) | Only `lintian` for `deb`, reproducible `SOURCE_DATE_EPOCH` for all; `check_sidecar` now provider-agnostic via `RawGetter` |

See `docs/decisions/2026-08-20-nfpm-adoptions.md` and `README.md:210` for the `nfpm`-inspired `relations`/`ancillaries`/`epoch` already shared.
