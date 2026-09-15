# Drop-in: makepkg

Part of the [drop-in roadmap](README.md). **Status:** open (Phase 3).
Current parity: [replacements.md §2.9](../architecture/replacements.md).

## The contract

`makepkg` executes a `PKGBUILD` — a bash script — with `$srcdir`,
`$pkgdir`, `$startdir`, and `$CARCH` set, calling `prepare()`, `pkgver()`,
`build()`, `check()`, and `package()` (plus `package_<name>()` for each
split package) in order, and writing `*.pkg.tar.zst` to `$PKGDEST`.

Its inputs are the arrays `source`, `sha256sums`, `depends`, `makedepends`,
`checkdepends`, `optdepends`, `provides`, `conflicts`, `replaces`,
`backup`, `arch`, `options`, and the `install=` script. Flags and env worth
mirroring: `-s`, `-r`, `-c`, `-C`, `-f`, `-p`, `--noconfirm`, `--needed`,
`--printsrcinfo`, `PKGDEST`, `SRCDEST`, `BUILDDIR`, `PKGEXT`, `PACKAGER`.

## Current state

- The in-process `.pkg.tar.zst` writer already emits `.PKGINFO` relations
  (`depend`, `optdepend`, `conflict`, `provides`, `replaces`, `backup`) and
  `.INSTALL` hooks: `lib/archarchive.rs`, `lib/plugins/arch.rs`.
- AUR import is **regex-only**: `lib/plugins/package_index/aur.rs`
  (`pkgbuild_to_yaml`) reads `pkgver`, `url`, `license`, and single-line
  `depends`/`makedepends`; `lib/wizard.rs` (`--from-aur`) is similar.
  PKGBUILD shell is never executed.
- Build dependencies for an imported recipe are installable with
  `--install-build-deps` (Phase 2).

## Gap checklist

- [ ] Execute the `PKGBUILD` in bash with makepkg's variables and phase order.
- [ ] Split packages: `pkgname=(...)` with `package_<name>()` → N artifacts.
- [ ] `pkgver()` and `pkgrel` computation; `--printsrcinfo` output and
      `.SRCINFO` input as a non-executing fast path.
- [ ] `source=()` fetching and checksums: `sha256sums`/`sha512sums`/
      `b2sums`/`md5sums`, `SKIP`, arch-suffixed arrays, `noextract`, VCS
      sources (`git+`, `svn+`, `hg+`), `validpgpkeys` verification.
- [ ] Parse `optdepends`/`provides`/`conflicts`/`replaces`/`backup`/
      `options`/`arch`/`install=` into packaging (the packager side exists;
      the PKGBUILD side does not).
- [ ] `options=()` handling: `strip`, `docs`, `debug` (`-debug` split),
      `!emptydirs`, `lto`, `staticlibs`.
- [ ] `check()` step plus `checkdepends` — only once `check()` is executed.
- [ ] CLI/env parity, including writing to `$PKGDEST` and respecting
      `$PKGEXT`/`$PACKAGER`.

## Insertion points

| Change | File |
|---|---|
| `pkgbuild` build-system plugin (execute, export vars, phase order) | new `lib/plugins/build_system/pkgbuild.rs`; register in `lib/plugins/build_system/mod.rs` |
| AUR index uses the executor instead of `pkgbuild_to_yaml` | `lib/plugins/package_index/aur.rs` |
| `--from-aur` can emit a real source recipe | `lib/wizard.rs` |
| Sandbox the executed shell | `lib/build.rs`, `lib/sourcebuild.rs` (`unshare -n`, currently unwired — `tooling.md` §1.8) |
| Split-package outputs | `lib/build.rs`, `lib/plugins/mod.rs` |
| `arch:` config block (`options`, `install`, `backup`) | `lib/config.rs` |

## Done when

- `lx build` produces a package from a real `PKGBUILD`, including a split
  one, and `pacman -Qip` and `bsdtar` accept the result.
- `lx init --from-aur` and `lx index install aur/<pkg>` share the executor
  and no longer scrape fields with regexes.
