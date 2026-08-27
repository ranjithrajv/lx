# lpt — Latest Package Tool

`lpt` repackages GitHub release binaries into `.deb` packages across Debian
suites and architectures, and installs/upgrades/removes the packages it
builds — an apt-like front end for software that only ships GitHub releases.
It's a Rust rewrite of the
[debian-multiarch-builder](https://github.com/ranjithrajv/debian-multiarch-builder)
GitHub Action, usable both as a local CLI and as a GitHub Action
(`action.yml`, see below).

Every package it produces is a real Debian package, not a thin ZIP-in-an-ar
wrapper: full dependency relations (`Depends`/`Recommends`/`Conflicts`/
`Replaces`/`Provides`/`Breaks`), epoch-aware versioning, man pages and
license files placed at their conventional FHS paths, and genuine `xz`
source-package compression — all output that real `dpkg-deb`/`dpkg-source`/
`lintian` accept without complaint, built entirely without Docker or a
Debian host.

It's also trustworthy by default about what it downloads and repackages:
a build refuses to proceed on an unverified asset unless you explicitly
say otherwise, every download's verification method and checksum are
recorded for later audit, and pinning a prerelease or draft tag gets you
a warning instead of a silent surprise.

## Requirements

- Rust (to build `lpt` itself; see [Development](#development)). That's
  it — **`lpt` never uses Docker, for anything:**
  - `lpt build` builds `.deb`s natively (`lib/debarchive.rs`); no
    `dpkg-deb`, no Debian toolchain, and cross-architecture builds never
    need QEMU (packaging only copies/`chmod`s the target binary, it's
    never executed).
  - `--source` builds source packages (`.dsc`/`.orig.tar.xz`/
    `.debian.tar.xz`) natively too — no `dpkg-source`.
  - `--lintian` requires a `lintian` binary on your `PATH` (e.g.
    `apt-get install lintian` on Debian/Ubuntu). lintian itself has no
    Rust equivalent to reach for, so this one flag needs it installed —
    but there's no Docker fallback to reach it through either way.

  See
  [`docs/decisions/2026-08-20-docker-free-deb-build.md`](docs/decisions/2026-08-20-docker-free-deb-build.md)
  and
  [`docs/decisions/2026-08-20-docker-free-lintian-source.md`](docs/decisions/2026-08-20-docker-free-lintian-source.md)
  for how, and the trade-offs involved.

## Quick start

Build every architecture a GitHub release publishes, no config file needed:

```sh
lpt build https://github.com/eza-community/eza
```

Build for just this machine's own architecture (skips QEMU entirely):

```sh
lpt build package.yaml --host
```

Generate a `package.yaml` interactively, with auto-discovered release
patterns:

```sh
lpt init
```

Or start from one of the bundled debian-multiarch-builder templates:

```sh
lpt init --template rust/eza    # or go/hugo, c/neovim, python/generic, …
```

(An unknown name lists all of them; see [`templates/README.md`](templates/README.md).)

## Commands

| Command | Purpose |
|---|---|
| `lpt build [config]` | Build `.deb`s (and optionally source packages) from a `package.yaml`, or zero-config from a GitHub URL |
| `lpt validate [config]` | Check a config resolves against a real release, without building |
| `lpt discover <owner/repo> [version]` | Auto-discover release-asset patterns and print a starter config |
| `lpt scan-deps [config]` | Report a release binary's shared-library dependencies, to verify/fill in `depends:` |
| `lpt init` | Interactively generate a `package.yaml`, with optional auto-discovery |
| `lpt install <package>` | Fetch and install a pre-built `.deb` from the `latest-debs` GitHub org |
| `lpt update [package]` | Check installed packages against their latest release, no install |
| `lpt upgrade [package]` | Upgrade installed packages to their latest release |
| `lpt remove <package>` | Remove (or `--purge`) an installed package |
| `lpt list` | List packages `lpt` has installed |

Run `lpt <command> --help` for the full flag reference. A few worth calling
out:

- **`--host`** (`build`): auto-detects this machine's architecture (`uname
  -m`) and builds only that one, skipping QEMU. Conflicts with
  `--architectures`.
- **`--local`** (`build`): package a local archive or directory from
  `local_payload:` in package.yaml, skipping the upstream download.
  Requires `version:` (or `--version`).
- **`--sign-key` / `--sign-method`** (`build`): sign built packages.
  Method `detach` (default) writes a sibling `.sig`; `debsign` embeds
  `_gpgorigin` inside the `.deb` (debsigs / nfpm-compatible). RPM embeds
  natively either way. Passphrase via `$LPT_SIGN_PASSPHRASE` or
  `$NFPM_PASSPHRASE`.
- **A GitHub URL in place of a config file** (`build`): `lpt build
  https://github.com/<owner>/<repo>` needs no `package.yaml` at all — builds
  every architecture the release publishes, with source packages included.
- **`--pinned-metadata`** (`build`): verify downloaded assets against a
  `release-metadata.json` provenance pin captured at vet time, instead of
  (or in addition to) the release's own live checksum sidecar.
- **`--source`** (`build`): also generate a Debian source package (`.dsc` +
  `.debian.tar.xz` + a shared `.orig.tar.xz`) per distribution.
- Version/architecture/distribution resolution, checksum verification, and
  reproducible-build hygiene (below) are all shared machinery — see
  `--dry-run` to preview a build matrix without downloading or building
  anything.

**Checksum verification is required by default.** When neither
`--pinned-metadata` nor a live `.sha256`/`.sha256sum` sidecar is available
for a downloaded asset (most GitHub releases don't publish one), `lpt
build` fails rather than silently continuing — pass `--allow-unverified` to
build anyway. `--no-verify` skips verification entirely, sidecar or not.
`lpt build`/`lpt validate` also flag prerelease and draft releases (e.g.
pinning `version: nightly` on a repo whose "latest" tag is an RC) so you
don't package pre-stable software without noticing.

With `--summary`, `build-summary.json` gets a `provenance` array — one
entry per unique asset downloaded, with the verification method used
(`pinned`/`sidecar`/`unverified (--allow-unverified)`/`skipped
(--no-verify)`), its resolved SHA-256, and the source URL — plus an
`unverified_assets` count, so a build can be audited after the fact instead
of just trusting a console log that's already scrolled away.

`lpt install`/`lpt upgrade` apply the same fail-closed default against the
release's own sidecar (there's no pin file in that flow): no sidecar means
no install unless you pass `--allow-unverified` there too.

`--summary` also prints a markdown badge block for your packaging repo's
own README: a "Built with lpt" badge, a suites/architectures-coverage
badge pair, and — only when running as the GitHub Action, where
`GITHUB_REPOSITORY` identifies the repo publishing the release —
latest-release and downloads badges too. Never guessed on a local run.

`install`/`update`/`upgrade`/`remove`/`list` track what they manage in a
local manifest (`installed.json` under your XDG data dir), cross-checked
against `dpkg`'s own record of what's actually installed — `lpt` is never
the sole source of truth for what's on your system.

## `package.yaml` reference

```yaml
package_name: eza          # required
github_repo: eza-community/eza   # required, "owner/repo"

artifact_format: tar.gz     # tar.gz | tgz | zip | raw (guessed if omitted)
description: "A modern replacement for ls"
maintainer: "Jane Doe <jane@example.com>"
license_spdx: MIT           # falls back to the upstream repo's detected license

debian_distributions: [bookworm, trixie, forky, sid]   # default: all five suites

binary_path: ""              # path to the binary within the extracted archive
binary_rename: ""            # rename the single installed binary to this name
bundle: false                 # see below

depends: ""                   # e.g. "libatomic1, libgtk-3-0"
recommends: ""                 # e.g. "bash-completion"
conflicts: ""                  # e.g. "eza-legacy"
replaces: ""                   # e.g. "eza-legacy"
provides: ""                   # e.g. "eza-cli"
breaks: ""                     # e.g. "eza-legacy (<< 2.0)"

version: ""                   # pin a specific upstream version (else: latest)
build_version: "1"            # Debian revision
epoch: ""                     # e.g. "1" -- for upstream version-numbering resets

# Local-only packaging (lpt build --local); skips the forge download.
# Existence is checked at build time. ${VAR} / ${VAR:-default} expand at parse.
local_payload: ""             # path to an archive or directory

signature:
  key_file: ""                # ASCII-armored secret key (env-expandable)
  key_id: ""                  # optional gpg --local-user
  method: detach              # detach (sibling .sig) | debsign (embedded _gpgorigin)
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
needs `libatomic1`). Emits a `Depends:` control-file line. Run `lpt
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
embeds an armored detach-signature as the `_gpgorigin` ar member (debsigs /
nfpm). For `.rpm`, the signature is always embedded in the header when
`key_file` is set. `${VAR}` / `${VAR:-default}` expand in the YAML at parse
time (e.g. `key_file: ${SIGNING_KEY_FILE}`).

Man pages (`*.1`–`*.9`, gzipped) and license files (`LICENSE`/`COPYING`/
`NOTICE`, any casing) sitting alongside the binary in a flat-mode release
are auto-installed to `/usr/share/man/man<N>/` and `/usr/share/doc/<pkg>/`
— no config needed. (Bundle-mode installs already preserve everything in
the upstream tree.)

## GitHub Action

`action.yml` wraps `lpt build` as a composite action — a drop-in
replacement for `debian-multiarch-builder`'s action.yml (same input/output
names):

```yaml
- uses: ranjithrajv/lpt@v1
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
repo has no published binary releases yet, the action builds `lpt` from
source (cached via `Swatinem/rust-cache`).

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

## Development

```sh
cargo build
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all
```

Pre-commit hooks (`.pre-commit-config.yaml`) run fmt/clippy/shellcheck/a
secret scan on every commit, and the fuller `cargo test` + coverage gate +
`cargo audit` on push:

```sh
pre-commit install
pre-commit install --hook-type pre-push
```

`docs/decisions/` records the non-obvious calls made while porting from the
bash action (library choices, config-format decisions, investigation
notes) — worth a look before changing behavior that mirrors upstream.
