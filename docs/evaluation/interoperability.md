# Evaluation: interoperability

Part of the [lx docs](../README.md). Companion evaluation:
[Composability](composability.md).

`lx` sits between two systems that already have their own conventions: an
upstream project publishing releases, and a host machine whose package
manager owns the filesystem. Interoperability is whether `lx` can exchange
data with both — consume what they produce, emit what they accept — without
either side needing to know `lx` exists.

This evaluation fixes the rubric first, then walks the repository for
evidence and records the gaps.

---

## Rubric

| # | Criterion | "Pass" means | Verdict |
|---|---|---|---|
| I1 | **Outbound artifacts** | Every package format `lx` emits is accepted by that format's reference implementation, not just by `lx`'s own reader | ✅ |
| I2 | **Outbound indexes** | Every repository index `lx` writes is consumed by the ecosystem's native client | ✅ |
| I3 | **Inbound acquisition** | `lx` can source software from the channels upstreams actually use, with no per-source core changes | ✅ |
| I4 | **Inbound import** | `lx` can read packages and recipes other tools produced (format conversion, foreign package managers) | 🟡 |
| I5 | **Interface compatibility** | At least the surfaces advertised as "drop-in" are input/output-compatible without edits | ✅ |
| I6 | **Co-existence** | `lx` orchestrates the host package manager and uses it as the source of truth, rather than shadowing it | ✅ |
| I7 | **Standards emission** | Metadata and provenance are emitted in consumer-ready standard formats | 🟡 |
| I8 | **Failure honesty** | Where interoperability is incomplete, `lx` fails closed and says so instead of silently guessing | ✅ |

Verdicts follow the [evaluation scale](README.md#method).

---

## I1 — Outbound artifacts accepted by reference tools

The strongest form of interoperability is being checked by the tool you
claim to interoperate with. `lx` builds every archive in-process — no
`dpkg-deb`, `rpmbuild`, `makepkg`, or `abuild` — so the risk is exactly that
it is only self-consistent. The suite guards against that by feeding the
output to the real consumers where they exist.

| Format | Reference consumer | Evidence |
|---|---|---|
| `.deb` | `dpkg-deb` | Standing test `tests/debarchive.rs::real_dpkg_deb_accepts_the_archive` runs real `dpkg-deb --info`/`--contents` and asserts the control fields and file modes; `tests/plugins.rs` spot-checks `--info` too |
| `.rpm` | `dnf` / `rpm` | Standing test `tests/verify_clients.rs::dnf_accepts_generated_repodata` (gated on `LX_DNF`) makes `dnf makecache` consume a repo built from an `lx` `.rpm`; magic `ED AB EE DB` is checked unguarded |
| `.pkg.tar.zst` | `pacman` | Standing test `tests/verify_clients.rs::pacman_accepts_generated_db` runs real `pacman -Sy`/`-Si` and asserts the package is exposed |
| `.apk` | `apk-tools` | Standing test `tests/plugins_repo_signing.rs::apk_verifies_with_real_apk_static` (gated on `LX_APK_STATIC`) verifies a signed `lx` `.apk`; the CHANGELOG records `apk update` succeeding from an `lx`-generated signed repo |
| `.ipk` | `opkg` | Standing test `tests/verify_clients.rs::opkg_accepts_generated_index` (gated on `LX_OPKG`) runs `opkg update` against `lx`'s `Packages.gz` |
| source `.dsc` | `dpkg-source` | Field order/content matched field-for-field against a real `dpkg-source -b` run, then reconstructed with real `dpkg-source -x` — recorded once in `docs/decisions/2026-08-20-docker-free-lintian-source.md`; the standing test `tests/source.rs::dsc_lists_orig_before_debian_and_matches_dpkg_source_field_order` pins the format |
| `.deb` correctness | `lintian` | `--lintian` shells out to the real binary (`lib/lintian.rs`); `tests/lintian.rs` pins the parser against captured real output |

The gated tests skip cleanly when a client is absent, so the suite stays
hermetic — but that also means a regression is only caught on a host with
the consumer installed. The eval treats I1 as ✅ because the reference
consumers exist and accept the output; it does not claim every row runs on
every machine.

Reproducibility is part of artifact interoperability: a package that only
rebuilds identically on one machine is harder to verify elsewhere.
`tests/debarchive.rs::same_input_produces_byte_identical_output` pins that,
and `SOURCE_DATE_EPOCH` / normalized `mtime`/`uid`/`gid` are used throughout
(`docs/guides/reproducible-builds.md`).

## I2 — Outbound repository indexes

`lx repo` / `lx publish` turn artifacts into something a package manager
subscribes to, per format:

| Format | Index written | Native consumer |
|---|---|---|
| apt | `Packages`, `Packages.gz`, `Release`, `InRelease`, `Release.gpg` | `apt` / `dpkg` |
| rpm | `repodata/` (`repomd.xml` + `repomd.xml.asc`) | `dnf` |
| pacman | `<repo>.db.tar.gz` + `.sig` | `pacman` |
| apk | `APKINDEX.tar.gz` + prepended `.SIGN.RSA.<key>` | `apk` |
| opkg | `Packages` + `Packages.sig` | `opkg` |

Index **signing** is wired per format (`lib/plugins/package_index/*`,
`tests/plugins_repo_signing.rs`), and the `LX_*`-gated client tests above
exercise the apt/opkg/pacman/rpm consumers against generated indexes.

## I3 — Inbound acquisition

`lx build` can start from the channels upstreams actually publish to,
without editing the core pipeline for a new one:

- **9 forge sources** — GitHub, GitLab, Gitea, Forgejo, Bitbucket, Gerrit,
  Gitee, SourceForge, and an explicit `custom` URL template
  (`lib/plugins/forge/`). Zero-config `lx build <url>` sniffs the host.
- **11 language registries** — npm, pip, gem, cargo, go, hex, dart, nuget,
  maven, composer, cpan (`lib/plugins/registry/`).
- **6 artifact formats** — `tar.gz`, `tar.xz`, `tar.zst`, `tar`, `zip`,
  `raw` (`lib/plugins/artifact/`), each fetched through a probe-agnostic
  `RawGetter` so checksum-sidecar verification works for any forge
  (`lib/checksum.rs`).
- **Provider-published checksums** — GitHub/Gitea asset `digest` and
  SourceForge feed `md5` are verified inline (CHANGELOG, "inline checksum
  verification").
- **Host truth** — `lx deps scan`/`--bindep` parse `DT_NEEDED` natively and
  resolve names through `dpkg -S` / `rpm -q --whatprovides` /
  `pacman -Qo`; `lx deps resolve` reads the dpkg `symbols`/`shlibs`
  databases (`lib/elfdeps.rs`, `lib/shlibdeps.rs`, `lib/bindep.rs`).
- **Host detection** — OS, codename, architecture, package manager, and
  native format from `/etc/os-release`, `uname`, and a `PATH` probe
  (`lib/info.rs`, `docs/reference/host-detection.md`).

## I4 — Inbound import (partial)

`lx convert` reads a built `.deb`, `.rpm`, or `.pkg.tar.zst`, extracts the
control fields and install tree, and rebuilds natively in the target format
(`lib/convert.rs`). All three readers are **in-process**: deb and Arch via
`debarchive`/`tar`, RPM via the `rpm` crate (control fields, scriptlets, and
payload) — no `rpm`, `rpm2cpio`, or `cpio` on `PATH`. `lx init --from-aur`
turns an AUR `PKGBUILD` into a starter `package.yaml` (shell becomes
comments, never executed), and legacy `debian-multiarch-builder`
`package.yaml` keys load as-is.

This is 🟡 because the import surface is deliberately narrower than a
general converter:

- `lx convert` is **deb↔rpm↔arch only** — no `apk`/`ipk`, and no `msix`,
  `osxpkg`, `freebsd`, or `snap`.
- It rebuilds the target's model rather than copying raw bytes, carrying
  relation fields (`Provides`/`Recommends`/`Suggests`/`Conflicts`/
  `Replaces`/`Breaks`/`Pre-Depends`), `Section`/`Priority`, epoch, and
  conffile/`%config` semantics, and rewriting dependency syntax and names
  across formats. Anything the target model doesn't express is still
  dropped: capabilities, xattrs, RPM triggers, debconf, and signatures.
- It converts **packages `lx` itself can produce**, not arbitrary fpm
  outputs with richer per-file metadata.

## I5 — Interface compatibility

Two drop-in surfaces carry the "replace without edits" claim:

- **`action.yml`** is a drop-in for `debian-multiarch-builder`'s action:
  same input names (`config-file`, `version`, `build-version`,
  `architecture`, `max-parallel`, `lintian-check`, `telemetry-enabled`,
  `save-baseline`, `pinned-metadata`, `output-dir`, `lx-version`) and output
  names (`packages`, `source-packages`, `summary-path`). `lx migrate lpt`
  rewrites existing workflows (`docs/guides/github-action.md`).
- **`lx deps resolve`** is a command drop-in for `dpkg-shlibdeps`: same job,
  versioned relations, fail-closed without `--ignore-missing-info`
  (`lib/shlibdeps.rs`).

The consumer subcommands (`lx get install/upgrade/update/remove/list/show/
search/rollback`) track deb-get parity, and old command names keep working
as hidden aliases (`docs/reference/commands.md`). `overrides:`,
`contents[].packager`, per-format scripts, and other fields were adopted
from nfpm for **config** parity — that is feature parity, not interface
compatibility, and is not counted here.

## I6 — Co-existence with the host

`lx` is an apt-like front end for forge software, not an installer backend.
The host package manager stays the owner:

- `lx install`/`upgrade`/`remove` dispatch on the detected host format and
  call `dpkg`/`apt`/`rpm`/`pacman` rather than writing the database.
- The local `installed.json` manifest is **cross-checked** against
  `dpkg`/`rpm`/`pacman`, so `lx` is never the sole source of truth
  (`lib/consumer.rs`, `lib/manifest.rs`).
- The consumer resolves the host's native format and honors
  `LX_INDEX_ORG`, so the same commands work on rpm and pacman hosts, not
  just Debian (`docs/reference/host-detection.md`).
- `musl: true` emits static binaries with no glibc symbol-version
  dependency, sidestepping the "built on new Ubuntu, unusable on old"
  incompatibility (`docs/decisions/2026-09-11-musl-static-builds.md`).
- `lx migrate native` targets the host manager as the destination for
  snap/flatpak/nix/`curl | sh` installs.

## I7 — Standards emission (partial)

| Standard / format | `lx` surface | Status |
|---|---|---|
| SPDX 2.3 SBOM | `lx build --sbom` → `<pkg>_<ver>.spdx.json` | ✅ emitted; `tests/sbom.rs::emit_writes_spdx_and_slsa` |
| SLSA v1-shaped provenance | `lx build --sbom` → `<pkg>_<ver>.slsa.json` (builder, materials, outputs with digests) | ✅ emitted |
| Checksum sidecars | `.sha256`/`.sha512` per artifact, plus `--pinned-metadata` | ✅ |
| Sigstore | `--cosign` keyless signing via the `cosign` binary | ✅ emit-side only |
| Consuming Sigstore / GitHub attestations | — | ❌ not consumed; an upstream publishing only an attestation fails closed |
| `dpkg-sig` (`_gpgbuilder`) | — | ❌ only `debsign` (`_gpgorigin`) and detached `.sig` |

The verdict is 🟡: `lx` **emits** provenance standards that none of `fpm`,
`nfpm`, `deb-get`, or `makedeb` do, but it does not yet **consume** upstream
attestations, which is the other half of standards interoperability.

## I8 — Failure honesty

Where interoperability is incomplete, the design is to stop rather than
guess:

- Builds refuse an unverified asset unless `--allow-unverified`/`--no-verify`
  is passed explicitly (`lib/debs.rs`, `lib/checksum.rs`).
- An upstream with only an attestation (no checksum sidecar) **fails
  closed** rather than being trusted.
- `lx deps resolve` errors on an unowned library unless
  `--ignore-missing-info`.
- `rpm.defines` is accepted but not applied by the in-process builder, and
  `lx` **warns** instead of dropping it silently
  (`docs/architecture/replacements.md`).

## Where interoperability is partial

Collected, so the ✅ rows above are not read as universal:

- **`dpkg-shlibdeps` parity is not total.** Virtual `Provides`, multiarch
  `pkg:arch` qualifiers, `debian/shlibs.local`, and full
  `-e`/`-T`/`-O`/`-d`/`-p`/`-l` parity are not covered
  (`docs/architecture/replacements.md` §2.6).
- **Conversion is a subset.** deb↔rpm↔arch only, with in-process readers for
  all three, but the target model can't express capabilities, xattrs, RPM
  triggers, debconf, or signatures, so those are dropped.
- **No upstream attestation consumption** (see I7).
- **`dpkg-sig` is not implemented**, and a live `debsig-verify` test does
  not exist.
- **Windows/macOS output is signed in-process** (`msix` p7x, xar
  `Signature`) from a PEM key + certificate; notarization is a separate
  Apple service and `.pkg` has no `Bom`.
- **Gated checks are gated.** The rpm/apk/ipk client tests only run when
  `LX_DNF`/`LX_APK_STATIC`/`LX_OPKG` name a binary.

## Verifying these claims

```sh
# Reference-tool checks that run when the tool is present
cargo test --test debarchive    # real dpkg-deb --info/--contents
cargo test --test verify_clients # pacman (if on PATH); opkg/dnf via LX_OPKG/LX_DNF

# Opt-in live checks
LX_APK_STATIC=/path/to/apk.static cargo test --test plugins_repo_signing
LX_NETWORK_TESTS=1 cargo test --test verify_clients   # Gitee / SourceForge APIs

# Inspect what a build produced, in the host's own tools
dpkg-deb --info foo.deb && dpkg-deb --contents foo.deb
rpm -qip foo.rpm
tar tzf foo.pkg.tar.zst
```

## See also

- [Composability](composability.md) — the companion evaluation.
- [`docs/architecture/replacements.md`](../architecture/replacements.md) —
  what `lx` replaces, consumes, and deliberately does not.
- [`docs/architecture/tooling.md`](../architecture/tooling.md) — every
  external tool `lx` consumes.
- [`docs/reference/host-detection.md`](../reference/host-detection.md) —
  smart defaults across Debian/rpm/Arch hosts.
- [`docs/analysis/landscape.md`](../analysis/landscape.md) — where `lx`
  sits relative to the rest of the ecosystem.
