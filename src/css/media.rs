//! CSS `@media` query parsing and evaluation (extracted from css.rs).

use super::{parse_length, split_at_top_level};

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
    let inner = q.trim_start_matches('(').trim_end_matches(')').trim();

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
        if let Some(px) = parse_length(rest.trim(), 16) {
            return MediaCondition::MaxWidth(px);
        }
    }
    if let Some(rest) = inner.strip_prefix("min-width:") {
        if let Some(px) = parse_length(rest.trim(), 16) {
            return MediaCondition::MinWidth(px);
        }
    }
    MediaCondition::Unknown
}
