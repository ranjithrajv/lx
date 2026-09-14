# Package indexes (`lx index`)

Part of the [lx docs](../README.md).

A unified index manager: one command, many upstream indexes. `lx index` fans
out across every *enabled* source — the
[LX community index](https://github.com/ranjithrajv/lx-index) (recipes +
prebuilt binaries), the [AUR](https://aur.archlinux.org/) (builds PKGBUILDs
into native packages), and any custom index you register (a git repository
of recipes in the same format as the LX community index). New sources are
**plugins**: implement the `PackageIndex` read role and add one line to the
registry — no fork, no recompile of core.

```sh
lx index search eza           # full-text across ALL enabled indexes
lx index install eza          # prebuilt first (LX index), build if no match
lx index info eza             # details from every index that has it
lx index update               # pull latest recipes + prebuilts

lx index list                 # show configured indexes
lx index add copr <url>       # register a custom index
lx index remove copr          # drop it
```

The registry lives in `~/.config/lx/indexes.yaml` and ships with the LX
community index, AUR, and the repology metadata source enabled by default.
The LX community index caches to `~/.cache/lx/index/` (a shallow git clone,
auto-refreshed; works offline on a stale cache with a warning); a custom
index caches under `~/.cache/lx/index/<name>/`. The repology
source caches to `~/.cache/lx/repology/` (JSON files refreshed via the
repology API on `lx index update`). `lx search` merges index results with the
`latest-debs` org and embedded templates; `--local` keeps it offline-only.

## Distro metadata (repology)

The repology source tracks what version of each project ships in 200+ distro
repositories. It is metadata-only — it cannot install anything, but it enriches
search results with cross-distro context:

```sh
lx index update               # refresh the repology cache from the API
lx search --distro eza        # show distro versions alongside org results
lx index status               # show host distro's repology identity
lx index outdated             # list packages where host distro lags upstream
lx index coverage             # find popular packages LX does not yet cover
```

With `--distro`, search results show the host distro's version, the newest
known version, and how many repos carry the package:

```
eza    A modern, maintained replacement for ls [latest-debs] — host: 0.18.0 (newest 0.20.0) [37 repos]
```

`lx index outdated` produces a gap list — packages where your distro ships an
older version than upstream — which is exactly the set the LX community recipe
index can fill.

`lx index coverage` compares repology's project set against the recipe index
and latest-debs org, then reports the gap — popular projects (by repo count)
that LX does not yet package. This validates `lx search` result quality: if a
project is everywhere in repology but missing from the index, `lx search`
won't find it, and that's a recipe worth contributing.

```
$ lx index coverage --min-repos 10
coverage: repology vs. LX (recipe index + latest-debs org)
  repology projects (≥10 repos): 34
  covered by LX:                 4 (11%)
  gap (not yet packaged):        30

top gap-fillers by repo count (showing 10 of 30):
  uv            408 repos
  python        380 repos
  node          350 repos
  ...
```

## Index enhancements

Beyond the core search/install/info workflow, `lx index` supports several
output modes and query refinements:

**JSON output** (`--json`) — machine-readable results for scripting, CI, and
external tools. Emits the full `IndexHit` struct including repology metadata:

```sh
lx index search --json curl | jq '.[] | select(.source=="repology") | .newest'
lx index info --json eza          # wraps each source's hit with its index name
```

**Source filtering** (`--repo <name>`) — scope a query to a single index by
name (use `lx index list` to see available sources). Errors on unknown names:

```sh
lx index search --repo repology curl     # metadata only
lx index search --repo aur curl          # buildable PKGBUILDs only
lx index search --repo nonexistent x     # → "no enabled index named 'nonexistent'"
```

**Verbose diagnostics** (`--verbose`) — shows how many indexes were searched
and the hit count per source. Useful for debugging stale caches or understanding
why a package isn't found:

```sh
lx index search --verbose no_such_pkg
# searched 3 index(es): lx-community:0, aur:0, repology:0
# no packages matched
```

**Coverage analysis** — `lx index coverage` now includes locally installed
packages (dpkg/rpm/pacman) in its "covered" set, so the gap list only contains
popular projects that are genuinely absent from both the indexes and the host:

```sh
$ lx index coverage --min-repos 100
coverage: repology vs. LX (recipe index + latest-debs org)
  repology projects (≥100 repos): 21
  covered by LX:                 6 (28%)
  gap (not yet packaged):        15
```

**Repology pagination** — `lx index update` now fetches 5 pages (~1000 projects)
from the repology API (up from 1 page / ~200 projects) with rate-limit-aware
delays between requests. The fuller cache makes `coverage` and `--distro`
enrichment more representative.

**AUR robustness** — null fields in the AUR RPC response (e.g. packages with no
maintainer) are handled gracefully instead of producing parse warnings.

The LX index runs `lx build` + `--sbom` on every merged recipe in CI, so
community contributions ship prebuilt binaries without the contributor
running their own release infra.

