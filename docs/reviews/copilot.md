見直しました。**前回指摘した主要な3点（`@media` の brace 解析、`calc()` の優先順位、rgba/hsla の雑な扱い）はかなり改善されています。**  
今回の最新マージは、**前回よりだいぶレビューしやすい状態**です。

ただし、まだ **“仕様上の簡略化” が “対応済み” に見える箇所** は残っています。  
なので評価は **前回の「高リスク」→ 今回は「中リスク」** に下がりました。

---

## PR Summary

対象は現在の最新 merge commit:

- merge commit: [`91cc671`](https://github.com/orarange/tobira/commit/91cc6712ddd48757ad9ed68b430b7b16f3c3372f)
- 実質変更 commit: [`7fda6c9`](https://github.com/orarange/tobira/commit/7fda6c9c6a88256edb9a953f67f9e39ec72c6b10)
- 差分: [`80f7d51...91cc671`](https://github.com/orarange/tobira/compare/80f7d51fc1c3a5b9874b8b8e263d066e25090e03...91cc6712ddd48757ad9ed68b430b7b16f3c3372f)

commit message からも意図が明確です。

- `fix @media brace parsing, calc() precedence, rgba blending, add 15 tests`

### 総評
**前回レビューに対する返しとしてかなり良い修正**です。

- 直したポイントが適切
- テストがしっかり増えている
- “見かけだけ対応” だった箇所を、少なくとも一段階ましにしている

---

## Core Changes

今回の主眼は、前回の大型 CSS 拡張に対する **correctness 補強** です。

特に以下が重要です。

1. `@media` の closing brace 探索を修正
2. `calc()` を乗除優先に修正
3. `rgba()/hsla()` を白背景ブレンドに変更
4. selector / media / calc / rgba まわりのテスト追加

---

## Other Changes

## 1. `@media` brace parsing 修正は妥当
前回一番危険だった箇所です。  
今回は depth tracking を入れていて、改善として正しいです。

```rust name=src/css.rs url=https://github.com/orarange/tobira/blob/91cc6712ddd48757ad9ed68b430b7b16f3c3372f/src/css.rs#L424-L439
fn find_matching_close_brace(source: &str) -> Option<usize> {
    let mut depth: u32 = 1;
    for (i, ch) in source.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}
```

```rust name=src/css.rs url=https://github.com/orarange/tobira/blob/91cc6712ddd48757ad9ed68b430b7b16f3c3372f/src/css.rs#L446-L453
while let Some(open_offset) = source[cursor..].find('{') {
    ...
    let block_text_raw = &source[block_start..];
    let Some(close_offset) = find_matching_close_brace(block_text_raw) else {
        break;
    };
```

### 評価
- 前回の本質的な不具合は直っている
- 最低限必要な修正として十分良い

### ただし
文字列中の `{` `}` を考慮していないので、**CSS string literal や url() を含む複雑ケースではまだ壊れうる**です。  
ただ、今の project scope を考えると許容範囲です。

---

## 2. `calc()` は明確に改善
前回の「左から順評価」は危険でしたが、今回は `*` `/` を先に collapse してから `+` `-` を処理しています。

```rust name=src/css.rs url=https://github.com/orarange/tobira/blob/91cc6712ddd48757ad9ed68b430b7b16f3c3372f/src/css.rs#L2076-L2104
// Pass 1: collapse * and / (higher precedence than + and -)
let mut i = 0;
while i < ops.len() {
    match ops[i] {
        '*' => {
            values[i] *= values[i + 1];
            values.remove(i + 1);
            ops.remove(i);
        }
        '/' if values[i + 1] != 0.0 => {
            values[i] /= values[i + 1];
            values.remove(i + 1);
            ops.remove(i);
        }
        _ => i += 1,
    }
}

// Pass 2: evaluate + and -
let mut result = values[0];
for (op, val) in ops.iter().zip(values[1..].iter()) {
    match op {
        '+' => result += val,
        '-' => result -= val,
        _ => {}
    }
}
```

### 評価
- 前回よりずっとよい
- 少なくとも commit message の意図どおり直っている
- テストも追加されている

```rust name=src/css.rs url=https://github.com/orarange/tobira/blob/91cc6712ddd48757ad9ed68b430b7b16f3c3372f/src/css.rs#L2804-L2811
#[test]
fn calc_multiplication_has_higher_precedence_than_addition() {
    // calc(2px + 3 * 4px) should be 2 + 12 = 14, NOT (2+3)*4 = 20
    ...
    assert_eq!(p.style.font_size_px, 14, "multiplication must bind tighter than addition");
}
```

### まだ残る懸念
- unary minus
- nested `calc(calc(...))`
- parentheses grouping
- CSS proper unit algebra

は未対応です。  
でも今は“危険な嘘実装”から“限定的だが自然な簡易実装”に進んだと見てよいです。

---

## 3. rgba/hsla の扱いは改善した
前回は alpha を閾値で捨てていましたが、今回は **白背景にブレンド** しています。

```rust name=src/css.rs url=https://github.com/orarange/tobira/blob/91cc6712ddd48757ad9ed68b430b7b16f3c3372f/src/css.rs#L2165-L2179
if let Some(arguments) = value
    .strip_prefix("rgba(")
    .and_then(|rest| rest.strip_suffix(')'))
{
    ...
    let a = parts[3].trim().parse::<f32>().ok()?.clamp(0.0, 1.0);
    if a == 0.0 {
        return None;
    }
    return Some(blend_with_white(r, g, b, a));
}
```

```rust name=src/css.rs url=https://github.com/orarange/tobira/blob/91cc6712ddd48757ad9ed68b430b7b16f3c3372f/src/css.rs#L2284-L2289
fn blend_with_white(r: u8, g: u8, b: u8, alpha: f32) -> Color {
    let blend = |channel: u8| -> u8 {
        (channel as f32 * alpha + 255.0 * (1.0 - alpha)).round() as u8
    };
    rgb(blend(r), blend(g), blend(b))
}
```

### 評価
これは **現レンダラが alpha compositing を持っていない前提では、かなり現実的な落とし所** です。  
少なくとも 0.5 を境に消えるよりは圧倒的に良いです。

---

## 4. テスト追加はかなり良い
今回の一番良い点のひとつです。  
attribute selector / pseudo-class / media / calc / rgba に対してテストが増えています。

代表例:

```rust name=src/css.rs url=https://github.com/orarange/tobira/blob/91cc6712ddd48757ad9ed68b430b7b16f3c3372f/src/css.rs#L2634-L2642
#[test]
fn attribute_exists_selector_matches() {
    let document = parse_document("<div><a href=\"#\">link</a><span>plain</span></div>");
    let stylesheet = parse_stylesheet("[href] { color: #ff0000; }");
    let styled = build_styled_tree(&document, &stylesheet, 1280);
    let a = find_first_element(&styled, "a").expect("a should exist");
    let span = find_first_element(&styled, "span").expect("span should exist");
    assert_eq!(a.style.color, 0xFF0000);
    assert_ne!(span.style.color, 0xFF0000);
}
```

```rust name=src/css.rs url=https://github.com/orarange/tobira/blob/91cc6712ddd48757ad9ed68b430b7b16f3c3372f/src/css.rs#L2761-L2777
#[test]
fn media_max_width_filters_rules_by_viewport() {
    let document = parse_document("<p>Hello</p>");
    let stylesheet = parse_stylesheet(
        "p { color: #0000ff; } @media (max-width: 600px) { p { color: #ff0000; } }",
    );
    ...
}
```

```rust name=src/css.rs url=https://github.com/orarange/tobira/blob/91cc6712ddd48757ad9ed68b430b7b16f3c3372f/src/css.rs#L2836-L2845
#[test]
fn rgba_half_transparent_blends_with_white() {
    let color = parse_color("rgba(0, 0, 0, 0.5)").expect("should return a color");
    ...
}
```

### 評価
前回の不安をかなり軽減しています。  
**“実装した” だけでなく “意図を固定した”** のが良いです。

---

## Merge Readiness and Risk Assessment

### 前回より改善した点
- `@media` の根本バグ修正
- `calc()` の根本バグ修正
- rgba/hsla の改善
- テスト増加

### 現時点の評価
**マージ品質としてはかなり改善。前回よりだいぶ安全。**

ただし、まだ以下は残っています。

---

## 残る懸念点

## 1. sibling combinator は依然として部分対応
ここは前回と同じです。  
複雑 selector ではまだ不完全です。

```rust name=src/css.rs url=https://github.com/orarange/tobira/blob/91cc6712ddd48757ad9ed68b430b7b16f3c3372f/src/css.rs#L1537-L1560
// Sibling combinators: for now we only partially support them
// (we'd need sibling data threaded into ancestor matching)
Combinator::AdjacentSibling | Combinator::GeneralSibling => {
    ...
    } else {
        return false;
    }
}
```

### コメント
これはコードコメントで自認しているので誠実ですが、  
**実装済み扱いで README や説明を出すと誤解を生む** ので注意です。

---

## 2. `vh` の換算値が `parse_length` と `resolve_calc_operand_f32` で不一致
これは今回見つかった小さめだけど気になる点です。

`parse_length` では:

```rust name=src/css.rs url=https://github.com/orarange/tobira/blob/91cc6712ddd48757ad9ed68b430b7b16f3c3372f/src/css.rs#L1992-L1998
if let Some(number) = value.strip_suffix("vh") {
    return parse_float(number).map(|p| (p * 800.0 / 100.0).round() as u32);
}
```

`calc()` 内部では:

```rust name=src/css.rs url=https://github.com/orarange/tobira/blob/91cc6712ddd48757ad9ed68b430b7b16f3c3372f/src/css.rs#L2122-L2127
if let Some(n) = t.strip_suffix("vh") {
    return parse_float(n).map(|f| f * 7.2); // viewport 720px
}
```

### 問題
- 通常の `vh` は 800px 基準
- `calc()` 内の `vh` は 720px 基準

**同じ単位が文脈で別値になる** のはバグです。

### 優先度
中。すぐ直したい。

---

## 3. `find_matching_close_brace` は文字列・エスケープ未考慮
今は簡易 CSS parser なので即 NG ではないですが、将来的に

- `content: "}"`  
- data URL
- quoted strings

が入ると壊れる可能性があります。

### ただし
今のスコープでは許容可能です。  
これは “将来の改善点” レベル。

---

## 4. テストは増えたが、まだ足りない箇所がある
今回の追加は良いです。  
でも、追加機能の広さに対してはまだ不足があります。

不足気味:
- `+` / `~` selector tests
- `:nth-child(2n+1)` のような一般式
- `min-width` / `max-width`
- `line-height`
- `text-indent`
- `letter-spacing`
- `font` shorthand
- `hsla()`
- nested `@media` or multiple media blocks

---

## 5. README / capability 説明がまだ古い
実装はかなり増えたのに README は前のままです。  
これは今の段階では結構重要です。

---

## 最終評価

### 前回からの変化
- **前回:** 方向性は良いが correctness が弱く高リスク
- **今回:** 主要な correctness 問題をちゃんと潰しており、かなり改善

### 現時点の判断
**この最新マージは概ね良い修正です。**  
前回よりかなり安心して読めま��。

### ただしブロッカー候補
もし厳密に見るなら、今の懸念の中で一番実害がありそうなのはこれです。

- **`vh` の基準値不一致**

それ以外は現時点では「仕様簡略化」または「今後の改善」で済むものが多いです。

---

## 結論

**再レビュー結果: 前回より明確に良くなった。 approve 寄りだが、小修正なら `vh` 不一致だけ直したい。**

---

## Want me to
1. **この最新マージに対する “approve / request changes” コメント文をそのまま書く**
2. **残っている懸念点だけを短く3行でまとめる**
3. **`vh` 不一致の修正案をコードレベルで示す**
4. **今回追加された CSS 対応項目を README 用に整理する**