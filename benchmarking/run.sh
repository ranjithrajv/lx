#!/usr/bin/env bash
#
# Benchmark `lx` against the tools it can replace.
#
# Prints a Markdown report to stdout. No network access: every benchmark
# uses a locally staged payload, so runs are comparable and reproducible.
# See benchmarking/README.md for the methodology and ground rules.
#
# Usage:
#   cargo build --release
#   ./benchmarking/run.sh              # default 5 timed runs per tool
#   RUNS=10 ./benchmarking/run.sh
#   LX_BIN=/usr/bin/lx ./benchmarking/run.sh
#
# Environment:
#   LX_BIN   lx binary under test (default: $PATH, else target/{release,debug}/lx)
#   RUNS     timed iterations per tool (default 5)
#   WARMUP   untimed warm-up iterations per tool (default 1)
#   BENCH_DIST  Debian suite for the single-.deb comparison (default trixie)

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUNS="${RUNS:-5}"
WARMUP="${WARMUP:-1}"
# Debian suite used for the one-.deb-vs-one-.deb comparison (see README).
BENCH_DIST="${BENCH_DIST:-trixie}"

if ! [[ "$RUNS" =~ ^[0-9]+$ ]] || (( RUNS < 1 )); then
    echo "error: RUNS must be a positive integer (got '$RUNS')" >&2
    exit 2
fi
if ! [[ "$WARMUP" =~ ^[0-9]+$ ]]; then
    echo "error: WARMUP must be a non-negative integer (got '$WARMUP')" >&2
    exit 2
fi

resolve_lx() {
    if [[ -n "${LX_BIN:-}" ]]; then
        printf '%s\n' "$LX_BIN"
        return
    fi
    if command -v lx >/dev/null 2>&1; then
        command -v lx
        return
    fi
    local candidate
    for candidate in "$ROOT/target/release/lx" "$ROOT/target/debug/lx"; do
        if [[ -x "$candidate" ]]; then
            printf '%s\n' "$candidate"
            return
        fi
    done
    return 1
}

LX_BIN="$(resolve_lx)" || {
    echo "error: no lx binary found. Build one with 'cargo build --release'" >&2
    echo "       or set LX_BIN=/path/to/lx." >&2
    exit 1
}
case "$LX_BIN" in
    */target/debug/lx)
        echo "warning: $LX_BIN is a debug build; results are not release-comparable." >&2
        ;;
esac

have() { command -v "$1" >/dev/null 2>&1; }

now_ns() { date +%s%N; }

# measure <shell-command> -> "<median_ms> <min_ms>"
measure() {
    local cmd="$1" i start end
    local -a times=()
    for ((i = 0; i < WARMUP; i++)); do
        bash -c "$cmd" >/dev/null 2>&1 || true
    done
    for ((i = 0; i < RUNS; i++)); do
        start="$(now_ns)"
        bash -c "$cmd" >/dev/null 2>&1 || true
        end="$(now_ns)"
        times+=( $(( (end - start) / 1000000 )) )
    done
    local sorted median min mid
    sorted="$(printf '%s\n' "${times[@]}" | sort -n)"
    min="$(printf '%s\n' "$sorted" | head -n 1)"
    mid=$(( (RUNS + 1) / 2 ))
    median="$(printf '%s\n' "$sorted" | sed -n "${mid}p")"
    printf '%s %s\n' "$median" "$min"
}

emit_row() { printf '| %s | %s | %s | %s | %s |\n' "$@"; }

# run_one <tool-label> <required-binary|""> <command> <notes>
run_one() {
    local tool="$1" required="$2" cmd="$3" notes="$4"
    if [[ -n "$required" ]] && ! have "$required"; then
        emit_row "$tool" "-" "-" "-" "$notes (not installed)"
        return
    fi
    if ! bash -c "$cmd" >/dev/null 2>&1; then
        emit_row "$tool" "failed" "-" "0" "$notes (command failed)"
        return
    fi
    local out median min
    out="$(measure "$cmd")"
    median="${out%% *}"
    min="${out##* }"
    emit_row "$tool" "$median" "$min" "$RUNS" "$notes"
}

tool_version() {
    case "$1" in
        lx) "$LX_BIN" --version 2>/dev/null | head -n 1 ;;
        dpkg-deb) dpkg-deb --version 2>/dev/null | head -n 1 ;;
        dpkg-shlibdeps) dpkg-shlibdeps --version 2>/dev/null | head -n 1 ;;
        dpkg-scanpackages) dpkg-scanpackages --version 2>/dev/null | head -n 1 ;;
        apt-ftparchive) apt-ftparchive --version 2>/dev/null | head -n 1 ;;
        fpm) fpm --version 2>/dev/null ;;
        nfpm) nfpm --version 2>/dev/null ;;
        *) echo "unknown" ;;
    esac
}

print_header() {
    local host_arch
    host_arch="$(dpkg --print-architecture 2>/dev/null || uname -m)"
    cat <<EOF
## Run $(date -u +%Y-%m-%d) — $(uname -m)

- Date:        $(date -u +%Y-%m-%dT%H:%M:%SZ)
- Host:        $(uname -srm)
- Architecture: $host_arch
- CPUs:        $(nproc 2>/dev/null || echo unknown)
- Timed runs:  $RUNS per tool ($WARMUP warm-up)
- Bench dist:  $BENCH_DIST (single-suite deb comparison)
- lx:          $(tool_version lx)
- Reference tools:
EOF
    local tool
    for tool in dpkg-deb dpkg-shlibdeps dpkg-scanpackages apt-ftparchive fpm nfpm; do
        if have "$tool"; then
            printf '  - %s: %s\n' "$tool" "$(tool_version "$tool")"
        else
            printf '  - %s: not installed\n' "$tool"
        fi
    done
    echo
}

bench_pack_deb() {
    local work payload root dpkg_arch
    work="$(mktemp -d "${TMPDIR:-/tmp}/lx-bench-deb.XXXXXX")"
    payload="$work/payload"
    mkdir -p "$payload"
    printf '#!/bin/sh\necho bench\n' > "$payload/bench"
    chmod +x "$payload/bench"
    cp /usr/bin/ls "$payload/tool" 2>/dev/null || cp /bin/ls "$payload/tool"
    printf 'bench data\n' > "$payload/data.txt"

    dpkg_arch="$(dpkg --print-architecture 2>/dev/null || echo amd64)"

    root="$work/root"
    mkdir -p "$root/DEBIAN" "$root/usr/bin"
    cp "$payload/bench" "$root/usr/bin/bench"
    cp "$payload/tool" "$root/usr/bin/tool"
    cat > "$root/DEBIAN/control" <<EOF
Package: bench-pkg
Version: 1.0.0
Architecture: $dpkg_arch
Maintainer: Bench <bench@example.com>
Description: benchmark payload
EOF

    cat > "$work/nfpm.yaml" <<EOF
name: bench-pkg
arch: $dpkg_arch
version: 1.0.0
contents:
  - src: $payload/bench
    dst: /usr/bin/bench
  - src: $payload/tool
    dst: /usr/bin/tool
EOF
    mkdir -p "$work/lx-out" "$work/nfpm-out"

    cat <<'EOF'
### Pack a .deb from a staged tree

Input: one payload directory (a script, a real ELF, and a data file),
staged fresh per run. `lx` is compared against the reference packagers that
take the same "here are files" input.

`lx` is restricted to one distribution (`--distributions "$BENCH_DIST"`) so
the row is one `.deb` vs. one `.deb`; without it `lx` builds the payload once
per Debian suite.

| Tool | Median (ms) | Min (ms) | Runs | Notes |
|---|---:|---:|---:|---|
EOF
    run_one "lx build --from-dir" "" \
        "'$LX_BIN' build --from-dir '$payload' --package-name bench-pkg --version 1.0.0 --format deb --host --distributions '$BENCH_DIST' --output '$work/lx-out'" \
        "also generates control/changelog/copyright + infers ELF deps"
    run_one "dpkg-deb --build" "dpkg-deb" \
        "dpkg-deb --build '$root' '$work/dpkg.deb'" \
        "builds the .deb only"
    run_one "fpm -s dir -t deb" "fpm" \
        "fpm -s dir -t deb -n bench-pkg -v 1.0.0 -C '$payload' --prefix /usr/bin . --package '$work/fpm.deb'" \
        "feature-parity reference packager"
    run_one "nfpm pkg -p deb" "nfpm" \
        "nfpm pkg -f '$work/nfpm.yaml' -p deb --target '$work/nfpm-out'" \
        "feature-parity reference packager"
    echo
    rm -rf "$work"
}

bench_resolve_deps() {
    local work elf pkgdir
    work="$(mktemp -d "${TMPDIR:-/tmp}/lx-bench-deps.XXXXXX")"
    elf="/usr/bin/ls"
    [[ -x "$elf" ]] || elf="/bin/ls"
    pkgdir="$work/pkg"
    mkdir -p "$pkgdir/debian"
    cat > "$pkgdir/debian/control" <<'EOF'
Source: bench-pkg
Section: utils
Priority: optional
Maintainer: Bench <bench@example.com>
Standards-Version: 4.6.0

Package: bench-pkg
Architecture: any
Depends: ${shlibs:Depends}
Description: benchmark payload
 benchmark payload
EOF

    if ! compgen -G "/var/lib/dpkg/info/*.symbols" >/dev/null && \
       ! compgen -G "/var/lib/dpkg/info/*.shlibs" >/dev/null; then
        cat <<'EOF'
### Resolve shared-library dependencies

Skipped: this host has no dpkg `symbols`/`shlibs` database (the Debian
files under `/var/lib/dpkg/info/`), which both `lx deps resolve` and
`dpkg-shlibdeps` read. Run on a Debian/Ubuntu host to record this row.

EOF
        rm -rf "$work"
        return
    fi

    cat <<EOF
### Resolve shared-library dependencies

Input: \`$elf\`. \`lx deps resolve\` is a command-level drop-in for
\`dpkg-shlibdeps\`; both read the local dpkg symbols/shlibs databases.

| Tool | Median (ms) | Min (ms) | Runs | Notes |
|---|---:|---:|---:|---|
EOF
    run_one "lx deps resolve" "" \
        "'$LX_BIN' deps resolve -O '$elf'" \
        "command drop-in for dpkg-shlibdeps"
    run_one "dpkg-shlibdeps -O" "dpkg-shlibdeps" \
        "cd '$pkgdir' && dpkg-shlibdeps -O '$elf'" \
        "reference implementation"
    echo
    rm -rf "$work"
}

bench_repo_index() {
    local work repo root n dpkg_arch
    if ! have dpkg-deb; then
        printf '### Build an apt repository index\n\nSkipped: dpkg-deb is needed to stage .debs.\n\n'
        return
    fi
    work="$(mktemp -d "${TMPDIR:-/tmp}/lx-bench-repo.XXXXXX")"
    repo="$work/repo"
    mkdir -p "$repo"
    dpkg_arch="$(dpkg --print-architecture 2>/dev/null || echo amd64)"
    for n in 1 2 3; do
        root="$work/root$n"
        mkdir -p "$root/DEBIAN" "$root/usr/bin"
        printf '#!/bin/sh\necho bench%d\n' "$n" > "$root/usr/bin/bench$n"
        chmod +x "$root/usr/bin/bench$n"
        cat > "$root/DEBIAN/control" <<EOF
Package: bench-pkg-$n
Version: 1.0.$n
Architecture: $dpkg_arch
Maintainer: Bench <bench@example.com>
Description: benchmark payload $n
EOF
        dpkg-deb --build "$root" "$repo/bench-pkg-${n}_1.0.${n}_${dpkg_arch}.deb" >/dev/null 2>&1
    done

    cat <<'EOF'
### Build an apt repository index

Input: a directory of three `.deb`s. `lx repo` writes an apt-consumable
index; the reference tools write the `Packages` file itself.

`lx` does more here: it gzips `Packages` and writes a `Release` file with
MD5/SHA1/SHA256 checksums. The reference rows time the `Packages` file
only, so this row is "whole `lx repo` vs. whole reference command".

| Tool | Median (ms) | Min (ms) | Runs | Notes |
|---|---:|---:|---:|---|
EOF
    run_one "lx repo" "" \
        "'$LX_BIN' repo --format deb '$repo'" \
        "writes Packages + Packages.gz + Release"
    run_one "dpkg-scanpackages" "dpkg-scanpackages" \
        "dpkg-scanpackages '$repo' /dev/null > '$work/Packages.dpkg'" \
        "uncompressed Packages only"
    run_one "apt-ftparchive packages" "apt-ftparchive" \
        "apt-ftparchive packages '$repo' > '$work/Packages.apt'" \
        "uncompressed Packages only"
    echo
    rm -rf "$work"
}

main() {
    print_header
    bench_pack_deb
    bench_resolve_deps
    bench_repo_index
    cat <<'EOF'
---

<!-- Generated by benchmarking/run.sh. Append to benchmarking/results.md to
keep a record, and note anything unusual (thermal throttling, a busy machine,
a non-release lx build). -->
EOF
}

main "$@"
