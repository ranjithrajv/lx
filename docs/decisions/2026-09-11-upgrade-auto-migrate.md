# System-wide `lx upgrade --all --auto-migrate`

**Date:** 2026-09-11
**Status:** Implemented
**Context:** `lx upgrade --all` previously only flagged distro packages as
outdated (plan-only). There was no single command that both identified and
fixed system-wide staleness — the `uv` move for "everything on my system that's
out of date."

## What changed

New `--auto-migrate` flag on `lx upgrade` (requires `--all`):

```
lx upgrade --all                  # plan-only: print migration targets
lx upgrade --all --auto-migrate   # apply: install lx-built versions
```

When `--auto-migrate` is passed:
1. After upgrading LX-managed packages, the command checks repology for
   distro-managed packages that are behind upstream.
2. Each flagged package is installed via the `lx install` backend (downloads
   from the `latest-debs` org).
3. The package is now tracked in the manifest for future `lx upgrade`.

## Implementation

- `lib/upgrade.rs` — added `auto_migrate` flag (clap `requires = "all"`),
  `check_distro_outdated()` now takes `&UpgradeArgs` and calls
  `migrate_distro_package()` when `auto_migrate` is set.
- `tests/upgrade_all.rs` — NEW: tests for the new args.

## Files changed

- `lib/upgrade.rs` — `auto_migrate` flag + `migrate_distro_package()`.
- `tests/upgrade_all.rs` — NEW.
