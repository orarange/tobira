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
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.tags = Counter()
        self.text = 0
        self.skip = 0
        self.ids = set()
        self.classes = Counter()

    def handle_starttag(self, tag, attrs):
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
        if tag in ("script", "style") and self.skip:
            self.skip -= 1

    def handle_data(self, data):
        if not self.skip:
            self.text += len(re.sub(r"\s+", " ", data).strip())


def stat(path):
    s = Stat()
    s.feed(open(path, encoding="utf-8", errors="replace").read())
    return s


def main():
    files = sys.argv[1:]
    stats = [(f, stat(f)) for f in files]
    for f, s in stats:
        print("%-24s elements %6d  text %7d  ids %5d" % (f, sum(s.tags.values()), s.text, len(s.ids)))
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


if __name__ == "__main__":
    main()
