# Registry source plugins: language package manager inputs

**Date:** 2026-09-11
**Context:** fpm supports packaging from language package managers (`-s npm`,
`-s gem`, `-s python`, `-s cpan`, `-s pear`). This was identified as a gap
in the fpm vs nfpm vs lx comparison. Rather than adding first-class CLI
flags for each ecosystem (fpm's approach), this implements them as plugins
so the core stays focused and new ecosystems can be added without editing
the build pipeline.

## What it does

`registry_source: npm|python|gem` in package.yaml selects a registry
source plugin. The plugin fetches the package from its language registry,
extracts it into a local directory, and the normal packaging pipeline
wraps it into a .deb/.rpm/.arch.

## Plugin architecture

```
lib/plugins/registry/
├── mod.rs      # RegistrySource trait + registry
├── npm.rs      # npm pack + extract
├── python.rs   # pip download --no-binary :all: + extract
└── gem.rs      # gem fetch + extract
```

The `RegistrySource` trait mirrors the existing `BuildSystem` and
`ForgeSource` traits:

```rust
pub trait RegistrySource: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn required_tools(&self) -> Vec<&'static str> { Vec::new() }
    fn fetch(&self, package: &str, version: &str, cfg: &PackageConfig)
        -> Result<RegistryPayload>;
}
```

`RegistryPayload` carries the extracted files directory, resolved version,
and detected description. The build pipeline sets `local_payload` and
`artifact_format = "raw"` then routes through `run_local()` — no new
packaging path needed.

## Why plugins, not CLI flags

fpm uses CLI flags (`-s npm`, `-s gem`). This works but couples every
ecosystem to the CLI surface. Plugins decouple:

- **Core stays small.** The build pipeline doesn't know about npm or pip.
- **New ecosystems are additive.** Adding cpan/hex/cargo-registry means
  one new file + one line in `all_registry_sources()`. No CLI changes, no
  pipeline changes.
- **Optional dependencies.** Each plugin declares `required_tools()`;
  the build fails early with a clear message if `npm`/`pip`/`gem` isn't
  on PATH.
- **Testable.** Each plugin is a self-contained unit with no global state.

## Design decisions

- **`github_repo` doubles as the registry package name.** No new config
  field — `github_repo: typescript` means the npm package "typescript".
  Falls back to `package_name` if `github_repo` is empty.
- **Version from `version:` field.** Same field as forge sources. Empty
  means "latest".
- **Description auto-detected from package metadata.** npm reads
  `package.json`, pip reads `pyproject.toml`/`setup.cfg`/`setup.py`,
  gem reads `metadata.gz`. Overridable via `description:` in package.yaml.
- **No toml dependency for pyproject.toml parsing.** Simple line-based
  parsing for the `[project]` section's `description =` key. Avoids
  adding a toml crate for a single string field.
- **`--no-binary :all:` for pip.** Forces sdist (source distribution)
  which is more portable than wheels (wheels are platform-specific and
  may have compiled extensions that won't run on the target system).

## Why RegistrySource is separate from ForgeSource

A natural question: why not merge `RegistrySource` into the existing
`ForgeSource` trait? They both answer "where does the package come
from?"

The answer: they operate at different levels of abstraction.

**`ForgeSource`** discovers *what is available* — it returns a
`Release` (tag + per-architecture asset list) that the pipeline then
downloads and matches to Debian architectures. It has 11 trait
methods for release discovery, license detection, repo metadata.

**`RegistrySource`** fetches *specific files* — it returns a
`RegistryPayload` (a local directory of files) that goes straight to
packaging. It has 4 methods.

Merging them would mean either:
- 11 methods where 7 are `unimplemented!()` for registry plugins, or
- A minimal 4-method trait that loses the forge discovery API that
  `validate.rs`, `scandeps.rs`, and `wizard.rs` depend on.

The pipeline dispatch is also different: forge sources are selected by
URL sniffing (`parse_any_forge_url`), registry sources by an explicit
`registry_source:` field. Conflating these creates implicit, error-prone
behavior.

See `docs/architecture/plugins.md` § "ForgeSource vs RegistrySource — why two
plugin types?" for the full comparison.

## Scope

Initial plugins: npm, python, gem. These cover the most common cases.
Future plugins (not in this PR): cpan, cargo-registry, hex, maven.
