# Development policy: Rust-only tooling (no shell scripts)

## Context

`lx` ships as a single Rust crate, but its *developer* surface had accumulated
scripts: a Bash benchmark harness (`benchmarking/run.sh`), five functional-test
scripts plus a runner under `utils/functional-tests/`, three pre-commit
helpers (`utils/license-check.sh`, `utils/secret-scan.sh`, `utils/coverage.sh`),
and a Python coverage post-processor (`utils/coverage-gaps.py`).

Those scripts duplicated logic the crate already owns (archive readers, SHA-256
helpers), pulled extra runtime interpreters (`bash`, `python3`, `awk`, `sed`,
`rpm2cpio`, `cpio`, `tar`) into the dev setup, and were the only part of the
project not covered by `cargo clippy` / `cargo test`.

## Decision

The repository is **Rust-only**, tooling included. No shell scripts (or Python,
Ruby, or Perl helpers) are permitted.

- All developer tasks are `cargo xtask <task>` subcommands in
  `src/bin/xtask/`, invoked through the alias in `.cargo/config.toml`.
- The ported tasks are: `policy`, `license-check`, `secret-scan`, `coverage`
  (wrapping `utils/covscan` + `cargo llvm-cov`), `func-tests` (the five CLI
  suites), `bench`, and `dev-setup`.
- Artifact inspection in the functional tests uses `lx`'s own in-process
  readers (`debarchive`/`rpmarchive`/`archarchive`) instead of shelling out to
  `rpm2cpio`/`cpio`/`tar`.
- The policy is **enforced**, not just documented: `cargo xtask policy` scans
  every tracked file and fails on a shell script (by extension or `#!` shebang)
  or a non-Rust script helper. It runs as the `rust-only` pre-commit hook.
- **Exemption:** GitHub Actions YAML (`action.yml`, `.github/workflows/*.yml`)
  has no shell-free form, so its inline `run:` snippets remain. They must stay
  thin; logic belongs in `cargo xtask` or the `lx` binary. The policy check
  only inspects tracked files, so YAML is naturally exempt.

## Consequences

- One toolchain (`cargo`) covers development; `make dev-setup` no longer
  installs `shellcheck`, and pre-commit no longer runs it.
- Adding a benchmark or functional-test row means editing Rust
  (`src/bin/xtask/bench.rs` / `functional.rs`), which is type-checked and
  linted like the rest of the tree.
- Contributors no longer need `bash`-specific quoting knowledge for dev tasks.
- If a script genuinely needs to exist (e.g. a package's own build system or a
  user-facing generated installer), it must live outside the tracked source
  tree — `lx` already *generates* shell installers and runs user `custom`
  build steps, and those are product outputs, not project tooling.
