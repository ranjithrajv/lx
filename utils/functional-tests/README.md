# `lx` functional tests

Re-runnable, end-to-end tests that exercise the CLI against real packages,
not mocks. Each suite is self-contained (builds its own fixtures, uses a temp
directory, exits non-zero on failure) and lives in the Rust xtask — there are
no shell scripts in this repository (see the Rust-only policy in
[`CONTRIBUTING.md`](../../CONTRIBUTING.md#development-policy-rust-only)).

## Run them

Build the release binary first:

```sh
cargo build --release
cargo xtask func-tests
```

Or via `make`:

```sh
make func-tests
```

All suites stop at the first failure unless `--no-fail-fast` (or
`FAIL_FAST=0`) is set. Point at a prebuilt binary with `--lx` (or `LX`), and
run a single suite with `--suite` (repeatable):

```sh
cargo xtask func-tests --lx ./target/release/lx
LX=/usr/bin/lx cargo xtask func-tests
cargo xtask func-tests --suite test-build
```

## What they cover

| Suite | Commands | Network? |
|---|---|---|
| `test-build` | `lx build` (offline payload matrix, reproducibility) | no |
| `test-convert` | `lx convert` (round trip, payload, idempotency, dry-run) | no |
| `test-info-validate-schema` | `lx info`, `lx validate`, `lx schema` | no |
| `test-repo-deps` | `lx repo` (apt index), `lx deps scan` | no |
| `test-catalog` | `lx list --catalog`, `lx show`, `lx index` (deb-get catalog) | no |

The source of truth is `src/bin/xtask/functional.rs`.

## What is NOT here (and why)

* **`lx install` / `upgrade` / `remove` / `reinstall` / `rollback`** — mutate
  the host package manager and need real, installable packages; these are best
  exercised in a container (see `tests/container_matrix.rs`, run via
  `make test-containers`).
* **`lx search` / `lx index`** — need a reachable package index / network.
* **`lx publish` / `lx init`** — interactive or network-bound.

For the fully offline, assertion-based suite (byte-level checks, not CLI
commands), see `tests/build_functional.rs`, run with the usual
`cargo test`.
