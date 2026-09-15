# Packaging for Linux, from the upstream side

**Date:** 2026-09-14
**Responds to:** Fresh, ["I hate packaging my software for Linux"](https://getfresh.dev/docs/blog/packaging-for-linux/)
**Status:** draft — outreach post, publishable as-is

---

Fresh's post is the best description of the problem I've read, because it
refuses to pretend there is a single winner. npm, cargo, AppImage, Flatpak,
deb, rpm, AUR, nix, mise, Homebrew-for-Linux, a tarball — every channel
serves *some* users, none serves all of them, and each one is another
toolchain to keep alive after a release.

`lx` starts from the opposite end. Instead of asking "which channel do I
add?", it asks "what does the machine already have?" — `dpkg`, `rpm`,
`pacman`, `apk` — and produces a package that tool already knows how to
install and upgrade. One config, native output per family, no container
and no distro toolchain required to build it.

This post maps each problem in the Fresh post to what `lx` actually does
today, and then to what it does not.

## The short version

| What Fresh described | What `lx` does |
|---|---|
| Nine-plus channels, each fragile | One `package.yaml` → deb/rpm/arch/apk/ipk; `--format all` builds them in one run, and `lx publish` also writes each format's repo index |
| Debian won't take the package without packaging every Rust dependency | `lx` builds a real `.deb` from your release or source *without* entering Debian's dependency policy; `--source` emits a valid `.dsc` if you later want to |
| A binary built on new Ubuntu won't load on old Ubuntu | `musl: true` → musl-static, no glibc dependency; `lx get install` falls back to a `+musl_{arch}.deb` |
| No automatic updates for `.deb`/`.rpm` | `lx publish` builds the formats and writes signed apt **and** rpm indices (plus pacman/apk/opkg), meant to be served as plain static files |
| Mise broke when a trust root rotated out from under you | Fail-closed checksum verification and `--pinned-metadata`; no third-party trust root sits in the path |
| AUR went read-only and releases stopped | AUR is an *input* (`lx init --from-aur`), not a dependency; Arch packages are built natively, without `makepkg` |
| Flatpak's sandbox flags, AppImage's FUSE and slow squashfs | A native package: no sandbox, no FUSE, no squashfs mount — it starts at the speed of the binary |
| "Do I need a package manager at all? I'll write a self-updater." | `lx` is that updater on deb, rpm, and pacman hosts, and keeps `apt`/`dnf`/`dpkg`/`pacman` as the source of truth instead of replacing them |

## One config, every native format

The length of the channel list in the Fresh post is a symptom of
format-centric packaging: each format is a separate toolchain with its own
trust model, and each one is a separate thing that can break after a
release. `lx` is upstream-centric: point it at the project, and the format
is an output choice.

You don't even need a config file to see the shape of it:

```sh
lx init --from sinelaw/fresh                 # scaffold a starter package.yaml
lx build https://github.com/sinelaw/fresh    # debs for every published arch
lx build package.yaml --format all           # deb, rpm, arch, apk, ipk in one run
lx publish package.yaml                      # ... plus every format's repo index
```

A minimal config, once you want to pin asset patterns, set dependencies,
or add maintainer scripts:

```yaml
# package.yaml
package_name: fresh
github_repo: sinelaw/fresh
description: "A terminal-based code editor"
maintainer: "Jane Doe <jane@example.com>"
license_spdx: Apache-2.0
musl: true
package_format: deb        # or rpm / arch / apk / ipk; --format all overrides
```

Everything above is written in-process: `lx` never shells out to
`dpkg-deb`, `dpkg-source`, `rpmbuild`, `makepkg` or `abuild`, and it needs
none of them on the machine building the package. So "build on the target
distro" stops being a requirement — you can produce all five formats from
whatever host runs CI.

## Debian family: a `.deb` you can ship, without the archive policy

The blocker Fresh names is real: a `.deb` that ships a Rust binary normally
implies a Debian source package whose every dependency is also a Debian
package, and keeping up with that across security updates is a full-time
job. That policy exists for good reasons, and `lx` does not route around
it.

What `lx` separates is *a `.deb` your users can install and upgrade* from
*a `.deb` that clears Debian's archive policy*. The first one is cheap and
is what almost every user means by "a deb"; the second is a project in
itself. `lx` builds the first from the release you already publish, or
from source, with real dependency metadata computed from the binary
(`--bindep`, and `lx deps resolve`, a `dpkg-shlibdeps` drop-in that reads the
dpkg `symbols`/`shlibs` databases). It does not require `debian/rules`, a
patch stack, or vendored sources.

If you *do* want to start the archive conversation later, `--source` emits
a valid `3.0 (quilt)` source package (`.dsc` + `.orig.tar.xz` +
`.debian.tar.xz`) per suite, verified field-for-field against a real
`dpkg-source -b`. It is a starting point for that process, not a shortcut
past it.

## Old glibc: ship musl, not another distro matrix

Fresh's post identifies glibc symbol versioning as the portability problem
that makes AppImage not-quite-universal, and that's the one `lx` has a
direct switch for. One line:

```yaml
musl: true
```

- **cargo** builds against `--target x86_64-unknown-linux-musl` (the
  rustup target is added automatically), **go** uses `CGO_ENABLED=0`,
  **cmake** uses `musl-gcc`/`musl-g++` + `-static`.
- For binary repacks, `lx` prefers the `*-linux-musl*` asset when the
  release ships one.
- The emitted package drops the usual `libc6` fallback from `Depends`,
  because a musl binary has no glibc requirement to declare.
- `--cross-target arm64` cross-compiles *and* enables musl-static, which
  is what makes one build work on a two-year-old Ubuntu and this year's
  Fedora at the same time.

The consumer side knows about it too: `lx get install` tries a
distro-specific build first, then falls back to `+musl_{arch}.deb`.

## Automatic updates: apt and dnf, from static files

This is the part of the Fresh post that asks for the right thing:

> It would've been nice if there was a quick serverless solution where you
> could just say a URL to look for newer versions of a single package, and
> both apt and dnf would remember that and update a package.

`lx repo` is that, using each distro's own mechanism instead of a bespoke
updater — and `lx publish` runs the whole build-and-index loop in one command:

```sh
lx publish package.yaml --origin fresh --sign-key "$KEY"      # build + index every format
lx repo ./dist/deb --format deb --multi-suite --origin fresh  # or index an existing directory
lx repo ./dist/rpm --format rpm --sign-key "$KEY"
```

The output is plain files — `Packages`/`Packages.gz`/`Release`/`InRelease`
for apt, `repodata/` for dnf — so it lives anywhere that can serve static
bytes: GitHub Pages, a release asset, an object store, a webroot. There is
no service to run and no daemon to keep up. Users add one line:

```text
# /etc/apt/sources.list.d/fresh.list
deb [signed-by=/usr/share/keyrings/fresh.gpg] https://sinelaw.github.io/fresh/apt stable main
```

```ini
# /etc/yum.repos.d/fresh.repo
[fresh]
name=Fresh
baseurl=https://sinelaw.github.io/fresh/rpm/
enabled=1
gpgcheck=1
```

From then on `apt upgrade` and `dnf upgrade` pick up new releases like any
other package, because they *are* any other package. No npm install script
that re-downloads a binary, no parallel updater, no per-channel release
checklist. `lx repo` writes pacman, apk, and opkg indices the same way, so
one static tree can serve the other families too.

The client side is no longer Debian-only either: `lx get install`/`upgrade`
resolve the *host's* native format (deb on dpkg hosts, rpm on rpm hosts, arch
on pacman hosts; `--format` to override) and read whichever org
`LX_INDEX_ORG` names, so the same `lx`-managed update path works across
families.

## Mise, AUR, and the "channel broke and nobody noticed" class of bug

The mise story — GitHub rotated an attestation certificate, mise had
pinned an old trust root, and Fresh broke silently until a user complained
— is about a dependency on infrastructure you don't control. Two
properties in `lx` are aimed at that class of failure:

- **Verification is fail-closed and pinned to the artifact.** `lx build`
  refuses to package an unverified asset by default. `--pinned-metadata
  release-metadata.json` verifies downloads against a provenance pin you
  captured at vet time, and `build-summary.json` records the verification
  method and SHA-256 of every asset. There is no external attestation root
  in the path that can rotate out from under a release (`--allow-unverified`
  and `--no-verify` are explicit, visible opt-outs).
- **AUR is an input, not a dependency.** `lx init --from-aur fresh` turns a
  `PKGBUILD` into a starter `package.yaml` — the shell becomes comments and
  is never executed — and `lx install <aur-pkg>` builds through the
  normal source path. If AUR is read-only, your users are not blocked, and
  "exotic" third-party channels stop being single points of failure.

## Flatpak flags and AppImage FUSE

`lx` doesn't fight the sandbox or the squashfs — it just isn't a sandbox.
It emits an ordinary native package: the binary at its FHS path, man pages
under `share/man`, the license under `share/doc/<pkg>/`, dependencies
declared. No FUSE, no squashfs mount on startup, no sandbox flags that
contradict what a TUI that "rampages around your machine and network"
actually needs. Startup is the binary's own startup.

For users already on a parallel install, `lx migrate native` plans — and only
on `--yes` executes — migration of snap, Flatpak, nix, and `curl | sh`
installs to the native package, with anything unmappable reported rather
than dropped.

## "Do I even need a package manager?"

The conclusion of the Fresh post — a statically linked musl binary with a
built-in self-updater as the recommended channel — is a reasonable
emergency measure. The observation worth making is that the self-updater
is the hard part, and it's the part you don't have to write yourself.

If the musl binary is the payload, `lx` packages it; the update path is
the native repo above. If you'd rather not run a repo, `lx` is the client —
and it is not Debian-only: on an rpm or pacman host the same commands fetch
that host's native asset and compare versions with that format's own
ordering.

```sh
lx get install fresh            # deb, rpm, or arch — whichever this host is
lx get install --format rpm fresh
lx get upgrade fresh
lx rollback fresh               # if the new release is bad
```

`lx` records what it installed in a local manifest (`installed.json`) and
cross-checks it against `dpkg`/`rpm`/`pacman`, so it is never the sole
source of truth about the system — which is exactly the failure mode a
bespoke self-updater has to solve from scratch. None of this removes the
other channels: the release tarball, the checksum-verified
`fresh-install.sh`, and the standalone musl binary all still exist. Native
packages become the default, not the only option.

## What this does not solve

A post that only lists wins isn't worth sending, so:

- **Official Debian/Ubuntu/Fedora inclusion.** `lx` does not get you into
  `main`/`extra`/Fedora. That still needs distro review and the dependency
  packaging. `--source` gives you a valid source package to start from, but
  `lx` does not lobby, and the policy does not disappear.
- **Maturity.** `lx` is new (2026), pre-1.0, with far fewer contributors
  than `fpm` or `nfpm`. The output packages are standard and checkable —
  real `dpkg-deb --info`, `dpkg-source -x`, and `lintian` accept them — but
  the tool adopting them is young, and its own first tagged release has not
  shipped yet.
- **No upstream attestation verification yet.** `lx` verifies checksums and
  a local `--pinned-metadata` pin; it does not yet consume Sigstore or
  GitHub artifact attestations, so an upstream that publishes only an
  attestation (no checksum sidecar) fails closed rather than being trusted.
- **Breadth.** No `freebsd`, `snap`, `tar`, `zip`, or self-extracting
  output; `msix` and `osxpkg` are best-effort and unsigned. Per-file
  `owner`/`group`/`mode` are supported via `contents[].file_info`. fpm and
  nfpm still cover the remaining outputs; `lx` complements rather than
  replaces them.
- **Hosted infrastructure.** `lx repo` is a generator, not a CDN. There is
  no snapshotting, retention policy, or sync service.
- **The matrix shrinks but doesn't vanish.** `--format all`/`lx publish`
  remove the per-format invocations, but you still choose formats and
  architectures, and source builds are native-arch only — one runner per
  architecture.

## What it would look like for Fresh

The distance from here to a shipped set of packages is small:

1. **Look, without committing.** `lx init --from sinelaw/fresh` writes a
   starter config from the release assets; `lx build
   https://github.com/sinelaw/fresh --format all` produces every format for
   every arch the release publishes, checksum-verified.
2. **Add a `package.yaml`** (the one above; `musl: true` if the release
   already ships a musl asset).
3. **Run it on release.** The composite action covers the Debian path as a
   drop-in for the existing packaging workflow:

   ```yaml
   - uses: ranjithrajv/lx@v1
     with:
       config-file: package.yaml
       version: ${{ github.ref_name }}
       build-version: '1'
       # lx-version: v0.1.0   # use a prebuilt musl-static lx instead of compiling
       # lintian-check: 'true'
   ```

   `lx` itself is built the same way now — its release workflow produces
   musl-static binaries — so the action can consume a pinned release
   (`lx-version`) instead of compiling from source. For the full multi-format
   release, one run step does it: `lx publish package.yaml --sign-key "$KEY"`.
4. **Publish the repo.** `lx publish` already wrote each format's signed
   index under `dist/<format>/`; push that directory to Pages and it is the
   apt and dnf source from the section above.

The expensive part of the Fresh list — nine toolchains, each with its own
release and its own failure mode — collapses to one config and one command
(`lx publish`). If any of it doesn't hold for Fresh's actual asset names, or
for a C dependency that won't build against musl, that is exactly the kind of
bug worth filing rather than working around.

## See also

- [`docs/analysis/landscape.md`](../analysis/landscape.md) — where `lx` sits relative
  to every other tool in the pipeline.
- [`docs/architecture/tooling.md`](../architecture/tooling.md) — what `lx` consumes and what it
  replaces, including the in-process list.
- [`docs/comparison/fpm-vs-nfpm-vs-lx.md`](../comparison/fpm-vs-nfpm-vs-lx.md) —
  category-by-category comparison with flags and fields.
- [`docs/decisions/2026-09-11-musl-static-builds.md`](../decisions/2026-09-11-musl-static-builds.md) —
  why musl over "build on the oldest distro".
- [`docs/decisions/2026-09-11-multi-suite-repo.md`](../decisions/2026-09-11-multi-suite-repo.md) —
  the `lx repo` layout.
- [`README.md`](../../README.md) — the full command and `package.yaml`
  reference.
