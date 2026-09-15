# Drop-in: dpkg-shlibdeps

Part of the [drop-in roadmap](README.md). **Status:** mostly done — leftovers
tracked here. Decision: [`decisions/2026-09-14-shlibdeps.md`](../decisions/2026-09-14-shlibdeps.md).

## The contract

`dpkg-shlibdeps` runs inside a Debian build, reads the built binaries, the
dpkg `symbols`/`shlibs` databases, and `debian/shlibs.local`, and emits
**versioned** dependency relations (`libcap2 (>= 2.66)`) into a substvars
file, failing closed when a library has no dependency information. Relevant
flags: `-e<binary>`, `-d<Build-Depends>`, `-p`, `-l<dir>`, `-T<substvars>`,
`-O`, `--ignore-missing-info`.

## Current state

`lib/shlibdeps.rs` implements the core:

- `needed_libraries_verbose` — `DT_NEEDED` plus the `(symbol, version)`
  requirements from `.gnu.version_r`/`VERNEED`, via the `object` crate.
- `ShlibsDb::open(admindir)` / `ShlibsDb::host()` — parse `*.symbols` and
  `*.shlibs`, indexed by soname.
- `resolve` — highest required minimum version, `symbols` over `shlibs`,
  self-provided and `exclude_pkg` handling; Debian version ordering lives in
  `lib/versioncmp.rs`.
- `lx deps resolve <path>…` with `--admindir`, `-T`, `-O`, and
  `--ignore-missing-info`; fail-closed, while the build path falls back to
  the host-package lookup so existing builds do not start failing.

## Landed

- `debian/shlibs.local` overrides, highest precedence (`with_shlibs_local`,
  auto-detected by `find_shlibs_local`; `--shlibs-local` to point elsewhere).
- `a | b` alternatives: the first installed alternative, else the first.
- Missing-symbol strictness: a required symbol whose *name* is absent from
  an existing `symbols` stanza is fatal for the command (a warning under
  `--ignore-missing-info`), while the build path ignores it. A known symbol
  under a different version tag is not flagged.
- Flag parity: `-e` (extra executables), `-p` (exclude the package being
  built), and `-d`/`-l`/`-S` accepted for compatibility (`-T`/`-O`/`-l`
  already existed).
- Virtual-package `Provides`: `<admindir>/status` is parsed, and a stanza
  package name that is not itself a real package but is provided by one is
  replaced with the real provider (preserving any `:arch`).
- Multiarch `pkg:arch`: the standalone command preserves the qualifier
  `dpkg -S` reports, dropping it only for the host's native architecture
  (non-dpkg hosts keep the plain package-manager lookup).
- Cross-suite / target resolution: `ShlibsDb::host()` reads
  `$LX_DPKG_ADMINDIR`, so a build or conversion can resolve against a target
  rootfs's dpkg database (`--admindir` for the standalone command).
- `--source`: `sourcebuild::compute_depends` resolves through the shared
  core, so Debian source-package control files carry **versioned**
  relations (rpm/arch get bare names in their own syntax).
- `lx convert`: `scan_and_fill_deps` uses the shared core, so rpm/arch
  conversions are filled from `symbols`/`shlibs` rather than only `dpkg -S`.

## Limits

- No per-suite `symbols` database is *fetched*; the operator points
  `--admindir` / `$LX_DPKG_ADMINDIR` at a target rootfs.
- Multiarch qualifiers come from `dpkg -S`, not from the library's own ELF
  architecture.
- Missing-symbol matching is name-based: a known symbol under an unexpected
  version tag is not flagged (avoids false positives).

## Insertion points

| Change | File |
|---|---|
| `Provides` / `symbols` / `shlibs` / `shlibs.local` / status | `lib/shlibdeps.rs` |
| `--source` control from the resolved relations | `lib/sourcebuild.rs` |
| Converted-package deps | `lib/convert.rs` |

## Done when

- `lx deps resolve` is flag-for-flag interchangeable with `dpkg-shlibdeps`
  for the common in-tree case, and `--source` controls carry its resolved
  versioned relations.
