# Feature Parity: lx vs deb-get

A living comparison of [lx](../../README.md) against
[wimpysworld/deb-get](https://github.com/wimpysworld/deb-get), the bash
client that installs third-party `.deb`s (apt repos, PPAs, GitHub/GitLab
releases, and direct downloads) on Debian/Ubuntu. Used to track what lx
has adopted, what it deliberately rejects, and what remains.

> Canonical capability details live in the
> [commands](../reference/commands.md) reference and the
> [plugin catalog](../architecture/plugin-catalog.md); this page tracks
> parity, and the tables here are a snapshot.

## TL;DR

| | deb-get | lx |
|---|---|---|
| **Language** | Bash | Rust |
| **Formats** | `.deb` only | `.deb`, `.rpm`, Arch, `.apk`, `.ipk`, MSIX, macOS |
| **Catalog** | Curated `01-main` + external `.repo` manifests | Same catalog, read in place |
| **Definitions** | `source`d as bash (runs code) | Parsed statically (never executed) |
| **Integrity** | HTTPS-only | Fail-closed checksums/sidecars, `--allow-unverified` opt-out |
| **Entry point** | `deb-get <verb>` | lx's own commands (`lx install`/`lx list --catalog`/`lx show`/`lx index`) |

lx installs from deb-get's catalog via a dedicated **`debget` read index**;
the deb-get verbs are folded into lx's existing commands rather than a
separate `lx deb-get` namespace. On Debian/Ubuntu hosts the `debget` source
is **enabled by default** in `indexes.yaml`, so `lx install`/`lx search
--index` consult the catalog automatically; on other hosts it can be added
with `lx index add debget --kind debget` (its `.deb`s would not install
there).

---

## 1. Command mapping

| deb-get | lx | Notes |
|---|---|---|
| `update [--repos-only] [--quiet]` | `lx index update` | refreshes the catalog (built-in `01-main` + external `.repo` repos); lx's refresh never touches apt, so it is already `--repos-only` |
| `upgrade [--dg-only]` | `lx upgrade` | lx only ever upgrades lx-managed packages, i.e. `--dg-only` is the default |
| `install <pkgs>` | `lx install <pkgs>` | native-first, then enabled indexes (incl. `debget`), then the org; `--source debget` forces the catalog |
| `reinstall <pkg>` | `lx install --reinstall <pkg>` | also `lx reinstall` (hidden shim) |
| `remove [--remove-repo] <pkgs>` | `lx remove <pkg>` | `--remove-repo` has no lx equivalent (lx does not tear down `/etc/apt/sources.list.d`) |
| `purge [--remove-repo] <pkgs>` | `lx remove --purge <pkg>` | |
| `show <pkgs>` | `lx show <pkg>` | falls back to the catalog definition for an available-but-uninstalled package |
| `search [--include-unsupported] <re>` | `lx search --index <re>` | unsupported catalog definitions are annotated inline rather than filtered by a flag |
| `list [--include-unsupported] [--raw\|--installed\|--not-installed]` | `lx list --catalog [--format table\|raw] [--installed\|--not-installed] [--include-unsupported]` | lists the **catalog**; lx's `lx list` (no `--catalog`) lists lx-managed packages |
| `prettylist [<repo>]` | `lx list --catalog --format pretty [--repo R]` | markdown table, same layout/icons |
| `csvlist [<repo>]` | `lx list --catalog --format csv [--repo R]` | same six columns |
| `cache` | `lx index clean --dry-run` | reports cached index data |
| `clean` | `lx index clean` | removes the refreshed catalog checkout/git cache |
| `fix-installed [--old-apps]` | `lx list --verify [--prune]` | reports manifest entries the host no longer has; `--prune` drops them (`--old-apps` has no equivalent) |
| `help`, `version` | `lx --help`, `lx --version` | |

## 2. Catalog & definition compatibility

deb-get's on-disk layout is read directly:

```text
/etc/deb-get/<NN-name>.repo     # first line = https base URL (external repos)
/etc/deb-get/<NN-name>.d/<pkg>  # one package definition per file
/etc/deb-get/99-local.d/<pkg>   # user overrides
```

* **Precedence** follows deb-get: the two-digit repo prefix orders repos and
  the highest wins for a duplicate name, so `99-local` overrides `01-main`.
* **`lx index update`** refreshes the built-in `01-main` from
  `wimpysworld/deb-get` and every external `<name>.repo` manifest, into
  `<name>.d/`. `00-builtin.repo` and `99-local.repo` are never touched.
  GitHub-hosted repos are fetched as a tarball; other repos fetch each
  relative package path from `<base>/packages/`. Absolute URLs in a manifest
  body are refused (matching deb-get's hardening).
* **Catalog root** is `$LX_DEBGET_DIR`, else `/etc/deb-get` when it exists
  (an actual deb-get install), else `~/.local/share/lx/debget` so a
  non-root user can still refresh a catalog. Writes to a root-owned
  `/etc/deb-get` fall back to `sudo install`, as deb-get does.

### Definitions are parsed, never executed

deb-get `source`s each definition as bash. lx deliberately never executes
upstream recipes (the same reason an AUR `PKGBUILD` becomes comments), so
the `debget` backend parses the **declarative subset**:

* `KEY=value` / `KEY="value"` / `KEY='value'` assignments;
* `get_github_releases "owner/repo"`, `get_gitlab_releases …`, and
  `get_website` as **markers** — lx resolves the release itself through its
  own forge clients;
* `post_download` hooks, computed `URL=$(…)` values, and arbitrary control
  flow are **reported as unsupported**, not guessed at or run.

## 3. Package methods

| METHOD | deb-get | lx | Notes |
|---|---|---|---|
| `apt` (`APT_REPO_URL` + key) | ✅ | ✅ | writes a keyring + `sources.list.d` entry via `sudo`, then `apt-get`; Debian/Ubuntu only |
| `ppa` | ✅ | ✅ | `add-apt-repository` then `apt-get`; Ubuntu only |
| `github` | ✅ | ✅ | latest release, arch-matched `.deb` |
| `gitlab` | ✅ | ✅ | same |
| `website` | ✅ | ⚠️ best-effort | scrapes `.deb` links from the page; definitions that compute a URL from a cached HTML index are unsupported |
| `direct` | ✅ | ✅ | literal `URL=` to a `.deb` |
| `snapd` | ✅ | ❌ | lx migrates snaps to native (`lx go-native`) instead of installing snapd |

`ARCHS_SUPPORTED`/`CODENAMES_SUPPORTED` are honoured as a host gate in
`lx list --catalog` (and the `--include-unsupported` escape hatch ignores it).

## 4. Deliberate differences

1. **No bash execution.** Definitions are data, not code. This is the same
   principle lx applies to AUR `PKGBUILD`s and `nfpm.yaml`.
2. **Fail-closed integrity.** deb-get trusts HTTPS transport. lx verifies
   provider/definition checksums and `.sha256` sidecars where available and
   requires `--allow-unverified` otherwise — the catalog did not lower lx's
   default.
3. **Multi-format, not deb-only.** On rpm/Arch hosts lx builds and installs
   those formats; the deb-get catalog is only enabled by default where
   `.deb`s apply.
4. **`lx list` keeps its meaning.** deb-get's `list` lists the catalog; lx's
   `list` lists what lx installed. The catalog view is the `--catalog` flag
   on the same command, not a separate namespace.

## 5. Remaining gaps

* `remove`/`purge --remove-repo` — lx does not tear down an apt source it
  added (deb-get does).
* `--old-apps` (rename history) for `fix-installed`.
* `snapd` method (intentionally not planned; see `lx go-native`).
* `search --include-unsupported` as a filter flag (lx annotates instead).
* `update --quiet` (lx's refresh is already non-interactive).

## 6. Using it

```sh
lx index update                                  # fetch/refresh the catalog
lx index add debget --kind debget                # opt in (non-Debian hosts)
lx list --catalog                                # catalog packages
lx list --catalog --installed                    # installed ones
lx list --catalog --format pretty --repo 01-main
lx list --catalog --format csv                   # deb-get csvlist
lx show google-chrome-stable                     # catalog definition
lx install google-chrome-stable                  # installs via the catalog
lx search --index docker                         # search the catalog too
lx list --verify                                 # fix-installed (audit)
lx index clean                                   # drop cached index data
```

`DEBGET_TOKEN` is accepted as an alias for `GITHUB_TOKEN` when resolving
GitHub releases, mirroring deb-get.
