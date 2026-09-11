# `lx convert` scriptlet carry-over

**Date:** 2026-09-11
**Status:** Implemented
**Context:** fpm carries pre/post-install scripts across format conversions.
`lx convert` previously dropped them — packages that relied on maintainer
scripts (e.g., creating users, registering services) lost that functionality
when converted.

## What changed

`lx convert` now extracts scriptlets from the source package and re-emits
them in the target format:

- **deb source**: reads `DEBIAN/{preinst,postinst,prerm,postrm,config,templates}`
  from the control.tar member.
- **rpm source**: reads `%{PREIN}`, `%{POSTIN}`, `%{PREUN}`, `%{POSTUN}`,
  `%{PRETRANS}`, `%{POSTTRANS}`, `%{VERIFYSCRIPT}` via `rpm -qp --queryformat`.

Script names are mapped to the target format's expected names:
- `preinst`/`pre` → `preinstall`
- `postinst`/`post` → `postinstall`
- `prerm`/`preun` → `preremove`
- `postrm`/`postun` → `postremove`
- `pretrans`, `posttrans`, `verify` → same name

## Implementation

- `lib/convert.rs` — added `scripts: BTreeMap<String, String>` to `SourceMeta`,
  `extract_deb_scripts()`, `extract_rpm_scripts()`, `apply_scripts_to_config()`.
- `tests/convert.rs` — added `convert_deb_dry_run_reports_metadata` test.

## Files changed

- `lib/convert.rs` — script extraction + mapping.
- `tests/convert.rs` — NEW test.
