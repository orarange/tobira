//! CSS `@media` query parsing and evaluation (extracted from css.rs).

use super::{parse_calc, parse_length, split_at_top_level};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MediaCondition {
    MaxWidth(u32),
    MinWidth(u32),
    Screen,
    Print,
    PrefersColorSchemeDark,
    All(Vec<MediaCondition>),
    Any(Vec<MediaCondition>),
    Not(Box<MediaCondition>),
    Unknown,
}

impl MediaCondition {
    pub(crate) fn matches(&self, viewport_width: u32) -> bool {
        match self {
            MediaCondition::MaxWidth(w) => viewport_width <= *w,
            MediaCondition::MinWidth(w) => viewport_width >= *w,
            MediaCondition::Screen => true,
            MediaCondition::Print => false,
            MediaCondition::PrefersColorSchemeDark => false,
            MediaCondition::All(list) => list.iter().all(|cond| cond.matches(viewport_width)),
            MediaCondition::Any(list) => list.iter().any(|cond| cond.matches(viewport_width)),
            MediaCondition::Not(inner) => !inner.matches(viewport_width),
            MediaCondition::Unknown => true,
        }
    }
}

pub(crate) fn parse_media_condition(query: &str) -> MediaCondition {
    let q = query.trim().to_ascii_lowercase();
    let parts = split_at_top_level(&q, ',');
    if parts.len() > 1 {
        return MediaCondition::Any(parts.iter().map(|part| parse_media_condition(part)).collect());
    }
    parse_media_condition_part(&q)
}

fn parse_media_condition_part(query: &str) -> MediaCondition {
    let q = query.trim();
    if let Some(rest) = q.strip_prefix("not ") {
        return MediaCondition::Not(Box::new(parse_media_condition_part(rest)));
    }

    let parts = split_media_and_conditions(q);
    if parts.len() > 1 {
        return MediaCondition::All(parts.iter().map(|part| parse_media_condition_part(part)).collect());
    }

    parse_media_atom(q)
}

fn split_media_and_conditions(input: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut depth_paren: u32 = 0;
    let mut depth_bracket: u32 = 0;
    let mut in_string: Option<char> = None;
    let mut escaped = false;
    let mut segment_start = 0;
    let bytes = input.as_bytes();
    let mut index = 0;
    while index < input.len() {
        let ch = input[index..].chars().next().unwrap();
        let ch_len = ch.len_utf8();
        if escaped {
            escaped = false;
            index += ch_len;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            q @ ('"' | '\'') if in_string.is_none() => in_string = Some(q),
            q if in_string == Some(q) => in_string = None,
            _ if in_string.is_some() => {}
            '(' => depth_paren += 1,
            ')' if depth_paren > 0 => depth_paren -= 1,
            '[' => depth_bracket += 1,
            ']' if depth_bracket > 0 => depth_bracket -= 1,
            'a' if depth_paren == 0 && depth_bracket == 0 && in_string.is_none() => {
                if index == 0 || bytes[index - 1].is_ascii_whitespace() {
                    let rest = &input[index..];
                    if rest.starts_with("and")
                        && rest[3..].chars().next().is_some_and(|c| c.is_whitespace())
                    {
                        let before = input[segment_start..index].trim();
                        if !before.is_empty() {
                            result.push(before.to_string());
                        }
                        let mut next = index + 3;
                        while next < input.len() {
                            let mut chars = input[next..].chars();
                            let Some(c) = chars.next() else { break };
                            if !c.is_whitespace() {
                                break;
                            }
                            next += c.len_utf8();
                        }
                        segment_start = next;
                        index = next;
                        continue;
                    }
                }
            }
            _ => {}
        }
        index += ch_len;
    }
    let tail = input[segment_start..].trim();
    if !tail.is_empty() {
        result.push(tail.to_string());
    }
    result
}

fn parse_media_atom(query: &str) -> MediaCondition {
    let q = query.trim();
    // Strip one layer of parens, not every trailing one: `trim_end_matches`
    // is greedy, so `(width >= calc(40rem))` came out as `width >= calc(40rem`
    // and no value in a media feature could be a function call.
    let inner = q
        .strip_prefix('(')
        .and_then(|rest| rest.strip_suffix(')'))
        .unwrap_or(q)
        .trim();

    if inner == "screen" || q == "screen" || inner == "all" || q == "all" {
        return MediaCondition::Screen;
    }
    if inner == "print" || q == "print" {
        return MediaCondition::Print;
    }
    if inner.contains("prefers-color-scheme") && inner.contains("dark") {
        return MediaCondition::PrefersColorSchemeDark;
    }
    if let Some(rest) = inner.strip_prefix("max-width:") {
        if let Some(px) = parse_media_length(rest.trim()) {
            return MediaCondition::MaxWidth(px);
        }
    }
    if let Some(rest) = inner.strip_prefix("min-width:") {
        if let Some(px) = parse_media_length(rest.trim()) {
            return MediaCondition::MinWidth(px);
        }
    }
    if let Some(condition) = parse_width_range(inner) {
        return condition;
    }
    MediaCondition::Unknown
}

/// A length in a media query. `rem` is against the initial font size (16px) by
/// definition -- a media query cannot depend on the document's own font size --
/// so the fixed 16 here is the spec behaviour, not a shortcut.
fn parse_media_length(value: &str) -> Option<u32> {
    let value = value.trim();
    if let Some(inner) = value
        .strip_prefix("calc(")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        return parse_calc(inner, 16);
    }
    parse_length(value, 16)
}

/// The range syntax: `(width >= 40rem)`, `(width < 769px)`, the reversed
/// `(40rem <= width)`, and the two-sided `(20rem <= width <= 60rem)`.
///
/// These were unrecognised, and an unrecognised query *matches*, so a page using
/// them applied its mobile and desktop rules at the same time and let source
/// order decide the winner. On MDN's docs pages that left the left sidebar at
/// `display: none` however wide the window was.
fn parse_width_range(inner: &str) -> Option<MediaCondition> {
    let mut operands: Vec<String> = Vec::new();
    let mut operators: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;

    let characters: Vec<char> = inner.chars().collect();
    let mut index = 0usize;
    while index < characters.len() {
        let character = characters[index];
        match character {
            '(' => {
                depth += 1;
                current.push(character);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(character);
            }
            '<' | '>' if depth == 0 => {
                let mut operator = String::from(character);
                if characters.get(index + 1) == Some(&'=') {
                    operator.push('=');
                    index += 1;
                }
                operands.push(current.trim().to_string());
                current.clear();
                operators.push(operator);
            }
            _ => current.push(character),
        }
        index += 1;
    }
    operands.push(current.trim().to_string());

    if operators.is_empty() {
        return None;
    }

    // `width > v` is `width >= v + 1` on a whole-pixel viewport, and likewise at
    // the other end. Reusing MinWidth/MaxWidth keeps evaluation in one place.
    let lower = |operator: &str, value: u32| match operator {
        ">=" => Some(MediaCondition::MinWidth(value)),
        ">" => Some(MediaCondition::MinWidth(value.saturating_add(1))),
        _ => None,
    };
    let upper = |operator: &str, value: u32| match operator {
        "<=" => Some(MediaCondition::MaxWidth(value)),
        "<" => Some(MediaCondition::MaxWidth(value.saturating_sub(1))),
        _ => None,
    };
    fn flip(operator: &str) -> &str {
        match operator {
            ">=" => "<=",
            ">" => "<",
            "<=" => ">=",
            "<" => ">",
            other => other,
        }
    }

    match (operands.as_slice(), operators.as_slice()) {
        // `width >= v` / `width < v`
        ([name, value], [operator]) if name.eq_ignore_ascii_case("width") => {
            let value = parse_media_length(value)?;
            lower(operator, value).or_else(|| upper(operator, value))
        }
        // `v <= width` -- same relation read from the other side.
        ([value, name], [operator]) if name.eq_ignore_ascii_case("width") => {
            let value = parse_media_length(value)?;
            let operator = flip(operator);
            lower(operator, value).or_else(|| upper(operator, value))
        }
        // `v1 <= width <= v2`
        ([low, name, high], [first, second]) if name.eq_ignore_ascii_case("width") => {
            let low = parse_media_length(low)?;
            let high = parse_media_length(high)?;
            let low = lower(flip(first), low)?;
            let high = upper(second, high)?;
            Some(MediaCondition::All(vec![low, high]))
        }
        _ => None,
    }
}
