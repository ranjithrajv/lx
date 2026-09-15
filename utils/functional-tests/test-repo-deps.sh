#!/bin/bash
# Functional tests: lx repo (build an index from local artifacts) and
# lx deps scan/resolve (ELF dependency analysis).
# Offline, re-runnable.
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
PAYLOAD="$TMP/payload"; mkdir -p "$PAYLOAD"
[ -e /bin/true ] && cp /bin/true "$PAYLOAD/mybinary" || cp "$LX" "$PAYLOAD/mybinary"

echo "[repo] build an apt repository index from local .debs"
mkdir -p "$TMP/debs"
"$LX" build --from-dir "$PAYLOAD" --package-name ft --version 1.0.0 \
  --format deb --host --distributions trixie --output "$TMP/debs" >/dev/null 2>&1 \
  || { bad "source .deb build failed"; exit 1; }
( ls "$TMP/debs"/*.deb >/dev/null 2>&1 ) || { bad "no .debs produced"; exit 1; }
# `lx repo <DIR>` writes the index into DIR itself (no -o) and does a live
# forge reachability check, so this step needs network.
if ! timeout 8 bash -c 'cat < /dev/null > /dev/tcp/github.com/443' 2>/dev/null; then
  echo "  SKIP: repo (no network to github.com)"
else
  if "$LX" repo "$TMP/debs" --format deb --suite trixie --origin test >/dev/null 2>&1; then
    [ -f "$TMP/debs/Packages" ] && [ -f "$TMP/debs/Packages.gz" ] \
      && pass "repo index (Packages, Packages.gz present)" \
      || bad "repo: Packages/Packages.gz missing"
    grep -q "Package: ft" "$TMP/debs/Packages" 2>/dev/null && pass "repo lists 'ft'" || bad "repo: 'ft' not in Packages"
  else
    bad "lx repo failed"
  fi
fi

echo "[deps scan] ELF dependency scanning via a real forge release"
# `lx deps scan` takes a package.yaml or GitHub URL (it fetches the release,
# downloads assets, and scans their ELFs) — not a raw binary path. Network-dependent.
if ! timeout 8 bash -c 'cat < /dev/null > /dev/tcp/github.com/443' 2>/dev/null; then
  echo "  SKIP: deps scan (no network to github.com)"
else
  if "$LX" deps scan https://github.com/eza-community/eza >/dev/null 2>&1; then
    out=$("$LX" deps scan https://github.com/eza-community/eza 2>/dev/null | head -8)
    [ -n "$out" ] && pass "deps scan produced output" || bad "deps scan: no output"
  else
    bad "deps scan failed"
  fi
fi

[ "$fail" -eq 0 ] && echo "REPO/DEPS: ALL PASS" || echo "REPO/DEPS: FAILURES"
exit $fail
