# Changelog

## Unreleased

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
- `lx-get install` falls back to a `+musl_{arch}.deb` asset when no
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

- The CLI is now `lx` (`lx-lib` crate, `lx-get` thin client). Env vars are
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
