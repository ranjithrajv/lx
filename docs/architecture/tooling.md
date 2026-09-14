# Tooling: what `lx` consumes, what `lx` replaces

This document is the map of **runtime tool consumption** in `lx` — every
external binary and service the program invokes, and under what conditions —
together with the inverse question: **which existing tools `lx` is a
drop-in (or feature-parity) replacement for**, and where the parity stops.

It is also the "drop-in replacement" analysis referenced from
[`decisions/2026-09-14-shlibdeps.md`](../decisions/2026-09-14-shlibdeps.md).

The short version: `lx` builds every package format **in-process** — it
never shells out to `dpkg-deb`, `dpkg-source`, `rpmbuild`, `abuild`,
`makepkg`, `tar`, `gzip`, `xz`, `ar`, or `dpkg-shlibdeps`. External tools
are consumed only where there is no in-tree equivalent:

1. **Compilers** for `build_mode: source` (`cmake`, `cargo`, `go`, …).
2. **Language package managers** for `registry_source:` inputs (`npm`,
   `pip`, `cargo`, …).
3. **The host package manager** — to query ownership (`dpkg -S`,
   `rpm -qf`, `pacman -Qo`) and to apply the packages `lx` builds
   (`dpkg -i`, `rpm -U`, `pacman -U`); `lx` orchestrates them, it does not
   replace them.
4. **Signing / linting** (`gpg`, `cosign`, `lintian` — the last one has no
   Rust equivalent, deliberately).
5. **Index / migration lookups** (`git` for the community index;
   `snap`/`flatpak`/`nix` only to *migrate away* from them).

---

## Part 1 — Tools `lx` consumes

Legend:

- **Always** — a core path needs it unconditionally.
- **Feature** — required only when a flag/config selects the feature.
- **Optional** — used when present, degrades with a warning when absent.
- **Dev** — build/test/CI only, never part of a user's `lx` runtime.

### 1.1 Source-build toolchains (`build_mode: source`)

Selected by `build_system:` (explicit) or auto-detected from the source
tree (`lib/plugins/build_system/mod.rs`). Each plugin declares
`required_tools()`; `lib/sourcebuild.rs::require_tool` checks them on
`PATH` before compiling and fails closed with a clear message.

| Tool | Plugin | When / how |
|---|---|---|
| `cmake`, `ninja` | `cmake.rs` | `cmake -S … -B … -G Ninja`, `cmake --build`, `cmake --install` with `DESTDIR` |
| `cargo` | `cargo.rs` | `cargo install --path . --root <stage> --locked` |
| `rustup` | `cargo.rs` | **only with `musl: true`** — `rustup target add <triple>` |
| `go` | `go.rs` | `go build -trimpath -ldflags "-s -w"` |
| `meson`, `ninja` | `meson.rs` | `meson setup` / `meson compile` / `meson install --destdir` |
| `make` | `make.rs` | `make -jN PREFIX=/usr`, `make DESTDIR=… install` |
| `./configure`, `autoreconf`/`autogen.sh`, `make` | `autotools.rs` | bootstraps `configure` when the tarball didn't ship one |
| `musl-gcc` / `musl-g++` | `cmake.rs`, `autotools.rs` | **only with `musl: true`** (`musl-tools` package) |
| `sh` | `custom.rs`, `sourcebuild.rs` | runs `prebuild_steps`, `build_commands`, `install_commands` via `sh -c` |
| `pkg-config` | `builddeps.rs` | mapped to the host package (`pkgconf` on Arch/Alpine) and installed on request |

`--install-build-deps` additionally installs the missing toolchain and
`build_depends:` with the host package manager (`lib/builddeps.rs`).

### 1.2 Language registry toolchains (`registry_source:`)

Each plugin in `lib/plugins/registry/` declares `required_tools()`; the
build fails early if the tool is missing.

| Tool(s) | `registry_source` | Mechanism |
|---|---|---|
| `npm` | `npm` | `npm pack` + extract |
| `pip`, `python3` | `python` | `pip download --no-binary :all:` + extract |
| `gem` | `gem` | `gem fetch` + extract |
| `cargo` | `cargo` | `cargo install --root <dir>` |
| `nuget` | `nuget` | `nuget install` + extract |
| `mvn` | `maven` | `mvn dependency:copy-dependencies` |
| `composer` | `composer` | `composer install` |
| `cpanm`, `perl` | `cpan` | `cpanm` + build install tree |
| `go` | `go` | `go get` + `go build` (static, `CGO_ENABLED=0`) |
| `mix`, `elixir` | `hex` | `mix deps.get` + stage |
| `dart` | `dart` | `dart pub get` + stage |

### 1.3 Host package managers (query + install)

`lx` uses these to (a) detect what is installed, (b) resolve ELF sonames
to an owning package, (c) install its output, and (d) install build
dependencies on request. **It does not replace them.**

| Tool | Used by | What for |
|---|---|---|
| `dpkg` | `install_pkg.rs`, `debs.rs`, `builddeps.rs`, `cargo.rs` | `dpkg -i`, `-s` status, `--print-architecture`, `-S` ownership |
| `dpkg-query` | `debs.rs`, `scandeps.rs`, `index/mod.rs`, `elfdeps.rs` | `-W -f` ownership/version, installed-list |
| `rpm` | `install_pkg.rs`, `convert.rs`, `scandeps.rs`, `bindep.rs`, `elfdeps.rs`, `index/mod.rs` | `rpm -U` install, `-qp --queryformat` metadata, `-qf` ownership, `-q` status |
| `rpm2cpio` + `cpio` | `convert.rs` | payload extraction when **converting from** `.rpm` (`--to deb/arch`) |
| `pacman` | `install_pkg.rs`, `scandeps.rs`, `bindep.rs`, `elfdeps.rs`, `index/mod.rs` | `pacman -U` install, `-Qo` ownership, `-Qq` local list |
| `apt-get` | `debs.rs`, `install_pkg.rs`, `builddeps.rs`, `go_native.rs` | dependency-fix after `dpkg -i` (`install -f`), build-dep install, `remove` |
| `dnf` / `yum` | `builddeps.rs` | host build-dependency install |
| `zypper` | `builddeps.rs` | host build-dependency install |
| `apk` | `builddeps.rs` | host build-dependency install / `apk info -e` query |
| `xbps-install` / `xbps-query` | `builddeps.rs` | Void host build-dependency install/query |
| `sudo` | `install_pkg.rs`, `debs.rs`, `builddeps.rs`, `go_native.rs` | privilege escalation for every install/remove |
| `ldconfig` | `scandeps.rs`, `bindep.rs` | soname → filesystem path fallback |

Host-manager detection order lives in `builddeps.rs::HostPm::detect`
(`apt-get`, `dnf`, `yum`, `zypper`, `pacman`, `apk`, `xbps-install`).

### 1.4 Supply-chain, signing, and lint

| Tool | Module | Required? | Notes |
|---|---|---|---|
| `gpg` | `lib/sign.rs` | Feature | `signature:` on `.deb` (detach `<file>.sig`, or embedded `_gpgorigin` for `method: debsign`); RPM embeds natively. Also signs `lx repo`'s apt `InRelease` and the rpm/pacman/opkg indexes. Keys imported into a throwaway `GNUPGHOME`. |
| `cosign` | `lib/cosign.rs` | Feature (`--cosign`) | Sigstore keyless `sign-blob`; needs OIDC token. |
| `lintian` | `lib/lintian.rs` | Feature (`--lintian`) | **The one external tool with no Rust equivalent** — deliberately shelled out. Errors clearly if absent. |
| `patchelf` | `lib/relocatable.rs` | Optional | Sets `$ORIGIN/../lib` RPATH on ELF binaries. Warns and skips when absent; tries `patchelf` then `patchelf-stable`. |
| `cosign`/Rekor | (via `cosign`) | Feature | Transparency-log upload happens inside `cosign`. |

GPG passphrases come from `$LX_SIGN_PASSPHRASE` (falling back to
`$NFPM_PASSPHRASE`). Alpine apk package and `APKINDEX` signing is
**in-process** (the `rsa` crate, RSA/SHA-1) — no host `openssl` or
`abuild-sign`; see §1.8.

### 1.5 Conversion (`lx convert`)

| Source format | External tools | Notes |
|---|---|---|
| `.deb` → any | none | `ar`/tar/gzip/xz decoding is in-process |
| `.rpm` → any | `rpm` (`-qp --queryformat`), `rpm2cpio`, `cpio` | metadata + scriptlets + payload; no native `.rpm` reader |
| `.pkg.tar.zst` → any | none | `zstd` + `tar` crates in-process |

Target-format rebuilds always go through the in-process packagers
(`debarchive`, `rpmarchive`, `archarchive`).

### 1.6 Index, search, and migration

| Tool | Module | Purpose |
|---|---|---|
| `git` | `lib/plugins/package_index/lx_community.rs` | shallow clone / pull of the LX community recipe index; works offline on a stale cache |
| `snap` | `lib/go_native.rs` | `snap list` to detect; `snap remove` (apply); `apt-get remove snapd` (`--remove-manager`) |
| `flatpak` | `lib/go_native.rs` | `flatpak list --app`; `flatpak uninstall -y` |
| `nix` | `lib/go_native.rs` | `nix profile list`; `nix profile remove` |
| `dpkg-query` / `rpm` / `pacman` | `lib/index/mod.rs` | which packages the host already has (coverage/outdated) |
| `uname` | `lib/build.rs` (`--host`), `lib/install_pkg.rs` | host architecture, mapped per format |
| `nproc` / `free` / `df` | `lib/optimize.rs` | parallelism, memory, and free-disk hints (all optional, best-effort) |

### 1.7 Network services consumed

These are HTTP APIs and feeds, not local binaries. All go through
`lx_lib::http` (rustls, blocking reqwest) with a 5-minute JSON API cache.

| Service | Where |
|---|---|
| GitHub Releases API | `lib/github.rs`, `forge/github.rs` |
| GitLab Releases API (v4, self-hosted capable) | `lib/gitlab.rs`, `forge/gitlab.rs` |
| Gitea / Codeberg API (v1) | `lib/gitea.rs`, `forge/gitea.rs` |
| Forgejo API (Gitea-compatible) | `lib/forgejo.rs`, `forge/forgejo.rs` |
| Bitbucket Cloud downloads API | `lib/bitbucket.rs`, `forge/bitbucket.rs` |
| Gerrit REST | `lib/gerrit.rs`, `forge/gerrit.rs` |
| Gitee API v5 | `lib/gitee.rs`, `forge/gitee.rs` |
| SourceForge project RSS (files as pseudo-releases) | `lib/sourceforge.rs`, `forge/sourceforge.rs` |
| Repology API (`--distro`, `index update/outdated/coverage`) | `lib/plugins/package_index/repology.rs` |
| AUR RPC + cgit (`PKGBUILD` fetch) | `lib/plugins/package_index/aur.rs` |
| LX community index git repo | `lib/plugins/package_index/lx_community.rs` |
| Sigstore / Rekor | via `cosign` |
| Anonymous telemetry POST (GitHub Action only) | `action.yml` (curl), `lib/telemetry.rs` writes local metrics only |

`build_system: custom`, `prebuild_steps`, and the generated shell
installer run `sh`/`curl`/`wget` **in user-supplied or user-run scripts**,
not as `lx` machinery.

### 1.8 Not consumed (the in-process list)

For contrast, the formats and transformations `lx` deliberately performs
itself, with the crate that replaces the usual external tool:

| Would-be tool | In-process replacement |
|---|---|
| `dpkg-deb --build` | `lib/debarchive.rs` (`ar` + `tar` + `flate2`/`lzma-rust2`/`zstd`) |
| `dpkg-source -b` | `lib/source.rs` + `debarchive::tar_xz_tree` (verified field-for-field against a real `dpkg-source -b`) |
| `rpmbuild` | `lib/rpmarchive.rs` (`rpm` crate `PackageBuilder`) |
| `makepkg` / Arch `.pkg.tar.zst` | `lib/archarchive.rs` (`tar` + `zstd` level 19) |
| `abuild` / apk-tools | `lib/apkarchive.rs` (concatenated gzip control+data) + in-process RSA/SHA-1 signing (`rsa` crate, no `abuild-sign`/`openssl`) |
| opkg / `.ipk` | `lib/ipkarchive.rs` (reuses the deb `ar` layout) |
| `dpkg-shlibdeps` | `lib/shlibdeps.rs` (`object` crate: `DT_NEEDED` + `VERNEED`; parses `symbols`/`shlibs`) |
| `readelf` / `objdump` / `ldd` | `lib/elfdeps.rs` (`object` crate) |
| `dpkg-scanpackages` / `apt-ftparchive` / `reprepro` / `createrepo_c` / `abuild` | `lib/repo.rs` + `lib/plugins/package_index/` (apt `Packages`/`Packages.gz`/`Release`/`InRelease`, rpm `repodata/`, pacman `<repo>.db.tar.gz`, apk `APKINDEX.tar.gz`, opkg `Packages`) |
| `tar` / `gzip` / `xz` / `zstd` / `ar` | `tar`, `flate2`, `lzma-rust2`, `zstd`, `ar` crates |
| `sha256sum` / `sha512sum` / `md5sum` | `sha2`, `md5` crates |
| `jq` / `yq` / `envsubst` | `serde_json`, `serde_yaml`, in-tree env expansion |

Notable caveat: the `--sandbox` flag is documented as wrapping source
compiles in `unshare -n`, and `build.rs` carries that help text, but
**`unshare` is not invoked anywhere in the code today** — grep across
`lib/` finds only the doc comment. Treat `--sandbox` as declared-but-not-yet-wired.

### 1.9 Dev / CI tools (never part of a user's runtime)

From `Makefile` and `.pre-commit-config.yaml`:

- Rust: `cargo`, `rustc`, `rustfmt`, `clippy`.
- `pre-commit`; hooks: `shellcheck`, `typos`, `taplo`, `markdownlint`,
  `cargo-deny`, `cargo llvm-cov` (+ `utils/covscan`), `python3`
  (`utils/coverage-gaps.py`), and the in-repo `utils/license-check.sh`,
  `utils/secret-scan.sh`.
- GitHub Actions (`action.yml`): `dtolnay/rust-toolchain`,
  `Swatinem/rust-cache`, `actions/cache`, `actions/upload-artifact`, plus
  a conditional `apt-get install lintian` when `lintian-check: true`.
- Tests shell out to the **real `dpkg-deb`** to validate output
  (`tests/debarchive.rs`, `tests/plugins.rs`) — that is a test oracle, not
  a runtime dependency.

---

Continue to [replacements.md](replacements.md) for what `lx` replaces
(Parts 2–4), or back to the [docs index](../README.md).

---
