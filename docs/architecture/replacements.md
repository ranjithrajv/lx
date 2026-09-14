# What `lx` replaces

The replacement story (Parts 2–4), split out from
[tooling.md](tooling.md), which covers what `lx` consumes.

## Part 2 — What `lx` is a drop-in replacement for

"Drop-in" is used precisely below. Three levels appear:

- **Drop-in** — you can point existing config/workflows at `lx` without
  changing their interface.
- **Feature parity** — `lx` covers the same capability set, but through its
  own (different) CLI/config surface.
- **Functional replacement** — `lx` produces the same *artifacts* another
  tool would, without being interface-compatible.

### 2.1 Summary

| Tool / project | Relationship | Level | Evidence |
|---|---|---|---|
| `debian-multiarch-builder` GitHub Action | replaced | **Drop-in** (same inputs/outputs) | `action.yml`; `lx migrate` rewrites workflows |
| `dpkg-deb` | replaced for **building** | Functional | `lib/debarchive.rs`; real `dpkg-deb --info/--contents` accepts output |
| `dpkg-source` | replaced for source packages | Functional | `lib/source.rs`; `dpkg-source -x` reconstructs output |
| `dpkg-shlibdeps` | replaced by `lx deps resolve` | **Drop-in** (command) | `lib/shlibdeps.rs`, decision doc |
| `debsign` / `debsigs` | replaced for signing | Compatible | `_gpgorigin` member; `signature.method: debsign`; Alpine `abuild-sign`/`openssl` replaced by in-process `apk-rsa` |
| `fpm` | replaced for the common path | Feature parity | [`comparison/fpm-vs-nfpm-vs-lx.md`](../comparison/fpm-vs-nfpm-vs-lx.md) |
| `nfpm` | replaced on covered formats | Feature parity | [`comparison/lx-vs-nfpm.md`](../comparison/lx-vs-nfpm.md) |
| `deb-get` | replaced (consumer client) | Feature parity | `lx get`, `show`, `search` "deb-get parity" |
| `makedeb` / AUR `makepkg` | replaced (import + native build) | Feature parity | `lx init --from-aur`, `lx index install` |
| `cargo-deb` / `cargo-dist` / `goreleaser` / `*2deb` | feature parity | Feature parity | checksum sidecars, shell installer, cosign, relocatable, SBOM |
| `dpkg-scanpackages` / `apt-ftparchive` / `reprepro` | replaced for repo publishing | Functional | `lx repo` |
| snap / flatpak / nix | migration source, not output | Functional | `lx migrate native` |
| `dpkg-buildpackage` / `debian/rules` | **not replaced** | — | `shlibdeps` decision doc, scope note |
| `lintian` | **consumed, not replaced** | — | `lib/lintian.rs` |
| `dpkg` / `apt` / `rpm` / `pacman` | **consumed, not replaced** | — | install/query paths above |

### 2.2 `debian-multiarch-builder` — exact drop-in

`lx` is a Rust rewrite of the
[debian-multiarch-builder](https://github.com/ranjithrajv/debian-multiarch-builder)
bash GitHub Action, and `action.yml` is a **drop-in replacement** for its
`action.yml`: same input names (`config-file`, `version`, `build-version`,
`architecture`, `max-parallel`, `lintian-check`, `telemetry-enabled`,
`save-baseline`, `pinned-metadata`, `output-dir`, …) and same output names
(`packages`, `source-packages`, `summary-path`).

Behavioral parity is explicit, e.g. `--source` is always passed because
the bash action always generated source packages (no opt-in flag existed);
`--allow-unverified` defaults to `true` for bash-action parity. Legacy
`debian-multiarch-builder` `package.yaml` keys load as-is (`summary:` →
`description`, `license:` → `license_spdx`, `download_pattern:` +
`architecture_map:` → per-arch `release_pattern`s). `lx migrate` rewrites a
packaging repo's workflows (`...@v1` refs and `lpt build` → `lx build`).

### 2.3 `fpm` — feature parity

Covers the fpm surface most users need: deb/rpm/apk/arch, relation fields
(`Depends`/`Recommends`/`Suggests`/`Conflicts`/`Provides`/`Replaces`/`Breaks`/`Pre-Depends`),
pre/post-install/remove **and** pre/post-upgrade scripts, RPM triggers,
ERB-like `<%= key %>` script templating, debconf templates/config, deb
triggers, custom control fields, per-format overrides, globs in
`contents:`, and language-package inputs (npm/gem/python/cpan via
`registry_source:`). `lx` adds what fpm lacks: source packages, checksum
verification, SBOM/SLSA, reproducible builds, zero-config URL builds.

**Where fpm still wins:** `freebsd`, `solaris`, `snap`, `tar`,
`sh` (self-extracting), `zip` outputs; PEAR, virtualenv, pleaserun, puppet
inputs; `--deb-shlibs`, `--deb-init`, `--deb-systemd`, `--deb-upstart`,
`--deb-default`, `--deb-meta-file`, `--deb-after-purge`.

### 2.4 `nfpm` — feature parity

`lx` adopted nfpm's design directly (see
`docs/decisions/2026-08-20-nfpm-adoptions.md`,
`…-batch2.md`, `…-scripts-triggers-parity.md`): the `overrides:` block,
`arch_variant`, `umask`, `version_schema`, `packager`, vendor, per-format
script mapping, deb triggers, `deb.compression`, `rpm.compression` /
`auto_provides` / `auto_requires` / `defines`, `contents[].packager`,
`disable_globbing`, and the `debsign`-style `_gpgorigin` signature. The
GPG passphrase env var falls back to `$NFPM_PASSPHRASE` for nfpm parity.
`rpm.auto_provides`/`auto_requires` are best-effort scans of the staged
ELF payload (sonames as `name()(N bit)`); `rpm.defines` is accepted but not
applied — the in-process builder has no rpmbuild macro engine, and lx warns
instead of dropping it silently.

The `contents:` DSL now also covers nfpm's per-file features: `file_info`
(`owner`/`group`/`mode`/`mtime`/`lang`), `expand: true`, and
`disown_subtree`. Modes/mtimes and ownership reach the deb/arch/apk tar
headers and the rpm file options; `lang` reaches RPM's `RPMTAG_FILELANGS`
(injected into the built header, since the `rpm` crate has no `%lang`
setter) and is ignored by the other formats, matching nfpm. An `nfpm.yaml`
can be converted with **`lx init --from-nfpm <nfpm.yaml>`** (see
`lib/nfpm.rs`), which maps the config and lists unmappable keys instead of
dropping them.

**Where nfpm still wins:** only nfpm's `.pfx` MSIX-signing *interface* —
lx signs MSIX natively (`AppxSignature.p7x`) and macOS pkg (xar
`Signature`), using `signature.key_file` + `signature.cert_file` rather than
accepting `msix.signature.pfx_file`.
lx adds source packages, forged-release fetching, an `nfpm.yaml` importer,
the consumer CLI, a public Rust embedding API (`lx_lib::api`), and
supply-chain features nfpm has no equivalent for.

### 2.5 `dpkg-deb` and `dpkg-source` — build-side functional replacement

`lx build` writes `.deb`s entirely in-process (`lib/debarchive.rs`): `ar`
with `debian-binary` + `control.tar.{gz,xz,zst}` + `data.tar.{gz,xz,zst}`,
sorted walk, normalized mtime/uid/gid, deterministic compression. It never
runs `dpkg-deb` or `tar`/`gzip`.

`--source` writes `.dsc` + `.orig.tar.xz` + `.debian.tar.xz` in-process
(`lib/source.rs`), with the `.dsc` field order and checksum-section
ordering verified against a real `dpkg-source -b` run, and the xz preset
matching `dpkg-source`'s default. Real `dpkg-deb --info`/`--contents` and
`dpkg-source -x` accept the output (used as test oracles and in the
decision docs' live verification).

**Scope limit:** this replaces the *builder*, not a general-purpose
reader/patch-stack maintainer. `debarchive::extract` is scoped to what
`lx` itself produces (gzip data.tar), and `lx` only ever emits the
`3.0 (quilt)` with-empty-patch-stack case.

### 2.6 `dpkg-shlibdeps` — command drop-in

`lx deps resolve <path>…` is described by its decision doc as "a drop-in for
`dpkg-shlibdeps` itself, not for `dpkg-buildpackage`". It parses the
binary's `DT_NEEDED` plus `(symbol, version)` requirements from
`.gnu.version_r`/`VERNEED` and reads the dpkg `symbols`/`shlibs` databases
to emit **versioned** relations (`libcap2 (>= 2.66)`), fail-closed without
`--ignore-missing-info`. Flags and semantics not covered yet: virtual
`Provides`, multiarch `pkg:arch` qualifiers, `debian/shlibs.local`,
full `-e`/`-T`/`-O`/`-d`/`-p`/`-l` parity.

The build pipeline uses the same core best-effort (falling back to
`dpkg -S`/`rpm -q --whatprovides`/`pacman -Qo`), so no existing build
starts failing.

### 2.7 `debsign` / `debsigs` / apk RSA (and not-yet `dpkg-sig`)

`signature.method: debsign` embeds an armored detached signature as the
`_gpgorigin` member (role selectable via `type: origin|maint|archive`),
compatible with debsigs/nfpm readers. `method: detach` writes a sibling
`.sig`. RPM signatures embed natively via the `rpm` crate. Alpine apk
signing (`apk-rsa`) and `lx repo --format apk` index signing are
in-process PKCS#1 v1.5 RSA/SHA-1 over the compressed control segment,
prepended as `.SIGN.RSA.<keyname>` — no `abuild-sign` or `openssl`.

**Not yet:** a `dpkg-sig` method (true `gpg --clearsign` of the dpkg-sig
control template → `_gpgbuilder`) and a live `debsig-verify`/`debsigs
--verify` test — tracked as follow-up in
`decisions/2026-08-28-debsign-local-payload.md`.

### 2.8 `deb-get` — consumer client parity

`lx`'s consumer subcommands mirror deb-get: `lx install/upgrade/update/
remove/list/show/search/reinstall/rollback`, plus `lx get <subcommand>` as
a thin build-free client. `lx show` and `lx search` are explicitly
documented as deb-get `show`/`search` parity (`lib/show.rs`,
`lib/search.rs`), with `lx search` adding `apt search`-style full-text over
installed dpkg long descriptions and exact-match-first ordering. `lx get
install` falls back to a `+musl_{arch}.deb` asset when no distro-specific
build exists.

Deliberately *not* copied from deb-get: its HTTPS-only trust model — `lx`
keeps fail-closed checksum verification instead.

### 2.9 `makedeb` / AUR (`makepkg`) — import and native build

`lx init --from-aur <pkg>` converts an AUR `PKGBUILD` into a starter
`package.yaml`, and `lx index install <aur-pkg>` fetches the `PKGBUILD`
and builds it through the normal source path — so an AUR package becomes a
native `.deb`/`.rpm`/`.pkg.tar.zst` without `makepkg`. Deliberate
difference from makedeb: recipes are **never executed** — PKGBUILD shell
becomes comments, not code (`lib/wizard.rs`, `lib/plugins/package_index/aur.rs`). Arch
dependency names are kept verbatim for downstream mapping.

### 2.10 `cargo-deb` / `cargo-dist` / `goreleaser` / `*2deb`

The CHANGELOG records "five new features for goreleaser/cargo-dist/*2deb
parity":

1. `.sha256`/`.sha512` sidecars for every artifact.
2. A generated `<package>-install.sh` (`curl | sh`, distro/arch detection,
   checksum verification, native install) — `lib/shell_installer.rs`.
3. `--cross-target <ARCH>` cross-compilation (auto-enables musl-static).
4. `--cosign` Sigstore keyless signing.
5. Dependency mapping from registry manifests across 9 ecosystems.

Plus relocatable binaries via `patchelf` `$ORIGIN/../lib` RPATH
(cargo-dist parity) and SPDX 2.3 + SLSA-v1-shaped provenance (`--sbom`) —
which none of the comparison tools emit. Note that the individual
`cargo-deb`/`cargo-dist` CLIs are **not** interface-compatible targets;
this is feature parity, not a drop-in for their command lines.

### 2.11 snap / flatpak / nix — migration, not output

`lx migrate native` consumes `snap`/`flatpak`/`nix` to **detect and migrate
away from** them: parse their listings, map each finding to an `lx`
package name, install the native package, and (on request) remove the
non-native source. Unmapped findings land in a `missingnative` report
instead of being dropped. This is the inverse of a drop-in replacement:
`lx` is the destination.

### 2.12 `dpkg-scanpackages` / `apt-ftparchive` / `reprepro` — functional replacement

`lx repo <dir>` turns a directory of built `.deb`s into an apt-servable
repository (`Packages`, `Packages.gz`, `Release` with MD5/SHA1/SHA256, and
a clearsigned `InRelease` when a key is given), with a `--multi-suite`
layout mirroring real Debian/Ubuntu archives. The `latest-debs` apt repo +
`lx install` forms a complete producer→distributor loop. It is a
functional replacement, not a flag-compatible `reprepro` (no snapshotting,
no CDN invalidation, no pool management).

---

## Part 3 — Deliberate non-replacements

These are consumed on purpose and will not be reimplemented:

- **`lintian`** — a large Debian-native Perl tool with no Rust equivalent;
  `--lintian` runs the real binary and errors clearly when absent.
- **`gpg`** — OpenPGP signing; the trade-off is stated in `lib/sign.rs`
  ("`gpg` is on every CI runner that matters").
- **`cosign`** — Sigstore keyless signing and Rekor upload.
- **`patchelf`** — RPATH rewriting; optional, best-effort.
- **`dpkg`/`apt`/`rpm`/`pacman`/`dnf`/`zypper`/`apk`/`xbps`** — `lx`
  orchestrates the host package manager; it is an apt-like *front end* for
  forge-release software, not an installer backend.
- **`rpm`/`rpm2cpio`/`cpio`** — only for reading `.rpm` files during
  `lx convert`; there is no native RPM reader yet.
- **`git`** — only to clone/pull the LX community recipe index.
- **`dpkg-buildpackage` / `debian/rules`** — out of scope; `lx deps resolve`
  replaces `dpkg-shlibdeps`, not the whole build.

---

## Part 4 — Cheat sheet

**What `lx` needs on `PATH` for each job**

| Job | External requirements |
|---|---|
| `lx build <forge url>` (binary repack) | *nothing* |
| `lx build --sbom` (SBOM + SLSA provenance) | *nothing extra* |
| `lx build --lintian` (deb only) | `lintian` |
| `lx build --source` (any `build_system`) | the system's `required_tools()` (cmake+ninja, cargo, go, meson+ninja, make) |
| `lx build --source --install-build-deps` | a host package manager (+ `sudo` unless root) |
| `lx build --sign-key …` | `gpg` |
| `lx build --cosign` | `cosign` (+ OIDC) |
| `lx build` staging ELF binaries (automatic RPATH patch) | `patchelf` (optional; warns and skips) |
| `lx build` with `registry_source:` (npm, python, gem, cargo, nuget, mvn, composer, cpan, go, hex, dart) | that ecosystem's tool (`npm`, `pip`+`python3`, `gem`, `cargo`, `nuget`, `mvn`, `composer`, `cpanm`+`perl`, `go`, `mix`+`elixir`, `dart`) |
| `lx install`/`upgrade`/`remove` | `dpkg`/`apt-get` (deb), `rpm` (rpm), `pacman` (arch) + `sudo` |
| `lx convert` from `.rpm` | `rpm`, `rpm2cpio`, `cpio` |
| `lx convert` from `.deb`/Arch | *nothing* |
| `lx repo --sign-key` | `gpg` |
| `lx deps resolve` | *nothing* (reads dpkg `symbols`/`shlibs`) |
| `lx deps scan` / `--bindep` | `dpkg`/`rpm`/`pacman` and/or `ldconfig` for package-name resolution (optional; sonames come from in-process ELF parsing) |
| `lx migrate native --yes` | `snap`/`flatpak`/`nix` (for the sources being migrated) + the host installer |
| `lx index` | `git` (LX community index); HTTP for AUR/repology |

**What `lx` produces without every one of those tools:** every package
format (deb/rpm/arch/apk/ipk/msix), source packages (`.dsc`, `.src.rpm`,
`PKGBUILD`), SBOM/SLSA, checksum sidecars, shell installers, and an
apt repository — all in-process.
