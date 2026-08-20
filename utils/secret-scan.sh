#!/usr/bin/env bash
# Lightweight secret scan over the staged diff: no external dependency
# (gitleaks/detect-secrets), just a handful of high-confidence patterns for
# the credential types this tool actually touches (GitHub tokens via
# GITHUB_TOKEN, cloud keys that might leak into a pasted example/test
# fixture). Not a replacement for a real scanner -- narrow by design to
# keep false positives near zero for a pre-commit hook.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# Only scan added lines (prefixed '+') so pre-existing matches in context
# lines don't trip the hook, and only in the staged diff (what's about to
# be committed).
diff="$(git diff --cached -U0 -- . ':!utils/secret-scan.sh')"

patterns=(
    'ghp_[A-Za-z0-9]{36}'                 # GitHub personal access token
    'gh[oust]_[A-Za-z0-9]{36}'            # GitHub OAuth/user/server/refresh token
    'github_pat_[A-Za-z0-9_]{22,}'        # GitHub fine-grained PAT
    'AKIA[0-9A-Z]{16}'                    # AWS access key ID
    'xox[baprs]-[A-Za-z0-9-]{10,}'        # Slack token
    '-----BEGIN (RSA|OPENSSH|EC|PGP|DSA) PRIVATE KEY-----'
)

found=0
for pat in "${patterns[@]}"; do
    hits=$(echo "$diff" | grep -E "^\+" | grep -Ev '^\+\+\+' | grep -oE "$pat" || true)
    if [[ -n "$hits" ]]; then
        echo "secret-scan: possible credential matching /$pat/ in staged changes"
        found=1
    fi
done

if [[ "$found" -ne 0 ]]; then
    echo "secret-scan: refusing to commit. If this is a false positive (e.g. a" >&2
    echo "  test fixture), rename the pattern to break the match or use" >&2
    echo "  'git commit --no-verify' deliberately." >&2
    exit 1
fi
exit 0
