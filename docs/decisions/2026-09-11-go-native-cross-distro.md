# `go-native` apply on rpm/Arch hosts

**Date:** 2026-09-11
**Status:** Implemented
**Context:** `lx go-native --yes` was previously deb-host-only. On rpm/Arch
hosts the plan doubled as a shopping list. This limited the command to
dpkg-based distros.

## What changed

`lx go-native --yes` now applies migrations on all hosts:

- **deb hosts**: `lx install` backend (unchanged).
- **rpm hosts**: downloads the `.rpm` from the `latest-debs` org, installs
  with `rpm -Uvh --force`.
- **arch hosts**: downloads the `.pkg.tar.zst` from the `latest-debs` org,
  installs with `pacman -U --noconfirm`.

The `is_native_installed()` check is also format-aware:
- deb: `dpkg -s <pkg>`
- rpm: `rpm -q <pkg>`
- arch: `pacman -Q <pkg>`

## Implementation

- `lib/go_native.rs` — replaced the deb-only block with format-aware
  `install_native_package()` and `is_native_installed()`.
- `lib/debs.rs` — added `find_asset_any()` for cross-format asset lookup.

## Files changed

- `lib/go_native.rs` — `install_native_package()`, `is_native_installed()`.
- `lib/debs.rs` — `find_asset_any()`.
