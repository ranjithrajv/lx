#!/usr/bin/env bash
# License header check: every .rs file must carry the SPDX license
# identifier for GPL-3.0-or-later. Runs on staged files only so it
# doesn't flag third-party or generated files outside the commit.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

SPDX='SPDX-License-Identifier: GPL-3.0-or-later'

# Collect staged .rs files (added or modified), excluding the target dir
# and any generated files.
mapfile -t files < <(git diff --cached --name-only --diff-filter=ACM -- '*.rs')

if [[ ${#files[@]} -eq 0 ]]; then
    exit 0
fi

missing=0
for f in "${files[@]}"; do
    # Skip files that don't exist (deleted in this commit).
    [[ -f "$f" ]] || continue
    if ! head -n 5 "$f" | grep -qF "$SPDX"; then
        echo "license-check: missing '$SPDX' header in $f" >&2
        missing=1
    fi
done

if [[ "$missing" -ne 0 ]]; then
    echo "license-check: add '// $SPDX' near the top of the file." >&2
    exit 1
fi
exit 0
