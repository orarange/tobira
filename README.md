# Scratch Browser

Chromium も WebView も使わずに、ブラウザの基本部品を自前で組み立てていくための最小実装です。

今あるもの:

- 手書きの `http://` クライアント
- 手書きの HTML トークナイザと DOM 風ツリー
- 端末向けの超シンプルなテキストレンダラ

まだないもの:

- `https://` 対応
- CSS パーサ / レイアウトエンジン
- JavaScript 実行
- タブ、履歴、アドレスバー
- GUI ウィンドウ描画

## Run

```bash
cargo run -- http://example.com
```

## Structure

- `src/url.rs`
  URL 解析と相対パス解決
- `src/http.rs`
  HTTP 通信、レスポンス解析、chunked 転送、リダイレクト追跡
- `src/html.rs`
  HTML をトークン化して木構造へ変換
- `src/render.rs`
  DOM 風ツリーを端末で読めるテキストへ変換
- `src/main.rs`
  アプリの入口

## Next Steps

1. `https://` を `rustls` などで追加する
2. CSS パーサを作って `display`, `margin`, `font-size` あたりから対応する
3. レイアウトツリーを導入する
4. Win32 API や `winit` ベースで独自ウィンドウを出す
5. クリックとスクロールを扱う

## Why This Shape

“ゼロからブラウザを作る”って言っても、いきなり全部やるのはだいぶやばいです。  
なので最初は「通信する」「読む」「描く」を分離して、あとから本物のブラウザっぽく育てられる形にしています。
