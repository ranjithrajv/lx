# The Linux packaging landscape, and where `lx` sits

A map of the tools around `lx`: what each one is actually for, which
stage of the "upstream software → installed on a machine" pipeline it
occupies, and where `lx` overlaps, complements, or deliberately stays
out of the way. Maturity is tracked separately, as
[technology readiness (TRL)](#technology-readiness-trl).

This is the broad survey. For field-by-field parity use
[`comparison/lx-vs-nfpm.md`](comparison/lx-vs-nfpm.md) and
[`comparison/fpm-vs-nfpm-vs-lx.md`](comparison/fpm-vs-nfpm-vs-lx.md);
for what `lx` consumes at runtime see [`tooling.md`](tooling.md).

---

## How to read this map

"Packaging tool" is an overloaded term — the things that call themselves
packagers solve different problems. Four axes separate them:

| Axis | One end | The other end |
|---|---|---|
| **Input** | You bring files (`contents:` DSL) | The tool fetches upstream (forge releases, registries, source) |
| **Output** | One format, done well | Many formats from one config |
| **Lifetime** | Build-time only (produces an artifact and exits) | Spans production → distribution → install → upgrade |
| **Scope** | A format assembler (byte-level) | A distribution system (repo, provenance, lifecycle) |

Most tools are strong on one axis and indifferent to the others. `lx`'s
bet is that the interesting unit is the **whole pipeline for upstream
software**, not any single format.

---

## The pipeline, and who occupies each stage

| Stage | What happens | Representative tools | `lx` |
|---|---|---|---|
| 1. Acquire | Find and download the upstream artifact | GitHub/GitLab/Gitea release APIs, `deb-get`, Homebrew, language PMs | ✅ 8 forge sources + 11 registries |
| 2. Compile | Build from source when no binary is published | `dpkg-buildpackage`/`debhelper`, `rpmbuild`/`mock`, `makepkg`, `abuild`, `ebuild`, CMake/Cargo/Go/Meson themselves | ✅ `build_mode: source` + build-system plugins |
| 3. Assemble | Turn a tree/files into a package | `fpm`, `nfpm`, `cargo-deb`, `cargo-generate-rpm`, `jdeb`, `checkinstall`, `alien` | ✅ deb/rpm/apk/arch/ipk + source packages, in-process |
| 4. Verify & sign | Prove what went in, sign what came out | `gpg`/`debsign`, `cosign`, `lintian`, checksum sidecars | ✅ fail-closed checksums, `--sbom`, `--cosign`, GPG |
| 5. Distribute | Make packages installable at scale | `reprepro`, `aptly`, `apt-ftparchive`, `dpkg-scanpackages`, `createrepo_c`, OBS, Cloudsmith/Gemfury/PackageCloud | ✅ `lx repo` (apt/rpm/pacman/apk/opkg, signable) + `lx publish` (build **and** index in one run) |
| 6. Consume | Install, upgrade, remove, roll back | `apt`/`dpkg`, `dnf`/`rpm`, `pacman`, `nala`, `gdebi`, `deb-get` | ✅ `lx install`/`upgrade`/`remove`/`rollback`, `lx get` — host-native deb/rpm/arch |
| 7. Migrate | Move off non-native installs | (mostly manual) | ✅ `lx go-native` (snap/flatpak/nix/`curl \| sh`) |

Almost every tool in the ecosystem lives in exactly one row. The map
below expands the crowded ones.

---

## Format assemblers — stage 3

These take files you already have and emit a package. They are the
closest peers to `lx build --from-dir`/`--from-file`.

| Tool | Language | Formats | Signature trait |
|---|---|---|---|
| [fpm](https://github.com/jordansissel/fpm) | Ruby | 15+ (deb, rpm, apk, pacman, osxpkg, freebsd, …) | Maximum breadth; converts between formats too |
| [nfpm](https://github.com/goreleaser/nfpm) | Go | deb, rpm, apk, arch, ipk, msix | Rich `contents:` DSL, per-file metadata, per-format overrides |
| [cargo-deb](https://github.com/kornelski/cargo-deb) | Rust | deb | Reads the Cargo manifest, no config file needed |
| [cargo-generate-rpm](https://github.com/cat-in-136/cargo-generate-rpm) | Rust | rpm | Cargo manifest → `.spec` → rpm |
| [jdeb](https://github.com/tcurdt/jdeb) | Java | deb | Maven/Ant integration |
| [checkinstall](https://github.com/checkinstall/checkinstall) | C | deb/rpm/Slackware | Watches `make install` and snapshots the result |
| [alien](https://en.wikipedia.org/wiki/Alien_(software)) | Perl | deb ↔ rpm ↔ tgz ↔ slp | Converts existing package files (lossy) |
| `dpkg-deb` / `rpmbuild` | C | one format each | The reference implementations `lx` replaces for *building* |

`lx` overlaps this layer at its edges (`--from-dir`/`--from-file`,
`build_mode: source`) but does not try to be a general file-driven
packager. `--format all` emits every format from one config, and `lx
publish` runs that plus the repository indexes in one command. If you want
`msix`, `osxpkg`, per-file `mode`/`owner`/`group`, or `expand: true`,
`nfpm` and `fpm` still win — see the comparison docs.

---

## Upstream-release repackagers — stages 1 + 3

A smaller, newer group: tools that assume the artifact already exists as
a *release* somewhere and close the gap to an installed package.

| Tool | Input | Output | What it does *not* do |
|---|---|---|---|
| [deb-get](https://github.com/wimpysworld/deb-get) | Upstream `.deb` URLs | Installed package | Does not build packages; HTTPS-only trust model |
| [Homebrew / Linuxbrew](https://brew.sh/) | Formulae (source or bottle) | Kegs in `/home/linuxbrew` | Not native packages; parallel, prefix-isolated |
| [makedeb](https://www.makedeb.org/) + AUR | `PKGBUILD` | `.deb` | Builds by executing recipes |
| `*2deb` converters | Language packages | `.deb` | Single ecosystem each |
| **`lx`** | Forge releases, registries, source, local files | deb/rpm/apk/arch/ipk + source packages + repo | Deliberately not `msix`/`osxpkg`/`snap`/`tar` output, and not a general distro build |

The distinguishing choice in this group is the **trust model**: deb-get
is HTTPS-only, makedeb executes recipes, and `lx` is fail-closed on
checksums with an auditable provenance array (`--summary`), plus optional
SBOM/SLSA output that none of the others emit.

---

## Consumer install fronts — stage 6

These install things on a machine. They are the destination `lx` targets
with `lx get` / `lx install`.

| Front | Native package manager? | Backed by a repo? | Notes |
|---|---|---|---|
| `apt`/`dpkg`, `dnf`/`rpm`, `pacman` | ✅ | ✅ | The distro's own; `lx` orchestrates, never replaces |
| `nala`, `gdebi` | ✅ (deb) | ✅ | Friendlier `apt` front ends |
| `deb-get` | ✅ (deb) | Upstream URLs | Upstream-first, no repo of its own |
| Homebrew / Linuxbrew | ❌ | ✅ (taps) | Prefix-isolated; bottles or source |
| [Nix](https://nixos.org/) | ❌ (own store) | ✅ (flakes/channels) | Declarative, hash-addressed, hermetic |
| [Flatpak](https://flatpak.org/) | ❌ | ✅ (remotes) | Sandboxed desktop runtime |
| [Snap](https://snapcraft.io/) | ❌ | ✅ (store) | Sandboxed, auto-updating |
| [AppImage](https://appimage.org/) | ❌ | ❌ | Single-file, no install |
| `mise`, `aqua`, `asdf`, `ubi`, `eget`, `binenv`, `webinstall.dev`, `pkgx` | ❌ | Varies | Fetch upstream release binaries/tools into a user prefix or version manager — the "dev-tool installer" crowd |
| **`lx get`** | ✅ (deb/rpm/arch) | `latest-debs` + `lx repo` (org overridable with `LX_INDEX_ORG`) | Host-native format, manifest + host-manager cross-check, rollback, musl fallback |

This is the row where `lx` most clearly differs from the "install
upstream binaries into a prefix" tools: `lx` produces **real distro
packages** that the host package manager owns, rather than a parallel
prefix or store. `lx go-native` exists precisely to convert the parallel
installs (snap, flatpak, nix, `curl | sh`) into that native form.

---

## Distribution & repositories — stage 5

Turning a pile of artifacts into something a package manager can consume.

| Tool | Ecosystem | Role |
|---|---|---|
| `dpkg-scanpackages`, `apt-ftparchive` | deb | Generate `Packages`/`Release` from a pool |
| `reprepro`, `aptly` | deb | Full repo/archive management, snapshots, signing |
| `createrepo_c` | rpm | `repomd.xml` metadata |
| [OBS](https://build.opensuse.org/) | cross | Hosted build service + repo publishing |
| Cloudsmith / Gemfury / PackageCloud | cross | Hosted artifact/repo SaaS |
| GitHub Releases, GitLab Releases | cross | De-facto upstream distribution (not a package repo) |
| **`lx repo`** | deb/rpm/pacman/apk/opkg | `Packages`/`Packages.gz`/`Release`/`InRelease` (single- or multi-suite), `repodata/`, `<repo>.db.tar.gz`, `APKINDEX.tar.gz`, opkg `Packages`; per-format index signing |
| **`lx publish`** | cross | `lx build` every format + `lx repo` each, into `<output>/<format>/` |

`lx repo` is a *functional* replacement for the generator half of this
row, not a hosted service: no snapshotting, no pool retention policy, no
CDN. For a small producer→distributor loop (`lx publish`, serve the
directory, consume with `lx get` or plain apt/dnf) it is intentionally the
whole story; for large archives you still reach for
`reprepro`/`aptly`/OBS.

---

## Distro-native builds — stage 2

The incumbent path: maintain a `debian/`, `.spec`, `PKGBUILD`, or
`APKBUILD` and drive the distro's build machinery. It produces the
highest-fidelity, policy-conformant packages, at the cost of per-distro
maintenance and a full toolchain.

| Ecosystem | Toolchain | Entry point |
|---|---|---|
| Debian/Ubuntu | `dh_make`, `debhelper`, `dpkg-buildpackage`, `lintian` | `debian/` |
| Fedora/RHEL/openSUSE | `rpmbuild`, `mock`, `fedpkg`, `koji` | `.spec` |
| Arch | `makepkg` | `PKGBUILD` |
| Alpine | `abuild`, `apk` | `APKBUILD` |
| Gentoo | Portage | `ebuild` |
| Void | `xbps-src` | `template` |

`lx` does **not** replace this layer. `lx shlibdeps` is a drop-in for
`dpkg-shlibdeps`, not for `dpkg-buildpackage`; `--source` emits a valid
`3.0 (quilt)` source package so it can *feed* a distro build, but `lx`
never runs `debian/rules` and does not maintain patch stacks. The target
is upstream-first software, not distro archive packages.

---

## Where `lx` sits

`lx` is unusual in spanning stages 1–6 for one specific path: **an
upstream project that publishes releases, delivered as a native,
upgradeable package on a machine that does not run the distro's build
infrastructure.**

```text
upstream release ──▶ verify ──▶ repack or compile ──▶ native package
      │                                                   │
      └── lx build [--format all] ────────────────────────┘
                                                          │
   lx publish ──▶ signed apt/rpm/… repo ──▶ lx get install ──▶ dpkg/rpm/pacman
```

That end-to-end span is why the comparison docs keep returning to the
same point: `fpm` and `nfpm` stop at the artifact, `deb-get` and the
version managers stop at install, `reprepro`/`aptly`/OBS stop at the
repo, and the distro-native toolchains assume you are maintaining a
distro package. Only `lx` starts from a forge URL and ends with an
installed, upgradable, natively-owned package.

### Compact matrix

Legend: ✅ does it · 🟡 partial / adjacent · — out of scope.

| Tool | Acquire | Compile | Assemble | Sign/attest | Repo | Consume | Migrate |
|---|---|---|---|---|---|---|---|
| `dpkg-buildpackage` + `debhelper` | 🟡 | ✅ | ✅ | 🟡 | — | — | — |
| `rpmbuild` / `mock` | 🟡 | ✅ | ✅ | 🟡 | — | — | — |
| `makepkg` / `abuild` | 🟡 | ✅ | ✅ | 🟡 | 🟡 | — | — |
| `fpm` | 🟡 | — | ✅ | 🟡 | — | — | — |
| `nfpm` | — | — | ✅ | ✅ | — | — | — |
| `cargo-deb` / `cargo-generate-rpm` | — | — | ✅ | 🟡 | — | — | — |
| GoReleaser | ✅ | 🟡 | ✅ | 🟡 | 🟡 | — | — |
| cargo-dist | ✅ | 🟡 | ✅ | 🟡 | — | 🟡 | — |
| `checkinstall` | — | 🟡 | ✅ | — | — | — | — |
| `alien` | — | — | 🟡 | — | — | — | — |
| `deb-get` | ✅ | — | — | 🟡 | — | ✅ | — |
| `nala` / `gdebi` | — | — | — | — | — | ✅ | — |
| `reprepro` / `aptly` / `apt-ftparchive` | — | — | — | ✅ | ✅ | — | — |
| OBS / Cloudsmith / Gemfury / PackageCloud | 🟡 | ✅ | ✅ | ✅ | ✅ | — | — |
| Homebrew / Linuxbrew | ✅ | ✅ | 🟡 | 🟡 | ✅ | ✅ | — |
| Nix | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | — |
| Flatpak / Snap / AppImage | ✅ | 🟡 | ✅ | ✅ | ✅ | ✅ | — |
| `mise` / `aqua` / `ubi` / `eget` / `asdf` / `pkgx` | ✅ | — | — | 🟡 | — | ✅ | — |
| **`lx`** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |

---

## Technology readiness (TRL)

Everything above is about **capability** — which stage of the pipeline a
tool occupies. [TRL](https://en.wikipedia.org/wiki/Technology_readiness_level)
is about **maturity**: how proven that capability is in real use. The
standard scale runs 1–9:

| TRL | Meaning |
|---|---|
| 1–3 | Basic principles → experimental proof of concept |
| 4 | Validated in a lab |
| 5 | Validated in a relevant environment |
| 6 | Demonstrated in a relevant environment |
| 7 | Prototype demonstrated in an operational environment |
| 8 | Complete and qualified; in wide operational use |
| 9 | Proven in operational use, at scale, for years |

Most of the incumbent landscape is TRL 9 — these are not experiments:

| TRL | Representative tools in this map |
|---|---|
| 9 | `dpkg`/`apt`, `rpm`/`dnf`, `pacman`/`makepkg`, `abuild`, `reprepro`/`aptly`/`apt-ftparchive`, `createrepo_c`, `fpm`, `nfpm`, `cargo-deb`, GoReleaser, Nix, Flatpak, Snap, AppImage |
| 8 | `cargo-dist`, `deb-get`, OBS and the hosted repo SaaS (Cloudsmith/Gemfury/PackageCloud) |
| 7 | Established `*2deb` converters |
| 6 | **`lx` — today** |

### Where `lx` is, and what moves it up

`lx` is **TRL 6**: the pipeline is demonstrated in a relevant environment.
Every stage runs end to end and is covered by tests — real `dpkg-deb
--info`, `dpkg-source -x`, and `lintian` accept the output, and the
producer→distributor→consumer loop works — but there is no tagged release
yet and only a few operators have run the whole loop.

| Level | What it would take for `lx` |
|---|---|
| 6 (now) | End-to-end works and is test-covered; reference tooling accepts the output; no tagged release |
| 7 | First tagged musl-static release, and an independent operator running build → publish → install/upgrade |
| 8 | Sustained use across the Debian, rpm, and Arch families; multiple independent maintainers; an upgrade/regression history |
| 9 | Years of field upgrades and a security-response record |

Readiness is not uniform across the pipeline either — the newer surfaces
are deliberately rated lower than the core:

| Dimension | TRL | Rationale |
|---|---|---|
| deb/rpm/arch packagers | 6 | Verified against the reference tools and `lintian`; pre-release |
| apk/ipk packagers | 5 | Implemented and indexable; less reference-tool verification |
| Consumer, Debian | 6 | The original path; exercised in tests and CI |
| Consumer, rpm/arch | 5 | Format-aware client added recently; no field use yet |
| Repository indexes, apt | 6 | Functional `reprepro`/`apt-ftparchive` replacement for the small loop |
| Repository indexes, rpm/pacman/apk/opkg | 5 | Newer; per-format signing wired but field-unproven |
| musl-static builds | 5 | Logic in place; the cross-architecture link is not yet proven in a release |
| `lx`'s own release pipeline | 4 | Workflow exists; no tagged release has run it end to end |

The takeaway for adoption: use the core build/verify path (TRL 6) with
confidence, but treat the newest surfaces as you would any early tool —
pin versions, verify the output yourself, and file what breaks.

---

## Overlaps and complements

`lx` is designed to sit *alongside* the host package manager and the
distro toolchain, not to swallow them. Natural pairings:

- **`lx build` + `nfpm`/`fpm`** — use `lx` for the forge-fetch/verify
  pipeline, and `nfpm`/`fpm` when you need an output format or a
  per-file control `lx` does not have (`msix`, `osxpkg`, `disown_subtree`).
- **`lx build` + `reprepro`/`aptly`/OBS** — `lx` produces the artifacts
  and `lx repo` the small repo; hand a large archive to a dedicated
  archive manager.
- **`lx --source` + `dpkg-buildpackage`** — `lx` can emit a Debian source
  package that a distro build then consumes.
- **`lx shlibdeps` alongside `dpkg-buildpackage`** — the command is a
  `dpkg-shlibdeps` drop-in usable inside a conventional Debian build.
- **`lx` + `lintian`/`gpg`/`cosign`** — verification and signing are
  consumed, never reimplemented (`lintian` deliberately, GPG and cosign
  for lack of a Rust equivalent).
- **`lx get` + the host package manager** — `lx` is an apt-like front end
  for forge software; `dpkg`/`apt`/`rpm`/`pacman` remain the backend and
  the cross-check of record. The client resolves the host's native format
  (`--format` to override) and follows `LX_INDEX_ORG`, so the same commands
  work on rpm and pacman hosts, not just Debian.

---

## Deliberate non-territory

Stated so the map does not imply ambitions that do not exist:

- **Not a full distro build system.** No `debian/rules`, no patch-stack
  maintenance, no `dpkg-buildpackage`/`rpmbuild` orchestration.
- **Not a runtime sandbox.** snap, Flatpak, and Nix solve isolation and
  hermeticity; `lx` treats them as migration *sources* (`lx go-native`),
  not as targets.
- **Not a hosted repository or CDN.** `lx repo` is a generator, not a
  service.
- **Not a universal converter.** `lx convert` handles deb↔rpm↔arch
  only, by native rebuild, and only for the shapes `lx` itself produces.
- **Not a package-manager replacement.** `dpkg`/`apt`/`rpm`/`pacman`/
  `dnf`/`zypper`/`apk`/`xbps` are consumed and orchestrated.
- **Not chasing Windows/macOS output.** `msix` and `osxpkg` are left to
  `nfpm` and `fpm`.

---

## Forces shaping the landscape

The map is not static; a few trends explain why `lx` looks the way it
does and where the adjacent tools are converging.

- **Docker-free, single-static-binary tooling.** `nfpm` displaced much of
  Ruby-`fpm` on exactly this axis, and `lx` applies the same reasoning to
  the whole build — every format in-process, no `dpkg-*`/`rpmbuild`/
  `makepkg`. `lx` now ships itself the same way (musl-static release
  binaries). See
  [`decisions/2026-08-20-docker-free-deb-build.md`](decisions/2026-08-20-docker-free-deb-build.md).
- **Supply-chain provenance.** SBOM (SPDX), SLSA-shaped provenance,
  Sigstore signing, and fail-closed checksum verification have moved from
  nice-to-have to expected. `lx` bakes them into the build rather than
  bolting them on.
- **Upstream-first distribution.** GitHub/GitLab Releases are the de-facto
  distribution channel for a huge share of software, which is why the
  "fetch the release, verify it, repack it" niche exists at all.
- **Reproducibility.** Bit-identical rebuilds (`SOURCE_DATE_EPOCH`,
  normalized metadata) are now a comparison axis; `lx` builds them into
  the archive writers.
- **Portability across distro ages.** glibc symbol versioning makes a
  binary built on a new distro unusable on an old one; musl-static builds
  (`musl: true`) sidestep it entirely — a gap neither `fpm` nor `nfpm`
  addresses.
- **Config-file convergence.** YAML config with per-format override
  blocks (`overrides:`) is now table stakes, adopted from `nfpm` and
  extended by `lx`.

---

## See also

- [`tooling.md`](tooling.md) — every external tool `lx` consumes, and
  what it replaces.
- [`comparison/lx-vs-nfpm.md`](comparison/lx-vs-nfpm.md) — living parity
  tracker against nfpm.
- [`comparison/fpm-vs-nfpm-vs-lx.md`](comparison/fpm-vs-nfpm-vs-lx.md) —
  category-by-category comparison with flags and fields.
- [`plugins.md`](plugins.md) — the eight plugin dimensions (packager,
  forge source, build system, registry source, artifact format, signer,
  dependency mapper, package index).
- [`dogfooding-roadmap.md`](dogfooding-roadmap.md) — how the index and
  dependency data feed back into the rest of `lx`.
