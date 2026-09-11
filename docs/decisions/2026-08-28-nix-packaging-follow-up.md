# Follow-up: Nix packaging ideas worth adopting

**Date:** 2026-08-28
**Context:** Compared `lpt` with Nix packaging (derivations / `mkDerivation`) to
see which Nix strengths map onto our model — repackaging forge release binaries
into real `.deb`/`.rpm`/Arch packages for apt/dnf/pacman users, not rebuilding
from source into `/nix/store`. This note records what to copy, what to skip, and
a phased follow-up plan.

## What we already overlap on

- Declarative config (`package.yaml`) and multi-arch matrices
- Fail-closed integrity (`--pinned-metadata`, sidecar checksums,
  `--allow-unverified` escape hatch)
- Content-addressed download cache (`lib/cache.rs`, SHA-256 key + TTL)
- Reproducible package bytes (`SOURCE_DATE_EPOCH`, sorted archives,
  release-time metadata — see `2026-08-20-reproducible-builds.md`)
- Dependency introspection (`lpt scan-deps` for ELF `DT_NEEDED`)

## Copy from Nix packaging (ranked)

### Phase 1 — lock + cache (highest ROI)

1. **Lockfile (`package.lock`)** — promote `--pinned-metadata` /
   `release-metadata.json` to a first-class, committable lock: exact tag, asset
   URL, SHA-256, forge, publish time per arch. CI builds refuse drift unless
   `--update-lock` (or similar) is passed. Mirrors Nix fixed-output / flake
   lock semantics without adopting flakes.

2. **Input-keyed artifact store** — extend the download cache model to *built*
   packages: key = hash(config + asset digest + format + dist + arch). Same
   inputs → reuse local artifact; foundation for a substituter later.

### Phase 2 — install UX

3. **Install generations + rollback** — today's `installed.json`
   (`lib/manifest.rs`) is a single snapshot. Keep a generation history
   (version, artifact path or hash, timestamp) so `lpt rollback` can
   `dpkg -i` the previous generation. Nix profiles, adapted for dpkg.

4. **Closure / `why-depends` report** — build on `scan-deps`: show which
   binary needs which shared lib, how that maps to declared `depends:`, and
   flag undeclared runtime deps. Reviewer-friendly; no full Nix closure graph.

### Phase 3 — recipe ergonomics

5. **Overrides / overlays** — thin layer over a base `package.yaml` (bump
   version, swap `depends`, override `release_pattern`) without forking
   templates. Org-wide base + local delta.

6. **Structured `meta:` block** — homepage, license, maintainers, description
   as one schema section feeding control files, `copyright`, and future
   listings (`lpt list --json`).

7. **Optional `check` / `installCheck`** — post-build smoke (`--help`, ELF
   arch match, optional user command) before publishing. Nix `checkPhase`
   lite.

8. **Multiple outputs (optional)** — split fat upstream trees into
   `pkg-bin` / `pkg-doc` / `pkg-man` when bundle mode ships more than the
   runtime needs.

### Phase 4 — remote cache

9. **Substituters** — if artifact hash `H` exists at
   `https://cache.example/<H>`, download instead of rebuild. Natural
   extension once Phase 1's artifact store exists.

## Skip (wrong fit for lpt)

| Nix idea | Why not |
|---|---|
| `/nix/store` + FHS rewrite | Fights apt/dpkg; out of scope |
| From-source `mkDerivation` as default | We wrap release binaries |
| patchelf / store-relative RPATH | Only if we ship non-FHS bundles at scale |
| Nix language / flakes as config | `package.yaml` + lockfile is enough |
| Channel model | Prefer explicit lockfiles over rolling channels |
| NixOS modules | Unrelated |

## Pros / cons reminder (positioning)

**lpt wins when:** upstream ships Linux binaries; consumers use apt/dnf/pacman;
you want real distro packages fast without Docker or a Nix install.

**Nix wins when:** hermetic from-source builds, exact closures, multi-version
coexistence, NixOS, or dev-env reproducibility matter more than shipping
`.deb`s.

They are complementary, not competitors — Nix can `fetchurl` the same release
tarballs; `lpt` serves users who stay on traditional package managers.

## Follow-up tasks

| # | Task | Trigger | Acceptance | Status |
|---|---|---|---|---|
| 1 | Design `package.lock` schema + `lpt lock` / `--locked` build | Any CI consumer asks for drift-proof builds | Lock committed; build fails on hash/tag mismatch without `--update-lock` | **Done** — `lib/lock.rs`, `build --update-lock`, verified in the download step (`VerifyMethod::Locked`) |
| 2 | Artifact store keyed by input hash | Phase 1 lock exists | Second identical build hits cache; `build-summary.json` records cache hit | **Done** — `lib/cache.rs::ArtifactCache`, `build --artifact-cache-dir` |
| 3 | `installed.json` generations + `lpt rollback` | Install/upgrade users report "can't undo" | Rollback restores prior dpkg version from generation history | **Done** — `Manifest::packages` now `Vec<PackageEntry>` per package; `lpt rollback` |
| 4 | `lpt why-depends` (or `scan-deps --explain`) | Reviewers ask "why libatomic1?" | Report ties ELF needs → declared `depends:` → owning package | **Done** — `lpt scan-deps --explain` |
| 5 | `package.yaml` overlays | Orgs fork the same template repeatedly | Base + overlay merges; modern keys win | **Done** — `PackageConfig::apply_overlay`, `build --overlay` / `validate --overlay` |
| 6 | `meta:` schema | Control-file duplication across tools | Single block renders description, copyright, homepage | Not started |
| 7 | `--check` post-build hook | First bad arch binary ships undetected | Optional command runs; non-zero exit fails build | Not started |
| 8 | Substituter URL config | Self-hosted or org binary cache wanted | `--substituter` or config field; miss falls back to local build | Not started |

**Suggested order:** 1 → 2 → 3 → 4 → (5–7 as needed) → 8. Items 1–5 shipped 2026-08-28; artifact-cache
keying additionally folds in build/sign/lintian flags (beyond the original config+asset+format+dist+arch
spec) so a cache hit is only served when every byte-affecting input matches, including detached-signature
jobs which are excluded from caching entirely since the cache doesn't track the sibling `.sig`.

**Estimate:** Phase 1 ~2–3 days; Phase 2 ~2 days; Phase 3 items are independent
half-days each; Phase 4 depends on hosting.

## Review date

2027-02 — re-check whether Phase 1 (lockfile) was started; if not, and no CI
consumer has asked for drift-proof builds, leave parked. Revisit substituters
only after local artifact store ships.
