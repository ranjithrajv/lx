# Docker-free `.deb` building (`lib/debarchive.rs`)

**Date:** 2026-08-20
**Context:** A landscape study comparing `lpt` against `deb-get`, `nfpm`, and `fpm` surfaced a real risk: Docker is a hard runtime dependency for `lpt build`, in the same landscape where Docker-free `nfpm` (Go) displaced Ruby-heavy `fpm` largely on that exact axis. Investigated what `nfpm` and `cargo-deb` (Rust) actually do differently.
**Decision context:** core build-path architecture change.

## Finding

A `.deb` is just an `ar` archive of three members (`debian-binary`, `control.tar.gz`, `data.tar.gz`) plus an `md5sums` control file. Neither `nfpm` nor `cargo-deb` shells out to `dpkg-deb`, `tar`, or `gzip` — both build the format directly with library calls. Nothing about the `.deb` format itself required Docker; Docker was only ever standing in for "a place `dpkg-deb` happens to be installed."

`lpt`'s own pipeline had no cross-compilation or execution dependency either: `docker build` without `--platform` always runs the Dockerfile's shell steps as the *host's* architecture, and the packaged binary contents were only ever copied/`chmod`'d, never executed (already noted in `2026-08-14-source-summary-qemu.md`). So removing Docker from this specific step was purely a "stop using a subprocess to write a well-understood file format" change, not an architecture change.

## Decision

Added `lib/debarchive.rs`: builds a `.deb` entirely in-process from a staged filesystem tree + a rendered control string. Dependencies added (mirroring `cargo-deb`'s own choices, verified via crates.io — `ar` 0.9.0, 8.9M downloads, keyword-tagged `deb`; `md5` 0.8.1, 146M downloads, maintained 2026-07):

- `ar` crate — outer archive container.
- `tar` crate (already a dependency) — `control.tar`/`data.tar`.
- `flate2` (already a dependency) — gzip compression, via `GzBuilder::mtime()` for a deterministic header.
- `md5` crate — the `md5sums` control member dpkg policy requires.

`src/build.rs`'s `build_deb_via_docker` (Dockerfile templating + `docker build`/`create`/`cp`) was replaced with `build_deb_native`, which stages the install tree directly with `std::fs` (ELF detection via magic-byte check instead of `file -b`, `chmod` via `PermissionsExt` instead of shell, symlinks via `std::os::unix::fs::symlink` instead of `ln -s`) and calls `lpt_lib::debarchive::build`. `render_dockerfile` and `build.rs`'s own `check_docker` were deleted. `write_control`/`write_changelog` now return `String` instead of writing template files for a Docker `COPY` step to pick up; `changelog.Debian.gz` is gzip'd in-process with the same deterministic-mtime treatment as `debarchive`'s own gzip calls, using the existing `reproducible_epoch()` (release `published_at` / `SOURCE_DATE_EPOCH`) rather than wall-clock time.

**Scope: `build` only.** `--lintian` (`lib/lintian.rs`) and `--source` (`src/source.rs`) still use Docker — lintian has no Rust equivalent to reach for, and `dpkg-source`'s general patch-stack handling wasn't reimplemented (out of scope here; `lpt` only ever produces the empty-patch-stack case, which is a real future opportunity, not attempted in this pass). `check_qemu_for` is kept as a defensive warning for those two flags on the same "may fail" basis it always used, though QEMU was never actually required for any part of this pipeline, `--source`/`--lintian` included, for the same reason (no foreign-arch execution anywhere).

## One format-fidelity trade-off found and accepted

The `tar` crate deliberately strips a leading `./` from stored paths (confirmed from its source: `copy_path_into_inner`'s `Component::CurDir` handling), with no raw-bytes escape hatch for the name field. `dpkg-deb --build` itself stores paths as `./usr/bin/foo` plus an explicit `./` root entry; `lpt`'s native output stores bare `usr/bin/foo` with no root entry. Verified this is still fully valid: a real `dpkg-deb --info`/`--contents` on `lpt`'s output reads it correctly, including the executable bit. Accepted as a cosmetic difference, not a compliance issue.

## Verification

- 5 new unit tests in `lib/debarchive.rs`, including one that shells out to the **real, locally-installed `dpkg-deb`** (not just our own reader) to confirm `--info`/`--contents` parse a generated package correctly and the executable bit survives — skips gracefully if `dpkg-deb` isn't on PATH.
- `stage_install_tree` (the native replacement for the old Dockerfile shell logic) got its own test coverage for flat mode, bundle mode's two symlink shapes (bin/ subdirectory and root-level executables), and the empty-`/usr/bin` failure guard — same cases the old `render_dockerfile` tests covered, ported to the new staging function.
- **Empirically re-verified Docker-freedom directly**, not just by code inspection: ran a real end-to-end build (`eza-community/eza`, live GitHub API, real download) with `docker`'s directory stripped from `$PATH` (confirmed unreachable: `env: 'docker': No such file or directory`). The `.deb` build succeeded; only the (expected, out-of-scope) `--source` step failed with `failed to run docker`.
- **Re-verified reproducibility holds** for the native path with the same empirical method as `2026-08-20-reproducible-builds.md`: built the same package twice, Docker-free, 30 seconds apart — byte-identical SHA256 both times.
- Full suite: 83 tests (42 lib + 41 bin) pass; `cargo clippy --all-targets --all-features -- -D warnings` and `cargo fmt --all -- --check` clean.

## A pre-existing quirk carried over unchanged

`binary_rename`'s old shell logic (`find /output/usr/bin -maxdepth 1 -type f`) doesn't follow symlinks by default, so in `bundle: true` mode — where `/usr/bin` only ever contains symlinks — the rename never actually fires. Verified this empirically against real GNU `find` before porting, then replicated the exact same behavior in `apply_binary_rename` (`DirEntry::file_type().is_file()`, which also doesn't follow symlinks) rather than silently fixing it as a side effect of this refactor. Worth a deliberate look later, but out of scope here.

## Review date

2027-08-20 — re-run the empirical dpkg-deb/reproducibility checks if `debarchive.rs`, `stage_install_tree`, or their dependency versions change.
