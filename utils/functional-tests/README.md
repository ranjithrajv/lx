# `lx` functional test scripts

Re-runnable shell scripts that exercise the CLI end-to-end against real
packages, not mocks. Each is self-contained (builds its own fixtures, uses
`mktemp`, exits non-zero on failure) and documents exactly how to re-run the
corresponding manual test.

## Run them

Build the release binary first:

```sh
cargo build --release
./utils/functional-tests/run-all.sh
```

Or individually (all honor `$LX` to point at a prebuilt binary):

```sh
LX=./target/release/lx ./utils/functional-tests/test-build.sh
LX=./target/release/lx ./utils/functional-tests/test-convert.sh
LX=./target/release/lx ./utils/functional-tests/test-info-validate-schema.sh
LX=./target/release/lx ./utils/functional-tests/test-repo-deps.sh
```

## What they cover

| Script | Commands | Network? |
|---|---|---|
| `test-build.sh` | `lx build` (offline payload matrix, reproducibility) | no |
| `test-convert.sh` | `lx convert` (round trip, payload, idempotency, dry-run) | no |
| `test-info-validate-schema.sh` | `lx info`, `lx validate`, `lx schema` | no |
| `test-repo-deps.sh` | `lx repo` (apt index), `lx deps scan` | no |

## What is NOT here (and why)

* **`lx install` / `upgrade` / `remove` / `reinstall` / `rollback`** — mutate
  the host package manager and need real, installable packages; these are best
  exercised in a container (see `tests/container_matrix.rs`, run via
  `make test-containers`).
* **`lx search` / `lx index`** — need a reachable package index / network.
* **`lx publish` / `lx init`** — interactive or network-bound.

For the fully offline, assertion-based suite (byte-level checks, not shell
commands), see `tests/build_functional.rs`, run with the usual
`cargo test`.
