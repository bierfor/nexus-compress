#!/bin/bash
# scripts/validate-ci.sh
#
# Sprint 5.6.28 hotfix #9 left us with a 1-character YAML
# indentation error that took down the entire release pipeline
# before any runner even woke up. The lesson: GitHub Actions
# validates YAML *before* anything else, so a stray space is a
# show-stopper we only catch when we push.
#
# This script parses every .github/workflows/*.yml file with
# python's strict YAML loader, so any syntax error surfaces in
# the local terminal in milliseconds instead of "Invalid
# workflow file on line 216" in the Actions UI 90 seconds later.
#
# Usage:
#   scripts/validate-ci.sh            # validate all workflows
#   scripts/validate-ci.sh file.yml   # validate one file
#
# Returns:
#   0 on success
#   1 on YAML syntax error (with the offending file + line)
#
# Sprint 5.6.28 lesson: we could (a) wire this as a pre-commit
# hook, (b) run it from a CI step before any build job, or
# (c) just remember to run it manually. Option (b) gives the
# most leverage for the least ceremony — every PR that touches
# .github/workflows/*.yml fails fast. The local install path
# is "just run it before tagging a release".

set -euo pipefail

# Default: every workflow file in the repo
if [ $# -eq 0 ]; then
    set -- .github/workflows/*.yml
fi

fail=0
for f in "$@"; do
    if [ ! -f "$f" ]; then
        echo "❌ not a file: $f"
        fail=1
        continue
    fi
    if ! python3 -c "import sys, yaml; yaml.safe_load(open(sys.argv[1]))" "$f" 2>/tmp/validate-ci.stderr; then
        echo "❌ YAML syntax error in $f:"
        sed 's/^/   /' /tmp/validate-ci.stderr
        fail=1
    else
        echo "✅ $f"
    fi
done
rm -f /tmp/validate-ci.stderr

if [ $fail -ne 0 ]; then
    echo ""
    echo "❌ One or more YAML files failed validation. Fix and re-run."
    exit 1
fi

echo "✅ All YAMLs impeccable."
exit 0
