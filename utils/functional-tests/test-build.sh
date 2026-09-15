#!/bin/bash
# Functional test: lx build (offline payload matrix).
# Re-runnable. Exits non-zero on any failure.
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
PAYLOAD="$TMP/payload"
mkdir -p "$PAYLOAD"
printf 'hello\n' > "$PAYLOAD/hello.txt"
# ELF payload: real binary if available, else the test binary itself.
if [ -e /bin/true ]; then cp /bin/true "$PAYLOAD/mybinary"; else cp "$LX" "$PAYLOAD/mybinary"; fi

echo "[build] offline payload matrix (host arch, --prefix)"
for fmt in deb rpm arch apk ipk; do
  out="$TMP/out-$fmt"; mkdir -p "$out"
  if "$LX" build --from-dir "$PAYLOAD" --prefix /usr/local/share/ft \
        --package-name ft --version 1.0.0 --format "$fmt" --host \
        --distributions default --output "$out" >/dev/null 2>&1; then
    # Expect at least one artifact with the right extension.
    case "$fmt" in
      deb) ext="deb";;
      rpm) ext="rpm";;
      arch) n=$(ls "$out"/*.pkg.tar.zst 2>/dev/null | wc -l); [ "$n" -ge 1 ] && pass "$fmt -> $n artifact(s)" || bad "$fmt: no .pkg.tar.zst"; continue;;
      apk) ext="apk";;
      ipk) ext="ipk";;
    esac
    n=$(ls "$out"/*."$ext" 2>/dev/null | wc -l)
    [ "$n" -ge 1 ] && pass "$fmt -> $n artifact(s)" || bad "$fmt: no .$ext files"
  else
    bad "$fmt: build failed"
  fi
done

echo "[build] reproducibility (deb, two builds, same hash)"
o1="$TMP/r1"; o2="$TMP/r2"; mkdir -p "$o1" "$o2"
"$LX" build --from-dir "$PAYLOAD" --prefix /usr/local/share/ft \
  --package-name ft --version 1.0.0 --format deb --host --output "$o1" >/dev/null 2>&1
"$LX" build --from-dir "$PAYLOAD" --prefix /usr/local/share/ft \
  --package-name ft --version 1.0.0 --format deb --host --output "$o2" >/dev/null 2>&1
h1=$(sha256sum "$o1"/*.deb | awk '{print $1}')
h2=$(sha256sum "$o2"/*.deb | awk '{print $1}')
[ "$h1" = "$h2" ] && pass "reproducible ($h1)" || bad "hash differs: $h1 vs $h2"

[ "$fail" -eq 0 ] && echo "BUILD: ALL PASS" || echo "BUILD: FAILURES"
exit $fail
