# -*- coding: utf-8 -*-
"""Compare a probe page's geometry between Chrome and tobira.

Each probe page ends with a script that writes one line per element into
`<pre id="out">`:

    <id> <x>,<y> <w>x<h>

Chrome is asked for the page with `--dump-dom`, tobira with `--cli`, and the
two sets of lines are compared name by name. Only the lines Chrome produced
are checked, so a probe can carry extra output for a human to read.

Usage (from the repo root):

    python -m http.server 8731 --directory tools/geom &
    python tools/geom/cmp.py g1.html

The pages must be served over HTTP rather than opened from disk: tobira's
loader only speaks http/https.
"""
import html
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))

CHROME = os.environ.get(
    "CHROME_PATH", r"C:/Program Files/Google/Chrome/Application/chrome.exe"
)
TOBIRA = os.environ.get(
    "TOBIRA_PATH", os.path.join(REPO, "target", "release", "tobira.exe")
)
PORT = os.environ.get("GEOM_PORT", "8731")


def extract(text):
    match = re.search(r'<pre id="out">(.*?)</pre>', text, re.S)
    if not match:
        return None
    return html.unescape(match.group(1)).strip()


def chrome(url):
    out = subprocess.run(
        [
            CHROME,
            "--headless",
            "--disable-gpu",
            # A throwaway profile, or a running Chrome steals the invocation.
            "--user-data-dir=" + os.path.join(HERE, "cud"),
            "--window-size=1280,900",
            "--virtual-time-budget=2500",
            "--dump-dom",
            url,
        ],
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        timeout=180,
    )
    return extract(out.stdout or "")


def tobira(url):
    out = subprocess.run([TOBIRA, "--cli", url], capture_output=True, timeout=240)
    # The CLI renders the <pre> as plain text among the rest of the page.
    return out.stdout.decode("utf-8", "replace").replace("\x00", "")


def main():
    if len(sys.argv) < 2:
        print(__doc__)
        return
    page = sys.argv[1]
    url = "http://127.0.0.1:%s/%s" % (PORT, page)

    left = chrome(url)
    if left is None:
        print("chrome: no output (is the server on %s?)" % PORT)
        return
    right_raw = tobira(url)

    want = {}
    for line in left.split("\n"):
        parts = line.strip().split(" ", 1)
        if len(parts) == 2:
            want[parts[0]] = parts[1]

    got = {}
    for line in right_raw.split("\n"):
        parts = line.strip().split(" ", 1)
        if len(parts) == 2 and parts[0] in want:
            got[parts[0]] = parts[1].strip()

    same = 0
    for name in want:
        mine = got.get(name, "MISSING")
        if mine == want[name]:
            same += 1
        else:
            print("  %-12s chrome=%-22s tobira=%s" % (name, want[name], mine))
    print("match %d/%d" % (same, len(want)))


main()
