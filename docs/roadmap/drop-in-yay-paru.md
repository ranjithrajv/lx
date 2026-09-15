# Drop-in: yay / paru

Part of the [drop-in roadmap](README.md). **Status:** open (Phase 5).
Depends on the [makepkg executor](drop-in-makepkg.md). Current parity:
[replacements.md §2.9](../architecture/replacements.md).

## The contract

`yay`/`paru` present a pacman-shaped surface and drive the AUR behind it:
locale-independent search and info (`-Ss`, `-Si`), install (`-S`), query
upgrades (`-Qu`), remove (`-Rns`), and a full system upgrade (`-Syu`).
Behind the surface they: resolve an AUR package's `depends`/`makedepends`
recursively (repo and AUR together, `provides`/virtual and version
constraints included), build in dependency order via `makepkg`, record AUR
packages as *foreign* so `pacman -Qm`/`-Qu` see them, show the PKGBUILD
diff before a rebuild, and expose AUR metadata (votes, popularity,
out-of-date, orphan, comments). Common flags: `--aur`, `--repo`,
`--noconfirm`, `--needed`, `--devel`, `--answerclean`, `--diffmenu`,
`--editmenu`, `--rebuild`, `--cleanafter`, `--mflags`.

## Current state

- `lx index` has an AUR read index (search/info/install) plus the LX
  community index and repology: `lib/plugins/package_index/aur.rs`,
  `lib/index/mod.rs`.
- `lx index install aur/<pkg>` fetches the PKGBUILD, converts it with a
  regex (`pkgbuild_to_yaml`), and builds through `build_from_recipe`.
- `--install-build-deps` installs AUR `makedepends` before compiling.
- `lx upgrade --all` adds repology-based system-wide freshness.

## Gap checklist

- [ ] AUR dependency graph: resolve `depends`/`makedepends` recursively
      across repo and AUR, handle `provides`/virtual and versioned deps, and
      topologically order the build (makedeps first).
- [ ] pacman-shaped surface (or an `lx aur` subcommand) for `-Ss`/`-Si`/
      `-S`/`-Qu`/`-Rns`/`-Syu`, with `--aur`/`--repo`, `--noconfirm`,
      `--needed`.
- [ ] PKGBUILD diff review: clone the AUR git repo, remember the last-built
      revision per package, print a unified diff before rebuilding
      (`--diffmenu`/`--editmenu`).
- [ ] AUR metadata beyond votes/maintainer: popularity, out-of-date, orphan
      status, last-modified, comments.
- [ ] VCS packages (`--devel`): build `-git`/`-svn`/`-hg` at latest and
      track rebuild triggers.
- [ ] Foreign-package tracking through the host manager, so `-Qm`/`-Qu`
      see lx-installed AUR packages. Note the AUR index currently probes
      `debs::dpkg_installed_version`, which is Debian-only; use the
      cross-platform `scandeps::pkg_installed_version` / `lib/info.rs`.
- [ ] Rebuild/cleanup flags: `--rebuild`, `--cleanafter`, `--answerclean`,
      `--mflags`/`--makepkg-conf` passthrough.
- [ ] AUR RPC v5 batch `info`/search-by-dependency with rate-limit backoff.

## Insertion points

| Change | File |
|---|---|
| AUR dependency graph and ordering | new `lib/aur.rs` (or extend `lib/index/mod.rs`) |
| Rich AUR metadata in hits | `lib/plugins/package_index/aur.rs` |
| Diff review and last-built revision state | `lib/index/mod.rs`, manifest under the XDG data dir |
| `-Qu`/`-Syu` over foreign packages | `lib/upgrade.rs`, `lib/consumer.rs` |
| Cross-platform "installed" check | `lib/scandeps.rs::pkg_installed_version`, `lib/info.rs` |
| CLI surface | `lib/cli.rs` |
| Build via the PKGBUILD executor | `drop-in-makepkg.md` |

## Done when

- `lx` can install an AUR package whose `makedepends` include another AUR
  package, building the dependency first.
- `lx` reports outdated foreign packages with `-Qu`-style output and shows
  a PKGBUILD diff before rebuilding.
