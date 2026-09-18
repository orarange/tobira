# tobira

Rust で一から書いとる web ブラウザ。Chromium も WebView も使わん。

## まず読む

**作業を始める前に必ず `HANDOFF.md` を読むこと。** この CLAUDE.md は入口の案内だけで、
現状・設計判断・試してダメやった方法は全部 HANDOFF.md に書いてある。

読む順:
1. `HANDOFF.md` — いまの状態、設計判断とその理由、試してダメやった方法、未確定・仮実装
2. `git status --short` と `git log --oneline -n 20`
3. `git branch --show-current`

HANDOFF.md と実装が食い違うたら実装が正。気付いた時点で HANDOFF.md を直す。
（例: 2026-09-04 版は「`<isindex>` は追わん」と書いてあったが、09-10 時点では実装済みで
html5lib 適合が 97.3% → 98.0% に上がっとる）

## 作業の締め方

- 機能が仕上がったら `git add .` してコミット
- **コミットしたら push する。ローカルに溜めん。**
  （2026-09-04 から 09-10 の間に 55 コミットが push されず溜まった）
- `cargo build --release` が通ること。警告は dead_code・unreachable pattern・
  deprecated 系のみが正常（実害のあるものは無い）
- `cargo test --release` の通過数を確認する。数え方は HANDOFF.md 参照（`tr -d '\000'` 必須）
- html5lib 適合率も一緒に見る:
  `cargo test --release --bin tobira -- tree_construction_conformance --nocapture`
- 高水準の状態が変わったら HANDOFF.md の `いまの状態` を更新
- 意味のある引き継ぎ・再開があったら `Session Log` に追記

## 数値だけ見るな

`--screenshot` で一枚撮って目で見ること。表が指定幅を無視する件も `<center>` が
表を中央寄せせん件も、撮るまで一つも見つからんかった。修正のたびに一枚撮る。

## CSS を触るとき

`change.md` の CSS Touch Policy に従う。最小限にして、触ったファイルと理由を change.md に記録。

## PR

タイトルにエージェント名を入れる。例: `[Claude] fix CSS calc() precedence`
