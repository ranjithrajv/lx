# Dependency Decision: base64, percent-encoding (and deliberate custom: forge SDK clients)

**Date:** 2026-08-26
**Context:** foss-mind audit of hand-rolled primitives in the source-provider clients added for the plugin architecture (gitlab/gitea/forgejo/bitbucket/gerrit).
**Decision context:** default

## Problem

The provider clients hand-rolled two standard primitives — base64 (Gerrit Basic auth) and percent-encoding (tag/project path encoding, duplicated in 4 files) — and question was raised whether the five REST clients themselves should be replaced by vendor SDK crates.

## Candidates evaluated

| Library | Score | Notes |
|---------|-------|-------|
| base64 0.22+ | ~97/100 | 1.47B downloads, 16k dependents, MIT/Apache-2.0, active; replaced 30-line hand-rolled impl |
| percent-encoding 2.3 | ~95/100 | servo/rust-url ecosystem, 839M downloads; replaced 4× duplicated encoders |
| gitlab crate 0.1902 | ~72/100 | Kitware-maintained but async-first, GitLab-upstream-pinned versioning (`=0.1902.0`, types change in patch releases); adaptation cost ≈ custom cost for our 6 endpoints |
| forgejo-api 0.11 | ~65/100 | Correct + OpenAPI-generated, but small adoption (31k downloads, 5 dependents); Gitea==Forgejo API so one crate could serve both plugins |
| gitea-sdk-rs 0.1.0 | Red ★ | 22 downloads, lone new maintainer, 0 dependents — authenticity/adoption hard disqualifier |

## Decision

**Chosen:** `base64` v0.22+, `percent-encoding` v2.3 — integrated.
**Kept custom (deliberate):** `lib/{gitlab,gitea,forgejo,bitbucket,gerrit}.rs` clients.

For the SDK crates: our per-client surface is ~6 endpoints sharing a unified design (blocking reqwest via `lib/http.rs`, shared `ApiCache` TTL cache, `RawGetter` for checksum sidecars, mapped error strings). The gitlab crate's aggressive-upstream versioning forces exact pins and patch-level type churn — Compatibility Yellow, Dependencies Yellow; async-first shape mismatches our blocking pipeline (Compatibility Yellow). Adaptation cost ≥ custom cost at current scope → keep custom per the decision flow.

**Score:** base64 97, percent-encoding 95 (threshold: 60)

## Integration notes

No wrappers needed — direct crate use. The kept-custom clients are the "wrapper pattern" in spirit: narrow, documented, and swappable behind `SourcePlugin`; if a provider's surface grows beyond releases/tags/license, re-evaluate the matching SDK crate.

## Review date

2027-02-26 (6 months) — re-score if: any client gains >10 endpoints, gitlab crate ships a blocking-first API or relaxes version pinning, or forgejo-api adoption grows 10×.
