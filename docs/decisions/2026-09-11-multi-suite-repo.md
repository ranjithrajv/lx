# Multi-suite repository layout for `lx repo`

**Date:** 2026-09-11
**Status:** Implemented
**Context:** `lx repo` previously produced a single-suite layout (one
`Packages`/`Release` in a flat directory). Real Debian/Ubuntu archives use a
multi-suite layout (`dists/bookworm/`, `dists/trixie/`, etc.) with a top-level
`Release` listing all suites. This is what `reprepro`/`aptly` produce and what
`apt` expects when pointed at a real archive.

## What changed

New `--multi-suite` flag on `lx repo`:

```
lx repo ./my-debs/ --multi-suite
```

When `--multi-suite` is passed:
1. **Discover suites** by scanning `dists/<suite>/` subdirectories. Each
   subdirectory's `.deb` files become that suite's package index.
2. **Per-suite indices**: each `dists/<suite>/` gets its own `Packages`,
   `Packages.gz`, and `Release` (with `Suite: <suite>`, `Codename: <suite>`).
3. **Top-level Release**: written at the repo root, listing all suites in
   `Suites:` and hashing every per-suite index file.

### Fallback

If no `dists/` subdirectory exists but `.deb` files are present in the root,
`--multi-suite` falls back to single-suite behavior (one `Release` with
`Suite: stable`). This makes it safe to use unconditionally.

### New flags

- `--components` (default: `main`) — comma-separated component list in Release.
- `--origin` (default: `latest-debs`) — Origin/Label in Release.

## Files changed

- `lib/repo.rs` — added `multi_suite`, `components`, `origin` fields to
  `RepoArgs`; added `run_multi_suite()`, `discover_suites()`,
  `build_packages_index_with_base()`, `sign_release()` helpers.
- `tests/repo.rs` — added multi-suite + fallback tests.
