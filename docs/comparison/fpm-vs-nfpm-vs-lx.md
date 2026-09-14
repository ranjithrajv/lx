# Detailed Feature Parity: fpm vs nfpm vs lx

A comprehensive, category-by-category comparison of the three major
Linux packagers, with specific flags, fields, and behaviors documented.

---

## 1. Package formats

### Format matrix

| Format | fpm flag | nfpm | lx |
|---|---|---|---|
| deb | `-t deb` | ✅ `deb` | ✅ default |
| rpm | `-t rpm` | ✅ `rpm` | ✅ `--format rpm` |
| apk (Alpine) | `-t apk` | ✅ `apk` | ✅ `--format apk` |
| arch (`.pkg.tar.zst`) | `-t pacman` | ✅ `archlinux` | ✅ `--format arch` |
| ipk (OpenWrt) | ❌ | ✅ `ipk` | ✅ `--format ipk` |
| msix (Windows) | ❌ | ✅ `msix` | ❌ |
| osxpkg (macOS) | `-t osxpkg` | ❌ | ❌ |
| freebsd | `-t freebsd` | ❌ | ❌ |
| solaris | `-t solaris` | ❌ | ❌ |
| p5p | ❌ | ❌ | ❌ |
| snap | `-t snap` | ❌ | ❌ |
| tar | `-t tar` | ❌ | ❌ |
| sh (self-extracting) | `-t sh` | ❌ | ❌ |
| zip | `-t zip` | ❌ | ❌ |
| **Source packages** (`.dsc`, `.src.rpm`, PKGBUILD) | ❌ | ❌ | ✅ `--source` |

### Key differences

- **fpm** supports 15+ formats. It can also *convert between formats*
  (e.g. `-s rpm -t deb` converts an RPM to a DEB).
- **nfpm** covers the 7 major Linux formats + Windows msix. It has
  format-specific sub-config blocks (`deb:`, `rpm:`, `apk:`,
  `archlinux:`, `ipk:`, `msix:`).
- **lx** focuses on deb/rpm/arch/apk/ipk and uniquely produces *source*
  packages (`.dsc` + `.orig.tar.xz` + `.debian.tar.xz`, `.src.rpm`,
  PKGBUILD). Format conversion is via `lx convert` (deb↔rpm↔arch) —
  reads source metadata + install tree, rebuilds natively in target
  format with scriptlet carry-over.

---

## 2. Input sources

### fpm — 15+ source types (CLI flags)

| Source | Flag | Auto-downloads |
|---|---|---|
| Directory | `-s dir` | — |
| npm package | `-s npm` | ✅ |
| Ruby gem | `-s gem` | ✅ |
| Python package | `-s python` | ✅ |
| CPAN module | `-s cpan` | ✅ |
| PEAR package | `-s pear` | ✅ |
| Node package | `-s npm` | ✅ |
| RPM (conversion) | `-s rpm` | — |
| deb (conversion) | `-s deb` | — |
| tar archive | `-s tar` | — |
| zip archive | `-s zip` | — |
| virtualenv | `-s virtualenv` | — |
| pleaserun | `-s pleaserun` | — |
| puppet | `-s puppet` | — |
| gem (from git) | `-s gem --gem-git-repo` | ✅ |

### nfpm — files only (config)

Files are supplied via the `contents:` DSL in `nfpm.yaml`:

```yaml
contents:
  - src: path/to/local/foo     # local file/glob
    dst: /usr/bin/foo
  - src: some/directory/        # directory tree
    dst: /etc
    type: tree
  - src: path/to/*.1.gz         # glob
    dst: /usr/share/man/man1/
```

### lx — files + forge releases + language PMs (config + zero-config)

```yaml
# Binary repack from forge release (auto-fetch + verify)
package_name: eza
github_repo: eza-community/eza

# Or local files
local_payload: path/to/archive.tar.gz

# Or source compilation
build_mode: source
build_system: cmake

# Or language package manager (registry source plugin)
registry_source: npm  # npm | python | gem | cargo | go | hex | dart | nuget | maven | composer | cpan
github_repo: typescript # package name in the registry
version: latest        # optional version constraint
```

**Registry source plugins** (`registry_source:` in package.yaml):

| Source | Language | Mechanism | Required tools |
|---|---|---|---|
| npm | JavaScript/Node | `npm pack` + extract | `npm` |
| python | Python | `pip download --no-binary :all:` + extract | `pip`, `python3` |
| gem | Ruby | `gem fetch` + extract | `gem` |
| cargo | Rust | `cargo install --root <dir>` | `cargo` |
| nuget | .NET/C# | `nuget install` + extract | `nuget` |
| maven | Java/Kotlin/Scala | `mvn dependency:copy-dependencies` | `mvn` |
| composer | PHP | `composer install` | `composer` |
| cpan | Perl | `cpanm` + build install tree | `cpanm`, `perl` |
| `go` | Go | `go get` + `go build` static binary | `go` |
| `hex` | Elixir/Erlang | `mix deps.get` + stage | `mix`, `elixir` |
| `dart` | Dart/Flutter | `dart pub get` + stage | `dart` |

Registry source plugins are modular — each is a separate `RegistrySource`
trait implementation registered in `lib/plugins/registry/`. Adding a new
ecosystem is implementing the trait and registering it.

**Unique to lx:** zero-config URL builds (`lx build https://github.com/owner/repo`),
auto-discovery of release assets, checksum verification (fail-closed),
source-package generation, and musl-static builds.

---

## 3. Dependency & control-field relations

### Relation fields compared

| Field | fpm | nfpm | lx |
|---|---|---|---|
| Depends | `-d 'name'` | ✅ overridable per-format | ✅ overridable per-format |
| Recommends | `--deb-recommends` | ✅ | ✅ |
| Suggests | `--deb-suggests` | ✅ | ✅ |
| Conflicts | `--conflicts` | ✅ | ✅ |
| Provides | `--provides` | ✅ | ✅ |
| Replaces | `--replaces` | ✅ | ✅ |
| Breaks | ❌ | ✅ | ✅ |
| Pre-Depends | `--deb-pre-depends` | ✅ (deb + generic) | ✅ |
| Epoch | `--epoch` | ✅ | ✅ |
| Build-Depends | `--deb-build-depends` | ❌ | ❌ |
| Version schema | ❌ | ✅ `semver` / `none` | ✅ `semver` / `none` |
| Per-format overrides | ❌ | ✅ `overrides.deb`, `overrides.rpm` | ✅ `overrides: {deb: ..., rpm: ...}` |
| Shlibs | `--deb-shlibs` | ❌ | ❌ |

### fpm dependency details

- Dependencies are set via repeated `-d` flags: `-d 'libc6' -d 'libfoo > 1.0'`
- No per-format override mechanism
- No Breaks support
- RPM dependencies use the same `-d` flag (fpm translates)

### nfpm dependency details

```yaml
depends:
  - git
  - nginx (>= 1.18.0)
overrides:
  deb:
    depends:
      - some-lib-dev
  rpm:
    depends:
      - some-lib-devel
```

### lx dependency details

```yaml
depends: "libatomic1, libgtk-3-0"
recommends: "bash-completion"
suggests: "eza-legacy"
conflicts: "lsd"
replaces: "eza-legacy"
provides: "eza-cli"
breaks: "eza-legacy (<< 2.0)"
predepends: "libc6"
overrides:
  deb:
    depends: "libatomic1"
  rpm:
    depends: "libatomic"
```

---

## 4. Maintainer metadata & control fields

| Field | fpm | nfpm | lx |
|---|---|---|---|
| Package name | `-n NAME` | ✅ `name` | ✅ `package_name` |
| Version | `-v VERSION` | ✅ `version` | ✅ `version` |
| Iteration/Release | `--iteration` | ✅ `release` | ✅ `build_version` |
| Maintainer | `-m MAINTAINER` | ✅ `maintainer` | ✅ `maintainer` |
| Vendor | `--vendor` | ✅ `vendor` (rpm) | ✅ legacy `vendor:` field |
| Description | `--description` | ✅ `description` | ✅ `description` |
| Homepage | `--url` | ✅ `homepage` | ✅ auto-derived from forge URL |
| License | `--license` | ✅ `license` | ✅ `license_spdx` |
| Section | `--category` | ✅ `section` | ✅ `section` |
| Priority | `--deb-priority` | ✅ `priority` | ✅ `priority` |
| Architecture | `-a ARCH` | ✅ `arch` | ✅ auto-detected |
| Packager | ❌ | ✅ `rpm.packager` | ✅ `packager` |
| Arch variant | ❌ | ✅ `deb.arch_variant` | ✅ `arch_variant` |
| Custom fields | `--deb-field 'FIELD: VALUE'` | ✅ `deb.fields` | ✅ `fields` |
| Compression | `--deb-compression xz` | ✅ `deb.compression` | ✅ `compression` |
| Umask | ❌ | ✅ `umask` | ✅ `umask` |
| File modes/owner/group | `--deb-user`, `--deb-group`, `--rpm-attr` | ✅ `file_info.mode/owner/group/lang` | 🟡 global umask only |

### fpm metadata details

- `--deb-field 'Bugs: url'` adds arbitrary control fields
- `--rpm-attr 750,user1,group1:/some/file` sets per-file RPM attributes
- `--deb-use-file-permissions` preserves source file modes
- `--category` maps to deb Section

---

## 5. Scripts & triggers

### Lifecycle scripts

| Script | fpm | nfpm | lx |
|---|---|---|---|
| pre-install | `--before-install` | ✅ `scripts.preinstall` | ✅ `scripts.preinstall` |
| post-install | `--after-install` | ✅ `scripts.postinstall` | ✅ `scripts.postinstall` |
| pre-remove | `--before-remove` | ✅ `scripts.preremove` | ✅ `scripts.preremove` |
| post-remove | `--after-remove` | ✅ `scripts.postremove` | ✅ `scripts.postremove` |
| pre-upgrade | `--before-upgrade` | ❌ | ✅ `scripts.preupgrade_script` |
| post-upgrade | `--after-upgrade` | ❌ | ✅ `scripts.postupgrade_script` |
| pre-transaction (rpm) | `--rpm-pretrans` | ❌ | ✅ `scripts.pretrans` |
| post-transaction (rpm) | `--rpm-posttrans` | ❌ | ✅ `scripts.posttrans` |
| verify (rpm) | `--rpm-verifyscript` | ❌ | ✅ `scripts.verify` |
| post-purge (deb) | `--deb-after-purge` | ❌ | ❌ |

### Debian-specific triggers & config

| Feature | fpm | nfpm | lx |
|---|---|---|---|
| debconf templates | `--deb-templates` | ❌ | ✅ `deb.templates` |
| debconf config | `--deb-config` | ❌ | ✅ `deb.config` |
| interest trigger | `--deb-interest` | ✅ `deb.triggers.interest` | ✅ `deb.triggers_interest` |
| interest_noawait | `--deb-interest-noawait` | ✅ `deb.triggers.interest_noawait` | ✅ `deb.triggers_interest_noawait` |
| activate trigger | `--deb-activate` | ✅ `deb.triggers.activate` | ✅ `deb.triggers_activate` |
| activate_noawait | `--deb-activate-noawait` | ✅ `deb.triggers.activate_noawait` | ✅ `deb.triggers_activate_noawait` |
| interest_await | ❌ | ❌ | ✅ `deb.triggers_interest_await` |
| activate_await | ❌ | ❌ | ✅ `deb.triggers_activate_await` |
| maintainer scripts force-errorchecks | ❌ | ❌ | ❌ |
| rules file | ❌ | ❌ | ✅ `deb.rules` |
| custom control file | `--deb-custom-control` | ❌ | ❌ |
| shlibs | `--deb-shlibs` | ❌ | ❌ |
| init script | `--deb-init` | ❌ | ❌ |
| systemd unit | `--deb-systemd` | ❌ | ❌ |
| upstart script | `--deb-upstart` | ❌ | ❌ |
| /etc/default file | `--deb-default` | ❌ | ❌ |
| meta file | `--deb-meta-file` | ❌ | ❌ |

### RPM-specific triggers

| Feature | fpm | nfpm | lx |
|---|---|---|---|
| RPM triggers (4 types) | `--rpm-trigger-*` | ❌ | ✅ `rpm.trigger_*` (deps + best-effort script) |
| digest algorithm | `--rpm-digest sha256` | ❌ | ❌ |
| compression | `--rpm-compression xz` | ✅ `rpm.compression` | ✅ `rpm.compression` (gzip/xz/lzma/zstd/none) |
| AutoProv/AutoReq | `--rpm-autoprov` | ❌ | ✅ `rpm.auto_provides` / `rpm.auto_requires` (best-effort: sonames scanned from the payload) |
| macro expansion | `--rpm-macro-expansion` | ❌ | ⚠️ `rpm.defines` accepted but not applied (no rpmbuild macro engine; warned) |
| rpmbuild define | `--rpm-rpmbuild-define` | ❌ | ⚠️ `rpm.defines` accepted but not applied (no rpmbuild macro engine; warned) |
| custom tag | `--rpm-tag` | ❌ | ❌ |

### Build-time hooks (lx only)

| Hook | lx |
|---|---|
| Pre-build shell steps | `prebuild_steps: ["go generate"]` |
| Custom build commands | `build_commands: ["make all"]` |
| Custom install commands | `install_commands: ["make install DESTDIR=$DESTDIR"]` |

### Script templating

| | fpm | nfpm | lx |
|---|---|---|---|
| Template engine | ERB (`--template-scripts`) | ❌ | `<%= key %>` expressions (`template_scripts: true`) |
| Template values | `--template-value KEY=VALUE` | — | Auto: name, version, maintainer, description, homepage, license, arch, dist, iteration, epoch, vendor, packager, prefix |
| Example | `--after-install script.sh --template-scripts --template-value name=myapp` | — | `template_scripts: true` + `<%= name %>` in script |

---

## 6. Signing & verification

| Feature | fpm | nfpm | lx |
|---|---|---|---|
| deb signing | ❌ | ✅ debsign + dpkg-sig | ✅ debsign + detach |
| rpm signing | `--rpm-sign` (rpmbuild) | ✅ PGP embedded | ✅ PGP embedded |
| apk signing | ❌ | ✅ RSA PEM | N/A |
| msix signing | ❌ | ✅ PFX | N/A |
| Checksum verification | ❌ | ❌ | ✅ fail-closed by default |
| SBOM output | ❌ | ❌ | ✅ SPDX 2.3 |
| SLSA provenance | ❌ | ❌ | ✅ SLSA v1 |
| Reproducible builds | 🟡 `--source-date-epoch-default` | ✅ SOURCE_DATE_EPOCH | ✅ SOURCE_DATE_EPOCH + release timestamps |
| Asset provenance audit | ❌ | ❌ | ✅ `--summary` provenance array |

---

## 7. Configurability & ergonomics

| Feature | fpm | nfpm | lx |
|---|---|---|---|
| Config file | ❌ (CLI only) | ✅ `nfpm.yaml` | ✅ `package.yaml` |
| JSON Schema | ❌ | ✅ published schema | ✅ generated via `lx schema` |
| Interactive init | ❌ | ✅ `nfpm init` | ✅ `lx init`, `--from-aur` |
| Zero-config URL build | ❌ | ❌ | ✅ |
| Dry-run | ❌ | ❌ | ✅ `--dry-run` |
| Validate without building | ❌ | ❌ | ✅ `lx validate` |
| Auto-discover patterns | ❌ | ❌ | ✅ `lx discover` |
| Dependency scanner | ❌ | ❌ | ✅ `lx scan-deps` |
| Glob patterns | ✅ (`-x` excludes) | ✅ `contents:` src | ✅ `contents:` src |
| Disable globbing | ❌ | ✅ `disable_globbing` | ✅ `disable_globbing` |
| Exclude patterns | ✅ `-x pattern` | ❌ | ❌ |
| Per-file mode/owner/group | ✅ `--rpm-attr`, `--deb-user/group` | ✅ `file_info` | 🟡 global umask |
| Changelog | `--deb-changelog`, `--rpm-changelog` | ✅ `changelog: file.yml` | ✅ auto-generated |
| Installed size | `--deb-installed-size` | ❌ | ❌ |
| Config files marking | ✅ `--config-files` | ✅ `type: config` | ✅ `type: config` |
| Config file noreplace | ❌ | ✅ `type: config\|noreplace` | ✅ `type: config\|noreplace` |
| Config file missingok | ❌ | ✅ `type: config\|missingok` | ✅ `type: config\|missingok` |
| Tree type | ❌ | ✅ `type: tree` | ✅ `type: tree` |
| Symlink type | ❌ | ✅ `type: symlink` | ✅ `type: symlink` |
| Dir type | ❌ | ✅ `type: dir` | ✅ `type: dir` |
| Ghost type (RPM) | ❌ | ✅ `type: ghost` | ✅ `type: ghost` |
| Per-packager content filter | ❌ | ✅ `packager: deb` | ✅ `packager: deb` |
| Disown subtree | ❌ | ✅ `disown_subtree` | ❌ |
| Env var expansion | ❌ | ❌ | ✅ `${VAR}` / `${VAR:-default}` |
| Overlay config merge | ❌ | ❌ | ✅ `apply_overlay()` |
| Expanded paths | ❌ | ✅ `expand: true` | ❌ |
| Options file | ✅ `--fpm-options-file` | ❌ | ❌ |
| Musl-static builds | ❌ | ❌ | ✅ `musl: true` |
| `from-dir`/`from-file` mode | ❌ | ❌ | ✅ `--from-dir`/`--from-file` |
| `--prefix` custom install path | ✅ `--prefix` | ❌ | ✅ `prefix:` / `--prefix` |
| **Language PM inputs** | ✅ npm/gem/python/cpan/pear | ❌ | ✅ `registry_source: npm/python/gem/cargo/go/hex/dart/nuget/maven/composer/cpan` (plugin) |

---

## 7a. Musl-static builds (old-distro portability)

A common Linux packaging problem: a binary built on a newer distro won't
run on older ones because glibc uses symbol versioning — a binary linked
against glibc 2.28 can't load on a system with glibc 2.17. Neither fpm nor
nfpm addresses this; the packager must handle it upstream (e.g. build in
an old-distro container, which inherits unpatched packages).

**lx** solves this with `musl: true` in package.yaml, producing a
musl-static binary with **no glibc dependency** — it runs on any Linux
regardless of distro age.

| Capability | fpm | nfpm | lx |
|---|---|---|---|
| Musl-static source builds | ❌ | ❌ | ✅ `musl: true` (cargo, go, cmake, custom) |
| Prefer musl release assets | ❌ | ❌ | ✅ auto-discovery prefers `*-musl*` assets |
| Omit `libc6` from Depends | ❌ | ❌ | ✅ `compute_depends()` skips libc6 fallback |
| Consumer musl fallback | ❌ | ❌ | ✅ `lx get install` falls back to `+musl_{arch}.deb` |
| `scan-deps --prefer-musl` | ❌ | ❌ | ✅ inspect what a musl binary's deps would be |

How it works per build system:

| `build_system` | Musl mechanism |
|---|---|
| `cargo` | `--target x86_64-unknown-linux-musl` (auto-installed via rustup) |
| `go` | `CGO_ENABLED=0` (fully static, no glibc) |
| `cmake` | `musl-gcc`/`musl-g++` + `-static` (requires `musl-tools`) |
| `custom` | user's responsibility via `build_commands` |

---

## 8. Consumer / distribution features

| Feature | fpm | nfpm | lx |
|---|---|---|---|
| Consumer CLI | ❌ | ❌ | ✅ `lx get` |
| Local manifest | ❌ | ❌ | ✅ `installed.json` |
| Rollback | ❌ | ❌ | ✅ `lx rollback` |
| Repo generation | ❌ | ❌ | ✅ `lx repo` |
| Search | ❌ | ❌ | ✅ `lx search` |
| Migrate snap/flatpak/nix → native | ❌ | ❌ | ✅ `lx go-native` |
| AUR import | ❌ | ❌ | ✅ `lx init --from-aur` |
| Lintian | ❌ | ❌ | ✅ `--lintian` |
| GitHub Action | ❌ | ❌ | ✅ `action.yml` |
| Sandboxed builds | ❌ | ❌ | ✅ `--sandbox` |
| Repo format conversion | ✅ (`-s rpm -t deb`) | ❌ | ✅ `lx convert` (deb↔rpm↔arch, native rebuild with scriptlet carry-over) |

---

## 9. Ecosystem & tooling

| Aspect | fpm | nfpm | lx |
|---|---|---|---|
| Age | 2011 (14+ years) | 2018 (7+ years) | 2026 (new) |
| Stars | 11.5k | 2.6k | — |
| Contributors | 200+ | 100+ | — |
| Package manager | Ruby gem | Go module / binary | Cargo / binary |
| CI integration | Any CI (shell) | Any CI (binary) | GitHub Action + binary |
| Library API | ❌ | ✅ Go library | 🟡 Rust (not yet public) |
| Completions | ❌ | ✅ bash/fish/zsh/powershell | ✅ bash/fish/zsh/powershell |

---

## Summary: when to use which

| Scenario | Best choice | Why |
|---|---|---|
| Convert RPM→deb or any format→any format | **fpm** | Unique conversion capability |
| Package a Python/gem/npm/CPAN module | **fpm** | Auto-downloads from language package managers |
| macOS .pkg / FreeBSD / Solaris / snap | **fpm** | Only tool supporting these |
| CI/CD packaging with rich per-format control | **nfpm** | Mature, binary, YAML config, per-file metadata |
| Windows msix output | **nfpm** | Only Linux-focused tool with msix |
| Package a GitHub release end-to-end | **lx** | Auto-fetch, verify, auto-install ancillaries |
| Supply-chain security (SBOM, verification) | **lx** | Checksum verification, SBOM, SLSA provenance |
| Consumer install/upgrade/rollback workflow | **lx** | Only tool with consumer CLI + repo serving |
| Source packages (`.dsc`, `.src.rpm`) | **lx** | Only tool producing source packages |
| Zero-config packaging from a URL | **lx** | `lx build https://github.com/owner/repo` |
| Binary that runs on old distros | **lx** | `musl: true` — no glibc dependency |
| No runtime dependencies | **nfpm** or **lx** | Both are single static binaries |
| Maximum format coverage | **fpm** | 15+ formats |
| RPM compression control | **nfpm** or **lx** | Both support `rpm.compression` (gzip/xz/lzma/zstd/none) |
| RPM AutoProv/AutoReq/macros | **fpm** or **lx** | fpm via flags; lx via `rpm.auto_provides`/`auto_requires`/`defines` |
| Upgrade-time scripts (pre/post-upgrade) | **fpm** or **lx** | fpm for rpm/deb/pacman; lx for deb/rpm/arch |
| Script templating in packaging scripts | **fpm** or **lx** | fpm uses ERB; lx uses `<%= key %>` expressions |
| Format conversion | **fpm** or **lx** | fpm: byte conversion (`-s rpm -t deb`); lx: native rebuild with scriptlet carry-over |

### Philosophical differences

- **fpm** is the swiss-army knife: "give me anything, I'll make it a
  package." It optimizes for breadth — of inputs, of outputs, of
  platforms. The cost is dependencies (Ruby) and shallow per-format
  control.

- **nfpm** is the precision instrument: "give me files, I'll make them
  a perfect package." It optimizes for correctness — per-file metadata,
  per-format overrides, format-specific signing. The cost is you must
  bring the files yourself.

- **lx** is the opinionated pipeline: "point me at a forge repo, I'll
  handle everything." It optimizes for automation — auto-fetch, verify,
  auto-install ancillaries, serve a repo, manage upgrades, and produce
  musl-static binaries that run on any Linux. The cost is it primarily
  handles forge releases (though `--from-dir`/`--from-file` covers
  arbitrary files) and has fewer output formats than fpm.
