# fpm vs nfpm vs lx — lifecycle & verdict

Sections 7–9 and the summary, split out from
[fpm-vs-nfpm-vs-lx.md](fpm-vs-nfpm-vs-lx.md) (formats, inputs, relations,
metadata, scripts, and signing).

## 7. Configurability & ergonomics

| Feature | fpm | nfpm | lx |
|---|---|---|---|
| Config file | ❌ (CLI only) | ✅ `nfpm.yaml` | ✅ `package.yaml` |
| JSON Schema | ❌ | ✅ published schema | ✅ generated via `lx schema` |
| Interactive init | ❌ | ✅ `nfpm init` | ✅ `lx init`, `--from-aur` |
| Zero-config URL build | ❌ | ❌ | ✅ |
| Dry-run | ❌ | ❌ | ✅ `--dry-run` |
| Validate without building | ❌ | ❌ | ✅ `lx validate` |
| Auto-discover patterns | ❌ | ❌ | ✅ `lx init --from` |
| Dependency scanner | ❌ | ❌ | ✅ `lx deps scan` |
| Glob patterns | ✅ (`-x` excludes) | ✅ `contents:` src | ✅ `contents:` src |
| Disable globbing | ❌ | ✅ `disable_globbing` | ✅ `disable_globbing` |
| Exclude patterns | ✅ `-x pattern` | ❌ | ❌ |
| Per-file mode/owner/group | ✅ `--rpm-attr`, `--deb-user/group` | ✅ `file_info` | ✅ `contents[].file_info` |
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
| Disown subtree | ❌ | ✅ `disown_subtree` | ✅ `contents[].disown_subtree` |
| Env var expansion | ❌ | ❌ | ✅ `${VAR}` / `${VAR:-default}` |
| Overlay config merge | ❌ | ❌ | ✅ `apply_overlay()` |
| Expanded paths | ❌ | ✅ `expand: true` | ✅ `contents[].expand` |
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
| `deps scan --prefer-musl` | ❌ | ❌ | ✅ inspect what a musl binary's deps would be |

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
| Repo generation | ❌ | ❌ | ✅ `lx repo` (deb/rpm/pacman/apk/opkg) + `lx publish` |
| Search | ❌ | ❌ | ✅ `lx search` |
| Migrate snap/flatpak/nix → native | ❌ | ❌ | ✅ `lx migrate native` |
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
| Library API | ❌ | ✅ Go library | ✅ `lx_lib::api` (Rust) |
| Completions | ❌ | ✅ bash/fish/zsh/powershell | ✅ bash/fish/zsh/powershell |

---

## Summary: when to use which

| Scenario | Best choice | Why |
|---|---|---|
| Convert RPM→deb or any format→any format | **fpm** | Unique conversion capability |
| Package a Python/gem/npm/CPAN module | **fpm** | Auto-downloads from language package managers |
| macOS .pkg / FreeBSD / Solaris / snap | **fpm** | Only tool supporting these |
| CI/CD packaging with rich per-format control | **nfpm** | Mature, binary, YAML config, per-file metadata |
| Windows msix output | **nfpm** (PFX) or **lx** (PEM key+cert) | both sign natively |
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
