#!/bin/bash
# Functional test: lx convert (deb<->rpm<->arch round trip).
# Re-runnable. Builds its own source deb first.
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

# Build a clean source deb (default /usr/bin staging).
"$LX" build --from-dir "$PAYLOAD" --package-name ft --version 3.0.0 \
  --format deb --host --distributions trixie --output "$TMP/src" >/dev/null 2>&1 \
  || { bad "source deb build failed"; exit 1; }
SRC=$(ls "$TMP/src"/*.deb | head -1)

echo "[convert] round trip: deb -> rpm -> arch -> deb"
rm -rf "$TMP/d2r" "$TMP/r2a" "$TMP/a2d"
"$LX" convert "$SRC" -t rpm -o "$TMP/d2r" >/dev/null 2>&1 && d2r=$(ls "$TMP/d2r"/*.rpm 2>/dev/null|head -1) || d2r=""
[ -n "$d2r" ] && pass "deb->rpm" || { bad "deb->rpm"; exit 1; }

"$LX" convert "$d2r" -t arch -o "$TMP/r2a" >/dev/null 2>&1 && r2a=$(ls "$TMP/r2a"/*.pkg.tar.zst 2>/dev/null|head -1) || r2a=""
[ -n "$r2a" ] && pass "rpm->arch" || { bad "rpm->arch"; exit 1; }

"$LX" convert "$r2a" -t deb -o "$TMP/a2d" >/dev/null 2>&1 && a2d=$(ls "$TMP/a2d"/*.deb 2>/dev/null|head -1) || a2d=""
[ -n "$a2d" ] && pass "arch->deb" || { bad "arch->deb"; exit 1; }

echo "[convert] payload survives each hop"
have_rpm2cpio() { command -v rpm2cpio >/dev/null 2>&1; }
# deb->rpm payload (needs rpm2cpio to inspect; skip if absent).
if have_rpm2cpio; then
  rm -rf "$TMP/x"; mkdir -p "$TMP/x"; ( cd "$TMP/x" && rpm2cpio "$d2r" 2>/dev/null | cpio -idm 2>/dev/null )
  find "$TMP/x" -name mybinary 2>/dev/null | grep -q . && pass "deb->rpm payload" || bad "deb->rpm: mybinary missing"
else
  echo "  SKIP: deb->rpm payload (rpm2cpio not on PATH)"
fi
# rpm->arch payload
rm -rf "$TMP/x"; mkdir -p "$TMP/x"; ( cd "$TMP/x" && tar --zstd -xf "$r2a" 2>/dev/null )
find "$TMP/x" -name mybinary 2>/dev/null | grep -q . && pass "rpm->arch payload" || bad "rpm->arch: mybinary missing"
# arch->deb payload (the strongest check: full round trip, back to a deb).
ar p "$a2d" data.tar.gz 2>/dev/null | tar tzf - 2>/dev/null | grep -q mybinary && pass "arch->deb payload" || bad "arch->deb: mybinary missing"

echo "[convert] idempotency (deb->rpm twice, same hash)"
rm -rf "$TMP/i1" "$TMP/i2"
"$LX" convert "$SRC" -t rpm -o "$TMP/i1" >/dev/null 2>&1
"$LX" convert "$SRC" -t rpm -o "$TMP/i2" >/dev/null 2>&1
h1=$(sha256sum "$TMP/i1"/*.rpm 2>/dev/null | awk '{print $1}')
h2=$(sha256sum "$TMP/i2"/*.rpm 2>/dev/null | awk '{print $1}')
[ -n "$h1" ] && [ "$h1" = "$h2" ] && pass "idempotent ($h1)" || bad "differs: $h1 vs $h2"

echo "[convert] --dry-run produces no artifact"
rm -rf "$TMP/dr"; "$LX" convert "$SRC" --dry-run -t rpm -o "$TMP/dr" >/dev/null 2>&1
find "$TMP/dr" -name "*.rpm" 2>/dev/null | grep -q . && bad "dry-run emitted .rpm" || pass "dry-run: no artifact"

echo "[convert] --to default = host format"
rm -rf "$TMP/def"; "$LX" convert "$SRC" -o "$TMP/def" >/dev/null 2>&1
[ "$(ls "$TMP/def"/* 2>/dev/null | wc -l)" -ge 1 ] && pass "--to default" || bad "--to default produced nothing"

[ "$fail" -eq 0 ] && echo "CONVERT: ALL PASS" || echo "CONVERT: FAILURES"
exit $fail
