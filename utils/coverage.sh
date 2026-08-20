#!/usr/bin/env bash
# Run llvm-cov, extract function spans with covscan, and print the
# uncovered-function checklist (excluding test fns).
#
# Also gates the build: exits non-zero if overall line coverage is below
# COVERAGE_MIN (default 40 -- current baseline is ~42%; a large slice of
# this codebase is Docker/dpkg/network orchestration that isn't realistic
# to unit-test, so the gate is scoped to it rather than chasing an
# unenforceable repo-wide target. Ratchet this up as coverage improves.
#
# Requires: cargo-llvm-cov, the utils/covscan binary (build it once with
#   `cargo build --manifest-path utils/covscan/Cargo.toml`)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
COVSCAN_BIN="$ROOT/utils/covscan/target/debug/covscan"
COV_JSON="${TMPDIR:-/tmp}/lpt-cov.json"
FNS_JSON="${TMPDIR:-/tmp}/lpt-fns.json"
COVERAGE_MIN="${COVERAGE_MIN:-40}"

if [[ ! -x "$COVSCAN_BIN" ]]; then
    cargo build --manifest-path "$ROOT/utils/covscan/Cargo.toml" >/dev/null
fi

cd "$ROOT"
cargo llvm-cov --all-targets --fail-under-lines "$COVERAGE_MIN" --json > "$COV_JSON"
"$COVSCAN_BIN" src lib > "$FNS_JSON"
python3 utils/coverage-gaps.py "$FNS_JSON" "$COV_JSON"
