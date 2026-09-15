#!/bin/bash
# Run every functional-test script in this directory. Stops at the first failure
# unless FAIL_FAST=0.
set -u
FAIL_FAST="${FAIL_FAST:-1}"
# This script lives at <repo>/utils/functional-tests/; resolve repo root from
# its own location (two levels up) so paths are correct however it is invoked.
REPO="$(cd "$(dirname "$0")/../.." && pwd)"
LX="${LX:-$REPO/target/release/lx}"

if [ ! -x "$LX" ]; then
  echo "Release binary missing: $LX  (run: cargo build --release)"
  exit 2
fi

cd "$REPO/utils/functional-tests" || exit
scripts=(test-build.sh test-convert.sh test-info-validate-schema.sh test-repo-deps.sh)
failed=()
for s in "${scripts[@]}"; do
  echo
  echo "========== $s =========="
  if LX="$LX" bash "./$s"; then
    echo "[ok] $s"
  else
    failed+=("$s")
    [ "$FAIL_FAST" = "1" ] && break
  fi
done

echo
if [ "${#failed[@]}" -eq 0 ]; then
  echo "ALL FUNCTIONAL TESTS PASSED (${#scripts[@]})"
  exit 0
else
  echo "FAILURES: ${failed[*]}"
  exit 1
fi
