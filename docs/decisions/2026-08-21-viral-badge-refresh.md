# Viral badge: fixed stale branding, added release/downloads/coverage badges

**Date:** 2026-08-21
**Context:** Asked what shields.io badges make sense for repos publishing packages built via `lpt`. Answering that question surfaced a real bug: `src/summary.rs`'s `viral_badge` (printed after every `--summary` build, meant to be pasted into the packaging repo's own README) still said "Built with debian-multiarch-builder" and linked to `ranjithrajv/debian-multiarch-builder` -- the bash predecessor this whole project rewrote, not `lpt` itself. Every build using this tool was advertising the wrong project. Asked to add three of the recommended badges (Latest release, Downloads, Supported suites/architectures); fixed the branding while touching the same function.

## What changed

- **Branding**: "Built with debian-multiarch-builder" → "Built with lpt", linking to `github.com/ranjithrajv/lpt` (matching `Cargo.toml`'s own `repository` field). The trailing "Try it free: `./build.sh --setup`..." line referenced the old bash tool's script interface too -- replaced with a real `lpt build` example matching the README's own Quick Start.
- **Latest release / Downloads badges**: `img.shields.io/github/v/release/<repo>` and `.../downloads/<repo>/total`. These need to know the repo *publishing* the built `.deb` as a GitHub release -- a different thing from `github_repo` (the upstream source being packaged, e.g. `eza-community/eza` when packaging `eza`). The publishing repo is only reliably knowable when actually running as the GitHub Action, via the standard `GITHUB_REPOSITORY` env var GitHub Actions sets automatically. On a local `lpt build --summary` run (no such env var), these two badges are omitted entirely rather than guessed -- there's no other trustworthy source for "where does this get published," and a wrong guess presented as confidently as a real answer would be worse than no badge at all (same principle already applied to `scan-deps`'s soname→package-name resolution).
- **Suites/architectures badges**: static `img.shields.io/badge/...` badges built from `inputs.distributions`/`architectures`, already available on every build regardless of context. New `badge_encode` helper percent-encodes spaces/`|` for shields.io's static-badge URL format.

## Verification

- New test: `badge_encode_percent_encodes_spaces_and_pipes`.
- **Live**: ran a real build with `--summary`, once with no `GITHUB_REPOSITORY` set (Latest Release/Downloads correctly omitted, Suites/Architectures still shown) and once with `GITHUB_REPOSITORY=latest-debs/eza-debian` set (simulating an Action run against a repo that genuinely exists and has releases, confirmed earlier this session) -- all four badge URLs fetched for real and returned `200` with actual rendered SVG badges, not just correctly-formatted-looking strings.
- Full suite: 110 tests (49 lib + 61 bin) pass; `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, and the full pre-push hook tier all clean.

## Review date

2027-08-21 -- if `lpt` ever gets a mechanism for tracking "where a build's output actually gets published" outside of GitHub Actions (e.g. a `--publish-repo` flag), revisit gating the Latest Release/Downloads badges on `GITHUB_REPOSITORY` alone.
