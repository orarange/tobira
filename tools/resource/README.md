# What a page costs

Being light is the second goal this browser states for itself (HANDOFF,
設計判断). Five instruments say whether tobira is *right*; none of them said
what being right costs, and the memory-shaped changes of the last days --
workers with a `Vm` each, glyph caches, a stylesheet memo -- went in
unmeasured.

```
python -m http.server 8731 --directory tools/geom &
python tools/resource/measure.py --pages          # the six reference pages
python tools/resource/measure.py g1.html          # a geom probe
python tools/resource/measure.py --pages --bless  # record the cost
```

Per page and per browser: **peak working set**, CPU seconds and wall seconds.
`baseline.tsv` holds tobira's, and a run fails when a page costs more than
`RESOURCE_SLACK` (25%) over what it cost before.

**Chrome is many processes and tobira is one.** Measuring `chrome.exe` alone
leaves out the renderer, the GPU process and the utilities -- most of the
memory -- so tobira would look several times heavier than it is for no reason
but the shape of the question. Every process in the tree is counted, on both
sides. It is the same lesson as asking both browsers for the same
`prefers-color-scheme`: **a comparison is only worth reading if both sides
were asked the same thing.**

Peak, not current: the number that decides whether a machine swaps is the
high-water mark, and a browser that frees its memory just before exiting
still needed it.

## First run, 2026-09-20, 1280x900

| page | tobira MiB | Chrome MiB | share | tobira CPU | Chrome CPU | tobira wall | Chrome wall |
|------|-----------:|-----------:|------:|-----------:|-----------:|------------:|------------:|
| news.ycombinator.com | **50.7** | 510.5 | **10%** | 0.33 | 1.66 | 1.51 | 1.09 |
| developer.mozilla.org | 297.1 | 588.2 | 51% | 2.12 | 2.48 | 2.89 | 2.73 |
| lobste.rs | 300.5 | 526.4 | 57% | 1.70 | 2.06 | 7.57 | 1.16 |
| react.dev | 460.7 | 604.8 | 76% | **38.56** | 2.58 | **59.46** | 0.97 |
| vuejs.org | 518.9 | 636.4 | 82% | 4.67 | 2.50 | 6.26 | 1.80 |
| ja.wikipedia.org | **599.1** | 566.2 | **106%** | 8.42 | 2.28 | 34.42 | 1.97 |

Two things the first run said, neither of them guessed beforehand:

**Memory is not the problem; CPU is.** tobira is lighter than Chrome on five
of six pages, and on a page without script it is a tenth of it. But react.dev
costs **fifteen times Chrome's CPU and sixty times its wall clock**, and
Wikipedia is the one page that costs *more memory* than Chrome.

**The settle loop is where the CPU goes.** Splitting react.dev:

```
TOBIRA_SETTLE_MS=0        6.06 CPU s    7.38 s wall
TOBIRA_SETTLE_MS=2000    38.05 CPU s   57.93 s wall   (the default)
TOBIRA_DYNAMIC_SCRIPTS=0 37.72 CPU s   61.78 s wall   (so: not the scripts)
```

### What the settle actually spends it on (measured, 2026-09-20)

Not the frame count -- the cost of one layout. Varying the budget:

| budget | geometry feeds | tobira CPU |
|-------:|---------------:|-----------:|
| 0 | 1 | 6.06 s |
| 250 ms | 6 | 11.14 s |
| 500 ms | 10 | 15.00 s |
| 1000 ms | 18 | 22.08 s |
| 2000 ms | 35 | 38.09 s |

**The slope is not the layout.** Timing both halves (`TOBIRA_TIME_LAYOUT=1`)
says where it really goes:

```
tick(changed=true)   n=34   48.9 s total   1437 ms each   <-- here
tick(changed=false)  n=91    0.1 s total      1 ms each
layout_styled_document n=35   1.5 s total     44 ms each
```

A whole-document layout costs **44 ms**, not 930. What costs 1.4 seconds is
applying a snapshot on a frame where the DOM changed: serialising the
document, parsing it again and rebuilding the styled tree. Thirty-four times.

Straight line: **0.93 CPU seconds per feed**, on a 5.6 s base. Each feed is
one `layout_styled_document` of the whole document, and react.dev is 1846
elements. The settle runs 125 frames but only 35 of them change anything, so
**the lever is the price of a layout, not the number of frames.**

Ending the loop early does not help here: the page mutates every three or
four frames for the whole two seconds (five `setInterval(fn, 60)` of its
own), so it never holds still. The loop does now stop after eight unchanged
frames -- which is what "settled" means -- but on this page that never comes.

**Thirty-two CPU seconds are spent settling one page.** The settle loop lays
the whole document out again every frame (HANDOFF, 2026-09-18) and react.dev
is 1846 elements. The on-demand layout that `layout_dirty` / `ensure_layout`
provides is evidently not reaching this path. That is the first thing to look
at, and it is worth remembering that the settle is also what took react.dev
from 62 elements to Chrome's 1846: **the cost bought correctness, and now the
cost can be seen.**
