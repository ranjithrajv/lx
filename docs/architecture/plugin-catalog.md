# Plugin catalog

Trait definitions and per-plugin detail for lx's eight plugin dimensions.
See [plugins.md](plugins.md) for the dimensions overview, wiring, and how to
add a plugin. Part of the [lx docs](../README.md).

## 2. Package Trait

`lib/plugins/mod.rs`

```rust
pub trait Packager: Send + Sync {
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

`lib/plugins/forge/mod.rs`

```rust
pub trait ForgeSource: Send + Sync {
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

`lib/plugins/mod.rs` (package), `lib/plugins/forge/mod.rs` (source), `lib/plugins/build_system/mod.rs` (build system):

```rust
// Package
pub fn all_plugins() -> Vec<Box<dyn Packager>> {
    vec![Box::new(deb::DebPlugin), Box::new(rpm::RpmPlugin), Box::new(arch::ArchPlugin)]
}
pub fn get_plugin(name: &str) -> Option<Box<dyn Packager>> { /* case-insensitive */ }

// Source
pub fn all_forge_sources() -> Vec<Box<dyn ForgeSource>> {
    vec![Box::new(github::GithubForgeSource), Box::new(gitlab::GitlabForgeSource), …]
}
pub fn get_forge_source(name: &str) -> Option<Box<dyn ForgeSource>> { /* case-insensitive */ }
pub fn parse_any_url(url: &str) -> Option<(String,String)> { // (source, repo)
    for p in all_forge_sources() { if let Some(repo)=p.parse_url(url) { return Some((p.name().into(), repo)) } }
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

// RegistrySource
pub fn all_registry_sources() -> Vec<Box<dyn RegistrySource>> {
    vec![Box::new(npm::NpmRegistrySource), Box::new(python::PythonRegistrySource),
         Box::new(gem::GemRegistrySource)]
}
pub fn get_registry_source(name: &str) -> Option<Box<dyn RegistrySource>> { /* case-insensitive */ }
```

No `dlopen`, no feature flags. Adding a format = `impl Packager` + one line in `all_plugins()`. Adding a source = `impl ForgeSource` + one line in `all_forge_sources()`. Adding a build system = `impl BuildSystem` + one line in `all_build_systems()`.

Selection in `lib/build.rs` / `lib/discovery.rs` / `lib/validate.rs`:

```
--format flag  >  package.yaml `package_format`  >  default "deb"   → Packager
--source flag  >  package.yaml `source`         >  default "github" → ForgeSource
--plus zero-config URL host sniffing: `parse_any_url("https://gitlab.com/…") → ("gitlab","owner/repo")`
```

BuildSystem selection in `lib/sourcebuild.rs`:

```
build_system: in package.yaml  >  auto-detect from source tree  >  error if none recognized
```

RegistrySource selection in `lib/build.rs`:

```
registry_source: in package.yaml  >  empty (forge release mode)
```

Auto-detection checks `recognize()` in registry order: `CMakeLists.txt` → cmake, `Cargo.toml` → cargo, `go.mod` → go, `meson.build` → meson, `configure`/`configure.ac`/`autogen.sh` → autotools, `Makefile` → make. `make` is last among the auto-detecting systems so more specific systems win; `custom` never auto-detects (explicit-only).

`cfg.effective_package_format()`, `cfg.effective_source()`, and `cfg.effective_registry_source()` normalise; `cfg.effective_distributions_for(format)` picks per-plugin defaults.

## 5. Shared Staging

`stage_install_tree()` (`lib/plugins/mod.rs`) is format-agnostic and reused by all three package plugins:

* `bundle:false` → keep only ELF files in `/usr/bin`, auto-install `*.1(.gz)` → `/usr/share/man/manN/*.gz` and `LICENSE*|COPYING*|NOTICE*` → `/usr/share/doc/<pkg>/`.
* `bundle:true` → `copy_dir_recursive` whole `binary_path` tree to `/usr/lib/<pkg>/` and `symlink_elf_executables` into `/usr/bin` (preserves `$ORIGIN` RPATH, versioned `.so` symlinks).
* `prefix` set (`--from-dir`/`--from-file` mode) → dump all files flat under `<root><prefix>` with no ELF detection or ancillary staging (fpm-style "you supply files").
* `binary_rename` and empty-`/usr/bin` guard.

This preserves the nfpm-inspired ancillary handling (`docs/decisions/2026-08-20-nfpm-adoptions.md`) centrally.

## 6. Package Plugins

### deb — `lib/plugins/deb.rs` + `lib/debarchive.rs` *(7 total: deb + rpm + arch + apk + ipk + msix + osxpkg)*

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

### apk — `lib/plugins/apk.rs` + `lib/apkarchive.rs`

*Extension* `apk`, *defaults* `alpine` (rolling), permissive matrix.

Alpine apk-tools v2 format: concatenated gzip members `[control.tar.gz][data.tar.gz]`, where the control member holds `.PKGINFO` and the data member holds the payload. `pkgver` is `{version}-r{build_version}` and `datahash` pins the SHA-256 of the compressed data member. Arch maps Debian → Alpine (`amd64→x86_64`, `armhf→armv7`, `i386→x86`). Because Alpine is musl-only, this is the natural output for `musl: true` builds. `scripts:` entries are staged as dotted control-segment files (`.pre-install`, `.post-install`, `.pre-deinstall`, `.post-deinstall`, `.pre-upgrade`, `.post-upgrade`). When `--sign-key` is an RSA private key, the control segment is signed and a `.SIGN.RSA.<keyname>` segment is prepended; unsigned packages install with `apk add --allow-untrusted`. (Signed packages are verified against real `apk-tools` in the test suite when `LX_APK_STATIC` points at an `apk.static`.)

### ipk — `lib/plugins/ipk.rs` + `lib/ipkarchive.rs`

*Extension* `ipk`, *defaults* `openwrt`, permissive matrix.

OpenWrt/opkg `.ipk` shares the `.deb` `ar` layout (`debian-binary` + `control.tar.gz` + `data.tar.gz`), so archiving delegates to `debarchive` and only the control dialect (`Package/Version/Architecture/Installed-Size/License/Depends/Provides/Conflicts`) and filename (`name_version_arch.ipk`) differ. `scripts:` are emitted as `preinst`/`postinst`/`prerm`/`postrm` control members, and config-typed `contents:` entries are listed in a `conffiles` member. Gzip is pinned regardless of `compression:` since it is the only format opkg has always accepted.

### msix — `lib/plugins/msix.rs` + `lib/msixarchive.rs`

*Extension* `msix`, *defaults* `windows`, permissive matrix. Windows MSIX output, mirroring nfpm's `msix` packager.

Builds an OPC (zip) container with `[Content_Types].xml`, `AppxManifest.xml`, and `AppxBlockMap.xml`, plus the staged payload at its destination paths. Metadata comes from the `msix:` config block: `publisher` (required), `properties` (display name/logo), `identity.resource_id`, `applications` (required; FullTrust entry points auto-add the `runFullTrust` restricted capability), `dependencies.target_device_families`, and `capabilities`. Version is normalized to MSIX's 4-part numeric form and the architecture maps from Debian naming (`amd64→x64`, `i386→x86`, …). **Native signing:** with `signature.key_file` (PKCS#8 RSA key, PEM) and `signature.cert_file` (X.509 cert, PEM), the packager appends a real `AppxSignature.p7x` (PKCS#7 over the Appx digests) using the `msix` crate; the certificate Subject must match the manifest `Publisher`. `msix.signature.pfx_file` is not supported.

### osxpkg — `lib/plugins/osxpkg.rs` + `lib/osxpkgarchive.rs`

*Extension* `pkg`, *defaults* `macos`, permissive matrix. macOS flat package, mirroring fpm's `osxpkg` output.

fpm shells out to macOS `pkgbuild`; lx writes the **xar** container in-process. The archive holds `PackageInfo` (identifier = `package_name`, version, install location from `prefix:` or `/`, script declarations) and a `Payload` member — a **newc `cpio`** archive of the staged tree, gzip-compressed — plus a `Scripts` cpio member when `scripts.preinstall`/`postinstall` are set. Header is big-endian; the TOC is zlib-compressed XML with an archive SHA-256 checksum and per-member `extracted`/`archived` SHA-1 checksums. `pkg` is accepted as an alias for `--format`. **Native signing:** with `signature.key_file` (PKCS#8 key, PEM) and `signature.cert_file` (X.509 cert, PEM), the built package is signed in place by the `pkg-xar` backend using `apple-xar`'s `XarSigner` (adds the `Signature` member over the archive checksum). No `Bom` is generated, so some `installer` paths may still require one.

## 7. BuildSystem Plugins (Compile Source)

`lib/plugins/build_system/{cmake,cargo,go,meson,autotools,make,custom}.rs` — 7 total. Used by `build_mode: source` to compile upstream source into a DESTDIR-style install tree.

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

### autotools — `lib/plugins/build_system/autotools.rs`

*Name* `autotools`, detects `configure` / `configure.ac` / `configure.in` / `autogen.sh`, requires `make`.

Bootstraps `configure` via `autogen.sh` or `autoreconf -fi` when the release tarball didn't ship one, then `./configure --prefix=/usr` (with `cmake_flags:` passed through as extra configure flags), `make -jN`, and `make DESTDIR=<stage> install`. `musl: true` sets `CC=musl-gcc`/`CXX=musl-g++`/`LDFLAGS=-static`.

### make — `lib/plugins/build_system/make.rs`

*Name* `make`, detects `Makefile` / `makefile` / `GNUmakefile`, requires `make`.

`make -jN PREFIX=/usr` then `make DESTDIR=<stage> PREFIX=/usr install`. Registered **last** among the auto-detecting systems so cmake/cargo/go/meson/autotools claim their trees first; `make` only handles plain-Makefile projects.

### custom — `lib/plugins/build_system/custom.rs`

*Name* `custom`, never auto-detects (explicit-only). Runs user-supplied `build_commands` then `install_commands` (with `$DESTDIR` set to the stage dir). Universal escape hatch for build systems without a dedicated plugin (npm, python, waf, …).

## 8. Source Plugins (Auto-Discovery)

9 total: `github` + `gitlab` + `gitea` + `forgejo` + `bitbucket` + `gerrit` + `gitee` + `sourceforge` + `custom`.

### github — `lib/plugins/forge/github.rs` + `lib/github.rs`

*Name* `github`, *description* `GitHub Releases (api.github.com, blocking reqwest)`.

* Wraps `lx_lib::github::GitHubClient` (blocking `reqwest` + 5-min JSON cache `api_cache_dir`; same shape as every other forge client).
* `parse_url` → `crate::build::parse_github_url` (`https://github.com/owner/repo(.git)?(/releases/…)?`).
* `latest_release` → `GET /repos/{owner}/{repo}/releases/latest`, `release_by_tag` → `GET /repos/{owner}/{repo}/releases/tags/{tag}`.
* `raw_get`/`releases`/`repo_license`/`repo_root`/`repo_file_text` map 1:1 to `GitHubClient` methods (dual-license `LICENSE-APACHE`+`LICENSE-MIT` detection uses `repo_root` + `repo_file_text`).
* Token: `resolve_source_token("github", cli_token)` prefers `cli --token` then `GITHUB_TOKEN` env.

### gitlab — `lib/plugins/forge/gitlab.rs` + `lib/gitlab.rs`

*Name* `gitlab`, *description* `GitLab Releases (gitlab.com / self-hosted, API v4)`.

* Wraps `lx_lib::gitlab::GitlabClient` (blocking `reqwest`, same 5-min cache, `GITLAB_API_URL`/`GITLAB_HOST` env, default `https://gitlab.com/api/v4`).
* `parse_url` → `parse_gitlab_url` (`https://gitlab.com/owner/repo`, custom host via `GITLAB_HOST`).
* `latest_release` → `GET /projects/{%2F-encoded}/releases?per_page=1`, `release_by_tag` → `GET /projects/{id}/releases/{tag}`.
* Mapping: `GitlabReleaseRaw {tag_name, assets:{links[]}}` → `Release {assets: links→Asset}`. `published_at: released_at||created_at → jiff`.
* Token: `GITLAB_TOKEN` env > `cli --token`.

### gitea — `lib/plugins/forge/gitea.rs` + `lib/gitea.rs`

*Name* `gitea`, `Gitea Releases (codeberg.org / self-hosted, API v1)`, default `https://codeberg.org/api/v1` (`DEFAULT_GITEA_API_URL`).

* Wraps `GiteaClient` (`GITEA_API_URL`/`GITEA_HOST` env, `GITEA_TOKEN`).
* `parse_url` → `parse_gitea_url` (`https://codeberg.org/owner/repo`).
* `latest_release` → `GET /repos/{owner}/{repo}/releases?limit=1`, `release_by_tag` → `GET /repos/{owner}/{repo}/releases/tags/{tag}`.
* Assets: `GiteaReleaseRaw {tag_name, assets[]}` → `Release`.

### forgejo — `lib/plugins/forge/forgejo.rs` + `lib/forgejo.rs`

*Name* `forgejo`, `Forgejo Releases (codeberg.org / self-hosted, Gitea-compatible)`, default `https://codeberg.org/api/v1` (`DEFAULT_FORGEJO_API_URL`).

* Wraps `ForgejoClient` (same `GiteaClient` logic but `FORGEJO_TOKEN`/`FORGEJO_HOST` env, falls back to `GITEA_TOKEN`/`GITEA_HOST` for compat).
* `parse_url` → `parse_forgejo_url` (`https://codeberg.org/owner/repo`, also `forgejo`-containing hosts).
* Same release mapping as Gitea.

### bitbucket — `lib/plugins/forge/bitbucket.rs` + `lib/bitbucket.rs`

*Name* `bitbucket`, `Bitbucket Cloud downloads (api.bitbucket.org, downloads as pseudo-releases)`, default `https://api.bitbucket.org/2.0`.

* Wraps `BitbucketClient` (`BITBUCKET_TOKEN`/`BITBUCKET_API_URL`, `Authorization: Bearer`).
* `latest_release` → `GET /repositories/{owner}/{repo}/downloads?pagelen=100` → pseudo-`Release {tag_name: "latest", assets: values[].links.download.href}`.
* `release_by_tag` → same as `latest_release` (tag ignored, `match_assets` filters by asset name).
* `parse_url` → `parse_bitbucket_url` (`https://bitbucket.org/{workspace}/{repo}` or `https://api.bitbucket.org/2.0/repositories/{workspace}/{repo}`).

### gitee — `lib/plugins/forge/gitee.rs` + `lib/gitee.rs`

*Name* `gitee`, `Gitee Releases (gitee.com / self-hosted, API v5)`, default `https://gitee.com/api/v5` (`DEFAULT_GITEE_API_URL`).

* Wraps `GiteeClient` (`GITEE_TOKEN`/`GITEE_HOST`/`GITEE_API_URL` env).
* `parse_url` → `parse_gitee_url` (`https://gitee.com/owner/repo`).
* `latest_release` → `GET /repos/{owner}/{repo}/releases?per_page=1`; `release_by_tag` → `GET /repos/{owner}/{repo}/releases/tags/{tag}` (falls back to scanning the list); `releases` → list endpoint.
* Mapping: `GiteeReleaseRaw {tag_name, created_at, assets[]}` → `Release`; `assets[]` carry `name` + `browser_download_url` (falls back to `attach_files[]`).

### sourceforge — `lib/plugins/forge/sourceforge.rs` + `lib/sourceforge.rs`

*Name* `sourceforge`, `SourceForge file releases (project RSS feed as pseudo-releases)`, default `https://sourceforge.net` (`DEFAULT_SOURCEFORGE_API_URL`).

* Wraps `SourceForgeClient` (no auth).
* Projects are identified by a **bare name** (`github_repo: sevenzip`), not `owner/repo`; `parse_url` accepts `https://sourceforge.net/projects/{project}/…` and `https://{project}.sourceforge.net/`.
* `latest_release` → parse `GET /projects/{project}/rss?limit=100` into a pseudo-`Release {tag_name: "latest", assets}` (each `<item>` → asset named by the path basename, with `filesize`); `release_by_tag` returns the same file set tagged with the requested version and lets `match_assets` select by filename.
* The feed's inline `md5` is carried on each asset and verified after download (no `.sha256` sidecar needed).

### custom — `lib/plugins/forge/custom.rs`

*Name* `custom`, `Direct URL template (upstream_url with {version}/{arch}) — no forge API`.

* Explicit-only (`parse_url` returns `None`) — selected via `source: custom`, never by URL sniffing.
* `upstream_url` in `package.yaml` is expanded per build with `{version}`, `{arch}`, and `{package_name}` placeholders; each architecture resolves to exactly one download URL, so `architectures:` must name the archs.
* `release_by_tag` returns an empty-asset release carrying the tag and `build.rs` builds the real per-arch asset map via `synthetic_release`; there is no release listing.
* No `.sha256` sidecar probing — verification is via `--pinned-metadata` / `package.lock`, falling back to `--allow-unverified` / `--no-verify`.

The GitHub and GitLab plugins share `lib/github::Release/Asset` types, so `match_assets`/`config_from_release`/`checksum`/`download` stay source-agnostic. `lx_lib::checksum::RawGetter` (`lib/checksum.rs:8`) is implemented for both `GitHubClient` and `GitlabClient`; `check_sidecar` (`lib/checksum.rs:108`) now takes `&dyn RawGetter`, and `src/build.rs:1141` provides `verify_sidecar_or_require_flag_source` adapter (`ForgeSource::raw_get` → `RawGetter`).

Config `lib/config.rs`:

```yaml
source: github          # github | gitlab | gitea | forgejo | bitbucket | gerrit | gitee | sourceforge | custom; alias source_provider, default github
gitlab_host: gitlab.example.com # optional self-hosted GitLab
gitea_host: gitea.example.com   # optional self-hosted Gitea
forgejo_host: codeberg.org      # optional self-hosted Forgejo
bitbucket_host: bitbucket.example.com # optional self-hosted Bitbucket
gerrit_host: review.gerrithub.io # optional self-hosted Gerrit
gitee_host: gitee.example.com   # optional self-hosted Gitee
package_format: deb     # deb | rpm | arch | apk | ipk | msix
build_system: cmake     # cmake | cargo | go | meson | autotools | make | custom (omit = auto-detect)
github_repo: owner/repo # alias repo / gitlab_repo / gitea_repo – provider-agnostic identifier
registry_source: npm        # npm | python | gem | cargo | go | hex | dart | nuget | maven | composer | cpan (uses github_repo as package name)
```

Zero-config `lx build https://gitlab.com/owner/repo` auto-sets `source=gitlab` + `github_repo=owner/repo` via `parse_any_url`. Omit `build_system:` to auto-detect from the source tree after fetch. When `registry_source` is set, `github_repo` is the registry package name and `source:` is ignored.

## 12. Cross-cutting plugin dimensions

The original four dimensions answer *"where does the payload come from /
what does it become."* Four more cover cross-cutting lifecycles and the
`lx index` fan-out that used to be hardcoded `match` branches in the core:

| Dimension | Trait / registry | Impls | Selected by | Replaces |
|---|---|---|---|---|
| **ArtifactFormat** | `lib/plugins/artifact/mod.rs` | `tar.gz` `tar.xz` `tar.zst` `tar` `zip` `raw` | `artifact_format:` / auto-detect | `build.rs::extract` + `discovery::guess_format` match |
| **Signer** | `lib/plugins/signer/mod.rs` | `gpg-detach` `rpm-pgp` `deb-debsign` `apk-rsa` `msix-p7x` `pkg-xar` | `(package_format, sign_method)` | `if format == "deb"` signing branches |
| **DependencyMapper** | `lib/plugins/depmap/mod.rs` | `debian` `rpm` `pacman` `alpine` `openwrt` | target `package_format` | per-format `match` inside `depmap.rs` |
| **PackageIndex** | `lib/plugins/package_index/mod.rs` | 5 write (`apt` `opkg` `pacman` `apk` `rpm`) + 4 read (`lx-community` `aur` `repology` `custom`) | `lx repo --format` / `indexes.yaml` | apt-only `lib/repo.rs` + `SourceKind` match in `index/registry.rs` |

* **ArtifactFormat** — `artifact_format:` (or filename auto-detection)
  selects how an upstream archive is unpacked. `tar.gz`/`tar.xz`/`tar.zst`/
  `tar`/`zip`/`raw` are all implemented (zip via the pure-Rust `zip` crate).
* **Signer** — the first backend whose `supports(format, method)` matches
  wins (`signer_for`). Detached backends write a sibling signature now
  (`gpg-detach`); embedded backends declare `embedded()` and the packager
  writes the signature while building the artifact (`rpm-pgp`,
  `deb-debsign`, and `apk-rsa` — Alpine's `.SIGN.RSA.<keyname>` over the
  control segment, signed in-process via the `rsa` crate). `msix-p7x` is
  embedded (the MSIX packager writes `AppxSignature.p7x` using the `msix`
  crate); `pkg-xar` is post-build (it rewrites the `.pkg` with
  `apple-xar`'s `XarSigner`). Both take a PEM key + X.509 certificate.
* **DependencyMapper** — each target format owns its package-name translation
  and version-operator syntax. Deb/RPM/Arch delegate to the shared
  ecosystem→Debian tables in `depmap.rs`; Alpine renders `name>=ver` and
  translates libc/runtime names; OpenWrt translates names but keeps opkg's
  Debian-style syntax.
* **PackageIndex** — one trait ([`PackageIndex`]) covering both sides of an
  index, with per-role defaults so a backend implements only the half it
  needs (`Capabilities::{READ, WRITE}`). **Write** (`lx repo --format
  deb|ipk|arch|apk|rpm`): writes the format's index (`Packages`/`Release`,
  `Packages`, `<repo>.db.tar.gz`, `APKINDEX.tar.gz`, `repodata/`); the apt
  backend delegates to the original `repo.rs`, the others read each
  artifact's metadata (`.ipk` control, `.PKGINFO`, `rpm -qp`) in-process.
  Index **signing** is per-format too (`sign_index`, run after
  `build_index`): apt `InRelease` (inline-clearsigned) + `Release.gpg`, opkg
  `Packages.sig`, pacman `<repo>.db.tar.gz.sig`, rpm
  `repodata/repomd.xml.asc`, and apk a prepended `.SIGN.RSA.<keyname>`
  segment (in-process RSA/SHA-1). **Read** (`lx index
  search/info/install/update`): fans out over every enabled source, looked
  up by canonical id in the registry (`get_index_backend`) rather than a
  hardcoded `match`; the LX community index and the AUR build/install,
  Repology is metadata-only and refuses `install`. Selection is by canonical
  id (`apt`, `opkg`, `pacman`, `apk`, `rpm`, `lx-community`, `aur`,
  `repology`); the `lx repo --format` vocabulary is an alias table
  (`FORMAT_ALIASES`: `deb→apt`, `ipk→opkg`, `arch→pacman`), and read
  backends expose the configured `indexes.yaml` name via `instance_name()`.
  `SourceKind::plugin_kind()` maps the config kind to the registry id
  (`custom` → the `GitIndexSource` git-recipe backend). Adding a backend is implementing
  `PackageIndex` and one line in `all_index_backends()`; the trait and impls
  live in `lib/plugins/package_index/` and the read backends are re-exported
  as `crate::index::<kind>` for the existing call sites.

