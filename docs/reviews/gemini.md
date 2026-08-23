CSSのパースを独自実装するなら、「CSS Syntax Module Level 3」という仕様書が聖書になります。

CSSは「なんとなく正規表現で切り出す」レベルだと、コメントのネストやクォート内のエスケープ、カスタムプロパティ（`--var`）などで必ず破綻するため、W3Cが定義している「字句解析（Tokenization）」**と**「構文解析（Parsing）」のアルゴリズムをそのままコードに落とし込むのが一番の近道です。

### 1. 読むべき最重要ドキュメント

以下のURLが公式の仕様書です。

* **[CSS Syntax Module Level 3 (W3C)](https://www.w3.org/TR/css-syntax-3/)**
* **Section 4: Tokenization**: 文字列を「Token（記号、数値、文字列など）」に分解する手順がステップバイステップで書かれています。
* **Section 5: Parsing**: トークンの列を「ルールセット」や「宣言」という構造体に組み立てるアルゴリズムが定義されています。



---

### 2. 実装のステップ

独自ブラウザ「Tobira」の Rust 実装に組み込む場合、以下の順序で進めることになります。

#### ステップ1: Tokenizer（字句解析機）の作成

まず、入力を `Token` 型の列に変換します。W3Cの仕様は「ステートマシン（状態遷移図）」の形式で書かれているため、Rustの `match` 文と相性が非常に良いです。

* **扱うべきトークン例**: `Ident` (識別子), `Function`, `AtKeyword`, `Hash`, `String`, `Dimension` (単位付き数値), `Whitespace` など。

#### ステップ2: Parser（構文解析機）の作成

トークンのストリームを読み込み、以下の構造（抽象構文木: AST）を作ります。

1. **At-Rules**: `@media`, `@import` など。
2. **Qualified Rules**: `h1 { color: red; }` のような通常のセレクタと宣言ブロック。
3. **Declarations**: `color: red` のようなプロパティ名と値のペア。

#### ステップ3: セレクタのパース

実は、上記の「Syntax Module」には「セレクタ（`.class` や `div > p`）」のパース詳細は書かれていません。それは別の仕様書にあります。

* **[Selectors Level 4](https://www.w3.org/TR/selectors-4/)**

---

### 3. 実装のヒント（Rust開発者向け）

Rustでスクラッチから書く場合、以下のライブラリの内部構造を参考にすると非常に勉強になります。

* **[cssparser (mozilla)](https://github.com/servo/rust-cssparser)**: Firefoxのエンジン（Servo）で使われている超高速パッサ。W3Cのアルゴリズムに忠実です。
* **[lightningcss](https://github.com/parcel-bundler/lightningcss)**: 最近のモダンなツールで、より読みやすい Rust コードで CSS を扱っています。

### 4. 注意点：CSSの「エラー耐性」

W3Cのドキュメントで最も重要なのは「エラーからの回復（Error Recovery）」のセクションです。
CSSは「1箇所間違えたら全部無視」ではなく、「セミコロンまで飛ばして次を読む」といった独特の粘り強さが求められます。仕様書の Section 5 にある「Consume a component value」などのアルゴリズムに従うと、この「ブラウザらしい挙動」が再現できます。

ドキュメントは英語ですが、アルゴリズム部分は「1. Let current input code point be... 2. If...」とプログラミングのロジックそのままに書かれているので、DeepL等を使えばそのままコードに変換できるはずですよ。

CSSパースのどのあたり（セレクタ、値、あるいは全体の構造）で今一番悩んでいますか？