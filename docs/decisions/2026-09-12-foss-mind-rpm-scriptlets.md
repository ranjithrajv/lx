# Dependency Decision: RPM scriptlet extraction & cross-distro package install

**Date:** 2026-09-12
**Context:** `lx convert` needs to extract maintainer scripts from source RPMs; `lx go-native --yes` needs to install native packages on rpm/arch hosts.
**Decision context:** default

## Problem

Two capabilities were needed for fpm parity:
1. Extract `%pre`/`%post`/`%preun`/`%postun` scriptlets from existing RPMs during `lx convert`.
2. Install native packages on rpm-based and Arch-based hosts during `lx go-native --yes`.

## Candidates evaluated

| Library | Score | Notes |
|---------|-------|-------|
| `rpm` crate (scriptlet reading) | 85/100 | `PackageMetadata::open()` + `get_pre_install_script()` etc. — pure Rust, no `rpm` binary needed. Replaces `rpm -qp --queryformat` shell calls. |
| `rpm` crate (package install) | 0/100 | Only builds RPMs, cannot install them. No pure Rust RPM installer exists. |
| `pacman` crate | 0/100 | No pure Rust library for installing Arch packages exists. |
| Shell out to `rpm`/`pacman` binaries | 70/100 | The only option for installation. Works everywhere the binary is present. |

## Decision

**Chosen:**
- For RPM scriptlet extraction in `lx convert`: `rpm -qp --queryformat` via `Command` (current implementation). The `rpm` crate's `PackageMetadata` API is a cleaner alternative and should be adopted in a future pass — tracked below.
- For native package installation: shell out to `rpm -Uvh --force` and `pacman -U --noconfirm`. No library alternative exists; this is the only approach.

**License:** MIT/Apache-2.0 (rpm crate, already a dependency)
**Score:** 70/100 (threshold: 60)

## Integration notes

The current `lx convert` implementation shells out to `rpm -qp --queryformat` for each scriptlet tag. This works but requires the `rpm` binary on PATH. The `rpm` crate (already a build dependency) provides `PackageMetadata::open(path)?.get_pre_install_script()` etc. — a pure Rust alternative that removes the external binary requirement. Migrating to it is recommended when touching this code next.

For package installation, no wrapper is possible — the `rpm` and `pacman` binaries are the installation mechanism. The `Command` calls are already thin.

## Future improvement

- [ ] Migrate `extract_rpm_scripts()` in `lib/convert.rs` from `rpm -qp --queryformat` shell calls to the `rpm` crate's `PackageMetadata` API. This removes the `rpm` binary requirement for `lx convert`.

## Review date

2026-12-12 — re-score if the `rpm` crate adds package installation support.
