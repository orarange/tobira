#!/usr/bin/env bash
# Test, then commit, then push -- as one command, so nobody has to join the
# chain by hand. Stops at the first red step. Usage, from the repo root:
#
#   scripts/ship.sh "commit message"
#
# The message may span lines. Anything already staged is committed; stage
# first. Side effects that are not part of the chain (stopping a local test
# server, say) belong before this script, not inside it.
set -euo pipefail
if [ $# -lt 1 ]; then
  echo "usage: scripts/ship.sh \"commit message\"" >&2
  exit 2
fi
cd "$(dirname "$0")/.."

# An untracked file is either something that should have been staged or
# something that should not be here (OneDrive once restored a source file a
# commit had deleted). Either way, look before shipping.
untracked="$(git ls-files --others --exclude-standard)"
if [ -n "$untracked" ] && [ "${SHIP_ALLOW_UNTRACKED:-0}" != "1" ]; then
  echo "untracked files; stage them, remove them, or set SHIP_ALLOW_UNTRACKED=1:" >&2
  echo "$untracked" >&2
  exit 3
fi

cargo test --release 2>&1 | tr -d '\000' | grep -aE "^test result|FAILED|panicked" \
  | awk '/^test result/{p+=$4; f+=$6; next} {print} END {print "passed=" p, "failed=" f; exit (f > 0)}'
TOBIRA_GC_VERIFY=1 cargo test --release 2>&1 | tr -d '\000' | grep -aE "^test result|gc-verify" \
  | awk '/^test result/{p+=$4; f+=$6; next} {print} END {print "gc passed=" p, "failed=" f; exit (f > 0)}'

git commit -q -m "$1"
git push origin master 2>&1 | tail -1
git rev-parse --short origin/master
