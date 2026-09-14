# Evaluation: composability

Part of the [lx docs](../README.md). Companion evaluation:
[Interoperability](interoperability.md).

`lx` spans a pipeline that most tools split across several programs:
discover a release, verify it, unpack it, compile it, stage a filesystem
tree, write one or more native archives, sign them, index them, and hand the
result to the host package manager. Composability is whether those pieces
are independent parts you can select and recombine — rather than one
hard-coded path that only runs end to end — and whether a new part can be
added without editing the core.

This evaluation fixes the rubric first, then walks the repository for
evidence and records the gaps.

---

## Rubric

| # | Criterion | "Pass" means | Verdict |
|---|---|---|---|
| C1 | **Orthogonal selection** | Each axis (format, source, build system, …) is chosen independently, with no hidden coupling | ✅ |
| C2 | **Open/closed extension** | A new capability is an `impl` plus one registration line, with no core-pipeline edits | 🟡 |
| C3 | **Shared substrate** | Behaviors shared by more than one plugin live in one place, not copied | ✅ |
| C4 | **Stage composition** | Whole stages compose into new pipelines (`build` → `repo` → `publish`, `convert`) | ✅ |
| C5 | **Config composition** | One config can target multiple formats and overlay per-format behavior | 🟡 |
| C6 | **Library reuse** | The building blocks are usable outside the `lx` binary | 🟡 |
| C7 | **Escape hatches** | A project that does not fit a plugin can compose its own build without forking | ✅ |
| C8 | **Failure isolation** | A gap in one axis degrades that axis, not the whole pipeline | ✅ |

Verdicts follow the [evaluation scale](README.md#method).

---

## C1 — Orthogonal selection

The core is a **format-agnostic pipeline plus eight independent plugin
dimensions**, each with its own registry and selection mechanism
(`docs/architecture/overview.md`):

| Dimension | Count | Selected by |
|---|---|---|
| Packager | 5 (deb, rpm, arch, apk, ipk) | `--format` / `package_format:` |
| ForgeSource | 9 (github, gitlab, gitea, forgejo, bitbucket, gerrit, gitee, sourceforge, custom) | `--source` / URL sniffing / `source: custom` |
| BuildSystem | 7 (cmake, cargo, go, meson, autotools, make, custom) | `build_system:` / auto-detect |
| RegistrySource | 11 (npm, python, gem, cargo, go, hex, dart, nuget, maven, composer, cpan) | `registry_source:` |
| ArtifactFormat | 6 (tar.gz, tar.xz, tar.zst, tar, zip, raw) | `artifact_format:` / filename |
| Signer | 4 (gpg-detach, rpm-pgp, deb-debsign, apk-rsa) | `(package_format, sign_method)` |
| DependencyMapper | 5 (debian, rpm, pacman, alpine, openwrt) | target `package_format` |
| PackageIndex | 8 — 5 write + 3 read | `lx repo --format` / `indexes.yaml` |

The dimensions are genuinely orthogonal, not a menu of presets: a project
can fetch from npm (`registry_source:`) instead of a forge, compile with
`custom` build commands instead of a recognized build system, unpack a
`tar.zst` asset, and emit Arch — each choice made separately. `Source` and
`RegistrySource` are deliberately **not** merged, because merging would
force an enum wrapper and a `match` at every call site
(`docs/architecture/plugins.md`).

`PluginSet` (`lib/plugins/plugin.rs`) is the shared registry primitive —
`get`/`names`/`first`/`take`/`take_first` over a static, order-sensitive
list — and `plugin_identity!` supplies `name`/`description` so each impl
carries only behavior. `lib/plugins/mod.rs` shows the pattern in miniature:
`all_packagers()`, `get_packager()`, `packager_names()`, and
`expand_formats()`, where `--format all` expands to the registry's names
rather than a hard-coded list.

## C2 — Open/closed extension (partial)

Adding a capability is designed to be a local change. The
[architecture docs](../architecture/plugins.md) prescribe the exact steps,
and each ends the same way: implement the trait, add one line to the
dimension's `all_*()` registry. For example, a new build system is
`lib/plugins/build_system/<name>.rs` + one entry in `all_build_systems()`;
`sourcebuild.rs` is build-system-agnostic and only calls `build_sys.build()`.

This is 🟡 rather than ✅ because "open" has limits:

- **Registration is static and explicit.** There is no `dlopen`, no feature
  flags, no config-file plugin loading (`lib/plugins/plugin.rs` says so
  directly). A new plugin is a compile.
- **The trait set is an internal contract, not a published API.** It is
  stable enough to add to, but it is not semver'd for third parties.
- **The new capability still has to be reachable.** A new `PackageIndex`
  backend is one registration line, but a *custom* read-index source is
  not yet a plugin at all (`SourceKind::Custom` returns `None` from
  `plugin_kind()`, `lib/index/registry.rs`).

## C3 — A shared substrate, not eight copies

The dimensions share implementation rather than re-deriving it:

| Shared piece | Location | Used by |
|---|---|---|
| `BuildContext` | `lib/plugins/mod.rs` | every packager |
| `stage_install_tree()` | `lib/plugins/mod.rs` | deb, rpm, arch, apk, ipk |
| `apply_contents()` / `apply_contents_with_config()` | `lib/plugins/mod.rs` | packagers + per-format `%config`/`backup` handling |
| `resolve_homepage()`, `format_release()`, `output_dir()` | `lib/plugins/mod.rs` | every packager |
| `PACKAGED_FROM_LINE`, `render_extra_fields()` | `lib/plugins/mod.rs` | binary control *and* source `debian/control` |
| `RawGetter` (+ `check_sidecar`) | `lib/checksum.rs` | every forge source |
| Archive builders | `lib/{deb,rpm,arch,apk,ipk}archive.rs` | packager plugins **and** `lx convert` |
| Package metadata rendering | `lib/pkgmeta.rs` | binary packages and source packages |
| Dependency tables | `lib/depmap.rs` | every `DependencyMapper` |

The DRY rule is enforced by absence of the obvious failure mode: the
format-specific plugins do not each contain their own tar walk, checksum
verifier, or changelog renderer. `stage_install_tree` in particular is the
single format-agnostic placement pass — bundle vs flat, `binary_rename`,
man-page/license staging — that every packager calls before writing its
archive.

## C4 — Stage composition

Stages compose into pipelines that are more than the sum of their commands:

```text
lx build <url>            acquire ──▶ verify ──▶ unpack ──▶ stage ──▶ packager
lx build --format all     … one acquire/verify pass, every registered packager
lx publish                build every format ──▶ write that format's repo index
lx repo <dir>             index an existing artifact directory, per format
lx convert <pkg> --to X   read format A ──▶ reuse plugin pipeline ──▶ format B
lx index <cmd>            fan out over enabled read backends (aur / repology / lx-community)
```

`lx publish` (`lib/publish.rs`) is the clearest example: it is literally
`build` followed by `repo`, per format, into `<output>/<format>/`, so the
producer→distributor loop is one command instead of a hand-written matrix.
`lx convert` composes in the other direction: it reads a foreign package,
then **reuses the same packager plugins** rather than writing a second
emitter. `--format deb,rpm` composes a subset; `--format all` composes the
registry.

## C5 — Config composition (partial)

`package.yaml` composes behavior declaratively:

- **`overrides:`** — per-format relation-field overrides keyed by
  `deb`/`rpm`/`arch`; an overridden field *replaces* the top-level value for
  that format (`docs/reference/package-yaml.md`).
- **`contents[].packager`** — a contents entry applies only to the named
  format, so one `contents:` list can hold format-specific files
  (`apply_contents_with_config`, `lib/plugins/mod.rs`).
- **Per-format blocks** — `deb:` (debconf, triggers, rules) and `rpm:`
  (triggers, compression, auto-deps) are ignored by the other formats
  rather than erroring.
- **`distribution_arch_overrides`** / `architectures:` — replace or pin the
  arch/suite matrix.
- **`fields:`** — arbitrary control fields, de-duplicated against the
  fields the renderer already emits.
- **`template_scripts` + `<%= key %>`** — maintainer scripts compose with
  build values (`lib/templating.rs`).
- **`registry_source:` + dependency mapping** — registry manifests are
  mapped to the target format's names and syntax (`lib/depmap.rs`).

It is 🟡 because per-file metadata is not uniform across formats:
`contents[].file_info` carries `mode`/`owner`/`group`/`mtime`/`lang`, but
`lang` maps only to RPM's `%lang` (other formats ignore it, matching nfpm),
so one cross-format config cannot attach a language outside the RPM payload.

## C6 — Library reuse (partial)

`lx` is structured as a library with a thin binary. `src/main.rs` only calls
`cli::run`; everything else lives in `lib/`, whose `lib.rs` documents the
reusable building blocks (`cache`, `checksum`, `debarchive`, `elfdeps`,
`github`, `lintian`, `optimize`, `pkgmeta`, `sign`, …) and keeps
`extern crate self as lx_lib` so code written against the old crate path
still works. Integration tests exercise internals directly
(`lx_lib::debarchive::build`, `lx_lib::archarchive::build`), which is only
possible because the pipeline is not hidden behind the CLI.

The library-first posture is explicit: `docs/decisions/2026-08-14-library-first-audit.md`
audits hand-rolled modules against existing crates and keeps the custom code
only where it is the right tool (a ~20-line TTY bar vs a 50-dependency
progress library).

It is 🟡 because `lx_lib` is **reuse-oriented, not a published API**: it is
one crate in this repository, not a versioned library other projects pin.
The comparison table already notes "Can be a library: not yet public API"
(`docs/comparison/lx-vs-nfpm.md`).

## C7 — Escape hatches

When no plugin fits, the pipeline exposes composition points instead of
forcing a fork:

- **`source: custom`** — `upstream_url` with `{version}`/`{arch}`/
  `{package_name}` placeholders, so a download URL that no forge plugin
  models is still a first-class source (`lib/plugins/forge/custom.rs`).
- **`build_system: custom`** — `build_commands`/`install_commands` with
  `$DESTDIR` set, the universal escape hatch for npm, waf, python, or any
  build system without a dedicated plugin (`lib/plugins/build_system/custom.rs`).
- **`prebuild_steps`** — host commands run before the build system.
- **`--from-dir` / `--from-file` / `--prefix`** — fpm's "you supply files"
  mode, bypassing forge and build entirely (`stage_install_tree`'s
  custom-prefix branch).
- **`contents:` DSL** — globs, `disable_globbing`, `tree`, `symlink`,
  `config`/`config|noreplace`, `dir`, `ghost`, per-packager entries.
- **`registry_source`** — 11 ecosystems plugged in without a new packager.

## C8 — Failure isolation

A plugin gap degrades one axis, not the pipeline:

- An unknown build system is an explicit error at resolution, after
  `detect_build_system`, rather than a half-built tree
  (`lib/sourcebuild.rs`).
- `rpm.defines` is accepted but not applied; `lx` warns rather than
  silently dropping it (`docs/architecture/replacements.md`).
- The ELF dependency scan is an improvement with a fallback: a soname with
  no dpkg information falls back to `dpkg -S`/`rpm -q`/`pacman -Qo` instead
  of failing the build (`lib/shlibdeps.rs`).
- The read-index fan-out skips disabled/absent backends rather than
  aborting the whole `lx index` call.

## Where composability is partial

Collected, so the ✅ rows above are not read as universal:

- **Static registration, no dynamic loading.** Adding a plugin recompiles
  the crate; there is no plugin file on disk to drop in.
- **The plugin API is internal**, not semver'd or published (see C6).
- **`RegistrySource` is single-arch** — it returns one payload and does not
  compose with the multi-arch release-asset model
  (`docs/architecture/plugins.md`).
- **`SourceKind::Custom` is not a plugin yet** (`lib/index/registry.rs`).
- **`rpm.defines` accepted, not applied; `default_distributions()`** on
  `Packager` is present but "not yet called by the live
  distribution-resolution path" (`lib/plugins/mod.rs`), which still reads
  the `DEFAULT_*_DISTRIBUTIONS` constants.
- **`custom` build system is a shell escape hatch**, not a trait-level
  plugin with `recognize()`/`required_tools()`.
- **Index backends are less uniform than packagers:** the apt write backend
  delegates to the original `lib/repo.rs`, while the others read each
  artifact's metadata in-process (`docs/architecture/plugin-catalog.md` §12).
- **Per-file metadata is not composable across formats** (see C5).
- **`rpm.auto_provides`/`auto_requires` are best-effort** ELF scans, not
  full rpmbuild macro expansion.

## Verifying these claims

```sh
# The registries and their selection
cargo test --test plugins          # packager registry, staging, archive magic
cargo test --test plugins_artifact # artifact-format registry + detection
cargo test --test plugins_signer   # signer selection by (format, method)
cargo test --test plugins_depmap   # per-format dependency mapping
cargo test --test plugins_repo     # package-index write backends

# Composition in practice
lx build package.yaml --format all # one config → every registered packager
lx publish package.yaml            # build + per-format index in one run
lx convert foo.rpm --to deb        # foreign format → plugin pipeline → deb

# Library reuse (the crate is `lx`; the library target is `lx_lib`)
cargo doc --no-deps -p lx          # the lib/ building blocks
```

## See also

- [Interoperability](interoperability.md) — the companion evaluation.
- [`docs/architecture/plugins.md`](../architecture/plugins.md) — the eight
  dimensions, wiring, and how to add a plugin.
- [`docs/architecture/plugin-catalog.md`](../architecture/plugin-catalog.md) —
  trait definitions and per-plugin detail.
- [`docs/reference/package-yaml.md`](../reference/package-yaml.md) — the
  composition surfaces in config.
- [`docs/decisions/2026-08-14-library-first-audit.md`](../decisions/2026-08-14-library-first-audit.md) —
  why the hand-rolled modules are the right tool.
