# Web Platform Tests, the reftests

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
