# Benchmark catalogue

Every benchmark worth recording for `lx` vs. the tools it can replace. This
is the backlog behind [`results.md`](results.md): the automated rows are run
by [`run.sh`](run.sh); the rest are reproducible by hand and should be added
to `run.sh` as the supporting tooling becomes available.

Levels are defined in
[`docs/drop-in/replacements.md`](../docs/drop-in/replacements.md):

- **Drop-in** — existing config/workflows point at `lx` unchanged.
- **Feature parity** — same capability set, different CLI/config surface.
- **Functional** — same artifacts, no interface compatibility.

Status key:

- `recorded` — automated and a record exists in `results.md`.
- `automated` — `run.sh` already covers it; needs a run on a capable host.
- `planned` — reproducible by hand; not yet in `run.sh`.
- `network` — needs a forge/registry/API; record with the caches warmed.
- `host tool` — needs `rpmbuild`, `makepkg`, `abuild`, … on the machine.
- `CI only` — the interface drop-in (the Action) is timed end-to-end in CI.

## 1. Drop-in (interface-compatible)

These are the only two replacements that are interface drop-ins, so they are
the highest-value records.

| ID | `lx` | Reference | Level | What's timed | Status |
|---|---|---|---|---|---|
| D1 | `action.yml` (`lx build` under GitHub Actions) | `debian-multiarch-builder` `action.yml` | Drop-in | One full action run: checkout → build → source packages → upload | CI only |
| D2 | `lx deps resolve <elf>` | `dpkg-shlibdeps` | Drop-in | Resolve one binary to versioned `Depends`, per invocation | recorded |

## 2. Packagers

Same "you supply files" job, format by format. The reference rows that are
missing tools on the recording host stay blank (`not installed`).

| ID | `lx` | Reference tool(s) | Level | What's timed | Status |
|---|---|---|---|---|---|
| P1 | `lx build --from-dir DIR --format deb` | `dpkg-deb --build`; `fpm -s dir -t deb`; `nfpm pkg -p deb` | Functional / parity | One `.deb` from a staged tree | recorded |
| P2 | `lx build --from-dir DIR --format rpm` | `rpmbuild -bb`; `fpm -s dir -t rpm`; `nfpm pkg -p rpm` | Functional / parity | One `.rpm` from a staged tree | host tool |
| P3 | `lx build --from-dir DIR --format arch` | `makepkg -f`; `fpm -s dir -t pacman`; `nfpm pkg -p archlinux` | Functional / parity | One `.pkg.tar.zst` | host tool |
| P4 | `lx build --from-dir DIR --format apk` | `abuild` / apk-tools; `fpm -s dir -t apk`; `nfpm pkg -p apk` | Functional / parity | One `.apk` | host tool |
| P5 | `lx build --from-dir DIR --format ipk` | `opkg-build`; `nfpm pkg -p ipk` | Functional / parity | One `.ipk` | host tool |
| P6 | `lx build --from-dir DIR --format all` | each reference tool, once per format | Feature parity | Every format in one run vs. the sum of the reference runs | host tool |
| P7 | `lx build --from-file FILE` | `fpm -s file` / `dpkg-deb --build` on a one-file root | Feature parity | One single-file package | planned |

## 3. Forge-release pipeline

The core value path: discover a release, download, verify, repack. These are
network-bound, so record them with `--cache-dir` / `--api-cache-dir` warm and
report cold and warm separately.

| ID | `lx` | Reference | Level | What's timed | Status |
|---|---|---|---|---|---|
| F1 | `lx build https://github.com/<owner>/<repo>` (zero config) | manual `curl` + checksum + `dpkg-deb --build` | Functional | Whole zero-config build for one arch/suite | network |
| F2 | `lx build package.yaml` (multi-arch) | `debian-multiarch-builder` action | Drop-in | Same release, all architectures, all suites | network |
| F3 | `lx build … --cache-dir D` reuse | re-download from the forge | — | Warm-cache rebuild vs. cold build | network |
| F4 | `lx build … --pinned-metadata release-metadata.json` | manual `sha256sum -c` against a sidecar | Functional | Verification step overhead on the same asset | planned |

## 4. Source packages (`--source`)

Emit the source artifact a distro would consume, without running a full
distro build stack.

| ID | `lx` | Reference | Level | What's timed | Status |
|---|---|---|---|---|---|
| S1 | `lx build --source` (deb) | `dpkg-source -b`; `dpkg-buildpackage -S` | Functional | `.dsc` + `.orig.tar.xz` + `.debian.tar.xz` emission | planned |
| S2 | `lx build --source --format rpm` | `rpmbuild -bs` | Functional | `.src.rpm` emission | host tool |
| S3 | `lx build --source --format arch` | `makepkg --printsrcinfo` / `makepkg -S` | Functional | PKGBUILD emission | host tool |
| S4 | `lx build --source --build-system <sys>` | the toolchain invoked directly (`cmake`, `cargo`, `go`, `meson`, `autotools`, `make`) | Functional | Config → compile → stage → package for one system | planned |

Compile time dominates S4; record it separately from the packaging rows so
it does not distort the packager comparison.

## 5. Format conversion

| ID | `lx` | Reference | Level | What's timed | Status |
|---|---|---|---|---|---|
| C1 | `lx convert pkg.deb --to rpm` | `fpm -s deb -t rpm` | Functional | One native rebuild | host tool |
| C2 | `lx convert pkg.rpm --to deb` | `fpm -s rpm -t deb`; `alien --to-deb` | Functional | One native rebuild | host tool |
| C3 | `lx convert pkg.pkg.tar.zst --to deb` | `fpm -s pacman -t deb` | Functional | One native rebuild | host tool |
| C4 | `lx convert pkg.deb --to arch` | `fpm -s deb -t pacman` | Functional | One native rebuild | host tool |

## 6. Repository indexing

| ID | `lx` | Reference | Level | What's timed | Status |
|---|---|---|---|---|---|
| R1 | `lx repo DIR` (apt) | `dpkg-scanpackages`; `apt-ftparchive packages` | Functional | Index a directory of `.deb`s | recorded |
| R2 | `lx repo DIR --multi-suite` | `reprepro`; `apt-ftparchive release` | Functional | Multi-suite `dists/` + top-level `Release` | planned |
| R3 | `lx repo DIR --format rpm` | `createrepo_c` | Functional | `repodata/` emission | host tool |
| R4 | `lx repo DIR --format arch` | `repo-add` | Functional | `<repo>.db.tar.gz` emission | host tool |
| R5 | `lx repo DIR --format apk` | `apk index` | Functional | `APKINDEX.tar.gz` emission | host tool |
| R6 | `lx repo DIR --format ipk` | `opkg-make-index` | Functional | opkg `Packages` emission | host tool |
| R7 | `lx publish` | the Action + `reprepro`/`createrepo_c` in CI | Functional | Build every format and index it in one run | network |

## 7. Signing

| ID | `lx` | Reference | Level | What's timed | Status |
|---|---|---|---|---|---|
| G1 | `lx build --sign-key K --sign-method debsign` | `debsigs --sign`; `debsign` | Functional | Embed `_gpgorigin` in one `.deb` | planned |
| G2 | `lx build --sign-key K` (deb, detach) | `gpg --detach-sign` | Functional | Sibling `.deb.sig` emission | planned |
| G3 | `lx build --format rpm --sign-key K` | `rpm --addsign` | Functional | Native RPM header signature | host tool |
| G4 | `lx build --format apk --sign-key K` (in-process RSA) | `abuild-sign`; `openssl dgst -sha1 -sign` | Functional | `.SIGN.RSA.<key>` emission | host tool |
| G5 | `lx repo DIR --sign-key K` (apt `InRelease`) | `gpg --clearsign`; `apt-ftparchive release` + `gpg` | Functional | Clearsigned `InRelease` | planned |

## 8. Dependency / ELF analysis

| ID | `lx` | Reference | Level | What's timed | Status |
|---|---|---|---|---|---|
| E1 | `lx deps resolve <elf>` | `dpkg-shlibdeps` | Drop-in | See D2 | recorded |
| E2 | `lx deps scan [config]` | `readelf -d`; `ldd`; `objdump -p` | Functional | Report a binary's shared-library needs | planned |
| E3 | `lx build … --bindep` | `dpkg-shlibdeps` run inside a build | Functional | ELF dep inference as part of packaging | planned |

## 9. Consumer client (deb-get / apt front-end parity)

| ID | `lx` | Reference | Level | What's timed | Status |
|---|---|---|---|---|---|
| K1 | `lx search <pattern>` | `apt-cache search`; `deb-get search` | Feature parity | Full-text search over the org + installed metadata | planned |
| K2 | `lx show <pkg>` | `apt show`; `dpkg -s`; `deb-get show` | Feature parity | One package's metadata | planned |
| K3 | `lx list` | `dpkg -l`; `deb-get list` | Feature parity | List `lx`-managed packages | planned |
| K4 | `lx install <pkg>` | `deb-get install` | Feature parity | Resolve → download → verify → install | network |
| K5 | `lx update [pkg]` | `deb-get update`; `apt update` | Feature parity | Freshness check, no install | network |
| K6 | `lx upgrade [pkg]` | `deb-get upgrade`; `apt upgrade` | Feature parity | Upgrade installed packages | network |
| K7 | `lx remove <pkg>` | `apt remove`; `dpkg -r` | Feature parity | Remove one package | planned |
| K8 | `lx rollback <pkg>` / `lx install --reinstall` | (no direct equivalent) | — | Record only to show cost of the feature | planned |

K4–K6 are dominated by network and the host package manager, not `lx`; report
them as "orchestration overhead" with the download/install time subtracted.

## 10. Index, migration, and import

| ID | `lx` | Reference | Level | What's timed | Status |
|---|---|---|---|---|---|
| I1 | `lx index search <pkg>` | AUR RPC; repology API query | Feature parity | One fan-out query across enabled indexes | network |
| I2 | `lx index update` | `git pull` of the recipe index | Feature parity | Refresh cached index data | network |
| I3 | `lx go-native --dry-run` | manual `snap list` / `flatpak list` / `nix profile list` | Functional | Detect + map installed non-native software | planned |
| I4 | `lx init --from-aur <pkg>` | `makedeb`; reading the `PKGBUILD` by hand | Feature parity | Convert an AUR `PKGBUILD` to `package.yaml` | planned |

## Promoting a row to automated

1. Move the row's status from `planned`/`network`/`host tool` to `automated`
   in this file.
2. Add a `bench_*` function to [`run.sh`](run.sh): stage the input, gate on
   the reference tool being present (`run_one` already prints
   `not installed`), time it, emit the row.
3. Run it, and append the report to [`results.md`](results.md); set the row's
   status to `recorded`.

Keep the recorded set honest: an automated row that cannot run on the current
host should say so explicitly rather than silently disappear.
