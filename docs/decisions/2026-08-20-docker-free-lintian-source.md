# Docker-free lintian and source packages

**Date:** 2026-08-20
**Context:** Follow-on to `2026-08-20-docker-free-deb-build.md`, which scoped `.deb` building's Docker removal deliberately narrow: `--lintian` and `--source` were left as Docker-only, since lintian has no Rust equivalent and `dpkg-source`'s general patch-stack handling wasn't attempted. Asked directly to close both gaps.
**Decision context:** completes the Docker-removal work; both remaining Docker call sites in the codebase.

## `--lintian` (`lib/lintian.rs`)

Not attempted as a reimplementation -- lintian is a large, Debian-native Perl static analyzer with dozens of independently-maintained checks; there's no reasonable "just port it to Rust" here, and that was never the actual blocker. The real fix: `run()` now checks for a host-installed `lintian` binary first (`lintian --version` succeeds) and calls it directly; only falls back to the existing Docker container path when no host `lintian` is found. The parsing logic (`parse()`, unchanged) doesn't care which produced the output.

Verified without touching this machine's real package set (no `lintian` in Arch's official repos; installing via AUR would be a heavier, more invasive action than this warranted) — built a fake `lintian` shell shim on `PATH` alongside a fake `docker` that fails, and ran `lpt build --lintian` for real against a live GitHub release. The fake shim's exact fake output (`E: eza: bad-version` etc.) appeared verbatim in the report, and the fake `docker` was never invoked (confirmed by its absence from stderr, which it would have written to if called) -- direct proof the host-first dispatch works, not just that the code compiles.

## `--source` (`src/source.rs`)

Reimplemented in full, not host-tool-delegated -- unlike lintian, `lpt` only ever produces the narrow slice of `dpkg-source` its own pipeline needs (format "3.0 (quilt)", always an empty patch stack, since there's no patch-maintenance workflow here), so the general tool's complexity was never actually in play. Three things `dpkg-deb -x`/`tar`/`dpkg-source -b` did in a container before:

1. **Extracting the reference `.deb`** — new `lpt_lib::debarchive::extract()` (ar read + gzip/plain data.tar unpack). Scoped to what `lpt` itself produces (gzip data.tar), not a general dpkg-deb-compatible reader, since the only caller extracts a `.deb` this same run just built.
2. **Building `.orig.tar.gz`** — generalized the existing `.deb`-building tar/gzip machinery into a new public `lpt_lib::debarchive::tar_gz_tree(fs_root, archive_prefix, mtime)`, parameterizing the archive-path prefix (`data.tar` needs `"./"`-relative paths; a source tarball needs `"<pkg>-<version>/usr/"`-relative ones). Same sorted-order/normalized-mtime treatment as the `.deb` path.
3. **`dpkg-source -b`** (the `.dsc` + `.debian.tar.gz`) — `debian.tar.gz` is just `tar_gz_tree` again (tarring the `debian/` directory, prefix `"debian/"`); the `.dsc` is hand-written text. Verified its exact field order/content against a real `dpkg-source -b` run first (not guessed): `Format`/`Source`/`Binary`/`Architecture`/`Version`/`Maintainer`/`Homepage`/`Standards-Version`/`Build-Depends`/`Package-List`/`Checksums-Sha1`/`Checksums-Sha256`/`Files`, each checksum section listing the orig tarball *before* the debian tarball -- confirmed this isn't alphabetical (the filenames would sort the other way) but a fixed convention, and matched it exactly rather than guessing. New `sha1` dependency (RustCrypto, matching the existing `sha2` version/style) for the `Checksums-Sha1` section; `md5`/`sha2` were already available.

### Compression: gzip, not xz -- a disclosed trade-off

The prior Docker path produced `.orig.tar.xz`/`.debian.tar.xz`. Checked for a pure-Rust XZ encoder to preserve that exactly: `lzma-rust2` looked promising (9.3M recent downloads, "100% standard Rust") but its current release requires rustc 1.85, while this project's declared `rust-version = "1.75"` MSRV made `cargo add` silently fall back to a 17-minor-version-old 0.2.2 (a large enough gap on a pre-1.0 crate that the API I'd researched likely wouldn't even match). `xz2` (FFI to liblzma) works but reintroduces a build-time C-library dependency, which sits against the spirit of this whole effort even though it's not a *runtime* Docker dependency. Chose gzip instead: already a dependency (`flate2`), zero MSRV risk, and Debian source format 3.0 accepts it as fully policy-compliant. This is a real, visible behavior change (different filenames), not silently absorbed -- renamed throughout (`README.md`, doc comments) rather than left inconsistent.

## Verification

- New `lib/debarchive.rs` tests: `extract_round_trips_build` (build then extract, byte-for-byte content and executable-bit-preserved), `extract_rejects_deb_with_no_data_tar`, `tar_gz_tree_uses_the_given_prefix_not_dot_slash`.
- New `src/source.rs` tests: `debian_version_strips_leading_non_digits`, and a `.dsc` field-order/content test built directly from the real `dpkg-source -b` reference captured before implementing.
- **Empirically verified end-to-end, not just unit-tested**, with the same fake-`docker`-that-fails shim used for the lintian check: ran `lpt build --source` against a live GitHub release with Docker genuinely unreachable. Succeeded, producing `.deb`/`.dsc`/`.orig.tar.gz`/`.debian.tar.gz`.
- **Fed the output back to real Debian tooling**: `dpkg-source -x` on the natively-generated `.dsc` correctly reconstructed the full source tree (warned "extracting unsigned source package" as expected -- the prior Docker path never signed either), including regenerating the `.pc/` quilt bookkeeping directory itself. Confirmed the reconstructed `usr/bin/eza` is a genuine, correctly-permissioned (`-rwxr-xr-x`) ELF executable and `debian/control` matches exactly.
- `check_qemu_for` (and its two call sites) removed as dead code once this landed: it was already unneeded for the native `.deb` build (see the prior decision doc), and now neither `--lintian`'s Docker fallback nor `--source` (no longer using Docker at all) executes a foreign-arch binary either, so nothing in the pipeline can hit the case it warned about.
- Full suite: 89 tests (45 lib + 44 bin) pass; `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`, and the full pre-push hook tier (including `cargo audit` on the new `sha1` dependency) all clean.

## Known limitations, disclosed rather than silently accepted

- **`.tar.gz` not `.tar.xz`** for source-package tarballs (see above) -- larger output, still fully valid.
- **Missing top-level directory self-entries**: `tar_gz_tree`'s walker never emits an entry for its own root (only for children found while recursing), so the orig tarball's `<pkg>-<version>/usr/` and the debian tarball's `debian/` directory itself aren't listed as their own tar members, unlike a real `dpkg-source -b`/`tar` run. Harmless for extraction (any real tar reader creates missing parent directories automatically, and `dpkg-source -x` proved this above) but a cosmetic difference from byte-identical fidelity to the old Docker path, same category as the `.deb`'s own `./`-prefix limitation noted in the prior decision doc.
- **`--lintian` now hard-requires a host `lintian`, no fallback at all** (see 2026-08-20 addendum below) -- if it's not on `PATH`, the build fails with a clear message rather than trying anything else.

## Addendum (2026-08-20, same day): Docker fallback removed entirely

The Docker fallback described above for `--lintian` was a deliberate middle ground at the time -- asked directly afterwards to remove it too, closing out every Docker code path in the project (`grep -rl '"docker"' src/ lib/` now returns nothing). `lib/lintian.rs` lost `run_docker`, `check_docker`, `ensure_lintian_image`, and the `lpt-lintian` image constant; `run()` now only ever calls the host `lintian` binary, with a clear error (distinguishing "not on PATH" from other spawn failures) instead of a silent fallback.

Two follow-on consequences, both handled rather than left as surprises:

- **`action.yml`'s `docker/setup-qemu-action` step was already dead weight** once `--source` went native in the base work above (QEMU was never actually required anywhere in this pipeline -- confirmed when `check_qemu_for` itself was deleted as dead code). Removed it.
- **CI runners don't ship `lintian`, and there's no Docker fallback to save that anymore.** Added a conditional `apt-get install lintian` step to `action.yml`, gated on `inputs.lintian-check == 'true'`, so the action's own `--lintian` support keeps working on GitHub-hosted runners.

Verified without any shim this time -- this dev machine genuinely has no `lintian` installed and no Docker fallback to fall into: `lpt build ... --lintian` failed with exactly the intended message (`--lintian requires the \`lintian\` binary on PATH (e.g. \`apt-get install lintian\` on Debian/Ubuntu); not found`), not a Docker-related error.

## Review date

2027-08-20 — re-run the empirical Docker-absent + real-dpkg-source-reconstruction checks if `source.rs`, `lib/debarchive.rs`'s tar/extract functions, or `lib/lintian.rs` change. Re-check `lzma-rust2`'s MSRV if this project's own `rust-version` is ever raised past 1.85 -- native XZ becomes viable again at that point.
