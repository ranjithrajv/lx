# Feature Parity: lx vs nfpm

A living comparison of [lx](../../README.md) against
[goreleaser/nfpm](https://github.com/goreleaser/nfpm), the mature Go
packager that inspires much of lx's design. Used to track what lx has
adopted, what it deliberately rejects, and what remains to be done.

> Canonical capability details live in the
> [commands](../reference/commands.md) and
> [`package.yaml`](../reference/package-yaml.md) references and the
> [plugin catalog](../architecture/plugin-catalog.md); this page tracks
> parity, and the tables here are a snapshot.

## TL;DR

| | nfpm | lx |
|---|---|---|
| **Language** | Go | Rust |
| **Philosophy** | General-purpose packager — you bring files, it wraps them | Opinionated — fetches forge releases, verifies, wraps them |
| **Primary use case** | Packaging CI/build artifacts you already have | Packaging upstream GitHub/GitLab/etc. release binaries |
| **Can be a library** | ✅ Go package | Not yet public API |

---

## 1. Package formats

| Format | nfpm | lx |
|---|---|---|
| **deb** | ✅ | ✅ |
| **rpm** | ✅ | ✅ |
| **apk** (Alpine) | ✅ | ✅ |
| **arch** (`.pkg.tar.zst`) | ✅ (`archlinux`) | ✅ |
| **ipk** (OpenWrt) | ✅ | ✅ |
| **msix** (Windows) | ✅ | ❌ |
| **Source packages** (`.dsc` + tarballs) | ❌ | ✅ (deb, `.src.rpm`, PKGBUILD) |

**nfpm still wins on Windows output** (msix has no lx equivalent), but lx
now covers every Linux format nfpm does. **lx is the only one that
produces proper source packages.**

---

## 2. Input model — where files come from

| | nfpm | lx |
|---|---|---|
| **Local files** (`contents:` with `src`/`dst`) | ✅ rich DSL (see below) | ✅ `local_payload:` + `binary_path` |
| **Glob patterns** | ✅ (`disable_globbing` to turn off) | ✅ (`disable_globbing` to turn off) |
| **Auto-fetch forge releases** | ❌ | ✅ (GitHub, GitLab, Gitea, Forgejo, Bitbucket, Gerrit, Gitee, SourceForge) |
| **Auto-detect release assets** | ❌ | ✅ (pattern matching on filenames → arches) |
| **Source compilation** | ❌ | ✅ (`build_mode: source` with pluggable build systems: cmake, cargo, go, meson, autotools, make, custom) |
| **Tree/bundle mode** | ✅ (`type: tree`) | ✅ (`bundle: true`) |

**nfpm is a "dumb" packager** — you give it files via its powerful
`contents:` DSL (flat files, trees, globs, symlinks, config files, ghost
files, dir ownership, per-packager entries, `disown_subtree`, `expand`,
`file_info` with custom `mode`/`mtime`/`owner`/`group`/`lang`). **lx is
opinionated** — it fetches releases, auto-installs ancillaries (man pages,
licenses), and supports both binary repack and source-build modes.

---

## 3. Dependency & control-field relations

| Field | nfpm | lx |
|---|---|---|
| **Depends** | ✅ (overridable per-packager) | ✅ (deb + overrides) |
| **Recommends** | ✅ | ✅ |
| **Suggests** | ✅ | ✅ (deb) |
| **Conflicts** | ✅ | ✅ |
| **Provides** | ✅ | ✅ |
| **Replaces** | ✅ | ✅ |
| **Breaks** | ✅ | ✅ |
| **Pre-Depends** | ✅ (deb-specific) | ✅ (deb) |
| **Predepends** (generic) | ✅ | ✅ (deb) |
| **Epoch** | ✅ | ✅ |
| **Version schema** (semver/none) | ✅ | ✅ |
| **Per-format overrides** | ✅ (`overrides.deb`, `overrides.rpm`, …) | ✅ (deb via `overrides:`) |

**Implemented.** `suggests:`, `predepends:`, and the `overrides:` block
(per-format relation-field overrides keyed by `deb`/`rpm`/`arch`) are
all parsed and applied through `effective_relations(format)` in the deb
plugin. The `Relations` struct renders `Suggests:` and `Pre-Depends:`
control lines when non-empty. For RPM, relation fields are mapped to
rpm-crate dependency tags (`requires`, `provides`, `conflicts`,
`obsoletes`, `recommends`, `suggests`) via `parse_rpm_relations()`, with
`Pre-Depends` folded in as `Requires` carrying the legacy `PREREQ` flag.
For Arch, they render as `.PKGINFO` `depend`/`optdepend`/`conflict`/
`provides`/`replaces` (Debian syntax translated to pacman, e.g.
`libc6 (>= 2.34)` → `libc6>=2.34`) and the same arrays in the generated
`PKGBUILD`.

---

## 4. Maintainer metadata & control fields

| | nfpm | lx |
|---|---|---|
| **Vendor** | ✅ (rpm-specific) | ✅ (rpm header tag) |
| **Section / Priority** | ✅ (deb) | ✅ (deb, configurable) |
| **Custom control fields** | ✅ (`deb.fields`) | ✅ (`fields:`) |
| **Homepage** | ✅ | ✅ (auto-derived from forge URL) |
| **Summary override** | ✅ (rpm) | ✅ (description as summary) |
| **Packager** (distinct from maintainer) | ✅ (rpm, arch) | ✅ (rpm header tag + Arch `.PKGINFO` `packager`, falls back to maintainer) |
| **License** | ✅ | ✅ (SPDX) |
| **Arch variant** (e.g. amd64v3) | ✅ (deb) | ✅ (deb, `arch_variant:`) |
| **Umask** | ✅ | ✅ |
| **Version schema** | ✅ (semver/none) | ✅ (semver/none) |

**Feature parity achieved.** lx covers all the metadata knobs nfpm does.
Section/Priority are configurable via `section:`/`priority:` fields,
packager via `packager:` (maps to RPM header tag, falls back to
maintainer), vendor via the legacy `vendor:` key, and arch variant via
`arch_variant:` appended to the `Architecture` control field.

---

## 5. Scripts & triggers

| | nfpm | lx |
|---|---|---|
| **preinstall / postinstall** | ✅ (overridable) | ✅ (deb/rpm/apk/ipk) |
| **preremove / postremove** | ✅ (overridable) | ✅ (deb/rpm/apk/ipk) |
| **pretrans / posttrans** (rpm) | ✅ | ✅ |
| **verify** (rpm) | ✅ | ✅ |
| **preupgrade / postupgrade** (apk/arch) | ✅ | ✅ (apk scripts + arch `.INSTALL`) |
| **debconf** (`templates`, `config`) | ✅ | ✅ |
| **deb triggers** (`interest`, `activate`) | ✅ | ✅ |
| **deb `rules`** | ✅ | ✅ |
| **Custom build hooks** | ❌ | ✅ (`prebuild_steps`, `build_commands`, `install_commands`) |

**Feature parity achieved.** lx maps nfpm's format-specific script names
to the right control members/scriptlets per format: deb `DEBIAN/{preinst,
postinst,prerm,postrm}` (mode 0755), rpm `%pre`/`%post`/`%preun`/`%postun`
plus `%pretrans`/`%posttrans`/`%verify`, apk `.pre-install`/`.post-install`/
`.pre-deinstall`/`.post-deinstall` (plus the upgrade hooks) and ipk
`preinst`/`postinst`/`prerm`/`postrm`, and arch `.INSTALL`
`pre_upgrade()`/`post_upgrade()` hooks. Deb-specific extras — debconf
`templates`/`config`, `rules`, and `interest`/`activate` triggers — live
in the `deb:` block. As a bonus lx keeps nfpm's build-time hooks
(`prebuild_steps`, custom `build_commands`/`install_commands` under
`$DESTDIR`) that nfpm has no equivalent for.

---

## 6. Signing & verification

| | nfpm | lx |
|---|---|---|
| **deb signing** | ✅ (debsign + dpkg-sig) | ✅ (debsign + detach) |
| **rpm signing** | ✅ (PGP, embedded) | ✅ (embedded) |
| **apk signing** | ✅ (RSA PEM) | ✅ (RSA/SHA-1, in-process) |
| **msix signing** | ✅ (PFX) | N/A |
| **Checksum verification** | ❌ | ✅ (fail-closed by default; sidecar + pinned metadata) |
| **SBOM / SLSA provenance** | ❌ | ✅ (`--sbom` → SPDX 2.3 + SLSA v1) |
| **Reproducible builds** | ✅ (SOURCE_DATE_EPOCH, mtime) | ✅ (SOURCE_DATE_EPOCH, release-publish timestamps, sorted archive walk) |

**lx leads on software-supply-chain security** — checksum verification
is mandatory by default, builds produce SBOM + SLSA provenance, and
`--summary` emits an auditable `provenance` array. nfpm has wider
format-specific signing but no verification, SBOM, or reproducibility
story.

---

## 7. Configurability & ergonomics

| | nfpm | lx |
|---|---|---|
| **Config file** | `nfpm.yaml` (full DSL) | `package.yaml` (simpler, opinionated) |
| **Templating** | ❌ (recommends envsubst/jsonnet) | ❌ (env-var expansion in some fields) |
| **JSON Schema** | ✅ (published schema) | ✅ (`lx schema`) |
| **Interactive init** | ✅ (`nfpm init`) | ✅ (`lx init`, `--from`, `--from-aur`) |
| **Zero-config URL build** | ❌ | ✅ (`lx build https://github.com/owner/repo`) |
| **Dry-run** | ❌ | ✅ (`--dry-run` previews build matrix) |
| **Validate without building** | ❌ | ✅ (`lx validate`) |
| **Auto-discover patterns** | ❌ | ✅ (`lx init --from`) |
| **Dependency scanner** | ❌ | ✅ (`lx deps scan` — ELF `DT_NEEDED` → `dpkg -S`) |
| **Musl-static builds** | ❌ | ✅ (`musl: true` — no glibc dep, runs on any Linux) |
| **Per-file mtime/mode/owner/group** | ✅ (`file_info`) | 🟡 (umask only; per-file mode/owner/group not yet) |
| **Umask control** | ✅ | ✅ |

---

## 8. Consumer / distribution features

| | nfpm | lx |
|---|---|---|
| **Consumer CLI** | ❌ | ✅ (`lx get`: install/upgrade/update/remove/rollback/search/list/show) |
| **Local manifest / state** | ❌ | ✅ (`installed.json`, cross-checked with dpkg) |
| **Rollback** | ❌ | ✅ (`lx rollback`) |
| **Repo generation** | ❌ | ✅ (`lx repo` — deb/rpm/pacman/apk/opkg indexes, per-format signing; `--multi-suite` for multi-suite layout; `lx publish` builds + indexes in one run) |
| **`apt search`-like search** | ❌ | ✅ (`lx search`, full-text + local + installed) |
| **Migrate snap/flatpak/nix → native** | ❌ | ✅ (`lx migrate native`, works on deb/rpm/arch hosts) |
| **Format conversion** | ❌ | ✅ (`lx convert` — deb↔rpm↔arch native rebuild with scriptlet carry-over) |
| **AUR import** | ❌ | ✅ (`lx init --from-aur`) |
| **Lintian** | ❌ | ✅ (`--lintian`, shelled out) |
| **GitHub Action** | ❌ (goreleaser has separate CI) | ✅ (`action.yml`, drop-in for debian-multiarch-builder) |
| **Sandboxed builds** | ❌ | ✅ (`--sandbox`, `unshare -n`) |

**lx is not just a packager — it's a full packaging workflow system.**
nfpm stops at producing an artifact; lx covers the entire lifecycle from
upstream discovery through building, installing, upgrading, rolling back,
and serving a repository.

---

## Summary verdict

| You should use nfpm if… | You should use lx if… |
|---|---|
| You already have files and need max control over their placement, per-file metadata, and distro policy fields | You want to package an upstream forge release end-to-end with minimal config |
| You need Windows (msix) output | You care about supply-chain verification, SBOMs, and reproducible builds |
| You need maintainer scripts (pre/post-install, triggers, debconf) | You want a consumer-facing install/upgrade/rollback workflow with repo serving |
| You need per-format dependency overrides, Suggests, Pre-Depends, etc. | You want zero-config URL builds, auto-discovery, and AUR import |
| You're embedding it as a Go library in another tool | You want source packages (`.dsc`, `.src.rpm`, PKGBUILD) |

**nfpm is the more mature, general-purpose packager with broader format
coverage and richer per-format control. lx is a more opinionated,
end-to-end packaging *system* that trades nfpm's low-level control for
automation, verification, and consumer-facing workflow features.**
