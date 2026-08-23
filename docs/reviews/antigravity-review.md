# Tobira ブラウザ — GUI / CSS レビュー

> レビュー日時: 2026-06-20  
> 対象コミット: HEAD (orarange/tobira)  
> レビュアー: Antigravity

## 概要

| モジュール | 行数 | 責務 |
|-----------|------|------|
| `src/gui.rs` | 5,549 | ウィンドウ管理、イベントハンドリング、ピクセルレンダリング、ブラウザ Chrome UI |
| `src/css.rs` | 5,748 | CSS パース、セレクタマッチング、スタイル計算、カスケード |
| `src/layout.rs` | 5,420 | レイアウト計算（ブロック、インライン、Flex、Grid）、DrawCommand 生成 |
| `src/font.rs` | 709 | フォントラスタライザ（fontdue）、テキスト描画 |
| `src/render.rs` | 269 | テキストベースレンダリング（Markdown 出力）|

**合計: ~16,750 行** — 外部 GUI フレームワーク不使用のフルスクラッチ実装。

---

## アーキテクチャ全体像

```
HTML (html.rs)
    └─ DOM Tree
CSS (css.rs)
    └─ Stylesheet
         └─ build_styled_tree()
               └─ StyledNode Tree
                     └─ layout_styled_document() (layout.rs)
                           └─ LayoutDocument + DrawCommands
                                 └─ paint_layout() / render_commands() (gui.rs)
                                       └─ Pixel Buffer (softbuffer)
                                             └─ Window (winit)
```

---

## ✅ 良い点（Strengths）

### 1. 堅実なアーキテクチャ分離

- CSS パース → スタイルツリー → レイアウト → ペイント の段階が明確に分離
- `StyledNode` が CSS 計算結果、`LayoutDocument` / `DrawCommand` がレイアウト結果をきちんと分けている
- `ComputedStyle` がほぼ全 CSS プロパティをカバー（80+ フィールド）

### 2. 非同期レンダリングパイプライン

- `start_load_worker` / `start_render_worker` でナビゲーションとレンダリングを別スレッドで実行
- `navigation_id` / `render_id` によるリクエストの競合管理が適切
- `content_dirty` フラグでアニメーション中の再レンダリングをコアレス（合体）

### 3. CSS 機能の網羅性

- **Flexbox**: direction, wrap, grow/shrink, basis, gap, order, align-items/self/content, justify-content 全対応
- **Grid**: template-columns/rows, fr/px/%, repeat(), auto-placement, span, auto-rows/columns
- **セレクタ**: descendant, child, sibling, attribute selectors, `:not()`, `:nth-child()`, `:hover/:focus/:active`
- **擬似要素**: `::before`, `::after`, `::placeholder`
- **カスタムプロパティ**: `var()` 置換、`@media` 内 root vars の条件分岐
- **フィルタ**: `blur()`, `brightness()`, `opacity()` のパースとレンダリング

### 4. ブラウザ Chrome UI

- カスタムタイトルバー（decorations off）＋手動ドラッグ移動（OS モーダルループを回避）
- アドレスバーの全機能：テキスト選択、Ctrl+A/C/X/V、ワード単位移動、IME 対応
- 戻る/進む/リロード/ナビゲーション + スクロール位置表示

### 5. インクリメンタル再スタイリング

- `build_styled_tree_incremental` で dirty root の dirty spine を計算し、変更のないサブツリーを丸ごと再利用
- 毎アニメーションフレームでの全再スタイリングを回避

### 6. レイヤーコンポジション

- `LayerCommand` で opacity / blur / brightness / scale / rotate をオフスクリーンバッファに描画
- `scratch` バッファプール（深度インデックス）でフレーム間再利用、フレームごとのアロケーション回避

---

## ⚠️ 注意点・改善の余地

### A. ファイルサイズと構造

> **警告**: 3ファイル（gui.rs, css.rs, layout.rs）がそれぞれ **5,400〜5,750行** — 非常に巨大。

- `gui.rs` が「イベントハンドリング」「Chrome 描画」「ピクセルレベルの描画関数」「アドレスバーエディタ」「フォーム制御」を全部含む
- `css.rs` が「パーサ」「セレクタマッチング」「プロパティ適用（`apply_declaration` 500行超）」「カラーパース」を全部含む

**提案**: モジュール分割

```
src/gui/
    mod.rs       (BrowserApp, イベントループ)
    chrome.rs    (タイトルバー、ボタン、アドレスバー描画)
    paint.rs     (draw_rect, draw_rounded_rect 等のピクセル描画)
    events.rs    (キーボード、マウス、IME ハンドリング)

src/css/
    mod.rs       (Stylesheet, ComputedStyle 型定義)
    parser.rs    (parse_stylesheet, strip_comments 等)
    selector.rs  (Selector, SimpleSelector, matches())
    properties.rs (apply_declaration, parse_color 等)
```

---

### B. CSS パーサの制限

> **重要**: 現在の CSS パーサは **正規表現/文字列ベース** で、W3C CSS Syntax Module Level 3 の tokenizer/parser ではない。

#### 問題点 1: `strip_comments` + `find('{')`

```rust
// css.rs:970 付近
while let Some(open_offset) = source[cursor..].find('{') {
```

- 文字列リテラル内の `{` に脆弱（例: `content: "{"` がブロック境界として誤認される可能性）
- `@charset`, `@import`, `@font-face` 等の @ ルールはスキップ/未対応

#### 問題点 2: `var()` の不完全なパース

```rust
// css.rs:1962-1964 付近
let Some(end) = result[inner_start..].find(')') else { break; };
```

- `var(--x, rgb(1,2,3))` のようなフォールバック値に `)` が含まれる場合に壊れる
- 反復回数制限（10回）で保護されているが、正しいパースではない

#### 問題点 3: `@media` 条件の制限

```rust
// css.rs:1088 付近
fn parse_media_condition(query: &str) -> MediaCondition {
```

- `and` / `or` / `not` の組合せ非対応
- `@media screen and (max-width: 768px)` は `screen` 部分を無視して `max-width` のみ認識
- `prefers-color-scheme: dark` はハードコード `false`（ダークモード非対応）

---

### C. スタイルカスケードの問題

#### 問題点 1: Specificity 計算

```rust
// css.rs:1833-1841 付近
applicable.extend(rule.declarations.iter().cloned().enumerate().map(
    |(declaration_index, declaration)| {
        (selector.specificity(), rule_index * 100 + declaration_index, declaration)
    },
));
```

- `selector.specificity()` は単一の `usize`（0〜1000スケール？）— CSS の (a, b, c) 3次元 specificity ではなく、数値一つにフラット化
- `inline_style` の specificity が `1_000` 固定

#### 問題点 2: `!important` 非対応

- `parse_inline_declarations` はプロパティ値をそのまま保存
- `!important` がある宣言を分離してカスケード順を変更する処理がない

#### 問題点 3: `RuleIndex` がテストのみ

```rust
// css.rs:27
#[cfg(test)]
rule_index: RuleIndex,
```

- 本番ビルドでは `RuleIndex::build` → `candidates_for` → 即座に破棄（毎スタイル計算で再構築）
- **修正**: `Stylesheet` に常に `RuleIndex` を保持し、`extend()` 時にも `rebuild()` を呼ぶ

---

### D. レイアウトエンジンの制限

#### 問題点 1: 負のマージン非対応

- `EdgeSizes` の全フィールドが `u32` — 負のマージン不可
- マージン折りたたみ（margin collapsing）が未実装

#### 問題点 2: `float` 未実装

- `float: left/right` のパース・レイアウトがない
- 多くの従来型サイトのレイアウトが崩れる原因

#### 問題点 3: テーブルレイアウト未実装

- `<table>` は通常のブロック要素として扱われる

---

### E. GUI レンダリングの問題

#### 問題点 1: アルファブレンディング非対応

- `draw_rect` / テキスト描画が直接ピクセル上書き — 半透明テキスト、サブピクセルレンダリングなし
- `effective_opacity` はレイヤーコンポジション時のみ適用

#### 問題点 2: グラデーション描画のパフォーマンス

```rust
// gui.rs:4541-4647 付近
for py in py_start..py_end {
    for px in gx..(gx + gw_u) {
        // ピクセルごとに角度計算 + 停止点補間
    }
}
```

- ピクセルごとのループで三角関数計算 — SIMD/プリコンピュートなし
- `border_radius` のコーナー判定もピクセルごと

#### 問題点 3: スクロールバー非表示

- スクロール位置はステータスバーの `scroll: X / Y px` テキストのみ
- ビジュアルスクロールバーがない

#### 問題点 4: `AddressBarState` の二重利用

```rust
// gui.rs:2611 付近
struct AddressBarState { ... }
// アドレスバーにも、ページ内テキスト入力にも同じ型を使用
struct FocusedPageInput {
    editor: AddressBarState,  // ← 流用
}
```

→ `TextEditorState` にリネーム推奨

---

### F. 型設計の気になる点

#### 問題点 1: `Color = u32` にアルファなし

```rust
pub type Color = u32;  // 0xRRGGBB のみ
```

- `rgba()` / `hsla()` をパースする際、アルファ値が捨てられる
- 半透明の色が opacity レイヤー経由でしか表現できない

#### 問題点 2: 整数エンコーディングの乱立

| フィールド | エンコーディング |
|-----------|----------------|
| `opacity` | 0–255 (u8) |
| `line_height` | 千分率 em (u32, 0=normal) |
| `transform_scale_x` | 千分率 (u32, 0=not set → 1000) |
| `transform_rotate_millideg` | millidegrees (i32) |
| `angle_deg_x1000` | degrees × 1000 (i32) |
| `flex_grow` / `flex_shrink` | × 100 整数 |

各フィールドにコメントは付いているが、統一的な固定小数点 abstraction がない。

#### 問題点 3: `ComputedStyle` の肥大化（~90 フィールド）

- `Clone` が頻繁に呼ばれる（テキストノードごとに親スタイルを clone）
- 継承プロパティ（color, font-size, etc.）と非継承プロパティ（border, padding, etc.）を分離し、継承側だけ `Rc` 共有すると効率的

---

## 🔧 実装済みだが部分的な機能（`🔧`マーク）

| 機能 | 現状 | 影響 |
|------|------|------|
| `position: sticky` | パース済み、relative として配置 | 実際のスクロール追従なし → ヘッダ固定サイトで崩れる |
| `transform: scale/rotate` | パース済み、LayerCommand にフィールドあり | ソフトウェアレンダラに実装なし → 回転/拡大が無視される |
| CSS `animation` / `@keyframes` | パース時に無視 | アニメーションが一切動かない |
| `transition` | 値は保存 | 状態変化時の補間なし |
| `@supports` | always-true 扱い | フィーチャ未サポート時にフォールバックが効かない |

---

## 📊 テストカバレッジ

- `css.rs` 内にインラインテスト（`#[cfg(test)]` ブロック）あり
- `tests/` ディレクトリに **43 ファイル** — ただしほぼ全て **JS エンジン/DOM テスト**
- **GUI / CSS / Layout のユニットテストが極めて少ない**
  - `RuleIndex` テスト用に `compute_style_naive` があるが、呼び出し箇所は `#[cfg(test)]` のみ
  - レイアウト結果（DrawCommand の座標）を検証するテストが見当たらない

> **推奨**: 入力 HTML + CSS → 期待する DrawCommand リスト のスナップショットテストを追加。レイアウトのリグレッション防止に効果的。

---

## 🎯 優先度付き改善提案

### 🔴 High Priority

| # | 改善内容 | 理由 |
|---|---------|------|
| 1 | **`!important` 対応** | 実サイトの CSS で頻出。カスケード順が崩れると全体のスタイルが狂う |
| 2 | **`float` の基本実装** | 従来型サイトのレイアウトに不可欠 |
| 3 | **`var()` パーサの修正** | ネストしたカッコを含むフォールバック値を正しくパース |
| 4 | **`RuleIndex` の本番有効化** | スタイル計算が O(rules × elements) → O(matching × elements) に改善 |

### 🟡 Medium Priority

| # | 改善内容 | 理由 |
|---|---------|------|
| 5 | **ファイル分割** | gui.rs / css.rs / layout.rs をそれぞれ 2〜3 モジュールに分割 |
| 6 | **負のマージン + マージン折りたたみ** | Box Model の正確性 |
| 7 | **スクロールバー UI** | UX の基本 |
| 8 | **`ComputedStyle` のスリム化** | `Rc` 共有または継承/非継承の分離 |

### 🟢 Low Priority

| # | 改善内容 | 理由 |
|---|---------|------|
| 9 | **RGBA カラーサポート** | `Color` 型をアルファ対応に拡張 |
| 10 | **CSS アニメーション基盤** | `@keyframes` パース + 補間ランタイム |
| 11 | **`transform: scale/rotate` レンダリング** | アフィン変換のソフトウェア実装 |
| 12 | **テーブルレイアウト** | `<table>` の列幅計算 |

---

## まとめ

Tobira は**外部レンダリングエンジン不使用**のフルスクラッチ Rust ブラウザとして、非常に充実した実装です。Flexbox / Grid / CSS カスタムプロパティ / フィルタ / ポジショニングが一通り動くのは大きな成果です。

**最大の課題:**

1. **CSS パーサが ad-hoc（文字列ベース）** — `gemini.md` / `GEMINI.md` の指針通り、CSS Syntax Module Level 3 準拠のトークナイザへの移行が理想
2. **巨大ファイルの分割** — 保守性・チーム開発の観点で急務
3. **テストの不足** — GUI/CSS/Layout のユニットテストがほぼ皆無

JS エンジン（vm.rs: 16,351行 + テスト43本）の充実度に比べると、GUI/CSS 側のテストインフラがアンバランスです。JS 同様の品質を目指すなら、CSS レイアウトのスナップショットテストが最初の一歩になります。
