# Changelog

## Unreleased

### `lx convert` metadata interoperability

- RPM metadata, scriptlets, and payload are read in-process via the `rpm`
  crate — no `rpm`, `rpm2cpio`, or `cpio` on `PATH`.
- Relation fields (`Provides`/`Recommends`/`Suggests`/`Conflicts`/
  `Replaces`/`Breaks`/`Pre-Depends`), `Section`/`Priority`, and the epoch
  are now carried into the target, with dependency syntax and names
  rewritten across formats like `Depends` already was.
- Architecture is normalized through the Debian name (`x86_64` → `amd64`).
- Fixed the built artifact being written into a `TempDir` that was dropped
  before the output was copied, which made a real conversion fail to produce
  a file (only dry-run paths were exercised before).

### `lx index` installs join the lx lifecycle; `lx install` consumes the indexes

- `ReadIndex::install` now returns an `InstallOutcome` instead of `()`.
  `lx index install` records a prebuilt install as an lx-managed generation,
  so index-installed packages appear in `lx list`/`lx show` and can be
  `lx upgrade`d or `lx rollback`ed. Recipe builds (which only produce
  artifacts) record nothing.
- `lx install` falls back to the enabled indexes (`registry::active_sources`)
  when the `latest-debs` org has no repository/release for the package, and
  gains `--source <index>` to resolve from one named index directly. Both
  routes record the install in the manifest.
- `lx upgrade` and `lx update` fall back to the enabled indexes for packages
  the org doesn't carry: they compare the index's advertised version against
  the installed one and upgrade through the same index. A source that only
  builds from a recipe (AUR) names no version, so it's reported rather than
  guessed.
- `lx search --index` / `--index-only` are wired to the index registry (they
  were previously declared but ignored). `--index` merges the org with the
  enabled indexes, `--index-only` searches the indexes alone; duplicate names
  collapse to the first (org before index, then registry order).
- The swapped `--raw`/`--local` help text on `lx search` is corrected.

### Documentation restructure

- `README.md` is now an overview + quick start; the command reference,
  `package.yaml` reference, host-detection, index, GitHub Action, and
  reproducible-builds sections moved to `docs/reference/` and `docs/guides/`.
- Architecture docs moved to `docs/architecture/`: `plugins.md` split into
  `plugins.md` + `plugin-catalog.md`, `tooling.md` split into `tooling.md` +
  `replacements.md`; the fpm comparison split into a lifecycle page.
- New `docs/README.md` hub and `docs/decisions/README.md` index; all
  cross-document links updated.

### Evaluation docs

- New `docs/evaluation/` with an index and three evidence-based
  quality-attribute evaluations: `interoperability.md` (exchange with the
  packaging ecosystem in both directions), `composability.md` (the
  plugin dimensions as selectable, combinable, extensible parts), and
  `composition-over-inheritance.md` (behaviour built from small composed
  parts rather than a widening base type or repeated `match`). Each
  fixes its rubric, names the code and tests behind every verdict, and
  records the partial/unmet criteria.

### Composition over inheritance: per-format dispatch onto the plugins

- `Packager` gains provided methods so the source-build wrapper, the build
  summary, and config validation no longer `match` on the format:
  `supports_source_build` / `archive_staged_tree` / `generate_source_package`
  (deb/rpm/arch override; other formats inherit a clear error) and
  `artifact_glob` (default `{package}_*.{extension}`, overridden by
  rpm/arch/apk/osxpkg).
- `lib/sourcebuild.rs` and the two `--source` bodies in `lib/build.rs`
  resolve the packager once and delegate; the detach-sign branch now goes
  through the new `signer::apply_post_build`, shared with the binary-build
  signing path.
- `lib/summary.rs` asks the packager for its `artifact_glob`; `lib/config.rs`
  validates `package_format`, `source`, `overrides:` keys, and
  `contents[].packager` against the plugin registries, and
  `effective_distributions_for` reads `Packager::default_distributions()`
  instead of a central `DEFAULT_*` table.
- Evaluation: `docs/evaluation/composition-over-inheritance.md` C8 moves from
  partial to ✅, and new `tests/plugins.rs` cases pin the source-build
  capability and artifact-glob conventions.

### Real-client and live-endpoint verification

- New `tests/verify_clients.rs` closes the last verification gaps:
  a real `pacman -Sy` confirms it accepts the generated `<repo>.db.tar.gz`
  (runs wherever `pacman` and an unprivileged user namespace are available),
  with gated `LX_OPKG=<opkg>` / `LX_DNF=<dnf>` tests for the opkg/rpm
  indexes where those clients exist. The live Gitee and SourceForge clients
  are exercised against the real APIs under `LX_NETWORK_TESTS=1`. All tests
  skip cleanly when the tool or opt-in is absent, so the suite stays
  hermetic.

### Command regrouping

- New `lx deps` group: `lx deps scan` (was `lx scan-deps`) and
  `lx deps resolve` (was `lx shlibdeps`).
- `lx go-native` is now `lx migrate native`; bare `lx migrate` (and
  `lx migrate --repo DIR`) keeps the legacy `lpt` behavior, also reachable
  as `lx migrate lpt`.
- Top-level `lx reinstall` is folded into `lx install --reinstall`: with no
  `--version`, it re-installs an lx-managed package's recorded version;
  otherwise it forces a reinstall of the resolved version.
- `lx json-schema` is renamed `lx schema` (aliases `json-schema`,
  `jsonschema`).
- `lx discover <owner/repo>` is folded into `lx init --from <owner/repo>`,
  which writes the discovered starter `package.yaml` (the old print-only
  behavior is unchanged under the `lx discover` alias).
- `lx get` gains `rollback`, matching the top-level consumer commands.
- The old names keep working as hidden aliases, so existing scripts and
  `action.yml` are unaffected.

### Plugin caveat fixes: scripts, apk index checksum, inline verification

- **apk/ipk maintainer scripts**: `scripts:` (pre/post-install,
  pre/post-remove, and the upgrade hooks) are now emitted for apk
  (`.pre-install`, `.post-install`, `.pre-deinstall`, `.post-deinstall`,
  `.pre-upgrade`, `.post-upgrade`) and ipk (`preinst`/`postinst`/`prerm`/
  `postrm`). ipk also writes a `conffiles` control member from config-typed
  `contents:` entries.
- **apk `APKINDEX` `C:` checksum** is now the `Q1`-prefixed base64 SHA-1 of
  the package's compressed *control* segment (not the whole file), and the
  indexer reads `.PKGINFO` from the control segment of signed packages.
  Verified end-to-end: `apk update` from an lx-generated, RSA-signed local
  repository succeeds under real `apk-tools`.
- **Inline checksum verification**: `Asset` carries provider-published
  checksums — the GitHub/Gitea asset `digest` (`sha256:…`) and SourceForge's
  feed `md5` — verified after download, before sidecar probing, so those
  sources no longer fail closed for want of a `.sha256` sidecar. Build
  provenance records the method as `inline`.

### `lx info` and host-aware defaults

- New `lx info` command auto-detects the host OS and package system
  (read-only, offline): OS/codename from `/etc/os-release`, kernel and
  machine from `uname`, the host package manager, its native package format,
  and the asset dist token `lx install`/`upgrade` match against. `--json`
  emits it machine-readably.
- Other commands now dogfood that detection for smart defaults:
  - `lx convert`'s `--to` is optional and defaults to the host's native
    format (deb/rpm/arch), so `lx convert foo.rpm` on a deb host targets deb.
  - `lx repo`'s `--format` defaults to the host's native format (falling back
    to `deb`) instead of always indexing as apt.
  - `lx init` prompts for `package_format` with the host's native format
    pre-filled, and only asks for Debian suites on deb builds (rpm/arch/apk
    use their built-in distribution sets).

### Format-aware consumer (`lx get` on rpm and pacman hosts)

- The consumer commands (`install`, `upgrade`, `update`, `remove`, `list`,
  `show`, `rollback`, `reinstall`) now dispatch on the host's native package
  format instead of assuming dpkg: `install` gained `--format deb|rpm|arch`
  (default: the host's own manager), install/remove use `rpm`/`pacman` where
  appropriate, and upgrade decisions use the format's own version ordering
  (Debian via `dpkg --compare-versions`; RPM/Arch via RPM `EVR` in
  `lib/versioncmp.rs`).
- The package org is configurable with `LX_INDEX_ORG` (default
  `latest-debs`), so a project can point the consumer client at its own org
  instead of the community one.
- `PackageEntry` now records the native `format`; manifests written before
  the field existed still load and default to `deb`.

### `--format all` and `lx publish`

- `lx build --format all` builds every registered packager; `--format
  deb,rpm` builds each listed format. A single format behaves exactly as
  before.
- New `lx publish` builds every requested format and generates that
  format's repository index in one run, each in its own
  `<output>/<format>/` subdirectory: the producer→distributor loop in one
  command.

### Release binaries and self-packaging

- `.github/workflows/release.yml` cross-compiles musl-static `lx` binaries
  for x86_64/aarch64 on `v*` tag push, refuses to publish a dynamically
  linked binary, publishes `lx-<tag>-<triple>.tar.gz` + `.sha256`, and
  dogfoods `lx build` on its own released assets.
- `action.yml` gained an `lx-version` input to consume a prebuilt release
  instead of always compiling from source (empty keeps the source build).
- `musl` is now declared in the generated JSON schema (`lx schema`).

### PackageIndex plugin dimension (merged RepoIndexer + IndexSource)

- **`RepoIndexer` (write) and `IndexSource` (read) are now one dimension.**
  Both sides are `PackageIndex` backends in `lib/plugins/package_index/`,
  selected by canonical id (`apt`, `opkg`, `pacman`, `apk`, `rpm`,
  `lx-community`, `aur`, `repology`); the user-facing `lx repo --format`
  values are an alias table (`FORMAT_ALIASES`: `deb→apt`, `ipk→opkg`,
  `arch→pacman`, `apk`/`rpm` unchanged).
- The trait defaults every role method and exposes
  `Capabilities::{READ, WRITE}`, so each backend implements only its half
  (write: `file_extension`/`build_index`/`sign_index`; read:
  `search`/`info`/`update`/`install`) rather than stubbing the other. Read
  backends keep the configured `indexes.yaml` name via `instance_name()`
  (used for `IndexHit::source` and the `--repo` filter) while `id()` stays
  canonical; a future local-repo readback backend can implement both roles.
- `get_index_backend()`/`all_index_backends()` replace `get_repo_indexer()`
  and `get_index_source()` in `lib/repo.rs` and `lib/index/registry.rs`; the
  `SourceKind` → constructor `match` is gone (`SourceKind::plugin_kind()`
  maps the config kind to the id). The three read backends are re-exported
  as `crate::index::{lx_community,aur,repology}` so
  `lib/search.rs`/`lib/upgrade.rs`/`lib/repology_depmap.rs` are unchanged.
- The read backends moved from `lib/index/*.rs` and the write backends from
  `lib/plugins/repo/*.rs` into `lib/plugins/package_index/`; `IndexOptions`
  lives there too.
- `tests/index_merge_golden.rs` freezes `lx repo` output for
  deb/ipk/arch/apk (+ multi-suite) as normalized SHA-256 goldens
  (`UPDATE_GOLDENS=1` regenerates) and asserts the registry covers all 8
  backends with the expected capabilities. No behaviour change to `lx repo`
  or `lx index`.

### zip extraction and Alpine apk signing

- **zip ArtifactFormat** now extracts (pure-Rust `zip` crate, deflate):
  `.zip` assets work via `artifact_format: zip` or filename auto-detection,
  preserving unix modes and rejecting traversal entries.
- **Alpine apk signing** (`Signer` backend `apk-rsa`): with `--sign-key`
  pointing at an RSA PEM private key, the apk packager signs the compressed
  control segment with in-process RSA/SHA-1 (the `rsa` crate — no host
  `openssl` needed) and prepends a `.SIGN.RSA.<keyname>` segment.
  `sign_key_id` overrides the key name (default: `<key file>.pub`).
- **Alpine index signing**: `lx repo --format apk --sign-key` RSA-signs the
  whole `APKINDEX.tar.gz` and prepends the signature segment.
- **Fixed apk v2 tar segmentation**: non-final segments (signature, control)
  must not carry end-of-archive records, but `tar::Builder` writes them on
  drop. Real `apk-tools` rejected the result as a bad archive; they are now
  stripped. Verified with `apk-tools` 2.14 and 3.0 (`apk verify`); an
  env-gated test (`LX_APK_STATIC=<apk.static>`) checks a signed package.
- `deny.toml`: the direct-`rsa` ban is dropped — it was already violated
  transitively by `rpm` → `pgp`, and apk signing now uses `rsa` directly.
  The RUSTSEC-2023-0071 exception in `[advisories]` still applies.
- apk control/signature tar segments now omit end-of-archive records, as
  the format requires (the data segment remains the terminator).

### Repository index signing for every format

- `lx repo --sign-key` now signs the index for **opkg** (`Packages.sig`),
  **pacman** (`<repo>.db.tar.gz.sig`), and **rpm**
  (`repodata/repomd.xml.asc`), not just apt. Alpine's `APKINDEX` needs an
  RSA repository key (not OpenPGP), so it reports "unsupported" rather
  than silently skipping. Implemented via `PackageIndex::sign_index`.
- apt `InRelease` is now a real **inline-clearsigned** document
  (`gpg --clearsign`); the previous build wrote an armored *detached*
  signature there. `Release.gpg` (armored detached) is written alongside.

### deb/rpm/arch build parity

- **Arch dependency metadata**: `.PKGINFO` now emits `depend`, `optdepend`,
  `conflict`, `provides`, `replaces`, and `backup` (from `contents:` config
  entries), translating Debian relation syntax to pacman
  (`libc6 (>= 2.34)` → `libc6>=2.34`). The same fields are rendered into the
  `PKGBUILD` for `--source` builds.
- **Arch epoch/packager/signature**: `epoch` is folded into `pkgver` (and the
  filename), `packager` comes from the config, and `--sign-key` writes a
  detached `.sig` (pacman-verifiable).
- **rpm**: `contents:` config entries are marked `%config`/`%config(noreplace)`;
  `Pre-Depends` folds into `Requires` with the PREREQ flag; `epoch` sets the
  RPM epoch header; `rpm.auto_provides`/`auto_requires` scan the payload for
  ELF sonames; `rpm.defines` warns instead of silently doing nothing.
- **Scripts**: rpm/arch maintainer scripts are now read from their configured
  file paths (previously the path string itself was embedded as the script
  body) and `template_scripts` applies to them. The `*_script` upgrade hooks
  are honored by rpm/arch too, so `lx convert` no longer drops scriptlets on
  the arch target.
- **`--bindep`**: ELF-detected dependencies are merged into rpm `Requires` and
  Arch `depends`, not only deb `Depends`.
- **`build_mode: source`** now wraps in the selected `--format` (`rpm`/`arch`,
  not just `deb`), including per-format distributions, signing, and
  source-package output.

### Four cross-cutting plugin dimensions

- **ArtifactFormat** (`lib/plugins/artifact/`) — pluggable archive
  extraction, selected by `artifact_format:` or auto-detected from the
  asset filename. Adds `tar.xz`/`tar.zst` extraction; `zip` is recognized
  with an actionable error. Replaces the `build.rs::extract` match.
- **Signer** (`lib/plugins/signer/`) — signing backends selected by
  `(package_format, sign_method)`: detached `gpg-detach` (now available for
  every format, not just deb) plus embedded `rpm-pgp`/`deb-debsign`.
- **DependencyMapper** (`lib/plugins/depmap/`) — one backend per target
  format (`debian`/`rpm`/`pacman`/`alpine`/`openwrt`). Alpine renders
  `name>=ver` and translates libc/runtime names; OpenWrt translates names
  but keeps opkg syntax. `depmap::map_dependency` routes through the
  registry.
- **PackageIndex (write role)** (`lib/plugins/package_index/`) — `lx repo
  --format deb|ipk|arch|apk|rpm` now writes apt, opkg, pacman, Alpine, or
  RPM repository indexes; the `IndexSource` read role was merged into the
  same dimension later (see above). The apt path delegates to the original
  `repo.rs` implementation.

### Removed: duplicate GitHub source plugin and octocrab

- The second GitHub source plugin (the synchronous client exposed
  separately) is gone. `source: github` is the single GitHub
  implementation, now backed by the blocking-`reqwest` client; the old
  value is no longer accepted.
- Dropped the `octocrab` and direct `tokio` dependencies. lx no longer
  creates an async runtime just to talk to GitHub, and the
  `aws-lc-rs`/`cmake` build requirement is gone. Every forge-source
  client is now synchronous and shares one HTTP/cache path.

### New packagers, forge sources, and build systems

- **Alpine `.apk` packager** (`package_format: apk`): apk-tools v2
  (concatenated gzip control/data members carrying `.PKGINFO` +
  `datahash`). The natural output for `musl: true`; unsigned packages
  install with `apk add --allow-untrusted`.
- **OpenWrt `.ipk` packager** (`package_format: ipk`): opkg `ar`
  container reusing the deb archiver, with an OpenWrt control dialect
  (`Package`/`Version`/`Architecture`/`Installed-Size`/`License`).
- **Gitee forge source** (`source: gitee`): API v5 release/assets,
  `GITEE_TOKEN`/`GITEE_HOST`, zero-config
  `lx build https://gitee.com/owner/repo`.
- **SourceForge forge source** (`source: sourceforge`): the project file
  RSS feed as pseudo-releases; projects use a bare name
  (`github_repo: sevenzip`).
- **Autotools build system** (`build_system: autotools`): `./configure &&
  make && make install`, bootstrapping `configure` via `autogen.sh` /
  `autoreconf -fi` when the tarball didn't ship one.
- **Make build system** (`build_system: make`): plain-`Makefile` projects
  (`make` + `DESTDIR`/`PREFIX` install), auto-detected last so the more
  specific systems get first claim.

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
