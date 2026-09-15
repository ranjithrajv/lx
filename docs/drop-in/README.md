# Drop-in replacements

Part of the [lx docs](../README.md).

`lx` is a drop-in (or feature-parity) replacement for a growing set of
tools. This folder is the single home for that story: the current parity
analysis, and the phased work closing the remaining input-contract gaps.

## The tools

| Tool | Relationship | Doc |
|---|---|---|
| makepkg | open (Phase 3) | [makepkg.md](makepkg.md) |
| rpmbuild | open (Phase 4) | [rpmbuild.md](rpmbuild.md) |
| yay / paru | feature parity (Phase 5) | [yay-paru.md](yay-paru.md) |
| dpkg-shlibdeps | mostly done | [dpkg-shlibdeps.md](dpkg-shlibdeps.md) |
| checkinstall | open (Phase 6) | [checkinstall.md](checkinstall.md) |

## What "drop-in" means

A **drop-in** lets you point the incumbent's input — a `PKGBUILD`, a
`.spec`, an AUR package name, a `make install` — at `lx` and get an
equivalent artifact. It does not mean re-implementing the incumbent's whole
toolchain; [replacements.md](replacements.md) records the deliberate
non-territory.

## Current parity

[What `lx` replaces](replacements.md) is the level-by-level table: what is
already a **drop-in**, what is **feature parity** through a different
surface, and what is a **functional replacement** that produces the same
artifacts without an interface match.

## Phases

The `shlibdeps` and `build-deps` decisions called themselves "Phase 1" and
"Phase 2" of the same analysis. Both have landed; the rest are tracked in
this folder.

| Phase | Scope | State |
|---|---|---|
| 1 | Symbol-versioned dependency resolution (`lx deps resolve`) | Done — [decision](../decisions/2026-09-14-shlibdeps.md) |
| 2 | Host build-dependency solving (`--install-build-deps`, AUR `makedepends`) | Done — [decision](../decisions/2026-09-14-build-deps.md) |
| 3 | makepkg input fidelity (PKGBUILD execution) | [makepkg.md](makepkg.md) |
| 4 | rpmbuild `.spec` input | [rpmbuild.md](rpmbuild.md) |
| 5 | yay / paru AUR capability parity (install-and-record done) | [yay-paru.md](yay-paru.md) |
| 6 | checkinstall install capture (first cut done) | [checkinstall.md](checkinstall.md) |
| — | `dpkg-shlibdeps` leftovers | [dpkg-shlibdeps.md](dpkg-shlibdeps.md) |

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
- `--sandbox`: source-build compile/install steps run under `unshare -n`
  (`lib/plugins/build_system/mod.rs::build_command`), probed once with a
  warn-and-fallback (`sandbox_available`).
- AUR/community recipe builds **install and record** their artifact
  (`lib/plugins/package_index/lx_community.rs`), and the AUR index uses the
  cross-platform installed check.
- `lx capture`: run an install command and package its `$DESTDIR` tree
  (`lib/capture.rs`).

## Cross-cutting foundations

Each foundation unblocks more than one tool, so they are sequenced before
the per-tool work that depends on them.

| Foundation | Unblocks | Where |
|---|---|---|
| PKGBUILD executor (bash, phase order) | makepkg, yay/paru | new `lib/plugins/build_system/pkgbuild.rs` |
| `.spec` parser/evaluator | rpmbuild | new `lib/plugins/build_system/spec.rs` |
| Multi-output builds (split packages / subpackages) | makepkg, rpmbuild | core build model (`lib/build.rs`, `lib/plugins/mod.rs`) |
| Install capture (run an installer, package the delta) | checkinstall | new `lib/capture.rs` |

## Priority

The phase numbers above are labels from the original decision sequence, not
execution priority. This is the order to build:

### Step 0 — safety and small correctness wins

1. **Wire `--sandbox` (`unshare -n`).** Must precede any execution of
   untrusted recipes or installers; today it is declared but never invoked
   (`tooling.md` §1.8).
2. ~~**Fix the AUR flow's two prerequisites.**~~ Done: the recipe path
   installs and records (`InstallOutcome`), and the AUR index uses the
   cross-platform installed check.
3. ~~**checkinstall capture.**~~ Done: `lx capture` runs an install command
   and packages the captured tree (`--exclude` filters included).
4. **`dpkg-shlibdeps` leftovers.** Virtual `Provides`, multiarch
   qualifiers, `shlibs.local`, flag parity, and feeding the resolved
   relations into `--source` and `lx convert`.

### Step 1 — foundations

5. **PKGBUILD executor** plus the `.SRCINFO`/`--printsrcinfo` static path
   (Phase 3). Unlocks makepkg and yay/paru.
6. **Multi-output builds** (split packages / subpackages). Shared by
   makepkg and rpmbuild.

### Step 2 — parity work

7. **AUR dependency graph + lx-native AUR surface** (yay/paru feature
   parity, Phase 5): resolver, install-and-record, foreign tracking, diff
   review, VCS packages. Needs 5 and 6.
8. **`.spec` parser/evaluator** (rpmbuild, Phase 4). The largest single
   piece; shelling out to `rpmspec`/`rpmbuild` for `%{…}` expansion is an
   acceptable first cut.

## See also

- [What `lx` replaces](replacements.md) — the current parity table and
  level definitions.
- [Tooling](../architecture/tooling.md) — what `lx` consumes at runtime.
- [Linux packaging landscape](../analysis/landscape.md) — where each tool
  sits in the pipeline.
