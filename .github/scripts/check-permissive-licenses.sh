#!/usr/bin/env bash
# Run from the checkout root. On Windows use Git Bash, not System32/WSL bash.
# Requires cargo and Python >= 3.11 (standard library only), a current Cargo.lock,
# and cached registry/index/package sources. Never downloads or builds/runs
# workspace artifacts.
# Fresh CI checkouts explicitly cargo fetch --locked before this offline check.
# Checks all features, normal edges, both Windows targets, and THIRD_PARTY.md.
# Build/dev dependencies and native libraries require a separate human review.
set -euo pipefail

# `python` is not always on PATH: on Windows only `python3` is, in Git Bash.
# Probe for a working interpreter and name the requirement when none exists.
for candidate in python3 python py; do
    if command -v "$candidate" >/dev/null 2>&1; then
        exec "$candidate" .github/scripts/license-audit.py "$@"
    fi
done

echo "check-permissive-licenses.sh: no Python >= 3.11 found (tried python3, python, py)" >&2
exit 127
