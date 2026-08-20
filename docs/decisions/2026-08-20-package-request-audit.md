# Package-request audit: fail-closed verification, provenance, prerelease warnings

**Date:** 2026-08-20
**Context:** Asked directly what enhancements to recommend for auditing a "package request" -- there was no existing feature or term by that name. Surveyed the existing download/verification pipeline (`src/build.rs`, `lib/checksum.rs`, `lib/github.rs`) to ground a concrete answer instead of guessing, since `lpt build` downloads arbitrary upstream binaries and repackages them into `.deb`s that get installed, often via sudo. Recommended four enhancements ranked by security value vs. cost; asked to implement the top three.

## 1. Fail closed on unverified downloads (was: warn and continue)

`verify_sidecar_or_warn` (`src/build.rs`) downgraded a missing checksum sidecar to a printed warning and proceeded with the build regardless. Most real-world GitHub releases don't publish a `.sha256`/`.sha256sum` sidecar, so this was the *default* path for most repos: zero integrity verification, with only a console line -- easy to miss in CI logs -- as evidence it happened.

Replaced with `verify_sidecar_or_require_flag`, which now fails the build unless one of:
- `--pinned-metadata` supplies a vetted SHA-256 for this asset + version (unchanged, already the strongest option and tried first), or
- a live sidecar matches (unchanged), or
- the new **`--allow-unverified`** flag is passed, in which case it still proceeds but with a louder `⚠` warning naming exactly what wasn't verified, or
- `--no-verify` was passed, which (unchanged) skips verification entirely and takes priority over everything else.

The error message on the default (neither flag) path names both escape hatches (`--allow-unverified`, `--pinned-metadata`/sidecar) so the failure is actionable, not just a dead end.

## 2. Provenance recorded into `build-summary.json`

Added a `VerifyMethod` enum (`Pinned`/`Sidecar`/`UnverifiedAllowed`/`SkippedNoVerify`) and threaded a shared `Arc<Mutex<Vec<Value>>>` provenance collector through `build_jobs`'s worker threads into `build_one`, so every unique asset download (one entry regardless of how many arch/dist jobs share it) gets a record: `{ asset, url, tag, method, sha256 }`. `sha256` is computed unconditionally (even on the `--no-verify`/`--allow-unverified` paths) so the audit trail always has *something* to compare against later, even when verification itself didn't happen.

`SummaryInputs` (`src/summary.rs`) gained a `provenance: Vec<Value>` field; `write()` embeds it as `"provenance"` in `build-summary.json` and adds a convenience `"unverified_assets"` count (assets whose method wasn't `pinned` or `sidecar`), so a CI pipeline or a human skimming the summary doesn't have to parse every entry to spot a problem.

This only fires with `--summary` (unchanged pre-existing gate) -- no new flag needed, the plumbing already existed via `Telemetry::summary_json()`'s sibling pattern.

## 3. Prerelease/draft surfaced, not silently ignored

`Release.prerelease`/`.draft` (`lib/github.rs`) were already parsed off every GitHub API response but never read anywhere outside their own field assignment -- confirmed via `grep -rn "\.draft\b\|\.prerelease\b"` returning zero non-definition call sites before this change. Someone pinning an explicit `version:` tag that happens to be an RC/nightly got no signal they were about to package pre-stable software as if it were a normal release.

New `pub(crate) fn warn_if_prerelease_or_draft` (`src/build.rs`) prints a `⚠️` line for either flag and is called from both `lpt build`'s `run()` (right after resolving the release, before any download starts) and `lpt validate`. Removed the now-inaccurate `#[allow(dead_code)]` annotations on both fields since they're genuinely read now.

## Not implemented (4th recommendation, out of scope for this pass)

Real signature/attestation verification (`cosign verify-blob`/`gh attestation verify`, hooking off an extended `--pinned-metadata` schema) was the fourth, higher-value-but-higher-cost recommendation -- a new subprocess dependency, and only useful for the subset of upstreams that publish Sigstore bundles or GitHub artifact attestations. Held back pending a concrete upstream that needs it, per the original recommendation.

**Deliberately not touched:** `src/debs.rs`'s own `verify_sidecar_or_warn`, used by `lpt install`/`lpt upgrade`. That path installs an already-built `.deb` from `lpt`'s own `latest-debs` GitHub org (a different trust boundary than packaging arbitrary upstream binaries) and already hard-fails on a checksum *mismatch* -- it only warns on a *missing* sidecar, which is a narrower and already more defensible gap than the one closed here. Left as a candidate for a future pass if warranted, not folded in silently.

## Verification

- New tests: `verify_method_as_str_names_every_variant` (`src/build.rs`), `release_from_octocrab_maps_prerelease_and_draft` (`lib/github.rs`, alongside an added assertion on the existing non-prerelease fixture), `provenance_is_embedded_and_unverified_assets_are_counted` (`src/summary.rs`).
- **Live, not just unit-tested**: built `eza` for real with no pin/sidecar available -- confirmed the default path now fails with the intended message; confirmed `--allow-unverified --summary` succeeds and `build-summary.json`'s `provenance`/`unverified_assets` fields are populated correctly with the real asset URL, tag, and computed SHA-256. Ran `lpt validate` against `neovim/neovim`'s real `nightly` prerelease tag (`gh api` used to find a live prerelease across several repos) and confirmed the prerelease warning fires against real GitHub API data, not a mock.
- Full suite: 101 tests (46 lib + 55 bin) pass; `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, and the full pre-push hook tier (coverage gate, `cargo audit`) all clean.

## Review date

2027-08-20 -- revisit the 4th recommendation (signature/attestation verification) if a concrete upstream that publishes Sigstore bundles or GitHub attestations comes up. Revisit `src/debs.rs`'s install-time verification if `lpt install`/`lpt upgrade` ever source packages from outside the `latest-debs` org.
