# Commands

Part of the [lx docs](../README.md). The [convert](commands.md#lx-convert) and
[info](commands.md#lx-info) command
groups are covered below, plus the flags worth calling out.

| Command | Purpose |
|---|---|
| `lx build [config]` | Build packages from a `package.yaml`, zero-config from a GitHub URL, or from files you supply (`--from-dir`/`--from-file`). `--format` selects one packager (deb/rpm/arch/apk/ipk/msix/osxpkg), `--format all` builds every format, and `--format deb,rpm` builds each listed |
| `lx convert <pkg>` | Convert a built package from one format to another (deb↔rpm↔arch) — reads metadata + install tree from source, rebuilds natively in target format. `--to` defaults to the host's native format |
| `lx capture --name <n> --version <v> '<cmd>'` | Build a package from an install command's output (`checkinstall`-style): run the command with `$DESTDIR`, package the captured tree as-is (`--exclude <glob>` to drop paths); deps are auto-detected from the captured ELFs |
| `lx validate [config]` | Check a config resolves against a real release, without building |
| `lx deps scan [config]` | Report a release binary's shared-library dependencies, to verify/fill in `depends:` |
| `lx deps resolve <path>…` | Resolve ELF libraries to versioned `Depends` (`dpkg-shlibdeps` parity: reads the dpkg `symbols`/`shlibs` databases; fail-closed unless `--ignore-missing-info`) |
| `lx init` | Interactively generate a `package.yaml`; `--from <owner/repo>` scaffolds one non-interactively by auto-discovering release assets (`package_format` pre-filled from the host); `--from-aur <pkg>` imports an AUR PKGBUILD; `--from-nfpm <nfpm.yaml>` converts an nfpm config |
| `lx install <package>` | Install a native package (deb/rpm/arch, resolved from the host). The host's own repositories are checked first (`pacman -Si`/`apt-cache policy`/`dnf repoquery`): a package the distro carries is installed by the native manager (`pacman -S`/`apt-get install`/`dnf install`) and is **not** recorded in the lx manifest, since the host manager owns it. Otherwise (with a note that it is falling back) the enabled package indexes are tried next (prebuilt, then build-from-recipe), with the `latest-debs` GitHub org as the fallback; `--source <index>` forces one named index. An index/org install records an lx-managed generation. `--reinstall` re-installs (an lx-managed package's recorded version) |
| `lx update [package]` | Check installed packages against their latest release, no install; packages the org doesn't carry are checked through the enabled indexes |
| `lx upgrade [package]` | Upgrade installed packages to their latest release; packages the org doesn't carry upgrade through the enabled indexes. `--all` adds a system-wide freshness check (repology); `--auto-migrate` takes over distro packages flagged as outdated |
| `lx remove <package>` | Remove (or `--purge`) an installed package |
| `lx list` | List packages `lx` has installed (cross-checked against the host manager). `--catalog` lists packages available from the deb-get catalog instead — `--format table\|raw` (deb-get `list`), `--format pretty` (`prettylist`), `--format csv` (`csvlist`), with `--repo`, `--installed`/`--not-installed`, and `--include-unsupported`. `--verify [--prune]` audits the manifest against the host (deb-get `fix-installed`) |
| `lx show <package>` | Show everything known about one package (manifest + dpkg); falls back to the deb-get catalog definition when not installed |
| `lx info` | Auto-detect and report the host OS and package system (`--json` for machine-readable output) |
| `lx rollback <package>` | Reinstall a prior generation of an `lx`-managed package |
| `lx search [pattern]` | Full-text regex search like `apt search`: name + descriptions (including installed packages' dpkg long descriptions), installed/candidate versions, exact matches first; `--index`/`--index-only` merge the enabled package indexes; `--local` searches the offline starter-template index |
| `lx repo <dir>` | Turn a directory of built packages into a servable repository (apt `Packages`/`Release`/`InRelease` by default). `--multi-suite` produces a multi-suite layout (`dists/<suite>/` + top-level `Release`); `--format` writes the rpm (`repodata/`), pacman (`<repo>.db.tar.gz`), apk (`APKINDEX.tar.gz`), or opkg index instead, and defaults to the host's native format |
| `lx publish [config]` | Build every requested format and generate that format's repository index in one run (`--formats`, default `deb,rpm,arch`), each in its own `<output>/<format>/` subdirectory — the producer→distributor loop |
| `lx go-native` | Migrate snap/flatpak/nix/`curl \| sh` installs to native packages (applies by default; `--dry-run` prints a detailed benefit report; works on deb/rpm/arch hosts). `--all` attempts every finding, unmapped ones under their own name (best-effort) |
| `lx schema` | Generate the JSON schema for `package.yaml` (aliases: `json-schema`, `jsonschema`) |
| `lx index <cmd>` | Package-index management — AUR, LX community index, repology distro metadata, and custom indexes (search/info/update/coverage/outdated/status/list/add/remove). Installation goes through `lx install`, which records an lx-managed generation |

Moved names keep working as hidden aliases so existing scripts don't break:
`lx scan-deps` → `lx deps scan`, `lx shlibdeps` → `lx deps resolve`,
`lx reinstall` → `lx install --reinstall`, `lx discover` → `lx init --from`.

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

### `lx go-native`

Migrate snap, flatpak, nix, and `curl … | sh` installs to **native**
packages (`.deb` on dpkg hosts, `.rpm` on rpm hosts, `.pkg.tar.zst` on Arch
hosts). **Applying is the default**; `--dry-run` prints the plan — with a
detailed per-package benefit report — and changes nothing:

```sh
lx go-native --dry-run        # plan + benefits: detect + map, print only
lx go-native --dry-run --all  # plan for EVERY finding (unmapped → own name)
lx go-native                  # apply: install natives, remove sources
lx go-native firefox          # apply, filtered to matching ids
lx go-native --all            # attempt everything, best-effort for unmapped
lx go-native --keep-source    # install natives, keep both
lx go-native --from flatpak,nix --skip-sh   # managed sources only
lx go-native --cleanup-sh     # auto-delete curl|sh orphans after install
```

How it works: `snap list` / `flatpak list --app` / `nix profile list` are
parsed (snap runtimes like `core22` are excluded — they'll never have a
native equivalent), and `curl | sh` installs are found by scanning
`/usr/local/bin`, `~/.local/bin`, and `/opt` for binaries no native manager
claims (`dpkg -S` / `rpm -qf` / `pacman -Qo`), with installer URLs
attributed from shell history. Each finding is mapped through a built-in
table to an `lx` package name; anything unmapped lands in a
`missingnative` report instead of being silently dropped — unless `--all`,
which attempts it under the finding's own command name (the plan marks those
rows `[best-effort: no curated mapping]`).

`--dry-run` explains what each switch buys, from local state only (no
network):

```
  [curl|sh] /usr/local/bin/yq → yq
      state      native package already installed — only the redundant copy is removed
      reclaims   ~11M from the non-native source
      deps       5/5 already satisfied on this host (0/5 new)
      why        native arch package: host-managed, signed repo/pinned checksum, no duplicated runtime

reclaimed when applied: ~60.9 MB across 3 package(s)
```

`state` is whether the native package is already installed, `reclaims` is the
size of the redundant non-native copy, `deps` is how much of the native
package's dependency set the host already satisfies, and `keeps` appears when
the native package does **not** provide the command (so the non-native copy
would be kept).

Safety rules: always start with `--dry-run` — it mutates nothing. `curl | sh`
orphans are cleaned up with `--cleanup-sh` (recorded in
`~/.local/share/lx/sh_orphans.json`); without it the plan prints manual `rm`
commands for you to review — `--all` alone does **not** delete orphans.
Applying works on all hosts — deb via `lx install`, rpm via `rpm -Uvh`, arch
via `pacman -U`; without an explicit `--format`, each native install resolves
through `lx install` (host repos → enabled indexes → org).
`--remove-manager` offers to drop `snapd` itself once everything from it
migrated (always confirms).

### `lx convert`

Convert a built package from one format to another — not byte conversion
(which loses metadata), but a native rebuild: reads control fields from the
source, extracts the install tree, and rebuilds via the target format
plugin. Maintainer scripts (pre/post-install) are carried over, symlinks in
the payload are preserved, and system-library dependency names are
translated across distros (deb ↔ rpm ↔ arch) where the cross-distro table
knows the name.

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

Verify gate: `--lint` runs the target format's checker on the converted
artifact and fails on errors — `lintian` for deb, `rpm -K` for rpm,
`namcap` for arch. The checker binary must be on `PATH` (run the gate on a
host with the target format's tooling installed); pass
`--lint-fail-on-warnings` to also fail on warnings.

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
- **`--sandbox`** (`build`, source mode only): runs compile and install
  steps under `unshare -n` (a network namespace), so a build cannot reach
  the network. `lx` probes for a usable `unshare -n` once and warns and runs
  unsandboxed when it is unavailable. Binary repacks execute nothing
  regardless.
- **`--install-build-deps`** (`build` source mode, and `lx install` when
  building from a recipe): install the missing host build dependencies —
  `build_depends:` (host-distro names), an AUR package's `makedepends`, and
  the selected build system's toolchain (`cmake`, `ninja`, `cargo`, `go`,
  `meson`, … mapped per host) — with the host package manager
  (apt/dnf/zypper/pacman/apk/xbps) before compiling. Uses `sudo` unless
  already root; `--dry-run --install-build-deps` prints the install command
  instead of running it. Without the flag, `lx` only reports what's missing
  and never installs anything.
- **`--build`** (`install`): build from an index recipe, skipping the org and
  any prebuilt asset. It is index-only (always the host's native format), so
  it cannot be combined with `--format`/`--arch`/`--distribution`.
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

