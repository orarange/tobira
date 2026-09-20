# Web Platform Tests

Two runners, because WPT holds two kinds of test.

- `harness.py` runs the **script tests** (`testharness.js`): a page makes
  assertions and reports each as pass or fail, the same shape as test262.
- `reftest.py` runs the **reftests**: two pages that must draw identically.

Both keep a baseline of what passes and fail when something that passed
stops passing.

## The script tests

```
python tools/wpt/harness.py workers          # everything under that path
python tools/wpt/harness.py css/cssom --bless
python tools/wpt/harness.py                  # all of it
```

WPT's own `testharnessreport.js` reports to an external runner over
`postMessage`. Ours -- the one file under `resources/` that is not theirs --
writes the results into a `<pre id="__wpt_out">` instead, and the runner reads
them back out of `TOBIRA_DUMP_DOM`.

**A page that never reported is counted apart from a page whose tests
failed**, with the first error said out loud. The two are different things:
one is the engine falling over, the other is the engine's answer. A count
with no reason is half a check.

Also counted apart: pages that need `testdriver.js`, which drives a real
browser's input from outside it and is not something to implement here.

`foo.any.js` tests are served by WPT as generated pages, one per global, and
none of them exist in the repository. The runner writes the **window**
variant beside the script before each run (`*.any.html`, regenerated every
time, not committed). The worker variants wait on `Worker`.

**A page that waits longer than the settle never reports.** The runner gives
each page `WPT_SETTLE_MS` (1500 by default) and then reads what it wrote, so a
test that waits four seconds for a timer is counted as never reporting rather
than as failing. Raising it changes the numbers a little and the wall-clock a
lot: `workers` scores 51 at 1500ms and 54 at 5000ms. Quote the setting with
the number.

First run, 2026-09-20, over everything vendored:
**3186 / 8933 assertions, 677 pages; 184 pages never reported, 82 need
testdriver.** By area:

| area | assertions | note |
|------|-----------:|------|
| `html/webappapis/timers` | 14 / 16 | |
| `css/cssom` | 1598 / 3457 | `sheet.cssRules` missing is 139 of the failures |
| `dom/events` | 79 / 417 | 77 more pages need testdriver |
| `workers` | 17 / 309 | `Worker is not defined` is 123 of them |

# The reftests

A reftest is two pages that must **draw identically**: the test, and a
reference built from markup whose behaviour is not in question. That takes one
browser, not two. `reftest.py` has tobira draw both and compares the two
pictures to each other, so a pass is **pixel-exact** -- the same engine drew
both sides, so there is no anti-aliasing to make allowances for.

That is the difference from `tools/pixel/diff.py`, which asks "does this look
like Chrome?" and can only answer in percentages. This asks "does this engine
agree with itself about what the spec says?", and the answer is yes or no.

```
python tools/wpt/reftest.py                # every vendored test
python tools/wpt/reftest.py linebox        # only paths containing that
python tools/wpt/reftest.py --bless        # record today's passes
```

`tests/fixtures/wpt/` holds the vendored subset, from
https://github.com/web-platform-tests/wpt at `269bca0` (2026-09-20):
`css/CSS2/linebox/` (line boxes, `vertical-align`, leading -- the family the
geometry probes are weakest in), `css/reference/nothing.html`, which many
tests match against, and `fonts/` for Ahem. `LICENSE.md` is theirs.

To add a directory:

```
git clone --depth 1 --filter=blob:none --sparse https://github.com/web-platform-tests/wpt.git
cd wpt && git sparse-checkout set css/CSS2/linebox css/<dir> fonts css/reference
```

copy it in, run with `--bless`, and commit `baseline.txt` with it.

First run, 2026-09-20: **6 / 11**. The five failures are all the same family:
`fractional-line-height` (the page comes out 8px taller than the reference),
`vertical-align-negative-leading-001`, `line-breaking-font-size-zero-001`,
`split-inline-borders`, `iframe-in-block-in-inline`.

Two things to know before reading a failure:

- **tobira has no `@font-face`.** Tests that name a web font get whatever
  system face the fallback picks -- on both sides, so a reftest is still
  meaningful, but a test *about* a font will not be.
- A screenshot is as tall as the page, so a test whose page is a different
  height from its reference differs over that whole band. The pixel count in
  the failure line includes it; the interesting number is whether it is zero.
