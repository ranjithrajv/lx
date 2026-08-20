#!/usr/bin/env python3
"""Cross-reference syn-extracted function spans (covscan JSON) with
llvm-cov JSON to list functions that are NOT 100% line-covered.

Usage:
  cargo llvm-cov --json > /tmp/cov.json
  utils/covscan/target/debug/covscan src lib > /tmp/fns.json
  python3 utils/coverage-gaps.py /tmp/fns.json /tmp/cov.json

A function is reported if any line within [start, end] has zero coverage.
Test-only functions (name starts with `tests::`) are ignored.
"""
import json
import sys


def line_counts(segments):
    """Map line number -> max count over segments anchored on that line."""
    counts = {}
    for seg in segments:
        line, col, cnt, has_count = seg[0], seg[1], seg[2], seg[3]
        if not has_count:
            continue
        counts.setdefault(line, 0)
        if cnt > counts[line]:
            counts[line] = cnt
    return counts


def main():
    fns_path, cov_path = sys.argv[1], sys.argv[2]
    fns = json.load(open(fns_path))
    cov = json.load(open(cov_path))

    per_file_counts = {}
    for f in cov["data"][0]["files"]:
        per_file_counts[f["filename"]] = line_counts(f["segments"])

    for module, funcs in sorted(fns.items()):
        for name, start, end in funcs:
            if name.startswith("tests::"):
                continue
            file_key = next(
                (k for k in per_file_counts if k.endswith("/" + module)),
                None,
            )
            counts = per_file_counts.get(file_key, {})
            uncovered = [ln for ln in range(start, end + 1)
                         if ln in counts and counts[ln] == 0]
            status = "COVERED" if not uncovered else "GAP"
            if status == "GAP":
                print(f"{module}::{name} [{start}-{end}] lines {uncovered}")
            else:
                print(f"{module}::{name} [{start}-{end}] OK")


if __name__ == "__main__":
    main()
