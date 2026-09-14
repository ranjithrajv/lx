# Host build-dependency solving and installation (`--install-build-deps`)

**Date:** 2026-09-14
**Context:** Phase 2 of closing the gap to `makepkg -s` / `rpmbuild`'s
`BuildRequires` / `yay`/`paru`. `lx`'s source builds compile on the host, so
they need host toolchains. `build_depends:` existed but named packages the
caller/CI had to install; `lx` only reported missing ones (and only via
`dpkg -s`, so non-Debian hosts treated *everything* as missing). That made
`lx index install aur/<pkg>` and `build_mode: source` unusable on a fresh
machine without manual setup.

## What it does

New `lib/builddeps.rs`:

- `HostPm` — the host package manager (`Apt`, `Dnf`, `Zypper`, `Pacman`,
  `Apk`, `Xbps`), detected from the *install* binaries on `PATH`
  (`apt-get`/`dnf`/`yum`/`zypper`/`pacman`/`apk`/`xbps-install`), with
  `is_installed()`, `install_program()`, and `name()`.
- `host_package(pm, tool)` — maps distro-independent build-tool names to
  the host's package (`ninja` → `ninja-build` on apt/rpm but `ninja` on
  Arch; `cargo` → `rust` on Arch; `go` → `golang-go`/`golang`/`go`;
  `pkg-config` → `pkgconf` on Arch/Alpine; `musl-gcc` → `musl-tools`/`musl`).
- `resolve(explicit, tools, pm)` — the solver: explicit `build_depends`
  verbatim plus the selected build system's `required_tools()` mapped to
  host packages, deduplicated and sorted.
- `missing()`, `install_command()` (with `sudo` when non-root), and
  `ensure_build_deps(deps, install, dry_run)`.

Wired into `sourcebuild::run` at two points: before the download for an
explicitly configured build system and the explicit `build_depends:`, and
again after auto-detection for the resolved build system's toolchain. The
new `lx build --install-build-deps` flag turns reporting into installing;
without it the behavior is unchanged (bail with the exact command to run).

## Design decisions

- **Closed enum + `match`, not a trait.** Six package managers whose query
  and install invocations differ; there is no third-party plugin dimension
  here (unlike packagers/forges), so a trait would be an interface with one
  implementation. An enum keeps the whole adapter in one file.
- **`build_depends:` stays in host-distro names; tool names are mapped.**
  Cross-distro *package-name* translation (Debian `libssl-dev` ↔ Fedora
  `openssl-devel`) is a real research problem with no reliable offline
  answer, and `depmap` already handles language-ecosystem deps — not
  distro renames. Build-system tool names, by contrast, are
  distro-independent and small in number, so they are mapped exactly.
- **Opt-in, never implicit.** `lx` historically never installed anything it
  wasn't asked to. Installing packages is significant state mutation, so it
  requires the explicit `--install-build-deps`. The flag *is* the consent —
  no interactive prompt, matching `lx`'s non-interactive build path
  (prompting would break CI). `--dry-run` prints the exact command first.
- **`sudo` unless root; fail with instructions otherwise.** If non-root
  without `sudo`, `ensure_build_deps` bails with the command to run as root
  rather than letting `apt-get` fail opaquely. The command is printed in
  real form either way, so it can be copy-pasted.
- **Detect on the install tool, not `dpkg`.** A non-Debian host can have
  `dpkg` installed as a foreign package (as this dev machine does, an Arch
  box with `dpkg` present); detecting on `apt-get`/`dnf`/`pacman`/… avoids
  resolving to a package manager that cannot actually install.
- **`is_installed` uses a real local query**, including checking
  `Status: install ok installed` for apt (since `dpkg -s` succeeds for a
  known-but-removed package).

### AUR `makedepends` wiring (same day)

`InstallOpts` gained `install_build_deps`, exposed as
`lx index install --install-build-deps`, and it is passed straight into the
`BuildArgs` that `build_from_recipe` constructs. `aur::pkgbuild_to_yaml` now
extracts `makedepends` and emits it as `build_depends:` (with the same
"Arch names kept verbatim" warning as `depends:`), and
`lx init --from-aur` emits the AUR RPC's `MakeDepends` the same way. So
`lx index install aur/<pkg> --install-build-deps` installs the package's
make deps before compiling, closing the `makepkg -s` loop.

Note this is inert for today's AUR flow: the generated recipe is still
binary-mode (the PKGBUILD's `build()`/`package()` bodies are never
executed), so `build_depends:` is only acted on once a recipe opts into
`build_mode: source` — which is exactly the Phase 4 boundary.

## Scope not taken (deliberately)

- No cross-distro renaming of `build_depends:` values (above). The AUR
  `makedepends` emitted into `build_depends:` are Arch names verbatim, so
  they are correct on an Arch host and approximate elsewhere.
- No `checkdepends` extraction: `lx` does not run a `check()` step, so
  installing check deps would be dead weight.
- No containers/chroots or isolated rootfs installs (`mock`,
  `arch-nspawn`, `pbuilder`): `lx` builds on the host by design. `--install-build-deps`
  mutates the host, which is the explicit trade.

## Verification

- `lib/builddeps.rs` unit tests: `host_package` distro differences,
  `resolve` dedup/verbatim, `install_command` shapes (sudo apt, pacman,
  xbps, apk), and detection of a manager on this host.
- `tests/builddeps.rs`: empty list is a no-op; report-only bails on a
  missing package (never installs); `install=true, dry_run=true` prints and
  returns without root; an installed package is not reinstalled.
- Live: `lx build <source config> --install-build-deps --dry-run` printed
  `would install build dependencies: sudo pacman -S --needed --noconfirm
  definitely-not-a-real-package-zzz` and the dry-run plan, exit 0; the same
  config without the flag bailed with the same command plus the flag hint.
- `cargo clippy --all-targets --all-features -- -D warnings` and
  `cargo fmt --all --check` clean.

## Known limitations

- `build_depends:` on a non-host distro's naming will not be translated, so
  an AUR-derived recipe still needs its `makedepends` mapped by hand.
- `is_installed` reflects the build host, not the target suite/arch — the
  same host-vs-target caveat as the dependency scanners.
