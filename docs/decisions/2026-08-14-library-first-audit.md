# Dependency Decision: library-first audit of hand-rolled modules

**Date:** 2026-08-14
**Context:** The parity work added several hand-rolled implementations. Before treating them as shipped, audit each against existing crates ("search before you ship").
**Decision context:** long-lived / core dependency (threshold 70)

## Custom code inventory

| Module | Lines | What it does | Possible crate |
|--------|-------|--------------|----------------|
| `summary.rs` `rfc3339` + `civil_from_days` | ~30 | RFC3339 UTC timestamp for build-start/end | `jiff` |
| `summary.rs` `glob_match` | ~30 | Match `{pkg}_*.deb` file names | `glob` |
| `build.rs` `build_jobs` worker pool | ~60 | Bounded thread pool, work-stealing by arch group | `rayon` |
| `optimize.rs` `detect_cores/memory/disk` | ~60 | Read /proc + statvfs for parallelism tuning | `sysinfo` |
| `progress.rs` `draw()` | ~25 | Inline `\r` terminal progress bar | `indicatif` |
| `cache.rs` flock | ~10 | Advisory file lock for concurrent cache access | `fd-lock` |

## Candidates evaluated

Crates verified to exist on crates.io (authenticity gate passed). **MSRV is the hard
compatibility gate — the project declares `rust-version = "1.75"`.**

| Library | Score | Notes |
|---------|-------|-------|
| `jiff` 0.2.35 | 95/100 | **MSRV 1.70 ✓** compatible. Unlicense OR MIT. 160M downloads, BurntSushi (ripgrep author). Replaces `civil_from_days` (~25 lines, hand-rolled Howard Hinnant algorithm). Clear fit. |
| `time` 0.3.55 | 40/100 | **MSRV 1.88 ✗** — incompatible with 1.75. Would need ancient pin. Rejected on compatibility. |
| `chrono` 0.4.45 | 50/100 | 734M downloads, but heavy (pulls `iana-time-zone` etc.) and previous decision log deliberately avoided it. `jiff` is lighter and MSRV-compatible. |
| `glob` 0.3.4 | 90/100 | **MSRV 1.63 ✓** compatible. MIT/Apache-2.0. rust-lang owned. Replaces `glob_match` (~25 lines). Clear fit. |
| `globset` 0.4.20 | 30/100 | **MSRV 1.88 ✗** — incompatible. Rejected. |
| `rayon` 1.12.0 | 35/100 | **MSRV 1.80 ✗** — incompatible with 1.75. Also our worker pool holds non-`Sync` `GitHubClient` + per-worker state, so rayon's work-stealing model doesn't fit cleanly. Custom pool is correct. |
| `sysinfo` 0.39.6 | 30/100 | **MSRV 1.95 ✗** — incompatible. Also heavier than 3 focused `/proc` + `statvfs` reads. Custom detection is correct. |
| `indicatif` 0.18.6 | 35/100 | **MSRV 1.85 ✗** — incompatible. Also our `\r` bar is ~20 lines gated to TTY; adding a 50-dep tree for that is not justified. Custom is correct. |
| `fd-lock` 4.0.4 | 70/100 | MSRV compatible, MIT/Apache-2.0. But we already depend on `libc` (flock is 2 syscall lines); adding a crate for that is marginal. Keep `libc::flock`. |

## Decision

**Chosen:** `jiff` v0.2.35 (for RFC3339 dates) and `glob` v0.3.4 (for filename globbing).
**Licenses:** Unlicense OR MIT; MIT OR Apache-2.0.
**Scores:** 95/100 and 90/100 (threshold: 70).
**MSRV impact:** both compatible with rust-version 1.75. No toolchain bump needed.

Rejected with rationale: `rayon`, `sysinfo`, `indicatif`, `fd-lock` are all legitimately
skipped — either MSRV-incompatible at the versions required, or the custom code is
already the right tool for the job (a ~20-line TTY bar vs a 50-dep progress library).
`chrono` and `time` lose to `jiff` on weight and MSRV respectively.

## Integration notes

- Replace `civil_from_days` + `rfc3339` in `summary.rs` with
  `jiff::Timestamp::from_second(epoch)?.to_string()` (RFC3339) — removes the
  `pub(crate) fn civil_from_days` used by `telemetry.rs` too.
- Replace `glob_match` with `glob::Pattern::new(&format!("{}_*.deb", pkg))?.matches(name)`.
- The two custom date/glob helpers were ~50 lines total; both replacements are drop-in
  and keep the same output format (tests assert exact strings).
- Signals that were Yellow and what flips them: none — both candidates are Green on
  every starred signal (authenticity, security, license, vendor independence).

## Status: APPLIED (2026-08-14)

- `jiff = "0.2.35"` and `glob = "0.3.4"` added to `Cargo.toml` (both MSRV ≤ 1.75;
  transitive `jiff-core`/`jiff-static`/`jiff-tzdb`/`jiff-tzdb-platform` are MSRV 1.70,
  `portable-atomic`/`portable-atomic-util` MSRV 1.34 — no toolchain bump).
- Removed all three hand-rolled helpers:
  - `summary.rs`: `rfc3339` now `jiff::Timestamp::from_second(...).strftime("%Y-%m-%dT%H:%M:%S%z")`;
    `civil_from_days` deleted; glob now `glob::Pattern`.
  - `telemetry.rs`: `timestamp()` uses jiff (`%Y-%m-%d %H:%M:%S`).
  - `build.rs`: `changelog_date()` uses jiff RFC 2822 — now emits the weekday
    (`Fri, 14 Aug 2026 00:00:00 +0000`) per Debian policy; the old hand-rolled
    output lacked it. This is an intentional improvement, and `civil_from_days` deleted.
- Fixed a latent test race surfaced by the swap: both telemetry tests previously
  shared one pid-based temp dir; each now uses `tempfile::tempdir()`.
- Verified end-to-end: `mkdeb build -v 0.9.3 ... --source --summary` produced a .deb
  + source package; `build-summary.json` carries RFC3339 timestamps, extracted
  `changelog.Debian.gz` shows the RFC 2822 trailer with weekday.
- Gate: 37 tests pass, `cargo fmt --check` clean, clippy clean (only the pre-existing
  `large_enum_variant` warning on the `Commands` enum in `cli.rs`), `cargo audit`
  unchanged (single pre-existing, accepted RUSTSEC-2023-0071).
- Note: the unrelated uv 0.9.3 e2e initially failed because `uv-debian/package.yaml`
  has no `binary_path` and uv's tarball nests binaries in a subdir — pre-existing
  config gap, not caused by this change; confirmed working with `binary_path` set.

## Review date

2027-08-14 — re-check `jiff`/`glob` MSRV and any CVEs (both have excellent maintenance;
a maintenance regression or new critical CVE would trigger a re-score).
