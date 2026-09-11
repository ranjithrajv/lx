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

**Status:** `lx search --distro` implemented. Coverage-gap reporting is
future work.

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

## Implementation phases

### Phase 1 (done)
- Repology backend (`lib/index/repology.rs`) with API + JSON cache
- `lx index update` refreshes repology data
- `lx index status` shows host distro identity and cache state
- `lx index outdated` lists packages where host distro lags upstream
- `lx search --distro` enriches search results with distro metadata
- `lx index info` shows full distro breakdown per package

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

---

## Data flow

```
repology API (1 req/s, cached locally)
        │
        ▼
~/.cache/lx/repology/projects.json    (bulk: first page, ~200 projects)
~/.cache/lx/repology/per-project/<n>.json  (per-project on demand)
        │
        ▼
lx index  ─── outdated / status / search / info
lx search ─── --distro enrichment (per-hit API lookup + cache)
        │
        ▼
future: lx init / lx go-native / lx upgrade / lx validate
```

The cache is the key enabler: repology's 1 req/s rate limit makes live
API calls impractical for interactive use. The local JSON cache (refreshed
on `lx index update`) lets every command read distro metadata instantly.
