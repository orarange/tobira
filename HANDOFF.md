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

## いまの状態（2026-09-18）

- ブランチ `master`。この文書を書いた時点の HEAD は `3b09958`
  （この文書のコミットが直後に乗る）。origin/master と同期しとる。
- `cargo build --release` 通る。警告は 35 件（2026-09-18 実測）で、全部無害:
  unreachable pattern 10（`html.rs` の `is_block_like` の重複リテラル 8、
  `compiler/expressions.rs` の網羅済み match の `_`、`vm.rs` の `createComment` が
  先の本物の腕に食われとる stub）、deprecated `boa_ast` `ImportCall::argument` 4、
  unused mut 2、残りは dead_code 系。数が増えたら中身を見ること。
  OneDrive が PDB を掴んで失敗することがある。そのときは `RUSTFLAGS='-C debuginfo=0'`。
- `cargo test --release` → **1176 通過 / 0 落ち**。
  `TOBIRA_GC_VERIFY=1` を付けても同じ数が通る（GC のルート漏れ監査。下記）。
  数え方: `cargo test --release 2>&1 | tr -d '\000' | grep -aE "^test result" | awk '{p+=$4; f+=$6} END {print p, f}'`
  （`tr -d '\000'` は必須。出力に NUL が混ざって grep が binary 扱いする）
- html5lib 木構築適合 **1213/1229 (98.7%)**（2026-09-18、Noah's Ark 条項で tests23 が 5/5）。
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
TOBIRA_DUMP_DEPTH=40 ./target/release/tobira --dump-styled <url>   # 深いところまで
./target/release/tobira --screenshot out.png <url>   # PNG。TOBIRA_SHOT_HEIGHT で高さ
```

**`--dump-styled` は既定で浅い。** 何もせんと HN でも 59 行しか出ん。
「その要素の箱が無い」と読める出力は、たいてい木が切れとるだけや
（2026-09-04 に一回これで誤診しとる）。深いところを見るときは必ず
`TOBIRA_DUMP_DEPTH` を上げること。
一行に出るのは `bg=`（背景色）、`bgimg=`（背景画像の URL）、`size=`（指定された
width/height）、`mask=`。`bg=none bgimg=<url>` なら「規則は当たっとって画像も
決まっとる」ので、外れとるのは塗りかレイアウトの側や。

主な環境変数: `TOBIRA_DEBUG_CONSOLE`（console と未捕捉エラー）、`TOBIRA_TRACE_STACK`、
`TOBIRA_DUMP_BOXES` / `TOBIRA_DUMP_DEPTH` / `TOBIRA_DUMP_WIDTH`、`TOBIRA_SHOT_HEIGHT`、
`TOBIRA_DEBUG_IMAGES` / `_ATOMIC` / `_FLEX` / `_PAINT` / `_TABLE` / `_CSS`、
`TOBIRA_H5_FILE`、`TOBIRA_INCREMENTAL_RESTYLE`。

Chrome との突き合わせは `tools/geom/`（README 参照）。参照ブラウザは **Chrome ヘッドレス**。
Edge は 2026-08-27 の更新以降 `--dump-dom` が無出力になったので使えん。

**数値だけ見るな。** 表が指定幅を無視する件も `<center>` が表を中央寄せせん件も、
`--screenshot` を足して目で見るまで一つも見つからんかった。修正のたびに一枚撮ること。

**「黙って既定に落ちる」CSS は集計で釣る。**
`TOBIRA_DEBUG_CSS=1 ./target/release/tobira --dump-styled <url>` が、
`apply_declaration` を素通りした宣言を頻度順で出す。実頁を三枚ほど回して
上から見るのが一番効率がええ。2026-09-10 の `pt` の件も `border-radius` の
複数値もこれで出た。**症状から探すより、落ちとる宣言から探すほうが速い。**

**単位は網羅した検体で試す。** `12pt` と `1pc` はちょうど 16px なので、
単位が丸ごと未対応でも「既定の 16px」と一致してしまう。この二つで確認しとったら
`pt` 未対応に気づかんかった。`tools/geom/units.html` に一通り並べてある。

**JS の穴を「素の Vm」で判定するな。** `Vm::new(Heap::new())` で走らせると
`Intl`・`Object.groupBy`・`Array.fromAsync`・`Promise.withResolvers`・
`Segmenter`・`DOMParser`・`Blob`・`FileReader` あたりが軒並み「無い」と出るが、
これらは `engine_host.rs:2845 RUNTIME_PRELUDE`（約1,500行の JS）が
host 側で入れとるだけで、エンジン本体には無い。素の Vm はプレリュードを積まん。
本当に穴かどうかは `EngineSession` 経由か、実頁を `TOBIRA_DEBUG_CONSOLE=1` で
見て判断すること。逆に、コンパイラで落ちるもの（bigint・`with`・`#x in o`・
`o?.#x`・オブジェクトリテラルのメソッド内 `super`）はプレリュードでは救えん。

**JS を触ったら `TOBIRA_GC_VERIFY=1` で一度回す。** GC が「回収せずに印だけ
付ける」モードになり、印の付いた cell を後から読んだら
`[gc-verify] a root holding this reference was not traced` が出る。挙動は
変わらんので、実頁を開くのにも使える。ルートの取りこぼしは stale な GcRef が
`None` を返すだけで落ちんため、症状が原因から遠い。**新しく Value や GcRef を
持つ入れ物を足したときは必ずこれを回すこと。** 全テストと実頁 5 枚で報告 0 が
今の状態。

**期待値は先に Chrome で確定させる。** 2026-09-05 に `background-repeat` を
追うたとき、合成頁 7 行のうち 1 行は**空白が正解**やった（10x10 の箱に 32x32 を
原寸で敷けば左上は空白）。Chrome を撮らんまま「全部出たら勝ち」で進んどったら、
正しい挙動を追いかけて壊しとった。`tools/geom/cmp.py` と同じ要領で
`chrome.exe --headless --screenshot` を先に一枚撮ること。

**検体は「区別がつく」ものを選ぶ。** 同じ 2026-09-05 の件で、8x8 の単色 PNG は
敷き詰めが壊れとっても箱が埋まって見えたので「塗れとる」と誤読しかけた。
中身に構造のある画像（実物の SVG）を並べて初めて、断片が隅に出て分かった。
**単色の検体は敷き詰めの検証に使えん。**

**この文書に書いてある見立ても疑うこと。** 2026-09-10 の夜に三つ外れた。
どれも「もっともらしいが測っとらん」書き方をしとった:

- 「Chrome との 1〜3px の字幅差はだいたい font-size が整数やから」→ **違う**。
  字送りに 4px の下限がかかっとったのが主因で、**整数の大きさでも外れとった**
  （10px で +6、16px で 0）。丸めの形やのうて下限の形やった。
- 「Chrome はこの機械で `sans-serif` に日本語既定を当てる」→ **違う**。
  Chrome headless は Noto Sans を使う（幅 129 でぴったり一致）。
  日本語の顔はどれも 129 やない。locale の話やのうて参照環境の性質。
- 「`tablew.html` が 0/6」→ **表の話やなかった**。幅も高さも六形とも
  合うとって、外れは y だけ、しかも 1 行ごとに +1 ずつ積もる形。
  真因は行の高さの丸め方（下記）。

2026-09-18 にもう一つ外れた（Mac 側の Claude の見立て、Windows 側で測った）:

- 「Noah's Ark 条項が無いと実頁で要素が爆発しとるはず。適合率より
  そっちが本命」→ **測った範囲では実害は無かった**。合成頁
  （`tools/geom/noah.html`、`<font size=4>` を閉じずに 50 個）では
  516 → 99 要素、`<p>` ごとの font が 50 → 3 で Chrome（96）と段落内は
  一致した。理論は正しい。けど実頁は abehiroshi top/menu・HN・lobste.rs
  の四枚とも**修正前から Chrome と同数**（161 / 103 / 810 / 827）で、
  修正後も一つも動かんかった。abehiroshi の `<font>` は全部閉じてある。
  「古い手書き HTML は閉じん」は、この四枚に関しては当たらんかった。
  この修正の実の取り分は html5lib +3 件と仕様準拠で、要素爆発の抑制は
  **検体でしか見えとらん**。

**外れとる数字を見たら、その頁が何を測っとるかを疑う前に、
どの軸が外れとるかを見ること。** 三つとも「合っとる軸」と「外れとる軸」を
分けた瞬間に別の話やと分かった。

**積もる 1px は行の高さを疑う。** 一行あたり 1px は目に見えんが、
二千行で 246px になる（Wikipedia の実測）。行の高さは
ascent・descent・line gap を**別々に丸めてから足す**。合計を一度だけ
丸めると、三つの端数が繰り上がる大きさでだけ 1 ずれる。
`tools/geom/lineheight.html` が 8〜40px を全部並べてある。

**Chrome を任意の瞬間で止めて比べられる。** アニメーションは
**負の遅れ ＋ `paused`** で止まる。負の遅れはその分進んだ位置から始める
という意味で、`paused` がそこで時計を止める。どちらのブラウザも走らせん
まま同じ瞬間を訊けるので、静止画の照合でアニメが比べられる。
`tools/geom/anim2.html` / `anim3.html` がこれ。

**顔の身元は幅で分かる。** 総称（`sans-serif` 等）が実際どの顔に当たるかは、
候補の顔を名前で並べて幅を測り、総称の幅と突き合わせれば特定できる。
名前で指定した顔は Chrome と元から一致しとるので、幅がそのまま身元になる。
`tools/geom/g6b.html` がこれ。

**実頁を掃くと、版面やのうて要素の意味の穴が出る。**
`--dump-styled` は「重なっとる文字の組」を数えて出す。それを実頁十枚ほどに
かけて、多い頁から見る。2026-09-10 の夜はこれで二つ出た:

- lobste.rs が **1250 組**。同じ文字が同じ座標に何度も、という形やった。
  `<details>` が閉じとるのに中身を畳んどらんかったせいで、記事ごとの
  折り畳みが二十五個ぶん積み上がっとった。
- そこから `hidden` 属性も丸ごと見とらんことが分かった。

**「同じ文字が同じ座標に」は版面の穴やない。** 隠すべきものを出しとる、
つまり要素の意味を読んどらんという形や。**違う**文字が重なっとるのは
版面の穴。この二つを先に分けること。

**追うとった件の途中で作った検体が、別の穴を出すことがある。**
theguardian.com の重なりを追うために横に流れる帯の検体を作ったら、
帯自体は正しかったが、flex の子の指定幅に padding が足されとらんことと、
flex の行が巻物の場所を取っとらんことが出た。**追い切れんかった回でも
検体は残す。**

**エンジンの速さを測るなら受け手の幅を変えて測る。** 呼び出し 20,000 回を
receiver の own property 数 1 / 20 / 100 / 400 で回すと、O(幅) の処理が
紛れ込んどるかどうかが一発で出る。平らなら正常。2026-09-04 の
`describe_receiver` はこれで見つけた（幅400で 21 倍）。

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
- **ループ予算（fuel）はターンごと**（`vm.rs` の `TURN_FUEL`、`refill_turn_fuel`）
  後退ジャンプ 1,000,000 回で `VmError::InfiniteLoop`。これを積み直すのは
  スクリプト実行・task callback・rAF の一周・`fire_dom_event` のそれぞれの頭。
  以前は `execute_with_this` でしか積まんかったので、最後の `<script>` の
  残りを頁の生涯の callback 全部で分け合っとった。一こま200回のアニメでも
  60fps で二分で尽きる。**頁の生涯の割り当てに戻したらあかん。**
- **event loop から逃げた例外は握り潰さん**（`vm.rs` の `report_job_error`）
  timer / rAF / microtask には返す先の呼び手がおらん。頁の console に Error として
  流し（snapshot に乗る）、`TOBIRA_DEBUG_CONSOLE` なら backtrace 付きで stderr にも出す。
  `take_job_errors` で host が引き取る。`run_due_jobs` が返すのは
  **走った**仕事の数で、投げた callback は数えん。
- **背景画像は命令をタイルごとに増やさん**（`gui.rs` の `draw_tiled_image`）
  命令が持つのは画像と `repeat_x` / `repeat_y` と一枚のタイルの寸法だけで、
  塗るときに箱を画素単位で走査して元画像から拾う。1px の画像を 1920x1080 に
  敷いても命令は 1 個。タイルごとに命令を吐く作りにすると 200 万命令になって
  描画も `--dump-styled` での診断も詰む。**そこへ戻したらあかん。**
- **繰り返しの両軸は一箇所から出す**（`css.rs` の `BackgroundRepeat::axes()`）
  `no-repeat` は「両軸とも回数 1」、`repeat-x` は「縦が 1」として同じ経路を
  通る。片軸だけ別実装にすると必ず片方がズレる。
- **周回ごとの束縛は「宣言された枠 ∩ 捕獲された枠」だけ**
  （`compiler/statements.rs` の `begin_loop_body` / `end_loop_body`）
  ループ変数は元から `FreshenLocal` をもらっとったが、本体で宣言した
  `let`/`const` はもらっとらんかった（2026-09-04 に修正）。
  `declare_block_scoped` が囲んでいるループ本体に枠を記録し、本体を畳んだ後で
  「その本体で作られた入れ子関数の upvalue descriptor（`is_local=true`）」と
  積を取る。**閉包を作らんループには命令が一つも増えん。**
  `var` は `declare_function_scoped` を通るので記録されん。関数スコープで
  一つの束縛が正しいので、**ここに var を混ぜたら今正しいものが壊れる。**
  `while` と `do-while` だけ `continue` の着地点をずらしてある（この二つは
  continue がループ先頭／条件へ飛ぶので、本体末尾の freshen を飛び越すため）。
- **GC のルートは型で強制する**（`src/engine/trace.rs`）
  `Trace` の実装は列挙を網羅 match で書き（`_ =>` 禁止）、構造体は全項目分解で
  書く（`..` 禁止）。`Vm::trace_roots` は `Vm` の 61 フィールドを一つ残らず
  並べてあり、参照を持たんものは `_` に理由付きで束ねてある。**フィールドや
  variant を足すとビルドが止まる。** 止まったらそれが仕掛けの作動や。
  辿るか、`_` にして「参照を持たん」と書くか、どちらかを決めること。
- **GC を起こしてええ場所は二箇所だけ**（`vm.rs` の `maybe_collect_garbage`）
  解釈ループの先頭と、event loop のターンの境目。しかも `reentry_depth == 0`
  のときだけ（`call_value_sync` と `invoke_builtin` で上げる）。
  理由: `Array.prototype.map` が結果を Rust の `Vec` に溜めながら JS を
  呼び戻す最中は、その `Vec` の中身をどのルートからも辿れん。
  **allocate の中で走らせたらあかん。**
- **`callables` と `string_cache` は弱い、DOM 側の表は強い**
  前者を強くすると全ての関数と全ての intern 済み文字列が不死になって、
  回収するものがほぼ無うなる。後者（`event_listeners`・`node_wrappers`・
  `mutation_observers`・`dom_interface_ctors`）を弱くすると、頁が要素に付けた
  expando が二回の読みの間で消える。寿命は DOM 側が持つ。
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
- **`src/html.rs` を python で書き戻すとき `newline=""` を付けん** — このファイルは
  NUL 入りの文字列リテラルを持つので **git に binary 扱いされる**。`core.autocrlf` の
  正規化が効かんため、書き込みで改行が LF から CRLF に変わると **全 4600 行が差分**に
  なる。20 行の変更が「4631 挿入 4608 削除」になった（2026-09-10、amend で直した）。
  binary 扱いかどうかは `grep` が "Binary file ... matches" と言うので分かる。
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
- **`background-size` の百分率**（`background-size: 50%`）は未対応で `auto`
  に落ちる。箱に対する割合なので解くのはレイアウト時になる。長さと
  `cover` / `contain` は効く。
- **背景の層は 1 枚目しか塗らん**（`css.rs` の `apply_background_shorthand`）。
  `background: url(a), url(b)` は a だけ。CSS では 1 枚目が手前なので、
  だいたいの頁では見た目が合う。
- **表のセル背景を二度塗っとる**。半透明を重ねると濃くなる。
- **差分 restyle** は既定 ON（`TOBIRA_INCREMENTAL_RESTYLE`）。
  `docs/JS_ROADMAP.md` の Phase5 に「blocker」と書いてあるのは古い記述。
- **文字列は intern される**（`vm.rs` の `make_string_value`）。GC が入って
  `string_cache` は弱表になったので漏れはせんが、cache は key として String を
  もう一部持っとる（intern 済みの分が 2 倍）。
  合わせて `string_text` が**毎回全長を clone** する。`if (s)` の真偽判定でもコピーが走る。
  `s.length` は `chars().count()`、`s[i]` は `chars().nth(i)` で両方 O(n)。
  ここは `Value::String` の表現に手が入るので独立した回が要る。
- **`Map` / `Set` は `Vec` の線形走査**（`value.rs` の `ObjectKind::Map`）。
  n=500 で 28ms、1000 で 104ms、2000 で 427ms ときれいに O(n²)。
- **GC は世界を止める単純な mark-sweep**。増分でも世代別でもない。今は
  収集一回が短い（実頁で目に見える停止は出とらん）が、live 集合が大きい頁で
  引っかかるようなら最初に手を入れるのはここ。
- **収集の閾値は「生き残りの 2 倍、最低 4096 枠 / 1MB」**（`vm.rs` の
  `rearm_gc_threshold`）。つまり最大でそのぶんのゴミは常に抱えとる。
  `s += 'x'` が n によらず 0.1〜1.7MB に収まるのはこの閾値のせい。
- **inline cache も hidden class も無い**。`PropertyKey::String(String)` なので
  `a.b` のたびに String を確保する。ディスパッチループは `Opcode` を二度 clone
  しとる（`Opcode` の中身は全部スカラーなので `Copy` を derive できるはず）。
  素の速度は約 15M opcode/秒。

## 次の一手（優先順）

2026-09-10 の夜に前の一覧（1〜10）は一巡し、そのあと実頁を掃いて出た
ぶんも足してある。各項目の数字は「今こうなっとる」という実測。

1. **theguardian.com の 37,871 組の重なり** — **未解明。今いちばん大きい
   実害。** 左上に細い柱ができて見出しが積み上がる（screenshot で見える）。
   潰した筋（全部この夜に確かめて外した）:
   - **JSON の漏れやない。** `<gu-island props="{…}">` の属性値は健全で、
     生の `>` も `<` も無い。積まれとる文字は頁の本文として正しい
   - `hidden` でも `<details>` でもない（両方この夜に直したが数字は動かん）
   - 横に流れる帯そのものでもない（`tools/geom/carousel.html` が示した）
   - **読み上げ専用の文字でもない。** `position:absolute; width:1px;
     overflow:hidden; clip:…` は tobira もちゃんと clip しとる。
     span を入れた頁と入れん頁で描かれる run 数が同じ
     （`tools/geom/visuallyhidden.html`）

   計器で分かっとること:
   - 内容幅が 1 になっとる要素は**深さ 8** から始まる。それより浅い親は正常。
     つまり深さ 7 あたりの親が 0 を配っとる
   - 数が多いのは `dcr-1shbixy`（読み上げ専用、これは正しく 1px）が 3312、
     `<h3 class="card-headline">` が 2760。**後者が本命**
   - `dcr-t197d6` は `display:none` と `display:block` の**両方**が頁の
     CSS に出てくる。どちらが勝つかは未確認

   計器の入れ方: `layout_block_element` の終わりで `content_width` が
   小さい要素を tag/class/深さ付きで出す。`--dump-styled` は浅いので
   これが一番速い。

2. **`font-size` の端数** — `LengthValue` が u32 なので `10pt`（13.333px）が
   13px になる。**縦への影響はほぼ無いと測って分かった**
   （`tools/geom/lineheight2.html` が 7/14 で、行の高さが変わるのは
   七形のうち一形だけ）。効くのは**横**で、`fsize.html` の残り 9 件と
   `overflow.html` の残り 7 件が全部これ。語の幅が 2〜4% ずれる。

   **規模を測った上で着手せんかった。** `font_size_px` の読みが 141 箇所、
   測定 API の呼び出しが 48 箇所で、一つ一つ「箱の勘定に使う整数か、
   測定に使う実寸か」の判断が要る。**やるなら設計を先に決めること。**
   案は、`ComputedStyle` が百分の一 px を持ち、`font_size_px()` を丸めた
   値を返すメソッドにして、font.rs の公開関数（`text_width_px` /
   `line_height_px` / `content_height_px` / `glyph_advance_px` /
   `draw_text`）は百分の一 px を取る。**描画と測定が同じ値を使うこと**が
   条件（違うと字が箱からはみ出す）。

3. **インライン矩形の残り（g2 / g4 / sup）** — `g4` が 11/14、`g2` が 14/22、
   `sup` が 2/10。**2026-09-10 に三度直そうとして三度悪化させた**
   （10/14 → 6〜8/14）。原因は `emit_line_impl` で閉じ印が自分の run より
   先に処理される構造で、その順序に他の計算がぶら下がっとる。
   **部分的に触ると壊れる。走査順を設計からやり直すこと。**
   詳しくは `tools/geom/README.md` の sup.html の節。

4. **絶対配置されたインラインの静的位置** — 流れの中の位置やのうて親の
   内容の左端になる。Chrome は「その前の語の後ろ」に置く
   （`tools/geom/visuallyhidden.html` で 151 に対して 8）。
   1x1 の読み上げ専用なら実害は無いが、位置決めした吹き出しや badge では
   効く。**直すなら行の組み立て段階**で、`layout.rs:5536` が今
   `static_x` に 0 を渡しとる。断片を集める時点では x が決まっとらんので、
   3 番と同じ「走査順の設計」の話になる。

5. **版面の幅から頁の巻物を引く** — Chrome は窓 1280 に対して
   **1264** を版面に使う（16px ぶん）。2026-09-10 に実測した:
   `#fixed{position:fixed;right:5px;width:20px}` が Chrome で x=1239、
   tobira で 1255。1239+20+5 = 1264。`g1` の `vw` の外れも同じ 16px。

   **頁全体の幅が動く話なので、照合頁 26 枚の基準が全部変わる。**
   一手でやって全部測り直すこと。`position:fixed` だけ先に直すのは
   筋が悪い（頁の中身は 1280 のまま巻物の下に潜る）。
   描画側の巻物幅が今 10px なのを 16px に寄せるかという意匠の判断も要る。

6. **縦の巻物の場所** — 横は入った（`1fa56de` と `e88da22`）。縦は
   `overflow: scroll` なら無条件なので**子を配置する前に幅から引ける**が、
   `auto` は「縦にはみ出すか」を先に知らなあかんので二段レイアウトが要る。

7. **clip がまだ軸ごとやない** — `overflow_x` / `overflow_y` は入った
   （`a056c38`）が、clip の判定は潰した一つの値で見とる。
   **実害を測った。無い。** `tools/geom/clipaxis.html` が 8/9 で、
   間違うて clip した行も、clip せんかった行も出とらん。
   唯一の差は「`overflow-x:hidden` の箱の子が Chrome では 105 幅」で、
   これは縦巻物が幅を食うぶん — 6 番の二段レイアウトの話であって
   軸ごとの clip の話やない。**急がんでええ、と測って言える。**

8. **`<summary>` の無い `<details>`** — ブラウザは既定の見出しを出す。
   `tools/geom/details.html` の d4 がこれで、Chrome 24px に対して 0。

9. **html5lib 残り 16 件** — 98.7%。tests23 の `<font>` の入れ子 3 件は
   2026-09-18 に Noah's Ark 条項（同型の活性整形要素は marker 以後 3 つまで、
   `html.rs` の `push_formatting`）で片付いた。残りの塊は
   `<col>`/`<colgroup>` の表構造（tests1 に 2）。後者は過去に
   1184 → 1115 の退化を出した領域と地続き。残りは文書の頭の空白
   （doctype01・tests15・tests19）と、表の中のフォームの里親付け。
   **頭の空白は一度試して増減ゼロやった**ので、あの二例がどの状態で
   文字を受け取っとるかを先に測ること。

10. **`Map` / `Set` の索引化** — 二乗であることは測った（n を倍にすると
   時間が約 3.7 倍、素のオブジェクトはきっちり 2 倍。n=4000 で
   Map 67ms 対オブジェクト 8ms）。**やらんと決めた。** `ObjectKind` の形を
   変える 43 箇所の話で、GC のときと同じ「登録漏れに気づける仕掛け」が
   要る領域。独立した回でやること。

11. **`Range.getBoundingClientRect` が無い** — `fsize.html` を最初その形で
   書いて全行 0 が返った。文字の run の幾何を Range に繋ぐ話。

12. **文字列が符号点で数えられとる** — `'😀'.length` が 1（JS は 2）、
   `charCodeAt(0)` が 128512（JS は 55357）。一貫はしとるので `s[i]` の
   往復は壊れとらんが、surrogate を触る polyfill が合わん。
   **直すなら添字も全部まとめて。**

13. **楕円の角** — `border-radius: 10px / 30px`。角は真円一つしか
   持っとらん。`radius2.html` の r4 と `radius.html` の r5。

14. **総称 `monospace` / `serif` の当て先** — Chrome headless は
   monospace 120 / serif 137、tobira は Consolas 132 / Georgia 124。
   **当てれば数字は上がるが、やらんかった。** Consolas のほうが code の
   見た目として明らかにええ。


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
| `src/engine/` | 自作 JS エンジン（コンパイラ + VM + GC）。boa は parser front-end のみ。手書きの字句解析器は無い（`lexer.rs` は死んどったので 2026-09-04 に削除） |
| `src/engine/trace.rs` | GC の到達可能性。**ここが壊れると事故が遠くで出る**。書き方の決まりは冒頭のコメント |
| `src/engine/heap.rs` | arena、世代番号、掃除、`TOBIRA_GC_VERIFY` の印 |
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

### 2026-09-18 - Claude (Windows、Mac 側の Claude と往復しながら)

Mac 側が GitHub と比べて「55 コミット未 push」を見つけたところから。
push して、入口の `CLAUDE.md` を足して、html5lib の残りで一番固まっとった
tests23（2/5）を Mac 側が Noah's Ark 条項の未実装と読み、こちらで確かめて直した。

- 落ちとる 3 件が「減る側」、通っとる 2 件が「残す側」（属性違い・marker 越し）で
  見立てどおり。積む場所は `html.rs` の一箇所だけ、属性は `BTreeMap` なので
  順序非依存の比較が `==` で済んだ。再構築（`reconstruct_formatting`）は
  その場で置き換える経路で push を通らんことも確認した。
- 数字: テスト 1176 / 0（動かず）、html5lib 1210 → **1213/1229 (98.7%)**。
  adoption01/02・tricky01 は動かず。HN と Wikipedia を一枚ずつ撮って崩れなし。
- **要素爆発の実測**は上の「見立ても疑え」の節。検体では 516 → 99、実頁四枚では
  差ゼロ。測る前に「本命」と言うとった見立ては外れ。
- 途中で見つけた別の穴（未修正）: `<script>` の中身に活性整形要素が
  再構築されて巻き付く。`noah.html` の stray 行で `script>font` と出る。
  Chrome は出さん。raw text の挿入で `reconstruct_formatting` を呼んどる筋。
- `--cli` で外部 `<script src>` が 404（HTML が返る）やと parse error になり、
  **それ以降の inline script が走らん**ように見えた（HN の保存頁で再現）。
  Chrome は外部の失敗を無視して次へ進む。要確認。

### 2026-09-10 - Claude (自走ループ二晩目: 8535535..c16844e, 23 コミット)

一晩目と同じ「次の一手を上から順に、09:00 まで自分で回せ」。一覧を
一巡し、そのあと実頁を掃いた。数字は 1154 → **1176 通過**、
html5lib 97.3% → **98.5%**、照合頁は 6 枚 → 26 枚。
何をどう直したかは各コミットに書いた。ここには**やり方の話だけ**残す。

**前半（一覧を一巡）と後半（実頁を掃く）で、出てくる穴の種類が違うた。**
前半は寸法のズレ、後半は要素の意味の穴。掃くほうが一件あたり大きい:
lobste.rs の 1250 組の重なりは `<details>` を畳んどらんかっただけやし、
github が開けんかったのは版面の無限再帰やった。

**この晩いちばん効いたやり方: 外れとる「軸」を見る**

三回とも、外れとる数字を見て「その頁が測っとる物」を疑わず、
**どの軸が外れとるか**を先に分けたら別の話やと分かった。

- `tablew.html` 0/6 → 幅と高さは六形とも一致、外れは y だけ、しかも
  1 行ごとに +1 ずつ積もる。表の話やのうて**行の高さの丸め方**やった。
  直したら 6/6。実頁で Wikipedia が 30072 → 29826（一行 1px が二千行）。
- `fsize.html` → **整数の大きさでも外れとった**。丸めの形やのうて
  **下限の形**（10px で +6、12px で +3、16px で 0）。字送りに 4px の
  下限がかかっとった。外したら 12px と 14px が完全一致。
- `overflow.html` 5/17 → 高さの差と幅の差が別々の原因。巻物の場所を
  取ったら 10/17、残りは全部 font-size の端数、と割り切れた。

**HANDOFF に書いてある見立てを三つ訂正した。** どれも
「もっともらしいが測っとらん」書き方をしとった。詳細は「測り方・見方」へ移した。
**この文書の断定は、次に触る人が測り直す前提で読むこと。**

**後半のやり方: 実頁を掃く**

`--dump-styled` の重なり報告を実頁十枚ほどにかけて、多い頁から見る。
**「同じ文字が同じ座標に」と「違う文字が重なっとる」を先に分ける。**
前者は隠すべきものを出しとる＝要素の意味の穴、後者は版面の穴。
lobste.rs は前者（`<details>`）、theguardian.com は後者で、
そちらは追い切れんかった。

追う途中で作った検体が別の穴を出すこともある。Guardian のために
横に流れる帯の検体を作ったら、帯自体は正しかったが flex の子の
指定幅と巻物の場所の二件が出た。**追い切れんかった回でも検体は残す。**

**Chrome を止めて比べる手**

アニメーションは負の遅れ ＋ `paused` で任意の瞬間に固定できる。
どちらのブラウザも走らせんまま同じ瞬間を訊けるので、静止画の照合で
アニメが比べられるようになった（`anim2.html` 9/9、`anim3.html` 14/14）。
顔の身元は幅で特定できる（`g6b.html`）。**照合の道具が増えた分、
次の回は「測ってから決める」がやりやすい。**

**やらんと決めたことと、その理由**

- `font-size` の端数。読み 141・呼び出し 48 箇所で、一つ一つ単位の判断が
  要る。**残り時間で測り直しまで終わらんと判断した。** 単位の変更が
  中途半端な状態が一番危ない。設計案は「次の一手」1 番に書いた。
- `Map` / `Set` の索引化。二乗であることは測った上で、`ObjectKind` の形を
  変える 43 箇所の話やから独立した回に回した。
- 総称 `monospace` を MS Gothic に当てる件。**照合の数字は上がるが
  見た目が悪うなる**ので入れとらん。
- 「本文前の先頭空白を捨てる」html5lib の 2 件。実装してみたら**増減
  ゼロ**やった（見当てた条件が成り立っとらん）ので入れずに戻した。
- 素の閉じタグでも引用符の中の `>` を無視する件。仕様上は正しいが
  **増減ゼロ**やったので戻した。全ての閉じタグが通る道やから、
  確かめられんものは触らんほうがええ。

**しくじり**

- **`printf` にコミット文を通してメッセージが切れた。** `(98.5%)` の `%` を
  書式指定と読まれた。amend で直した。**コミット文は必ずファイル経由で。**
- **巻物の場所を、高さの変数にだけ足した。** 箱は高うなったが**流れが
  動かん**かった（高さも次の物の位置も cursor から決まる）。巻物の中に
  巻物がある形で内側の伸びを外側が数えられん。計器を入れて
  「reserve=true やのに高さが 29」を見るまで気づかんかった。
  cursor を進める形に直した。
- **`html.rs` の行末を CRLF と思い込んだ。** `od` の見方を間違えた。
  実際は LF。確かめてから直したので事故にはならんかったが、
  **前の晩の自分の註釈を確かめずに信じかけた**。
- ヒアドキュメントで `\u{000C}` を含む Rust を流して壊した。
  **二晩続けて同じ罠**。Write でファイルに書くこと。

**github.com が開けんかった件**

`thread 'tobira-engine-js' has overflowed its stack` の一行だけ出て
プロセスごと落ちとった。**JS は一行も走っとらんかった。**
段階的に潰して（boa パーサ／モジュール単体／DOM の深さ／CSS 解析）、
版面計算の中の `inline-flex` の採寸が**自分自身を呼び直す無限再帰**に
行き着いた。深いのやのうて底が無いので、スタックを 32MB → 512MB に
上げても落ちたままやった、というのが切り分けの決め手。

### 2026-09-10 - Claude (自走ループ一晩: d1bd3cd..32e7f58)

「HANDOFF の次の一手を上から順に、09:00 まで自分で回せ」で走った回。
一件ずつ Chrome と突き合わせ → テスト → 実頁 screenshot → コミット、を繰り返した。
細かい経緯は各コミットに書いた。ここには**やり方の話だけ**残す。

**効いたやり方**

- **着手前に Chrome を撮る。** 今夜これで三回救われた。`background-repeat` の
  合成頁は 7 行のうち 1 行が「空白が正解」やったし、表の幅も 6 形のうち 4 形は
  元から合っとった。撮らずに「全部直す」で入っとったら正しい挙動を壊しとった。
- **落ちとる宣言から探す。** `TOBIRA_DEBUG_CSS=1 --dump-styled` の集計を実頁
  三枚で回して上から潰す。`pt` 未対応も `border-radius` の複数値もこれで出た。
  症状から探すより速い。
- **網羅した検体を作る。** ループ本体の束縛は 22 形、単位は 9 形並べた。
  22 形のほうは `var` が 222 で**正しい**ことに気づけて、素朴な修正が今
  正しいものを壊すのを止められた。単位のほうは `12pt` と `1pc` がちょうど
  16px で「未対応やのに一致する」罠を可視化した。
- **一歩ごとに全部回す。** テスト・html5lib・幾何照合 g1..g6・実頁 3〜5 枚。
  表を触った回はこれで安心して入れられた（あの領域は過去に 1184 → 1115 を
  出しとる）。

**しくじったやり方**

- **インライン矩形に三度突っ込んで三度悪化させた**（g4 10/14 → 6〜8/14）。
  部分的に触れる場所やないと分かるまでに一時間近く溶かした。
  三度目で止めて記録に回したのは正しかったが、**二度目で止めるべきやった。**
- **自分で書いた註釈を踏んだ。** 「ヒアドキュメントでパッチを流すな、必ず
  Write でファイルに書け」と HANDOFF に書いてあるのに流して壊した。
- **`html.rs` を python で書き戻して 4600 行の差分を作った。** binary 扱いで
  `core.autocrlf` が効かん。amend で直した。両方「試してダメやった方法」に足した。

**判断して**やらなかった**こと**

- 巻物の 16px 確保。二段レイアウトと、描画側の巻物幅を変えるかという意匠の
  判断が要る。Chrome の実測だけ `tools/geom/scrollbar.html` に残した。
- 総称 font family の locale 対応。`g6` の落ちはバグやのうてこれ、と特定だけした。
- `<col>` の表構造（html5lib 3 件）。過去の大退化と同じ領域で、割に合わん。

### 2026-09-05 - Claude (background-size に長さを持たせる: 18b94c4)

前回「`background-repeat: repeat` が敷き詰めをせん」と書いたが、**それも
まだ半分やった**。敷き詰めは動いとって、原寸で敷くのが問題。真因は
`background-size` の長さ指定が丸ごと落ちとったこと。詳しくはコミット。

- 一日で同じ件について**二回、原因を言い直した**。「箱が無い」→「敷き詰めが
  未実装」→「`background-size` の長さが落ちとる」。毎回、次の検体を作るまでは
  もっともらしかった。**症状の説明がついた時点で止めると間違う。**
- 二回目から抜けられたのは、SVG と PNG を並べたから。単色やと壊れとっても
  埋まって見える。三回目に着地できたのは、`arrow3.html` で
  `background-repeat` だけを動かしたら「repeat 系が全滅、no-repeat だけ通る」
  と出て、そこから「no-repeat の経路は何をしとるんや」と逆から見たから。
- **Chrome を先に撮ったのが効いた。** 7 行のうち 1 行（r4）は空白が正解で、
  それを知らんまま「全部出たら勝ち」で進んどったら、正しい挙動を追いかけて
  壊しとった。期待値は再現頁の中に書き込んである。
- ついでに `background-position` の初期値が 50 やった（CSS は 0% 0%）。
  `object-position` から写した間違いで、背景が常に引き伸ばされとったから
  今まで見えんかった。`object_position_x/y` は別の欄なので触っとらん。
- 退化の確認は Chrome との幾何照合（g1..g6 が README と全部同じ）と実頁 4 枚。

### 2026-09-04 - Claude (HN の矢印 → background-repeat: 4448441)

矢印を追ったら、はるかに広い穴に行き当たった。詳しくはコミットと
`tools/geom/README.md`。ここには外した推測を残す。**全部外れとる。**

- 「インラインの中のブロック（`<a>` の中の `<div>`）が怪しい」— 外れ。
  同じ入れ子でも背景が色なら四角が出る。
- 「中身が空やから箱が消えとる」— 外れ。箱はある。
- 「表や `<center>` が効いとる」— 外れ。表の外でも同じ。
- 「多層の短縮形が壊れとる」— **半分だけ当たり**。短縮形は 1 層目しか
  読まんので、HN のように `no-repeat` が 2 層目にあると効かず `repeat` の
  ままになる。これ自体は CSS 的に正しい挙動で、落ちとるのは repeat の塗り。
  ここで止めとったら間違った場所を直しとった。

- **一回はっきり誤診した。** `--dump-styled | grep votearrow` が空やったので
  「箱が生成されとらん」と判断して、前回の HANDOFF にもそう書いた。実際は
  dump が既定で浅く、木が途中で切れとっただけ。`TOBIRA_DUMP_DEPTH` を上げたら
  箱も `bgimg=triangle.svg` も普通に在った。**出力が無いことを、物が無いことの
  証拠にしたらあかん。** 道具の既定値を先に確かめること。
- 続けて `bg=` が background-color しか出しとらんことにも気づいたので、
  `bgimg=` と `size=` を dump に足した。見た目が背景画像だけの箱は、
  これが無いと未設定の箱と見分けがつかん。
- 軸を一つずつ外した頁を並べて撮る、というやり方が効いた。形の軸（表・
  `<center>`・`<a>`・空要素）を全部外してもまだ落ちる、というのが分かった
  時点で、探す場所がレイアウトから塗りへ移った。
- 最後、SVG と PNG を並べたのが決め手。PNG（単色）やと repeat でも埋まって
  見えるので「塗られとる」と読めてまう。**単色の検体は敷き詰めの検証に使えん。**

### 2026-09-04 - Claude (ループ本体の束縛: 6b879fa)

- GC の回で踏んだ closure バグを直した。詳しくはコミットに書いた。
- **この種の壊れ方が一番たちが悪い**、というのがこの回の教訓。投げん、
  console にも出ん、頁は表示される。ただ listener が全部最後の要素を見る。
  「未捕捉 JS エラー 0」を合格基準にしとると素通りする。
  これまで「一致に近い」と判定してきた頁も、当たり判定は確かめとらん。
- 直す前に 22 形の検体を作ったのが効いた。`var n=i` が 222、`for(var i)` が
  333 で**正しい**（関数スコープ）ことに気づけたのはこれのおかげで、
  素朴に「本体で確保された枠を全部新しくする」と書いとったら、今正しい
  ものを壊しとった。**直す前に「変えたらあかんもの」を先に固定すること。**
- 費用の心配（閉包を作らんループが毎周払うのでは）は、捕獲の情報が
  upvalue descriptor にそのまま在ったので回避できた。時間やのうて
  **命令数**で確かめてある（閉包なしのループは +0）。
- HN の投票矢印はこれでは直らんかった。ただ `--dump-styled` に箱が
  一つも出とらんことが分かったので、切り分けは一つ進んだ（次の一手 5 番）。

### 2026-09-04 - Claude (GC を繋ぐ: 35f6ce1)

- 出発点は「文字列の intern をやめれば漏れが止まる」という筋やったが、
  **実験したら間違いやった**。cache を切った版と比べると、`s += 'x'` の
  n²/2 は 1 バイトも減らん（arena 側に積まれとるため）うえ、同じ文字列を
  繰り返し作る頁では 88〜250 倍悪化した。cache は漏れの原因やのうて、
  GC が無いことへの緩和策やった。**着手前に対照を取ったから気づけた。**
  真犯人は GC が無いこと一つ。
- ルート列挙は「今あるもの」より「これから増えるもの」が事故る、という
  指摘を受けて、型で強制する形にした（網羅 match と全項目分解）。
  これは実際に何度も効いた。書いとる最中に `Vm` へフィールドを足すたび
  ビルドが止まって、辿るかどうかを毎回決めさせられた。
- `TOBIRA_GC_VERIFY` は**自分の実装の穴を一つ見つけた**。検証モードだけ
  弱表を掃除しとらんかったせいで誤検知が 4 件出た。検証の仕掛けが自分の嘘で
  鳴るのが一番あかんので、掃除は両モードでやるようにした。
- 途中で**既存の closure バグ**を踏んだ（ループ本体の `const` を捕まえた
  closure が全部同じ束縛を見る）。GC とは無関係で、入れる前のコードでも
  再現する。GC のコミットに混ぜると濁るので別にして、「次の一手」の 7 番に
  積んだ。テストのほうを壊れとらん書き方に変えてある。

### 2026-09-04 - Claude (JS エンジンの棚卸し: 64b87f7..8c081ee)

ユーザーの「JSエンジンどうなってるか突き詰めて」から。コードを読むだけでなく
使い捨ての test を書いて実測した（測ったら消した）。出てきたものは上の
「設計判断」「未確定・仮実装」「次の一手」に散らしてある。ここには経緯だけ。

- 構成は boa_parser → boa_ast → 自作コンパイラ（4,994行）→ 自作 VM（21,060行、
  builtin 499個）。`heap.rs` は arena まで作ってあるが収集器は繋がっとらん。
- **一番でかいのは fuel がターンを跨いで漏れる件やった**。measurement で
  「前段で 950,000 回まわしてから setTimeout を張ると callback が完走せん、
  しかも try/catch にも掛からず戻り値は健康そのもの」を再現できたのが決め手。
  直したうえで `tests/event_loop_fuel.rs` を置いた。補充を外すと 5 本中 3 本落ちる
  ことを確認済み（回帰テストが本当に回帰を捕まえるかは必ず確かめること）。
- 二番目は `describe_receiver`。例外用の文字列を正常系で毎回作っとった。
  **受け手の幅を変えて測る**と一発で出た（幅400で 21 倍）。この測り方は
  「測り方・見方」に書いた。
- ユーザーの指摘で修正の形が変わったのが一箇所。fuel は「補充」だけ入れれば
  ええと思うとったが、握り潰しを直さんと次に本物の無限ループを踏んだとき
  同じ無言停止になり「fuel 直したはずやのに」で捜査が倍こじれる、と。
  そのとおりなので同じコミットに入れた。
- 素の `Vm` でエンジンの穴を判定して一度間違えた。プレリュードは host 側にある。
  同じ罠を踏まんように「測り方・見方」に書いた。

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
