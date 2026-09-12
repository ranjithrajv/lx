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
