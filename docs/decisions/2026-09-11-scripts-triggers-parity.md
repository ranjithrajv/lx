# Scripts & triggers parity with nfpm

**Date:** 2026-09-11
**Status:** Implemented
**Context:** Closing the last feature gap in the Scripts & triggers
category of the lx-vs-nfpm comparison. The four basic lifecycle scripts
(pre/post-install/remove) were already wired into deb and rpm; this pass
adds the format-specific extras nfpm supports: rpm `%pretrans`/`%posttrans`
/`%verify`, Arch `.INSTALL` pre/post-upgrade hooks, and the deb-only
features (debconf templates/config, maintainer triggers, `rules`).

## What changed

### 1. RPM: `%pretrans` / `%posttrans` / `%verify`

`Scripts` (`lib/config.rs`) gained `pretrans`, `posttrans`, `verify`
fields. `BuildOptions` (`lib/rpmarchive.rs`) gained matching
`pre_trans`, `post_trans`, `verify_script` options, wired to the
`rpm` crate's `pre_trans_script`/`post_trans_script`/`verify_script`
builder methods. The rpm plugin (`lib/plugins/rpm.rs`) passes them
through. Empty values are treated as unset (no scriptlet emitted).

### 2. Arch: `.INSTALL` pre/post-upgrade hooks

`Scripts` gained `preupgrade`/`postupgrade` fields. A new
`render_install_script()` helper (`lib/archarchive.rs`) emits a
`.INSTALL` body with `pre_upgrade()` / `post_upgrade()` functions
(none returned when both empty, so no spurious `.INSTALL` member).
`build()` takes a new `install_script: Option<&str>` parameter; the
arch plugin (`lib/plugins/arch.rs`) threads the configured hooks
through. Arch packages that set neither hook produce no `.INSTALL`,
matching existing behavior.

### 3. Deb: debconf, triggers, rules

A new `DebConfig` struct (`lib/config.rs`) holds deb-only features:

- `rules` → `DEBIAN/rules` (mode 0755)
- `templates` → `DEBIAN/templates` (mode 0644) — debconf
- `config` → `DEBIAN/config` (mode 0755) — debconf
- `triggers_interest` / `triggers_activate` → `interest <name>` /
  `activate <name>` lines in `DEBIAN/triggers` (mode 0644)

A new `deb_extra_members()` helper (`lib/plugins/mod.rs`) reads these
from the build environment and emits the corresponding
`ControlMember`s. The deb plugin's `archive_staged_tree` layers them
after the existing maintainer scripts and conffiles. Validation rejects
empty trigger entries.

### 4. Schema & tests

- `lib/schema.rs`: `scripts` properties extended with the six new
  fields; a new `deb` block documents the five deb-specific fields.
- `tests/rpmarchive.rs`: new test covers pretrans/posttrans/verify
  building cleanly.
- `tests/archarchive.rs`: four new tests cover `render_install_script`
  (both-hooks, pre-only, none) and `.INSTALL` presence/absence in the
  built archive.
- `tests/deb_extras.rs`: new file — eight tests cover empty case, each
  member individually (verifying correct mode), combined output, trigger
  lines, YAML parsing, and validation rejection of empty entries.
- `tests/schema.rs`: extended to assert all new script and deb fields
  appear in the generated schema.

## Verification

```
cargo test    → all pass (existing + new)
cargo clippy  → clean (-D warnings)
cargo fmt     → clean
```

## nfpm feature parity: achieved

Every row in the Scripts & triggers comparison category is now ✅ for
lx. The remaining gaps between lx and nfpm are in other categories
(package formats, maintainer metadata, per-file `file_info`, umask),
not scripts/triggers.
