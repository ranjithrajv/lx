# Tooling: what `lx` consumes, what `lx` replaces

This document is the map of **runtime tool consumption** in `lx` — every
external binary and service the program invokes, and under what conditions —
together with the inverse question: **which existing tools `lx` is a
drop-in (or feature-parity) replacement for**, and where the parity stops.

It is also the "drop-in replacement" analysis referenced from
[`decisions/2026-09-14-shlibdeps.md`](decisions/2026-09-14-shlibdeps.md).

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
| `gpg` | `lib/sign.rs` | Feature | `signature:` on `.deb` (detach `<file>.sig`, or embedded `_gpgorigin` for `method: debsign`); also clearsigns `lx repo`'s `InRelease`. Keys imported into a throwaway `GNUPGHOME`. |
| `cosign` | `lib/cosign.rs` | Feature (`--cosign`) | Sigstore keyless `sign-blob`; needs OIDC token. |
| `lintian` | `lib/lintian.rs` | Feature (`--lintian`) | **The one external tool with no Rust equivalent** — deliberately shelled out. Errors clearly if absent. |
| `patchelf` | `lib/relocatable.rs` | Optional | Sets `$ORIGIN/../lib` RPATH on ELF binaries. Warns and skips when absent; tries `patchelf` then `patchelf-stable`. |
| `cosign`/Rekor | (via `cosign`) | Feature | Transparency-log upload happens inside `cosign`. |

GPG passphrases come from `$LX_SIGN_PASSPHRASE` (falling back to
`$NFPM_PASSPHRASE`).

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
| `git` | `lib/index/lx_community.rs` | shallow clone / pull of the LX community recipe index; works offline on a stale cache |
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
| Repology API (`--distro`, `index update/outdated/coverage`) | `lib/index/repology.rs` |
| AUR RPC + cgit (`PKGBUILD` fetch) | `lib/index/aur.rs` |
| LX community index git repo | `lib/index/lx_community.rs` |
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
| `abuild` / apk-tools | `lib/apkarchive.rs` (concatenated gzip control+data) |
| opkg / `.ipk` | `lib/ipkarchive.rs` (reuses the deb `ar` layout) |
| `dpkg-shlibdeps` | `lib/shlibdeps.rs` (`object` crate: `DT_NEEDED` + `VERNEED`; parses `symbols`/`shlibs`) |
| `readelf` / `objdump` / `ldd` | `lib/elfdeps.rs` (`object` crate) |
| `dpkg-scanpackages` / `apt-ftparchive` / `reprepro` | `lib/repo.rs` (`Packages`, `Packages.gz`, `Release`, `InRelease`) |
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

## Part 2 — What `lx` is a drop-in replacement for

"Drop-in" is used precisely below. Three levels appear:

- **Drop-in** — you can point existing config/workflows at `lx` without
  changing their interface.
- **Feature parity** — `lx` covers the same capability set, but through its
  own (different) CLI/config surface.
- **Functional replacement** — `lx` produces the same *artifacts* another
  tool would, without being interface-compatible.

### 2.1 Summary

| Tool / project | Relationship | Level | Evidence |
|---|---|---|---|
| `debian-multiarch-builder` GitHub Action | replaced | **Drop-in** (same inputs/outputs) | `action.yml`; `lx migrate` rewrites workflows |
| `dpkg-deb` | replaced for **building** | Functional | `lib/debarchive.rs`; real `dpkg-deb --info/--contents` accepts output |
| `dpkg-source` | replaced for source packages | Functional | `lib/source.rs`; `dpkg-source -x` reconstructs output |
| `dpkg-shlibdeps` | replaced by `lx shlibdeps` | **Drop-in** (command) | `lib/shlibdeps.rs`, decision doc |
| `debsign` / `debsigs` | replaced for signing | Compatible | `_gpgorigin` member; `signature.method: debsign` |
| `fpm` | replaced for the common path | Feature parity | [`comparison/fpm-vs-nfpm-vs-lx.md`](comparison/fpm-vs-nfpm-vs-lx.md) |
| `nfpm` | replaced on covered formats | Feature parity | [`comparison/lx-vs-nfpm.md`](comparison/lx-vs-nfpm.md) |
| `deb-get` | replaced (consumer client) | Feature parity | `lx get`, `show`, `search` "deb-get parity" |
| `makedeb` / AUR `makepkg` | replaced (import + native build) | Feature parity | `lx init --from-aur`, `lx index install` |
| `cargo-deb` / `cargo-dist` / `goreleaser` / `*2deb` | feature parity | Feature parity | checksum sidecars, shell installer, cosign, relocatable, SBOM |
| `dpkg-scanpackages` / `apt-ftparchive` / `reprepro` | replaced for repo publishing | Functional | `lx repo` |
| snap / flatpak / nix | migration source, not output | Functional | `lx go-native` |
| `dpkg-buildpackage` / `debian/rules` | **not replaced** | — | `shlibdeps` decision doc, scope note |
| `lintian` | **consumed, not replaced** | — | `lib/lintian.rs` |
| `dpkg` / `apt` / `rpm` / `pacman` | **consumed, not replaced** | — | install/query paths above |

### 2.2 `debian-multiarch-builder` — exact drop-in

`lx` is a Rust rewrite of the
[debian-multiarch-builder](https://github.com/ranjithrajv/debian-multiarch-builder)
bash GitHub Action, and `action.yml` is a **drop-in replacement** for its
`action.yml`: same input names (`config-file`, `version`, `build-version`,
`architecture`, `max-parallel`, `lintian-check`, `telemetry-enabled`,
`save-baseline`, `pinned-metadata`, `output-dir`, …) and same output names
(`packages`, `source-packages`, `summary-path`).

Behavioral parity is explicit, e.g. `--source` is always passed because
the bash action always generated source packages (no opt-in flag existed);
`--allow-unverified` defaults to `true` for bash-action parity. Legacy
`debian-multiarch-builder` `package.yaml` keys load as-is (`summary:` →
`description`, `license:` → `license_spdx`, `download_pattern:` +
`architecture_map:` → per-arch `release_pattern`s). `lx migrate` rewrites a
packaging repo's workflows (`...@v1` refs and `lpt build` → `lx build`).

### 2.3 `fpm` — feature parity

Covers the fpm surface most users need: deb/rpm/apk/arch, relation fields
(`Depends`/`Recommends`/`Suggests`/`Conflicts`/`Provides`/`Replaces`/`Breaks`/`Pre-Depends`),
pre/post-install/remove **and** pre/post-upgrade scripts, RPM triggers,
ERB-like `<%= key %>` script templating, debconf templates/config, deb
triggers, custom control fields, per-format overrides, globs in
`contents:`, and language-package inputs (npm/gem/python/cpan via
`registry_source:`). `lx` adds what fpm lacks: source packages, checksum
verification, SBOM/SLSA, reproducible builds, zero-config URL builds.

**Where fpm still wins:** `osxpkg`, `freebsd`, `solaris`, `snap`, `tar`,
`sh` (self-extracting), `zip` outputs; PEAR, virtualenv, pleaserun, puppet
inputs; `--deb-shlibs`, `--deb-init`, `--deb-systemd`, `--deb-upstart`,
`--deb-default`, `--deb-meta-file`, `--deb-after-purge`.

### 2.4 `nfpm` — feature parity

`lx` adopted nfpm's design directly (see
`docs/decisions/2026-08-20-nfpm-adoptions.md`,
`…-batch2.md`, `…-scripts-triggers-parity.md`): the `overrides:` block,
`arch_variant`, `umask`, `version_schema`, `packager`, vendor, per-format
script mapping, deb triggers, `deb.compression`, `rpm.compression` /
`auto_provides` / `auto_requires` / `defines`, `contents[].packager`,
`disable_globbing`, and the `debsign`-style `_gpgorigin` signature. The
GPG passphrase env var falls back to `$NFPM_PASSPHRASE` for nfpm parity.

**Where nfpm still wins:** `msix` (Windows) output; per-file
`file_info.mode/owner/group/lang`; `disown_subtree`; `expand: true`.
`lx` adds source packages, forged-release fetching, the consumer CLI, and
supply-chain features nfpm has no equivalent for.

### 2.5 `dpkg-deb` and `dpkg-source` — build-side functional replacement

`lx build` writes `.deb`s entirely in-process (`lib/debarchive.rs`): `ar`
with `debian-binary` + `control.tar.{gz,xz,zst}` + `data.tar.{gz,xz,zst}`,
sorted walk, normalized mtime/uid/gid, deterministic compression. It never
runs `dpkg-deb` or `tar`/`gzip`.

`--source` writes `.dsc` + `.orig.tar.xz` + `.debian.tar.xz` in-process
(`lib/source.rs`), with the `.dsc` field order and checksum-section
ordering verified against a real `dpkg-source -b` run, and the xz preset
matching `dpkg-source`'s default. Real `dpkg-deb --info`/`--contents` and
`dpkg-source -x` accept the output (used as test oracles and in the
decision docs' live verification).

**Scope limit:** this replaces the *builder*, not a general-purpose
reader/patch-stack maintainer. `debarchive::extract` is scoped to what
`lx` itself produces (gzip data.tar), and `lx` only ever emits the
`3.0 (quilt)` with-empty-patch-stack case.

### 2.6 `dpkg-shlibdeps` — command drop-in

`lx shlibdeps <path>…` is described by its decision doc as "a drop-in for
`dpkg-shlibdeps` itself, not for `dpkg-buildpackage`". It parses the
binary's `DT_NEEDED` plus `(symbol, version)` requirements from
`.gnu.version_r`/`VERNEED` and reads the dpkg `symbols`/`shlibs` databases
to emit **versioned** relations (`libcap2 (>= 2.66)`), fail-closed without
`--ignore-missing-info`. Flags and semantics not covered yet: virtual
`Provides`, multiarch `pkg:arch` qualifiers, `debian/shlibs.local`,
full `-e`/`-T`/`-O`/`-d`/`-p`/`-l` parity.

The build pipeline uses the same core best-effort (falling back to
`dpkg -S`/`rpm -q --whatprovides`/`pacman -Qo`), so no existing build
starts failing.

### 2.7 `debsign` / `debsigs` (and not-yet `dpkg-sig`)

`signature.method: debsign` embeds an armored detached signature as the
`_gpgorigin` member (role selectable via `type: origin|maint|archive`),
compatible with debsigs/nfpm readers. `method: detach` writes a sibling
`.sig`. RPM signatures embed natively via the `rpm` crate.

**Not yet:** a `dpkg-sig` method (true `gpg --clearsign` of the dpkg-sig
control template → `_gpgbuilder`) and a live `debsig-verify`/`debsigs
--verify` test — tracked as follow-up in
`decisions/2026-08-28-debsign-local-payload.md`.

### 2.8 `deb-get` — consumer client parity

`lx`'s consumer subcommands mirror deb-get: `lx install/upgrade/update/
remove/list/show/search/reinstall/rollback`, plus `lx get <subcommand>` as
a thin build-free client. `lx show` and `lx search` are explicitly
documented as deb-get `show`/`search` parity (`lib/show.rs`,
`lib/search.rs`), with `lx search` adding `apt search`-style full-text over
installed dpkg long descriptions and exact-match-first ordering. `lx get
install` falls back to a `+musl_{arch}.deb` asset when no distro-specific
build exists.

Deliberately *not* copied from deb-get: its HTTPS-only trust model — `lx`
keeps fail-closed checksum verification instead.

### 2.9 `makedeb` / AUR (`makepkg`) — import and native build

`lx init --from-aur <pkg>` converts an AUR `PKGBUILD` into a starter
`package.yaml`, and `lx index install <aur-pkg>` fetches the `PKGBUILD`
and builds it through the normal source path — so an AUR package becomes a
native `.deb`/`.rpm`/`.pkg.tar.zst` without `makepkg`. Deliberate
difference from makedeb: recipes are **never executed** — PKGBUILD shell
becomes comments, not code (`lib/wizard.rs`, `lib/index/aur.rs`). Arch
dependency names are kept verbatim for downstream mapping.

### 2.10 `cargo-deb` / `cargo-dist` / `goreleaser` / `*2deb`

The CHANGELOG records "five new features for goreleaser/cargo-dist/*2deb
parity":

1. `.sha256`/`.sha512` sidecars for every artifact.
2. A generated `<package>-install.sh` (`curl | sh`, distro/arch detection,
   checksum verification, native install) — `lib/shell_installer.rs`.
3. `--cross-target <ARCH>` cross-compilation (auto-enables musl-static).
4. `--cosign` Sigstore keyless signing.
5. Dependency mapping from registry manifests across 9 ecosystems.

Plus relocatable binaries via `patchelf` `$ORIGIN/../lib` RPATH
(cargo-dist parity) and SPDX 2.3 + SLSA-v1-shaped provenance (`--sbom`) —
which none of the comparison tools emit. Note that the individual
`cargo-deb`/`cargo-dist` CLIs are **not** interface-compatible targets;
this is feature parity, not a drop-in for their command lines.

### 2.11 snap / flatpak / nix — migration, not output

`lx go-native` consumes `snap`/`flatpak`/`nix` to **detect and migrate
away from** them: parse their listings, map each finding to an `lx`
package name, install the native package, and (on request) remove the
non-native source. Unmapped findings land in a `missingnative` report
instead of being dropped. This is the inverse of a drop-in replacement:
`lx` is the destination.

### 2.12 `dpkg-scanpackages` / `apt-ftparchive` / `reprepro` — functional replacement

`lx repo <dir>` turns a directory of built `.deb`s into an apt-servable
repository (`Packages`, `Packages.gz`, `Release` with MD5/SHA1/SHA256, and
a clearsigned `InRelease` when a key is given), with a `--multi-suite`
layout mirroring real Debian/Ubuntu archives. The `latest-debs` apt repo +
`lx install` forms a complete producer→distributor loop. It is a
functional replacement, not a flag-compatible `reprepro` (no snapshotting,
no CDN invalidation, no pool management).

---

## Part 3 — Deliberate non-replacements

These are consumed on purpose and will not be reimplemented:

- **`lintian`** — a large Debian-native Perl tool with no Rust equivalent;
  `--lintian` runs the real binary and errors clearly when absent.
- **`gpg`** — OpenPGP signing; the trade-off is stated in `lib/sign.rs`
  ("`gpg` is on every CI runner that matters").
- **`cosign`** — Sigstore keyless signing and Rekor upload.
- **`patchelf`** — RPATH rewriting; optional, best-effort.
- **`dpkg`/`apt`/`rpm`/`pacman`/`dnf`/`zypper`/`apk`/`xbps`** — `lx`
  orchestrates the host package manager; it is an apt-like *front end* for
  forge-release software, not an installer backend.
- **`rpm`/`rpm2cpio`/`cpio`** — only for reading `.rpm` files during
  `lx convert`; there is no native RPM reader yet.
- **`git`** — only to clone/pull the LX community recipe index.
- **`dpkg-buildpackage` / `debian/rules`** — out of scope; `lx shlibdeps`
  replaces `dpkg-shlibdeps`, not the whole build.

---

## Part 4 — Cheat sheet

**What `lx` needs on `PATH` for each job**

| Job | External requirements |
|---|---|
| `lx build <forge url>` (binary repack) | *nothing* |
| `lx build --sbom` (SBOM + SLSA provenance) | *nothing extra* |
| `lx build --lintian` (deb only) | `lintian` |
| `lx build --source` (any `build_system`) | the system's `required_tools()` (cmake+ninja, cargo, go, meson+ninja, make) |
| `lx build --source --install-build-deps` | a host package manager (+ `sudo` unless root) |
| `lx build --sign-key …` | `gpg` |
| `lx build --cosign` | `cosign` (+ OIDC) |
| `lx build` staging ELF binaries (automatic RPATH patch) | `patchelf` (optional; warns and skips) |
| `lx build` with `registry_source:` (npm, python, gem, cargo, nuget, mvn, composer, cpan, go, hex, dart) | that ecosystem's tool (`npm`, `pip`+`python3`, `gem`, `cargo`, `nuget`, `mvn`, `composer`, `cpanm`+`perl`, `go`, `mix`+`elixir`, `dart`) |
| `lx install`/`upgrade`/`remove` | `dpkg`/`apt-get` (deb), `rpm` (rpm), `pacman` (arch) + `sudo` |
| `lx convert` from `.rpm` | `rpm`, `rpm2cpio`, `cpio` |
| `lx convert` from `.deb`/Arch | *nothing* |
| `lx repo --sign-key` | `gpg` |
| `lx shlibdeps` | *nothing* (reads dpkg `symbols`/`shlibs`) |
| `lx scan-deps` / `--bindep` | `dpkg`/`rpm`/`pacman` and/or `ldconfig` for package-name resolution (optional; sonames come from in-process ELF parsing) |
| `lx go-native --yes` | `snap`/`flatpak`/`nix` (for the sources being migrated) + the host installer |
| `lx index` | `git` (LX community index); HTTP for AUR/repology |

**What `lx` produces without every one of those tools:** every package
format (deb/rpm/arch/apk/ipk), source packages (`.dsc`, `.src.rpm`,
`PKGBUILD`), SBOM/SLSA, checksum sidecars, shell installers, and an
apt repository — all in-process.
