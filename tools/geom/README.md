# Geometry probes

Synthetic pages whose only job is to make Chrome and tobira disagree out loud.
Each one ends with a script that writes `<id> <x>,<y> <w>x<h>` for a list of
elements into `<pre id="out">`; `cmp.py` reads that from both browsers and
prints every line that differs.

```
python -m http.server 8731 --directory tools/geom
python tools/geom/cmp.py g4.html
```

| page | what it probes | last score |
|------|----------------|-----------:|
| `g1.html` | box model, flex, grid, position | 28/29 |
| `g2.html` | inline formatting, line breaking | 13/22 |
| `g3.html` | inline-block baselines | 6/7 |
| `g4.html` | inline element hitboxes | 5/14 |
| `g5.html` | modern CSS (custom properties, logical props, clamp) | 22/26 |
| `g6.html` | font-family name resolution | 9/12 |

Scores are from 2026-09-04 at 1280px. They are not asserted anywhere — this is
a hand-run tool, not a test. `g2` and `g4` are the weak pair and both are about
the same thing: how many rectangles an inline box owns and where they split.
