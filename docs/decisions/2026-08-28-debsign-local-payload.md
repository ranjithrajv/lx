# Debsign (`_gpgorigin`), local payloads, and env expansion

**Date:** 2026-08-28
**Context:** Close the remaining nfpm signing gap (embedded deb signatures)
and support packaging a local archive/directory without a forge download.
No prior design note existed for this slice; this record is that note.

## What shipped

### 1. `signature.method: debsign` → `_gpgorigin`

- `sign::clearsign(payload, req)` armored-detach-signs bytes via
  `gpg --armor --detach-sign` (hermetic temp `GNUPGHOME`, same import path
  as `gpg_detach_sign`). Despite the historical name, this is **not**
  `gpg --clearsign`: debsigs / nfpm's debsign method expect a detached
  signature over `debian-binary` ‖ control.tar.* ‖ data.tar.*.
- `debarchive::build_full` takes an optional `OriginSigner`; when present,
  appends `_gpgorigin` after `data.tar.*` (dpkg ignores trailing members).
- Default method remains `detach` (sibling `<pkg>.deb.sig`) for backward
  compatibility. CLI `--sign-method` overrides `signature.method`.
- Deb plugin wires the signer when method is `debsign`; `build.rs` skips
  post-build detach in that case.

### 2. `--local` + `local_payload`

- Config field `local_payload` (existence checked at build time, not parse).
- `lpt build --local` skips forge release fetch/download; builds the usual
  arch×dist matrix from the local archive or directory. Requires `version:`
  or `--version`.

### 3. Env expansion at parse

- `PackageConfig::parse_str` runs `expand_env_vars` over the YAML text:
  `${VAR}`, `${VAR:-default}`, `$$` → `$`. Missing `VAR` without a default
  fails parse. Lets CI inject `signature.key_file` / `local_payload` paths.

Schema + README updated accordingly.

## Verification

- Unit: origin-signer marker closure asserts a 4th `_gpgorigin` ar member;
  env expansion (+ missing var / `:-` default); method validation;
  `local_payload` parse; local `--dry-run` matrix; clearsign gpg round-trip
  (skips if gpg unavailable).
- Clippy `-D warnings` and fmt clean on the change set.

## Follow-up: `dpkg-sig` method + live `debsig-verify`

**Trigger:** a consumer needs `_gpgbuilder`-style clearsigned manifests, or
we want end-to-end proof against `debsig-verify` / `debsigs --verify`.

**Scope:**
1. Add `signature.method: dpkg-sig` (nfpm parity): true `gpg --clearsign` of
   the dpkg-sig control-file template → `_gpgbuilder` (or configured type).
2. Optionally rename `clearsign` → `armored_detach_sign_bytes` and reserve
   `clearsign` for the real `--clearsign` path to kill the naming debt.
3. Live test: build a debsign-signed `.deb` with a throwaway key and verify
   with `debsig-verify` / `debsigs --verify` when those tools are on PATH
   (skip gracefully otherwise), mirroring `debarchive`'s real-`dpkg-deb`
   pattern.

**Done since this note:** `signature.type` (`origin`/`maint`/`archive`) and
`contents[].packager` format filter (nfpm parity).

**Acceptance:** method enum documents both; a signed package verifies with
at least one of `debsig-verify` / `debsigs --verify` in CI when available.

**Estimate:** half-day.

## Review date

2027-02 — re-check whether anyone asked for `dpkg-sig`; if not, leave the
follow-up parked. Spot-check a real debsign-signed package against
`debsig-verify` if that tool is routinely available on the build hosts by
then.
