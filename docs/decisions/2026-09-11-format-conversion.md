# `lx convert`: format conversion (deb↔rpm↔arch)

**Date:** 2026-09-11
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
1. **Extract metadata** from the source package (control fields for `.deb`,
   `rpm -qp --queryformat` for `.rpm`, `.PKGINFO` for `.pkg.tar.zst`).
2. **Extract the install tree** (data.tar for `.deb`, rpm2cpio+cpio for
   `.rpm`, zstd+tar for `.pkg.tar.zst`).
3. **Rebuild natively** via the target format plugin (`deb`/`rpm`/`arch`),
   reusing the same `BuildContext` pipeline as `lx build`.

This produces a proper native package, not a converted archive — dependency
relations, maintainer metadata, and FHS placement are all correct for the
target format.

## Overrides

All metadata fields can be overridden via CLI flags:
`--package-name`, `--version`, `--arch`, `--distribution`, `--build-version`.

## Known limitations

- **rpm source** requires `rpm` and `rpm2cpio` binaries on PATH (no native
  Rust RPM reader for arbitrary packages).
- **arch source** requires `zstd` crate (already a dependency).
- Scriptlets (pre/post-install) from the source are **not** carried over —
  the converted package has no maintainer scripts. This matches fpm's
  behavior for cross-format conversion.
- Dependencies are carried over verbatim; distro-specific naming differences
  (e.g. `libatomic1` vs `libatomic`) are not auto-translated.

## Files changed

- `lib/convert.rs` — NEW: `ConvertArgs`, `run()`, extraction + rebuild logic.
- `lib/cli.rs` — added `Commands::Convert` variant + dispatch.
- `lib/lib.rs` — added `pub mod convert`.
- `tests/convert.rs` — NEW: unit tests for the convert command.
