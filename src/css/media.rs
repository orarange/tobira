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
    /// A feature this renderer answers "no" to.
    Never,
    /// A feature nothing here recognises.
    ///
    /// Per spec an unknown media feature makes the query false, and that is what
    /// real pages count on. postcss ships breakpoints as a resolved
    /// `@media (max-width: 899px)` *plus* the original
    /// `@media (--viewport-below-md)`; a browser skips the second because it
    /// cannot read it. Treating it as a match applied firefox.com's mobile rules
    /// at every width -- including `font-size: 0` on the header download button,
    /// which shrank its label to a smear over the logo.
    Unknown,
}


/// Whether the desktop is set to a dark colour scheme.
///
/// A browser answers `prefers-color-scheme` from the machine it runs on, and
/// pages lean on it hard: firefox.com paints the whole lower half of its front
/// page from a gradient it only emits under `prefers-color-scheme: dark`.
/// Answering "light" on a dark desktop left that half white with dark text --
/// nothing like what every other browser on the same machine shows.
///
/// Read once. Nothing here notices the setting changing mid-session.
fn prefers_dark() -> bool {
    static DARK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *DARK.get_or_init(read_system_dark_mode)
}

#[cfg(target_os = "windows")]
fn read_system_dark_mode() -> bool {
    // `AppsUseLightTheme` is 0 for dark. Read through `reg` rather than taking a
    // dependency on the Windows API crates for one value; it runs once.
    let Ok(output) = std::process::Command::new("reg")
        .args([
            "query",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
            "/v",
            "AppsUseLightTheme",
        ])
        .output()
    else {
        return false;
    };
    let text = String::from_utf8_lossy(&output.stdout);
    // "    AppsUseLightTheme    REG_DWORD    0x0"
    text.split_whitespace()
        .next_back()
        .and_then(|value| value.strip_prefix("0x"))
        .and_then(|hex| u32::from_str_radix(hex, 16).ok())
        .is_some_and(|light| light == 0)
}

#[cfg(not(target_os = "windows"))]
fn read_system_dark_mode() -> bool {
    // No portable way to ask, and light is the safer default: a page that only
    // styles one scheme styles the light one.
    false
}

impl MediaCondition {
    pub(crate) fn matches(&self, viewport_width: u32) -> bool {
        match self {
            MediaCondition::MaxWidth(w) => viewport_width <= *w,
            MediaCondition::MinWidth(w) => viewport_width >= *w,
            MediaCondition::Screen => true,
            MediaCondition::Print => false,
            MediaCondition::PrefersColorSchemeDark => prefers_dark(),
            MediaCondition::All(list) => list.iter().all(|cond| cond.matches(viewport_width)),
            MediaCondition::Any(list) => list.iter().any(|cond| cond.matches(viewport_width)),
            MediaCondition::Not(inner) => !inner.matches(viewport_width),
            MediaCondition::Never => false,
            MediaCondition::Unknown => false,
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

    // Features whose answer this renderer actually knows. Everything else falls
    // through to `Unknown`, which does not match -- so these have to be spelled
    // out or a page loses the styles meant for an ordinary desktop browser.
    if let Some((feature, value)) = inner.split_once(':') {
        let feature = feature.trim();
        let value = value.trim();
        let yes = |matches: bool| {
            if matches {
                MediaCondition::Screen
            } else {
                MediaCondition::Never
            }
        };
        match feature {
            // A mouse: hover works and the pointer is fine.
            "hover" | "any-hover" => return yes(value == "hover"),
            "pointer" | "any-pointer" => return yes(value == "fine"),
            // A window is wider than it is tall often enough, and pages use this
            // to pick a phone layout.
            "orientation" => return yes(value == "landscape"),
            // No accessibility preferences are set.
            "prefers-reduced-motion" | "prefers-reduced-transparency" | "prefers-contrast" => {
                return yes(value == "no-preference");
            }
            "forced-colors" => return yes(value == "none"),
            "prefers-color-scheme" => {
                return yes(value == if prefers_dark() { "dark" } else { "light" });
            }
            // Everything is displayed, nothing is being scripted away.
            "scripting" => return yes(value == "enabled"),
            _ => {}
        }
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

#[cfg(test)]
mod color_scheme_tests {
    use super::{MediaCondition, parse_media_condition, prefers_dark};

    /// The answer depends on the desktop this runs on, so the test pins the
    /// relationship rather than the value: exactly one of the two schemes
    /// matches, and `prefers-color-scheme: dark` agrees with the bare form the
    /// parser has its own branch for.
    #[test]
    fn exactly_one_colour_scheme_matches() {
        let light = parse_media_condition("(prefers-color-scheme: light)").matches(1280);
        let dark = parse_media_condition("(prefers-color-scheme: dark)").matches(1280);

        assert_ne!(light, dark, "a desktop is one scheme or the other, not both");
        assert_eq!(
            dark,
            MediaCondition::PrefersColorSchemeDark.matches(1280),
            "both routes to the dark query must give the same answer"
        );
        assert_eq!(dark, prefers_dark());
    }
}
