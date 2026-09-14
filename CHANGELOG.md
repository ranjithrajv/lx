# Changelog

## Unreleased

### fpm feature parity: pre/post-upgrade, RPM triggers, script templating

- **Pre-/post-upgrade scripts**: `scripts.preupgrade_script` and
  `scripts.postupgrade_script` map to `DEBIAN/preupgrade` +
  `DEBIAN/postupgrade` (deb), `%pretrans`/`%posttrans` (rpm), and
  `pre_upgrade()`/`post_upgrade()` (arch). Mirrors fpm's
  `--before-upgrade`/`--after-upgrade`.
- **RPM triggers**: new `rpm:` config block with `trigger_pre_install`,
  `trigger_post_install`, `trigger_pre_uninstall`,
  `trigger_post_uninstall`. Each entry is `"package: script_path"`.
  Trigger dependencies are emitted with the correct RPM trigger flags
  (`TRIGGERPREIN`, `TRIGGERIN`, `TRIGGERUN`, `TRIGGERPOSTUN`). Scripts
  are best-effort (full `%triggerin`/`%triggerun` scriptlets need
  rpmbuild or a future rpm-crate version). Mirrors fpm's 4
  `--rpm-trigger-*` flags.
- **Script templating**: `template_scripts: true` enables ERB-like
  `<%= key %>` substitution in all maintainer scripts. 13 variables are
  auto-populated: `name`, `version`, `maintainer`, `description`,
  `homepage`, `license`, `arch`, `dist`, `iteration`, `epoch`, `vendor`,
  `packager`, `prefix`. Mirrors fpm's `--template-scripts`.
- **Glob patterns in `contents:`**: `src` patterns with `*`, `?`, `[` are
  expanded via the `glob` crate. `disable_globbing: true` opts out.
  Mirrors fpm's glob support in `contents:`.
- **RPM relation fields**: `depends`, `provides`, `conflicts`, `replaces`,
  `recommends`, `suggests`, `breaks` now map to rpm-crate dependency
  tags (`requires`, `provides`, `conflicts`, `obsoletes`, `recommends`,
  `suggests`). RPM packages now carry proper dependency metadata.
- **Packager field**: new `packager:` config field maps to the RPM
  `packager` header tag (falls back to maintainer). Mirrors fpm's
  `--rpm-packager` and nfpm's `rpm.packager`.
- **Arch variant**: new `arch_variant:` field (e.g. `amd64v3`) appends
  to the `Architecture` control field. Mirrors nfpm's
  `deb.arch_variant`.
- **Version schema**: new `version_schema: ` field (`semver` default or
  `none`). Semver mode strips `v` prefix and normalizes. Mirrors nfpm's
  `version_schema`.
- **Umask control**: new `umask:` field (octal, e.g. `0o002`) masks file
  permissions for all staged files. Mirrors nfpm's `umask`.
- **Per-format relation overrides**: `overrides: {deb: {depends: ...}}`
  lets the same field have different values per format. Mirrors nfpm's
  `overrides`.
- **Suggests + Pre-Depends**: `suggests:` and `predepends:` fields now
  render `Suggests:` and `Pre-Depends:` control lines. Mirrors nfpm.

### Registry source plugins (language package managers)

- New `registry_source:` field in package.yaml selects a language package
  manager plugin. Initial plugins: `npm`, `python`, `gem`, `cargo`, `go`,
  `hex`, `dart`, `nuget`, `maven`, `composer`, `cpan`. These fetch from
  language registries instead of forge releases, producing a local payload
  directory that flows through the normal packaging pipeline. The `go` plugin
  builds a static binary (CGO_ENABLED=0). The `hex` plugin stages Elixir
  packages via `mix deps.get`. The `dart` plugin stages Dart/Flutter packages
  via `dart pub get`. Each registry source is a plugin implementing the
  `RegistrySource` trait in `lib/plugins/registry/` — adding a new ecosystem
  is implementing the trait and registering it.

### Relocatable binaries, full dependency resolution, auto-dep-mapping

- **Relocatable binaries:** ELF binaries get RPATH set to `$ORIGIN/../lib`
  via patchelf, enabling installation to any path (cargo-dist parity).
- **Full dependency resolution:** Reads dependency files from all registry
  ecosystems (package.json, requirements.txt, Cargo.toml, Gemfile,
  composer.json, mix.exs, pubspec.yaml, go.mod, pom.xml, Makefile.PL),
  parses version constraints, resolves them to system packages.
- **Auto-dep-mapping:** 60+ known dependency mappings across 9 ecosystems
  (npm, python, gem, cargo, cpan, composer, hex, dart, maven). Three-way
  conversion: Debian ↔ RPM ↔ Arch package names.

### Package naming conventions and architecture auto-detection

- **Package naming:** When `package_name` is not set, lx derives a
  convention-compliant name per ecosystem and format (Debian: `libfoo-bar-perl`,
  `ruby-foo`; RPM: `perl-Foo-Bar`; Arch: `perl-foo-bar`).
- **Architecture auto-detection:** Pure-code packages (Python, Ruby, Perl
  libraries) get `Architecture: all`; compiled tools get `Architecture: any`.
  Manual override via `architecture:` field (auto/all/any).

### Five new features for goreleaser/cargo-dist/*2deb parity

1. **Checksum sidecars** — `.sha256` and `.sha512` files generated alongside
   each package artifact for integrity verification.
2. **Shell installer** — `<package>-install.sh` script auto-generated per
   build. Detects OS/distro/arch, downloads matching package, verifies
   checksum, installs via dpkg/rpm/pacman. `curl | sh` ready.
3. **Cross-compilation** — `--cross-target <ARCH>` flag builds for a
   different architecture than the host. Auto-enables musl-static linking.
4. **Cosign signing** — `--cosign` flag signs packages with Sigstore
   keyless signing (requires cosign on PATH + OIDC token).
5. **Dependency mapping** — auto-infers `depends:` from registry package
   dependency files (package.json, requirements.txt, Cargo.toml, etc.)
   using per-ecosystem mapping tables. Covers npm, python, gem, cargo,
   composer, cpan, hex, and more.

### Binary dependency detection and auto-dep-mapping

- **Binary dependency detection** (`--bindep`): Scans ELF binaries for
  DT_NEEDED shared-library entries, maps sonames to system packages via
  dpkg/rpm/pacman. Catches ACTUAL dependencies (what the binary links
  against) rather than guessing. 53 tests.
- **Full dependency resolution**: Reads dependency files from all 9
  registry ecosystems, parses version constraints (~, ^, >=), resolves
  to system packages with three-way Debian/RPM/Arch conversion.
- **Repology fallback**: When hardcoded mappings don't cover a dependency,
  queries Repology's API for 200+ distributions.
- **Package naming conventions**: Auto-derives convention-compliant names
  (libjson-perl, ruby-rake, python3-requests) per ecosystem and format.
- **Architecture auto-detection**: Pure-code packages get `Architecture: all`,
  compiled tools get `Architecture: any`. Manual override via `architecture:`.

### Meson build-system plugin

- New `build_system: meson` plugin for projects using the Meson build system
  (GNOME, systemd-adjacent, many C/C++ projects). Recognizes `meson.build`,
  invokes `meson setup` / `meson compile` / `meson install --destdir`. Supports
  musl-static builds via `--cross-file=musl`. BuildSystem count: 4 → 5.

### Musl-static builds for old-distro portability

- `musl: true` in package.yaml produces a musl-static binary with no
  glibc dependency — runs on any Linux regardless of distro age. For source
  builds, each build system plugin adjusts its compile flags (`cargo`:
  `--target x86_64-unknown-linux-musl` auto-installed via rustup; `go`:
  `CGO_ENABLED=0`; `cmake`: `musl-gcc` + `-static`). For binary repacks,
  musl-named release assets (e.g. `*-linux-musl.tar.gz`) are preferred
  during auto-discovery. `compute_depends()` omits the `libc6` fallback
  for musl binaries.
- `lx scan-deps --prefer-musl` prefers musl release assets when scanning
  dependencies.
- `lx get install` falls back to a `+musl_{arch}.deb` asset when no
  distro-specific build exists for the host, so users on old distros can
  still install packages.

### Build-system plugins for source builds

- `build_mode: source` now dispatches to pluggable build-system plugins
  instead of hardcoding cmake vs custom. Four plugins ship: `cmake`
  (default, auto-detects `CMakeLists.txt`), **`cargo`** (auto-detects
  `Cargo.toml`, `cargo install --path . --root <DESTDIR>`), **`go`**
  (auto-detects `go.mod`, `go build -trimpath`), and `custom` (explicit
  `build_commands`/`install_commands`). Adding a new ecosystem is
  implementing the `BuildSystem` trait and registering it — no edits to
  the source-build pipeline. `build_system:` in package.yaml is now
  optional: omitted means auto-detect from the source tree.

### "You supply files" mode (`--from-dir`/`--from-file`)

- `lx build` can now package files you supply directly, with no forge
  release fetch — fpm-style "you supply files" mode. `lx build --from-dir
  ./dist/ --package-name myapp --version 1.0.0` builds a `.deb` from a
  directory; `--from-file` does the same for a single file. A base
  `package.yaml` is optional and provides extra metadata (dependencies,
  contents, signing); `github_repo` is not required. `--prefix` sets a
  custom install path inside the package (e.g. `/usr/local/bin`).

### Rename: `lpt` → `lx`

- The CLI is now `lx` (`lx-lib` crate, `lx get` consumer subcommand). Env vars are
  `LX_*` (`LX_MAINTAINER`, `LX_SIGN_PASSPHRASE`), caches live under
  `~/.cache/lx`, the manifest under `<data>/lx/installed.json`, and the
  GitHub repo moved to `ranjithrajv/lx` (the old URL redirects).
- **Migration is one command:** `lx migrate` moves the manifest and caches
  (never clobbers existing `lx` state; idempotent), and
  `lx migrate --repo DIR` rewrites a packaging repo's workflows
  (`ranjithrajv/lpt@` → `ranjithrajv/lx@`, `lpt build` → `lx build`,
  cache keys/paths). Review the diff, then commit.
- Action consumers: change `uses: ranjithrajv/lpt@…` to
  `uses: ranjithrajv/lx@…`. Inputs/outputs are unchanged.
