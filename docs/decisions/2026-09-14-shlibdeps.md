# `lx shlibdeps`: symbol-versioned dependency resolution (dpkg-shlibdeps parity)

**Date:** 2026-09-14
**Context:** Phase 1 of closing the gap to `dpkg-shlibdeps` (see the
"drop-in replacement" analysis). `lx` could already answer "which sonames
does this binary need?" via `elfdeps` (`DT_NEEDED`), and `--bindep` /
`compute_depends_from_dir` could map those sonames to *package names* via the
host package manager (`dpkg -S` / `rpm -q --whatprovides` / `pacman -Qo`).
What it could not do was what `dpkg-shlibdeps` actually does: read the
binary's **required symbol versions** and the dpkg **`symbols`/`shlibs`**
databases to emit a **versioned** relation (`libfoo1 (>= 1.2.3)`), failing
closed when a library has no dependency information.

## What it does

New `lib/shlibdeps.rs`:

- `needed_libraries_verbose(bytes)` — `DT_NEEDED` (reusing `elfdeps`) plus the
  `(symbol, version)` pairs required from each library, parsed from the ELF
  `.gnu.version_r` / `VERNEED` sections via the `object` crate. No
  `dpkg-shlibdeps`/`readelf` subprocess.
- `ShlibsDb::open(admindir)` / `ShlibsDb::host()` — parses every
  `<admindir>/info/*.symbols` and `*.shlibs` once, indexed by soname. Empty
  on non-dpkg hosts.
- `resolve(needs, db, exclude_pkg, self_provided)` — for each non-essential,
  non-self-provided library: prefer the `symbols` stanza (highest minimum
  version over the required symbols, falling back to the stanza header
  version), else the `shlibs` line; otherwise report the soname as
  unresolved. Deduplicates by package and sorts.
- `scan_elfs(paths)` — the shared tree scan (requirements + self-provided
  sonames) used by `scandeps::compute_depends_from_dir`,
  `bindep::detect_binary_deps`, and the command.
- `lx shlibdeps <path>…` — the standalone command. Positional ELF
  files/directories, `--admindir`, `-T <substvars>`, `-O`, and
  `--ignore-missing-info`.

## Design decisions

- **`symbols` first, `shlibs` second.** Same preference order as
  `dpkg-shlibdeps`: a package's `symbols` file gives per-symbol minimum
  versions; `shlibs` only gives a coarse per-library version. Both formats
  are parsed in-process (a stanza is `<soname> <pkg> <minver>` followed by
  ` sym@ver minver` lines; a shlibs line is `<lib> <soversion> <relation>`).
- **Two failure policies, deliberately.** The build pipeline is
  *best-effort*: unresolved sonames fall back to the existing
  `dpkg -S`/`rpm`/`pacman` lookup, so rpm/arch hosts and libraries with no
  dpkg metadata behave exactly as before, and no existing build starts
  failing. The standalone command is *fail-closed*: no dependency
  information is an error unless `--ignore-missing-info`, matching
  `dpkg-shlibdeps`.
- **`exclude_pkg` / self-provided libraries.** A package must not depend on
  itself. `detect_binary_deps_excluding` drops relations on the package
  being built, and `resolve` skips sonames the scanned tree itself provides
  (`libfoo.so.*` files) — the `debian/*/DEBIAN/shlibs` behavior, needed for
  `bundle: true` packages that ship their own shared libraries.
- **Debian version comparison implemented in-tree, not a new crate.** Picking
  the *highest* required minimum version needs dpkg's version ordering
  (epoch, `~` before everything, numeric digit runs, revision). That is
  ~60 lines and was written directly (`debian_version_cmp`) rather than
  adding a dependency for it.
- **Host-local, informational in the build path; the command is the
  authority.** `ShlibsDb::host()` reflects the build host's installed dpkg
  database, not the target suite/architecture — consistent with the existing
  `pkg_owner` caveat. Cross-suite resolution against an explicit
  `--admindir`/rootfs is available at the command level.

## Scope not taken (deliberately)

- No `Provides:`/virtual-package handling, multiarch (`pkg:arch`) qualifiers,
  `debian/shlibs.local`, alternatives merging beyond the first `|` branch, or
  `-e`/`-T`/`-O`/`-d`/`-p`/`-l` flag-for-flag parity. These can be added on
  top of the same `ShlibsDb`/`resolve` core when a real caller needs them.
- No `lx debian/rules` execution: the command is a drop-in for
  `dpkg-shlibdeps` itself, not for `dpkg-buildpackage`.

## Verification

- `tests/shlibdeps.rs`: Debian version ordering (numeric `1.9 < 1.10`,
  epochs, `~`, revisions); `symbols` resolution (highest minimum over
  required symbols, `#MINVER#` and concrete headers); `shlibs` fallback;
  unresolved/exclude/self-provided paths; `needed_libraries_verbose` against
  the host's real `/bin/ls` (libc imports must carry `VERNEED` versions);
  `scan_elfs` self-provided detection; and two CLI tests — empty-directory
  error, and fail-closed-without-info vs `--ignore-missing-info` (skipped
  when the probe binary has only essential libraries).
- Live: `lx shlibdeps /bin/ls` fails closed on `libcap.so.2` on this
  Arch host (no dpkg database); with a synthetic `--admindir` symbols stanza
  it emits `libcap2` and `libcap2 (>= 2.66)` for the `#MINVER#` and concrete
  header cases respectively.
- `cargo clippy --all-targets --all-features -- -D warnings` clean. (The three
  failing `tests/config.rs` cases are pre-existing, unrelated to this change:
  they still assert that `meson` is rejected and that the new apk/ipk
  packagers are invalid.)

## Known limitations

- `symbols` matching is `(symbol, version)`-exact; when a required symbol is
  absent from the stanza it falls back to the header version rather than
  erroring like `dpkg-shlibdeps` would. Conservative for the build path;
  the command still fails closed when there is no stanza/shlibs line at all.
