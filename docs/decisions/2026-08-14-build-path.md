# Build path verification: Docker .deb builder fixes

**Date:** 2026-08-14
**Context:** First real end-to-end `mkdeb build` run (against `eza-community/eza` v0.23.5) surfaced several defects in the Docker-based `.deb` builder that unit tests (which mock the GitHub layer) could not catch.

## Defects found and fixed (`src/build.rs`)

1. **Docker build context wrong.** `docker build` ran with `.` (process cwd) as context, but the Dockerfile and staged `output/` lived in a tempdir. Fixed by `current_dir(ctx.path())` and a relative `BINARY_SOURCE=binary-source` build-arg, with extracted binaries copied into the context.
2. **`/output/DEBIAN` never created.** The action's Dockerfile does `RUN mkdir -p /output/DEBIAN`; the rendered Dockerfile omitted it, so `dpkg-deb --build`'s control processing failed (`envsubst` exit 2). Added `/output/DEBIAN` to the mkdir.
3. **`docker create` missing command.** Scratch stage needs a command argument (`/`) for `docker create`; the action passes it. Added.
4. **Changelog/copyright staged in the wrong location.** They were written under `output/usr/share/doc/`, but the Dockerfile `COPY`s them from the context root (`output/changelog.Debian`, `output/copyright`). Moved to `output/` root, matching the action's `templates/output/` layout.
5. **Tempdir dropped before copy.** `deb_dest` lived inside the build-context tempdir, which was dropped when `build_deb_via_docker` returned — the source vanished before `build_one` copied it to `dist/`. `build_deb_via_docker` now returns the `TempDir` alongside the path.
6. **Debian version prefix.** Upstream tags (`v0.23.5`, `bun-v1.3.14`) violate Debian Version policy (must start with a digit). Now strips leading non-digits, mirroring the action's `sed -E 's/^[^0-9]*//'`.
7. **Deb filename convention.** Now matches the repo's existing convention (`uv_0.12.3-1+bookworm_amd64.deb`): `{package}_{debian_version}-{build}+{dist}_{arch}.deb`.
8. **Download timeouts.** GitHub release downloads occasionally timed out; added connect (30s) and total (300s) timeouts on the blocking client.

## Verification

`mkdeb build eza-debian/package.yaml --version v0.23.5 --architectures arm64,amd64 --distributions bookworm,trixie` produced four valid `.deb`s with correct per-dist `Version`/`Architecture` control fields; the extracted `eza` binary runs (`eza v0.23.5 [+git]`).

## Notes

- Debug `eprintln!` lines were removed after diagnosis.
- The Docker build output is suppressed on success (`Stdout::null()`); failures surface `bail!` messages with the docker stderr.