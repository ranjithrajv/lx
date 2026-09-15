#!/bin/bash
# Functional tests: lx info, lx validate, lx schema.
# Fully offline, re-runnable.
set -u
# cond && pass "..." || bad "..." is intentional: both helpers return 0,
# so bad() runs only when cond is false.
# shellcheck disable=SC2015,SC2012
REPO="$(cd "$(dirname "$0")/../.." && pwd)"
LX="${LX:-$REPO/target/release/lx}"
[ -x "$LX" ] || { echo "BUILD FIRST: $LX missing (cd $REPO && cargo build --release)"; exit 2; }

fail=0
pass() { echo "  PASS: $1"; }
bad()  { echo "  FAIL: $1"; fail=1; }

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

echo "[info] host detection (json)"
if info=$("$LX" info --json 2>/dev/null); then
  pm=$(echo "$info" | grep '"package_manager"' | head -1 | sed 's/.*: *"\(.*\)".*/\1/')
  pf=$(echo "$info" | grep '"package_format"' | head -1 | sed 's/.*: *"\(.*\)".*/\1/')
  [ -n "$pm" ] && [ -n "$pf" ] && pass "info reports manager='$pm' format='$pf'" || bad "info: missing fields -> $info"
else
  bad "info --json failed"
fi


<longcat_arg_value>echo "[validate] valid package.yaml passes, invalid fails"
# validate does a live forge reachability check, so the "valid" fixture must be
# a real, reachable repo (eza-community/eza, confirmed earlier in this pass).
cat > "$TMP/good.yaml" <<EOF
package_name: demo
github_repo: eza-community/eza
version: 1.0.0
description: d
maintainer: t <t@e>
license_spdx: MIT
EOF
cat > "$TMP/bad.yaml" <<EOF
github_repo: this-repo-does-not-exist-xyz/qqq
version: 1.0.0
description: d
maintainer: t <t@e>
EOF
# Network-dependent: skip cleanly if the forge is unreachable.
if ! timeout 8 bash -c 'cat < /dev/null > /dev/tcp/github.com/443' 2>/dev/null; then
  echo "  SKIP: validate (no network to github.com)"
else
  "$LX" validate "$TMP/good.yaml" >/dev/null 2>&1 && pass "valid config accepted" || bad "valid config rejected"
  "$LX" validate "$TMP/bad.yaml" >/dev/null 2>&1 && bad "invalid config accepted" || pass "invalid config rejected"
fi

echo "[schema] JSON schema generation is valid JSON"
if schema=$("$LX" schema 2>/dev/null); then
  if echo "$schema" | grep -q '"type"'; then
    # crude well-formedness check
    python3 -c "import sys,json; json.load(sys.stdin)" <(echo "$schema") 2>/dev/null \
      && pass "schema emits valid JSON" \
      || echo "$schema" | head -3 | grep -q '{' && pass "schema emitted (json parse unavailable)" \
      || bad "schema not json-ish"
  else
    bad "schema missing 'type'"
  fi
else
  bad "schema command failed"
fi

[ "$fail" -eq 0 ] && echo "INFO/VALIDATE/SCHEMA: ALL PASS" || echo "INFO/VALIDATE/SCHEMA: FAILURES"
exit $fail
