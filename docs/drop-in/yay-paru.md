# Feature parity: yay / paru

Part of the [drop-in roadmap](README.md). **Status:** feature-parity target
(Phase 5), not a drop-in. Depends on the [makepkg executor](makepkg.md).
Current parity: [replacements.md §2.9](replacements.md).

## What feature parity means here

`yay`/`paru` are pacman front ends: they expose pacman's flags **and** drive
the AUR. The target is the same capability set through **lx's own surface**,
not accepting yay/pacman flags.

**In scope — the AUR-helper capabilities:**

- Search and info across the repos and the AUR.
- Install an AUR package: resolve its dependencies, build it via `PKGBUILD`,
  install the result.
- Upgrade AUR packages and list which are outdated.
- Remove AUR packages.
- List foreign (AUR-originated) packages.
- Clean build cache / build directories.
- PKGBUILD diff review before a rebuild.
- VCS (`-git`-style) packages with rebuild tracking.
- Fetch a PKGBUILD without building.
- AUR metadata: votes, popularity, out-of-date, orphan, maintainer.

**Deliberately out of scope (no interface compatibility required):**

- pacman/yay flag compatibility (`-Ss`, `-Si`, `-S`, `-Qu`, `-Rns`,
  `-Syu`, `-Qm`, `--noconfirm`, `--devel`, …) — `lx` uses its own verbs.
- Repository-package operations and full `-Syu` transaction orchestration —
  `pacman` stays the repo backend (`replacements.md` Part 3).
- Linking libalpm. Shelling out to pacman for local-DB queries
  (`pacman -T`, `-Qq`, `-Qi`, `-Si`, `-Sp`) is sufficient for parity.

## Current state

- `lx index` has an AUR read index (search/info/install) plus the LX
  community index and repology: `lib/plugins/package_index/aur.rs`,
  `lib/index/mod.rs`.
- `lx index install aur/<pkg>` fetches the PKGBUILD, converts it with a
  regex (`pkgbuild_to_yaml`), builds through `build_from_recipe`, and now
  **installs the artifact and records it** as an lx-managed generation.
- `--install-build-deps` installs AUR `makedepends` before compiling.
- `lx upgrade --all` adds repology-based system-wide freshness.

## Capability checklist

- [ ] AUR dependency graph: resolve `depends`/`makedepends`/`checkdepends`
      recursively across repo and AUR, handle `provides`/virtual and
      versioned deps, and topologically order the build (makedeps first).
- [ ] An lx-native AUR surface (`lx aur search/info/install/upgrade/remove/
      list/clean`) — same capabilities, `lx` verbs; no pacman-flag
      compatibility.
- [x] Install the built artifact and record it: the recipe path hands the
      artifact to the host manager and returns an `InstallOutcome`, so the
      package joins `lx list`/`upgrade`/`rollback`.
- [ ] PKGBUILD diff review: clone the AUR git repo, remember the last-built
      revision per package, print a unified diff before rebuilding.
- [ ] AUR metadata beyond votes/maintainer: popularity, out-of-date, orphan
      status, last-modified.
- [ ] VCS packages: build `-git`/`-svn`/`-hg` at latest and track rebuild
      triggers.
- [x] Cross-platform installed check: the AUR index uses
      `consumer::installed_version(pkg, InstallFormat)` instead of the
      Debian-only `dpkg_installed_version`.
- [ ] Foreign-package tracking through the host manager, so an AUR package
      installed by `lx` is visible as foreign on Arch (`pacman -Qm`).
- [ ] Rebuild and cleanup controls: rebuild, clean-after, answer-clean,
      makepkg-config passthrough.
- [ ] AUR RPC v5 batch `info`/search-by-dependency with rate-limit backoff.

## Insertion points

| Change | File |
|---|---|
| AUR dependency graph and ordering | new `lib/aur.rs` (or extend `lib/index/mod.rs`) |
| Install-and-record the recipe build | `lib/plugins/package_index/lx_community.rs`, `lib/consumer.rs` |
| Rich AUR metadata in hits | `lib/plugins/package_index/aur.rs` |
| Diff review and last-built revision state | `lib/index/mod.rs`, manifest under the XDG data dir |
| Outdated/upgrade over foreign packages | `lib/upgrade.rs`, `lib/consumer.rs` |
| Cross-platform "installed" check | `lib/consumer.rs::installed_version` |
| `lx`-native AUR command surface | `lib/cli.rs` |
| Build via the PKGBUILD executor | `makepkg.md` |

## Done when

- `lx` can install an AUR package whose `makedepends` include another AUR
  package, building the dependency first, then installing and tracking the
  result.
- `lx` lists the AUR packages it owns as outdated when a newer `pkgver`
  exists, and shows a PKGBUILD diff before rebuilding.
