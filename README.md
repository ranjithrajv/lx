# lx — build, install, and distribute Linux packages

`lx` builds installable packages from many kinds of upstream: forge
release binaries (GitHub, GitLab, Gitea, Forgejo, Bitbucket, Gerrit),
source compilation, language package registries (npm, pip, gem), or files
you supply directly. Output is real Debian packages (`.deb`), RPM
(`.rpm`), or Arch (`.pkg.tar.zst`) — full dependency relations,
epoch-aware versioning, man pages and license files at their conventional
FHS paths — built natively on bare metal with no containers, no
emulation, no Debian host required.

It also installs/upgrades/removes the packages it builds — an apt-like
front end for software that ships forge releases, plus the tooling to run
a prebuilt apt repository from its output. It's a Rust rewrite of the
[debian-multiarch-builder](https://github.com/ranjithrajv/debian-multiarch-builder)
GitHub Action, usable as a local CLI (`lx`, with the `lx get` consumer
subcommand), as a GitHub Action (`action.yml`, [see the guide](docs/guides/github-action.md)),
and as a repo publisher (`lx repo`).

It's also **host-aware**: `lx` auto-detects the OS, architecture, package
manager, and native package format it's running on, then defaults each
command to suit — one CLI, no per-distro runbook.

## Documentation

Full docs live in [`docs/`](docs/README.md):

- [Commands](docs/reference/commands.md) — every subcommand and flag
- [`package.yaml` reference](docs/reference/package-yaml.md) — all config fields
- [Host detection](docs/reference/host-detection.md) — the smart-defaults table
- [Package indexes](docs/reference/indexes.md) — `lx index`, repology, coverage
- [GitHub Action](docs/guides/github-action.md) · [Reproducible builds](docs/guides/reproducible-builds.md)
- [Architecture](docs/architecture/overview.md) — plugin dimensions, catalog, tooling
- [Evaluations](docs/evaluation/README.md) — [Interoperability](docs/evaluation/interoperability.md) · [Composability](docs/evaluation/composability.md)
- [Comparisons](docs/comparison/lx-vs-nfpm.md) · [Landscape](docs/analysis/landscape.md) · [Decisions](docs/decisions/README.md)

## Requirements

- Rust (to build `lx` itself; see [Development](#development)). That's
  it — **`lx` builds natively on bare metal:**
  - `lx build` writes `.deb`s in-process (`lib/debarchive.rs`); no
    `dpkg-deb`, no Debian toolchain, and cross-architecture packaging
    only copies/`chmod`s the target binary — it's never executed, so any
    host can package any architecture.
  - `--source` writes source packages (`.dsc`/`.orig.tar.xz`/
    `.debian.tar.xz`) in-process too — no `dpkg-source`.
  - `--lintian` shells out to a `lintian` binary on your `PATH` (e.g.
    `apt-get install lintian` on Debian/Ubuntu): the one external tool
    with no Rust equivalent.

  See
  [`docs/decisions/2026-08-20-docker-free-deb-build.md`](docs/decisions/2026-08-20-docker-free-deb-build.md)
  and
  [`docs/decisions/2026-08-20-docker-free-lintian-source.md`](docs/decisions/2026-08-20-docker-free-lintian-source.md)
  for how, and the trade-offs involved.

## Quick start

Build every architecture a forge release publishes, no config file needed:

```sh
lx build https://github.com/eza-community/eza
```

Build for just this machine's own architecture (native-only):

```sh
lx build package.yaml --host
```

Package files you supply — no forge release, minimal config (fpm-style
"you supply files" mode):

```sh
lx build --from-dir ./myapp-dist/ --package-name myapp --version 1.0.0
lx build --from-file ./myapp --package-name myapp --version 1.0.0
lx build --from-dir ./dist/ --package-name myapp --version 1.0.0 --prefix /usr/local/bin
```

With a base `package.yaml` for extra metadata (dependencies, contents,
signing, etc.):

```sh
lx build package.yaml --from-dir ./myapp-dist/
```

Package from a language registry (input source plugin), via package.yaml:

```yaml
# package.yaml — npm example
package_name: typescript
registry_source: npm
github_repo: typescript    # npm package name
version: latest            # optional
```

```sh
lx build package.yaml
```

Generate a `package.yaml` interactively, with auto-discovered release
patterns:

```sh
lx init
```

Or scaffold one non-interactively from a forge repo (auto-discovers the
release assets) or a bundled debian-multiarch-builder template:

```sh
lx init --from eza-community/eza    # discover + write a starter package.yaml
lx init --template rust/eza         # or go/hugo, c/neovim, python/generic, …
```

(An unknown name lists all of them; see [`templates/README.md`](templates/README.md).)


## Development

```sh
cargo build
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all
```

Pre-commit hooks (`.pre-commit-config.yaml`) run fmt/clippy/shellcheck/
secret scan/license check/typos on every commit, and the fuller
`cargo test` + coverage gate + `cargo deny` + doc lint on push:

```sh
make dev-setup                              # install hook binaries (idempotent)
pre-commit install
pre-commit install --hook-type pre-push
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for the full setup guide, hook
reference, and project layout.

Performance comparisons against the tools `lx` replaces (dpkg-deb,
dpkg-shlibdeps, dpkg-scanpackages, fpm, nfpm) are recorded in
[`benchmarking/`](benchmarking/): run `./benchmarking/run.sh` to reproduce
and log a new result.

[`docs/decisions/`](docs/decisions/README.md) records the non-obvious calls
made while porting from the bash action (library choices, config-format
decisions, investigation notes) — worth a look before changing behavior
that mirrors upstream.
