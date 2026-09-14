# Commands

Part of the [lx docs](../README.md). The [migrate](commands.md#lx-migrate-native),
[convert](commands.md#lx-convert), and [info](commands.md#lx-info) command
groups are covered below, plus the flags worth calling out.

| Command | Purpose |
|---|---|
| `lx build [config]` | Build packages from a `package.yaml`, zero-config from a GitHub URL, or from files you supply (`--from-dir`/`--from-file`). `--format` selects one packager (deb/rpm/arch/apk/ipk), `--format all` builds every format, and `--format deb,rpm` builds each listed |
| `lx convert <pkg>` | Convert a built package from one format to another (deb↔rpm↔arch) — reads metadata + install tree from source, rebuilds natively in target format. `--to` defaults to the host's native format |
| `lx validate [config]` | Check a config resolves against a real release, without building |
| `lx deps scan [config]` | Report a release binary's shared-library dependencies, to verify/fill in `depends:` |
| `lx deps resolve <path>…` | Resolve ELF libraries to versioned `Depends` (`dpkg-shlibdeps` parity: reads the dpkg `symbols`/`shlibs` databases; fail-closed unless `--ignore-missing-info`) |
| `lx init` | Interactively generate a `package.yaml`; `--from <owner/repo>` scaffolds one non-interactively by auto-discovering release assets (`package_format` pre-filled from the host) |
| `lx install <package>` | Fetch and install a pre-built native package (deb/rpm/arch, resolved from the host) from the `latest-debs` GitHub org. `--reinstall` re-installs (an lx-managed package's recorded version) |
| `lx update [package]` | Check installed packages against their latest release, no install |
| `lx upgrade [package]` | Upgrade installed packages to their latest release. `--all` adds a system-wide freshness check (repology); `--auto-migrate` takes over distro packages flagged as outdated |
| `lx remove <package>` | Remove (or `--purge`) an installed package |
| `lx list` | List packages `lx` has installed |
| `lx show <package>` | Show everything known about one package (manifest + dpkg) |
| `lx info` | Auto-detect and report the host OS and package system (`--json` for machine-readable output) |
| `lx rollback <package>` | Reinstall a prior generation of an `lx`-managed package |
| `lx search [pattern]` | Full-text regex search like `apt search`: name + descriptions (including installed packages' dpkg long descriptions), installed/candidate versions, exact matches first; `--local` searches the offline starter-template index |
| `lx repo <dir>` | Turn a directory of built packages into a servable repository (apt `Packages`/`Release`/`InRelease` by default). `--multi-suite` produces a multi-suite layout (`dists/<suite>/` + top-level `Release`); `--format` writes the rpm (`repodata/`), pacman (`<repo>.db.tar.gz`), apk (`APKINDEX.tar.gz`), or opkg index instead, and defaults to the host's native format |
| `lx publish [config]` | Build every requested format and generate that format's repository index in one run (`--formats`, default `deb,rpm,arch`), each in its own `<output>/<format>/` subdirectory — the producer→distributor loop |
| `lx migrate lpt [--repo DIR]` | Carry legacy `lpt` state (manifest, caches) and workflows to `lx`. Bare `lx migrate` is equivalent |
| `lx migrate native` | Migrate snap/flatpak/nix/`curl \| sh` installs to native packages (plan by default, `--yes` to apply; works on deb/rpm/arch hosts) |
| `lx schema` | Generate the JSON schema for `package.yaml` (aliases: `json-schema`, `jsonschema`) |
| `lx index <cmd>` | Unified package-index manager — AUR, LX community index, repology distro metadata, and custom indexes (search/install/info/update/coverage/outdated/status) |

Moved names keep working as hidden aliases so existing scripts don't break:
`lx scan-deps` → `lx deps scan`, `lx shlibdeps` → `lx deps resolve`,
`lx go-native` → `lx migrate native`, `lx reinstall` →
`lx install --reinstall`, `lx discover` → `lx init --from`.

`lx get` is the consumer subcommand group —
`install`/`upgrade`/`update`/`remove`/`show`/`reinstall`/`rollback`/`list`/`search`,
no build machinery. Same manifest, same org:

```sh
lx get install eza
lx get upgrade --owned-only   # skip entries removed outside lx
```

On an rpm or pacman host the same commands operate on that host's native
format: `install`/`upgrade` fetch the `.rpm` or `.pkg.tar.zst` asset,
`remove` uses `rpm -e`/`pacman -R`, and versions are compared with that
format's own ordering. `--format` overrides host detection, and
`LX_INDEX_ORG` points the client at a different GitHub org (default
`latest-debs`). Each manifest entry records the format it was installed as.

## Command details

### `lx migrate native`

Migrate snap, flatpak, nix, and `curl … | sh` installs to **native**
packages (`.deb` on dpkg hosts, `.rpm` on rpm hosts, `.pkg.tar.zst` on Arch
hosts). Plan by default — nothing is installed or removed until you pass
`--yes`:

```sh
lx migrate native                  # plan: detect + map, print only
lx migrate native firefox          # plan, filtered to matching ids
lx migrate native --yes            # apply: install natives, remove sources
lx migrate native --yes --keep-source      # install natives, keep both
lx migrate native --from flatpak,nix --skip-sh   # managed sources only
lx migrate native --yes --cleanup-sh   # auto-delete curl|sh orphans after install
```

How it works: `snap list` / `flatpak list --app` / `nix profile list` are
parsed (snap runtimes like `core22` are excluded — they'll never have a
native equivalent), and `curl | sh` installs are found by scanning
`/usr/local/bin`, `~/.local/bin`, and `/opt` for binaries no native manager
claims (`dpkg -S` / `rpm -qf` / `pacman -Qo`), with installer URLs
attributed from shell history. Each finding is mapped through a built-in
table to an `lx` package name; anything unmapped lands in a
`missingnative` report instead of being silently dropped.

Safety rules: `curl | sh` orphans are cleaned up with `--cleanup-sh`
(recorded in `~/.local/share/lx/sh_orphans.json`); without it the plan
prints manual `rm` commands for you to review. Applying works on all
hosts — deb via `lx install`, rpm via `rpm -Uvh`, arch via `pacman -U`.
`--remove-manager` offers to drop `snapd` itself once everything from it
migrated (always confirms).

### `lx convert`

Convert a built package from one format to another — not byte conversion
(which loses metadata), but a native rebuild: reads control fields from the
source, extracts the install tree, and rebuilds via the target format
plugin. Maintainer scripts (pre/post-install) are carried over.

```sh
lx convert foo_1.0_amd64.deb --to rpm        # deb → rpm
lx convert foo-1.0.x86_64.rpm --to deb        # rpm → deb
lx convert foo-1.0-arch-x86_64.pkg.tar.zst --to deb  # arch → deb

lx convert foo-1.0.x86_64.rpm                 # --to omitted: targets the host's
                                              # native format (here, deb on Debian)
```

Overrides: `--package-name`, `--version`, `--arch`, `--distribution`,
`--build-version`. Use `--dry-run` to preview metadata without building.

`--to` is optional: when omitted, `lx` targets the host's native format
(auto-detected, see `lx info`). So `lx convert foo.rpm` on a Debian box
produces a `.deb`, and `lx convert foo.deb` on an Arch box produces a
`.pkg.tar.zst`.

### `lx info`

Report what `lx` detects about this host — read-only and offline, nothing is
downloaded or installed. It parses `/etc/os-release`, runs `uname`, and probes
`PATH` for the host package manager, then prints the OS, kernel, architecture,
package manager, native package format, and the distro token `lx install`/
`lx upgrade` match prebuilt release assets against (see [Host detection &
smart defaults](host-detection.md) for the sample output):

```sh
lx info            # human report
lx info --json     # machine-readable, for scripts/CI
```

Useful when a package is missing from a release (the `Asset dist` token is
what the consumer looks for) or to confirm which format `lx get` will pick on
an rpm, pacman, or Alpine host. The same detection powers the smart defaults
of `lx convert --to`, `lx repo --format`, and `lx init`'s `package_format`
prompt.

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
  the PGP signature natively, and apk embeds an in-process RSA/SHA-1
  signature (`.SIGN.RSA.<keyname>`). Passphrase via `$LX_SIGN_PASSPHRASE`
  or `$NFPM_PASSPHRASE`.
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
- **`--sandbox`** (`build`, source mode only): intended to run compile
  steps under `unshare -n` (no network, private mounts). **Currently
  declared but not yet wired** — the flag is accepted but no `unshare`
  invocation happens (see `docs/tooling.md` §1.8). Binary repacks execute
  nothing regardless.
- **`--install-build-deps`** (`build` source mode, and `index install` when
  building from a recipe): install the missing host build dependencies —
  `build_depends:` (host-distro names), an AUR package's `makedepends`, and
  the selected build system's toolchain (`cmake`, `ninja`, `cargo`, `go`,
  `meson`, … mapped per host) — with the host package manager
  (apt/dnf/zypper/pacman/apk/xbps) before compiling. Uses `sudo` unless
  already root; `--dry-run --install-build-deps` prints the install command
  instead of running it. Without the flag, `lx` only reports what's missing
  and never installs anything.
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

