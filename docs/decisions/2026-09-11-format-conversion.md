# `lx convert`: format conversion (deb↔rpm↔arch)

**Date:** 2026-09-11 (updated 2026-09-15)
**Status:** Implemented
**Context:** fpm's `-s rpm -t deb` is a common need — teams that maintain one
format but need to ship to another. lx previously had no equivalent; you had
to rebuild from the forge release in each format separately.

## What changed

New `lx convert` command (`lib/convert.rs`):

```
lx convert ./foo_1.0_amd64.deb --to rpm
lx convert ./foo-1.0.x86_64.rpm --to deb --output ./dist/
lx convert ./foo-1.0-arch-x86_64.pkg.tar.zst --to deb
```

How it works (not byte conversion — native rebuild):
1. **Extract metadata** from the source package (control fields + the
   `conffiles`/`triggers` control members for `.deb`, the in-process `rpm`
   crate for `.rpm`, `.PKGINFO` + `.INSTALL` for `.pkg.tar.zst`).
2. **Extract the install tree** (data.tar for `.deb`, in-process rpm payload
   extraction for `.rpm`, zstd+tar for `.pkg.tar.zst`), skipping each format's
   control members (rpm headers, arch `.PKGINFO`/`.MTREE`/`.INSTALL`).
3. **Rebuild natively** via the target format plugin (`deb`/`rpm`/`arch`),
   reusing the same `BuildContext` pipeline as `lx build`.

This produces a proper native package, not a converted archive — dependency
relations, maintainer metadata, and FHS placement are all correct for the
target format.

## Overrides

All metadata fields can be overridden via CLI flags:
`--package-name`, `--version`, `--arch`, `--distribution`, `--build-version`.

## Known limitations

- **rpm source/target** is read and written entirely in-process via the `rpm`
  crate — no `rpm`/`rpm2cpio` binaries required.
- **arch source** requires `zstd` crate (already a dependency).
- Symlinks in the payload are recreated as symlinks (previously they aborted
  conversion when dangling/absolute, or were flattened into full copies).
- Maintainer scripts **are** carried over (deb `preinst`/`postinst`/`prerm`/
  `postrm`, rpm `%pre`/`%post`/`%preun`/`%postun`/`%pretrans`/`%posttrans`/
  `%verify`, arch `.INSTALL` hooks), mapped onto the target's own names. Arch
  can only express pre/post-upgrade hooks, so install/remove hooks are
  best-effort there.
- Dependencies are carried over and their syntax is rewritten per target;
  distro-specific naming differences (e.g. `libatomic1` vs `libatomic`, or
  `libc6` vs `glibc`) are not fully auto-translated. Auto-generated rpm
  soname/capability requires are dropped rather than emitted as invalid
  target names.

## Cross-distro system-library translation

A converted package is only useful if its dependencies resolve on the target
distro. `lib/distmap.rs` holds a curated deb↔rpm↔arch table keyed by upstream
project (Debian lists versioned sonames as aliases; the first is the canonical
reverse name). `convert::translate_name` now calls `distmap::translate` in
either direction, so `glibc`→`libc6` (rpm→deb), `openssl-libs`→`openssl`
(rpm→arch), etc. Unknown names pass through unchanged rather than being
guessed. `depmap::deb_to_rpm_name`/`deb_to_arch_name` now delegate to the same
table so the two never diverge.

## Output verify gate

`lx convert --lint` runs the target format's checker on the artifact and fails
on errors: `lintian` (deb), `rpm -K` (rpm), `namcap` (arch). The checker must
be on `PATH` (run the gate on a host with the target format's tooling); a
missing tool is an error, not a silent pass. `--lint-fail-on-warnings` also
fails on warnings. Implemented in `lib/pkgverify.rs`, wired into
`ConvertArgs`.

## Files changed

- `lib/convert.rs` — NEW: `ConvertArgs`, `run()`, extraction + rebuild logic;
  distmap name translation; `--lint` gate.
- `lib/distmap.rs` — NEW: curated deb↔rpm↔arch system-package table +
  `translate()`.
- `lib/pkgverify.rs` — NEW: target-format lint/verify gate (lintian / `rpm -K`
  / `namcap`).
- `lib/depmap.rs` — `deb_to_rpm_name`/`deb_to_arch_name` delegate to distmap.
- `lib/archarchive.rs` — `extract` also skips `.INSTALL`.
- `lib/config.rs` — `github_repo` optional at deserialize time; new
  `load_for_local` for `--from-dir`/`--from-file`.
- `lib/build.rs` — `--from-dir` surfaces a malformed config instead of silently
  defaulting.
- `lib/rpmarchive.rs` — `parse_rpm_relations` parses Debian/RPM/pacman
  constraint spellings.
- `lib/cli.rs` — added `Commands::Convert` variant + dispatch.
- `lib/lib.rs` — added `pub mod convert`, `pub mod distmap`, `pub mod pkgverify`.
- `tests/convert.rs` — NEW: functional tests for scripts, conffiles, symlinks,
  arch metadata, distmap translation.
- `tests/rpmarchive.rs` — NEW: deb-style relation constraint parsing.
- `docs/reference/commands.md` — `--lint` documented.
- `docs/analysis/landscape.md` — `lx convert` rated TRL 5.
