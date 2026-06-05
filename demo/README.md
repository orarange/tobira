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
