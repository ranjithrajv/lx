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
        ├─→ lx build              → auto-fill depends: (source: done, binary: next)
        ├─→ lx validate           → pre-flight dep correctness gate
        ├─→ lx convert            → fill missing deps (rpm/arch)
        ├─→ lx show               → declared vs actual dep comparison
        └─→ lx index install      → pre-flight missing-lib warning
```

Each command stops operating in isolation. They all consume the same
cross-distro truth.

---

## 1. `lx index outdated` → drives `lx build` / recipe contributions

**Status:** `lx index outdated` implemented. Integration with `lx build` /
`lx contribute` is future work.

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
- `lx build` could accept `--from-outdated` to build the top N gap-fillers
  without the user manually looking up each forge URL.
- A future `lx index gaps` (packages tracked by repology in 50+ repos but
  missing from the recipe index entirely) would be the highest-value
  contribution target — popular software nobody has packaged yet.

---

## 2. `lx index outdated` → feeds `lx go-native`

**Status:** `lx go-native` exists. `--from-outdated` integration is future work.

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

## 3. `lx index info <pkg>` → jump-starts `lx init`

**Status:** `lx index info` shows repology metadata. `--from` integration
with `lx init` is future work.

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

## 4. `lx search --distro` → validates recipe-index coverage

**Status:** `lx search --distro` and `lx index coverage` implemented.

Without repology, `lx search` only knows about packages in the latest-debs
org. With it, you see the full picture:

```
$ lx search --distro uv
  uv  ... [latest-debs] — host: 0.12.13 (newest) [389/408 repos outdated]
```

This tells you three things at a glance:

- **Is the latest-debs prebuilt needed?** Yes if the host is outdated, no
  if the host is current.
- **How popular is this package?** 408 repos track it — high value to
  package.
- **Is the recipe index complete?** If repology shows a package
  everywhere but the recipe index doesn't have it, that's a gap.

**Dogfooding path:**

- A future `lx index coverage` command compares repology's project set
  against the recipe index. Output: "repology tracks 12,847 projects with
  50+ repos; the recipe index covers 142. Top 50 by repo count not yet
  packaged: ..."
- This turns the index into a **coverage dashboard** — you can see at a
  glance how much of the distro gap LX fills, and where to focus
  contribution effort.

---

## 5. Repology + `lx upgrade` → cross-distro freshness-aware upgrades

**Status:** `lx upgrade` exists for LX-managed packages. `--all` distro
awareness is future work.

`lx upgrade` currently upgrades LX-managed packages against their forge
releases. With repology, it can also flag distro-managed packages that are
behind:

```
$ lx upgrade --all
  lx-managed:
    eza 0.18.0 → 0.20.0  (rebuild from recipe)
  distro-managed but outdated (use `lx install` to take over):
    neovim 0.9.5 → 0.11.0
    ripgrep 14.1.0 → 15.2.0
```

**Dogfooding path:**

- `lx upgrade --all` becomes a system-wide freshness check, not just an
  LX-package check.
- The distro-outdated list is a **migration target** — packages LX could
  manage on the user's behalf. Each entry links to `lx install` or
  `lx go-native` to take over management.
- Over time, the ratio of "lx-managed" to "distro-managed but outdated"
  becomes a health metric for how much of the system LX is keeping
  current.

---

## 6. Repology → `lx validate` pre-flight for recipes

**Status:** `lx validate` exists. Gap-impact warnings are future work.

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
- A future CI check on the recipe index could cross-reference repology:
  "this recipe fills a gap affecting 89 repos — mark as high priority."
- Recipes that fill large gaps could be auto-surfaced in
  `lx index contribute` as suggested starting points.

---

## 7. `lx scan-deps` / `elfdeps` → cross-command dependency correctness

**Status:** `lx scan-deps` implemented. `elfdeps` + `pkg_owner` (cross-platform:
dpkg/rpm/pacman) + dynamic libc detection. Currently only dogfooded by `lx build`
source mode (`sourcebuild::compute_depends`).

The ELF dependency scanner is the **ground truth** for what a binary actually
needs at runtime. Every other command that touches `depends:` should converge
on it.

```
elfdeps (DT_NEEDED parsing, no ldd/objdump)
    │
    ├─→ lx build source mode     → auto-fill depends: (done)
    ├─→ lx build binary repack   → auto-fill + advisory warn
    ├─→ lx validate              → pre-flight dep correctness gate
    ├─→ lx convert               → fill missing deps (rpm/arch get empty deps today)
    ├─→ lx show                  → "ELF needs" section vs dpkg-recorded deps
    └─→ lx index install         → pre-flight: warn on missing host libs
```

### 7a. `lx build` binary repack → auto-fill + advisory warn

**What happens today:** Binary repacks use `cfg.depends` from `package.yaml`
as-is. If the user leaves `depends:` empty, the package ships with no runtime
dependencies — broken install guaranteed.

**Dogfooding path:**

- After extraction (build.rs ~line 1770), scan `binary_dir` ELF files via
  `elfdeps::needed_libraries()`.
- If `cfg.depends` is empty, auto-populate from non-essential sonames resolved
  via `pkg_owner` — same logic source mode already uses.
- If `cfg.depends` is non-empty, advisory-warn when scanned sonames are not
  covered. Fail-closed with `--strict`.
- Insertion point: before `plugin.build()`, inside `build_one`.

### 7b. `lx validate` → pre-flight dependency correctness gate

**What happens today:** `validate` checks that assets exist and patterns
resolve. It never inspects `depends:` — a recipe with wrong/empty deps
validates clean.

**Dogfooding path:**

- After asset resolution (validate.rs ~line 120), download each arch asset,
  extract, scan ELF files.
- Compare non-essential sonames against `cfg.depends` via
  `declared_package_names()`.
- Output:
  ```
  ✓ recipe resolves against eza release v0.20.0
  ⚠ depends: declares [libgit2.so.15] but binary needs [libssh2.so.1]
    → add libssh2-1 to depends:
  ```
- New flag: `--check-deps` (opt-in initially, default later).

### 7c. `lx convert` → fill missing deps for rpm/arch

**What happens today:** `convert.rs` carries `depends` verbatim from the
source package. For RPM (line 316) and Arch (line 377) conversions, `depends`
is always **empty** — converted packages ship with zero runtime dependencies.

**Dogfooding path:**

- After `extract_install_tree` (convert.rs ~line 122), scan ELF files in the
  install tree.
- Auto-populate `depends` when the source left it empty (rpm/arch path).
- Cross-verify when the source provided deps (deb→rpm where naming
  conventions differ).
- Insertion point: before `build_target`, populate the `PackageConfig.depends`
  field.

### 7d. `lx show` → "ELF needs" section

**What happens today:** `show` displays the dpkg-recorded `Depends:` field
only. No comparison against what the binary actually needs.

**Dogfooding path:**

- After `dpkg_depends` display (show.rs ~line 49), read installed ELF files
  via `dpkg -L <pkg>`, scan with `elfdeps::needed_libraries()`.
- Add section:
  ```
  depends:    libgit2.so.15, libssh2.so.1  (dpkg)
  elf_needs:  libgit2.so.15, libssh2.so.1, libcrypto.so.3  (scanned)
  ```
- Discrepancies are highlighted: declared-but-not-needed (bloat) and
  needed-but-not-declared (broken install risk).

### 7e. `lx index install` → pre-flight missing-lib warning

**What happens today:** `lx index install` fetches a prebuilt `.deb` and
installs it. No check that the host has the required shared libraries.

**Dogfooding path:**

- Before or after installing, scan the `.deb`'s ELF files.
- Check each non-essential soname against installed packages via `pkg_owner`.
- Warn: "this package needs `libfoo.so.1` — not found on this host."
- Insertion point: in `lx_community.rs` install path (~line 210-281),
  after extracting the prebuilt.

---

## Implementation phases

### Phase 1 (done)
- Repology backend (`lib/index/repology.rs`) with API + JSON cache
- `lx index update` refreshes repology data (5 pages, ~1000 projects)
- `lx index status` shows host distro identity and cache state
- `lx index outdated` lists packages where host distro lags upstream
- `lx search --distro` enriches search results with distro metadata
- `lx index info` shows full distro breakdown per package
- `lx index search --json` and `lx index info --json` for machine-readable output
- `lx index search --repo <name>` to query a single index
- `lx index search --verbose` shows per-source hit counts for debugging
- `lx index coverage` includes locally installed packages (dpkg/rpm/pacman) in the covered set
- Repology pagination (5 pages, ~1000 projects) for more representative coverage
- AUR null-field handling (no parse warnings on packages with null maintainer/description)

### Phase 2 (done)
- `lx index coverage` — compares repology's project set against the
  recipe index + latest-debs org, reports coverage % and the top
  gap-fillers by repo count. Flags: `--min-repos` (popularity
  threshold), `--limit` (gap list length), `--gaps-only` (suppress
  the totals breakdown).

### Phase 3 (next)
- `lx init --from repology <name>` — scaffold recipe from repology
  metadata
- `lx index contribute --from-outdated` — scaffold a recipe for a
  detected gap-filler

### Phase 4 (future)
- `lx go-native --from-outdated` — migrate stale distro packages
- `lx upgrade --all` — system-wide freshness check with distro awareness
- `lx validate` gap-impact warnings
- Recipe-index CI cross-referenced against repology for priority scoring

### Phase 5 (scan-deps dogfooding)
- `lx build` binary repack: auto-fill empty `depends:` from ELF scanning;
  advisory-warn on mismatch with declared deps
- `lx validate --check-deps`: pre-flight gate comparing declared `depends:`
  against actual ELF `DT_NEEDED` sonames
- `lx convert`: fill missing deps for rpm/arch conversions (currently empty)
- `lx show`: add "ELF needs" section showing scanned vs dpkg-recorded deps
- `lx index install`: pre-flight warn when host lacks required shared libs

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
lx index  ─── outdated / status / search / info / coverage
lx search ─── --distro enrichment (per-hit API lookup + cache)
        │
        ▼
future: lx init / lx go-native / lx upgrade / lx validate

────────────────────────────────────────────────────────────────

elfdeps (DT_NEEDED parsing, natively via `object` crate)
pkg_owner (dpkg / rpm / pacman, auto-detected)
detect_libc_packages (dynamic libc detection, OnceLock-cached)
        │
        ▼
lx build source mode  ─── auto-fill depends: (done)
lx build binary repack ── auto-fill + advisory warn
lx validate            ── pre-flight dep correctness gate
lx convert             ── fill missing deps (rpm/arch)
lx show                ── "ELF needs" vs dpkg-recorded deps
lx index install       ── pre-flight: warn on missing host libs
```

The cache is the key enabler for repology: repology's 1 req/s rate limit
makes live API calls impractical for interactive use. The local JSON cache
(refreshed on `lx index update`) lets every command read distro metadata
instantly.

For scan-deps, the key enabler is `OnceLock`-cached `detect_libc_packages()`
— the package-manager query runs once per process, then every
`is_essential_libc_soname()` check is a hash-set lookup.
