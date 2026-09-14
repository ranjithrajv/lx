# `package.yaml` reference

Part of the [lx docs](../README.md). Every field the config accepts.

```yaml
package_name: eza          # required
github_repo: eza-community/eza   # required (--from-dir/--from-file omit this)

artifact_format: tar.gz     # tar.gz | tar.xz | tar.zst | tar | zip | raw (guessed if omitted)
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
suggests: ""                   # e.g. "eza-legacy"
conflicts: ""                  # e.g. "eza-legacy"
replaces: ""                   # e.g. "eza-legacy"
provides: ""                   # e.g. "eza-cli"
breaks: ""                     # e.g. "eza-legacy (<< 2.0)"
predepends: ""                 # e.g. "libc6" (must be fully installed first)
section: ""                    # deb Section (default: "utils")
priority: ""                   # deb Priority (default: "optional")
arch_variant: ""               # deb arch variant, e.g. "amd64v3"

version: ""                   # pin a specific upstream version (else: latest)
build_version: "1"            # Debian revision
epoch: ""                     # e.g. "1" -- for upstream version-numbering resets
version_schema: semver        # semver (default; strips v-prefix) | none
umask: ""                     # octal umask for files, e.g. "0o002" (default: inherit)
packager: ""                  # packager string; rpm: packager header tag, deb: Packager field

template_scripts: false       # enable ERB-like <%= key %> templating in maintainer scripts.
                              # Available: name, version, maintainer, description, homepage,
                              # license, arch, dist, iteration, epoch, vendor, packager, prefix

# Maintainer scripts. Paths are in the build environment. For deb they become
# DEBIAN/{preinst,postinst,prerm,postrm,preupgrade,postupgrade} (mode 0755);
# for rpm they map to %pre/%post/%preun/%postun/%pretrans/%posttrans/%verify;
# for arch preupgrade/postupgrade go in .INSTALL.
scripts:
  preinstall: ""              # DEBIAN/preinst | %pre | —
  postinstall: ""             # DEBIAN/postinst | %post | —
  preremove: ""               # DEBIAN/prerm | %preun | —
  postremove: ""              # DEBIAN/postrm | %postun | —
  pretrans: ""                # — | %pretrans | —
  posttrans: ""               # — | %posttrans | —
  verify: ""                  # — | %verify | —
  preupgrade_script: ""       # DEBIAN/preupgrade | %pretrans | pre_upgrade()
  postupgrade_script: ""      # DEBIAN/postupgrade | %posttrans | post_upgrade()

# Per-format dependency overrides keyed by package format ("deb"/"rpm"/"arch").
# An overridden field *replaces* the top-level value for that format.
overrides:
  deb:
    depends: ""               # override depends for deb only
  rpm:
    depends: ""               # override depends for rpm only

# Local-only packaging (lx build --local); skips the forge download.
# Existence is checked at build time. ${VAR} / ${VAR:-default} expand at parse.
local_payload: ""             # path to an archive or directory

signature:
  key_file: ""                # ASCII-armored secret key (env-expandable); apk: RSA key
  key_id: ""                  # optional gpg --local-user
  method: detach              # deb: detach (sibling .sig) | debsign (embedded _gpg{type})
  type: origin                # debsign role: origin | maint | archive
  # apk: key_file drives the in-process RSA/SHA-1 signer; method is ignored.

# Debian-specific: debconf, triggers, rules. Ignored by rpm/arch.
deb:
  rules: ""                   # path to debian/rules (→ DEBIAN/rules, mode 0755)
  templates: ""               # path to debconf templates (→ DEBIAN/templates, mode 0644)
  config: ""                  # path to debconf config script (→ DEBIAN/config, mode 0755)
  triggers_interest: []       # triggers this package registers interest in
  triggers_interest_await: [] # interest triggers that wait for the trigger
  triggers_interest_noawait: [] # interest triggers that don't wait
  triggers_activate: []       # triggers this package activates
  triggers_activate_await: [] # activate triggers that wait
  triggers_activate_noawait: [] # activate triggers that don't wait

# RPM-specific: triggers, compression, auto-deps. Ignored by deb/arch.
# Each trigger entry is "package: script_path" — the package is the trigger
# condition (fire when this package is installed/removed), the script runs
# when it fires.
rpm:
  trigger_pre_install: []     # %triggerprein (before another package installs)
  trigger_post_install: []    # %triggerin (after another package installs)
  trigger_pre_uninstall: []   # %triggerun (before another package removes)
  trigger_post_uninstall: []  # %triggerpostun (after another package removes)
  compression: ""             # gzip (default) | xz | lzma | zstd | none
  auto_provides: true         # provide shared-library sonames in the payload
  auto_requires: true         # require shared-library sonames in the payload
  defines: []                 # rpmbuild macros — accepted but not applied by
                              # the in-process builder (warned, never silent)

# Source builds (build_mode: source): fetch the upstream tag and compile on
# the host instead of repacking release assets. Requires an explicit
# architectures: list (all entries must equal the host arch — native only).
build_mode: source            # binary (default) | source
build_system: cmake           # cmake (default) | cargo | go | meson | autotools | make | custom (omit = auto-detect)
musl: false                    # musl-static binary: no glibc dep, runs on any Linux
upstream_url: ""              # tarball root; default: github <repo>/archive
upstream_ref: ""              # tag to fetch; default: resolved version
build_depends: []             # host packages the compile needs (host-distro names).
                              # Default: caller/CI preinstalls; `lx build --install-build-deps`
                              # installs the missing ones (plus the build system's toolchain)
                              # via the host package manager.
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
deps scan` first to see exactly which shared libraries the actual release
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

| `build_system` | Packager | Detects | Build |
|---|---|---|---|
| `cmake` | CMake + Ninja | `CMakeLists.txt` | cmake configure → build → DESTDIR install |
| `cargo` | Cargo (Rust) | `Cargo.toml` | `cargo install --path . --root <DESTDIR>` |
| `go` | Go | `go.mod` | `go build -trimpath` → `<DESTDIR>/bin/<name>` |
| `meson` | Meson + Ninja | `meson.build` | `meson setup` → build → DESTDIR install |
| `autotools` | GNU Autotools | `configure`/`configure.ac` | `./configure --prefix=/usr` → make → DESTDIR install |
| `make` | GNU Make | `Makefile`/`makefile` | `make PREFIX=/usr` → DESTDIR make install |
| `custom` | User commands | (explicit-only) | `build_commands` / `install_commands` with `$DESTDIR` |

If `build_system:` is omitted, `lx` auto-detects from the source tree
(`CMakeLists.txt` → cmake, `Cargo.toml` → cargo, `go.mod` → go,
`meson.build` → meson, `configure`/`configure.ac` → autotools, `Makefile`
→ make). Set it explicitly to override or to use `custom`. Per-build-system
internals are in the [plugin catalog](../architecture/plugin-catalog.md).

**`registry_source:`** — fetch from a language package manager instead of a
forge release. When set, the package to fetch is taken from `github_repo`
(or `package_name`), and the version from `version`. Each registry source
is a plugin implementing the `RegistrySource` trait — adding a new ecosystem
is implementing the trait and registering it in `lib/plugins/registry/`.

| `registry_source` | Language | Fetches via | Required tools |
|---|---|---|---|
| `npm` | JavaScript/Node | `npm pack` + extract | `npm` |
| `python` | Python | `pip download --no-binary :all:` + extract | `pip`, `python3` |
| `gem` | Ruby | `gem fetch` + extract | `gem` |
| `cargo` | Rust | `cargo install --root <dir>` | `cargo` |
| `nuget` | .NET/C# | `nuget install` + extract | `nuget` |
| `maven` | Java/Kotlin/Scala | `mvn dependency:copy-dependencies` | `mvn` |
| `composer` | PHP | `composer install` | `composer` |
| `cpan` | Perl | `cpanm` + build install tree | `cpanm`, `perl` |
| `go` | Go | `go get` + `go build` static binary | `go` |
| `hex` | Elixir/Erlang | `mix deps.get` + stage | `mix`, `elixir` |
| `dart` | Dart/Flutter | `dart pub get` + stage | `dart` |

Per-plugin fetch details are in the
[plugin catalog](../architecture/plugin-catalog.md).

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
fallback for musl binaries. The consumer client (`lx get install`) falls
back to a `+musl_{arch}.deb` asset when no distro-specific build exists.

**`--cross-target <ARCH>`** — cross-compile for a different architecture
(e.g., `--cross-target arm64` on an amd64 host). Automatically enables
musl-static linking for reproducible multi-arch builds.

**`--bindep`** — detect binary dependencies via ELF analysis (default: on).
Scans built binaries for shared-library links and maps them to system
packages. Catches ACTUAL dependencies rather than guessing.

**`--cosign`** — sign built packages with cosign (Sigstore keyless signing).
Requires `cosign` on PATH and an OIDC token (e.g., in GitHub Actions).
Produces `.sig` signature files alongside each artifact.

**Checksum sidecars** — every build automatically generates `.sha256` and
`.sha512` checksum files alongside each package artifact for integrity
verification.

**Shell installer** — every build generates a `<package>-install.sh` script
that detects the user's OS/distro/arch, downloads the matching package,
verifies its checksum, and installs it via the native package manager
(`dpkg`/`rpm`/`pacman`). Usage: `curl -sSL <url>/install.sh | sh`.

**`architecture:`** — override the package architecture field. `"auto"`
(default) uses the target architecture. `"all"` forces `Architecture: all`
for pure-code packages (Python/Ruby/Perl libraries). `"any"` forces
`Architecture: any` for compiled tools. Auto-detected from registry source
when not set.

**Package naming conventions** — when `package_name` is not set, lx derives
a convention-compliant name from the registry name:

- Debian: `libjson-perl`, `ruby-rake`, `node-underscore`, `python3-requests`
- RPM: `perl-JSON-XS`, `ruby-rake`, `python3-requests`
- Arch: `perl-json-xs`, `ruby-rake`, `python-requests`

**Relocatable binaries** — ELF binaries are automatically patched with
`patchelf` to use `$ORIGIN/../lib` RPATH, so they work from any install path.

**Dependency resolution** — lx reads dependency files from registry packages
(package.json, requirements.txt, Cargo.toml, etc.), resolves version
constraints, and maps them to system packages across 9 ecosystems with 60+
known mappings and three-way Debian/RPM/Arch name conversion, falling back
to Repology for unknown packages.

**`lx init --from-aur <pkg>`** — convert an AUR PKGBUILD into a starter
`package.yaml` (makedeb-orphan migration path). Guesses are commented for
review: the `github_repo` guess (loud `FIXME` when the AUR URL isn't
GitHub), Arch dependency names kept verbatim for Debian mapping, AUR
`makedepends` emitted as `build_depends:` (so `--install-build-deps` can
install them), and `build()` presence mapped to `build_mode: source` hints.
Recipes are never executed — PKGBUILD shell becomes comments, not code.

**`lx repo <dir>`** — turn a directory of built packages into a repository
the host package manager can consume. Defaults to apt (`Packages` +
`Packages.gz`, control fields read natively, `Release` with
MD5/SHA1/SHA256, and clearsigned `InRelease` with `--sign-key`);
`--format` writes the rpm (`repodata/`), pacman (`<repo>.db.tar.gz`), apk
(`APKINDEX.tar.gz`), or opkg index instead, each signable. Serve `<dir>`
over HTTP and point `sources.list` (or `dnf`/`pacman`) at it — the
producer→distributor loop, with `lx install`/`lx get` as the client (or
`lx publish` to build and index every format in one command).

Man pages (`*.1`–`*.9`, gzipped) and license files (`LICENSE`/`COPYING`/
`NOTICE`, any casing) sitting alongside the binary in a flat-mode release
are auto-installed to `/usr/share/man/man<N>/` and `/usr/share/doc/<pkg>/`
— no config needed. (Bundle-mode installs already preserve everything in
the upstream tree.)

