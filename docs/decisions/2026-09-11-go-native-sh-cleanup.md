# `curl | sh` auto-delete in `lx go-native`

**Date:** 2026-09-11
**Status:** Implemented
**Context:** `lx go-native` previously never deleted `curl | sh` orphans —
it printed manual `rm` commands for the user to run. This was intentional
(orphans have no package database, so deletion is risky). But for users who
want a one-shot migration, the manual step is friction. The solution: an
opt-in `--cleanup-sh` flag with a side-manifest for audit.

## What changed

New `--cleanup-sh` flag on `lx go-native`:

```
lx go-native --yes --cleanup-sh
```

When `--cleanup-sh` is passed:
1. After the native install succeeds, `curl | sh` orphan binaries are
   automatically removed (`rm` the binary, clean up empty parent dirs under
   `/opt`).
2. Each removal is recorded in a side-manifest
   (`~/.local/share/lx/sh_orphans.json`) for audit — what was removed,
   which native package replaced it, when.

Without `--cleanup-sh`, behavior is unchanged (manual cleanup commands
printed).

## Side-manifest

`~/.local/share/lx/sh_orphans.json` tracks orphan removals independent of
dpkg:

```json
{
  "removed": [
    {
      "path": "/usr/local/bin/uv",
      "native_package": "uv",
      "removed_at": "2026-09-11T12:00:00Z"
    }
  ]
}
```

This keeps a record even though the orphan was never in any package
database.

## Files changed

- `lib/go_native.rs` — added `cleanup_sh` flag, `remove_sh_orphan()`,
  `record_sh_cleanup()`, `ShOrphanManifest`, `ShOrphanRecord`.
