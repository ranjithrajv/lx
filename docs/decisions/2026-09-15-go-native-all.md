# `lx go-native --all` and `--dry-run`

**Date:** 2026-09-15
**Status:** Implemented
**Context:** `lx go-native` mapped detected
snap/flatpak/nix/`curl | sh` installs through a curated table. Anything not
in that table was reported under `missingnative` and left alone. That is the
right default for *mapping*, but two things were missing: a machine full of
`curl | sh` tools could not be converged to native packages in one run
without naming each one, and the plan listed mappings without saying what any
of them was worth. The command was also opt-in (`--yes`), which made the
common case (converge this machine) the one you had to know a flag for.

## What changed

New `--dry-run` flag (applying is the default):

```
lx go-native --dry-run            # plan + benefit report; mutates nothing
lx go-native                      # apply
```

`--all` (and an explicit target, which already behaved this way) attempts the
unmapped findings too, using the finding's own command name as the native
package name — a snap/nix id, a flatpak app-id's last segment, or a `curl | sh`
binary's basename. The curated table is still consulted first, so
`snap spotify` becomes `spotify-client`, never the guessed `spotify`.

Best-effort rows are marked in the plan and counted:

```
migration plan (83 detected, 83 mapped):
  [flatpak] app.zen_browser.zen (1.14.11b) → zen   [best-effort: no curated mapping]
  [curl|sh] /usr/local/bin/yq → yq
  ...
  (80 best-effort guess(es) — the native package name is the package's own
   name; verify before applying)
```

Plan-only with `--dry-run` (applying is the default).

## Naming

The command is top-level `lx go-native`. Prefix inference is off at the CLI
root so an unknown name is a usage error rather than resolving to whichever
live command shares its prefix.

## Benefit report

`--dry-run` also answers *why* each switch is worth making, from local state
only (no network): whether the native package is already installed, the space
the redundant non-native copy reclaims, how much of the native package's
dependency set the host already satisfies, and whether the native package
would actually keep the command.

```
  [curl|sh] /usr/local/bin/yq → yq
      state      native package already installed — only the redundant copy is removed
      reclaims   ~11M from the non-native source
      deps       5/5 already satisfied on this host (0/5 new)
      why        native arch package: host-managed, signed repo/pinned checksum, no duplicated runtime

reclaimed when applied: ~60.9 MB across 3 package(s)
```

## Safety

- `--all` alone does **not** delete `curl | sh` orphans; `--cleanup-sh` is
  still required. Source removal keeps the existing guard: the native package
  must actually provide the command, or the non-native copy is kept.
- A guessed native name that doesn't exist simply fails its install and is
  reported; it never causes the source to be removed (only successfully
  installed entries reach the removal pass).
- `--all` cannot be combined with the curated-only default accidentally: it
  is an explicit flag; `--dry-run` prints the plan before anything mutates.

## Install source

While here, a migration's native install no longer forces
`--format <host>` into `lx install`, which had pinned it to the `latest-debs`
org (an explicit `format` disables index preference). Without an explicit
`--format`, each install now resolves the normal way — host repositories
first (native-first), then the enabled indexes, then the org. An explicit
`--format` still pins the target, preserving the cross-distro override.

## Implementation

- `lib/go_native.rs` — `GoNativeArgs::all` / `::dry_run` (`--yes` removed);
  `mapped_from_table()` split out of `map_native()` so the plan can
  distinguish a curated mapping from a guess; `PlanEntry::guessed`;
  `go_native_attempt_probe()` test probe; `native_benefit()` / `deps_shared()`
  / `human_bytes()` for the dry-run report; `install_native_package()` takes
  `Option<&str>` (the explicit `--format`).
- `tests/go_native.rs` — `--all` fallback and curated-wins tests.
- Docs: `docs/reference/commands.md`, `docs/drop-in/replacements.md`.
