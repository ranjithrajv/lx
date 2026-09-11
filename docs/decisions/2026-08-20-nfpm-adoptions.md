# Adoptions from nfpm: relations, ancillary files, epoch

**Date:** 2026-08-20
**Context:** After completing the Docker-free work (`2026-08-20-docker-free-deb-build.md`, `2026-08-20-docker-free-lintian-source.md`), did a fresh feature comparison against nfpm (a mature, widely-used Go tool covering similar ground). Asked to adopt three of the resulting recommendations; a fourth (auto-detecting shell completions) was deliberately excluded as too ambiguous to do safely without an explicit config field.

## 1. Dependency relations beyond `depends`

`PackageConfig` (`src/config.rs`) gained `recommends`, `conflicts`, `replaces`, `provides`, `breaks` — all `String`, `#[serde(default)]`, same shape as the existing `depends`. `write_control` (`src/build.rs`) renders each via a new `relation_field(name, value)` helper that emits `"{name}: {value}\n"` only when non-empty, so unset fields simply don't appear (matching Debian control-file convention — no empty `Recommends:` lines). `Suggests`/`Pre-Depends` were deliberately left out, matching the reviewed recommendation.

## 2. Ancillary file preservation in flat-mode builds

Flat-mode builds (a release tarball with no internal FHS layout) previously kept only ELF binaries from the release's top level and silently discarded everything else — man pages, `LICENSE`/`COPYING`/`NOTICE` files. Bundle mode was never affected: it already preserves the whole tree via `copy_dir_recursive`.

`stage_install_tree` (`src/build.rs`) now takes an added `mtime: i64` and, for each non-ELF top-level file, calls a new `stage_ancillary_file`, which:
- recognizes `*.1`–`*.9` (optionally `.gz`) as man pages via `man_section`, gzips them (`gzip_file_to`, `mtime`-normalized for reproducibility) into `/usr/share/man/man<N>/`;
- recognizes `LICENSE*`/`COPYING*`/`NOTICE*` case-insensitively via `is_license_like`, copying into `/usr/share/doc/<pkg>/`;
- otherwise does nothing — no attempt at shell-completion detection (the excluded 4th recommendation), since guessing at `bash`/`zsh`/`fish` completion file conventions without an explicit signal risks silently misplacing files.

Verified via `stage_install_tree_flat_mode_installs_man_pages_and_license` (synthetic binary dir with a `.1` man page and a `LICENSE` file, asserting both land at the correct FHS paths) plus `man_section_recognizes_plain_and_gzipped_pages` and `is_license_like_matches_common_names_case_insensitively`. Not re-verified against a real GitHub release in this original pass — `eza-community/eza`'s release tarball only ships `./eza` at the top level, so it couldn't exercise this path.

**Update (2026-08-21, later validation pass):** verified live against a real release that actually ships top-level ancillary files. `BurntSushi/ripgrep`'s musl release tarball ships `LICENSE-MIT` and `COPYING` at its top level (its man page, `doc/rg.1`, is nested and correctly *not* picked up, matching the deliberate top-level-only scope). Built the package for real and confirmed via `dpkg-deb --contents` that both landed exactly where expected: `usr/share/doc/ripgrep/COPYING` and `usr/share/doc/ripgrep/LICENSE-MIT`, alongside the normal `usr/share/doc/ripgrep/{changelog.Debian.gz,copyright}`. This closes the previously-open verification gap.

## 3. Epoch support

`PackageConfig` and `source.rs`'s `Pkg` both gained `epoch: String` (`#[serde(default)]`). `with_epoch` (previously private to `build.rs`, computing `cfg`-derived epoch prefixing) was refactored to `pub(crate) fn with_epoch(epoch: &str, version: &str) -> String` taking the epoch directly, so `source.rs` can reuse it without needing a `PackageConfig`.

Applied only to `Version:`-bearing content — control file, changelog entries, `.dsc` `Version:` field — never to filenames, per Debian policy (`:` isn't filename-safe, and epoch is conventionally omitted from `.deb`/source-package names). `source.rs` now computes `content_version` (epoch-prefixed) separately from the pre-existing `src_version` (epoch-free, still used for `tree`/`debian_tar_name`/`dsc_name`); `render_dsc`'s parameter was renamed from `src_version` to `content_version` to make this split explicit at the call site rather than implicit.

## Verification

- New unit tests: `parses_dependency_relations_and_epoch` (config.rs); `write_control_includes_all_relation_fields_when_set`, `write_control_prefixes_version_with_epoch_but_not_filename`, `write_control_omits_epoch_prefix_when_unset`, `write_changelog_includes_epoch_in_version`, `stage_install_tree_flat_mode_installs_man_pages_and_license`, `man_section_recognizes_plain_and_gzipped_pages`, `is_license_like_matches_common_names_case_insensitively` (build.rs); `content_version_gets_epoch_but_filenames_dont` (source.rs).
- **Live end-to-end build**, not just unit tests: built against `eza-community/eza` with all five relation fields and `epoch: "1"` set in `package.yaml`, then inspected the real output with actual `dpkg-deb --info`/`--contents` (not this project's own code) — confirmed `Version: 1:0.23.5-1+bookworm` and all six relation lines (`Depends`/`Recommends`/`Conflicts`/`Replaces`/`Provides`/`Breaks`) rendered correctly, filenames stayed epoch-free.
- Full suite: 98 tests (45 lib + 53 bin) pass; `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, and the full pre-push hook tier (coverage gate ≥40%, `cargo audit`) all clean.

## Review date

2027-08-20 — if a real upstream release with a top-level man page or `LICENSE` file is packaged through this tool, spot-check its `dpkg-deb --contents` output against item 2's expectations as a bonus live confirmation (not required — the unit test already covers the logic directly).
