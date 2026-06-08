# エンジン検証デモ

`engine_demo.html` は、自作エンジン(`TOBIRA_ENGINE`)経由で DOM イベントが
動くかを目視確認するためのページです。クリックで表示が変わる要素だけで
構成しています(カウンター / リスト追加 / トグル / CustomEvent / 非同期)。

## 動かし方

このブラウザは `file://` を読めない(HTTP/HTTPS のみ)ので、ローカルの
静的サーバ経由で開きます。

```sh
# 1) demo/ ディレクトリで静的サーバを起動(どれか1つ)
cd demo
python -m http.server 8000
#   or: npx serve -l 8000 .
#   or: php -S localhost:8000

# 2) 別ターミナルで、エンジンを有効にしてブラウザ起動
#    (リポジトリのルートで)
TOBIRA_ENGINE=1 cargo run --release -- http://localhost:8000/engine_demo.html
```

Windows (PowerShell) の場合:

```powershell
$env:TOBIRA_ENGINE = "1"
cargo run --release -- http://localhost:8000/engine_demo.html
```

## 確認ポイント

- **上部バナーが緑**(「✅ 初期スクリプト実行 OK」)→ 初期スクリプトが走った
- **「非同期」欄が緑**(Promise.then / setTimeout)→ 非同期の settle が効いている
- **各ボタンを押して表示が変わる** → DOM イベントがエンジンで配送されている
  - カウンターが増える
  - リストにアイテムが増える
  - 箱が点灯/消灯する
  - CustomEvent の受信メッセージが出る

`TOBIRA_ENGINE` を付けずに起動すれば従来の boa 版になるので、見た目を
比較できます。

## ⚛️ React 18 デモ (`react-demo.html`)

**本物の** react@18.3.1 / react-dom の本番ビルドを、ゼロから書いた JS
エンジンで動かすデモです(`react.production.min.js` /
`react-dom.production.min.js` をそのまま読み込んでいます)。

```sh
cd demo
python -m http.server 8000
# 別ターミナル(リポジトリのルート)で:
TOBIRA_ENGINE=1 cargo run --release -- http://localhost:8000/react-demo.html
```

### 確認ポイント

- **① カウンター** — `useState` + `onClick`。＋/− で数字が増減、リセットで 0。
- **② 制御コンポーネント** — `value` + `onChange`。入力すると下に `echo: …` が即時反映。
- **③ TODO リスト** — key 付きレンダリング。入力して「追加」で行が増え、「削除」で消える。

`useState` / `useEffect` / 合成イベント(`onChange` は内部で `input` イベントに
マッピング)/ 差分再描画 / key 付きリストの再構成まで、React の本番コードが
そのまま走っています。

> この demo が壊れていないことは
> `cargo test --bin tobira react_demo_file -- --nocapture` で自動検証できます
> (実際の `react-demo.html` を読み込み、外部バンドルを inline 展開して、
> マウント→クリック→入力→追加→削除を本物の DOM イベントで駆動します)。
