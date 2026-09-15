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

## Remaining gaps

- [ ] `Provides` / virtual-package resolution.
- [ ] Multiarch (`pkg:arch`) qualifiers.
- [ ] `debian/shlibs.local` and alternatives merging beyond the first `|`
      branch.
- [ ] Exact `dpkg-shlibdeps` error semantics when a required symbol is
      missing from a stanza (today it falls back to the header version).
- [ ] Full flag parity: `-e`, `-d`, `-p`, `-l`, and `-S`.
- [ ] Target-suite resolution wired end to end: `--admindir` accepts one,
      but a per-suite symbols database is not generated for cross-suite
      builds.
- [ ] Feed the resolved relations into `--source` (`debian/control`
      `Depends`) and into `lx convert`'s rpm/arch output instead of copying
      a static `depends:`.

## Insertion points

| Change | File |
|---|---|
| `Provides`/virtual, multiarch, `shlibs.local`, alternatives | `lib/shlibdeps.rs` |
| Flag parity (`-e`/`-d`/`-p`/`-l`/`-S`) | `lib/cli.rs`, `lib/shlibdeps.rs` |
| Per-suite symbols database | `lib/repo.rs`, `lib/index/`, `lib/shlibdeps.rs` |
| Source-package control from the scan | `lib/source.rs` |
| Converted package deps | `lib/convert.rs` |

## Done when

- `lx deps resolve` is flag-for-flag interchangeable with `dpkg-shlibdeps`
  for the common in-tree case, and `--source` controls carry its resolved
  versioned relations.
