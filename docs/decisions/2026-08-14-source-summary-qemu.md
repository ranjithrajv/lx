# Action parity: source packages, build summary, QEMU detection

**Date:** 2026-08-14
**Context:** Closing remaining parity gaps between the `mkdeb` CLI and the `debian-multiarch-builder` GitHub Action: Debian source packages, `build-summary.json`, and QEMU/binfmt diagnostics. This complements the earlier lintian, control-file, and build-path decisions.

## Source packages (`src/source.rs`, `--source`)

Mirrors `src/lib/source-package.sh` + `templates/source/*`:

- One source package per **distribution** (architecture-independent), named `{pkg}_{ver}-{build}+{dist}`, format **3.0 (quilt)**: `.dsc`, `.debian.tar.xz`, plus a single shared `.orig.tar.xz`.
- The reference `.deb` (prefer `amd64`, else first match for the dist) is unpacked via `dpkg-deb -x` to form the upstream tree; `DEBIAN/` and `usr/share/doc/` are stripped so the shared `.orig.tar.xz` is payload-only.
- `dpkg-deb`, `tar`, and `dpkg-source` run in the `mkdeb-src` container image (debian:bookworm + dpkg-dev + xz-utils) because the host is non-Debian. **Key fix:** the container mounts the *real* host paths (`-v <host workdir>:/work -v <host out_dir>:/out`), and extraction + `DEBIAN/`/`usr/share/doc` cleanup run inside the same container invocation — a naive host-side `remove_dir_all` failed because the container writes root-owned files the host user cannot delete.
- Best-effort: a per-dist failure is logged (`⚠️ source package for <dist> failed`) without failing the binary builds, mirroring the action.

## Build summary (`src/summary.rs`, `--summary`)

Mirrors `src/lib/summary.sh` `generate_build_summary` with a matching schema: `package/version/build_version/full_version/github_repo`, `architectures`/`distributions` arrays, `total_packages`/`total_size_bytes`/`total_size_human`, `build_duration_seconds`/`build_start`/`build_end` (RFC 3339 UTC, computed with Howard Hinnant's civil-from-days — no `chrono` dependency), `parallel_builds`/`max_parallel`, `packages` (name+size), `lintian`/`telemetry` (`{}`; lintian runs as a separate command), and `success_rate` (`total_packages * 100 / attempted`).

## QEMU/binfmt detection (`src/build.rs`)

- `host_arch()` maps `uname -m` to a Debian arch name; `check_qemu_for(arch)` warns once per foreign architecture when no `qemu-*` handler is registered under `/proc/sys/fs/binfmt_misc/`, pointing at `tonistiigi/binfmt` (the local analogue of the action's `docker/setup-qemu-action` + binfmt diagnostics).
- Worded as "may fail": `dpkg-deb` only *packages* the copied ELF (never executes it), so foreign-arch builds can succeed without QEMU on this pipeline.

## Verification

- `cargo clippy --all-targets`: 0 warnings. `cargo test`: 28 passed (now 33 after optimize + cache additions). `cargo fmt --check`: clean.
- Full e2e: `--distributions bookworm,trixie --architectures amd64,arm64,armhf --source --summary --max-parallel 3` built 6 `.deb`s (25s), 2 `.dsc`+`.debian.tar.xz`, shared `.orig.tar.xz`, and a valid `build-summary.json` (success_rate 100). Foreign-arch QEMU warnings printed, builds still succeeded.

## Notes

- License SPDX flows into `debian/copyright` in the source tree; `NOASSERTION` fallback when no license was detectable.
- Clippy type-complexity on the cache test closure resolved with a `DownloadFn` type alias.

## Follow-up: dynamic parallelism, resource pooling, API cache

Closing the remaining functional gaps vs the action's plumbing layer:

- **`src/optimize.rs`** mirrors `ci-optimization.sh` (`optimal_parallel_jobs`: 2 GB RAM, 1 core, 5 GB disk per job; most restrictive wins; capped at 8; CI overhead reserves one job) and `resource-pool.sh` (`apply_graceful_degradation`: warn + clamp when the requested level exceeds capacity). Detection reads `nproc`/`/proc/cpuinfo`, `/proc/meminfo`/`free`, and `statvfs` (via the existing `libc` dep) for free disk. `--max-parallel` now defaults to **0 = auto**; `resolve_max_parallel` is pure (tests stay deterministic), `effective_max_parallel` warns and applies.
- **`src/github.rs` API cache** mirrors `github-api.sh` `/tmp/github_api_cache`: `release_by_tag`, `latest_release`, `list_releases`, and `repo_license` responses are cached as JSON for 300 s via `--api-cache-dir`. Domain types (`Release`, `Asset`, `RepoLicense`) gained `Serialize`/`Deserialize`. Best-effort (a missing/full cache dir never fails a build).
- **Parallel container-name race (bug found by e2e):** `docker create` used the process pid as the container name, so parallel arch workers raced on the same name (`Conflict. The container name ... is already in use`). Now unique per `(pid, dist, arch)`: `mkdeb-<pid>-<dist>-<arch>`.

Verified: `--max-parallel 4 --distributions bookworm,trixie --architectures amd64,arm64,armhf --source --summary` builds all 6 `.deb`s concurrently (4s with warm caches) + 2 source packages + summary; `--max-parallel 99` is clamped to the resource optimum (6 on this 16-core host) with a warning; a second run with `--api-cache-dir` serves cached release/license JSON (cache mtimes unchanged). `cargo clippy --all-targets`: 0 warnings; `cargo test`: 33 passed; `cargo fmt --check`: clean.

## Follow-up: telemetry, live progress, viral badge

The last cosmetic gaps, mirroring `telemetry.sh`, `progress.sh`, and `generate_viral_badge`:

- **`src/telemetry.rs` (`--telemetry`)** mirrors the action's `TELEMETRY_ENABLED` (default false, no-op when off): `init_telemetry` seeds `.telemetry/metrics.json`; `record_build_stage(_complete)` appends to `stages.log`; `record_build_failure` appends to `failures.log`; `finalize_telemetry` adds `build_end_time`/`build_duration`/`build_completed`; `summary_json()` is `get_telemetry_summary`, and `build-summary.json` now carries the real `telemetry` object (was a `{}` stub). Wired per-arch into the parallel workers.
- **`src/progress.rs` (`--progress`)** mirrors `progress.sh`: a shared state file (`/tmp/build_progress.json`) with per-arch status + counters (`total_archs`, `completed`/`failed`/`running`/`pending`, `architectures`), updated under a mutex across parallel workers, plus an inline `\r [████░░] NN% (n/N) ...` bar that only renders on a TTY (the action's dashboard is interactive-only too). Cleaned up after the build.
- **Viral badge** in `src/summary.rs` mirrors `generate_viral_badge`: after saving the summary, prints the shields.io markdown block with success rate, build time, package count, and arch count.

Verified: `--telemetry` produces `.telemetry/{metrics.json,stages.log}` (build_initialization → per-arch → build_completion) and populates `summary.telemetry`; `--progress` writes and then removes `/tmp/build_progress.json`; `--summary` prints the viral badge. `cargo test`: 37 passed (4 new: 2 telemetry + 2 progress). Clippy 0 new warnings (pre-existing `large size difference between variants` in cli.rs remains, unrelated).