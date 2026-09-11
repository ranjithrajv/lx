# Adopt remaining nfpm features (relations, glob, metadata, version schema, umask)

**Date:** 2026-09-11
**Status:** Implemented & verified (270 tests pass)
**Context:** Closing the remaining feature gaps identified in the
lx-vs-nfpm comparison. Eight features adopted in one pass.

## What changed

### 1. RPM relation fields
`lib/rpmarchive.rs` gained `RpmRelations` (a struct of
`Vec<rpm::Dependency>`), `parse_rpm_relations()` which parses
comma-separated config strings into `Dependency::any(name)`, and a
`relations` field on `BuildOptions`. The builder applies them via the
rpm crate's `.requires()`, `.provides()`, `.conflicts()`,
`.obsoletes()`, `.recommends()`, `.suggests()` methods. The rpm plugin
(`lib/plugins/rpm.rs`) wires `cfg.effective_relations("rpm")` through.
Mapping: depends→requires, recommends→recommends, suggests→suggests,
conflicts→conflicts, replaces→obsoletes, provides→provides,
breaks→conflicts.

### 2. Glob patterns in `contents:`
`lib/plugins/mod.rs::apply_contents()` now detects glob patterns
(`*`, `?`, `[`) in `src` and expands them via the `glob` crate when
`disable_globbing` is false. Matches are staged into `dst` as a
directory (file type) or copied as subtrees (tree type). Empty matches
bail with a clear error. Controlled by the new `disable_globbing:
bool` config field.

### 4. Section / Priority (deb)
Already implemented — `section:` and `priority:` fields exist with
`effective_section()` / `effective_priority()` defaults. Documented as
parity.

### 5. Vendor / Packager (rpm)
`PackageMeta` in `lib/rpmarchive.rs` gained `vendor` and `packager`
fields (`Option<&str>`), applied via the rpm crate's `.vendor()` /
`.packager()` builder methods. The rpm plugin passes
`cfg.effective_packager()` (which falls back to maintainer when
unset). Vendor is not yet exposed as a dedicated config field — the
legacy `vendor:` key still folds into `fields:`.

### 6. Arch variant (deb)
New `arch_variant: String` field on `PackageConfig` with
`effective_arch_variant()`. The deb plugin's `render_control()` appends
it to the architecture (e.g. `amd64v3`).

### 7. Version schema
New `version_schema: String` field (default `"semver"`) with
`effective_version_schema()`. `lib/pkgmeta.rs` gained
`normalize_version(version, schema)` — the semver arm strips a `v`/`V`
prefix then any non-digit prefix; `none` returns the version as-is.
The build pipeline (`lib/build.rs`) uses it in place of the bare
`strip_upstream_prefix()`.

### 8. Umask control
New `umask: String` field with `effective_umask() -> Option<u32>` that
parses octal (accepts `0o002` or `002`). `lib/plugins/mod.rs` gained
`apply_umask(path, umask)` which does `mode & !umask`. Applied in
`stage_file()`, `stage_ancillary_file()`, and `stage_install_tree()`
for all file-staging paths.

### 9. Deb trigger await/noawait variants
`DebConfig` gained four new `Vec<String>` fields:
`triggers_interest_await`, `triggers_interest_noawait`,
`triggers_activate_await`, `triggers_activate_noawait`. The
`deb_extra_members()` helper renders them as `interest_await <name>` /
`interest_noawait <name>` / `activate_await <name>` /
`activate_noawait <name>` lines in `DEBIAN/triggers`.

## Schema & tests

- `lib/schema.rs`: documented all new fields (`arch_variant`,
  `version_schema`, `umask`, `packager`, `disable_globbing`, deb trigger
  await/noawait variants).
- `tests/new_features.rs`: 12 tests covering version normalization,
  arch variant, umask parsing, packager fallback, version schema, and
  validation.
- Existing tests updated for new `BuildOptions` / `PackageMeta` fields.

## Verification

```
cargo test    → 270 passed, 0 failed
cargo clippy  → clean (-D warnings)
cargo fmt     → clean
```

## nfpm feature parity: expanded

The lx-vs-nfpm comparison now shows parity in: dependency relations,
scripts & triggers, maintainer metadata & control fields, umask, version
schema, and glob patterns. Remaining intentional gaps: apk/ipk/msix
formats (out of scope), per-file file_info (mode/owner/group per
contents entry).
