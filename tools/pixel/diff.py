# -*- coding: utf-8 -*-
"""Compare what a page *looks like* between Chrome and tobira.

`tools/geom/cmp.py` reads `getBoundingClientRect`, so it sees where boxes
are and nothing else: colour, borders, shadows, the shape of the glyphs and
what covers what are invisible to it. This takes a screenshot from each and
subtracts them.

    python -m http.server 8731 --directory tools/geom &
    python tools/pixel/diff.py g1.html            # one page
    python tools/pixel/diff.py --all              # every geom probe, scored
    python tools/pixel/diff.py --all --bless      # write baseline.tsv

A page never reaches 0%: the two engines rasterise glyph edges differently,
so text areas differ by a few percent no matter what. The number to watch is
whether a page's own figure grew. `baseline.tsv` holds what each page scored,
and `--all` fails when one is worse by more than a tenth of a point.

Output per page: the share of pixels that differ, the box the differences sit
in, and which of the page's own elements that box lands on (read from the
`out` block the geom probes already write). A diff image goes to
`tools/pixel/out/<page>.png`: tobira on the left, Chrome on the right, the
difference mask under them.
"""
import html
import os
import re
import subprocess
import sys

from PIL import Image
import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
GEOM = os.path.join(REPO, "tools", "geom")
OUT = os.path.join(HERE, "out")
BASELINE = os.path.join(HERE, "baseline.tsv")

CHROME = os.environ.get(
    "CHROME_PATH", r"C:/Program Files/Google/Chrome/Application/chrome.exe"
)
TOBIRA = os.environ.get(
    "TOBIRA_PATH", os.path.join(REPO, "target", "release", "tobira.exe")
)
PORT = os.environ.get("GEOM_PORT", "8731")
WIDTH = int(os.environ.get("PIXEL_WIDTH", "1280"))
HEIGHT = int(os.environ.get("PIXEL_HEIGHT", "900"))
# How far apart two pixels must be before they count as different. Glyph
# edges land a shade apart everywhere; a real difference is not subtle.
THRESHOLD = int(os.environ.get("PIXEL_THRESHOLD", "32"))
# How much worse than the baseline a page may score before `--all` fails.
SLACK = float(os.environ.get("PIXEL_SLACK", "0.1"))


def shoot_chrome(url, path):
    subprocess.run(
        [
            CHROME,
            "--headless",
            "--disable-gpu",
            "--hide-scrollbars",
            "--force-device-scale-factor=1",
            "--user-data-dir=" + os.path.join(GEOM, "cud"),
            "--window-size=%d,%d" % (WIDTH, HEIGHT),
            "--virtual-time-budget=2500",
            "--screenshot=" + path,
            url,
        ],
        capture_output=True,
        timeout=180,
    )
    return os.path.exists(path)


def shoot_tobira(url, path):
    env = dict(os.environ)
    env["TOBIRA_DUMP_WIDTH"] = str(WIDTH)
    env["TOBIRA_SHOT_HEIGHT"] = str(HEIGHT)
    subprocess.run(
        [TOBIRA, "--screenshot", path, url],
        capture_output=True,
        timeout=240,
        env=env,
    )
    return os.path.exists(path)


def boxes_from_probe(page):
    """The `<id> <x>,<y> <w>x<h>` lines the geom probes write, as rectangles."""
    path = os.path.join(GEOM, page)
    if not os.path.exists(path):
        return []
    text = open(path, encoding="utf-8", errors="replace").read()
    boxes = []
    for name, x, y, w, h in re.findall(
        r"([A-Za-z][\w-]*)\s+(-?\d+),(-?\d+)\s+(\d+)x(\d+)", html.unescape(text)
    ):
        boxes.append((name, int(x), int(y), int(w), int(h)))
    return boxes


def compare(page, keep_image=True):
    url = "http://127.0.0.1:%s/%s" % (PORT, page)
    stem = page.rsplit(".", 1)[0]
    os.makedirs(OUT, exist_ok=True)
    left_path = os.path.join(OUT, stem + ".tobira.png")
    right_path = os.path.join(OUT, stem + ".chrome.png")
    for path in (left_path, right_path):
        if os.path.exists(path):
            os.remove(path)
    if not shoot_chrome(url, right_path):
        return None, "chrome wrote no image (is the server on %s?)" % PORT
    if not shoot_tobira(url, left_path):
        return None, "tobira wrote no image"

    left = Image.open(left_path).convert("RGB")
    right = Image.open(right_path).convert("RGB")
    # tobira stops at the page's own height; compare the part both drew.
    width = min(left.width, right.width)
    height = min(left.height, right.height)
    a = np.asarray(left.crop((0, 0, width, height)), dtype=np.int16)
    b = np.asarray(right.crop((0, 0, width, height)), dtype=np.int16)
    delta = np.abs(a - b).max(axis=2)
    mask = delta > THRESHOLD
    share = 100.0 * mask.sum() / mask.size

    rows = np.nonzero(mask.any(axis=1))[0]
    cols = np.nonzero(mask.any(axis=0))[0]
    if len(rows):
        box = (int(cols[0]), int(rows[0]), int(cols[-1]), int(rows[-1]))
    else:
        box = None

    if keep_image:
        canvas = Image.new("RGB", (width * 2, height * 2), (24, 24, 24))
        canvas.paste(left.crop((0, 0, width, height)), (0, 0))
        canvas.paste(right.crop((0, 0, width, height)), (width, 0))
        shown = np.zeros((height, width, 3), dtype=np.uint8)
        shown[..., 0] = mask * 255
        shown[..., 1] = (delta.clip(0, 255)).astype(np.uint8) // 2
        canvas.paste(Image.fromarray(shown), (0, height))
        canvas.save(os.path.join(OUT, stem + ".png"))

    note = ""
    if box is not None:
        hits = [
            name
            for name, x, y, w, h in boxes_from_probe(page)
            if x < box[2] and x + w > box[0] and y < box[3] and y + h > box[1]
        ]
        note = " box=%d,%d..%d,%d" % box
        if hits:
            note += " over " + ",".join(hits[:6])
    return share, "%5.2f%%%s" % (share, note)


def read_baseline():
    scores = {}
    if os.path.exists(BASELINE):
        for line in open(BASELINE, encoding="utf-8"):
            parts = line.split()
            if len(parts) == 2:
                scores[parts[0]] = float(parts[1])
    return scores


def main():
    args = [a for a in sys.argv[1:]]
    if not args:
        print(__doc__)
        return
    bless = "--bless" in args
    args = [a for a in args if a != "--bless"]

    if args == ["--all"]:
        pages = sorted(
            name
            for name in os.listdir(GEOM)
            if name.endswith(".html")
        )
    else:
        pages = args

    baseline = read_baseline()
    scores = {}
    worse = []
    for page in pages:
        share, text = compare(page)
        print("%-22s %s" % (page, text))
        if share is None:
            continue
        scores[page] = share
        was = baseline.get(page)
        if was is not None and share > was + SLACK:
            worse.append("%s %.2f%% -> %.2f%%" % (page, was, share))

    if scores:
        print(
            "mean %.2f%% over %d pages"
            % (sum(scores.values()) / len(scores), len(scores))
        )
    if bless:
        with open(BASELINE, "w", encoding="utf-8") as handle:
            for page in sorted(scores):
                handle.write("%s\t%.2f\n" % (page, scores[page]))
        print("baseline.tsv written: %d pages" % len(scores))
        return
    for line in worse:
        print("WORSE " + line)
    if worse:
        sys.exit(1)


main()
