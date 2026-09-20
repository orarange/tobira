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
| `g2.html` | inline formatting, line breaking | 14/22 |
| `g3.html` | inline-block baselines | 6/7 |
| `g4.html` | inline element hitboxes | 11/14 |
| `g5.html` | modern CSS (custom properties, logical props, clamp) | 23/26 |
| `g6.html` | font-family name resolution | 10/12 |
| `g6b.html` | which face each generic family resolves to | 17/32 |
| `lineheight.html` | `line-height: normal` at every size | 22/22 |
| `tablew.html` | table widths in a narrow parent | 6/6 |
| `anim.html` | `@keyframes` at load | 5/5 |
| `anim2.html` | animation held at a chosen moment | 9/9 |
| `anim3.html` | the easing curves | 14/14 |
| `layer.html` | opacity / transform layers | 5/5 |
| `radius.html` | `border-radius` shorthand forms | 5/6 |
| `radius2.html` | each corner read back separately | 7/8 |
| `faces.html` | which font file each family actually loads | 4/8 |
| `units.html` | every length unit, resolved | 4/9 |
| `xform.html` | `transform` boxes | 5/6 |
| `mask.html` | `mask-image` | a picture, not a score: no `<pre id="out">`, look at it |
| `scrollbar.html` | when a scrollbar takes room | 7/7 |
| `overflow.html` | overflow in a narrow parent | 10/17 |
| `overflow2.html` | `overflow-x` / `-y` set apart | 12/12 |
| `clipaxis.html` | whether one-axis clipping costs anything | 8/9 |
| `fsize.html` | text width at fractional type sizes | 3/12 |
| `lineheight2.html` | line height at fractional type sizes | 7/14 |
| `sup.html` | superscripts and `vertical-align` | 2/10 |
| `details.html` | a closed `<details>` folds its contents away | 3/6 |
| `hidden.html` | the `hidden` attribute | 7/7 |
| `carousel.html` | a horizontally scrolling strip of cards | 11/13 |
| `flexgap.html` | `gap` on a flex row, and the row's own box | 9/9 |
| `visuallyhidden.html` | screen-reader-only text, and where it sits | 4/7 |
| `noah.html` | fifty unclosed `<font>`: the Noah's Ark clause caps them at three per paragraph | 5/5 |

Scores are from **2026-09-20** at 1280px, every page rescored the same day
against the Chrome of that day: **283 / 368 = 76.9%** over the 31 pages that
carry an `out` block. They are not asserted anywhere — this is a hand-run
tool, not a test. Re-run the lot and put today's numbers here rather than
trusting the column; the old ones had drifted by ten days.

`arrow.html`, `arrow2.html`, `arrow3.html`, `arrow4.html` and `mask.html`
write no `out` block. They are pictures to look at with `--screenshot`, not
probes `cmp.py` can score, and it says "chrome: no output" for them.

Weakest first, which is where to work: `sup` 2/10, `fsize` 3/12, `details`
3/6, `faces` 4/8, `units` 4/9, `visuallyhidden` 4/7, `g6b` 17/32,
`overflow` 10/17, `lineheight2` 7/14, `g2` 14/22. **`fsize` and
`lineheight2` share one cause**: `ComputedStyle::font_size_px` is a `u32`, so
`font-size: 0.8333em` computes 13 where Chrome keeps 13.3333, and every text
width downstream drifts.

**Read which axis is wrong before deciding what a page is telling you.** Three
of these were misread that way. `tablew` sat at 0/6 and looked like a table
problem; every width and height in it was already right and only `y` was
wrong, by one more pixel per row — it was the line height. `fsize` looked like
a rounding problem; the whole-number sizes were wrong too, which made it a
floor instead. `overflow` mixed two unrelated causes in one score.

`g2` and `g4` are the weak pair and both are about the same thing: how many
rectangles an inline box owns and where they split. `sup` is the same root.

Two of these pages exist to identify something rather than to score:
`g6b.html` names candidate faces so the width says which face a generic family
resolved to, and `anim2`/`anim3` hold an animation still (a negative delay plus
`paused`) so a moment part-way through one can be compared at all.

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

## sup.html — 上付き・下付き・vertical-align（2026-09-10）

`g2` の 1px ずれを追う途中で作った。Chrome との差:

| | Chrome | tobira |
|---|---|---|
| `x<sup>2</sup>y` の div の高さ | 22 | 21 |
| `x<sub>2</sub>y` の div の高さ | 21 | 19 |
| `vertical-align:super` の div | 24 | 18 |
| `<sub>` 自身の箱 | 7x15 | 7x19 |
| `font-size:10px` の span の箱 | 6x11 | 6x17 |

分かっとること:

- **`vertical-align: super` / `sub` が効いとらん**（`<sup>`/`<sub>` タグは効く）。
  d3 の div が Chrome の 24 に対して 18 で、span が持ち上がっとらん。
- **`<sup>`/`<sub>` の字が小さうなっとらん**。Chrome は 15px 相当の箱、tobira は
  親と同じ 19。
- **インライン要素が自分の字やのうて親の字の高さを返す場合がある。**
  `font-size:10px` の span が 17（親の 16px の内容領域）を返す。
  原因は `apply_inline_marks` の逃げ道が container の字で高さを出すこと。

**2026-09-10 に直そうとして三度失敗した。** 逃げ道の発動条件を変える、
開き印と閉じ印を run の前後に分ける、の二案とも `g4` を 10/14 から
6〜8/14 に落とした。閉じ印が run より先に処理される構造そのものを変える
必要があって、その順序に他の計算がぶら下がっとる。**着手するなら
`emit_line_impl` の走査順を設計からやり直すこと。部分的に触ると壊れる。**
