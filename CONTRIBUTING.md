# Contributing to lx

Thanks for your interest! This guide covers the local setup so you can
lint, test, and ship changes with confidence.

## Quick Start

```sh
# 1. Clone
git clone https://github.com/ranjithrajv/lx.git
cd lx

# 2. Install the Rust toolchain (if you don't have one)
#    https://rustup.rs
rustup show   # installs the toolchain from rust-version in Cargo.toml

# 3. Install dev tools (pre-commit hooks + their binaries)
make dev-setup

# 4. Install pre-commit hooks into .git
pre-commit install
pre-commit install --hook-type pre-push
```

That's it — `make dev-setup` installs every tool the hooks need.

## Dev Tools Installed by `make dev-setup`

| Tool | Install Command | Used By Hook |
|------|----------------|--------------|
| [pre-commit](https://pre-commit.com) | `pip install pre-commit` or `brew install pre-commit` | Hook runner |
| [cargo-deny](https://github.com/EmbarkStudios/cargo-deny) | `cargo install cargo-deny` | `cargo-deny` (pre-push) |
| [typos](https://github.com/crate-ci/typos) | `cargo install typos-cli` | `typos` (pre-commit) |
| [taplo](https://github.com/tamasfe/taplo) | `cargo install taplo-cli --locked` | `taplo-fmt` (pre-commit) |
| [markdownlint-cli](https://github.com/igorshubovych/markdownlint-cli) | `npm install -g markdownlint-cli` | `markdownlint` (pre-push) |
| [shellcheck](https://www.shellcheck.net) | `apt install shellcheck` / `brew install shellcheck` | `shellcheck` (pre-commit) |

Already have them? `make dev-setup` is idempotent — it skips anything
that's already installed.

## Pre-commit Hooks

Hooks run in two tiers:

### On every commit (`pre-commit`)

| Hook | What it does |
|------|-------------|
| `cargo-fmt` | Auto-fix Rust formatting |
| `cargo-clippy` | Lint with `-D warnings` (hard fail) |
| `cargo-locked` | Ensure `Cargo.lock` is in sync |
| `shellcheck` | Lint shell scripts |
| `secret-scan` | Block credentials in staged diff |
| `license-check` | Require `SPDX-License-Identifier: GPL-3.0-or-later` on `.rs` files |
| `typos` | Spell check Rust, Markdown, TOML, YAML, shell |
| `taplo-fmt` | Auto-fix TOML formatting |

### On push (`pre-push`)

| Hook | What it does |
|------|-------------|
| `cargo-test` | Run all tests |
| `coverage-gate` | Fail if line coverage < 40% |
| `cargo-deny` | License compliance, banned crates, duplicate deps, advisories |
| `cargo-doc` | Deny rustdoc warnings (broken links, missing docs) |
| `markdownlint` | Lint Markdown files |

### Skipping Hooks

For emergencies only — CI will still enforce everything:

```sh
git commit --no-verify      # skip pre-commit hooks
git push --no-verify        # skip pre-push hooks
```

## Project Layout

```
src/              # Thin binary entrypoint (main.rs)
lib/              # All logic lives here (lx_lib) — build, forge clients, plugins
tests/            # Integration tests, mirrors lib/ structure
utils/            # Helper scripts (coverage, secret scan, covscan)
benchmarking/     # lx vs. the tools it replaces: harness + recorded results
docs/             # Documentation (start at docs/README.md): reference/,
                  # guides/, architecture/, comparison/, analysis/,
                  # decisions/ (ADR-style notes), outreach/
```

## Releasing lx

Releases are cut from a `v*` tag — pushing one triggers
`.github/workflows/release.yml`:

```sh
git tag v0.1.0
git push origin v0.1.0
```

You can also run the workflow by hand from the Actions tab
(`workflow_dispatch`), optionally naming an existing `v*` tag; the tag must
still match `v*` or the build job refuses to publish.

The workflow builds `lx` statically for `x86_64-unknown-linux-musl` and
`aarch64-unknown-linux-musl` with a **native** cross toolchain
(`musl-tools` + `gcc-aarch64-linux-gnu`, no Docker `cross`), refuses to
publish any binary that isn't statically linked, and for each target
publishes:

- `lx-<tag>-<triple>.tar.gz` (containing the `lx` binary)
- `lx-<tag>-<triple>.tar.gz.sha256`

Both tarballs and checksums are attached to the GitHub Release for the tag
(with generated release notes). As a follow-up job, `lx` then dogfoods its
own `lx build` against those assets via `.github/lx/package.yaml` to
produce `.deb`/`.rpm`/`.pkg.tar.zst` packages and attaches them to the same
release. That job is `continue-on-error: true` until the release pipeline
has a proven release, so a packaging hiccup can't fail the actual release.

The composite action consumes these artifacts: set the `lx-version` input
(e.g. `lx-version: v0.1.0`) to download and checksum-verify the prebuilt
binary instead of compiling from source; leaving it empty keeps the
build-from-source default.

## License

By contributing, you agree that your contributions will be licensed
under the same license as the project: **GPL-3.0-or-later**. Every
`.rs` file must carry the `// SPDX-License-Identifier: GPL-3.0-or-later`
header — the `license-check` hook enforces this.
