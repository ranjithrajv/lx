# Broadening adoptions (deb-get / makedeb / MPR lessons)

Date: 2026-09-11. Context: feature-parity review against
`debian-multiarch-builder`, `deb-get`, and `makedeb` concluded lpt leads at
*producing* packages but borrows nothing from the consumer/index side. This
pass adopts eight ideas, deliberately bounded so each ships independently.

## What was adopted (and the bound)

1. **Recipe index surface** — `lpt search --local` over the embedded
   templates. Bound: no server, no new index format; the shared fleet
   configs remain the seed corpus for a future community index.
2. **Consumer UX** — `lpt show`, `lpt reinstall`, `upgrade --owned-only`.
   All three reuse manifest + dpkg as sources of truth; `reinstall` is a
   thin wrapper over `install --reinstall` with recorded coordinates.
3. **Generic build hook** — `prebuild_steps` + `build_system: custom` with
   `build_commands`/`install_commands` ($DESTDIR). Bound: ordered `sh -c`
   strings, not a second language; cmake stays the blessed path.
4. **Prebuilt repo** — `lpt repo` writes `Packages`/`Release`/`InRelease`
   with native control-tar parsing. Bound: single-suite layout, no
   snapshotting, no CDN invalidation.
5. **SBOM + SLSA** — `--sbom` emits SPDX 2.3 + SLSA-v1-shaped provenance
   from existing provenance receipts. Bound: unsigned statements; signing
   them is future work (the `.sig` machinery already exists).
6. **AUR importer** — `lpt init --from-aur` renders *comments*, never code:
   recipes stay declarative and reviewable. Caught live: AUR RPC uses
   capitalized field names (`Name`, `Depends`, …).
7. **Thin client** — `lpt-get` second binary, consumer commands only.
   Bound: same manifest/org, no protocol work.
8. **Sandbox** — `--sandbox` wraps source-compile steps in
   `unshare -n`, best-effort with loud fallback. Binary repacks ignore it
   (they execute nothing). Network-hungry builds should skip it.

## `apt search` parity in `lpt search`

Full-text match covers name + org blurb + installed dpkg long
description + (local) template body; hits show installed and
manifest-recorded candidate versions; exact-name matches sort first.
Per-hit latest-release lookup was rejected: one API call per hit burns
rate limit for a list view — the manifest version is the candidate.

## What was NOT adopted (and why)

- makedeb's "it's all just bash" recipes — arbitrary code in recipes
  makes an index unreviewable; lpt stays declarative with a fenced hook.
- deb-get's HTTPS-only trust model — lpt's fail-closed verification is
  the advantage; not diluted for recipe count.
- A second recipe language — PKGBUILD import, not PKGBUILD support.
