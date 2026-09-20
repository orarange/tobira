# What the page looks like

`tools/geom/cmp.py` reads `getBoundingClientRect`, so it sees where the boxes
are and nothing else. Colour, borders, backgrounds, shadows, the shape of the
glyphs, what covers what: nothing measured any of that until this.

`diff.py` takes a screenshot from Chrome and one from tobira at the same
width and subtracts them.

```
python -m http.server 8731 --directory tools/geom &
python tools/pixel/diff.py g1.html          # one page, and a diff image
python tools/pixel/diff.py --all            # all 36 probes, checked against baseline.tsv
python tools/pixel/diff.py --all --bless    # record today's figures as the baseline
```

Each line is the share of pixels that differ by more than `PIXEL_THRESHOLD`
(32 per channel by default), the box those differences sit in, and which of
the probe's own named elements that box covers. `out/<page>.png` is tobira on
the left, Chrome on the right, and the difference mask below.

**No page reaches 0%.** Two engines rasterise glyph edges differently, so
anything with text differs by a couple of percent whatever happens. The
figure to watch is a page's own, against what it scored before:
`baseline.tsv` holds that, and `--all` exits non-zero when a page is worse by
more than a tenth of a point. Bless deliberately, and read the diff of
`baseline.tsv` as the record of what a change did to the picture.

First run, 2026-09-20, 1280x900, **mean 3.20% over 36 pages**. Worst:
`carousel` 9.08%, `fsize` 6.29%, `arrow3` 5.44%, `g6b` 5.01%, `anim` 4.92%.
Best: `mask` 0.81%, `lineheight` 1.06%, `anim2` 1.47%, `layer` 1.53%.

This scores the five pages `cmp.py` cannot (`arrow.html` … `arrow4.html`,
`mask.html`): they write no `out` block because they are pictures, and a
picture is exactly what this compares.

Knobs: `PIXEL_WIDTH` / `PIXEL_HEIGHT` (1280x900), `PIXEL_THRESHOLD`,
`PIXEL_SLACK` (0.1 points), `GEOM_PORT`, `CHROME_PATH`, `TOBIRA_PATH`.
`out/` and the screenshots in it are not committed.
