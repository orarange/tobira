# -*- coding: utf-8 -*-
"""Count what is in a serialized DOM, to put two of them side by side.

    python tools/scripterr/domstat.py chrome.html off.html on.html

Prints, per file: elements, text characters (whitespace squeezed), and the
tag histogram; then the tags whose counts differ between the files. This is
for "did the scripts change the document", not for geometry -- for that,
`tools/geom/`.
"""
import re
import sys
from collections import Counter
from html.parser import HTMLParser


class Stat(HTMLParser):
    # <template> content is inert in a browser, and a declarative shadow
    # root (`<template shadowrootmode>`) is folded into a shadow tree that
    # Chrome's --dump-dom does not print. Counting it would make a page look
    # bigger for holding what the other side rendered elsewhere, so it is
    # left out of the counts and reported on its own line.
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.tags = Counter()
        self.text = 0
        self.skip = 0
        self.in_template = 0
        self.templates = 0
        self.shadow_roots = 0
        self.ids = set()
        self.classes = Counter()
        self.words = Counter()

    def handle_starttag(self, tag, attrs):
        if tag == "template":
            self.templates += 1
            if any(k == "shadowrootmode" for k, _ in attrs):
                self.shadow_roots += 1
            self.in_template += 1
            return
        if self.in_template:
            return
        self.tags[tag] += 1
        if tag in ("script", "style"):
            self.skip += 1
        for k, v in attrs:
            if k == "id" and v:
                self.ids.add(v)
            if k == "class" and v:
                for c in v.split():
                    self.classes[c] += 1

    def handle_endtag(self, tag):
        if tag == "template" and self.in_template:
            self.in_template -= 1
            return
        if self.in_template:
            return
        if tag in ("script", "style") and self.skip:
            self.skip -= 1

    def handle_data(self, data):
        if self.skip or self.in_template:
            return
        squeezed = re.sub(r"\s+", " ", data).strip()
        self.text += len(squeezed)
        if squeezed:
            self.words[squeezed] += 1


def stat(path):
    s = Stat()
    s.feed(open(path, encoding="utf-8", errors="replace").read())
    return s


def main():
    files = sys.argv[1:]
    stats = [(f, stat(f)) for f in files]
    for f, s in stats:
        print(
            "%-24s elements %6d  text %7d  ids %5d  templates %3d (shadow roots %3d)"
            % (f, sum(s.tags.values()), s.text, len(s.ids), s.templates, s.shadow_roots)
        )
    if len(stats) < 2:
        return
    base_name, base = stats[0]
    for f, s in stats[1:]:
        print("--- %s vs %s: tags that differ" % (f, base_name))
        for tag in sorted(set(base.tags) | set(s.tags)):
            a, b = base.tags[tag], s.tags[tag]
            if a != b:
                print("  %-12s %5d -> %5d" % (tag, a, b))
        only_here = sorted(s.ids - base.ids)
        only_base = sorted(base.ids - s.ids)
        if only_here:
            print("  ids only in %s: %s" % (f, " ".join(only_here[:30])))
        if only_base:
            print("  ids only in %s: %s" % (base_name, " ".join(only_base[:30])))
        # Text runs on one side only. Same element count with different
        # words is a JS that did or did not rewrite something (lobste.rs
        # showed "9 hours ago" against "2026-09-17 11:44:39"), and the
        # element counts alone never show it.
        text_here = [w for w in s.words if w not in base.words]
        text_base = [w for w in base.words if w not in s.words]
        print("  text runs only in %s: %d%s" % (f, len(text_here), "  e.g. " + " | ".join(w[:40] for w in text_here[:5]) if text_here else ""))
        print("  text runs only in %s: %d%s" % (base_name, len(text_base), "  e.g. " + " | ".join(w[:40] for w in text_base[:5]) if text_base else ""))


if __name__ == "__main__":
    main()
