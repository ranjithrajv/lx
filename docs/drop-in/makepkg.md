# Drop-in: makepkg

Part of the [drop-in roadmap](README.md). **Status:** open (Phase 3).
Current parity: [replacements.md §2.9](replacements.md).

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

## Consume or replace `makepkg`?

Two different layers are easy to conflate:

- **Assembler** — staged tree → `.pkg.tar.zst` (`.PKGINFO`/`.MTREE`/payload).
  `lx` already replaces this in-process (`lib/archarchive.rs`); that part is
  settled and not the question.
- **Driver** — execute the `PKGBUILD` bash, export makepkg's variables, run
  the phase functions, resolve `source=()` and checksums, compute `pkgver()`,
  and expand split packages. This is the missing layer, and it is the real
  question.

Today `lx` does neither for the driver: it regex-imports the PKGBUILD text,
so it consumes the *format* but neither consumes nor faithfully replaces the
tool.

**Consuming `makepkg`** (shelling out) buys fidelity — it is the reference
implementation — but it costs:

- An Arch-only dependency. `makepkg` ships with `pacman`; on Debian/rpm
  hosts it does not exist, so `lx` could no longer build Arch packages on any
  host, which is a core value.
- Host-config variability. `makepkg` reads `/etc/makepkg.conf` and
  `~/.makepkg.conf`; `options=` (strip, debug), compression, and `PKGEXT`
  change the output. `lx` deliberately keeps output controlled and
  reproducible (`SOURCE_DATE_EPOCH`, normalized archives).
- No help for the AUR→deb/rpm path, which is exactly where the AUR flow is
  used on non-Arch hosts — so `lx` would still need a second, different code
  path.
- A runtime dependency the project otherwise avoids (`tooling.md` §1.8).

**Replacing the driver** — a `pkgbuild` build-system plugin that runs the
recipe in bash and hands the staged tree to `lx`'s in-process assembler —
keeps one pipeline for every format, works on any host, and matches the
project's model of consuming *inputs* and replacing *builders*.

The `cmake`/`cargo`/`go` analogy does not transfer: those are generic
compilers `lx` should not reimplement, whereas `makepkg` is the Arch
*packaging driver* — the layer `lx` already owns and wants to control.

**Decision:** replace the driver, and consume `makepkg` only as a read-only
oracle:

- Execute the recipe ourselves (sandboxed, opt-in), then assemble in-process.
- Use `makepkg --printsrcinfo` as an authoritative metadata oracle *when it
  is present*, and as a test reference — the same pattern as using real
  `dpkg-deb`/`lintian` as oracles.
- Offer an explicit `build_backend: makepkg` opt-in for byte-for-byte
  makepkg output, with the config-variability caveat, never as the default.
- Read committed `.SRCINFO` for metadata everywhere (no execution, no
  makepkg needed) — also the safe first stage.

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
| Sandbox the executed shell | route recipe steps through `lib/plugins/build_system/mod.rs::build_command` (`--sandbox` is now wired) |
| Split-package outputs | `lib/build.rs`, `lib/plugins/mod.rs` |
| `arch:` config block (`options`, `install`, `backup`) | `lib/config.rs` |

## Done when

- `lx build` produces a package from a real `PKGBUILD`, including a split
  one, and `pacman -Qip` and `bsdtar` accept the result.
- `lx init --from-aur` and `lx install <pkg>` share the executor
  and no longer scrape fields with regexes.
