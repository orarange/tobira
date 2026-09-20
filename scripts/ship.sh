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

# The same look, one step later. `git add -A` sweeps a new file straight
# past the check above -- it is staged by then, not untracked -- which is how
# 24 commits in one day went out under an `add` the repo's own rules forbid.
# Nothing wrong was committed that day, but the day's work had created three
# new kinds of generated file, and only `.gitignore` stood between them and
# the history. So: say what is about to be added for the first time, and make
# somebody agree to it.
added="$(git diff --cached --name-only --diff-filter=A)"
if [ -n "$added" ] && [ "${SHIP_ALLOW_NEW:-0}" != "1" ]; then
  count="$(printf '%s
' "$added" | wc -l | tr -d ' ')"
  echo "$count file(s) would be added to the repository for the first time:" >&2
  printf '%s
' "$added" | head -40 >&2
  if [ "$count" -gt 40 ]; then
    echo "  ... and $((count - 40)) more" >&2
  fi
  echo "read the list; if they all belong in the history, set SHIP_ALLOW_NEW=1" >&2
  exit 4
fi

# CI builds without --release, and a debug build's stack frames are fatter:
# a recursion guard tuned on release frames overflowed there, and CI was red
# for twelve runs while this script reported the suite passing. The engine
# crate is where that bites, so it is checked in debug as well.
cargo test --lib 2>&1 | tr -d '\000' | grep -aE "^test result|FAILED|panicked" \
  | awk '/^test result/{p+=$4; f+=$6; next} {print} END {if (p == "") {print "debug: no test results"; exit 1} print "debug passed=" p, "failed=" f; exit (f > 0)}'

cargo test --release 2>&1 | tr -d '\000' | grep -aE "^test result|FAILED|panicked" \
  | awk '/^test result/{p+=$4; f+=$6; next} {print} END {print "passed=" p, "failed=" f; exit (f > 0)}'
TOBIRA_GC_VERIFY=1 cargo test --release 2>&1 | tr -d '\000' | grep -aE "^test result|gc-verify" \
  | awk '/^test result/{p+=$4; f+=$6; next} {print} END {print "gc passed=" p, "failed=" f; exit (f > 0)}'

git commit -q -m "$1"
git push origin master 2>&1 | tail -1
git rev-parse --short origin/master
