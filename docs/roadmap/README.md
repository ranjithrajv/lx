# Drop-in replacement roadmap

Part of the [lx docs](../README.md).

The [drop-in replacement analysis](../architecture/replacements.md) and
[`tooling.md`](../architecture/tooling.md) describe where `lx` stands today.
This directory tracks the remaining *input-contract* gaps: the tools whose
native input `lx` does not yet accept end to end.

**Drop-in** here means you can point the incumbent's input — a `PKGBUILD`,
a `.spec`, an AUR package name, a `make install` — at `lx` and get an
equivalent artifact. It does not mean re-implementing the incumbent's whole
toolchain (see [Part 3 of the analysis](../architecture/replacements.md)
for the deliberate non-territory).

## Phases

The `shlibdeps` and `build-deps` decisions called themselves "Phase 1" and
"Phase 2" of the same analysis. Both have landed; the rest are tracked here.

| Phase | Scope | State |
|---|---|---|
| 1 | Symbol-versioned dependency resolution (`lx deps resolve`) | Done — [decision](../decisions/2026-09-14-shlibdeps.md) |
| 2 | Host build-dependency solving (`--install-build-deps`, AUR `makedepends`) | Done — [decision](../decisions/2026-09-14-build-deps.md) |
| 3 | makepkg input fidelity (PKGBUILD execution) | Open — [doc](drop-in-makepkg.md) |
| 4 | rpmbuild `.spec` input | Open — [doc](drop-in-rpmbuild.md) |
| 5 | yay / paru AUR resolution and UX | Open — [doc](drop-in-yay-paru.md) |
| 6 | checkinstall install capture | Open — [doc](drop-in-checkinstall.md) |
| — | `dpkg-shlibdeps` leftovers | Mostly done — [doc](drop-in-dpkg-shlibdeps.md) |

## Already landed (do not redo)

These appeared in earlier gap lists and are now implemented:

- Arch `.PKGINFO` relation and `backup` metadata — `lib/archarchive.rs`
  (`render_pkginfo_with`), `lib/plugins/arch.rs`.
- RPM `auto_provides` / `auto_requires` ELF scans —
  `lib/rpmarchive.rs` (`auto_elf_relations`).
- Per-file `contents:` metadata (`file_info`, `expand`, `disown_subtree`) —
  `lib/filemeta.rs`, `lib/plugins/contents.rs`.
- `dpkg-shlibdeps` core (symbols/shlibs, versioned relations) —
  `lib/shlibdeps.rs`.
- Host build-dependency solving — `lib/builddeps.rs`.

## Cross-cutting foundations

Each foundation unblocks more than one tool, so they are sequenced before
the per-tool work that depends on them.

| Foundation | Unblocks | Where |
|---|---|---|
| Wire `--sandbox` (`unshare -n`) | makepkg, rpmbuild, checkinstall | `lib/build.rs`, `lib/sourcebuild.rs` (`tooling.md` §1.8: declared, not wired) |
| PKGBUILD executor (bash, phase order) | makepkg, yay/paru | new `lib/plugins/build_system/pkgbuild.rs` |
| `.spec` parser/evaluator | rpmbuild | new `lib/plugins/build_system/spec.rs` |
| Multi-output builds (split packages / subpackages) | makepkg, rpmbuild | core build model (`lib/build.rs`, `lib/plugins/mod.rs`) |
| Install capture (run an installer, package the delta) | checkinstall | new `lib/capture.rs` |

## Priority

1. **Wire `--sandbox` and add capture mode.** Small, and capture reuses the
   existing `custom`/`DESTDIR` path (`lib/plugins/build_system/custom.rs`).
2. **PKGBUILD executor + split packages.** Unlocks makepkg and is the
   prerequisite for yay/paru.
3. **AUR dependency graph + pacman-style UX.** Needs 2.
4. **`.spec` evaluator.** The largest single piece; shelling out to
   `rpmspec`/`rpmbuild` for `%{…}` expansion is an acceptable first cut.

## See also

- [What `lx` replaces](../architecture/replacements.md) — the current parity
  table and level definitions.
- [Tooling](../architecture/tooling.md) — what `lx` consumes at runtime.
- [Linux packaging landscape](../analysis/landscape.md) — where each tool
  sits in the pipeline.
