# Handoff

This file is the canonical handoff note for this repo.
Update it whenever work switches between Codex, Claude, Gemini, Copilot, or a fresh session after a context reset.

## Handoff Rules

- Read this file, `git status --short`, and the latest `git log --oneline -n 20` before making assumptions.
- Confirm the current branch with `git branch --show-current` before starting work.
- Work in the branch / checkout the user has currently designated; do not assume a separate Claude/Codex split unless the user explicitly asks for one.
- CSS files may be edited when the current task genuinely needs it. Keep the change minimal, call out any non-trivial CSS touch in `change.md`, and prefer review before broadening a CSS-heavy diff.
- Update the `Current Snapshot` section whenever the high-level state changes.
- Append a short entry to `Session Log` whenever meaningful work is handed off or resumed.
- Do not stage unrelated local helper artifacts unless the user explicitly asks for them.
  Local-only artifacts are now actually enforced by `.gitignore`:
  `target/`, `.claude/`, `.repomix/`, `.vscode/`, `repomix-output*.xml`.
  (Until 2026-08-23 this list claimed those paths were untracked while they were in fact tracked.)
- **PR title** — When opening a pull request, always include the agent's name in the title.
  Example: `[Claude] fix CSS calc() precedence` / `[Codex] add image lazy-loading`

## いまの状態（2026-09-04）

- ブランチ `master`。`origin/master` と同期済み。この文書を書いた時点の HEAD は
  `c55a5b5`（`tools/geom/` の移設とこの文書のコミットが直後に乗る）。
- `cargo build --release` 通る。警告は dead_code のみ。
  OneDrive が PDB を掴んで失敗することがある。そのときは `RUSTFLAGS='-C debuginfo=0'`。
- `cargo test --release` → **1126 通過 / 0 落ち**。
  数え方: `cargo test --release 2>&1 | tr -d '\000' | grep -aE "^test result" | awk '{p+=$4; f+=$6} END {print p, f}'`
  （`tr -d '\000'` は必須。出力に NUL が混ざって grep が binary 扱いする）
- html5lib 木構築適合 **1192/1229 (97.0%)**。
  `cargo test --release --bin tobira -- tree_construction_conformance --nocapture`
  が `.dat` ごとの内訳を出す。合計は自分で足す。`TOBIRA_H5_FILE=<名前>` で一本に絞れる。
- 動作確認できとる範囲（`--screenshot` で目視、JS エラーは `TOBIRA_DEBUG_CONSOLE=1`）:
  - **一致に近い**: ja.wikipedia.org、abehiroshi.la.coocan.jp、news.ycombinator.com（投票矢印を除く）
  - **中身は出るが意匠が甘い**: react.dev、vuejs.org、developer.mozilla.org
  - 上記いずれも未捕捉 JS エラー 0。
  - 確認しとらん: 認証の要る頁、フォーム POST、動画、Google/YouTube の実経路
    （`src/browser.rs` に synthetic fallback が残っとる。実物とは別物と思うこと）

### 測り方・見方

```powershell
cargo run -- https://example.com/            # GUI
cargo run --release -- --cli <url>           # テキスト出力
./target/release/tobira --dump-styled <url>  # 箱の一覧（cmd[] は12個で切れる。数を信じるな）
./target/release/tobira --screenshot out.png <url>   # PNG。TOBIRA_SHOT_HEIGHT で高さ
```

主な環境変数: `TOBIRA_DEBUG_CONSOLE`（console と未捕捉エラー）、`TOBIRA_TRACE_STACK`、
`TOBIRA_DUMP_BOXES` / `TOBIRA_DUMP_DEPTH` / `TOBIRA_DUMP_WIDTH`、`TOBIRA_SHOT_HEIGHT`、
`TOBIRA_DEBUG_IMAGES` / `_ATOMIC` / `_FLEX` / `_PAINT` / `_TABLE` / `_CSS`、
`TOBIRA_H5_FILE`、`TOBIRA_INCREMENTAL_RESTYLE`。

Chrome との突き合わせは `tools/geom/`（README 参照）。参照ブラウザは **Chrome ヘッドレス**。
Edge は 2026-08-27 の更新以降 `--dump-dom` が無出力になったので使えん。

**数値だけ見るな。** 表が指定幅を無視する件も `<center>` が表を中央寄せせん件も、
`--screenshot` を足して目で見るまで一つも見つからんかった。修正のたびに一枚撮ること。

## 設計判断とその理由

- **スクリプトを走らせる前にレイアウトを済ませる**（`engine_host.rs:4465 start_with_styles`）
  `getBoundingClientRect` が 0 を返すと、寸法を見て分岐する現代の頁は軒並み死ぬ。
  そこで HTML と stylesheet から先に `layout_geometry` を回し、その矩形と計算済み
  スタイルを host に積んでからスクリプトを起こす。実頁の JS が通るようになった最大の要因。
  代償: スクリプトが DOM をいじった後の再レイアウトは反映されん。今は「初期状態の幾何」だけ。
- **stylesheet は文字列で渡す**（`browser.rs:1530 collect_stylesheet_text`）
  パース済みの `Stylesheet` は `Rc` を抱えとってスレッドを越えられん。JS はワーカー
  スレッドで走るので、テキストのまま渡して向こうで parse し直す。二度手間やが安全。
  `STYLESHEET_MEMO`（`browser.rs:1492`）で取得だけは使い回す。
- **`getComputedStyle` は host に問い合わせる**（`vm.rs` の `computed_style_value` →
  `DomRead::ComputedStyle`、`engine_host.rs:2083`）
  以前は `style` 属性とタグ既定しか見とらんかった。カスケードの結果を返さんと
  「クラスで色を付けて JS で読む」という普通の書き方が全部外れる。
- **font family は名前で引く**（`css.rs:7168 parse_font_family` → `FontFamilyKind::Named(u16)`、
  `font.rs:120 WINDOWS_FAMILY_FILES`）
  総称（serif/sans/mono）に丸めとると Georgia も Verdana も同じ絵になって字幅がずれる。
  インターン済みの id で持ち、`family_is_installed` が実在を確かめてから採用する。
  **字ごとに fallback する**のが肝（`font.rs:668 fonts_for` を `cached_glyph` 側で
  `[named, Sans]` の順に回す）。Georgia に日本語は無いので、これが無いと日本語版
  Wikipedia の見出しが全部豆腐になる。実際なった。
- **インライン箱の矩形は「run 番号 + バイト位置」で印を打つ**
  （`layout.rs:1099 push_marker` → `layout.rs:6474 apply_inline_marks`）
  理由は下の「試してダメやった方法」参照。
- **`<isindex>` は追わん**と決めた。html5lib の残りに数件あるが、現実の頁に無い。
- **省リソースは第二目標**。2026-08-23 に「まず実用ブラウザ」へ方針変更済み。
  メモリのために正しさを落とす判断はもうしとらん。

## 試してダメやった方法

- **インライン印を `LineSpan` として積む** — run の統合が壊れた。span を分けると
  丸めが二回入って行分けが変わり、Wikipedia の重なりが 90 → 120 に増えた。
- **印を run 番号だけで持つ** — 隣り合う span が統合されると番号がずれ、別の要素の
  箱が返る。**最終形**は `(run index, byte offset)` の組で、`push_span`
  （`layout.rs:970`）の中に統合時の付け替えを入れてある。ここは触るなら慎重に。
- **`maybe_auto_close`（`html.rs:2266`）に `"td" | "th" | "tr" if in_bare_table(...)` の
  腕を足して、その中で `maybe_auto_close` を呼ぶ** — 同じタグで無限再帰。
  html5lib が 1184 → 1115 に落ちた。今は `clear_to_table_context`（`html.rs:2257`）を
  既存の腕に畳み込んである。
- **`resolve_table_width` で `.max(preferred_width)` を取る**（`layout.rs:7021`）—
  指定幅が中身の幅に負けて、`width="600"` の表が中身なりに広がっとった。外した。
- **`extract_url` を「値が `)` で終わる」前提で書く** — `background: url(x.svg) no-repeat`
  が丸ごと外れる。さらに `<position>/<size>` のスラッシュ判定を url 判定より先に
  置くと、絶対 URL の `https://…` がスラッシュ持ちなので位置と誤読される。
  インライン `<style>`（相対パス）だけ動いて linked stylesheet が動かん、という
  紛らわしい症状になる。今は `apply_background_shorthand`（`css.rs:7876`）で層ごとに分解。
- **ヒアドキュメント（`python - <<'PYEOF'`）でパッチ script を流す** — バックスラッシュが
  一段食われて `\n` や `\u{...}` が壊れる。**必ず Write でファイルに書いてから実行する。**
  また、`sub()` 失敗で `sys.exit` すると最後の `write` に到達せず、それまでの成功分が
  黙って消える。パッチ script は全部 subst してから一度だけ書くこと。
- **`image` を `use` する** — ローカルモジュール `src/image.rs` と衝突する。
  crate のほうは `::image::` と書く。

## 未確定・仮実装

- **属性の名前空間**は実体として持っとらん。`xlink:href` などは正規化した名前で
  照合しとるだけ。大抵の頁は通るが、`getAttributeNS` の厳密な挙動とは違う。
- **`transform`** は translate をレイアウト時、scale/rotate を描画時に効かせとる。
  絵は合うようになった（`layout.rs:3328 transformed_layer_bounds`）が、
  `scale()` と `translate()` を並べると 15px ずれる。設計として割れとる。
- **`transform: scale` した箱の hitbox が 0**。描画は直したが測る側が変換前を見とる。
- **`overflow: auto` / `scroll` は `Hidden` とほぼ同じ扱い**。巻物は無い。
  さらに親より広い子は押し込まれる（あふれん）。MDN の崩れはこれ。
- **巻物の幅**: Chrome は版面から 16px 引く。tobira は 10px の巻物を上に重ねて
  全幅で組む。`vw` / `vh` / `position: fixed` が Chrome と 16px 違う原因。
  どちらに寄せるか未決。
- **`document.fonts`、`Element.animate`、`navigator.clipboard`、`Intl.Segmenter`、
  `CSSStyleSheet` / `adoptedStyleSheets`、Blob / File / FileReader、DOMParser** は
  `engine_host.rs:2845 RUNTIME_PRELUDE` の JS 実装。形だけ合わせた張りぼてで、
  実際には何も起きんものが多い（`animate` は最終状態を即座に適用、
  `URL.createObjectURL` は `data:` URL を返す）。存在チェックを通すためのもの。
- **`checkVisibility`** は prelude 版と native 版（`DomNodeCheckVisibility`）が両方ある。
  native が勝つ。prelude 側は消し忘れ。
- **CSS の遷移とアニメーション**は丸ごと無い。最終状態が即座に出る。
- **表のセル背景を二度塗っとる**。半透明を重ねると濃くなる。
- **差分 restyle** は既定 ON（`TOBIRA_INCREMENTAL_RESTYLE`）。
  `docs/JS_ROADMAP.md` の Phase5 に「blocker」と書いてあるのは古い記述。

## 次の一手（優先順）

1. **`overflow` があふれるようにする** — `src/layout.rs`。
   `Overflow::Auto` / `Scroll` が今 `Hidden` と同じ道を通っとる（`layout.rs:2700`、
   `layout.rs:3703` の `element.style.overflow == Overflow::Hidden` の判定周り）。
   まず「子の幅を親に丸めるのをやめる」だけで MDN の崩れは改善するはず。
   巻物 UI まではいらん。クリップだけ正しくして、はみ出しを許す。
2. **インライン箱の高さが 1px 高い** — ここが一番割に合う。
   `cmp.py g4.html` の落ち 9 件のうち **7 件は高さが `x17` であるべきところ
   `x18` になっとるだけ**（`s1` `s2` `w1` `w2` `long` `e1`、それと空 div の
   `800x0` が `800x1`）。原因は行の芯の丸め — `font.rs:474 line_height_px` が
   face の `new_line_size` をそのまま切り上げとる。Chrome は ascent + descent を
   別々に丸めて足す。ここを合わせるだけで g4 は 5/14 → 11〜12/14 になるはず。
   `layout.rs:6946 below_baseline` と対で見ること。
3. **インライン箱の矩形の残り（g2 / g4）** — 1px を直した後に残るのはこれ。
   `layout.rs:6474 apply_inline_marks` が `inline_rects` を作り、
   `layout_styled_document`（`layout.rs:564`）の末尾で親へ union しながら
   `ElementHitbox` に流す。残るのは
   (a) 空白だけの div の中の空 `<span>` が y=0 に落ちる（`e2`: Chrome は `0,18`）
   (b) 行をまたぐ `<span>`（Chrome は複数矩形、こっちは union 一個）
   (c) `<a>` の x が 3px ずれる（`lnk`: chrome `63,36` / tobira `60,36`）——
   直前の `<i>` の後の空白の幅。
4. **`transform` の hitbox** — `layout.rs:3328 transformed_layer_bounds` が層の
   大きさは出しとるので、同じ値を `record_container_box`（`layout.rs:9005`）に
   渡して hitbox にも反映させる。scale と translate の適用段を揃えるのは
   その後（設計判断が要る。translate も描画時に寄せるほうが筋がええ）。
5. **HN の投票矢印** — 未解明。`triangle.svg` の取得は成功しとる
   （`TOBIRA_DEBUG_IMAGES=1` で "style image ok"）。同じ CSS と入れ子を
   合成頁で作ると出る。次は `TOBIRA_DEBUG_PAINT=1` で描画命令が生成されて
   おるかどうかから切り分ける。命令が無いならレイアウト、あるなら描画。
6. **CSS transition / animation** — 一番でかい未実装。`@keyframes` のパース、
   時間軸、再描画の駆動が要る。着手するなら独立した回を丸ごと使うこと。
7. **html5lib 残り 37 件** — 大半は adoption agency の深いところ。
   `TOBIRA_H5_FILE=adoption01.dat` から。費用対効果は 1〜6 より低い。

## 主なモジュール

| ファイル | 中身 |
|---|---|
| `src/browser.rs` | 頁の読み込み、synthetic fallback（YouTube/Google/frameset）、stylesheet 収集 |
| `src/html.rs` | 手書き HTML パーサ。末尾に `mod html5lib_conformance` |
| `src/css.rs` | パーサ、セレクタ照合、`ComputedStyle`、`@media`、`calc()`、色 |
| `src/layout.rs` | レイアウト全部。約 14,000 行。テキスト整形・表・flex・grid・描画命令 |
| `src/font.rs` | face の読み込みとグリフ。名前付き family、字ごとの fallback |
| `src/gui.rs` | 窓、アドレス欄、当たり判定、`paint_layout` |
| `src/main.rs` | CLI。`--cli` / `--dump-styled` / `--screenshot` |
| `src/engine/` | 自作 JS エンジン（コンパイラ + VM + GC）。boa は parser front-end のみ |
| `src/engine_host.rs` | DOM ↔ JS の橋。`RUNTIME_PRELUDE`、`start_with_styles` |
| `src/js.rs` | スクリプト実行の入り口とポリシー |
| `tools/geom/` | Chrome 突き合わせ用の合成頁と `cmp.py` |

## 作業の流儀

- `git add -A` は使わん。`git add -u` か、パスを明示。
- commit message は `git commit -F <tempfile>`（日本語が壊れるため）。
- 性能を測るときは必ず `--release`。デバッグビルドの数字は意味が無い。
- PowerShell script に日本語パスを直書きせん。`[Environment]::GetFolderPath('MyDocuments')`。
- backup の robocopy は `/E`。**`/MIR` は使わん**（消える）。
- 引いた Web 標準は `Z:\vscode\tobira-specs\` に md で残す。実装前に `INDEX.md` を見る。
- 履歴を失ったら、まず `Z:\vscode\` のアーカイブを見る（2026-08-07 の事故はそこから復旧した）。

## よく使うコマンド

```powershell
cargo run
cargo run --release -- --cli https://news.ycombinator.com/
cargo test --release
git log --oneline -n 20

# Chrome 突き合わせ
python -m http.server 8731 --directory tools/geom
python tools/geom/cmp.py g4.html

# AI branch merge loop（5分ごとに codex/* と claude/* をテストが通れば merge）
.\scripts\merge-loop.ps1 -IntervalSeconds 300
.\scripts\merge-loop.ps1 -Once -DryRun
```

## Session Log

### 2026-09-04 - Claude (Chrome parity campaign: 2d2dfbc..c55a5b5, 34 commits)

上の各節（いまの状態 / 設計判断 / 試してダメやった方法 / 未確定 / 次の一手）が
この回の成果物やと思ってええ。ここには経緯だけ残す。

- 出発点は「html5lib の残り」と「未実装 API」を潰す自律ループ。途中から
  Chrome と合成頁で幾何を突き合わせる方式に切り替えた（`tools/geom/`）。
- 数値: html5lib 1162 → 1192/1229、テスト 1097 → 1126 通過（落ち 0）。
- 途中でユーザーに「見た目とか確認しながら進めてる？」と指摘された。
  そのとおり数値しか見とらんかった。`--screenshot` を足して目で見るようにしたら
  即座に二つ（表の指定幅、`<center>` と表）出てきた。**この指摘は効いた。**
- 最後に入れた「名前付き font family」が日本語 Wikipedia の見出しを豆腐にする
  退化を生んで、字ごとの fallback で直した（`c55a5b5`）。
  名前で face を引く変更を入れるときは、必ず CJK の頁を一枚撮ること。
- `tools/geom/` はこの回まで session 限りの一時ディレクトリにあった。
  次のセッションで消えるのでリポジトリへ移した。

### 2026-08-23 - Claude (repo tidy; Web Storage and document.cookie actually wired)

Started as a cleanup pass over the repo root, but the compiler warnings led to
two real gaps.

- **Repo root** — 12 root markdown files down to 4. Roadmaps and design notes
  moved to `docs/`, external reviews to `docs/reviews/`, index in `docs/README.md`.
  Five throwaway scripts from the 2026-05-16 `codex/codex` merge (`fix_layout.py`,
  `resolve_gui_regex.py`, `dump_gui_marker.py`, `dump_pull_conflicts.py`,
  `pull_conflicts.md`) deleted — they had that merge's conflict text hardcoded.
- **`.gitignore` now matches what this file claimed.** The Handoff Rules said
  `.claude/`, `.repomix/`, `copilot.md`, `gemini.md` and
  `repomix-output.xmlbrowser.xml` were untracked; all five were in fact tracked
  and `.gitignore` only listed `/target` and `scroll_debug.txt`. Generated
  artifacts are untracked and ignored now (-12,570 lines).
- **`Host::storage` was a stub.** `BrowserHost::storage()` returned
  `StorageResult::None` for every op, so `localStorage`/`sessionStorage` silently
  dropped writes and read back null — while `site_state.rs` held a complete,
  working per-origin store that nothing called. `demo/storage-demo.html` exercised
  all of it and did nothing. Implemented the bridge; `storage_keys()` added to
  `site_state` for `key(n)`/`length` ordering.
- **`document.cookie` returned `""`.** Hardcoded in `vm.rs`, with no setter at
  all, even though the HTTP layer already fed `Set-Cookie` into `site_state`'s
  jar and sent `Cookie` headers from it. `StorageAreaKind::Cookie` already
  existed in the host protocol and was unused — the getter and setter now route
  through it, so cookies set over the wire are visible to JS and vice versa.
  HttpOnly cookies stay hidden from script, per spec.
- **`TextDecoder` label bug** — `if ... { "utf-8" } else { "utf-8" }`, so
  `new TextDecoder('shift-jis')` reported `encoding === 'utf-8'` instead of
  throwing. Now accepts the WHATWG utf-8 labels and throws `RangeError` otherwise.
- Dead scaffolding removed: a `while depth <= 10 { break; }` ResizeObserver loop
  (clippy `never_loop`, a deny-level lint) plus the `EventLoop` field that existed
  only for it; a `let _ = first_line; // suppress unused warning` in
  `layout_nowrap_fragments` covering a variable that was always true.

Verification: `cargo test` 718 passed / 0 failed. Compiler warnings 18 -> 10.
The remaining ones are `dead_code` and still need case-by-case judgement — the
storage half of the earlier audit turned out to be a missing wire, not dead code,
so the rest deserve the same scrutiny before deleting anything.

### 2026-07-26 - Claude PM / Codex (legacy align/valign semantics)

- Remaining abehiroshi diffs vs Chrome, both root-caused in css.rs:
  1. `apply_legacy_attributes` mapped the `align` attribute of ANY element to
     `style.text_align` (inheritable), so `<table align="center">` centered every piece of
     text inside the table. Per HTML, table `align` only positions the table box (layout
     already handles that via `table_x`). Now skipped for `table`; div/p/h*/td/tr keep the
     mapping.
  2. `vertical_align` defaulted to `Top` for all elements; the HTML default for table
     cells is middle. menu.htm's `<td>bullet</td><td><p><a>link</a></p></td>` rows looked
     two-lined because the `<p>` top margin pushed the link down while the bullet stayed
     top-aligned. td/th now default to `VerticalAlign::Middle` (valign attr / CSS still
     override; the layout-side application at cell placement already existed).
- Tests: table-align box-centered-text-left, div/td align regressions, td default middle
  offset, valign=top override. `cargo test` 716 green.

### 2026-07-24 - Claude PM / Codex (table column shrink — no more horizontal overflow)

- Follow-up to the table-cell inline-flow fix: the abehiroshi page still overflowed
  horizontally (long bold text ran past the viewport, clipped at the right edge).
  Root cause: column sizing had an expand path only. `compute_column_widths` hands each
  column its max-content width, and when the sum exceeded the available width the
  `saturating_sub` fed 0 to `expand_column_widths` — no shrink pass existed, and the table
  width was then recomputed FROM the oversized columns.
- Fix (Codex, spec by Claude): added a shrink pass mirroring browser auto table layout.
  `TableColumnSizing` now carries per-column `mins` (unbreakable floor: loaded image draw
  width, form controls, nested-table min sum; text floors at ~one char since
  `push_wrapped_word` breaks long words char-by-char). `shrink_column_widths` distributes
  the overflow proportionally to each column's (width - min), unlocked columns first, then
  width-attr-locked columns; never below the floor. Cells re-wrap naturally at the
  narrower widths.
- Tests: long CJK cell text stays within a 400px container and wraps to multiple lines;
  a 300px image column keeps its floor while the text column absorbs the shrink.
  `cargo test` 709 green.

### 2026-07-24 - Claude PM / Codex (table-cell inline flow — 3x line spacing fix)

- User report: the abehiroshi frameset page rendered with ~3x vertical line spacing and
  inline runs split across lines (aspect totally off vs Chrome). Probe (temporary layout
  test) quantified it: `<br>`-separated lines are 23px apart in a `<div>` but 69px apart in
  a `<td>`, and `Left<strong>:</strong>` in a td put ":" on its own line.
- Root cause: `layout_table_cell()` laid out each cell child individually via
  `layout_node`, so every text run / inline element became its own paragraph-like block
  (with margins), instead of grouping consecutive inline children into one inline flow the
  way `layout_mixed_children()` does for divs.
- Fix (Codex, spec by Claude): `layout_table_cell` now delegates to
  `layout_mixed_children`. Also: `measure_cell_preferred_width` sums consecutive inline
  children (with `<br>` as line separator) instead of max-per-child, and the inline text
  whitespace model was tightened — `pending_space` now follows actual leading/trailing
  whitespace of text fragments, so `Left<strong>:</strong>` no longer gains a phantom space
  (this improves inline text joins globally, not just tables).
- Tests: td `<br>` line gaps == div line gaps; inline children share a line in a td;
  block children (nested table/div) still stack. `cargo test` 707 green.
- Note: a parallel session landed `0ad0f34` (mailto:/tel:/javascript: hrefs skip relative
  resolution via `has_url_scheme()`), which bumped the suite past 703 before this fix.

### 2026-07-24 - Claude PM / Codex (frameset loss — engine VOID_ELEMENTS + stray end tags)

- User report: abehiroshi.la.coocan.jp (a `<frameset cols=18,82>` page) rendered only the
  left menu frame; the right content frame vanished. Probe showed `expand_frameset` received
  a frameset with just ONE frame child.
- Two stacked defects, both fixed:
  1. The engine-side `VOID_ELEMENTS` list (engine_host.rs) was missing `"frame"` (html.rs's
     `is_void_element` has it). The load path round-trips HTML through the engine DOM
     (`serialize_node`), so `<frame>` re-serialized as `<frame ...></frame>`.
  2. html.rs `close_element` unwound the ENTIRE open stack when an end tag had no matching
     open element. On re-parse, the void `<frame>`'s stray `</frame>` closed `frameset`+`html`,
     dropping the second frame out of the frameset. Per HTML5, unmatched end tags are now
     ignored (pre-scan the stack; return if no match). This also protects legacy pages with
     stray `</td>` / `</font>` etc. from tree destruction.
- Tests: frameset with `</frame>` close tags keeps 2 sibling frames; stray `</b>` is ignored;
  engine snapshot serializes frame as void (no `</frame>`, `<frame ` x2). `cargo test` 703
  green. Real site verified: both frames render (menu + profile content).
- Flagged separately (task chip): `annotate_resource_urls` resolves `mailto:` hrefs against
  the base URL, producing `https://host/mailto:...` — scheme-qualified hrefs should skip
  relative resolution.

### 2026-07-24 - Claude PM / Codex (inline image rendering — InlineFragment::Image)

- User report: abehiroshi.la.coocan.jp/nonno/nonno.htm showed "[image]" text links where
  Chrome shows magazine covers. Diagnosis: fetch/decode/ImageStore were all fine (verified
  with a temporary probe — every cover JPEG decoded); the gap was purely in layout.
  `collect_inline_fragments()` unconditionally replaced inline-context `<img>` with its
  alt text / "[image]" — `InlineFragment` had no image variant, so any `<a><img></a>` or
  text-mixed image (this page is `<td><a><img></a><br>No.N</td>`) never drew. Only the
  block path (`layout_image_element`) could draw images.
- Fix (Codex, spec by Claude): new `InlineFragment::Image` + `InlineImageSpec` +
  `LineBuilder::push_image()`, modeled on the existing `Control` inline-box pattern.
  Store-hit images become sized fragments (`image_dimensions` against available width);
  store-miss keeps the alt/"[image]" fallback. All three white-space paths handle images
  (normal wraps like a word); emit is bottom-aligned in the line box, honors
  opacity/filter via LayerCommand, and emits a LinkCommand hitbox for `<a>`-wrapped
  images (gated on pointer-events). `ImageStore` + available width now thread through
  `flatten/collect_inline_fragments`.
- Tests: linked inline image emits ImageCommand + link hitbox and no "[image]" text;
  store-miss falls back to alt; image raises the line advance. `cargo test` 700 green.

### 2026-07-24 - Claude PM / Codex (HTML tokenizer char-boundary panic + non-HTML Content-Type)

- Fixed a crash reported from a GUI session: navigating to a JPEG URL panicked at
  `src/html.rs:219` ("byte index is not a char boundary"). Root cause: in `tokenize()`,
  when `<` is followed by a non-tag-name character, the recovery path advanced `index += 1`
  (one byte), landing inside a multi-byte UTF-8 char (U+FFFD from lossy-decoding binary).
  Fix: advance by the char's `len_utf8()`. Regression tests with `\u{FFFD}` and a
  JPEG-like lossy byte string.
- Added Content-Type awareness to the document load path (`browser.rs`):
  new `synthesize_non_html_document()` — `image/*` responses become a synthetic
  `<html><body><img src="{final_url}"></body></html>` viewer document (like real browsers),
  `text/plain` gets HTML-escaped and wrapped in `<pre>`. `text/html` / missing
  content-type unchanged. Unit tests for both paths + escaping.
- Verified: `cargo test` 697 green; `--cli https://httpbin.org/image/jpeg` no longer
  panics (renders the img document); rollupjs.org still CLEAN.

### 2026-07-23 - Claude PM / Codex (module top-level scope isolation — rollupjs.org CLEAN)

- Root-caused the rollupjs.org `object is not callable (kind Array, ["items"])` crash:
  module top-level `let`/`const`/`var`/`function` bindings were compiled as **shared flat
  globals keyed by name** (`resolve_declaration_binding` returned `Global` at top level even
  for modules), and closure references compiled to call-time `GetGlobal(name)`. With minified
  multi-chunk bundles, a later-executed module's same-named top-level binding clobbers the
  first module's value. Exact real-site chain: VPAlgoliaSearchBox chunk declares
  `var $i=["items"]` → overwrites framework's `$i` (Vue `withCtx`) → framework's exported
  `L = $u = e => $i` returns the array → theme's `vs=_o(); vs(renderFn)` throws.
  (The source-position backtrace `at <script> (2:40706)` from 33ab431 is what made this findable.)
- Minimal repro: module A `const inner=()=>"real"; export const outer=()=>inner;`, module B
  imports `outer`, declares its own `const inner=["items"]`, and `outer()` returned B's array.
- Fix (Codex, spec by Claude): when compiling a module (`module_context.is_some()`), top-level
  declarations become **frame locals** (`Rc<RefCell>` cells) like function bodies, so closures
  capture cells via the existing upvalue machinery instead of falling through to `GetGlobal`.
  Import live-bindings (per-use namespace `GetProp`) and export emission (`resolve_binding` →
  `SetProp`) work unchanged. Script (non-module) behavior untouched (window sharing).
  - `compiler/mod.rs`: `is_module_top_level()` helper.
  - `compiler/scope.rs`: Var/Let/Const storage arms gated with it.
  - `compiler/statements.rs`: function-decl hoist + switch-case hoist + block-nested `var`
    collection enabled at module top level; `predeclare_hoisted` extended to cover
    destructuring pattern names, class declarations, and export-wrapped declarations.
  - `compiler/patterns.rs`: `collect_binding_names` / `collect_pattern_names` helpers.
- New `tests/module_scope_isolation.rs` (6 tests): cross-module name collision, mutual
  recursion, forward reference, destructuring capture, block-`var` capture, and
  no-globalThis-leak. `cargo test` 691 green; `TOBIRA_VERIFY_BYTECODE=1` green.
- **rollupjs.org now renders CLEAN end-to-end** (nav, hero, feature cards, footer; no
  uncaught errors). This closes the module-scope arc that source-position backtraces opened.
- Note for later: `load_module_graph` eagerly executes dynamically-imported chunks (they land
  in `post_order` before their importer) and each `<script type="module">` tag builds its own
  registry (a shared dep imported by two tags would re-execute and its namespace object be
  recreated). Both are latent correctness leads, not urgent.

### 2026-06-19 - Claude PM / Codex (compiler split + GC evidence)

- Split the 4423-line `src/engine/compiler.rs` monolith into focused submodules under `src/engine/compiler/` (no logic change, 571 tests green at every step, each extraction its own commit): `mod.rs` 392 (core: structs/new/finish/emit/function compilation), `scope.rs` 267 (ScopeFrame/UpvalueState/OuterBindings + binding resolution), `modules.rs` 327 (import/export), `classes.rs` 465 (class/super), `patterns.rs` 363 (destructuring), `statements.rs` 1536 (control flow), `expressions.rs` 1124. Submodules are children of `compiler`, so they reach FunctionCompiler's private fields; moved methods are `pub(super)`.
- Added GC evidence (read-only, no collector landed): `Vm::heap()` accessor + `tests/gc_heap_growth.rs`. Measured — fresh VM ≈ 380 builtin objects; a 2000-iteration loop of unreachable `{…}`+`[…]` literals leaves ≈ 4380 live (grew ≈ 4000 = 2/iter, zero reclaimed). Confirms the heap is monotonic within a run (no in-session collection). A reclaiming mark-sweep collector is intentionally deferred — it needs review because the `callables` side-table and closure-upvalue `Rc<RefCell>` cells are roots outside the arena. When it lands, `heap_grows_without_in_session_collection` flips to assert reclamation.
- Context: these two items are debt paydown surfaced by an external code critique (compiler monolith + no in-session GC were its only landed points; its boa-GC and "200 tests" claims were stale).
- Real-page campaign: implemented the `URL` global (hand-rolled parser, no new dep) — nodejs.org and tailwindcss.com stopped on `URL is not defined`; both now advance (nodejs.org's next wall is `getAttribute is not a function`). New `tests/url_global.rs`.
- Call-stack correctness (`tests/call_stack_depth.rs`): StackOverflow is now a catchable `RangeError` ("Maximum call stack size exceeded", the web-standard message) instead of a fatal abort — `try { recurse() } catch {}` works; frame cap raised 1024 -> 10_000 (was far below real engines). This did NOT fix vitejs.dev: its bundle hits a genuine *uncaught* infinite recursion, a separate semantic gap.
- More campaign wins (each its own commit + test, all green): `crypto.getRandomValues`/`randomUUID` (was an empty stub); `typeof` of window-globals now reports the real type (`typeof crypto` was "undefined" — `GetGlobalOptional` skipped the window-global fallback, breaking `typeof crypto !== 'undefined'` feature-detection); `matchMedia()` returns a MediaQueryList with (no-op) `addEventListener`/`media` (Bootstrap color-modes); `document.currentScript` gained `getAttribute`/`hasAttribute` (fathom/beacon/framework `currentScript.getAttribute('data-…')`). Result: **expressjs.com is now CLEAN**; nodejs.org, tailwindcss.com, getbootstrap.com all advanced past their first walls. 587 tests green.
- Also added `Object.prototype.propertyIsEnumerable` (httpbin/swagger-ui). 588 tests green.
- **Top next target = dynamic `import()`**: `Unimplemented("import() calls")` is the single most common remaining wall — crates.io, svelte.dev, rollupjs.org, webpack.js.org (4+ sites). A mid-size ESM feature (async load + instantiation) with scope choices, best done interactively on top of the existing module graph (load_module_graph / ModuleContext). Other deeper targets: getbootstrap `bootstrap is not defined`; vitejs.dev genuine infinite recursion; webpack/next `modules[id].call` (react.dev, typescriptlang); babeljs.io/pnpm.io parse error on a "https:" script (looks like fetched content isn't JS — a networking/CDN issue, not a parser gap). CLEAN: example, HN, Wikipedia, web.dev, vuejs, rust-lang, docs.rs, lodash, expressjs, prettier, tc39.es.
- `document.currentScript` follow-up: the stub still lacks the real element's `data-*` attributes (getAttribute returns null for them); a full fix returns the actual DOM script node via host integration.
- Proactive standard-library audit (probe of ~120 prototype methods + ~80 globals): added the cleanly-missing ones — Math hyperbolics/expm1/fround, Number.isSafeInteger, Object.getOwnPropertySymbols, Date.UTC/parse/getTimezoneOffset, Object.prototype.propertyIsEnumerable, TextEncoder/TextDecoder, escape/unescape, WeakRef, Headers, FormData. Remaining gaps are large/design-gated or N/A (Request/Response/Blob/File/FileReader, WebSocket/Worker, BigInt, Intl, DOMParser, Audio/Notification). **597 tests green.** Note: each added builtin grows the fresh-VM object count, tracked by tests/gc_heap_growth.rs (fresh-VM bound now 600).

### 2026-06-19 - Claude PM / Codex (real-page campaign: rust-lang, react.dev + doc refresh)

- Fixed rust-lang.org crash: object literal `{__proto__: value}` now sets `[[Prototype]]` (Annex B.3.1) instead of creating an own `__proto__` property. Root cause was highlight.js's `Object.freeze({__proto__:null,...})` + `for...in` enumerating the bogus own property (`typeof null === "object"`) → `Object.getOwnPropertyNames(null)` threw. New opcode `SetObjectLiteralProto`; primitives ignored (no throw). Regression tests in `tests/proto_literal.rs`. (`bac4893`, `af64f1b`)
- Fixed react.dev regex abort: swapped the JS regex backend from the Rust `regex` crate to `regress` (JS-compatible: look-ahead/-behind/backreferences). New `src/engine/js_regex.rs` adapter keeps vm.rs call sites mostly unchanged; `translate_regex_named_groups` dropped (regress supports `(?<name>)` natively); `regex` dependency removed. Regression tests added to `tests/regexp_coverage.rs`. react.dev's next wall is webpack-internal `modules[id].call` (deeper; content already renders). (`04bfc2f`)
- Refreshed stale docs: README/HANDOFF said `boa_engine` (removed 2026-06-12) and `200` tests (now `571`); clarified the north-star scope (render YouTube's page DOM/CSS ≠ play its video on the CPU `softbuffer` renderer). These stale lines had invited an off-base external critique.
- Verified: `cargo test` `571` passing; release build green.

### 2026-05-25 - Codex (shadow DOM / composed path)

- Added `customElements` lifecycle scaffolding plus `attachShadow(...)` support, slot assignment helpers, and `ShadowRoot` / `slot` accessors.
- Implemented shadow-boundary event retargeting and `Event.composedPath()` for composed events so WebComponents listeners see browser-like targets.
- Added regression coverage for custom element upgrade callbacks, attribute change callbacks, and shadow DOM host / slot behavior.
- Verified the updated state with `cargo test` (`200` passing tests) and `cargo build`.

### 2026-05-18 - Codex (Node / fragment DOM APIs)

- Added browser-grade Node accessors to the JS DOM bridge, including `nodeType`, `nodeName`, `nodeValue`, sibling accessors, and `isConnected` on document and element nodes.
- Added structural mutation helpers: `cloneNode(...)`, `replaceChild(...)`, `removeChild(...)`, `append(...)`, `prepend(...)`, `before(...)`, `after(...)`, `replaceWith(...)`, and `replaceChildren(...)`.
- Added `document.createDocumentFragment(...)` and fragment flattening during insertion so DOM batches behave more like a real browser.
- Verified the updated state with `cargo test` (`188` passing tests) and `cargo build`.

### 2026-05-19 - Codex (event loop / timer queue)

- Replaced the immediate timer / animation / microtask fallback path with queued host-task plumbing so callbacks do not reenter the current JS turn immediately.
- Added queued support for `queueMicrotask(...)`, `setTimeout(...)`, `setInterval(...)`, and `requestAnimationFrame(...)`, plus `clearTimeout(...)`, `clearInterval(...)`, and `cancelAnimationFrame(...)` handle cleanup.
- Added a regression test that confirms nested timeouts defer to the next turn instead of recursively firing in the same turn.
- Updated the README and JS roadmap so the documented JS runtime status matches the queued task behavior.
- Verified the updated state with `cargo test` (`193` passing tests) and `cargo build`.

### 2026-05-19 - Codex (characterData / splitText)

- Added browser-like `CharacterData` support for text nodes, including `data`, `length`, `nodeValue`, and `splitText(...)`.
- Updated `textContent` / `nodeValue` setters so text-node edits now emit `characterData` mutation records instead of only child-list churn.
- Added a regression test that confirms `MutationObserver` receives `characterData` changes and that `splitText(...)` preserves text-node sibling relationships.
- Updated the README and roadmap notes to reflect the deeper text-node DOM surface.
- Verified the updated state with `cargo test` (`193` passing tests) and `cargo build`.

### 2026-05-24 - Codex (async UI / background render)

- Moved page navigation into a background worker so the title bar and address bar remain responsive while page loading is in flight.
- Added a separate background render worker that produces content frames off the UI thread, then hands completed frames back through user events.
- Removed any loading-screen style UI; the chrome stays interactive and the content area updates when the async work completes.
- Verified the updated state with `cargo test` (`196` passing tests) and `cargo build`.

### 2026-05-24 - Codex (policy update)

- Relaxed the CSS-editing guardrail because the user explicitly said CSS may be touched when needed.
- Dropped the Claude/Codex branch-split assumption from the shared handoff rules so future work can follow the current shared branch/worktree the user designates.

### 2026-05-14 - Codex

- Inspected the repo after user said Claude had advanced implementation during a context gap.
- Confirmed the repo has moved to the `tobira` name and the current branch head is `91cc671`.
- Confirmed `cargo test` is green with `74` passing tests.
- Added this handoff file and linked it from `README.md`.
- Established the rule that this file should be updated on every handoff / resume.

### 2026-05-14 - Codex (DOM / JS pass)

- Reworked `src/js.rs` so script execution runs against a lightweight mutable DOM instead of mostly fake stubs.
- Added DOM-backed support for selectors, element creation, child insertion/removal, `innerHTML`, `textContent`, `classList`, and ID/class mutation.
- Changed `document.write(...)` handling to mutate the DOM and recursively execute script tags written by scripts.
- Fixed a parsing correctness bug by teaching `src/html.rs` to keep raw-text contents for `script`, `style`, `title`, and `textarea`.
- Verified the current state with `cargo test` (`77` passing tests) and `cargo build`.

### 2026-05-14 - Codex (DOM demo follow-up)

- Added `demo/dom-demo.html` and `demo/dom-demo.js` to exercise the new DOM-backed JS path locally.
- Updated `README.md` so the documented JS scope matches the current implementation better and includes the new DOM demo command.

### 2026-05-14 - Codex (clipboard fix)

- Added address-bar clipboard support backed by the OS clipboard via `arboard`.
- `Ctrl+C`, `Ctrl+X`, and `Ctrl+V` now work against the current address-bar selection / insertion point.
- Added focused tests for selected-text and cut-selection behavior in `src/gui.rs`.

### 2026-05-15 - Codex (parallel branch workflow)

- Confirmed the current Codex branch is `codex/codex`.
- Recorded the new workflow: Codex and Claude may implement in parallel on separate branches, with merge reconciliation handled later through GitHub Copilot / the user's preferred merge flow.
- Future handoffs should always note the active branch before assuming current repo state.

### 2026-05-15 - Codex (JS runtime foundation pass)

- Moved `process_document_scripts` onto a dedicated larger-stack worker thread to reduce the chance of crashing on large bundles.
- Raised script execution budgets and removed the old pattern-based prefilter that used to skip `fetch` / `XMLHttpRequest` scripts outright.
- Added Promise job draining after top-level eval, Promise-backed `fetch`, and a minimal `XMLHttpRequest` object.
- Added JS navigation propagation so `location.href` changes can trigger a follow-up page load during initial script processing.
- Added DOM property reflection and `document.createTextNode()` support to improve dynamic script insertion and general DOM compatibility.

### 2026-05-15 - Copilot (merge-loop setup)

- Added `JS_ROADMAP.md` as the living plan for taking JavaScript support from lightweight and useful to browser-grade.
- Linked the roadmap from `README.md` so future sessions can find the priority order quickly.
- Created `scripts/merge-loop.ps1` — a PowerShell loop that runs every N seconds, finds unmerged `codex/*` + `claude/*` branches, runs `cargo test`, and merges passing ones into master.
  - Usage: `.\scripts\merge-loop.ps1 -IntervalSeconds 300` (default 5 min)
  - Flags: `-Once` (single cycle), `-DryRun` (no actual commit/push)
- Created `.github/workflows/ai-branch-merge-loop.yml` — GitHub Actions version that triggers on push to AI branches and on a 10-minute cron schedule.

### 2026-05-16 - Codex (event plumbing demo)

- Added a dedicated `demo/events-demo.html` / `demo/events-demo.js` page for verifying native page event plumbing.
- Updated the docs to reflect that bubbling DOM event dispatch covers `click`, `input`, `change`, `submit`, `keydown`, and `keyup`, while `focus` and `blur` remain target-only.
- Kept the roadmap and handoff notes in sync with the remaining capture-phase, richer listener option, and live-value reflection gaps.

### 2026-05-16 - Codex (keyboard event plumbing)

- Added page keyboard event dispatch for focused inputs so scripts can observe `keydown` and `keyup` before browser default actions run.
- Included keyboard metadata in the event payload (`key`, `code`, modifier flags, and `repeat`) and added demo logging for manual inspection.
- The next event-system gap is richer listener options and capture-phase dispatch, not basic key delivery.

### 2026-05-16 - Codex (keyboard roadmap step)

- Tightened the GUI event loop so focused page inputs receive `keydown` before default handling and `keyup` after the edit path finishes.
- Added a regression test that checks keyboard event metadata reaches JS listeners on the document.
- Updated the living roadmap and demo copy to treat keyboard delivery as a completed milestone and the next phase as richer listener options / capture phase.

### 2026-05-16 - Codex (viewport, focus, and scroll sync)

- Wired GUI viewport size changes into the JS runtime so `window.innerWidth` / `window.innerHeight` stay current and `resize` listeners fire on actual browser resizes.
- Added JS-visible focus state through `document.activeElement` and `document.hasFocus()`-style behavior for the currently focused page control.
- Exposed `window.scrollY`, `window.pageYOffset`, and `scrollTop`-style DOM accessors, plus `scroll` events when the user scrolls the GUI.
- Added regression coverage for viewport resize, focus / blur, and scroll event handling.

### 2026-05-16 - Codex (branch switch after merge)

- Moved Codex work from `codex/codex` to a fresh branch, `codex/js-event-capture`, so the next JS/event slice can continue cleanly after the previous merge.
- Keep future Codex implementation work on this branch unless the user explicitly asks to switch again.

### 2026-05-16 - Codex (layout reflow cache)

- Added a lightweight layout cache keyed by viewport width and page revision.
- Invalidated cached layout when JS-driven DOM snapshots change the page content.
- Updated the README, roadmap, and handoff notes to reflect the incremental reflow work.

### 2026-05-16 - Codex (inline style bridge)

- Added a native `element.style` bridge that reflects inline CSS through `cssText`, `setProperty(...)`, `getPropertyValue(...)`, and common style accessors.
- Added a regression test that checks inline style mutations serialize back into the DOM snapshot.

### 2026-05-16 - Codex (style property matrix expansion)

- Expanded the inline style bridge to cover more text, size, and border-related properties that the current layout engine already understands.
- Added regression coverage for the expanded style accessors and the browser-facing serialization path.

### 2026-05-16 - Codex (CSS boundary clarification)

- Confirmed on the Claude `claude/phase5-css` branch that the broad CSS parser/layout foundation should be treated as complete for this repo.
- Reframed the remaining CSS work for Codex as Phase 6 visual effects / advanced rendering and JS-driven reflow integration, not parser/layout duplication.

### 2026-05-16 - Codex (capture listener groundwork)

- Added capture-phase dispatch and `once` listener support to the DOM event bridge for ordinary page controls.
- Added regression tests for capture order, once-listener removal, and capture-sensitive `removeEventListener(...)`.
- Updated the roadmap, README, and event demo copy so the next session starts from the current event semantics instead of the pre-capture baseline.

### 2026-05-16 - Codex (live input sync)

- Removed the stale page-control value cache so rendered inputs now trust the DOM-backed `value` as the source of truth when they are not focused.
- Kept focused native editors authoritative during typing, while syncing their live text back into the DOM attribute on each edit path.
- Added a small regression test to lock in the focused-editor-vs-DOM value precedence.

### 2026-05-16 - Codex (merge prep checkpoint)

- Current branch `codex/js-event-capture` is clean and pushed with the latest live input sync work.
- PR #40 is the active merge target for the current JS/event progress checkpoint.
- The next likely follow-up after merge is storage/cookies and richer history/back-forward behavior.

### 2026-05-16 - Gemini (branch merge)

- Merged `codex/js-event-capture` into master, resolving conflicts in HANDOFF.md, README.md, and src/browser.rs.
- Also merging `claude/phase2-css` (position/z-index/flexbox) — in progress.

### 2026-05-16 - Claude (CSS phase2 merge fix-up + Copilot review pass)

- Fixed deep merge regressions introduced when `claude/phase2-css` was merged into master (`10e3399`):
  - Restored `FormControlCommand` and `FormControlKind` type definitions that were lost in the merge.
  - Re-unified `merge_fragment` (bad conflict resolution had split it into two fragments, leaving controls-extend outside any function).
  - Fixed `layout_preformatted_fragments` Control arm (referenced undeclared variables from a different function).
  - Fixed `LayoutContext` initialization (missing `..LayoutContext::default()`).
  - Fixed `layout_block_element` / `layout_mixed_children` call sites (missing `current_form: None` argument).
  - Fixed `browser.rs` test `ComputedStyle` literals (missing `effective_opacity` field).
- Addressed 3 remaining Copilot issues flagged before the rate limit:
  - `BoxShadow.color`: changed `u32` → `Option<u32>` (None = inherit `currentColor`).
  - `TextCommand.line_height_px`: new field; `clip_commands_to_box` now clips on line height, not font size.
  - `MAX_OFFSCREEN_PIXELS` in `gui.rs`: reduced from 8192×8192 (268 MB) to 4096×4096 (64 MB).
- Ran 5 Copilot review rounds (PRs #42 → #43 → #44 → #46 → #47); each round fixed all flagged comments.
  - Final PR #47 merged with zero Copilot comments.
- `cargo test`: 134 passing, 0 failed.
- `CSS_ROADMAP.md` was missing from master (was on `claude/phase2-css` only); PR #48 (`claude/add-css-roadmap`) adds it.

### 2026-05-16 - Claude (Phase 5 CSS roadmap — full implementation)

Implemented all Phase 5 CSS roadmap items across 6 batches on `claude/phase5-css` (PR #49).

- **Batch 1** — CSS math + images:
  - `clamp()`, `min()`, `max()` in all length contexts, including nested inside `calc()`
  - `aspect-ratio` (milliratio u32 to keep `Eq`), applied in image layout
  - `object-fit` / `object-position` with 5 rendering modes in `draw_scaled_image`
  - `content: attr(name)` resolved from element attributes in `::before`/`::after`

- **Batch 2** — Interactive pseudo-classes + element hitboxes:
  - `:hover`, `:focus`, `:active` as real pseudo-classes threaded through the entire cascade
  - `InteractiveState` struct passed into `build_styled_tree` + selector matching
  - `ElementHitbox` emitted per block element → GUI hit-tests to find hovered node
  - `BrowserPage.relayout()` + GUI re-renders only when hovered node changes

- **Batch 3** — Flex extensions + form pseudo-classes:
  - `display: inline-flex`, `align-content`, `flex-flow` shorthand
  - `:checked`, `:disabled`, `:enabled` pseudo-classes

- **Batch 4** — CSS Grid layout:
  - Full `display: grid` / `display: inline-grid` with auto-placement engine
  - `grid-template-columns/rows`, `fr` units (two-pass), `repeat()`, `span N`
  - `grid-auto-rows/columns`, explicit line-number placement

- **Batch 5** — Intrinsic sizing + sticky + cursor:
  - `min-content`, `max-content`, `fit-content()` as `LengthValue` variants
  - `position: sticky` lays out as relative (scroll-offset tracking deferred)
  - `CursorKind` enum (14 variants), `pointer-events: none` gates hitboxes

- **Batch 6** — Filter + pseudo-elements + parser stubs:
  - `filter: blur(px)`, `brightness(f)`, `opacity(f)` parsed into dedicated fields
  - `::placeholder`, `::selection` parsed; `compute_placeholder_style()` API
  - `@supports` (always-true), `@layer` (name ignored), ~20 no-op properties

- `cargo test`: 157 passing (was 134 at start of session), 0 failed.
- CSS_ROADMAP.md updated: Phase 5 → ✅, Phase 6 future work documented.

### 2026-05-16 - Codex (storage and cookie support)

- Added origin-scoped `localStorage` and `sessionStorage` backed by shared site state.
- Added `document.cookie` getter/setter behavior and request/response cookie propagation in the HTTP layer.
- Added `demo/storage-demo.html` so storage and cookie state can be exercised manually.

### 2026-05-16 - Codex (browser history back/forward)

- Added browser-level history tracking for full document loads.
- Added back/forward chrome buttons and `Alt+Left` / `Alt+Right` shortcuts.
- Kept same-document soft navigation in sync with the browser history entry for the current page.

### 2026-05-25 - Codex (goal lock)

- Locked the north star in the roadmap and handoff notes: Chrome-level practicality so Google / YouTube / other complex sites can be browsed and operated without synthetic fallback pages.
- Reaffirmed the working order as WebComponents / shadow DOM details, DOM mutation to reflow / hit-test synchronization, fetch / XHR / history / storage browser-grade behavior, and real-site stability checks.

### 2026-05-25 - Codex (slotchange and assignedSlot)

- Added `assignedSlot` on nodes and synchronous `slotchange` dispatch when slot distribution changes.
- Kept the implementation local to the existing shadow DOM bridge so it stays aligned with the current WebComponents work.

### 2026-05-25 - Codex (flattened slot assignment helpers)

- Extended `slot.assignedNodes(...)` and `slot.assignedElements(...)` with a `flatten` option so nested slot trees can be traversed more like a real browser.
- Updated the roadmap and README to reflect the broader WebComponents surface.

### 2026-05-17 - Codex (DOM traversal & manipulation APIs)

- Added `matches(...)`, `closest(...)`, and `contains(...)` to the DOM bridge so selector-driven event delegation code can walk the tree without special cases.
- Added `firstElementChild`, `lastElementChild`, `previousElementSibling`, and `nextElementSibling` accessors for framework-style traversal.
- Added dynamic `document.body`, `document.head`, and `document.documentElement` getters to stay consistent as the DOM grows.
- Extended `classList` with live helpers (`value`, `length`, `item(...)`, `toString()`, `replace(...)`, `toggle(...)`).
- Added live NamedNodeMap-style `element.attributes` collection with `length`, `item(...)`, `getNamedItem(...)`, and array-like iteration.
- Added `hasAttribute(...)`, `hasAttributes(...)`, `getAttributeNames(...)`, and `toggleAttribute(...)` to elements.
- Added regression coverage for DOM traversal, sibling lookup, attributes collection, token list, and dynamic getters.

### 2026-05-17 - Codex (script-driven scroll & history scroll restore)

- Added `window.scrollTo(...)`, `window.scrollBy(...)`, and node `scrollTop` setter support, wired back into GUI viewport scroll state.
- Extended same-document and browser-level history entries to store and restore scroll positions on `history.back()` / `history.forward()`.
- Added `demo/scroll-demo.html` so the new scroll APIs can be exercised manually.

### 2026-05-17 - Codex (computed style, header and state APIs)

- Added `matches(...)`, `closest(...)`, and `contains(...)` to the lightweight DOM bridge so event delegation code can inspect and climb the tree without special cases.
- Added `firstElementChild`, `lastElementChild`, `previousElementSibling`, and `nextElementSibling` accessors so framework-style traversal paths can read the surrounding element structure.
- Added a regression test that exercises selector matching, ancestor lookup, containment, and sibling traversal together on a small nested DOM tree.

### 2026-05-17 - Codex (script-driven scroll APIs)

- Added `window.scrollTo(...)`, `window.scrollBy(...)`, and `scrollTop` setter support so scripts can move the viewport directly.
- Wired JS scroll changes back into the GUI scroll state so the rendered page and `window.scrollY` stay aligned.
- Added regression coverage for scroll-position getters, setters, and scroll-driven event handling.

### 2026-05-17 - Codex (scroll demo page)

- Added `demo/scroll-demo.html` so the new scroll APIs can be exercised manually without digging through source code.
- The demo uses a tall DOM tree plus buttons for `scrollTo`, `scrollBy`, and `scrollTop` setter checks.

### 2026-05-17 - Codex (CSS boundary policy)

- Defined a clearer boundary for CSS work: treat the Claude `claude/phase5-css` branch as the CSS parser/layout owner and avoid broad or destructive CSS edits from Codex.
- Documented the exception workflow for JS tasks that genuinely need CSS-facing integration: keep the diff minimal, request Copilot review, and log touched files in `change.md`.
- Kept the current update CSS-neutral; this change only tightened coordination rules and documentation.

### 2026-05-17 - Codex (dynamic document root getters)

- Converted `document.body`, `document.head`, and `document.documentElement` to dynamic getters so they stay consistent if the DOM is extended after load.
- Added a regression test that creates body/head nodes after startup and verifies the getters track the live tree.
- Updated the roadmap and README to reflect the current DOM consistency surface.

### 2026-05-17 - Codex (mutation snapshot refresh)

- Made GUI-driven DOM attribute writes refresh the live page snapshot so mutation notifications can bump layout revision and invalidate cached reflow immediately.
- Added a regression test that mutates the root element, then verifies the refreshed page snapshot and layout revision update together.
- Recorded the new snapshot-refresh behavior in the README and roadmap notes.

### 2026-05-17 - Codex (same-document history scroll restore)

- Extended same-document history entries to store scroll positions, and restored them on `history.back()` / `history.forward()`.
- Added a regression test that walks a same-document history stack and verifies the stored scroll position comes back with each entry.
- Updated the README and roadmap notes to mention same-document scroll restoration.

### 2026-05-17 - Codex (full-document history scroll restore)

- Extended browser-level history entries to store scroll positions, and restored them when navigating back and forward across document loads.
- Updated the browser history load path so scroll state is reapplied after a full document load when history demands it.
- Recorded the browser-level scroll restoration behavior in the README and roadmap notes.

### 2026-05-17 - Codex (computed style and DOM token list helpers)

- Added `getComputedStyle(...)` snapshots for common layout-sensitive values, including inherited color / font / spacing properties and shorthand box values.
- Extended `classList` with `value`, `length`, `item(...)`, `toString()`, `replace(...)`, and force-aware `toggle(...)`.
- Added `hasAttributes(...)` and `toggleAttribute(...)` on elements so scripts can introspect and flip attributes without manual DOM plumbing.
- Updated the README and roadmap notes to reflect the broader DOM / computed-style surface.

### 2026-05-17 - Codex (attribute collection live bridge)

- Added a live `element.attributes` collection with `length`, `item(...)`, `getNamedItem(...)`, named lookup, and array-like iteration support.
- Added regression coverage for attributes collection indexing, named lookup, and iteration order.
- Updated the README and roadmap notes to reflect that live attribute collection support is now available.

### 2026-05-17 - Codex (fetch/XHR response headers)

- Added response header iteration helpers to the lightweight fetch response surface.
- Added XHR `getResponseHeader(...)` and `getAllResponseHeaders()` support backed by the stored response header map.
- Added regression coverage for response header iteration plus XHR header access.

### 2026-05-17 - Codex (history state and hashchange/popstate)

- Added `history.state` support for same-document session history entries.
- Dispatched `popstate` on history back/forward and `hashchange` on same-document fragment changes.
- Added regression coverage for `hashchange` and `popstate` dispatch behavior.

### 2026-05-17 - Codex (YouTube synthetic fast path)

- Short-circuited generic YouTube home / non-watch loads to a synthetic shell before starting the heavy JS session.
- Kept the watch-page summary path intact while avoiding the runaway memory growth seen on the full YouTube app shell.
- Verified the new path with a process-memory smoke test that stabilized instead of growing without bound.

### 2026-05-16 - Codex (browser history back/forward)

- Added browser-level history tracking for full document loads.
- Added back/forward chrome buttons and `Alt+Left` / `Alt+Right` shortcuts.
- Kept same-document soft navigation in sync with the browser history entry for the current page.

### 2026-06-19 - Claude PM / Codex (dynamic import())

- Implemented dynamic `import()` (preload model, user-chosen Option A) end-to-end. Step 1 (compiler+VM): `ModuleContext.dynamic_imports` map + `Opcode::DynamicImport` wraps a module namespace (or undefined→reject) in a Promise; literal specifiers resolve from the map, computed/unknown reject gracefully. Step 2 (host): `load_module_graph` walks the full AST (boa_ast Visitor) for `import("literal")` calls and preloads those module graphs (non-fatal on failure), populating `dynamic_imports`. crates.io/rollupjs/webpack.js.org/svelte.dev all clear the old `Unimplemented("import() calls")` compile wall; vuejs.org still renders (no ESM regression). 599 tests green (tests/dynamic_import.rs). Not yet: computed `import(var)` (rejects — needs runtime specifier resolution in the VM).
- New leads surfaced after the unblock: svelte.dev `Invalid URL` (our hand-rolled URL parser is too strict for some input — clean fix candidate); rollupjs `Object.create prototype must be an object or null`.

### 2026-07-22 - Claude (Object.defineProperty descriptor merge)

- Root-caused the rollupjs.org `Object.create prototype must be an object or null` lead: `Object.defineProperty` (and `Reflect.defineProperty` / `Object.defineProperties`) replaced the existing own property with the parsed descriptor wholesale, so Babel's per-class `Object.defineProperty(fn, "prototype", {writable:false})` clobbered `fn.prototype` to `undefined`, and the next `class B extends A` died inside `_inherits`.
- Implemented spec-style ValidateAndApplyPropertyDefinition merging (`value_to_property_descriptor_merged`): fields absent from the descriptor object inherit the existing property's attributes; accessor/data kind switches follow the spec; mixed accessor+value descriptors now throw TypeError. `Object.create` errors now report the received type, with an optional backtrace dump under `TOBIRA_DEBUG_CONSOLE`.
- Verified headless via `--cli`: rollupjs.org clears the Algolia module crash (next lead: `object is not callable` in `theme.DwJmkNlp.js`); svelte.dev now loads with meaningful content and no `Invalid URL` in the current checkout. `645` tests green including new `tests/define_property_merge.rs` (9 cases).

### 2026-07-22 - Claude PM / Codex (Symbol.iterator as a real property)

- `Symbol.iterator` existed only on `generator_prototype`, `URLSearchParams`, `Headers`, and `FormData`. `for..of` over arrays/strings/Map/Set worked solely through the VM's internal fast path, so reading the property returned `undefined`: `[][Symbol.iterator]`, `''[Symbol.iterator]`, `new Map()[Symbol.iterator]`, `arguments[Symbol.iterator]` were all missing. Transpiled bundles gate on exactly this (`_createForOfIteratorHelper` reads `o[Symbol.iterator]` and throws `Invalid attempt to iterate non-iterable instance` when it is absent), so every Babel/SWC-compiled `for..of` over a non-array broke on real sites.
- Wired it as a real data property (`writable: true, enumerable: false, configurable: true`) on `Array.prototype` (identical function object to `Array.prototype.values`), `Map.prototype` (=== `entries`), `Set.prototype` (=== `values`), `TypedArray.prototype` (new `TypedArrayProtoValues`), and `String.prototype` (new `StringProtoIterator`, iterating by Unicode code point so astral characters stay whole). `arguments` inherits from `Array.prototype` and needs nothing of its own.
- Added a shared `%IteratorPrototype%`-style `iterator_prototype` carrying `next` and a self-returning `Symbol.iterator`; `ForOfIterator` objects and `generator_prototype` both inherit from it. Without this, iterators themselves were not iterable, which left the main real-world case (`for (const x of arr.values())` under a transpiler) still broken. Both methods are non-enumerable, so iterators no longer leak `next` into `Object.keys` / `for..in`.
- Deliberate scope extension: `Map.prototype.entries/keys/values` and `Set.prototype.values` now return an iterator instead of an array, matching the spec. Verified no regression across `for..of`, spread, `Array.from`, and destructuring.
- `652` tests green, including new `tests/symbol_iterator_property.rs` (7 cases, covering the `_createForOfIteratorHelper` shape applied to iterator results, not just to containers).
- Still open from the earlier probe: arrow functions and shorthand/class methods still have a `.prototype` (spec says they must not).

### 2026-07-22 - Claude PM / Codex (DOM interface constructors are real functions)

- The `DOM_INTERFACE_NAMES` globals (`HTMLElement`, `EventTarget`, `Node`, `Element`, …) were built with `allocate_ordinary_object`, so `typeof HTMLElement` was `"object"`, `.name` was missing, and `.prototype.constructor` was undefined. Sites feature-detect with `typeof HTMLElement === 'function'` before installing web-component code, and transpiled `class X extends HTMLElement` reaches `Reflect.construct(HTMLElement, …)` which died in `require_callable`.
- They are now callable objects backed by a new `BuiltinId::DomInterfaceConstructor`, with a non-writable `name`, a `prototype.constructor` back-reference, and browser-matching descriptors. A plain call `HTMLElement()` throws `TypeError: Illegal constructor`; `new HTMLElement()` and `super()` from a subclass construct normally.
- Fixed a latent hole this exposed: `instanceof_value` short-circuited on DOM interface constructors, checking only `host_node_interfaces` and returning `false` for anything else. So `class X extends HTMLElement {}; new X() instanceof HTMLElement` was `false` even though the prototype chain was correct. The interface check is now a fallback — host nodes match by interface name, everything else falls through to the ordinary prototype-chain walk. Verified both directions (deep subclass chains match; plain objects, unrelated classes, and sibling interfaces still do not).
- `require_callable`'s bare `object is not callable` now names the offender (`name`, or `constructor.name`, plus the `ObjectKind`). Error-path only. This should make the remaining real-site walls much cheaper to diagnose.
- `661` tests green: new `tests/dom_interface_constructors.rs` plus a real-host `instanceof` case in `tests/phase6_dom.rs` (`document.body instanceof HTMLElement/Element/Node/EventTarget`).
- **Next lead — `Reflect.construct` ignores its newTarget argument, and `new.target` is not propagated.** This is pre-existing and NOT specific to DOM interfaces: it reproduces on ordinary functions (`Object.getPrototypeOf(Reflect.construct(Base, [], Derived)) === Derived.prototype` is `false`; `new.target` inside a constructor invoked via `Reflect.construct` is not the passed newTarget). Babel's `_createSuper` is built on exactly this, so every transpiled `class extends` currently gets the wrong instance prototype — likely the widest-reaching remaining JS gap and the best next target. (Done in the next entry.)

### 2026-07-22 - Claude PM / Codex (newTarget propagation)

- Threaded an explicit newTarget through the construct path: new `construct_value_with_new_target(_sync)`, with the old two-argument entry points delegating with `new_target = constructor` so every existing call site is unchanged. `construct_this_value` now derives the prototype from the newTarget (spec: OrdinaryCreateFromConstructor), the closure frame's `new_target` is the passed newTarget, and bound constructors forward the newTarget they received instead of substituting the bound target.
- `Reflect.construct` reads its third argument, defaults it to the target when absent/undefined, and throws `TypeError: Reflect.construct: The last argument is not a constructor` when it is present but not constructible (new `is_constructor_value` helper).
- Payoff: Babel's `_inherits` + `_createSuper` shape now works end to end — a transpiled subclass instance gets the subclass prototype, keeps the base in its chain, and both subclass and base methods resolve. Before this, every transpiled `class extends` produced an instance with the *base* prototype, silently losing all subclass methods.
- Scope extension found along the way: `Object.create(proto, descriptors)` silently ignored its second argument. The pre-existing `tests/define_property_merge.rs` used that form but only asserted the prototype link, so nothing caught it. Now implemented over own *enumerable* keys with spec descriptor defaults; verified accessor descriptors, `false` defaults, non-enumerable descriptor-map entries being skipped, and non-object descriptors throwing.
- **Known limitation, now covered by a test that asserts the current behaviour:** a native `super()` is lowered to `Opcode::Call` rather than routed through the construct path, so `new.target` inside a base constructor reached via `new Derived()` is `undefined` instead of `Derived`. Measured: standalone `new A()` correctly gives `A`; `Reflect.construct(A, [], B)` correctly gives `B`; only the native `super()` chain is wrong. The common abstract-class guard (`if (new.target === Abstract) throw`) still behaves correctly by accident. Fixing it means changing how the compiler lowers `super()`.
- `670` tests green. Note for whoever picks this up: Codex's first pass had *rewritten* `tests/new_target.rs` and dropped two pre-existing passing cases (the `new.target` guard pattern and the native-class-constructor case). They were restored from `HEAD`. Watch for this when handing an existing test file to an agent.

### 2026-07-22 - Real-site status after the three JS fixes

Headless sweep via `--cli` with `TOBIRA_DEBUG_CONSOLE=1`, after `Symbol.iterator` + DOM interface constructors + newTarget:

- **svelte.dev — CLEAN.** 6.8 KB of rendered content, zero uncaught JS errors.
- **webpack.js.org — advanced.** The old lead `property assignment requires an object (got undefined)` is **gone** (0 occurrences). The page now renders its nav and content (4.0 KB). New wall: `TransformStream is not defined` — a missing Web Streams global, a different class of gap (add the global, or stub it well enough for the bundle's feature detection).
- **rollupjs.org — renders fully** (2.4 KB: hero, feature cards, nav, all links). One error remains, unchanged in location: `object is not callable (kind Array)` in `assets/chunks/theme.DwJmkNlp.js` (the VitePress theme JS — search box, appearance toggle). Content is unaffected. The `(kind Array)` detail is new, courtesy of the improved `require_callable` diagnostic: something is calling an Array. A plausible shape is Babel's `_createForOfIteratorHelperLoose` doing `it = it.call(o)` where `o[Symbol.iterator]` resolved to an array rather than a function — worth checking which object that is before assuming.
- **crates.io — still title-only** (122 B: just `# crates.io: Rust Package Registry`, no errors reported). Unchanged by this work; it is an Ember SPA and the gap is elsewhere. Needs its own investigation.

Recommended next targets, in order: (1) `TransformStream` / Web Streams globals for webpack.js.org — smallest and well-defined; (2) the rollupjs `kind Array` callee — now much cheaper to chase with the improved diagnostic; (3) native `super()` `new.target` propagation (compiler lowering change); (4) crates.io's empty render.

### 2026-07-23 - (separate local session) strip leftover BOMs

- `src/engine/vm.rs` and `src/engine_host.rs` still carried a leading UTF-8 BOM from the old encoding accident. Removed both (commit `527c55c`); no code change. All tracked `*.rs` are now BOM-free.

### 2026-07-23 - Claude PM / Codex (global `eval`)

- Implemented the global `eval` function. It was entirely absent (`typeof eval === "undefined"`); Google Search's inline script #2 died on `eval is not defined` and collapsed the results page to a 407-byte fallback.
- **Indirect eval only.** The evaluated code runs at global scope, reusing the existing `eval_source` machinery (originally built for `document.write`'d `<script>`). Global reads/writes and `var`/`function` declarations leaking to the global object all work; the completion value is returned (`eval("1+1") === 2`). Non-string arguments are returned unchanged. Parse/compile failures throw a `SyntaxError` (the `document.write` path still uses its old `TypeError` mapping — the two now share `eval_source_with_errors` with an error-kind flag). New `BuiltinId::GlobalEval`; `window.eval`/`globalThis.eval` resolve to the same function.
- Completion value needed a compiler path: `compile_for_eval_completion` / `compile_statements_preserving_final_expression` leave the final expression-statement on the stack instead of `Pop`-ing it. Verified stack balance under `TOBIRA_VERIFY_BYTECODE=1` (full suite green with it on).
- **Explicitly out of scope — direct eval.** `function f(){ var local = 1; return eval("local"); }` cannot see `local` (throws ReferenceError); direct eval would need the compiler to special-case `eval(...)` call sites and keep local scopes reifiable. If a real site relies on direct eval, this will not help it.
- **Completion-value limitation:** only a final *expression statement* is returned. `eval("if (true) 5")` yields `undefined`, not `5` (spec would give `5`). Acceptable for now.
- Adjacent gap noticed while testing (NOT fixed): `new String("x")` returns a primitive string, not a wrapper object — `typeof new String("x") === "string"`. Unrelated to eval; flag for later.
- `679` tests green (new `tests/global_eval.rs`, 9 cases). Codex left the working tree with 28 line-ending-only dirtied files again (real changes were only the 3 source files + the new test); restored as before. This has happened on every Codex run this session — the agent writes LF into CRLF files. Not harmful (committed blobs are LF via `core.autocrlf`), but check `git diff --numstat` before staging.

### 2026-07-23 - Claude (real-site sweep + rollupjs deep-dive; diagnostics)

- After `eval`, re-swept Google/search engines. All render their top shell but die in the heavy dynamic JS: Google Search `eval` wall cleared but next line does `undefined()` in the same inline script; Google top `_DumpException is not a function`; Bing `_w is not defined`; DuckDuckGo non-function call; Google News `object is not callable (kind Ordinary)`. Google-class obfuscated loaders are many-layered — one fix just exposes the next wall. YouTube: `www.youtube.com` returns a Google **login/consent** page (bot-treated), not the app; only engine gap there is `<canvas>.getContext` unwired. Video playback is out of scope regardless (CPU renderer).
- webpack.js.org's `TransformStream` wall was investigated and is **not small**: `vendor.js` uses `new TransformStream({...}).pipeThrough(...)` — it's the Vercel AI SDK (in-page AI assistant) using the full Web Streams API (ReadableStream/WritableStream/pipeThrough/reader/backpressure, all async), not feature detection. The doc **body already renders** without it; Streams would only power the AI widget. Deferred as a large, low-ROI-for-this-page item. (User confirmed: treat webpack as body-OK, move on.)
- **rollupjs.org `object is not callable (kind Array)` — narrowed but not yet located.** Built two diagnostics to chase it (both kept, see below). Findings: the culprit value is the array **`["items"]`** (length 1), **called as a plain function** (`this=undefined`) with **a single function argument**, at **module top level** (`theme.DwJmkNlp.js`, backtrace shows only `<script>`). The source never literally calls an array (`]()` appears 0 times in the 106 KB bundle), so our engine is **mis-evaluating some expression to `["items"]`**. Ruled out simple array-method bugs: `Object.keys(obj).reduce/forEach/map/filter/find/some/every/sort/flatMap`, `Object.entries/values(...).x`, and direct `["items"].reduce/forEach` all work (15/15 probes pass). The two literal `["items"]` in source are Vue `createVNode(..., 8, ["items"])` **dynamicProps** (data, not callees), and they sit inside render closures — not the top-level `<script>` frame — so they are NOT the culprit. Leading remaining hypothesis: a **scope/binding-resolution bug** in heavily-minified single-letter-name code, where a helper reference (e.g. Vue's `withCtx`/`computed`, called as `X(fn)`) resolves to the wrong binding holding `["items"]`. Pinia store setup (`Sc=Oo()` at top level, `Object.keys(getters).reduce(...=>...A(()=>...))`) is the most likely neighborhood.
- **Blocker: the bytecode has no source-line info** (`FunctionProto { code: Vec<Opcode> }`; no spans). So backtraces are function-name-only (`capture_backtrace`) and cannot map to a minified column. Pinpointing this (and future minified-site bugs) efficiently needs **source-line/column tracking in the bytecode** so backtraces read `at <name> (url:line:col)`. That is the recommended foundational next step — it cracks this bug and makes every future real-site debug far cheaper. Medium task (compiler emits spans per opcode; `capture_backtrace` formats them).
- **Diagnostics kept** (proved their worth locating the above; my own code, error-path only): (1) `describe_non_callable_object` now previews an Array callee's length + first elements, so `object is not callable (kind Array, length 1, ["items"])`; (2) module-execution errors now append the backtrace (mirroring the classic-script path), so ESM failures surface `\n    at <script>` etc. — previously only classic `<script>`/inline errors did.

### 2026-07-23 - Claude PM / Codex (source-position backtraces + rollupjs pinpointed)

- **Source positions now flow to backtraces.** The parser front-end is still boa (`boa_ast` 0.21.1; "boa removal" was only the runtime); boa's `Call`/`New` nodes impl `Spanned` and expose column-level positions. The compiler now records `(line, column)` for every Call/New/tagged-template site into a new `FunctionProto.call_positions: Vec<CallSitePosition>` (empty when unused → no overhead), and `capture_backtrace` binary-searches it by the frame's `ip-1` to print `at <name> (line:column)`. This is the foundation the earlier blocker asked for — it makes minified-site debugging tractable (column matters; bundles are ~1 line). `685` tests green (new `tests/call_source_position.rs`, incl. a same-line-different-columns test).
- **rollupjs.org `["items"]` PINPOINTED with the new backtrace.** It reads `at <script> (2:40706)`. `theme.DwJmkNlp.js` line 2 col 40706 is `const vs=_o(); … const ys=vs((e,t,n,o,s,i)=>(l(),P("div",gs)));` — the failing call is `vs(arrowFn)`. So `vs = _o()` returned the array `["items"]` instead of a function. `_o` is the ESM import `L as _o` from `./framework.P2XOc7lE.js`.
- **Import-alias hypothesis DISPROVEN — bug is in executing the framework helper, not in binding.** I traced the whole binding chain against boa 0.21 semantics (`ImportSpecifier.binding`=local, `.export_name`=imported; `ExportSpecifier.private_name`=local, `.alias`=exported). `compile_import_declaration` (modules.rs) registers `_o → Named{framework, "L"}`; the use-site (`statements.rs` `ModuleImport` arm) emits `emit_module_import_name(framework, "L")` = fetch namespace["L"]; `compile_export_list` reads private_name/alias correctly. All correct. Also: the failure is at the `vs(` call, not the `_o(` call, so `_o` (framework export `L`) IS a callable function — but **`L()` returns the array `["items"]` instead of a wrapper function.** So the remaining bug is that our engine mis-executes the Vue runtime helper `L`'s body (in `framework.P2XOc7lE.js`) to yield `["items"]`. NEXT STEP: locate `L`'s definition in `framework.P2XOc7lE.js` (use the new backtrace / a probe on `_o`'s value right after `vs=_o()`), identify which JS construct in `L`'s body our engine evaluates to `["items"]`, and reproduce that construct in isolation. (`L` is likely Vue's `withScopeId`/`pushScopeId`-family helper: `fo("data-v-…")` then `vs=_o()` then `vs(renderFn)`.)
- **Process hazards hit this session (for the next agent):**
  - **Codex ran `cargo fmt`** at the end of the first source-position run, reformatting the ENTIRE codebase (38 files, thousands of whitespace-only lines). Unshippable. Recovered via `git checkout -- .` and re-ran Codex with an explicit "DO NOT run cargo fmt / minimal diff / list only the 4 target files" banner — the redo produced a clean 4-file diff. Always forbid `cargo fmt` in Codex prompts and check `git diff --numstat` before staging.
  - **Disk filled up** (`target/` reached 28.8 GB; C: hit 0.01 GB free) mid-session from repeated rebuilds, causing spurious `autocfg`/`os error 112` build panics that look like test failures but are not. `cargo clean` freed it (→ 22 GB). If builds panic in build scripts, check free space first.

### 2026-08-07 - Claude (git history loss + recovery)

- **The repo's git history was destroyed and has been reconstructed.** This checkout used to be a git *worktree* whose parent repo (`vscode/browser`, holding the object database) was deleted around 2026-07-27 by a bulk cleanup script that used `rmdir /s /q` with a `robocopy /MIR` fallback — a method that bypasses the Recycle Bin. Every git command failed with `fatal: not a git repository: .../vscode/browser/.git/worktrees/browser-js-engine`. Working files were untouched.
- **Nothing was lost.** The user keeps an archive of `vscode/` on the `Z:` drive, and the parent repo was copied there on 2026-07-27 — after the last commit (`44e9fe1`, 2026-07-26). The full object database and all 47 refs survived there. Everything below is now back in this repo. (That archive has since been renamed `Z:\vscode\tobira-repo-archive`; see the naming-consolidation entry below.)
- **Order of investigation, for the next time this happens**: local Recycle Bin (empty — the `robocopy /MIR` method bypasses it), VSS shadow copies (need admin), `Temp\tobira-fix-backup` (empty), OneDrive's *online* Recycle Bin (30-day retention, untried), **and any external/archive drive** — the last one is what actually had it. Ask about archives before concluding a loss.
- **Recovery performed**:
  1. Backed up the working tree first (`robocopy /E /XD target`, 4.8 MB) to `C:\Users\user\AppData\Local\tobira-backup-20260807`, including the broken `.git` pointer file.
  2. As an interim measure (before the archive was known), re-cloned the remote `--no-checkout`, moved the fresh `.git` in, set `core.autocrlf true` (matching the old config — without it every file shows as a whole-file CRLF rewrite), and `git reset` mixed (*not* checkout, which would overwrite the working files) to rebuild the index from `91291ea`. Those squashed recovery commits are kept on the branch `recovery-squash-20260807` and are redundant; delete it whenever.
  3. Once the `Z:` archive was found, verified it byte-for-byte: `git archive 44e9fe1 | tar -x` into a temp dir, then `diff -r --strip-trailing-cr` against the working tree. **Identical** apart from this HANDOFF note and two gitignored react fixtures. (So `src/layout.rs`'s 2026-08-07 mtime was an OneDrive touch, not an uncommitted edit.)
  4. Added the archive as the remote `archive`, fetched all 47 refs into `refs/remotes/archive/*` plus tags, and reset `master` to the real `44e9fe1`.
- **The 18 recovered commits** (`91291ea..44e9fe1`) cover: defineProperty descriptor merge, `Symbol.iterator` as a real property, DOM interfaces as real constructors, `new.target` propagation, BOM removal, global `eval`, not-callable/module-error diagnostics, source-position backtraces, module top-level bindings made frame-local (**rollupjs.org fully working** — the `["items"]` bug from the previous entry is fixed), HTML tokenizer char-boundary panic, inline image rendering, frameset loss, scheme-prefixed href resolution, table cell inline flow, table column shrinking, and legacy `align`/`valign`.
- **Verified**: `cargo test` → `716` passed, `0` failed, 52 suites.
- **Not yet pushed** — `master` is 18 commits ahead of `origin/master`, plus 46 other refs that exist only on the archive. Pending the user's go-ahead.

### 2026-08-07 - Claude (naming consolidated on "tobira"; backup layout)

The project name is `tobira`; the old `browser` naming is being retired. Package name, crate name (`tobira_engine`), GitHub repo and the working directory were already `tobira`, so this pass covered the leftovers.

- **All 21 archive-only branches were pushed to GitHub first**, before touching anything on `Z:`. Every ref that carried commits not reachable from `master` — `codex/make-js-engine` (61), `codex/js-engine` (55), `codex/js-event-capture` (20), the `feat/*` branches, four `worktree-agent-*` — now exists on `origin`. Nothing depends on the local disk any more.
- **`Z:` layout is now**:
  - `Z:\vscode\tobira` — a fresh copy of this checkout including `.git` (excluding `target/`). This is the up-to-date backup; **there wasn't one before** — the archive only held the pre-incident parent repo.
  - `Z:\vscode\tobira-repo-archive` — the old parent repo, renamed from `browser`. Kept as the historical object database. The `archive` git remote points here; it is redundant now that everything is on `origin`, so removing it is fine.
  - `Z:\vscode\tobira-specs` — unrelated spec cache, untouched.
- **Still to delete** (verified redundant, but the tool refused the recursive delete — do it by hand): `Z:\vscode\browser-claude`, `Z:\vscode\browser-codex`, `Z:\vscode\browser-content-thread`. These are worktree remnants whose `.git` is a dangling pointer into the deleted OneDrive path. Each was diffed against its branch tip (`feat/outline-text-decoration`, `codex/make-js-engine`, `codex/content-thread`) and matches exactly — no uncommitted work.
- **Deliberately not renamed**: `src/browser.rs`. "The browser itself" is an accurate module name, and `tobira/src/tobira.rs` would be redundant. Left alone.
- ~~**Known leftover**: `repomix-output.xmlbrowser.xml` is tracked but is a generated artifact with a mangled filename.~~ Resolved 2026-08-23: untracked and gitignored.

### 2026-08-23..25 - Claude (CSS/layout conformance sprint; verified and merged)

49 commits landed across three days (12 on 08-23, 20 on 08-24, 17 on 08-25). The last 21 were developed on the branch
`worktree-css-html-conformance` in a dedicated worktree and have now been fast-forwarded into `master` (`ef607f2`) and pushed.
Both `master` and that branch exist on `origin`; the branch is kept as a backup ref.

- **What the sprint covered** (only 6 source files, +3681/-128, incl. the new `src/svg.rs` at 1299 lines):
  - flex: reverse direction + `order`, `flex-shrink`, widths unified on the margin box, item content-width measurement,
    nested containers that were ignoring their own `display`
  - absolute positioning: spec-correct placement, `top`/`left: auto` keeping the static position, `bottom` pinning to the
    bottom edge, plus four foundational fixes in one commit
  - `inline-block` as an atomic inline, then its padding, then margins counted into its width (three successive commits)
  - intrinsic width measured as "the widest line", and an element counting its own dimensions
  - `calc()` percentages resolved against the containing block, `rem` against the root font size, `overflow: hidden`
    actually clipping, SVG + `data:` URL images, a crash on large `viewBox`, and a comment-stripper that was corrupting UTF-8
  - 49 new tests came with it (40 in `layout.rs`, 9 in `svg.rs`)
- **Verified after the merge**: `cargo test` 867 passed / 0 failed / 9 ignored / 64 suites; `cargo build --release` clean in 1m57s.
- **Real-page measurements** (release binary at `ef607f2`, via `TOBIRA_DEBUG_CSS=1 --dump-styled`):
  - Wikipedia (ja, "HTML") - **healthy**: 3707 elements, 35 KB visible text, 1091 hitboxes, content laid out at the correct 1192px width
  - MDN (ja) - content is all there but the layout collapses; cause traced to named grid areas, see Known Gaps
  - Google top - 406 elements but only **188 bytes** of visible text against 82 KB hidden; still the known JS wall
- **Tooling gained this sprint**: `--dump-styled` now also takes `TOBIRA_DUMP_DEPTH=<n>`, `TOBIRA_DUMP_IMAGES`, and
  `TOBIRA_DUMP_TEXT`, and prints the command mix plus an svg/raster image split.
- **Two roadmap documents were wrong and have been corrected** (details in each file):
  - `docs/JS_ROADMAP.md` called Phase 5 (Layout Reflow) an unstarted "architectural blocker". It is implemented and on by
    default - `incremental_restyle_enabled()` in `src/browser.rs`, with `compute_dirty_roots()` / `relayout()` and a
    dedicated test. `TOBIRA_INCREMENTAL_RESTYLE=0` forces the old full-rebuild path.
  - `docs/CSS_ROADMAP.md` marked `flex-shrink` complete back in Phase 2, yet it was genuinely implemented on 08-25.
- **Housekeeping done in the same pass**: the redundant `.claude/worktrees/css-html-conformance` worktree was removed
  (identical commit, nothing unique, 5.3 GB of `target/` reclaimed; its stale lock named a dead session whose PID had already
  been recycled). The NAS copy at `Z:\vscode\tobira` was 50 commits stale at `c766553` and has been fast-forwarded to
  `ef607f2`. Session transcripts and memory now back up to `Z:\vscode\claude-sessions` via
  `~/.claude/scripts/backup-sessions.ps1` (run by hand; uses robocopy `/E`, never `/MIR`).
- **Note for whoever writes here next**: this log had gone silent between 08-07 and 08-25 while 49 commits landed. The entries
  above were reconstructed from `git log` and fresh measurements, so they are thinner on "what we tried and rejected" than the
  older entries. Keep appending as you go.
