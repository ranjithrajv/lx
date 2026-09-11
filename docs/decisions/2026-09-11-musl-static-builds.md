# `musl: true` — musl-static builds for old-distro portability

**Date:** 2026-09-11
**Context:** A common pain point in Linux packaging (described in [Fresh's blog
post](https://getfresh.dev/docs/blog/packaging-for-linux/)) is that binaries
built on a newer distro won't run on older ones — glibc symbol versioning means
a binary linked against glibc 2.28 can't load on a system with glibc 2.17. The
"correct" fix (build in an old-distro container) inherits unpatched packages;
the practical fix (musl static linking) produces a single binary that runs on
any Linux.

## What it does

`musl: true` in package.yaml produces a musl-static binary with no glibc
dependency. For source builds, each build system plugin adjusts its compile
flags. For binary repacks, musl-named release assets are preferred. The
consumer client (`lx-get install`) falls back to a `+musl_{arch}.deb` when no
distro-specific build exists.

## Why musl over "build on oldest distro"

| Approach | Drawbacks | Musl alternative |
|---|---|---|
| Build in old-distro container | Inherits unpatched packages; requires maintaining old build environments | Host compiles musl target, no container needed |
| glibc symbol floor capping | Toolchain-specific, fragile version scripts | musl has no glibc; the problem vanishes |
| Ship separate binary per distro | N distros × M assets | One musl asset works everywhere |

Musl isn't universal — some C libraries don't build cleanly against musl, and
projects with glibc-specific features (NSS, certain `ioctl`s) may not work.
But for the majority of Rust/Go/C projects, it's the simplest universal
solution. Users who can't use musl fall back to the existing "build on oldest
suite" approach.

## Build system specifics

- **Cargo**: `--target x86_64-unknown-linux-musl` (or equivalent for the
  host arch). Auto-installed via `rustup target add`. Supports all Debian
  architectures Rust has musl targets for (amd64, arm64, armhf, i386,
  ppc64el, s390x, riscv64).
- **Go**: `CGO_ENABLED=0` — produces a fully static binary by default, no
  glibc dependency.
- **CMake**: `musl-gcc`/`musl-g++` compiler + `-static` linker flag. Requires
  `musl-tools` (Debian/Ubuntu) on the host.
- **Custom**: user's responsibility via `build_commands`.

## Scope decisions

- **`compute_depends()` skips `libc6` fallback for musl binaries.** A musl
  binary's `DT_NEEDED` is empty; the existing fallback to `libc6` would be
  incorrect. Returns an empty string so the `Depends:` field carries no
  spurious glibc requirement.
- **Binary repack: prefer musl assets, don't require them.** If a release
  ships both `*-linux-gnu.tar.gz` and `*-linux-musl.tar.gz`, the musl variant
  is selected. If only glibc assets exist, the build proceeds with those (the
  user gets a glibc binary rather than no binary).
- **Consumer fallback in `lx-get install`.** Tries dist-specific asset first
  (`+{dist}_{arch}.deb`), then `+musl_{arch}.deb`. Users on old distros where
  no dist-specific build exists get a working install with a notice.
- **No `--musl` CLI flag on `lx build`.** The musl choice is a packaging
  decision (how the binary should be linked), not a per-invocation override —
  it belongs in `package.yaml`. `lx scan-deps --prefer-musl` exists as an
  inspection aid (which asset *would* be selected).
