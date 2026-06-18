//! JS 互換正規表現バックエンド。`regress` クレートをラップし、旧 `regex`
//! クレート相当の API(captures/find/split 等)を提供する。look-ahead/
//! look-behind/後方参照など JS 固有機能をサポートする。
use core::ops::Range;
use regress::Regex as RegressRegex;

/// 1 つのマッチ範囲(バイトオフセット)とマッチ文字列を保持する。
pub struct JsMatch {
    start: usize,
    end: usize,
    text: String,
}

impl JsMatch {
    pub fn start(&self) -> usize {
        self.start
    }
    pub fn end(&self) -> usize {
        self.end
    }
    pub fn as_str(&self) -> &str {
        &self.text
    }
}

fn js_match_from_range(range: Range<usize>, text: &str) -> JsMatch {
    JsMatch {
        start: range.start,
        end: range.end,
        text: text[range].to_string(),
    }
}

/// 1 回のマッチのキャプチャ群。`groups[0]` が全体マッチ。
pub struct JsCaptures {
    groups: Vec<Option<JsMatch>>,
    named: Vec<(String, Option<JsMatch>)>,
}

impl JsCaptures {
    /// グループ index 取得(0=全体マッチ)。未参加グループは None。
    pub fn get(&self, index: usize) -> Option<&JsMatch> {
        self.groups.get(index).and_then(|slot| slot.as_ref())
    }
    /// 全体 + キャプチャグループ数(旧 `regex::Captures::len` 互換: 1 + グループ数)。
    pub fn len(&self) -> usize {
        self.groups.len()
    }
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }
    /// 名前付きグループ取得。
    pub fn name(&self, name: &str) -> Option<&JsMatch> {
        self.named
            .iter()
            .find(|(declared, _)| declared == name)
            .and_then(|(_, slot)| slot.as_ref())
    }
    /// 宣言済み名前付きグループを順に列挙((名前, 値))。
    pub fn named_iter(&self) -> impl Iterator<Item = (&str, Option<&JsMatch>)> {
        self.named
            .iter()
            .map(|(name, slot)| (name.as_str(), slot.as_ref()))
    }
}

fn captures_from_match(m: &regress::Match, text: &str) -> JsCaptures {
    let mut groups = Vec::with_capacity(m.captures.len() + 1);
    for index in 0..=m.captures.len() {
        groups.push(m.group(index).map(|range| js_match_from_range(range, text)));
    }
    let named = m
        .named_groups()
        .map(|(name, range): (&str, Option<Range<usize>>)| {
            (
                name.to_string(),
                range.map(|range| js_match_from_range(range, text)),
            )
        })
        .collect();
    JsCaptures { groups, named }
}

/// JS 互換コンパイル済み正規表現。
pub struct JsRegex {
    inner: RegressRegex,
}

impl JsRegex {
    /// source + flags をコンパイル。flags は i/m/s/u/v を解釈、g/y は無視。
    /// 失敗時はエラー文字列を返す。
    pub fn compile(source: &str, flags: &str) -> Result<JsRegex, String> {
        RegressRegex::with_flags(source, flags)
            .map(|inner| JsRegex { inner })
            .map_err(|error: regress::Error| error.to_string())
    }

    pub fn is_match(&self, text: &str) -> bool {
        self.inner.find(text).is_some()
    }

    pub fn find(&self, text: &str) -> Option<JsMatch> {
        self.inner
            .find(text)
            .map(|m: regress::Match| js_match_from_range(m.range(), text))
    }

    pub fn find_at(&self, text: &str, start: usize) -> Option<JsMatch> {
        self.inner
            .find_from(text, start)
            .next()
            .map(|m: regress::Match| js_match_from_range(m.range(), text))
    }

    pub fn find_iter(&self, text: &str) -> Vec<JsMatch> {
        self.inner
            .find_iter(text)
            .map(|m: regress::Match| js_match_from_range(m.range(), text))
            .collect()
    }

    pub fn captures(&self, text: &str) -> Option<JsCaptures> {
        self.inner.find(text).map(|m| captures_from_match(&m, text))
    }

    pub fn captures_at(&self, text: &str, start: usize) -> Option<JsCaptures> {
        self.inner
            .find_from(text, start)
            .next()
            .map(|m| captures_from_match(&m, text))
    }

    pub fn captures_iter(&self, text: &str) -> Vec<JsCaptures> {
        self.inner
            .find_iter(text)
            .map(|m| captures_from_match(&m, text))
            .collect()
    }

    /// 区切り正規表現で分割。マッチ間の部分文字列を返す(キャプチャ挿入なし、
    /// 旧 `regex::Regex::split` 相当の単純動作)。
    pub fn split(&self, text: &str) -> Vec<String> {
        let mut segments = Vec::new();
        let mut last = 0;
        for m in self.inner.find_iter(text) {
            let range = m.range();
            segments.push(text[last..range.start].to_string());
            last = range.end;
        }
        segments.push(text[last..].to_string());
        segments
    }
}
