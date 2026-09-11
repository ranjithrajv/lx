# lx — build, install, and distribute Linux packages

`lx` builds installable packages from forge release binaries *or* from
source (GitHub, GitLab, Gitea, Forgejo, Bitbucket, Gerrit) across Debian
suites and architectures (`.deb`), RPM distros (`.rpm`), and Arch
(`.pkg.tar.zst`), and installs/upgrades/removes the packages it builds — an
apt-like front end for software that only ships forge releases, plus the
tooling to run a prebuilt apt repository from its output. It's a Rust
rewrite of the
[debian-multiarch-builder](https://github.com/ranjithrajv/debian-multiarch-builder)
GitHub Action, usable as a local CLI (`lx`, with the thin `lx-get`
consumer client), as a GitHub Action (`action.yml`, see below), and as a
repo publisher (`lx repo`).

Every package it produces is a real Debian package, not a thin ZIP-in-an-ar
wrapper: full dependency relations (`Depends`/`Recommends`/`Conflicts`/
`Replaces`/`Provides`/`Breaks`), epoch-aware versioning, man pages and
license files placed at their conventional FHS paths, and genuine `xz`
source-package compression — all output that real `dpkg-deb`/`dpkg-source`/
`lintian` accept without complaint, built natively on bare metal — no containers, no emulation, no Debian host required.

It's also trustworthy by default about what it downloads and repackages:
a build refuses to proceed on an unverified asset unless you explicitly
say otherwise, every download's verification method and checksum are
recorded for later audit, and pinning a prerelease or draft tag gets you
a warning instead of a silent surprise.

## Requirements

- Rust (to build `lx` itself; see [Development](#development)). That's
  it — **`lx` builds natively on bare metal:**
  - `lx build` writes `.deb`s in-process (`lib/debarchive.rs`); no
    `dpkg-deb`, no Debian toolchain, and cross-architecture packaging
    only copies/`chmod`s the target binary — it's never executed, so any
    host can package any architecture.
  - `--source` writes source packages (`.dsc`/`.orig.tar.xz`/
    `.debian.tar.xz`) in-process too — no `dpkg-source`.
  - `--lintian` shells out to a `lintian` binary on your `PATH` (e.g.
    `apt-get install lintian` on Debian/Ubuntu): the one external tool
    with no Rust equivalent.

  See
  [`docs/decisions/2026-08-20-docker-free-deb-build.md`](docs/decisions/2026-08-20-docker-free-deb-build.md)
  and
  [`docs/decisions/2026-08-20-docker-free-lintian-source.md`](docs/decisions/2026-08-20-docker-free-lintian-source.md)
  for how, and the trade-offs involved.

## Quick start

Build every architecture a forge release publishes, no config file needed:

```sh
lx build https://github.com/eza-community/eza
```

Build for just this machine's own architecture (native-only):

```sh
lx build package.yaml --host
```

Package files you supply — no forge release, minimal config (fpm-style
"you supply files" mode):

```sh
lx build --from-dir ./myapp-dist/ --package-name myapp --version 1.0.0
lx build --from-file ./myapp --package-name myapp --version 1.0.0
lx build --from-dir ./dist/ --package-name myapp --version 1.0.0 --prefix /usr/local/bin
```

With a base `package.yaml` for extra metadata (dependencies, contents,
signing, etc.):

```sh
lx build package.yaml --from-dir ./myapp-dist/
```

Generate a `package.yaml` interactively, with auto-discovered release
patterns:

```sh
lx init
```

Or start from one of the bundled debian-multiarch-builder templates:

```sh
lx init --template rust/eza    # or go/hugo, c/neovim, python/generic, …
```

(An unknown name lists all of them; see [`templates/README.md`](templates/README.md).)

## Commands

| Command | Purpose |
|---|---|
| `lx build [config]` | Build `.deb`s (and optionally source packages) from a `package.yaml`, zero-config from a GitHub URL, or from files you supply (`--from-dir`/`--from-file`) |
| `lx validate [config]` | Check a config resolves against a real release, without building |
| `lx discover <owner/repo> [version]` | Auto-discover release-asset patterns and print a starter config |
| `lx scan-deps [config]` | Report a release binary's shared-library dependencies, to verify/fill in `depends:` |
| `lx init` | Interactively generate a `package.yaml`, with optional auto-discovery |
| `lx install <package>` | Fetch and install a pre-built `.deb` from the `latest-debs` GitHub org |
| `lx update [package]` | Check installed packages against their latest release, no install |
| `lx upgrade [package]` | Upgrade installed packages to their latest release |
| `lx remove <package>` | Remove (or `--purge`) an installed package |
| `lx list` | List packages `lx` has installed |
| `lx show <package>` | Show everything known about one package (manifest + dpkg) |
| `lx reinstall <package>` | Reinstall the recorded version of an `lx`-managed package |
| `lx rollback <package>` | Reinstall a prior generation of an `lx`-managed package |
| `lx search [pattern]` | Full-text regex search like `apt search`: name + descriptions (including installed packages' dpkg long descriptions), installed/candidate versions, exact matches first; `--local` searches the offline starter-template index |
| `lx repo <dir>` | Turn a directory of `.deb`s into an apt-servable repository (`Packages`/`Release`/`InRelease`) |
| `lx migrate [--repo DIR]` | Carry legacy `lpt` state (manifest, caches) and workflows to `lx` |
| `lx index <cmd>` | Unified package-index manager — AUR, LX community index, and custom indexes (search/install/info/update/add/remove/list) |
| `lx go-native` | Migrate snap/flatpak/nix/`curl \| sh` installs to native packages (plan by default, `--yes` to apply) |

`lx-get` is a thin companion binary with the consumer half only —
`install`/`upgrade`/`update`/`remove`/`show`/`reinstall`/`list`/`search`,
no build machinery. Same manifest, same org:

```sh
lx-get install eza
lx-get upgrade --owned-only   # skip entries removed outside lx
```

### `lx go-native`

Migrate snap, flatpak, nix, and `curl … | sh` installs to **native** packages (`.deb` on dpkg hosts, with
`--format rpm|arch` planning elsewhere). Plan by default — nothing is
installed or removed until you pass `--yes`:

```sh
lx go-native                  # plan: detect + map, print only
lx go-native firefox          # plan, filtered to matching ids
lx go-native --yes            # apply: install natives, remove sources
lx go-native --yes --keep-source      # install natives, keep both
lx go-native --from flatpak,nix --skip-sh   # managed sources only
```

How it works: `snap list` / `flatpak list --app` / `nix profile list` are
parsed (snap runtimes like `core22` are excluded — they'll never have a
native equivalent), and `curl | sh` installs are found by scanning
`/usr/local/bin`, `~/.local/bin`, and `/opt` for binaries no native manager
claims (`dpkg -S` / `rpm -qf` / `pacman -Qo`), with installer URLs
attributed from shell history. Each finding is mapped through a built-in
table to an `lx` package name; anything unmapped lands in a
`missingnative` report instead of being silently dropped.

Safety rules: `curl | sh` orphans are **never auto-deleted** — the plan
prints manual `rm` cleanup for you to review. Applying is currently
deb-host-only (the `lx install` backend); elsewhere the plan doubles as a
shopping list. `--remove-manager` offers to drop `snapd` itself once
everything from it migrated (always confirms).

Run `lx <command> --help` for the full flag reference. A few worth calling
out:

- **`--host`** (`build`): auto-detects this machine's architecture (`uname
  -m`) and builds only that one, natively. Conflicts with
  `--architectures`.
- **`--local`** (`build`): package a local archive or directory from
  `local_payload:` in package.yaml, skipping the upstream download.
  Requires `version:` (or `--version`).
- **`--from-dir <path>`** (`build`): "you supply files" mode — build a
  package from a directory of files you supply, with no forge fetch.
  Requires `--package-name` and `--version` if not given in `package.yaml`.
  Files are staged into the package (ELF binaries → `/usr/bin`, or
  `--prefix`). An existing `package.yaml` is loaded as a base for extra
  metadata (dependencies, contents, signing) but `github_repo` is not
  required. Conflicts with `--local`.
- **`--from-file <path>`** (`build`): like `--from-dir` but for a single
  file. The file is installed to `/usr/bin` (or `--prefix`). Conflicts with
  `--from-dir`.
- **`--package-name <name>`** (`build`): override or set the package name
  for `--from-dir`/`--from-file` builds (and zero-config builds).
- **`--prefix <path>`** (`build`): install prefix inside the package for
  `--from-dir`/`--from-file` (e.g. `"/usr/local/bin"`, `"/opt/myapp"`).
  Files are staged under this absolute path instead of the default
  `/usr/bin`. Must start with `/`.
- **`--sign-key` / `--sign-method`** (`build`): sign built packages.
  Method `detach` (default) writes a sibling `.sig`; `debsign` embeds
  `_gpgorigin` inside the `.deb` (debsigs / nfpm-compatible). RPM embeds
  natively either way. Passphrase via `$LX_SIGN_PASSPHRASE` or
  `$NFPM_PASSPHRASE`.
- **A GitHub URL in place of a config file** (`build`): `lx build
  https://github.com/<owner>/<repo>` needs no `package.yaml` at all — builds
  every architecture the release publishes, with source packages included.
- **`--pinned-metadata`** (`build`): verify downloaded assets against a
  `release-metadata.json` provenance pin captured at vet time, instead of
  (or in addition to) the release's own live checksum sidecar.
- **`--source`** (`build`): also generate a Debian source package (`.dsc` +
  `.debian.tar.xz` + a shared `.orig.tar.xz`) per distribution.
- **`--sbom`** (`build`): also emit `<pkg>_<ver>.spdx.json` (SPDX 2.3 SBOM
  over built artifacts + upstream materials) and `<pkg>_<ver>.slsa.json`
  (SLSA v1-style provenance) into the output dir.
- **`--sandbox`** (`build`, source mode only): run compile steps under
  `unshare -n` (no network, private mounts) when the kernel permits, else
  warn and run unsandboxed. Binary repacks ignore it — they execute nothing.
- Version/architecture/distribution resolution, checksum verification, and
  reproducible-build hygiene (below) are all shared machinery — see
  `--dry-run` to preview a build matrix without downloading or building
  anything.

**Checksum verification is required by default.** When neither
`--pinned-metadata` nor a live `.sha256`/`.sha256sum` sidecar is available
for a downloaded asset (most forge releases don't publish one), `lx
build` fails rather than silently continuing — pass `--allow-unverified` to
build anyway. `--no-verify` skips verification entirely, sidecar or not.
`lx build`/`lx validate` also flag prerelease and draft releases (e.g.
pinning `version: nightly` on a repo whose "latest" tag is an RC) so you
don't package pre-stable software without noticing.

With `--summary`, `build-summary.json` gets a `provenance` array — one
entry per unique asset downloaded, with the verification method used
(`pinned`/`sidecar`/`unverified (--allow-unverified)`/`skipped
(--no-verify)`), its resolved SHA-256, and the source URL — plus an
`unverified_assets` count, so a build can be audited after the fact instead
of just trusting a console log that's already scrolled away.

`lx install`/`lx upgrade` apply the same fail-closed default against the
release's own sidecar (there's no pin file in that flow): no sidecar means
no install unless you pass `--allow-unverified` there too.

`--summary` also prints a markdown badge block for your packaging repo's
own README: a "Built with lx" badge, a suites/architectures-coverage
badge pair, and — only when running as the GitHub Action, where
`GITHUB_REPOSITORY` identifies the repo publishing the release —
latest-release and downloads badges too. Never guessed on a local run.

`install`/`update`/`upgrade`/`remove`/`list` track what they manage in a
local manifest (`installed.json` under your XDG data dir), cross-checked
against `dpkg`'s own record of what's actually installed — `lx` is never
the sole source of truth for what's on your system.

## `package.yaml` reference

```yaml
package_name: eza          # required
github_repo: eza-community/eza   # required (--from-dir/--from-file omit this)

artifact_format: tar.gz     # tar.gz | tgz | zip | raw (guessed if omitted)
description: "A modern replacement for ls"
maintainer: "Jane Doe <jane@example.com>"
license_spdx: MIT           # falls back to the upstream repo's detected license

debian_distributions: [bookworm, trixie, forky, sid]   # default: all five suites

binary_path: ""              # path to the binary within the extracted archive
binary_rename: ""            # rename the single installed binary to this name
bundle: false                 # see below
prefix: ""                   # --from-dir/--from-file: install prefix inside the package (e.g. "/usr/local/bin")

depends: ""                   # e.g. "libatomic1, libgtk-3-0"
recommends: ""                 # e.g. "bash-completion"
conflicts: ""                  # e.g. "eza-legacy"
replaces: ""                   # e.g. "eza-legacy"
provides: ""                   # e.g. "eza-cli"
breaks: ""                     # e.g. "eza-legacy (<< 2.0)"

version: ""                   # pin a specific upstream version (else: latest)
build_version: "1"            # Debian revision
epoch: ""                     # e.g. "1" -- for upstream version-numbering resets

# Local-only packaging (lx build --local); skips the forge download.
# Existence is checked at build time. ${VAR} / ${VAR:-default} expand at parse.
local_payload: ""             # path to an archive or directory

signature:
  key_file: ""                # ASCII-armored secret key (env-expandable)
  key_id: ""                  # optional gpg --local-user
  method: detach              # detach (sibling .sig) | debsign (embedded _gpg{type})
  type: origin                # debsign role: origin | maint | archive

# Source builds (build_mode: source): fetch the upstream tag and compile on
# the host instead of repacking release assets. Requires an explicit
# architectures: list (all entries must equal the host arch — native only).
build_mode: source            # binary (default) | source
build_system: cmake           # cmake (default) | cargo | go | custom (omit = auto-detect)
musl: false                    # musl-static binary: no glibc dep, runs on any Linux
upstream_url: ""              # tarball root; default: github <repo>/archive
upstream_ref: ""              # tag to fetch; default: resolved version
build_depends: []             # host packages the compile needs (CI preinstalls; lx never apt-gets)
cmake_flags: []               # extra cmake configure flags
prebuild_steps: []            # sh steps in the source dir after unpack, before configure
build_commands: []            # custom build steps (build_system: custom)
install_commands: []          # custom install steps into $DESTDIR (custom; required)
build_suites: []              # suites to build; default: configured distributions
skip_suites: []               # suites to skip
```

**Legacy debian-multiarch-builder configs load as-is.** Every key the bash
action's templates and zero-config wizard emitted is accepted and folded
into its modern equivalent — `summary:` → `description`, `license:` →
`license_spdx`, `vendor:` → a `Vendor` control field,
`dependencies:` (list) → `depends`, and `download_pattern:` +
`architecture_map:` expanded into per-arch `release_pattern`s when no
modern `architectures:` block is present (`{version}`/`{arch}`/
`{package_name}` placeholders supported). Modern keys win whenever both are
set. The documented-but-unimplemented upstream knobs are real here too:
`distribution_arch_overrides` replaces the built-in arch/suite matrix for a
named architecture, and package.yaml-level `max_parallel` /
`parallel_builds: false` set parallelism defaults that an explicit
`--max-parallel` always beats.

**Expired suites drop out automatically**, mirroring the action's
`filter_expired_distributions`: once Debian's LTS support for a suite ends
(bullseye: 2026-08-31), it stops being built even if listed — by config
default or explicit `--distributions` alike.

**`architectures:`** — omit entirely for full auto-discovery across every
supported architecture. Two explicit forms:

```yaml
# Pin exact release assets per architecture:
architectures:
  amd64:
    release_pattern: "eza_x86_64-unknown-linux-gnu.tar.gz"
  arm64:
    release_pattern: "eza_aarch64-unknown-linux-gnu.tar.gz"

# Or just restrict auto-discovery to a named subset:
architectures: [amd64, arm64, armhf]
```

**`bundle: true`** — for upstreams that ship a full install tree rather than
standalone binaries (e.g. `zed-industries/zed`'s `zed.app/{bin,lib,libexec,share}`,
where the launcher's RPATH is `$ORIGIN`-relative and only works if the tree
stays intact). Installs the whole `binary_path` tree under
`/usr/lib/<package_name>/` and symlinks its executables into `/usr/bin`,
instead of flattening loose files there.

**`depends:`** — for binaries needing a runtime library a bare Debian
install doesn't have by default (e.g. pnpm's Node single-executable binary
needs `libatomic1`). Emits a `Depends:` control-file line. Run `lx
scan-deps` first to see exactly which shared libraries the actual release
binary needs (parsed natively from its ELF `DT_NEEDED` entries, no
`ldd`/`objdump` required) rather than guessing -- it flags which ones are
just glibc/essential and, when `dpkg` is available locally, best-effort
resolves the rest to an owning package via `dpkg -S`.

**`recommends:`/`conflicts:`/`replaces:`/`provides:`/`breaks:`** — the rest
of Debian's dependency-relation fields, each emitted only when non-empty.
Useful for packages superseding an older name, providing a virtual package,
or needing a version-gated incompatibility declared up front rather than
discovered at install time.

**`epoch:`** — set when an upstream project resets or renumbers its own
versioning (e.g. `1.0.0` after a `2024.03` calendar-versioned run) such
that plain string comparison would otherwise sort the new release as
*older*. Appears in the `Version:` field (`<epoch>:<version>`) but never in
filenames, per Debian policy.

**`local_payload:` + `--local`** — package an already-downloaded archive or
an extracted directory without hitting GitHub/GitLab. Requires `version:`
(or `--version`). Path existence is checked at build time, not parse time.

**`signature:`** — sign built packages. For `.deb`, `method: detach`
(default) writes `<pkg>.deb.sig` beside the artifact; `method: debsign`
embeds an armored detach-signature as `_gpg{type}` (default `_gpgorigin`;
`type:` may be `origin` / `maint` / `archive`). For `.rpm`, the signature
is always embedded in the header when `key_file` is set. `${VAR}` /
`${VAR:-default}` expand in the YAML at parse time (e.g.
`key_file: ${SIGNING_KEY_FILE}`).

**`contents[].packager`** — restrict an overlay entry to one format
(`deb` / `rpm` / `arch`). Omit to apply to every format.

**`build_mode: source`** — for upstreams that publish no Linux binaries
(e.g. quickshell). Fetches the source tag, compiles once on the host
using the selected `build_system` plugin, computes `Depends` from the
staged ELFs (`DT_NEEDED` → owning host packages via `dpkg -S`, `libc6`
fallback), and wraps one `.deb` per suite — natively, no containers.
Native-arch only: every `architectures:` entry must equal the host arch
(run once per native host, like one matrix cell per runner). The host
glibc is the symbol floor, so build on the oldest suite you ship.
`build_depends_suites`/`build_apt_sources` are accepted for
debian-multiarch-builder config compat but not applied (container-only
concepts); `build_depends` names host packages instead.

**`build_system:`** — which build system compiles the source. Defaults to
`cmake` (auto-detected from `CMakeLists.txt` if `build_system:` is omit).

| `build_system` | Plugin | Detects | Build |
|---|---|---|---|
| `cmake` | CMake + Ninja | `CMakeLists.txt` | cmake configure → build → DESTDIR install |
| `cargo` | Cargo (Rust) | `Cargo.toml` | `cargo install --path . --root <DESTDIR>` |
| `go` | Go | `go.mod` | `go build -trimpath` → `<DESTDIR>/bin/<name>` |
| `custom` | User commands | (explicit-only) | `build_commands` / `install_commands` with `$DESTDIR` |

If `build_system:` is omitted, `lx` auto-detects from the source tree
(`CMakeLists.txt` → cmake, `Cargo.toml` → cargo, `go.mod` → go). Set it
explicitly to override or to use `custom`.

**`musl: true`** — produce a musl-static binary with no glibc dependency,
so the package runs on any Linux regardless of distro age (solves the
"binary built on new Ubuntu won't run on old Ubuntu" problem). For source
builds, each build system plugin adjusts its compile flags:

| `build_system` | Musl mechanism |
|---|---|
| `cargo` | `--target x86_64-unknown-linux-musl` (auto-installed via rustup) |
| `go` | `CGO_ENABLED=0` (fully static, no C dependencies) |
| `cmake` | `musl-gcc`/`musl-g++` + `-static` (requires `musl-tools`) |
| `custom` | user's responsibility via `build_commands` |

For binary repacks (`build_mode: binary`, the default), `musl: true`
prefers musl-named release assets (e.g. `*-linux-musl.tar.gz`) over glibc
variants during auto-discovery. `compute_depends()` omits the `libc6`
fallback for musl binaries. The consumer client (`lx-get install`) falls
back to a `+musl_{arch}.deb` asset when no distro-specific build exists.

**`lx init --from-aur <pkg>`** — convert an AUR PKGBUILD into a starter
`package.yaml` (makedeb-orphan migration path). Guesses are commented for
review: the `github_repo` guess (loud `FIXME` when the AUR URL isn't
GitHub), Arch dependency names kept verbatim for Debian mapping, and
`build()` presence mapped to `build_mode: source` hints. Recipes are never
executed — PKGBUILD shell becomes comments, not code.

**`lx repo <dir>`** — turn built `.deb`s into an apt-servable repository:
`Packages` + `Packages.gz` (control fields read natively), `Release`
(MD5/SHA1/SHA256), and clearsigned `InRelease` with `--sign-key`. Serve
`<dir>` over HTTP and point `sources.list` at it — the
producer→distributor loop with `lx install` as the client.

Man pages (`*.1`–`*.9`, gzipped) and license files (`LICENSE`/`COPYING`/
`NOTICE`, any casing) sitting alongside the binary in a flat-mode release
are auto-installed to `/usr/share/man/man<N>/` and `/usr/share/doc/<pkg>/`
— no config needed. (Bundle-mode installs already preserve everything in
the upstream tree.)

## GitHub Action

`action.yml` wraps `lx build` as a composite action — a drop-in
replacement for `debian-multiarch-builder`'s action.yml (same input/output
names):

```yaml
- uses: ranjithrajv/lx@v1
  with:
    config-file: package.yaml
    version: v0.24.0
    build-version: '1'
    # architecture: all          # or a single arch; default: all
    # lintian-check: 'true'
    # pinned-metadata: release-metadata.json
```

Outputs: `packages` (space-separated `.deb` filenames), `source-packages`
(`.dsc` filenames), `summary-path` (`build-summary.json`). It needs no host
`apt-get` dependency install at all — building, source packages, and
`lintian` (when the runner has one) are all native or host-tool-backed
rather than shelled-out `tar`/`jq`/`yq`/`dpkg-*` in a container. Since this
repo has no published binary releases yet, the action builds `lx` from
source (cached via `Swatinem/rust-cache`).

### Attribution & telemetry

Pin an exact, greppable ref so usage is attributable via code search
(`uses: ranjithrajv/lx@v1`):

```yaml
- uses: ranjithrajv/lx@v1   # keep major tag moving; cite full version in issues
```

Anonymous run telemetry is opt-out (`telemetry-enabled: 'true'` default):
one POST per run with `action ref, arch, job status, runner arch` only —
no repo, user, or PII. It fires only when the `LX_TELEMETRY_URL` repo
Variable is set (your collector endpoint), so forks/private users emit
nothing by default. Set `telemetry-enabled: 'false'` to disable entirely.

## Reproducible builds

Builds are reproducible per
[Debian's definition](https://wiki.debian.org/ReproducibleBuilds): building
the same input twice, at different times, produces byte-identical output —
verified empirically (see
[`docs/decisions/2026-08-20-reproducible-builds.md`](docs/decisions/2026-08-20-reproducible-builds.md)
for the investigation). Package
metadata timestamps (changelog date, copyright year) come from the GitHub
release's own publish time rather than build time, and the source-package
`.orig.tar.xz`/`.debian.tar.xz` members are built (`lib/debarchive.rs`)
walking their contents in sorted order with normalized mtime/owner/group.
Both respect the standard `SOURCE_DATE_EPOCH` environment variable if you
want to pin an exact value.

## Package indexes (`lx index`)

A unified index manager: one command, many upstream indexes. `lx index` fans
out across every *enabled* source — the
[LX community index](https://github.com/ranjithrajv/lx-index) (recipes +
prebuilt binaries), the [AUR](https://aur.archlinux.org/) (builds PKGBUILDs
into native packages), and any custom index you register. New sources are
**plugins**: implement the `IndexSource` trait and add one line to the registry
— no fork, no recompile of core.

```sh
lx index search eza           # full-text across ALL enabled indexes
lx index install eza          # prebuilt first (LX index), build if no match
lx index info eza             # details from every index that has it
lx index update               # pull latest recipes + prebuilts

lx index list                 # show configured indexes
lx index add copr <url>       # register a custom index
lx index remove copr          # drop it
```

The registry lives in `~/.config/lx/indexes.yaml` and ships with the LX
community index, AUR, and the repology metadata source enabled by default.
The LX community index caches to `~/.cache/lx/index/` (a shallow git clone,
auto-refreshed; works offline on a stale cache with a warning). The repology
source caches to `~/.cache/lx/repology/` (JSON files refreshed via the
repology API on `lx index update`). `lx search` merges index results with the
`latest-debs` org and embedded templates; `--local` keeps it offline-only.

### Distro metadata (repology)

The repology source tracks what version of each project ships in 200+ distro
repositories. It is metadata-only — it cannot install anything, but it enriches
search results with cross-distro context:

```sh
lx index update               # refresh the repology cache from the API
lx search --distro eza        # show distro versions alongside org results
lx index status               # show host distro's repology identity
lx index outdated             # list packages where host distro lags upstream
lx index coverage             # find popular packages LX does not yet cover
```

With `--distro`, search results show the host distro's version, the newest
known version, and how many repos carry the package:

```
eza    A modern, maintained replacement for ls [latest-debs] — host: 0.18.0 (newest 0.20.0) [37 repos]
```

`lx index outdated` produces a gap list — packages where your distro ships an
older version than upstream — which is exactly the set the LX community recipe
index can fill.

`lx index coverage` compares repology's project set against the recipe index
and latest-debs org, then reports the gap — popular projects (by repo count)
that LX does not yet package. This validates `lx search` result quality: if a
project is everywhere in repology but missing from the index, `lx search`
won't find it, and that's a recipe worth contributing.

```
$ lx index coverage --min-repos 10
coverage: repology vs. LX (recipe index + latest-debs org)
  repology projects (≥10 repos): 34
  covered by LX:                 4 (11%)
  gap (not yet packaged):        30

top gap-fillers by repo count (showing 10 of 30):
  uv            408 repos
  python        380 repos
  node          350 repos
  ...
```

The LX index runs `lx build` + `--sbom` on every merged recipe in CI, so
community contributions ship prebuilt binaries without the contributor
running their own release infra.

## Development

```sh
cargo build
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all
```

Pre-commit hooks (`.pre-commit-config.yaml`) run fmt/clippy/shellcheck/
secret scan/license check/typos on every commit, and the fuller
`cargo test` + coverage gate + `cargo deny` + doc lint on push:

```sh
make dev-setup                              # install hook binaries (idempotent)
pre-commit install
pre-commit install --hook-type pre-push
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for the full setup guide, hook
reference, and project layout.

`docs/decisions/` records the non-obvious calls made while porting from the
bash action (library choices, config-format decisions, investigation
notes) — worth a look before changing behavior that mirrors upstream.
