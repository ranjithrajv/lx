# Evaluation: composition over inheritance

Part of the [lx docs](../README.md). Companion evaluations:
[Interoperability](interoperability.md) and
[Composability](composability.md).

Rust has no class inheritance, so the "inheritance" this evaluation is about
is not `class B extends A`. It is the code shape that grows in its place when
a program spans many variants: a **god base type** with a method for every
capability, a variant **`enum`** whose every arm is matched again at each
call site, and **copy-pasted per-variant code** standing in for a shared
superclass. Composition over inheritance asks whether `lx` builds behaviour
out of small, independent parts that are held and invoked polymorphically,
rather than out of one widening type hierarchy or a `match` repeated wherever
a variant is used.

This evaluation fixes the rubric first, then walks the repository for
evidence and records the gaps.

---

## Rubric

| # | Criterion | "Pass" means | Verdict |
|---|---|---|---|
| C1 | **Capability-sized interfaces** | Each trait models one concern; no base type carries unrelated capabilities | ✅ |
| C2 | **Polymorphism at the seam** | Callers hold a `Box<dyn Trait>` from a registry and invoke it; no variant `match` grows at the call site | ✅ |
| C3 | **Shared behaviour by composition** | Behaviour common to several impls lives in one composed helper that each impl calls, not copied into each impl | ✅ |
| C4 | **Defaults instead of required overrides** | A new impl overrides only what differs; the trait supplies the rest | ✅ |
| C5 | **Identity is data, not behaviour** | `name`/`description` are declared once per type, not re-declared per trait | ✅ |
| C6 | **Shallow, acyclic trait graph** | Traits compose by a single identity supertrait; no deep or diamond hierarchy | ✅ |
| C7 | **Additive extension** | A new variant is a new file plus one registry line, with no core-pipeline edits | ✅ |
| C8 | **Residual dispatch is explicit and bounded** | Where an `enum`/`match` has not been replaced, it is narrow, documented, and off the plugin-dispatch path | ✅ |

Verdicts follow the [evaluation scale](README.md#method).

---

## C1 — Capability-sized interfaces

Each plugin dimension is a trait that answers exactly one question, sized to
that question rather than to a shared base:

| Trait | Question | Methods |
|---|---|---|
| `Packager` (`lib/plugins/mod.rs`) | What artifact does a format produce? | 4 required + 4 provided + identity |
| `ForgeSource` (`lib/plugins/forge/mod.rs`) | What releases/assets exist? | 10 + identity |
| `BuildSystem` (`lib/plugins/build_system/mod.rs`) | How is a source tree compiled? | 3 (2 provided) + identity |
| `RegistrySource` (`lib/plugins/registry/mod.rs`) | Give me these files from a registry | 2 (1 provided) + identity |
| `ArtifactFormat` (`lib/plugins/artifact/mod.rs`) | How is an archive unpacked? | 3 (1 provided) + identity |
| `Signer` (`lib/plugins/signer/mod.rs`) | How is an artifact signed? | 3 (1 provided) + identity |
| `DependencyMapper` (`lib/plugins/depmap/mod.rs`) | How is a dependency rendered for a format? | 5 (2 provided) + identity |
| `PackageIndex` (`lib/plugins/package_index/mod.rs`) | Publish or read an index | role defaults + identity |

The sizes differ by an order of magnitude — 2 methods to 10 — which is the
point: no single interface was inflated to cover every variant. `ForgeSource`
carries the discovery surface because a forge genuinely has many discovery
operations; `RegistrySource` carries two because a registry genuinely has
two fetch operations. The architecture doc records the decision **not** to
merge them into one type even though both answer "where does the package come
from?": a merged trait would either carry methods that are `unimplemented!()`
for one side, or drop the other side's capabilities
(`docs/architecture/plugins.md`).

Cross-cutting concerns are separate traits rather than methods bolted onto a
base: `RawGetter` (`lib/checksum.rs`) is a one-method trait for "fetch these
bytes", implemented by each forge client and consumed generically by
`check_sidecar`. `PackageIndex` splits its two roles with
`Capabilities::{READ, WRITE}` (`lib/plugins/package_index/mod.rs`) rather than
requiring every backend to implement both.

## C2 — Polymorphism at the seam

Callers do not branch on which plugin they have; they ask a registry for a
`Box<dyn Trait>` and call the trait method. The registry is the single
dispatch point:

```rust
// lib/plugins/mod.rs
pub fn get_packager(name: &str) -> Option<Box<dyn Packager>> {
    PluginSet::new(all_packagers()).take(&crate::config::canonical_format(name))
}
```

The pattern is repeated per dimension — `get_forge_source`,
`get_build_system`/`detect_build_system`, `get_registry_source`,
`get_artifact_format`/`detect_artifact_format`, `signer_for`,
`get_dependency_mapper`, `get_index_backend` — and each returns a trait
object. `signer_for(format, method)` is the clearest replacement of
inheritance-flavoured branching: the old shape was `if format == "deb"` /
`if format == "rpm"` inside the build path; now
`signer::apply_post_build(format, method, …)` asks `signer_for` and the
*first registered backend whose `supports()` returns true* wins, and the
same helper serves both the binary build and the source-build wrapper
(`lib/plugins/signer/mod.rs`). Likewise `depmap.rs` no longer carries a
growing `match format`; it delegates to `get_dependency_mapper(format)`
(`docs/architecture/plugins.md` §12).

`PluginSet` (`lib/plugins/plugin.rs`) is the shared registry primitive behind
every lookup — `get`/`names`/`first`/`take`/`take_first` — so a dimension's
free functions are one-liners and no dimension re-invents list traversal.

## C3 — Shared behaviour by composition

When several impls would otherwise "inherit" the same code by copying it, the
code lives in one helper the impls call. The evidence is the recent refactor
series, each of which removed duplication without changing a public
signature:

| Shared helper | Location | Replaces |
|---|---|---|
| `Plugin` + `PluginSet` + `plugin_identity!` | `lib/plugins/plugin.rs` | Per-dimension `name`/`description` and `all_*`/`get_*`/`*_names` scaffolding |
| `stage_install_tree()`, `apply_contents*()` | `lib/plugins/mod.rs` | Per-packager file placement |
| `registry/staging.rs` (`run_tool`, `find_downloaded_archive`, `extract_payload`, `unwrap_lone_dir`, `description_or`) | `lib/plugins/registry/staging.rs` | Per-registry external-command + extract + describe plumbing |
| Package metadata rendering | `lib/pkgmeta.rs` | Binary *and* source control/changelog/copyright templates |
| `build_index`/`sign_index` defaults, `artifacts_with_ext`, `parse_key_value` | `lib/plugins/package_index/mod.rs` | Per-backend index plumbing |
| `check_sidecar` + `RawGetter` | `lib/checksum.rs` | `build.rs`'s and `debs.rs`'s separate sidecar verifiers |

Concretely, commit `05d5425` ("Add a shared staging pipeline for
tool-backed registry sources") reduced `npm`, `python`, `gem`, `cargo`,
`composer`, `cpan`, `nuget`, and `maven` to their ecosystem-specific bits by
extracting `staging.rs`, and `818c283` finished the coverage for `dart`,
`go`, and `hex`. Commit `9889603` moved identity out of seven traits into
`plugin_identity!` — 56 files, `+464 −465`, with the public
`all_*`/`get_*`/`*_names` signatures unchanged so callers were untouched.
This is composition doing the job inheritance would otherwise be reached for:
the shared behaviour is a *call*, visible at each impl, not a base class
whose changes silently propagate.

The DRY rule is also enforced by a real bug, not just tidiness:
`docs/decisions/2026-08-21-dry-solid-cleanup.md` records that the parallel
rendering in `source.rs` had drifted from `build.rs` (hardcoded changelog
date and copyright year) and that the two sidecar verifiers had diverged
enough that `--allow-unverified` could swallow a genuine checksum mismatch.
Both were closed by composing the one shared implementation.

## C4 — Defaults instead of required overrides

Where a capability is optional, the trait provides a default so an impl that
does not care never writes the method at all:

- `BuildSystem::recognize` defaults to `false` and `required_tools` to empty
  (`lib/plugins/build_system/mod.rs`), so only auto-detecting systems write
  `recognize` and only tool-backed systems write `required_tools`.
- `ArtifactFormat::aliases` defaults to empty (`lib/plugins/artifact/mod.rs`).
- `DependencyMapper::repo_family` defaults to `None` and `render` to the
  `name (constraint)` form that Debian and RPM share; Alpine overrides only
  `render` (`lib/plugins/depmap/mod.rs`).
- `Signer::embedded` defaults to `false`; embedded backends flip it.
- `Packager::artifact_glob` defaults to `{package}_*.{file_extension}`, so
  the convention follows from `file_extension` unless a format's filename
  differs (`rpm`, `arch`, `apk`, `osxpkg` override).
- `Packager::supports_source_build`/`archive_staged_tree`/
  `generate_source_package` default to "no source build" / a clear error;
  `deb`, `rpm`, and `arch` override them (`lib/plugins/mod.rs`).
- `PackageIndex` gives every role method a default that fails with an
  actionable message keyed off `id`/`capabilities`, so a backend implements
  only the half it supports. `id` itself defaults to `Plugin::name`, and
  `instance_name` defaults to `id` (`lib/plugins/package_index/mod.rs`).

The effect is that a new impl's diff is proportional to what is actually
different, and the defaults are the written-down contract of what "not
overriding" means.

## C5 — Identity is data, not behaviour

`name`/`description` used to be re-declared on every dimension trait and
re-implemented in every impl. Now they live on one supertrait, `Plugin`, and
are declared per type with a one-line macro:

```rust
plugin_identity!(ApkRsa, "apk-rsa", "Alpine apk v2 RSA/SHA-1 signature");
```

`lib/plugins/plugin.rs` documents the conversion and the macro expands to an
`impl Plugin` with the two string constants. The refactor reached all eight
dimensions, `PackageIndex` last (`7f1fcbd`), where `id` became a provided
default over `Plugin::name` so existing call sites such as `instance_name()`
kept working. `IndexBackend::description()` was then changed to ask the
backend instance instead of storing a duplicate string (`bc6f564`,
`lib/plugins/package_index/mod.rs`). Identity that is data cannot drift from
the trait as the trait grows.

## C6 — A shallow, acyclic trait graph

The trait graph is two levels deep and has no diamonds: every plugin trait
extends `Plugin` and nothing else; `Plugin` extends nothing but the marker
bounds `Send + Sync`.

```rust
pub trait Plugin: Send + Sync { /* name, description */ }
pub trait Packager: Plugin { … }
pub trait BuildSystem: Plugin { … }
pub trait Signer: Plugin { … }
// … one `: Plugin` supertrait each
```

There is no "signer that is also a packager" hierarchy and no deeper chain.
Where two dimensions genuinely share behaviour they share a *value* or a
*helper*, not an ancestor: `BuildContext` (`lib/plugins/mod.rs`) is passed to
every packager; `Release`/`Asset` (`lib/github.rs`) are shared structs that
the source-agnostic `match_assets` consumes; `RawGetter` is a standalone
trait the forge clients implement. The shallow graph is what keeps C1
possible: no capability is inherited just because a type sits under a base
that has it.

## C7 — Additive extension

Adding a variant is a new file plus one registration line, and the steps are
written down per dimension (`docs/architecture/plugins.md` §10):

- Packager: `lib/<format>archive.rs` + `lib/plugins/<format>.rs` + one entry
  in `all_packagers()`.
- BuildSystem: `lib/plugins/build_system/<name>.rs` + one entry in
  `all_build_systems()`; `sourcebuild.rs` is build-system-agnostic and only
  calls `build_sys.build()`.
- RegistrySource: `lib/plugins/registry/<name>.rs` + one entry in
  `all_registry_sources()`.
- PackageIndex: `impl PackageIndex` + one entry in `all_index_backends()`.

The recent refactors demonstrated this on the existing set rather than
asserting it: `9889603` converted fifty-odd impl files with no caller
changes, and `7f1fcbd` converted the last dimension (`PackageIndex`)
"with all public call sites unchanged". Because the seam is a trait object,
the new variant is reached by the same `get_*` the old ones use — no branch
to add, no `match` arm to remember.

## C8 — Residual dispatch is explicit and bounded

The per-format `match` is gone from every surface that grows with the
plugin set. The source-build wrapper resolves a `Packager` once and asks it,
instead of switching on the format string:

| Was | Now |
|---|---|
| `match format` to pick `archive_staged_tree` (`lib/sourcebuild.rs`) | `packager.archive_staged_tree(&ctx)` |
| `match format` to pick the source-package emitter (`lib/sourcebuild.rs`, twice in `lib/build.rs`) | `packager.generate_source_package(&out, &pkg)` |
| `matches!(format, "deb" \| "rpm" \| "arch")` guard | `packager.supports_source_build()`; the supported list is derived from `all_packagers()` |
| `match format` for the detach-sign branch | `signer::apply_post_build()` (embedded vs detached) |
| `match package_format` for the summary glob (`lib/summary.rs`) | `packager.artifact_glob(&package)` |
| Hardcoded name lists in `lib/config.rs` (`package_format`, `source`, `overrides:` keys, `contents[].packager`) | `get_packager()` / `get_forge_source()` / `packager_names()` |
| `match format` for default distributions (`config.rs`) | `packager.default_distributions()` — the method that existed but was unwired |

Adding a packager now requires no arm in any of those. The only
format-specific code a new packager must write is its own trait impl
(`file_extension`, `default_distributions`, `arch_supported_for_dist`,
`build`, and — if it supports source builds or has a filename quirk — the
overrides).

The remaining `match`/`enum` dispatch is off the plugin-dispatch path:

- **Host/consumer adapters.** `consumer.rs`, `install_pkg.rs`, `pkgname.rs`,
  `convert.rs`, and `go_native.rs` branch on the *host* format because they
  orchestrate `dpkg`/`rpm`/`pacman` rather than a `lx` plugin; registering a
  packager does not add a branch to them.
- **Shared lookup tables keyed by an explicit format argument.**
  `lib/depmap.rs` (`normalize_version`, `to_format`, `format_to_family`)
  holds operator/name tables that take the format as an argument; this is
  the shared-substrate pattern of C3, not call-site dispatch, and is the
  point at which a brand-new dependency-syntax family would touch shared
  code.

Neither category is a `match` over the plugin registry, so the criterion
holds.

## Where composition is partial

Collected, so the ✅ rows above are not read as universal:

- **Registration is static.** A plugin is a compile, not a drop-in file;
  there is no `dlopen`/feature-flag/config loading (`lib/plugins/plugin.rs`
  says so directly). The composition happens at compile time.
- **The trait set is an internal contract**, not a semver'd public
  extension API (see [composability](composability.md) C2/C6).
- **`SourceKind::Custom` is not a plugin yet.** `lib/index/registry.rs`
  maps the enum to a backend id and returns `None` for `Custom`, so a custom
  read index still needs code.
- **`SourceKind` is an enum in the config/registry layer** whose
  `plugin_kind()` is a small `match`; the plugin side is composed, but the
  persisted-config vocabulary is still an enum.
- **A few shared tables remain format-keyed** (`lib/depmap.rs` operator and
  name tables), so a brand-new dependency-syntax family would add to a
  shared helper rather than to a plugin (see C8).

## Verifying these claims

```sh
# Each dimension's registry resolves to trait objects and is complete
cargo test --test plugins          # packager registry, shared staging, archives,
                                   # `supports_source_build`, `artifact_glob`
cargo test --test plugins_artifact # ArtifactFormat registry + detection
cargo test --test plugins_signer   # Signer selection by (format, method)
cargo test --test plugins_depmap   # DependencyMapper selection + rendering
cargo test --test plugins_repo     # PackageIndex write backends

# The composed seams the C8 refactor introduced
cargo test --test plugins          # source-build capability + artifact glob
cargo test --test summary          # build summary glob from the packager
cargo test --test build            # --source uses generate_source_package
cargo test --test config           # format/source validation from the registry

# The identity/registry refactor did not change callers
cargo test --lib --test plugins    # 89 lib + 18 plugins at commit 9889603

# The composed library surface is documented and link-clean
cargo doc --no-deps -p lx
cargo clippy --all-targets --all-features -- -D warnings
```

The refactor commits state their own verification: `9889603` checked
`cargo check --all-targets`, `cargo test --lib` + `--test plugins`,
`cargo clippy --all-targets --all-features -D warnings`, and `cargo fmt` in
an isolated worktree; `05d5425`, `7f1fcbd`, and `818c283` the same.

## See also

- [Composability](composability.md) — the operator- and plugin-author-facing
  counterpart: selecting, combining, and extending the parts.
- [Interoperability](interoperability.md) — the exchange evaluation.
- [`docs/architecture/plugins.md`](../architecture/plugins.md) — the eight
  dimensions, the Source/RegistrySource non-merge decision, and the
  add-a-plugin steps.
- [`docs/architecture/plugin-catalog.md`](../architecture/plugin-catalog.md)
  §12 — the cross-cutting dimensions and what each replaces.
- [`lib/plugins/plugin.rs`](../../lib/plugins/plugin.rs) — `Plugin`,
  `PluginSet`, and `plugin_identity!`.
- [`docs/decisions/2026-08-21-dry-solid-cleanup.md`](../decisions/2026-08-21-dry-solid-cleanup.md) —
  the composition pass that closed two real bugs.
