# -*- coding: utf-8 -*-
"""Run Web Platform Tests' script tests (testharness.js) against tobira.

A testharness test is a page that runs assertions and reports each one as
pass or fail -- the same shape as test262, but driven by the page rather than
by a runner. WPT's own `testharnessreport.js` reports to a test runner over
`postMessage`; ours (vendored beside `testharness.js`) writes the results
into a `<pre>` instead, and this reads them back out of the dumped DOM.

    python tools/wpt/harness.py workers        # every vendored test under that path
    python tools/wpt/harness.py workers/Worker-structure-message.html
    python tools/wpt/harness.py workers --bless

`harness-baseline.txt` lists the individual assertions that pass, by
`<file>::<test name>`. One that stops passing fails the run, as in test262:
a page's total is not the gate, because a page that stops loading at all
would otherwise look like a page whose tests merely went quiet.
"""
import html
import os
import re
import subprocess
import sys
import threading
import http.server
import functools

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
ROOT = os.path.join(REPO, "tests", "fixtures", "wpt")
OUT = os.path.join(HERE, "out")
BASELINE = os.path.join(HERE, "harness-baseline.txt")
TOBIRA = os.environ.get(
    "TOBIRA_PATH", os.path.join(REPO, "target", "release", "tobira.exe")
)
PORT = int(os.environ.get("WPT_PORT", "8735"))
SETTLE_MS = os.environ.get("WPT_SETTLE_MS", "1500")


def serve():
    handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory=ROOT)
    handler.log_message = lambda *args, **kwargs: None
    server = http.server.ThreadingHTTPServer(("127.0.0.1", PORT), handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


def run_page(relative):
    """The lines our report shim wrote, or (None, why) if it never reported."""
    os.makedirs(OUT, exist_ok=True)
    dump = os.path.join(OUT, re.sub(r"[^A-Za-z0-9]+", "_", relative)[:80] + ".html")
    if os.path.exists(dump):
        os.remove(dump)
    env = dict(os.environ)
    env["TOBIRA_DUMP_DOM"] = dump
    env["TOBIRA_SETTLE_MS"] = SETTLE_MS
    env["TOBIRA_COLOR_SCHEME"] = "light"
    url = "http://127.0.0.1:%d/%s" % (PORT, relative)
    env["TOBIRA_DEBUG_CONSOLE"] = "1"
    try:
        finished = subprocess.run(
            [TOBIRA, "--cli", url], capture_output=True, timeout=120, env=env
        )
    except subprocess.TimeoutExpired:
        return None, "tobira did not finish in 120s"
    noise = (finished.stderr or b"").decode("utf-8", "replace")
    # The first thing that went wrong, which is what a page that never
    # reported needs said about it. A count with no reason is half a check.
    why = next(
        (
            line.strip()
            for line in noise.split("\n")
            if re.search(r"error|exception|panic|not defined|undefined is not", line, re.I)
        ),
        "no result block and nothing said",
    )
    if not os.path.exists(dump):
        return None, "no DOM dumped: " + why[:120]
    text = open(dump, encoding="utf-8", errors="replace").read()
    match = re.search(r'<pre id="__wpt_out">(.*?)</pre>', text, re.S)
    if not match:
        return None, why[:160]
    return [
        line.strip()
        for line in html.unescape(match.group(1)).split("\n")
        if line.strip()
    ], None


# WPT serves `foo.any.js` as several generated pages -- one per global the
# test asks for -- and keeps none of them in the repository. This writes the
# window one, which is the shape everything here can run, next to the script.
ANY_TEMPLATE = """<!doctype html>
<meta charset="utf-8">
<!-- Generated from %(script)s by tools/wpt/harness.py; WPT's server does
     this itself and keeps no file. Regenerated on every run. -->
<script src="/resources/testharness.js"></script>
<script src="/resources/testharnessreport.js"></script>
<div id="log"></div>
%(extra)s<script src="%(name)s"></script>
"""


def generate_any_pages(needle):
    """Write the window variant of every `*.any.js`, and name them."""
    generated = []
    for base, _dirs, files in os.walk(ROOT):
        for name in sorted(files):
            if not name.endswith(".any.js"):
                continue
            source = os.path.join(base, name)
            relative = os.path.relpath(source, ROOT).replace("\\", "/")
            if needle and needle not in relative:
                continue
            text = open(source, encoding="utf-8", errors="replace").read(4000)
            globals_line = re.search(r"//\s*META:\s*global=([^\n]*)", text)
            wanted = globals_line.group(1).strip() if globals_line else "window,worker"
            # `global=worker` alone means the test is not meant to run in a
            # document at all; the worker variants need a worker to run in.
            if "window" not in wanted and "default" not in wanted and wanted:
                continue
            extra = ""
            for script in re.findall(r"//\s*META:\s*script=([^\n]*)", text):
                extra += '<script src="%s"></script>\n' % script.strip()
            page = name[: -len(".js")] + ".html"
            out_path = os.path.join(base, page)
            open(out_path, "w", encoding="utf-8").write(
                ANY_TEMPLATE % {"script": name, "name": name, "extra": extra}
            )
            generated.append(os.path.relpath(out_path, ROOT).replace("\\", "/"))
    return generated


def collect(needle):
    tests, driven = [], []
    generated = set(generate_any_pages(needle))
    for base, _dirs, files in os.walk(ROOT):
        for name in sorted(files):
            if not name.endswith(".html"):
                continue
            relative = os.path.relpath(os.path.join(base, name), ROOT)
            relative = relative.replace("\\", "/")
            if needle and needle not in relative:
                continue
            path = os.path.join(base, name)
            text = open(path, encoding="utf-8", errors="replace").read(4000)
            if "testharness.js" not in text:
                continue
            # `testdriver.js` drives a real browser's input from outside it.
            # There is nothing to implement here; these are counted apart.
            if "testdriver" in text:
                driven.append(relative)
                continue
            tests.append(relative)
    return tests, driven, generated


def main():
    args = sys.argv[1:]
    bless = "--bless" in args
    args = [a for a in args if a != "--bless"]
    needle = args[0] if args else None

    server = serve()
    try:
        pages, driven, generated = collect(needle)
        if not pages:
            print("no testharness tests under %s" % (needle or ROOT))
            return
        passing, failed, dead = [], [], []
        for relative in pages:
            lines, why = run_page(relative)
            if lines is None:
                dead.append((relative, why))
                continue
            for line in lines:
                if line.startswith("PASS "):
                    passing.append("%s::%s" % (relative, line[5:]))
                elif line.startswith("HARNESS OK"):
                    pass
                else:
                    failed.append("%s\t%s" % (relative, line))
        total = len(passing) + len(failed)
        print(
            "wpt testharness: %d / %d assertions pass over %d pages"
            % (len(passing), total, len(pages) - len(dead))
        )
        print(
            "  %d pages never reported, %d need testdriver (not runnable here), "
            "%d pages generated from .any.js"
            % (len(dead), len(driven), len(generated))
        )
        reasons = {}
        for line in failed:
            why = line.split("--", 1)[-1].strip() if "--" in line else line.split("\t")[-1]
            reasons[why[:90]] = reasons.get(why[:90], 0) + 1
        for why, count in sorted(reasons.items(), key=lambda kv: -kv[1])[:12]:
            print("  %4d x %s" % (count, why))
        # Why a page never reported matters more than how many did not: it is
        # the engine's own failure, not the test's verdict.
        dead_reasons = {}
        for _relative, why in dead:
            dead_reasons[why] = dead_reasons.get(why, 0) + 1
        for why, count in sorted(dead_reasons.items(), key=lambda kv: -kv[1])[:8]:
            print("  DEAD %4d x %s" % (count, why))

        before = set()
        if os.path.exists(BASELINE):
            before = {
                line.strip()
                for line in open(BASELINE, encoding="utf-8")
                if line.strip()
            }
        if bless:
            # Only the pages this run visited are rewritten: blessing one
            # directory must not throw away the record of every other.
            ran = {relative for relative in pages}
            kept = {line for line in before if line.split("::", 1)[0] not in ran}
            with open(BASELINE, "w", encoding="utf-8") as handle:
                handle.write("\n".join(sorted(kept | set(passing))) + "\n")
            print(
                "harness-baseline.txt: %d assertions here, %d kept from elsewhere"
                % (len(passing), len(kept))
            )
            return
        # Only the pages this run actually visited are compared: running one
        # file must not report every other test in the baseline as lost.
        ran = {relative for relative in pages}
        before = {line for line in before if line.split("::", 1)[0] in ran}
        now = set(passing)
        gained = sorted(now - before)
        lost = sorted(before - now)
        if gained:
            print("newly passing (%d):" % len(gained))
            for name in gained[:30]:
                print("  + %s" % name)
        for name in lost:
            print("  - %s" % name)
        if lost:
            sys.exit(1)
    finally:
        server.shutdown()


main()
