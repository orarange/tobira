use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceLocation {
    pub line: usize,
    pub column: usize,
    pub offset: usize,
}

impl SourceLocation {
    #[must_use]
    pub const fn new(line: usize, column: usize, offset: usize) -> Self {
        Self {
            line,
            column,
            offset,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceSpan {
    pub start: SourceLocation,
    pub end: SourceLocation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LexGoal {
    Div,
    RegExp,
    TemplateTail,
    HashbangOrRegExp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateChunk {
    pub raw: String,
    pub cooked: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    Eof,
    Identifier(String),
    PrivateIdentifier(String),
    Keyword(&'static str),
    NumericLiteral(String),
    BigIntLiteral(String),
    StringLiteral { raw: String, cooked: String },
    TemplateNoSubstitution(TemplateChunk),
    TemplateHead(TemplateChunk),
    TemplateMiddle(TemplateChunk),
    TemplateTail(TemplateChunk),
    RegularExpression { body: String, flags: String },
    BooleanLiteral(bool),
    NullLiteral,
    Punctuator(&'static str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: SourceSpan,
    pub preceded_by_line_terminator: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexError {
    pub message: String,
    pub location: SourceLocation,
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} at line {}, col {}",
            self.message, self.location.line, self.location.column
        )
    }
}

impl std::error::Error for LexError {}

pub struct Lexer<'a> {
    source: &'a str,
    offset: usize,
    line: usize,
    column: usize,
    saw_any_token: bool,
}

impl<'a> Lexer<'a> {
    #[must_use]
    pub fn new(source: &'a str) -> Self {
        Self {
            source,
            offset: 0,
            line: 1,
            column: 1,
            saw_any_token: false,
        }
    }

    #[must_use]
    pub fn source(&self) -> &'a str {
        self.source
    }

    #[must_use]
    pub fn is_eof(&self) -> bool {
        self.offset >= self.source.len()
    }

    #[must_use]
    pub fn location(&self) -> SourceLocation {
        SourceLocation::new(self.line, self.column, self.offset)
    }

    pub fn tokenize_all(&mut self) -> Result<Vec<Token>, LexError> {
        let mut tokens = Vec::new();
        loop {
            let token = self.next_token(LexGoal::HashbangOrRegExp)?;
            let done = matches!(token.kind, TokenKind::Eof);
            tokens.push(token);
            if done {
                return Ok(tokens);
            }
        }
    }

    pub fn next_token(&mut self, goal: LexGoal) -> Result<Token, LexError> {
        let preceded_by_line_terminator = self.skip_trivia(goal)?;
        let start = self.location();

        if matches!(goal, LexGoal::TemplateTail) {
            return self.lex_template_after_substitution(preceded_by_line_terminator, start);
        }

        if self.is_eof() {
            return Ok(self.make_token(TokenKind::Eof, start, preceded_by_line_terminator));
        }

        if matches!(goal, LexGoal::HashbangOrRegExp)
            && !self.saw_any_token
            && self.starts_with("#!")
        {
            self.skip_hashbang()?;
            return self.next_token(LexGoal::RegExp);
        }

        let ch = self.peek_char().expect("checked eof");
        let token = match ch {
            '"' | '\'' => self.lex_string(ch, preceded_by_line_terminator, start)?,
            '`' => self.lex_template_start(preceded_by_line_terminator, start)?,
            '#' => self.lex_private_identifier(preceded_by_line_terminator, start)?,
            '.' if self
                .peek_next_char()
                .is_some_and(|next| next.is_ascii_digit()) =>
            {
                self.lex_number(preceded_by_line_terminator, start, true)?
            }
            '/' => self.lex_slash(goal, preceded_by_line_terminator, start)?,
            _ if is_identifier_start(ch) => {
                self.lex_identifier_or_keyword(preceded_by_line_terminator, start)?
            }
            _ if ch.is_ascii_digit() => {
                self.lex_number(preceded_by_line_terminator, start, false)?
            }
            _ => self.lex_punctuator(preceded_by_line_terminator, start)?,
        };

        self.saw_any_token = !matches!(token.kind, TokenKind::Eof);
        Ok(token)
    }

    fn lex_identifier_or_keyword(
        &mut self,
        preceded_by_line_terminator: bool,
        start: SourceLocation,
    ) -> Result<Token, LexError> {
        let mut ident = String::new();
        while let Some(ch) = self.peek_char() {
            if is_identifier_part(ch) {
                ident.push(ch);
                self.advance_char();
            } else {
                break;
            }
        }

        let kind = match ident.as_str() {
            "true" => TokenKind::BooleanLiteral(true),
            "false" => TokenKind::BooleanLiteral(false),
            "null" => TokenKind::NullLiteral,
            _ => keyword_for_identifier(&ident)
                .map(TokenKind::Keyword)
                .unwrap_or(TokenKind::Identifier(ident)),
        };
        Ok(self.make_token(kind, start, preceded_by_line_terminator))
    }

    fn lex_private_identifier(
        &mut self,
        preceded_by_line_terminator: bool,
        start: SourceLocation,
    ) -> Result<Token, LexError> {
        self.advance_char();
        let Some(ch) = self.peek_char() else {
            return Err(self.error_here("unterminated private identifier"));
        };
        if !is_identifier_start(ch) {
            return Err(self.error_here("invalid private identifier"));
        }

        let mut ident = String::new();
        while let Some(ch) = self.peek_char() {
            if is_identifier_part(ch) {
                ident.push(ch);
                self.advance_char();
            } else {
                break;
            }
        }

        Ok(self.make_token(
            TokenKind::PrivateIdentifier(ident),
            start,
            preceded_by_line_terminator,
        ))
    }

    fn lex_string(
        &mut self,
        quote: char,
        preceded_by_line_terminator: bool,
        start: SourceLocation,
    ) -> Result<Token, LexError> {
        self.advance_char();
        let content_start = self.offset;
        let mut cooked = String::new();

        loop {
            let Some(ch) = self.peek_char() else {
                return Err(self.error_here("unterminated string literal"));
            };

            if ch == quote {
                let raw = self.source[content_start..self.offset].to_string();
                self.advance_char();
                return Ok(self.make_token(
                    TokenKind::StringLiteral { raw, cooked },
                    start,
                    preceded_by_line_terminator,
                ));
            }

            if is_line_terminator(ch) {
                return Err(self.error_here("unterminated string literal"));
            }

            if ch == '\\' {
                self.advance_char();
                cooked.push_str(&self.read_escape_sequence(false)?);
            } else {
                cooked.push(ch);
                self.advance_char();
            }
        }
    }

    fn lex_number(
        &mut self,
        preceded_by_line_terminator: bool,
        start: SourceLocation,
        started_with_dot: bool,
    ) -> Result<Token, LexError> {
        let number_start = self.offset;
        let mut saw_fraction = started_with_dot;
        let mut saw_exponent = false;

        if started_with_dot {
            self.advance_char();
            self.consume_digits(10);
        } else if self.starts_with("0x") || self.starts_with("0X") {
            self.advance_char();
            self.advance_char();
            if !self.consume_digits(16) {
                return Err(self.error_here("invalid hexadecimal literal"));
            }
        } else if self.starts_with("0o") || self.starts_with("0O") {
            self.advance_char();
            self.advance_char();
            if !self.consume_digits(8) {
                return Err(self.error_here("invalid octal literal"));
            }
        } else if self.starts_with("0b") || self.starts_with("0B") {
            self.advance_char();
            self.advance_char();
            if !self.consume_digits(2) {
                return Err(self.error_here("invalid binary literal"));
            }
        } else {
            self.consume_digits(10);
            if self.peek_char() == Some('.')
                && self.peek_next_char().is_some_and(|ch| ch.is_ascii_digit())
            {
                saw_fraction = true;
                self.advance_char();
                self.consume_digits(10);
            }
            if matches!(self.peek_char(), Some('e' | 'E')) {
                saw_exponent = true;
                self.advance_char();
                if matches!(self.peek_char(), Some('+' | '-')) {
                    self.advance_char();
                }
                if !self.consume_digits(10) {
                    return Err(self.error_here("invalid exponent in numeric literal"));
                }
            }
        }

        let is_bigint = self.peek_char() == Some('n');
        if is_bigint {
            if saw_fraction || saw_exponent {
                return Err(self.error_here("BigInt literal cannot use decimal point or exponent"));
            }
            self.advance_char();
        }

        let raw = self.source[number_start..self.offset].to_string();
        let kind = if is_bigint {
            TokenKind::BigIntLiteral(raw)
        } else {
            TokenKind::NumericLiteral(raw)
        };

        Ok(self.make_token(kind, start, preceded_by_line_terminator))
    }

    fn lex_template_start(
        &mut self,
        preceded_by_line_terminator: bool,
        start: SourceLocation,
    ) -> Result<Token, LexError> {
        self.advance_char();
        let content_start = self.offset;
        let mut cooked = String::new();

        loop {
            let Some(ch) = self.peek_char() else {
                return Err(self.error_here("unterminated template literal"));
            };

            if ch == '`' {
                let raw = self.source[content_start..self.offset].to_string();
                self.advance_char();
                return Ok(self.make_token(
                    TokenKind::TemplateNoSubstitution(TemplateChunk { raw, cooked }),
                    start,
                    preceded_by_line_terminator,
                ));
            }

            if ch == '$' && self.peek_next_char() == Some('{') {
                let raw = self.source[content_start..self.offset].to_string();
                self.advance_char();
                self.advance_char();
                return Ok(self.make_token(
                    TokenKind::TemplateHead(TemplateChunk { raw, cooked }),
                    start,
                    preceded_by_line_terminator,
                ));
            }

            if ch == '\\' {
                self.advance_char();
                cooked.push_str(&self.read_escape_sequence(true)?);
            } else {
                cooked.push(ch);
                self.advance_char();
            }
        }
    }

    fn lex_template_after_substitution(
        &mut self,
        preceded_by_line_terminator: bool,
        start: SourceLocation,
    ) -> Result<Token, LexError> {
        if self.is_eof() {
            return Err(self.error_here("unterminated template literal"));
        }

        let content_start = self.offset;
        let mut cooked = String::new();

        loop {
            let Some(ch) = self.peek_char() else {
                return Err(self.error_here("unterminated template literal"));
            };

            if ch == '`' {
                let raw = self.source[content_start..self.offset].to_string();
                self.advance_char();
                return Ok(self.make_token(
                    TokenKind::TemplateTail(TemplateChunk { raw, cooked }),
                    start,
                    preceded_by_line_terminator,
                ));
            }

            if ch == '$' && self.peek_next_char() == Some('{') {
                let raw = self.source[content_start..self.offset].to_string();
                self.advance_char();
                self.advance_char();
                return Ok(self.make_token(
                    TokenKind::TemplateMiddle(TemplateChunk { raw, cooked }),
                    start,
                    preceded_by_line_terminator,
                ));
            }

            if ch == '\\' {
                self.advance_char();
                cooked.push_str(&self.read_escape_sequence(true)?);
            } else {
                cooked.push(ch);
                self.advance_char();
            }
        }
    }

    fn lex_slash(
        &mut self,
        goal: LexGoal,
        preceded_by_line_terminator: bool,
        start: SourceLocation,
    ) -> Result<Token, LexError> {
        match goal {
            LexGoal::RegExp | LexGoal::HashbangOrRegExp => {
                self.lex_regular_expression(preceded_by_line_terminator, start)
            }
            LexGoal::Div | LexGoal::TemplateTail => {
                self.advance_char();
                let kind = if self.peek_char() == Some('=') {
                    self.advance_char();
                    TokenKind::Punctuator("/=")
                } else {
                    TokenKind::Punctuator("/")
                };
                Ok(self.make_token(kind, start, preceded_by_line_terminator))
            }
        }
    }

    fn lex_regular_expression(
        &mut self,
        preceded_by_line_terminator: bool,
        start: SourceLocation,
    ) -> Result<Token, LexError> {
        self.advance_char();
        let body_start = self.offset;
        let mut in_class = false;
        let mut escaped = false;

        loop {
            let Some(ch) = self.peek_char() else {
                return Err(self.error_here("unterminated regular expression literal"));
            };

            if is_line_terminator(ch) {
                return Err(self.error_here("unterminated regular expression literal"));
            }

            if escaped {
                escaped = false;
                self.advance_char();
                continue;
            }

            match ch {
                '\\' => {
                    escaped = true;
                    self.advance_char();
                }
                '[' => {
                    in_class = true;
                    self.advance_char();
                }
                ']' => {
                    in_class = false;
                    self.advance_char();
                }
                '/' if !in_class => {
                    let body = self.source[body_start..self.offset].to_string();
                    self.advance_char();
                    let flags_start = self.offset;
                    while self.peek_char().is_some_and(is_identifier_part) {
                        self.advance_char();
                    }
                    let flags = self.source[flags_start..self.offset].to_string();
                    return Ok(self.make_token(
                        TokenKind::RegularExpression { body, flags },
                        start,
                        preceded_by_line_terminator,
                    ));
                }
                _ => {
                    self.advance_char();
                }
            }
        }
    }

    fn lex_punctuator(
        &mut self,
        preceded_by_line_terminator: bool,
        start: SourceLocation,
    ) -> Result<Token, LexError> {
        const PUNCTUATORS: &[&str] = &[
            ">>>=", "&&=", "||=", "??=", "**=", "===", "!==", ">>>", "<<=", ">>=", "&&", "||",
            "??", "**", "?.", "++", "--", "<<", ">>", "<=", ">=", "==", "!=", "+=", "-=", "*=",
            "%=", "&=", "|=", "^=", "=>", "...", "{", "}", "(", ")", "[", "]", ".", ";", ",", "<",
            ">", "+", "-", "*", "%", "&", "|", "^", "!", "~", "?", ":", "=", "/",
        ];

        for punctuator in PUNCTUATORS {
            if self.starts_with(punctuator) {
                self.advance_bytes(punctuator.len());
                return Ok(self.make_token(
                    TokenKind::Punctuator(punctuator),
                    start,
                    preceded_by_line_terminator,
                ));
            }
        }

        let found = self.peek_char().unwrap_or('\0');
        Err(self.error_here(format!("unexpected character '{found}'")))
    }

    fn skip_trivia(&mut self, goal: LexGoal) -> Result<bool, LexError> {
        let mut saw_line_terminator = false;

        loop {
            let Some(ch) = self.peek_char() else {
                return Ok(saw_line_terminator);
            };

            if is_js_whitespace(ch) {
                self.advance_char();
                continue;
            }

            if is_line_terminator(ch) {
                saw_line_terminator = true;
                self.advance_char();
                continue;
            }

            if ch == '/' && self.peek_next_char() == Some('/') {
                self.advance_char();
                self.advance_char();
                while let Some(next) = self.peek_char() {
                    if is_line_terminator(next) {
                        break;
                    }
                    self.advance_char();
                }
                continue;
            }

            if ch == '/' && self.peek_next_char() == Some('*') {
                self.advance_char();
                self.advance_char();
                loop {
                    let Some(next) = self.peek_char() else {
                        return Err(self.error_here("unterminated block comment"));
                    };
                    if is_line_terminator(next) {
                        saw_line_terminator = true;
                    }
                    if next == '*' && self.peek_next_char() == Some('/') {
                        self.advance_char();
                        self.advance_char();
                        break;
                    }
                    self.advance_char();
                }
                continue;
            }

            if matches!(goal, LexGoal::HashbangOrRegExp)
                && !self.saw_any_token
                && self.starts_with("#!")
            {
                self.skip_hashbang()?;
                saw_line_terminator = true;
                continue;
            }

            return Ok(saw_line_terminator);
        }
    }

    fn skip_hashbang(&mut self) -> Result<(), LexError> {
        if !self.starts_with("#!") {
            return Ok(());
        }
        while let Some(ch) = self.peek_char() {
            if is_line_terminator(ch) {
                break;
            }
            self.advance_char();
        }
        Ok(())
    }

    fn read_escape_sequence(&mut self, template_mode: bool) -> Result<String, LexError> {
        let Some(ch) = self.peek_char() else {
            return Err(self.error_here("unterminated escape sequence"));
        };

        let cooked = match ch {
            '\'' => {
                self.advance_char();
                "'".to_string()
            }
            '"' => {
                self.advance_char();
                "\"".to_string()
            }
            '\\' => {
                self.advance_char();
                "\\".to_string()
            }
            'b' => {
                self.advance_char();
                "\u{0008}".to_string()
            }
            'f' => {
                self.advance_char();
                "\u{000C}".to_string()
            }
            'n' => {
                self.advance_char();
                "\n".to_string()
            }
            'r' => {
                self.advance_char();
                "\r".to_string()
            }
            't' => {
                self.advance_char();
                "\t".to_string()
            }
            'v' => {
                self.advance_char();
                "\u{000B}".to_string()
            }
            '0' if !matches!(self.peek_next_char(), Some('0'..='9')) => {
                self.advance_char();
                "\0".to_string()
            }
            '\r' | '\n' | '\u{2028}' | '\u{2029}' if !template_mode => {
                self.advance_char();
                String::new()
            }
            'u' => {
                self.advance_char();
                self.read_unicode_escape()?
            }
            'x' => {
                self.advance_char();
                let value = self.read_fixed_hex(2)?;
                let Some(ch) = char::from_u32(value) else {
                    return Err(self.error_here("invalid hex escape sequence"));
                };
                ch.to_string()
            }
            other => {
                self.advance_char();
                other.to_string()
            }
        };

        Ok(cooked)
    }

    fn read_unicode_escape(&mut self) -> Result<String, LexError> {
        if self.peek_char() == Some('{') {
            self.advance_char();
            let start = self.offset;
            while self.peek_char().is_some_and(|ch| ch != '}') {
                let Some(ch) = self.peek_char() else {
                    break;
                };
                if !ch.is_ascii_hexdigit() {
                    return Err(self.error_here("invalid unicode escape sequence"));
                }
                self.advance_char();
            }
            if self.peek_char() != Some('}') {
                return Err(self.error_here("unterminated unicode escape sequence"));
            }
            let digits = &self.source[start..self.offset];
            self.advance_char();
            let value = u32::from_str_radix(digits, 16)
                .map_err(|_| self.error_here("invalid unicode escape sequence"))?;
            let Some(ch) = char::from_u32(value) else {
                return Err(self.error_here("invalid unicode code point"));
            };
            return Ok(ch.to_string());
        }

        let value = self.read_fixed_hex(4)?;
        let Some(ch) = char::from_u32(value) else {
            return Err(self.error_here("invalid unicode escape sequence"));
        };
        Ok(ch.to_string())
    }

    fn read_fixed_hex(&mut self, width: usize) -> Result<u32, LexError> {
        let start = self.offset;
        for _ in 0..width {
            let Some(ch) = self.peek_char() else {
                return Err(self.error_here("unterminated hex escape sequence"));
            };
            if !ch.is_ascii_hexdigit() {
                return Err(self.error_here("invalid hex escape sequence"));
            }
            self.advance_char();
        }
        u32::from_str_radix(&self.source[start..self.offset], 16)
            .map_err(|_| self.error_here("invalid hex escape sequence"))
    }

    fn consume_digits(&mut self, radix: u32) -> bool {
        let mut consumed = false;
        while let Some(ch) = self.peek_char() {
            if ch == '_' {
                self.advance_char();
                continue;
            }
            if ch.is_digit(radix) {
                consumed = true;
                self.advance_char();
            } else {
                break;
            }
        }
        consumed
    }

    fn make_token(
        &self,
        kind: TokenKind,
        start: SourceLocation,
        preceded_by_line_terminator: bool,
    ) -> Token {
        Token {
            kind,
            span: SourceSpan {
                start,
                end: self.location(),
            },
            preceded_by_line_terminator,
        }
    }

    fn error_here<M: Into<String>>(&self, message: M) -> LexError {
        LexError {
            message: message.into(),
            location: self.location(),
        }
    }

    fn starts_with(&self, text: &str) -> bool {
        self.source[self.offset..].starts_with(text)
    }

    fn peek_char(&self) -> Option<char> {
        self.source[self.offset..].chars().next()
    }

    fn peek_next_char(&self) -> Option<char> {
        let current = self.peek_char()?;
        let next_offset = self.offset + current.len_utf8();
        self.source[next_offset..].chars().next()
    }

    fn advance_bytes(&mut self, len: usize) {
        for _ in 0..len {
            let Some(ch) = self.peek_char() else {
                return;
            };
            self.advance_char_with_expected(ch);
        }
    }

    fn advance_char(&mut self) -> Option<char> {
        let ch = self.peek_char()?;
        self.advance_char_with_expected(ch);
        Some(ch)
    }

    fn advance_char_with_expected(&mut self, ch: char) {
        if ch == '\r' {
            self.offset += 1;
            if self.peek_char() == Some('\n') {
                self.offset += 1;
            }
            self.line += 1;
            self.column = 1;
            return;
        }

        self.offset += ch.len_utf8();
        if matches!(ch, '\n' | '\u{2028}' | '\u{2029}') {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += ch.len_utf8();
        }
    }
}

fn keyword_for_identifier(ident: &str) -> Option<&'static str> {
    Some(match ident {
        "break" => "break",
        "case" => "case",
        "catch" => "catch",
        "class" => "class",
        "const" => "const",
        "continue" => "continue",
        "debugger" => "debugger",
        "default" => "default",
        "delete" => "delete",
        "do" => "do",
        "else" => "else",
        "export" => "export",
        "extends" => "extends",
        "finally" => "finally",
        "for" => "for",
        "function" => "function",
        "if" => "if",
        "import" => "import",
        "in" => "in",
        "instanceof" => "instanceof",
        "new" => "new",
        "return" => "return",
        "super" => "super",
        "switch" => "switch",
        "this" => "this",
        "throw" => "throw",
        "try" => "try",
        "typeof" => "typeof",
        "var" => "var",
        "void" => "void",
        "while" => "while",
        "with" => "with",
        "yield" => "yield",
        "let" => "let",
        "await" => "await",
        "async" => "async",
        "of" => "of",
        "static" => "static",
        _ => return None,
    })
}

fn is_identifier_start(ch: char) -> bool {
    ch == '$' || ch == '_' || ch.is_alphabetic()
}

fn is_identifier_part(ch: char) -> bool {
    is_identifier_start(ch) || ch.is_ascii_digit()
}

fn is_line_terminator(ch: char) -> bool {
    matches!(ch, '\r' | '\n' | '\u{2028}' | '\u{2029}')
}

fn is_js_whitespace(ch: char) -> bool {
    matches!(
        ch,
        ' ' | '\t' | '\u{000B}' | '\u{000C}' | '\u{00A0}' | '\u{FEFF}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}'
    )
}

#[cfg(test)]
mod tests {
    use super::{LexGoal, Lexer, TokenKind};

    #[test]
    fn lexes_regex_and_division_with_goal_switch() {
        let mut lexer = Lexer::new("/foo/i / bar");
        let regex = lexer.next_token(LexGoal::RegExp).unwrap();
        assert!(matches!(
            regex.kind,
            TokenKind::RegularExpression { ref body, ref flags } if body == "foo" && flags == "i"
        ));

        let div = lexer.next_token(LexGoal::Div).unwrap();
        assert_eq!(div.kind, TokenKind::Punctuator("/"));
    }

    #[test]
    fn lexes_template_parts_and_tracks_line_terminators() {
        let mut lexer = Lexer::new("`\nhello ${name}!`");
        let head = lexer.next_token(LexGoal::RegExp).unwrap();
        assert!(matches!(head.kind, TokenKind::TemplateHead(_)));

        let ident = lexer.next_token(LexGoal::RegExp).unwrap();
        assert_eq!(ident.kind, TokenKind::Identifier("name".to_string()));

        let close = lexer.next_token(LexGoal::RegExp).unwrap();
        assert_eq!(close.kind, TokenKind::Punctuator("}"));

        let tail = lexer.next_token(LexGoal::TemplateTail).unwrap();
        assert!(matches!(
            tail.kind,
            TokenKind::TemplateTail(super::TemplateChunk { ref raw, .. }) if raw == "!"
        ));
    }
}
