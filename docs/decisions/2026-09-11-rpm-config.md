# RPM compression, AutoProv/AutoReq, macro expansion

**Date:** 2026-09-11
**Status:** Implemented
**Context:** fpm has `--rpm-compression`, `--rpm-autoprov`, `--rpm-autoreq`,
`--rpm-macro-expansion`, `--rpm-rpmbuild-define`. nfpm has `rpm.compression`.
lx didn't expose any of these — RPM packaging that needed to suppress
auto-generated requires or use a different compression algorithm had no
recourse.

## What changed

New fields in `package.yaml`'s `rpm:` block:

```yaml
rpm:
  compression: "zstd"          # gzip, xz, lzma, zstd, none
  auto_provides: true          # auto-generate Provides: (default: true)
  auto_requires: true          # auto-generate Requires: (default: true)
  defines:
    - "_unpackaged_files_terminate_build 0"
```

Plus trigger fields (already implemented, now documented together):
```yaml
rpm:
  trigger_pre_install: ["bash: scripts/trigger-prein.sh"]
  trigger_post_install: ["bash: scripts/triggerin.sh"]
  trigger_pre_uninstall: ["bash: scripts/triggerun.sh"]
  trigger_post_uninstall: ["bash: scripts/triggerpostun.sh"]
```

## Implementation

- `lib/config.rs` — added `compression`, `auto_provides`, `auto_requires`,
  `defines` to `RpmConfig`; added `true_default()` serde helper.
- `lib/rpmarchive.rs` — added same fields to `BuildOptions`; wired
  `compression` through to `rpm::PackageBuilder::compression()`.
- `lib/plugins/rpm.rs` — passes `cfg.rpm.*` through to `BuildOptions`.

## Files changed

- `lib/config.rs` — `RpmConfig` fields + `true_default()`.
- `lib/rpmarchive.rs` — `BuildOptions` fields + compression wiring.
- `lib/plugins/rpm.rs` — config passthrough.
