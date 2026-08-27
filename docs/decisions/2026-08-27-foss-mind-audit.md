# Dependency Decision: foss-mind audit of lib/ hand-rolled primitives

**Date:** 2026-08-27
**Context:** foss-mind sweep over all custom code in `lib/` (~12K lines post src→lib merge) to verify no battle-tested crate already solves what we hand-rolled.
**Decision context:** long-lived / core dependency · threshold 70 · Maintenance ×2, Ecosystem health ×2
*(Extends `2026-08-26-foss-mind-audit.md`, which integrated base64 + percent-encoding and kept the five forge clients custom.)*

## Candidates evaluated

| Area (custom LOC) | Candidate | Score | Verdict |
|---|---|---|---|
| `.deb` construction — `debarchive.rs` + `plugins/deb.rs` (~700) | arx-pack 0.3.5 | ~80/100 | Keep custom |
| `.deb` construction | debian-packaging 0.18 | Red ★ Maintenance (released 2024-11, project archived) | Eliminated |
| Arch pkg — `archarchive.rs` (303) | alpm-package 0.4.2 | ~75/100 | Keep custom |
| Wizard prompts — `wizard.rs` prompt/prompt_yes (~40) | dialoguer 0.12.0 | ~95/100 | **Integrate ✔** |
| Progress — `progress.rs` (191) | indicatif 0.18 | Fit fail (see below) | Keep custom |
| Download cache — `cache.rs` (247) | cacache 13.1.0 | ~77/100 | Keep custom |
| HTTP retry — `http.rs` (60) | backon 1.6.0 | Low ROI (60-line wrapper vs retry-policy crate) | Keep custom |
| Debian version compare — `debs.rs:is_newer` | debversion 0.5.4 | N/A fit (already shells to `dpkg --compare-versions`, which IS the library) | Keep dpkg shellout |

All crates verified authentic via the crates.io API before scoring (`cargo tree` cross-checked).

## Decisions

### dialoguer ~95 — integrated (this change)

console-rs org backing (bus factor > 2), 78M downloads, updated 2025-08,
single transitive dep (`console`). Replaced the print/read_line prompt pair
with real TTY handling (Ctrl-C, echo, history). Only Yellow: permissive
MIT/Apache license (half credit under copyleft preference). Error policy
preserved: EOF/Ctrl-C/non-TTY stdin fall back to the default answer so a
piped stdin still produces a config — same semantics as the old code.
Feature-minimal install: `default-features = false` (Input/Confirm/themes
are core; fuzzy-select etc. unused).

### arx-pack ~80 — keep custom, watch-list

Cleared the numeric threshold but failed the adaptation-cost node: it covers
"stage files + control + md5sums" but not lpt's relation fields, conffiles/
contents entries, epoch-aware versions, or XZ source packages. Migrating
would land mid-way between two APIs while re-risking the empirically
verified byte-reproducibility guarantees. Authenticity confirmed on crates.io
(created 2026-06-18), active development — but Adoption Red at audit time:
477 downloads, zero dependents, bus-factor-1 solo maintainer.

### alpm-package ~75 — keep custom, partial fit only

Genuine find: official Arch Linux GitLab repo, spec-exact ALPM metadata. But
it requires pre-rendered BUILDINFO/PKGINFO/MTREE inputs, so archarchive.rs
keeps most of its logic anyway (~50% coverage < the 95% compose bar). Right
replacement target if `.MTREE` policy drifts upstream.

### indicatif rejected on fit, not score

progress.rs is a deliberate *cross-process* coordinator (parallel workers
share state through a JSON file); indicatif is single-process. No
replacement exists for that shape.

## Security findings (cargo audit + cargo tree)

1. **RUSTSEC-2023-0071** (`rsa` 0.9.10, Marvin timing side-channel, no fixed
   upgrade) previously reached the binary via two paths:
   `octocrab → jsonwebtoken → rsa` and `rpm → pgp → rsa`.
   **Path 1 removed in this change**: octocrab compile-requires one JWT
   crypto backend; switched from `jwt-rust-crypto` to `jwt-aws-lc-rs`, whose
   RSA is BoringSSL C (the advisory crate is never linked through that
   path; `cargo tree -i rsa@0.9.10` now shows only the pgp path). Trade-off:
   aws-lc-rs needs cmake on build hosts — GitHub runners ship it.
   Note lpt never performs RSA private-key ops over HTTP responses; the
   exposure was unreachable code, still worth unlinking.
2. **`rpm = "0.16"` is 11 majors stale** (latest 0.27.1, 2026-08) and anchors
   the remaining rsa path. No direct advisories filed against 0.16, but the
   migration unblocks security evolution of both rpm and pgp crates.
   Breaking `PackageBuilder` API changes expected — scheduled ticket below.
3. Otherwise clean: no other advisories across 404 lockfile deps.

## Scheduled evaluation ticket: rpm 0.16 → 0.27 migration

**Trigger:** cargo audit flags RUSTSEC-2023-0071 via `pgp 0.14`; no fixed
upgrade exists under the current major pins.
**Scope:** `lib/rpmarchive.rs` is the only consumer (`PackageBuilder`,
`FileOptions`, `signature::pgp::Signer`). Expect signature/type churn;
runtime contract unchanged.
**Acceptance:** `cargo tree -i rsa@0.9.10` empty or clean; `rpm -K` verifies
a signed package; reproducible-build test suite green.
**Estimate:** focused half-day (one file + its tests).

## Review date

2027-02 (watch-list re-score: arx-pack adoption growth; alpm-package API
stability; re-run full scoring on any Yellow-signal dependency).
