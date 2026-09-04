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

## arrow*.html — HN の投票矢印がなぜ出んかったか（2026-09-04）

`arrow.html` が形の軸（表・`<center>`・`<a>` の中の `<div>`・空要素）を一つずつ外す。
どれも無関係やった。`arrow2.html` が背景の書き方を分ける。`arrow3.html` が
`background-repeat` だけを変える。`arrow4.html` が SVG と PNG で比べる。

結論: **`background-size` の長さ指定が丸ごと落ちとった。** `BackgroundSize` に
長さの変種が無く、`background-size: 10px` が `Auto` に化けとった。そこへ
`background-repeat: repeat`（CSS の初期値。HN の `no-repeat` は 2 層目にあるので
1 層目には効かん、これは仕様どおり）が重なると、画像は原寸で敷かれる。
HN は 10x10 の箱に 32x32 の SVG なので、見えとる左上 10x10 が空白で、
矢印が丸ごと消えて見えとった。2026-09-05 に修正済み。

途中で「敷き詰めが未実装」と読んだが**それは違うかった**。敷き詰め自体は
`draw_tiled_image` で動いとって、原寸で敷くのが問題やった。単色の PNG やと
埋まって見えるので、そこで止まると原因を取り違える。

**期待値は Chrome で確定させること。** `arrow3.html` の r4 は Chrome でも空白で、
それが正しい。「全部出たら勝ち」やない。

`dot.png` は 8x8 の赤一色、`triangle.svg` は HN から取ってきた実物。
