# Adopt fpm features: pre/post-upgrade, RPM triggers, script templating

**Date:** 2026-09-11
**Status:** Implemented & verified (283 tests pass)
**Context:** Closing the remaining feature gaps vs fpm identified in the
three-way fpm-vs-nfpm-vs-lx comparison.

## What changed

### 1. Pre-/post-upgrade scripts

New fields on `Scripts`: `preupgrade_script`, `postupgrade_script`.

- **deb**: `DEBIAN/preupgrade` and `DEBIAN/postupgrade` control members
  (mode 0755). Non-standard deb control files that packagers wire into
  preinst/postinst.
- **rpm**: document that existing `pretrans`/`posttrans` serve double
  duty as upgrade hooks.
- **arch**: document that existing `preupgrade`/`postupgrade` map to
  `pre_upgrade()`/`post_upgrade()` in `.INSTALL`.

### 2. RPM triggers (4 types)

New `RpmConfig` struct under the `rpm:` key:

```yaml
rpm:
  trigger_pre_install:
    - "bash: scripts/trigger-prein.sh"
  trigger_post_install:
    - "bash: scripts/triggerin.sh"
  trigger_pre_uninstall:
    - "bash: scripts/triggerun.sh"
  trigger_post_uninstall:
    - "bash: scripts/triggerpostun.sh"
```

Each entry is `"package: script_path"`. The trigger *dependency* (the
condition) is emitted as a `rpm::Dependency` with the matching trigger
flag (`TRIGGERPREIN`, `TRIGGERIN`, `TRIGGERUN`, `TRIGGERPOSTUN`). The
associated script is emitted as a best-effort post-install scriptlet.

**Limitation:** the `rpm` crate has trigger *constants* but no builder
API for native `%triggerin`/`%triggerun` scriptlets. Full trigger script
support requires rpmbuild or a future rpm-crate version. The trigger
dependencies (the condition) are always correctly emitted.

### 3. Script templating (ERB-like)

New `template_scripts: bool` field on `PackageConfig` (default false).
When true, all maintainer scripts are processed through a template
engine before staging.

Available variables:
`<%= name %>`, `<%= version %>`, `<%= maintainer %>`, `<%= description %>`,
`<%= homepage %>`, `<%= license %>`, `<%= arch %>`, `<%= dist %>`,
`<%= iteration %>`, `<%= epoch %>`, `<%= vendor %>`, `<%= packager %>`,
`<%= prefix %>`.

Implementation: regex-based `<%= key %>` replacement from a context
built from `PackageConfig` + `ResolvedJob`. Unknown keys are left as-is
with a stderr warning.

## Files changed

- `lib/config.rs` — `preupgrade_script`/`postupgrade_script` fields,
  `RpmConfig` struct, `template_scripts` field, validation
- `lib/templating.rs` — NEW: template engine (`render_template`,
  `build_context`, 5 unit tests)
- `lib/lib.rs` — `pub mod templating;`
- `lib/rpmarchive.rs` — `RpmTrigger` struct, `parse_rpm_triggers()`,
  trigger fields on `BuildOptions`, trigger application in
  `build_with_options()`
- `lib/plugins/mod.rs` — `maintainer_script_members()` takes
  `&BuildContext`, applies template rendering when enabled, emits
  preupgrade/postupgrade control members
- `lib/plugins/deb.rs` — updated call site
- `lib/plugins/rpm.rs` — parses `rpm.trigger_*` config into
  `RpmTrigger` + flags
- `lib/schema.rs` — documented all new fields
- `tests/fpm_parity.rs` — 6 integration tests

## Verification

```
cargo test    → 283 passed, 0 failed
cargo clippy  → clean (-D warnings)
cargo fmt     → clean
```

## Known limitations

- **RPM trigger scripts**: emitted as post-install scriptlets
  (best-effort). True `%triggerin`/`%triggerun` scriptlets need rpmbuild
  or rpm-crate enhancement.
- **pre-upgrade/post-upgrade on rpm**: reuse `pretrans`/`posttrans`
  rather than separate scriptlets (RPM has no separate upgrade
  scriptlet concept).
- **Script templating**: regex-based, not full ERB. No conditionals
  or loops — just variable substitution.
