# Native-first `lx install`: the host repos are asked before any index

**Date:** 2026-09-15
**Status:** Implemented
**Context:** `lx install <pkg>` resolved straight to the enabled package
indexes (AUR/LX community, prebuilt then build-from-recipe) and the
`latest-debs` GitHub org. On an Arch host, `lx install herdr` therefore built
herdr from its AUR PKGBUILD — an entirely reasonable outcome for an
AUR-only package, but the command never first asked what a plain
`pacman -S herdr` would have answered. A package that already lives in the
distro's own repositories should be installed by the distro, not fetched from
a forge release or rebuilt from a recipe.

## What changed

`lx install` probes the host's **own repositories** before the enabled
indexes / `latest-debs` org:

| Host | Probe | Delegate |
|---|---|---|
| Arch | `pacman -Si <pkg>` | `sudo pacman -S --noconfirm <pkg>` |
| deb | `apt-cache policy <pkg>` | `sudo apt-get install -y <pkg>` |
| rpm | `dnf repoquery --latest-limit 1 …` (`yum` fallback) | `sudo dnf install -y <pkg>` |
| apk | `apk search -x <pkg>` | `sudo apk add <pkg>` |

When the probe finds a candidate:

```
firefox 120.0-1 is available from the host repositories
Run `sudo pacman -S --noconfirm firefox`? [y/N]
```

…and the install is done by the host manager.

When the probe finds nothing, lx says so before falling through, so the
source switch is never silent:

```
herdr is not in the host repositories; falling back to the enabled package indexes, then the latest-debs org
```

## The tracking decision: delegate, do not track

A native-first install is **not** written to the lx manifest. Repository
packages are owned by the host manager: `lx list`/`update`/`upgrade`/
`rollback` should not pretend to manage them, and `lx upgrade` must not try to
re-fetch a package the distro tracks. lx only manifests what it built or
fetched itself. (This is the "delegate, don't track" option, chosen over
recording native installs as lx-managed generations.)

## When native-first does *not* run

The probe is skipped whenever the user named a different source of truth, so
it can never hijack an explicit request:

- `--source <index>` — a specific index was asked for.
- `--build` — build from a recipe, by definition not a repo package.
- `--format` / `--arch` / `--distribution` — an org-targeted request.
  (These already disable index preference; native-first shares that gate.)
- `--version` — a pinned version the repo can't be assumed to carry.
- `--download-only` — there is nothing to download from a repo install.
- the package is already **lx-managed** — its recorded source wins, so a
  plain `lx install` never silently switches an lx package to the host
  manager.

## Implementation

- `lib/consumer.rs` — `native_repo_version()` (format-dispatched probe) with
  a pure `parse_native_repo_version()` per host spelling, and
  `install_native()` / `native_install_argv()` for the delegation. `run_sudo`
  became generic over `AsRef<OsStr>` so it serves both `&[&str]` and the
  owned native argv.
- `lib/install.rs` — the native-first step in `run()` and its
  `native_first_applies()` gate (unit-tested).
- Docs: `docs/reference/commands.md`, `docs/reference/indexes.md`,
  `docs/drop-in/replacements.md`.

## Deliberate non-goals

- **Not** a fallback after a failed index lookup: native-first is a *first*
  step, so the host repos win on the plain path rather than only rescuing
  misses.
- **No** native-first in `lx upgrade`; keeping repo packages current is the
  host manager's job (`pacman -Syu`).
