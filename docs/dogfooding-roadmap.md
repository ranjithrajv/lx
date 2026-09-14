# Dogfooding Roadmap

The LX index now consumes [repology](https://repology.org/) — cross-distro
package metadata for 200+ repositories. This turns the index into a **brain**
that can drive every other LX command with real data about where the host
distro stands relative to upstream and the rest of the ecosystem.

This document maps how repology data feeds back into existing and future LX
features — the dogfooding loop.

---

## The core loop

```
repology index (what ships where, what's outdated)
        │
        ├─→ lx index outdated     → backlog for lx build / lx contribute
        ├─→ lx search --distro    → coverage dashboard (recipe gaps)
        ├─→ lx go-native          → migrate stale distro packages
        ├─→ lx init --from        → prefilled recipe scaffolds
        ├─→ lx upgrade --all      → system-wide freshness + migration targets
        └─→ lx validate            → gap-impact warnings on recipes

elfdeps / pkg_owner (what binaries actually need at runtime)
        │
        ├─→ lx build              → auto-fill depends: (source + binary)
        ├─→ lx validate           → pre-flight dep correctness gate
        ├─→ lx convert            → fill missing deps (rpm/arch)
        ├─→ lx show               → declared vs actual dep comparison
        └─→ lx index install      → pre-flight missing-lib warning
```

Each command stops operating in isolation. They all consume the same
cross-distro truth. Which edges already ship and which remain is tracked by
[priority](#priority-queue) below and summarized under
[Completed](#completed).

---

## Priority queue

Remaining work, highest priority first. Completed work is summarized under
[Completed](#completed).

| Priority | Task | Status | Detail |
|---|---|---|---|
| **P1** | `lx index install` pre-flight missing-lib warning | todo | §1 |
| **P2** | `lx init --from repology <name>` — scaffold from metadata | todo | §2 |
| **P2** | `lx index contribute --from-outdated` / `lx build --from-outdated` | todo | §3 |
| **P3** | `lx go-native --from-outdated` — migrate stale distro packages | todo | §4 |
| **P3** | `lx validate` — repology gap-impact warnings | todo | §5 |
| **P4** | Repology-scored recipe-index CI | todo (infra) | §6 |

Priority rationale:

- **P1 — finish the correctness loop.** Four of the five `elfdeps`
  dogfooding targets now ship; `lx index install` is the last one. It guards
  the one remaining path that can hand a user a package whose shared
  libraries aren't present.
- **P2 — cheapest repology → recipe wins.** The metadata is already cached,
  so scaffolding (`lx init --from repology`) and the outdated backlog
  (`--from-outdated`) are small changes that directly grow the recipe index.
- **P3 — freshness and migration.** Bigger, user-facing surfaces that build
  on the now-shipped `lx upgrade --all`.
- **P4 — ecosystem/infra.** Recipe-index CI scoring depends on the recipe
  index and its CI, so it lands last.

---

## 1. `lx index install` → pre-flight missing-lib warning

**Priority: P1.** The last open item in the ELF dependency loop.

**What happens today:** `lx index install` fetches a prebuilt `.deb` (or the
host's native format) and installs it. Nothing checks that the host actually
has the shared libraries the package needs — a package built for a different
image can install "successfully" and then fail at first run.

**Dogfooding path:**

- Before or after installing, scan the package's ELF files with
  `elfdeps::needed_libraries()`.
- Check each non-essential soname against installed packages via
  `pkg_owner` (dpkg/rpm/pacman, auto-detected).
- Warn: "this package needs `libfoo.so.1` — not found on this host."
- Insertion point: the `lx-community` install path, after extracting the
  prebuilt.

---

## 2. `lx init --from repology <name>` → recipe scaffold

**Priority: P2.** Cheapest repology → recipe win; metadata is already cached.

**Status:** `lx index info` shows repology metadata. `--from repology`
integration with `lx init` is future work (`lx init --from-aur` already
exists as the first such importer).

`lx init` interactively scaffolds a recipe. Repology already knows the
project's summary, categories, licenses, and which distros carry it:

```
$ lx index info eza
  distros: 210 repos
  newest: 0.20.0
  licenses: MIT
```

**Dogfooding path:**

- `lx init --from repology <name>` prefills the recipe scaffold from
  repology metadata: description, license, category. Less manual entry,
  fewer mistakes.
- The forge URL can be auto-detected from repology's `srcname` field or by
  matching against known forge patterns (GitHub/GitLab/Gitea).
- The version field can default to the newest known release, so the
  generated recipe starts from current upstream, not a stale guess.

---

## 3. `lx index outdated` → `lx index contribute` / `lx build --from-outdated`

**Priority: P2.** Turns the detected gap into a contribution/build backlog.

**Status:** `lx index outdated` is implemented; the `lx contribute` /
`--from-outdated` wiring is not (there is no `lx index contribute` command
or `--from-outdated` flag yet). `lx index gaps` is superseded by the shipped
`lx index coverage` (see [Completed](#completed)).

The outdated list is a ready-made backlog for the recipe index:

```
$ lx index outdated --limit 20
  eza        host: 0.18.0  →  newest: 0.20.0
  ripgrep    host: 14.1.0  →  newest: 15.2.0
  neovim     host: 0.9.5   →  newest: 0.11.0
```

Each row is a package where the distro version is stale (repology confirms
it), LX can build a newer one from the forge release, and a community recipe
would fill a real, detected gap.

**Dogfooding path:**

- `lx index contribute <name>` currently scaffolds a blank recipe. With the
  outdated list as input, it becomes: "package the 47 projects the index
  flagged as outdated on this host." The index becomes the **prioritization
  engine** for community contributions.
- `lx build --from-outdated` builds the top N gap-fillers without the user
  manually looking up each forge URL.
- (Covered today by `lx index coverage`.) The highest-value contribution
  target is popular software nobody has packaged yet — repology projects in
  50+ repos but absent from the recipe index.

---

## 4. `lx go-native --from-outdated` → migrate stale distro packages

**Priority: P3.** Bigger addressable set, builds on the shipped
`lx upgrade --all`.

**Status:** `lx go-native` exists. `--from-outdated` integration is future
work.

`lx go-native` finds snap/flatpak/nix/`curl | sh` installs and plans
native-package migrations. It currently starts from what's already on the
machine. The repology data extends it to distro packages that are behind
upstream:

```
$ lx go-native --from-outdated
  plan: replace distro neovim 0.9.5 with lx-built neovim 0.11.0
  plan: replace distro ripgrep 14.1.0 with lx-built ripgrep 15.2.0
```

Now `go-native` migrates **distro packages that are behind upstream**, not
just non-native formats. Same end state (native `.deb`/`.rpm`/Arch),
bigger addressable set.

**Dogfooding path:**

- The outdated list is the migration target list. Each entry has a known
  newest version — `go-native` can plan the replacement without a separate
  lookup.
- After migration, the package moves from "distro-managed" to
  "lx-managed," and `lx upgrade` takes over freshness tracking.

---

## 5. `lx validate` → repology gap-impact warnings

**Priority: P3.** Advisory context for authors and reviewers.

**Status:** `lx validate` exists (including the shipped `--check-deps` ELF
gate). The repology gap-impact warnings are future work.

`lx validate` checks a `package.yaml` resolves against a real release. With
repology, it can warn about the gap the recipe fills:

```
$ lx validate ./neovim.yaml
  ✓ recipe resolves against neovim/neovim release v0.11.0
  ⚠ newest distro version is 0.9.5 (Debian 12) — this recipe fills a known gap
  ⚠ 89 repos still outdated — high impact if published
```

**Dogfooding path:**

- Validation carries **context about the gap the recipe fills** — useful
  for the author (prioritization) and a future recipe reviewer (impact
  assessment).
- Recipes that fill large gaps could be auto-surfaced in
  `lx index contribute` as suggested starting points.

---

## 6. Repology-scored recipe-index CI

**Priority: P4.** Infra; depends on the recipe index and its CI.

**Status:** future work.

A CI check on the recipe index could cross-reference repology: "this recipe
fills a gap affecting 89 repos — mark as high priority." Scoring contributions
by the size of the gap they close makes review and merge order data-driven
rather than first-come-first-served.

---

## Completed

Already dogfooded. Kept here so the loop's remaining work is unambiguous.

### ELF dependency loop (`elfdeps` / `pkg_owner`)

The ELF dependency scanner is the **ground truth** for what a binary actually
needs at runtime, and every command that touches `depends:` has now converged
on it. `elfdeps` parses `DT_NEEDED` natively (no `ldd`/`objdump`), `pkg_owner`
resolves sonames via dpkg/rpm/pacman (auto-detected), and
`detect_libc_packages()` is `OnceLock`-cached so the package-manager query
runs once per process.

- ✅ **`lx build` source mode** — auto-fill `depends:` from staged ELFs.
- ✅ **`lx build` binary repack** — auto-fill an empty `depends:` and
  advisory-warn when scanned sonames aren't covered by a non-empty
  `depends:`.
- ✅ **`lx validate --check-deps`** — pre-flight gate comparing declared
  `depends:` against actual ELF sonames.
- ✅ **`lx convert`** — fills missing deps for rpm/arch conversions (which
  previously shipped empty) and cross-verifies source-provided deps.
- ✅ **`lx show`** — "ELF needs" section alongside the dpkg-recorded
  `Depends:`.

### Repology index brain

- ✅ Repology backend with API + JSON cache.
- ✅ `lx index update` (5 pages, ~1000 projects), `lx index status`,
  `lx index outdated`, `lx index info`.
- ✅ `lx index search --json` / `--repo <name>` / `--verbose`.
- ✅ `lx search --distro` — per-hit distro metadata.
- ✅ `lx index coverage` — repology vs. recipe index + latest-debs org, with
  `--min-repos` / `--limit` / `--gaps-only`, including locally installed
  packages in the covered set.
- ✅ `lx upgrade --all` + `--auto-migrate` — system-wide freshness check and
  takeover of distro packages flagged outdated.
- ✅ AUR null-field handling (no parse warnings on null maintainer/description).

---

## Data flow

```
repology API (1 req/s, cached locally)
        │
        ▼
~/.cache/lx/repology/projects.json    (bulk: 5 pages, ~1000 projects)
~/.cache/lx/repology/per-project/<n>.json  (per-project on demand)
        │
        ▼
lx index  ─── outdated / status / search / info / coverage        ✅
lx search ─── --distro enrichment (per-hit API lookup + cache)    ✅
lx upgrade ── --all / --auto-migrate                              ✅
        │
        ▼
todo: lx init --from repology / lx index contribute --from-outdated
      lx go-native --from-outdated / lx validate gap warnings

────────────────────────────────────────────────────────────────

elfdeps (DT_NEEDED parsing, natively via `object` crate)
pkg_owner (dpkg / rpm / pacman, auto-detected)
detect_libc_packages (dynamic libc detection, OnceLock-cached)
        │
        ▼
lx build source mode  ─── auto-fill depends:                     ✅
lx build binary repack ── auto-fill + advisory warn              ✅
lx validate            ── --check-deps gate                      ✅
lx convert             ── fill missing deps (rpm/arch)           ✅
lx show                ── "ELF needs" vs dpkg-recorded deps      ✅
lx index install       ── pre-flight: warn on missing host libs  ⏳ P1
```

The cache is the key enabler for repology: repology's 1 req/s rate limit
makes live API calls impractical for interactive use. The local JSON cache
(refreshed on `lx index update`) lets every command read distro metadata
instantly.

For scan-deps, the key enabler is `OnceLock`-cached `detect_libc_packages()`
— the package-manager query runs once per process, then every
`is_essential_libc_soname()` check is a hash-set lookup.
