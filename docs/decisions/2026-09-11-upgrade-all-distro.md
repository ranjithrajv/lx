# System-wide `lx upgrade --all` (distro-aware)

**Date:** 2026-09-11
**Status:** Implemented
**Context:** `lx upgrade` previously only upgraded LX-managed packages. There
was no single command showing "everything on my system that's out of date" —
you needed `apt list --upgradable` for distro packages + `lx upgrade` for
lx-managed ones. The repology integration (Phase 1-2 of the dogfooding
roadmap) now provides the data to unify this.

## What changed

New `--all` flag on `lx upgrade`:

```
lx upgrade --all
```

When `--all` is passed, after upgrading LX-managed packages, the command
checks repology for distro-managed packages that are behind upstream:

```
lx-managed packages:
  ↑ eza: 0.18.0 -> 0.20.0
  1 upgraded, 0 failed, 0 up to date

system-wide freshness check (distro packages behind upstream):
  3 package(s) where distro lags upstream:

    neovim               host: 0.9.5        → newest: 0.11.0
    ripgrep              host: 14.1.0       → newest: 15.2.0
    bat                  host: 0.24.0       → newest: 0.25.0

  to take over management of a package, run `lx install <name>`
```

This turns `lx upgrade --all` into a system-wide freshness check. Distro
packages flagged as outdated become migration targets — `lx install` can
take them over, after which `lx upgrade` tracks them.

## Implementation

- `lib/upgrade.rs` — added `all` flag, `check_distro_outdated()` function.
- `lib/index/repology.rs` — added `outdated_packages()` method returning
  `Vec<IndexHit>` of packages where the host distro lags upstream.

## Files changed

- `lib/upgrade.rs` — `all` flag + `check_distro_outdated()`.
- `lib/index/repology.rs` — `outdated_packages()` method.
