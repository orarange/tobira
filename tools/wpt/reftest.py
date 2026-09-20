# -*- coding: utf-8 -*-
"""Run Web Platform Tests reftests against tobira, with no other browser.

A reftest is a pair of pages that must **render identically**: the test, and
a reference written in markup whose behaviour is not in question. So it needs
one browser, not two -- tobira draws both and the two pictures are compared to
each other. There is no glyph-rasterisation noise to allow for, because the
same engine drew both sides, so a pass is pixel-exact.

That is the difference from `tools/pixel/diff.py`, which asks "does this look
like Chrome?" and can only ever answer in percentages. This one asks "does
this engine agree with itself about what the spec says?" and answers yes or
no.

    python tools/wpt/reftest.py                 # every vendored test
    python tools/wpt/reftest.py linebox         # only paths containing that
    python tools/wpt/reftest.py --bless         # record today's passes

`baseline.txt` lists the tests that pass; one that stops passing fails the
run, as in `tests/fixtures/test262`.
"""
import os
import re
import subprocess
import sys
import threading
import http.server
import functools

from PIL import Image
import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
ROOT = os.path.join(REPO, "tests", "fixtures", "wpt")
OUT = os.path.join(HERE, "out")
BASELINE = os.path.join(HERE, "baseline.txt")
TOBIRA = os.environ.get(
    "TOBIRA_PATH", os.path.join(REPO, "target", "release", "tobira.exe")
)
PORT = int(os.environ.get("WPT_PORT", "8734"))
WIDTH = int(os.environ.get("WPT_WIDTH", "800"))
HEIGHT = int(os.environ.get("WPT_HEIGHT", "600"))
# Anti-aliasing is deterministic within one engine, so the only slack needed
# is for nothing at all. A pixel that differs at all differs.
THRESHOLD = int(os.environ.get("WPT_THRESHOLD", "0"))


def serve():
    handler = functools.partial(
        http.server.SimpleHTTPRequestHandler, directory=ROOT
    )
    handler.log_message = lambda *args, **kwargs: None
    server = http.server.ThreadingHTTPServer(("127.0.0.1", PORT), handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


def shoot(url, path):
    env = dict(os.environ)
    env["TOBIRA_DUMP_WIDTH"] = str(WIDTH)
    env["TOBIRA_SHOT_HEIGHT"] = str(HEIGHT)
    env["TOBIRA_COLOR_SCHEME"] = "light"
    if os.path.exists(path):
        os.remove(path)
    try:
        subprocess.run(
            [TOBIRA, "--screenshot", path, url],
            capture_output=True,
            timeout=120,
            env=env,
        )
    except subprocess.TimeoutExpired:
        return False
    return os.path.exists(path)


def references(path):
    """The `match` / `mismatch` links a test carries, as (kind, relative url)."""
    text = open(path, encoding="utf-8", errors="replace").read()
    found = []
    for rel, href in re.findall(
        r'<link\s+[^>]*rel=["\']?(match|mismatch)["\']?[^>]*>', text
    ) and re.findall(
        r'<link\s+(?:[^>]*\s)?rel=["\']?(match|mismatch)["\']?[^>]*href=["\']([^"\']+)["\']',
        text,
    ) or []:
        found.append((rel, href))
    if not found:
        for rel, href in re.findall(
            r'<link\s+(?:[^>]*\s)?href=["\']([^"\']+)["\'][^>]*rel=["\']?(match|mismatch)["\']?',
            text,
        ):
            found.append((href, rel)[::-1])
    return found


def collect():
    tests = []
    for base, _dirs, files in os.walk(ROOT):
        for name in sorted(files):
            if not name.endswith(".html") or name.endswith("-ref.html"):
                continue
            path = os.path.join(base, name)
            for kind, href in references(path):
                relative = os.path.relpath(path, ROOT).replace("\\", "/")
                tests.append((relative, kind, href))
                break
    return tests


def compare(test_png, ref_png):
    left = Image.open(test_png).convert("RGB")
    right = Image.open(ref_png).convert("RGB")
    width = max(left.width, right.width)
    height = max(left.height, right.height)
    canvas_a = Image.new("RGB", (width, height), (255, 255, 255))
    canvas_b = Image.new("RGB", (width, height), (255, 255, 255))
    canvas_a.paste(left, (0, 0))
    canvas_b.paste(right, (0, 0))
    delta = np.abs(
        np.asarray(canvas_a, dtype=np.int16) - np.asarray(canvas_b, dtype=np.int16)
    ).max(axis=2)
    differing = int((delta > THRESHOLD).sum())
    return differing, delta.size


def main():
    args = sys.argv[1:]
    bless = "--bless" in args
    args = [a for a in args if a != "--bless"]
    needle = args[0] if args else None

    if not os.path.isdir(ROOT):
        print("no vendored tests at %s" % ROOT)
        return
    os.makedirs(OUT, exist_ok=True)
    server = serve()
    try:
        tests = collect()
        if needle:
            tests = [t for t in tests if needle in t[0]]
        passing, failures = [], []
        for relative, kind, href in tests:
            base = "http://127.0.0.1:%d/%s" % (PORT, relative)
            ref_url = base.rsplit("/", 1)[0] + "/" + href if not href.startswith(
                "/"
            ) else "http://127.0.0.1:%d%s" % (PORT, href)
            stem = re.sub(r"[^A-Za-z0-9]+", "_", relative)[:80]
            test_png = os.path.join(OUT, stem + ".test.png")
            ref_png = os.path.join(OUT, stem + ".ref.png")
            if not shoot(base, test_png) or not shoot(ref_url, ref_png):
                failures.append((relative, "one side drew nothing"))
                continue
            differing, total = compare(test_png, ref_png)
            same = differing == 0
            if same == (kind == "match"):
                passing.append(relative)
            else:
                failures.append(
                    (
                        relative,
                        "%d of %d pixels differ (%s expected)"
                        % (differing, total, kind),
                    )
                )
        print(
            "wpt reftests: %d / %d pass" % (len(passing), len(passing) + len(failures))
        )
        for relative, why in failures:
            print("  FAIL %s\t%s" % (relative, why))

        if bless:
            with open(BASELINE, "w", encoding="utf-8") as handle:
                handle.write("\n".join(sorted(passing)) + "\n")
            print("baseline.txt written: %d tests" % len(passing))
            return
        before = set()
        if os.path.exists(BASELINE):
            before = {
                line.strip()
                for line in open(BASELINE, encoding="utf-8")
                if line.strip()
            }
        now = set(passing)
        for gained in sorted(now - before):
            print("  + %s" % gained)
        lost = sorted(before - now)
        for name in lost:
            print("  - %s" % name)
        if lost:
            sys.exit(1)
    finally:
        server.shutdown()


main()
