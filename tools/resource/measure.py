# -*- coding: utf-8 -*-
"""What a page costs: memory and CPU, tobira against Chrome.

Being light is the second goal this browser states for itself, and until now
nothing measured it. Five instruments say whether tobira is *right*; this one
says what being right costs.

    python -m http.server 8731 --directory tools/geom &
    python tools/resource/measure.py --pages       # the six reference pages
    python tools/resource/measure.py g1.html       # a geom probe
    python tools/resource/measure.py --pages --bless

Per page: peak working set, CPU seconds, and wall time, for each browser, and
tobira as a share of Chrome. `baseline.tsv` holds what each page cost, and a
run fails when a page costs more than `RESOURCE_SLACK` (25%) over it.

**Chrome is many processes and tobira is one.** Measuring `chrome.exe` alone
leaves out the renderer, the GPU process and the utilities -- most of the
memory -- and tobira would look several times heavier than it is for no
reason but the shape of the question. Every process in the tree is counted,
on both sides, which is the same lesson as asking both browsers for the same
`prefers-color-scheme`.

Peak working set, not current: the number that decides whether a machine
swaps is the high-water mark, and a browser that frees memory just before it
exits still needed it.
"""
import os
import subprocess
import sys
import time

import psutil

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
GEOM = os.path.join(REPO, "tools", "geom")
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
# How much more a page may cost than its baseline before the run fails.
SLACK = float(os.environ.get("RESOURCE_SLACK", "0.25"))
# How often the process tree is sampled. Peak working set is read from the OS
# per process, so this only has to be often enough to catch a child that
# lives and dies between samples.
POLL_S = float(os.environ.get("RESOURCE_POLL_S", "0.05"))

REFERENCE_PAGES = [
    "https://react.dev/",
    "https://ja.wikipedia.org/wiki/\u30d6\u30e9\u30a6\u30b6",
    "https://developer.mozilla.org/en-US/",
    "https://vuejs.org/",
    "https://news.ycombinator.com/",
    "https://lobste.rs/",
]


def page_url(page):
    if page.startswith("http://") or page.startswith("https://"):
        return page
    return "http://127.0.0.1:%s/%s" % (PORT, page)


def watch(command, env=None):
    """Run a command, following every process it starts.

    Returns (peak working set in bytes, CPU seconds, wall seconds). A child
    that exits before the next sample still counts: its peak and its CPU time
    are taken when it is last seen, and kept.
    """
    started = time.time()
    process = subprocess.Popen(
        command,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        env=env,
    )
    try:
        root = psutil.Process(process.pid)
    except psutil.NoSuchProcess:
        return 0, 0.0, time.time() - started

    # Per pid, the largest peak and the most CPU seen. Keeping them per pid
    # rather than summing every sample means a process is counted once, at
    # its own high-water mark.
    peaks, cpu = {}, {}
    while process.poll() is None:
        try:
            tree = [root] + root.children(recursive=True)
        except psutil.NoSuchProcess:
            break
        for member in tree:
            try:
                info = member.memory_info()
                times = member.cpu_times()
            except (psutil.NoSuchProcess, psutil.AccessDenied):
                continue
            peak = getattr(info, "peak_wset", info.rss)
            peaks[member.pid] = max(peaks.get(member.pid, 0), peak)
            cpu[member.pid] = max(
                cpu.get(member.pid, 0.0), times.user + times.system
            )
        time.sleep(POLL_S)
    process.wait()
    return sum(peaks.values()), sum(cpu.values()), time.time() - started


def measure_chrome(url):
    return watch(
        [
            CHROME,
            "--headless",
            "--disable-gpu",
            "--hide-scrollbars",
            "--force-device-scale-factor=1",
            "--user-data-dir=" + os.path.join(GEOM, "cud"),
            "--window-size=%d,%d" % (WIDTH, HEIGHT),
            "--virtual-time-budget=2500",
            "--blink-settings=preferredColorScheme=1",
            "--screenshot=" + os.path.join(HERE, "chrome.png"),
            url,
        ]
    )


def measure_tobira(url):
    env = dict(os.environ)
    env["TOBIRA_DUMP_WIDTH"] = str(WIDTH)
    env["TOBIRA_SHOT_HEIGHT"] = str(HEIGHT)
    env["TOBIRA_COLOR_SCHEME"] = "light"
    return watch(
        [TOBIRA, "--screenshot", os.path.join(HERE, "tobira.png"), url], env=env
    )


def mib(value):
    return value / (1024.0 * 1024.0)


def read_baseline():
    scores = {}
    if os.path.exists(BASELINE):
        for line in open(BASELINE, encoding="utf-8"):
            parts = line.rstrip("\n").split("\t")
            if len(parts) == 3:
                scores[parts[0]] = (float(parts[1]), float(parts[2]))
    return scores


def main():
    args = sys.argv[1:]
    bless = "--bless" in args
    args = [a for a in args if a != "--bless"]
    if not args:
        print(__doc__)
        return
    if args == ["--pages"]:
        pages = REFERENCE_PAGES
    elif args == ["--all"]:
        pages = sorted(n for n in os.listdir(GEOM) if n.endswith(".html"))
    else:
        pages = args

    baseline = read_baseline()
    measured, worse = {}, []
    print(
        "%-34s %10s %10s %6s  %8s %8s  %8s %8s"
        % (
            "page",
            "tob MiB",
            "chr MiB",
            "share",
            "tob cpu",
            "chr cpu",
            "tob wall",
            "chr wall",
        )
    )
    for page in pages:
        url = page_url(page)
        t_mem, t_cpu, t_wall = measure_tobira(url)
        c_mem, c_cpu, c_wall = measure_chrome(url)
        share = (100.0 * t_mem / c_mem) if c_mem else 0.0
        name = page if len(page) <= 34 else page[:31] + "..."
        print(
            "%-34s %10.1f %10.1f %5.0f%%  %8.2f %8.2f  %8.2f %8.2f"
            % (name, mib(t_mem), mib(c_mem), share, t_cpu, c_cpu, t_wall, c_wall)
        )
        measured[page] = (mib(t_mem), t_cpu)
        was = baseline.get(page)
        if was and was[0] > 0 and mib(t_mem) > was[0] * (1 + SLACK):
            worse.append(
                "%s memory %.1f -> %.1f MiB" % (page, was[0], mib(t_mem))
            )
        if was and was[1] > 0 and t_cpu > was[1] * (1 + SLACK):
            worse.append("%s cpu %.2f -> %.2f s" % (page, was[1], t_cpu))

    if measured:
        print(
            "mean: tobira %.1f MiB, %.2f CPU s over %d pages"
            % (
                sum(v[0] for v in measured.values()) / len(measured),
                sum(v[1] for v in measured.values()) / len(measured),
                len(measured),
            )
        )
    if bless:
        kept = {k: v for k, v in baseline.items() if k not in measured}
        with open(BASELINE, "w", encoding="utf-8") as handle:
            for page in sorted(set(kept) | set(measured)):
                value = measured.get(page, kept.get(page))
                handle.write("%s\t%.1f\t%.2f\n" % (page, value[0], value[1]))
        print("baseline.tsv: %d pages here, %d kept" % (len(measured), len(kept)))
        return
    for line in worse:
        print("WORSE " + line)
    if worse:
        sys.exit(1)


main()
