# Packager architecture

Part of the [lx docs](../README.md). Per-plugin detail:
[plugins.md](plugins.md) and [plugin-catalog.md](plugin-catalog.md).

`lx` has eight independent plugin dimensions, each extensible without
editing the core pipeline:

| Dimension | Purpose | Count | Selection |
|---|---|---|---|
| **Packager** | Produce installable artifact | 7 (deb, rpm, arch, apk, ipk, msix, osxpkg) | `--format` / `package_format:` |
| **ForgeSource** | Discover forge releases/assets | 9 (github, gitlab, gitea, forgejo, bitbucket, gerrit, gitee, sourceforge, custom) | `--source` / URL sniffing / `source: custom` |
| **BuildSystem** | Compile source tree | 7 (cmake, cargo, go, meson, autotools, make, custom) | `build_system:` |
| **RegistrySource** | Fetch from language registries | 11 (npm, python, gem, cargo, go, hex, dart, nuget, maven, composer, cpan) | `registry_source:` |
| **ArtifactFormat** | Unpack a release asset | 6 (tar.gz, tar.xz, tar.zst, tar, zip, raw) | `artifact_format:` / filename |
| **Signer** | Sign the artifact | 4 (gpg-detach, rpm-pgp, deb-debsign, apk-rsa) | `(package_format, sign_method)` |
| **DependencyMapper** | Map deps to target names/syntax | 5 (debian, rpm, pacman, alpine, openwrt) | target `package_format` |
| **PackageIndex** | Publish **or** read/discover a package index | 8 — 5 write (apt, opkg, pacman, apk, rpm) + 3 read (lx-community, aur, repology) | `lx repo --format` / `indexes.yaml` |

Source and RegistrySource are **not merged** — Source discovers *what's
available* (returns release metadata), RegistrySource fetches *specific
files* (returns a local directory). See
[`docs/plugins.md`](plugins.md) for the full architecture.

Every package it produces is a real native package, not a thin wrapper:
full dependency relations (`Depends`/`Recommends`/`Conflicts`/`Replaces`/
`Provides`/`Breaks`), epoch-aware versioning, man pages and license files
placed at their conventional FHS paths, and genuine source-package
compression — all output that the host's own reference tooling
(`dpkg-deb`/`dpkg-source`/`lintian` for deb) accepts without complaint,
built natively on bare metal — no containers, no emulation, no Debian host
required.

The relation and config fields below carry into every format: deb
`Depends`/`Recommends`/`Suggests`/`Conflicts`/`Replaces`/`Provides`/`Breaks`/
`Pre-Depends`; rpm `Requires`/`Recommends`/`Suggests`/`Conflicts`/`Obsoletes`/
`Provides` (plus best-effort `auto_provides`/`auto_requires`); and Arch
`.PKGINFO` `depend`/`optdepend`/`conflict`/`provides`/`replaces`. `contents:`
entries of kind `config` become deb conffiles, rpm `%config`/`%config(noreplace)`,
and the pacman `backup` list. `epoch` is emitted as RPM's epoch header and
folded into Arch's `pkgver`, and `--sign-key` signs deb/arch detached (rpm
embeds the PGP signature natively).

It's also trustworthy by default about what it downloads and repackages:
a build refuses to proceed on an unverified asset unless you explicitly
say otherwise, every download's verification method and checksum are
recorded for later audit, and pinning a prerelease or draft tag gets you
a warning instead of a silent surprise.

