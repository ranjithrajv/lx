# Reproducible builds: empirical audit and fix

**Date:** 2026-08-20
**Context:** Asked directly whether `lpt build` output is reproducible per [Debian's definition](https://wiki.debian.org/ReproducibleBuilds) (same source + environment ⇒ byte-identical artifacts, regardless of *when* you build). Verified empirically rather than by code inspection alone.

## Method

Built the same package twice (`eza-community/eza`, `amd64`/`bookworm`, `--source`), ~90 seconds apart, from a cold `lpt build` invocation each time, then diffed the actual output bytes (`sha256sum`, `diff` on the `.dsc`).

## Findings

**The `.deb` came out byte-identical** — but only by coincidence, not by design. `changelog_date()`/`write_copyright()` in `src/build.rs` used `SystemTime::now()`, but truncated to *day* (changelog: `%d %b %Y 00:00:00`) and *year* granularity respectively. Within the same calendar day/year that's accidentally constant. A build made at 23:59 vs. one at 00:01 the next day — or across a New Year's boundary — would embed a different date/year and produce a different `.deb`. Not reproducible per the definition, which requires independence from *when* you build, not "usually the same day."

**The `.dsc` was NOT reproducible** — different SHA256 both times. Traced to `eza_0.23.5.orig.tar.xz` differing in both content *and* size (989972 vs 989836 bytes). `src/source.rs` built it with a raw:

```sh
tar -cJf /work/orig.tar.xz --transform 's|^tree|pkg-ver|' -C /work tree/usr
```

— no `--sort=name`, `--mtime`, or `--owner=0 --group=0 --numeric-owner`. GNU `tar` reads directory entries in filesystem order, which isn't guaranteed stable across separate extractions into fresh temp directories — a well-documented reproducible-builds gotcha. The sibling `debian.tar.xz` *did* come out identical both times, because that one is built by `dpkg-source` itself, which has had reproducibility fixes upstream for years — the gap was specifically in our own hand-rolled `tar` invocation.

## Fix

1. **`src/source.rs`**: added `--sort=name --mtime=@<epoch> --owner=0 --group=0 --numeric-owner` to the orig-tarball `tar` invocation. The mtime respects `SOURCE_DATE_EPOCH` (the reproducible-builds.org standard) if set, else falls back to a fixed epoch (`0`).
2. **`lib/github.rs`**: added `Release.published_at` (Unix epoch from GitHub's own release timestamp, via octocrab's `published_at`), threaded through `build.rs`'s `ResolvedJob`.
3. **`src/build.rs`**: `changelog_date()`/`write_copyright()` no longer call `SystemTime::now()` at all. A shared `reproducible_epoch()` helper prefers `SOURCE_DATE_EPOCH` → the release's own `published_at` → a fixed fallback. Same release ⇒ same metadata, regardless of when it's built.

## Verification

Re-ran the exact same empirical test after the fix: all four artifacts (`.deb`, `.dsc`, `.orig.tar.xz`, `.debian.tar.xz`) came out byte-identical (same SHA256) across two builds ~90 seconds apart. 7 new unit tests cover the day/epoch-boundary determinism, the `SOURCE_DATE_EPOCH` override, and the copyright-year derivation — `cargo test`/`clippy -D warnings`/`fmt` all clean.

## Notes

- `SOURCE_DATE_EPOCH` is honored in both fixed locations (`source.rs`'s tar invocation, `build.rs`'s `reproducible_epoch()`) independently — there's no single shared config plumbing them together, since they're in different modules with no existing coupling. If a third reproducibility-sensitive site is added later, consider centralizing.
- This audit didn't touch Docker image/layer determinism (irrelevant — only the final `.deb`/`.dsc` bytes extracted from the container matter, not the image itself) or `dpkg-deb`'s own `ar`-container timestamp handling (already deterministic in the installed `dpkg-deb` 1.23.7 without any explicit `SOURCE_DATE_EPOCH` from us, confirmed by the identical `.deb` hash before this fix even landed).

## Review date

2027-08-20 — re-verify with the same empirical double-build test if `source.rs`'s tar invocation or `build.rs`'s timestamp handling changes.
