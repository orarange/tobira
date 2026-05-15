use std::collections::BTreeMap;
use std::rc::Rc;

use crate::html::{Element, Node};

pub type Color = u32;

pub const DEFAULT_TEXT_COLOR: Color = 0x1D232E;
pub const DEFAULT_BACKGROUND_COLOR: Color = 0xFFFDF8;
pub const DEFAULT_LINK_COLOR: Color = 0x2A5DB0;

// ─────────────────────────────────────────────────────────────────────────────
// Stylesheet / Rule types
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stylesheet {
    pub rules: Vec<Rule>,
    pub root_vars: std::collections::BTreeMap<String, String>,
}

impl Stylesheet {
    pub fn extend(&mut self, other: Stylesheet) {
        self.rules.extend(other.rules);
        self.root_vars.extend(other.root_vars);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    selectors: Vec<Selector>,
    declarations: Vec<Declaration>,
    /// None = always apply; Some(cond) = apply only when cond matches
    media: Option<MediaCondition>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MediaCondition {
    MaxWidth(u32),
    MinWidth(u32),
    Screen,
    Print,
    PrefersColorSchemeDark,
    Unknown,
}

impl MediaCondition {
    fn matches(&self, viewport_width: u32) -> bool {
        match self {
            MediaCondition::MaxWidth(w) => viewport_width <= *w,
            MediaCondition::MinWidth(w) => viewport_width >= *w,
            MediaCondition::Screen => true,
            MediaCondition::Print => false,
            MediaCondition::PrefersColorSchemeDark => false,
            MediaCondition::Unknown => true,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Selector types
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
struct Selector {
    parts: Vec<SelectorPart>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SelectorPart {
    simple: SimpleSelector,
    combinator: Option<Combinator>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Combinator {
    Descendant,
    Child,
    AdjacentSibling,
    GeneralSibling,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SimpleSelector {
    tag_name: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
    universal: bool,
    pseudo_classes: Vec<PseudoClass>,
    attributes: Vec<AttributeCondition>,
    never_match: bool, // for ::before / ::after pseudo-elements
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PseudoClass {
    FirstChild,
    LastChild,
    NthChild(i32, i32), // (a, b) → matches when (index - b) % a == 0 (1-based index)
    Not(Vec<SimpleSelector>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AttributeCondition {
    name: String,
    operator: AttrOperator,
    value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AttrOperator {
    Exists,
    Equals,
    Contains,
    StartsWith,
    EndsWith,
    Word,
    DashPrefix,
}

// ─────────────────────────────────────────────────────────────────────────────
// Declaration
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    property: String,
    value: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Element identity (for selector matching)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
struct ElementIdentity {
    tag_name: String,
    id: Option<String>,
    classes: Vec<String>,
    attributes: BTreeMap<String, String>,
}

/// Returns a shared empty `Rc<[ElementIdentity]>` without allocating on each call.
/// Used for synthetic `AncestorSlot`s created during selector matching where no
/// sibling data is needed.
fn empty_siblings_rc() -> Rc<[ElementIdentity]> {
    thread_local! {
        static EMPTY: Rc<[ElementIdentity]> = Rc::from([]);
    }
    EMPTY.with(Rc::clone)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AncestorSlot {
    element: ElementIdentity,
    sibling_index: usize,
    sibling_count: usize,
    /// The parent's full sibling identity list (shared `Rc`, no per-element cloning).
    /// `siblings[..prec_count]` yields this element's preceding siblings.
    /// Top-level elements without a parent use an empty Rc.
    siblings: Rc<[ElementIdentity]>,
    /// Index of this element in `siblings` (equal to the number of preceding siblings).
    prec_count: usize,
}

impl AncestorSlot {
    fn preceding_siblings(&self) -> &[ElementIdentity] {
        &self.siblings[..self.prec_count]
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Enums used in ComputedStyle
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Display {
    Block,
    Inline,
    ListItem,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerticalAlign {
    Top,
    Middle,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhiteSpaceMode {
    Normal,
    Pre,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FontFamilyKind {
    Sans,
    Monospace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LengthValue {
    Pixels(u32),
    Percent(u32),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EdgeSizes {
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
    pub left: u32,
}

impl EdgeSizes {
    pub fn all(value: u32) -> Self {
        Self {
            top: value,
            right: value,
            bottom: value,
            left: value,
        }
    }

    pub fn vertical(top: u32, bottom: u32) -> Self {
        Self {
            top,
            right: 0,
            bottom,
            left: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextTransform {
    None,
    Uppercase,
    Lowercase,
    Capitalize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoxSizing {
    ContentBox,
    BorderBox,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoxShadow {
    pub offset_x: i32,
    pub offset_y: i32,
    pub blur: u32,
    pub color: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overflow {
    Visible,
    Hidden,
    Auto,
    Scroll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListStyleType {
    Disc,
    Circle,
    Square,
    Decimal,
    None,
}

// ─────────────────────────────────────────────────────────────────────────────
// ComputedStyle
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComputedStyle {
    pub display: Display,
    pub color: Color,
    pub background_color: Option<Color>,
    pub margin: EdgeSizes,
    pub padding: EdgeSizes,
    pub width: Option<LengthValue>,
    pub height: Option<LengthValue>,
    pub font_size_px: u32,
    pub font_family: FontFamilyKind,
    pub text_align: TextAlign,
    pub vertical_align: VerticalAlign,
    pub font_weight: bool,
    pub underline: bool,
    pub white_space: WhiteSpaceMode,
    // ── new fields ──
    pub border: EdgeSizes,
    pub border_color: Color,
    pub border_style_none: bool,
    pub border_radius: u32,
    pub outline_width: u32,
    pub outline_color: Option<Color>,
    /// line-height in thousandths of em; 0 = "normal"
    pub line_height: u32,
    /// opacity 0–255; 255 = opaque
    pub opacity: u8,
    pub effective_opacity: u8,
    pub font_style_italic: bool,
    pub text_transform: TextTransform,
    pub text_indent: u32,
    pub letter_spacing: i16,
    pub max_width: Option<u32>,
    pub min_width: u32,
    pub max_height: Option<u32>,
    pub min_height: u32,
    pub box_sizing: BoxSizing,
    pub overflow: Overflow,
    pub list_style_type: ListStyleType,
    pub cursor_pointer: bool,
    pub text_decoration_color: Option<Color>,
    pub box_shadow: Option<BoxShadow>,
}

impl ComputedStyle {
    fn for_element(tag_name: &str, parent: Option<&Self>) -> Self {
        let parent_font_size = parent.map(|s| s.font_size_px).unwrap_or(16);
        let mut style = Self {
            display: default_display(tag_name),
            color: parent.map(|s| s.color).unwrap_or(DEFAULT_TEXT_COLOR),
            background_color: None,
            margin: default_margin(tag_name),
            padding: EdgeSizes::default(),
            width: None,
            height: None,
            font_size_px: parent_font_size,
            font_family: parent
                .map(|s| s.font_family)
                .unwrap_or(FontFamilyKind::Sans),
            text_align: parent.map(|s| s.text_align).unwrap_or(TextAlign::Left),
            vertical_align: VerticalAlign::Top,
            font_weight: parent.map(|s| s.font_weight).unwrap_or(false),
            underline: parent.map(|s| s.underline).unwrap_or(false),
            white_space: parent
                .map(|s| s.white_space)
                .unwrap_or(WhiteSpaceMode::Normal),
            // new fields – most not inherited
            border: EdgeSizes::default(),
            border_color: parent.map(|s| s.color).unwrap_or(DEFAULT_TEXT_COLOR),
            border_style_none: false,
            border_radius: 0,
            outline_width: 0,
            outline_color: None,
            line_height: parent.map(|s| s.line_height).unwrap_or(0),
            opacity: 255,
            effective_opacity: 255,
            font_style_italic: parent.map(|s| s.font_style_italic).unwrap_or(false),
            text_transform: parent
                .map(|s| s.text_transform)
                .unwrap_or(TextTransform::None),
            text_indent: 0,
            letter_spacing: parent.map(|s| s.letter_spacing).unwrap_or(0),
            max_width: None,
            min_width: 0,
            max_height: None,
            min_height: 0,
            box_sizing: BoxSizing::ContentBox,
            overflow: Overflow::Visible,
            list_style_type: ListStyleType::Disc,
            cursor_pointer: false,
            text_decoration_color: None,
            box_shadow: None,
        };

        match tag_name {
            "body" => {
                style.margin = EdgeSizes::all(8);
            }
            "h1" => {
                style.font_size_px = 32;
                style.font_weight = true;
                style.margin = EdgeSizes::vertical(18, 12);
            }
            "h2" => {
                style.font_size_px = 28;
                style.font_weight = true;
                style.margin = EdgeSizes::vertical(16, 10);
            }
            "h3" => {
                style.font_size_px = 24;
                style.font_weight = true;
                style.margin = EdgeSizes::vertical(14, 8);
            }
            "h4" => {
                style.font_size_px = 20;
                style.font_weight = true;
                style.margin = EdgeSizes::vertical(12, 8);
            }
            "h5" => {
                style.font_size_px = 18;
                style.font_weight = true;
                style.margin = EdgeSizes::vertical(10, 6);
            }
            "h6" => {
                style.font_size_px = 16;
                style.font_weight = true;
                style.margin = EdgeSizes::vertical(10, 6);
            }
            "a" => {
                style.color = DEFAULT_LINK_COLOR;
                style.underline = true;
            }
            "pre" => {
                style.font_family = FontFamilyKind::Monospace;
                style.white_space = WhiteSpaceMode::Pre;
                style.margin = EdgeSizes::vertical(12, 12);
                style.padding = EdgeSizes::all(8);
                style.background_color = Some(0xF2EEE7);
            }
            "code" => {
                style.font_family = FontFamilyKind::Monospace;
                style.padding = EdgeSizes::all(2);
                style.background_color = Some(0xF2EEE7);
            }
            "strong" | "b" => {
                style.font_weight = true;
            }
            "small" => {
                style.font_size_px = parent_font_size.saturating_sub(2).max(12);
            }
            "big" => {
                style.font_size_px = parent_font_size.saturating_add(2);
            }
            _ => {}
        }

        style
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// StyledNode tree
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StyledNode {
    Element(StyledElement),
    Text(StyledText),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyledElement {
    pub tag_name: String,
    pub attributes: BTreeMap<String, String>,
    pub style: ComputedStyle,
    pub children: Vec<StyledNode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyledText {
    pub text: String,
    pub style: ComputedStyle,
}

// ─────────────────────────────────────────────────────────────────────────────
// Public API
// ─────────────────────────────────────────────────────────────────────────────

/// Split `input` on `delimiter` but only at depth 0 (ignoring delimiters inside
/// parentheses/brackets and quoted strings).  This prevents `:not(.a, .b)` from
/// being split on the inner comma.
fn split_at_top_level(input: &str, delimiter: char) -> Vec<String> {
    let mut result = Vec::new();
    let mut depth: u32 = 0;
    let mut in_string: Option<char> = None;
    let mut escaped = false;
    let mut segment_start = 0;
    for (index, ch) in input.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            // Handle backslash escapes both inside strings AND at the top level
            // (e.g. `\,` in a selector must not be treated as a delimiter).
            '\\' => {
                escaped = true;
            }
            q @ ('"' | '\'') if in_string.is_none() => {
                in_string = Some(q);
            }
            q if in_string == Some(q) => {
                in_string = None;
            }
            _ if in_string.is_some() => {}
            '(' | '[' => {
                depth += 1;
            }
            ')' | ']' if depth > 0 => {
                depth -= 1;
            }
            c if c == delimiter && depth == 0 => {
                result.push(input[segment_start..index].to_string());
                segment_start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    result.push(input[segment_start..].to_string());
    result
}

fn find_matching_close_brace(source: &str) -> Option<usize> {
    let mut depth: u32 = 1;
    for (i, ch) in source.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

pub fn parse_stylesheet(input: &str) -> Stylesheet {
    let mut rules = Vec::new();
    let source = strip_comments(input);
    let mut cursor = 0;

    while let Some(open_offset) = source[cursor..].find('{') {
        let selector_start = cursor;
        let selector_end = cursor + open_offset;
        let block_start = selector_end + 1;

        let block_text_raw = &source[block_start..];
        let Some(close_offset) = find_matching_close_brace(block_text_raw) else {
            break;
        };
        let block_end = block_start + close_offset;

        let selector_text = source[selector_start..selector_end].trim();
        let block_text = source[block_start..block_end].trim();
        cursor = block_end + 1;

        if selector_text.is_empty() {
            continue;
        }

        // Handle @media blocks
        if selector_text.starts_with('@') {
            let at_lower = selector_text.to_ascii_lowercase();
            if at_lower.starts_with("@media") {
                let media_query = selector_text["@media".len()..].trim();
                let media_cond = parse_media_condition(media_query);
                // The block_text is the inner CSS of the @media block
                // Parse the inner rules and tag them with the media condition
                let inner_stylesheet = parse_stylesheet(block_text);
                for mut rule in inner_stylesheet.rules {
                    rule.media = Some(media_cond.clone());
                    rules.push(rule);
                }
            }
            // other at-rules are skipped
            continue;
        }

        if block_text.is_empty() {
            continue;
        }

        let selectors = split_at_top_level(selector_text, ',')
            .iter()
            .filter_map(|s| parse_selector(s.trim()))
            .collect::<Vec<_>>();
        let declarations = parse_inline_declarations(block_text);

        if !selectors.is_empty() && !declarations.is_empty() {
            rules.push(Rule {
                selectors,
                declarations,
                media: None,
            });
        }
    }

    // Pre-pass: collect :root and html CSS variables from the raw source
    let mut root_vars = std::collections::BTreeMap::new();
    {
        let source2 = strip_comments(input);
        let mut cur2 = 0;
        while let Some(open_off) = source2[cur2..].find('{') {
            let sel_start = cur2;
            let sel_end = cur2 + open_off;
            let blk_start = sel_end + 1;
            let blk_raw = &source2[blk_start..];
            let Some(close_off) = find_matching_close_brace(blk_raw) else { break };
            let blk_end = blk_start + close_off;
            let sel_text = source2[sel_start..sel_end].trim().to_ascii_lowercase();
            let blk_text = source2[blk_start..blk_end].trim();
            cur2 = blk_end + 1;
            let is_root = sel_text.split(',').any(|s| {
                let s = s.trim();
                s == ":root" || s == "html"
            });
            if is_root {
                for decl in parse_inline_declarations(blk_text) {
                    if decl.property.starts_with("--") {
                        root_vars.insert(decl.property.clone(), decl.value.clone());
                    }
                }
            }
        }
    }
    Stylesheet { rules, root_vars }
}

fn parse_media_condition(query: &str) -> MediaCondition {
    let q = query.trim().to_ascii_lowercase();
    // Strip surrounding parens if present
    let inner = q.trim_start_matches('(').trim_end_matches(')').trim();

    if inner == "screen" || q == "screen" {
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

pub fn parse_inline_declarations(input: &str) -> Vec<Declaration> {
    let stripped = strip_comments(input);
    split_at_top_level(&stripped, ';')
        .into_iter()
        .filter_map(|entry| {
            let (property, value) = entry.split_once(':')?;
            let property = property.trim().to_ascii_lowercase();
            let value = value.trim().to_string();
            if property.is_empty() || value.is_empty() {
                return None;
            }
            Some(Declaration { property, value })
        })
        .collect()
}

pub fn build_styled_tree(
    document: &Node,
    stylesheet: &Stylesheet,
    viewport_width: u32,
) -> StyledNode {
    let ancestors = Vec::new();
    build_node(
        document,
        stylesheet,
        None,
        &ancestors,
        0,
        0,
        &[],
        None,
        viewport_width,
    )
}

fn build_node(
    node: &Node,
    stylesheet: &Stylesheet,
    parent_style: Option<&ComputedStyle>,
    ancestors: &[AncestorSlot],
    sibling_index: usize,
    sibling_count: usize,
    preceding_siblings: &[ElementIdentity],
    // The parent's shared full-sibling Rc (all children of the same parent).
    // When Some, used directly for AncestorSlot.siblings to avoid a per-element clone.
    // None at the root or for nodes without an element parent.
    parent_all_sibling_ids: Option<Rc<[ElementIdentity]>>,
    viewport_width: u32,
) -> StyledNode {
    match node {
        Node::Text(text) => {
            let mut style = parent_style
                .cloned()
                .unwrap_or_else(|| ComputedStyle::for_element("body", None));
            // If the parent is a block stacking context (opacity < 255, non-inline), the
            // LayerCommand handles compositing at the parent's opacity. The text node's
            // effective_opacity should be 255 inside the layer to avoid double application.
            if let Some(parent) = parent_style {
                let parent_is_block = !matches!(parent.display, Display::Inline);
                if parent.opacity < 255 && parent_is_block {
                    style.effective_opacity = 255;
                }
            }
            StyledNode::Text(StyledText {
                text: text.clone(),
                style,
            })
        }
        Node::Element(element) => {
            let style = compute_style(
                element,
                stylesheet,
                parent_style,
                ancestors,
                sibling_index,
                sibling_count,
                preceding_siblings,
                viewport_width,
            );
            // Pre-build the full sibling identity list once for all children to share.
            let all_sibling_ids: Rc<[ElementIdentity]> = element
                .children
                .iter()
                .filter_map(|c| if let Node::Element(e) = c { Some(ElementIdentity::from(e)) } else { None })
                .collect::<Vec<_>>()
                .into();
            let child_element_count = all_sibling_ids.len();

            // `current_slot` records this element's position in its parent's sibling list so
            // that ancestor-combinator matching can call `ancestor.preceding_siblings()`.
            // Re-use the parent's shared `Rc<[ElementIdentity]>` when available (threaded in
            // via `parent_all_sibling_ids`) so that all siblings of the same parent share one
            // allocation.  Falls back to a fresh Rc for top-level / root nodes.
            let current_slot = AncestorSlot {
                element: ElementIdentity::from(element),
                sibling_index,
                sibling_count,
                siblings: parent_all_sibling_ids.unwrap_or_else(|| Rc::from(preceding_siblings)),
                prec_count: sibling_index,
            };
            let mut next_ancestors = ancestors.to_vec();
            next_ancestors.push(current_slot);

            let mut elem_sibling_idx = 0;

            let children = element
                .children
                .iter()
                .map(|child| {
                    let (idx, count, prec_snap) = if matches!(child, Node::Element(_)) {
                        let idx = elem_sibling_idx;
                        elem_sibling_idx += 1;
                        (idx, child_element_count, &all_sibling_ids[..idx])
                    } else {
                        (0, 0, &all_sibling_ids[..0])
                    };
                    build_node(
                        child,
                        stylesheet,
                        Some(&style),
                        &next_ancestors,
                        idx,
                        count,
                        prec_snap,
                        Some(all_sibling_ids.clone()), // share parent's Rc with all children
                        viewport_width,
                    )
                })
                .collect();

            StyledNode::Element(StyledElement {
                tag_name: element.tag_name.clone(),
                attributes: element.attributes.clone(),
                style,
                children,
            })
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn compute_style(
    element: &Element,
    stylesheet: &Stylesheet,
    parent_style: Option<&ComputedStyle>,
    ancestors: &[AncestorSlot],
    sibling_index: usize,
    sibling_count: usize,
    preceding_siblings: &[ElementIdentity],
    viewport_width: u32,
) -> ComputedStyle {
    let mut style = ComputedStyle::for_element(&element.tag_name, parent_style);
    let parent_font_size = parent_style.map(|c| c.font_size_px).unwrap_or(16);
    apply_legacy_attributes(&mut style, element, parent_font_size);

    let identity = ElementIdentity::from(element);
    let mut css_variables = stylesheet.root_vars.clone();
    let mut applicable: Vec<(usize, usize, Declaration)> = Vec::new();

    for (rule_index, rule) in stylesheet.rules.iter().enumerate() {
        // Check media condition
        if let Some(cond) = &rule.media {
            if !cond.matches(viewport_width) {
                continue;
            }
        }
        for selector in &rule.selectors {
            if selector.matches(
                &identity,
                ancestors,
                sibling_index,
                sibling_count,
                preceding_siblings,
            ) {
                // First pass: collect CSS variables
                for decl in &rule.declarations {
                    if decl.property.starts_with("--") {
                        css_variables.insert(decl.property.clone(), decl.value.clone());
                    }
                }
                applicable.extend(rule.declarations.iter().cloned().enumerate().map(
                    |(declaration_index, declaration)| {
                        (
                            selector.specificity(),
                            rule_index * 100 + declaration_index,
                            declaration,
                        )
                    },
                ));
                break; // each rule contributes once per matching selector
            }
        }
    }

    if let Some(inline_style) = element.attribute("style") {
        let inline_decls = parse_inline_declarations(inline_style);
        // collect inline CSS variables first
        for decl in &inline_decls {
            if decl.property.starts_with("--") {
                css_variables.insert(decl.property.clone(), decl.value.clone());
            }
        }
        applicable.extend(
            inline_decls
                .into_iter()
                .enumerate()
                .map(|(index, declaration)| (1_000, usize::MAX - 1_000 + index, declaration)),
        );
    }

    applicable.sort_by_key(|(specificity, order, _)| (*specificity, *order));

    for (_, _, mut declaration) in applicable {
        // skip CSS custom properties
        if declaration.property.starts_with("--") {
            continue;
        }
        // substitute var() references
        if declaration.value.contains("var(") {
            declaration.value = substitute_vars(&declaration.value, &css_variables);
        }
        apply_declaration(&mut style, &declaration, parent_font_size);
    }

    style.effective_opacity = parent_style
        .map(|parent| {
            // CSS opacity < 1 creates a stacking context for ALL element types, including
            // inline (per the CSS spec).  For block/table elements the LayerCommand
            // compositor handles the parent's opacity, so children reset effective_opacity
            // to their own opacity.  For inline elements we currently do not emit a
            // LayerCommand (inline content is painted as flat TextCommands), so the
            // stacking-context reset is still applied for consistency: inline opacity
            // boundaries are composited approximately rather than via an offscreen buffer.
            let parent_is_stacking_context = parent.opacity < 255;
            if parent_is_stacking_context {
                style.opacity
            } else {
                ((parent.effective_opacity as u16 * style.opacity as u16) / 255) as u8
            }
        })
        .unwrap_or(style.opacity);

    style
}

fn substitute_vars(value: &str, vars: &BTreeMap<String, String>) -> String {
    let mut result = value.to_string();
    let mut iterations = 0;
    while result.contains("var(") && iterations < 10 {
        iterations += 1;
        let Some(start) = result.find("var(") else {
            break;
        };
        let inner_start = start + 4;
        let Some(end) = result[inner_start..].find(')') else {
            break;
        };
        let inner = &result[inner_start..inner_start + end];
        let (var_name, fallback) = if let Some(comma) = inner.find(',') {
            (&inner[..comma], Some(inner[comma + 1..].trim()))
        } else {
            (inner.trim(), None)
        };
        let replacement = vars
            .get(var_name.trim())
            .map(|s| s.as_str())
            .or(fallback)
            .unwrap_or("")
            .to_string();
        result = format!(
            "{}{}{}",
            &result[..start],
            replacement,
            &result[inner_start + end + 1..]
        );
    }
    result
}

fn apply_declaration(style: &mut ComputedStyle, declaration: &Declaration, parent_font_size: u32) {
    let value = &declaration.value;
    match declaration.property.as_str() {
        "color" => {
            if let Some(color) = parse_color(value) {
                style.color = color;
            }
        }
        "background" | "background-color" => {
            style.background_color = parse_color(value);
        }
        "display" => {
            if let Some(display) = parse_display(value) {
                style.display = display;
            }
        }
        "font-size" => {
            if let Some(font_size) = parse_font_size(value, parent_font_size) {
                style.font_size_px = font_size.max(8);
            }
        }
        "font-family" => {
            if let Some(font_family) = parse_font_family(value) {
                style.font_family = font_family;
            }
        }
        "font-weight" => {
            style.font_weight = parse_font_weight(value).unwrap_or(style.font_weight);
        }
        "font-style" => {
            let v = value.trim().to_ascii_lowercase();
            style.font_style_italic = matches!(v.as_str(), "italic" | "oblique");
        }
        "font" => {
            parse_font_shorthand(style, value, parent_font_size);
        }
        "width" => {
            let v = value.trim().to_ascii_lowercase();
            if v == "auto" {
                style.width = None;
            } else {
                style.width = parse_length_value(value, parent_font_size);
            }
        }
        "height" => {
            let v = value.trim().to_ascii_lowercase();
            if v == "auto" {
                style.height = None;
            } else {
                style.height = parse_length_value(value, parent_font_size);
            }
        }
        "max-width" => {
            let v = value.trim().to_ascii_lowercase();
            if v == "none" {
                style.max_width = None;
            } else {
                style.max_width = parse_length(value, parent_font_size);
            }
        }
        "min-width" => {
            let v = value.trim().to_ascii_lowercase();
            if v == "auto" {
                style.min_width = 0;
            } else {
                style.min_width = parse_length(value, parent_font_size).unwrap_or(0);
            }
        }
        "max-height" => {
            let v = value.trim().to_ascii_lowercase();
            if v == "none" {
                style.max_height = None;
            } else {
                style.max_height = parse_length(value, parent_font_size);
            }
        }
        "min-height" => {
            style.min_height = parse_length(value, parent_font_size).unwrap_or(0);
        }
        "text-align" => {
            if let Some(text_align) = parse_text_align(value) {
                style.text_align = text_align;
            }
        }
        "vertical-align" => {
            if let Some(va) = parse_vertical_align(value) {
                style.vertical_align = va;
            }
        }
        "text-decoration" => {
            let v = value.trim().to_ascii_lowercase();
            if v.contains("none") {
                style.underline = false;
            } else if v.contains("underline") {
                style.underline = true;
            }
            // line-through etc → no change to underline (don't set it)
        }
        "text-decoration-color" => {
            style.text_decoration_color = parse_color(value);
        }
        "text-transform" => {
            style.text_transform = parse_text_transform(value);
        }
        "text-indent" => {
            style.text_indent = parse_length(value, parent_font_size).unwrap_or(0);
        }
        "letter-spacing" => {
            let v = value.trim().to_ascii_lowercase();
            if v == "normal" {
                style.letter_spacing = 0;
            } else if let Some(px) = parse_signed_length(value, parent_font_size) {
                style.letter_spacing = px;
            }
        }
        "white-space" => {
            if let Some(ws) = parse_white_space(value) {
                style.white_space = ws;
            }
        }
        "margin" => {
            if let Some(edges) = parse_box_shorthand(value, parent_font_size) {
                style.margin = edges;
            }
        }
        "padding" => {
            if let Some(edges) = parse_box_shorthand(value, parent_font_size) {
                style.padding = edges;
            }
        }
        "margin-top" => {
            if let Some(v) = parse_length(value, parent_font_size) {
                style.margin.top = v;
            }
        }
        "margin-right" => {
            if let Some(v) = parse_length(value, parent_font_size) {
                style.margin.right = v;
            }
        }
        "margin-bottom" => {
            if let Some(v) = parse_length(value, parent_font_size) {
                style.margin.bottom = v;
            }
        }
        "margin-left" => {
            if let Some(v) = parse_length(value, parent_font_size) {
                style.margin.left = v;
            }
        }
        "padding-top" => {
            if let Some(v) = parse_length(value, parent_font_size) {
                style.padding.top = v;
            }
        }
        "padding-right" => {
            if let Some(v) = parse_length(value, parent_font_size) {
                style.padding.right = v;
            }
        }
        "padding-bottom" => {
            if let Some(v) = parse_length(value, parent_font_size) {
                style.padding.bottom = v;
            }
        }
        "padding-left" => {
            if let Some(v) = parse_length(value, parent_font_size) {
                style.padding.left = v;
            }
        }
        // Border shorthands
        "border" => {
            parse_border_shorthand(style, value, parent_font_size);
        }
        "border-width" => {
            if let Some(edges) = parse_box_shorthand(value, parent_font_size) {
                style.border = edges;
            }
        }
        "border-top" => {
            parse_border_side_shorthand(style, value, parent_font_size, "top");
        }
        "border-right" => {
            parse_border_side_shorthand(style, value, parent_font_size, "right");
        }
        "border-bottom" => {
            parse_border_side_shorthand(style, value, parent_font_size, "bottom");
        }
        "border-left" => {
            parse_border_side_shorthand(style, value, parent_font_size, "left");
        }
        "border-top-width" => {
            if let Some(v) = parse_length(value, parent_font_size) {
                style.border.top = v;
            }
        }
        "border-right-width" => {
            if let Some(v) = parse_length(value, parent_font_size) {
                style.border.right = v;
            }
        }
        "border-bottom-width" => {
            if let Some(v) = parse_length(value, parent_font_size) {
                style.border.bottom = v;
            }
        }
        "border-left-width" => {
            if let Some(v) = parse_length(value, parent_font_size) {
                style.border.left = v;
            }
        }
        "border-color" => {
            if let Some(color) = parse_color(value) {
                style.border_color = color;
            }
        }
        "border-top-color" => {
            if let Some(color) = parse_color(value) {
                style.border_color = color; // simplified: single color
            }
        }
        "border-right-color" | "border-bottom-color" | "border-left-color" => {
            if let Some(color) = parse_color(value) {
                style.border_color = color;
            }
        }
        "border-style" => {
            let v = value.trim().to_ascii_lowercase();
            style.border_style_none = v == "none";
        }
        "border-radius" => {
            style.border_radius = parse_length(value, parent_font_size).unwrap_or(0);
        }
        "outline" => {
            parse_outline_shorthand(style, value, parent_font_size);
        }
        "outline-width" => {
            style.outline_width = parse_length(value, parent_font_size).unwrap_or(0);
        }
        "outline-color" => {
            style.outline_color = parse_color(value);
        }
        "line-height" => {
            style.line_height = parse_line_height(value, parent_font_size);
        }
        "opacity" => {
            if let Ok(f) = value.trim().parse::<f32>() {
                style.opacity = (f.clamp(0.0, 1.0) * 255.0).round() as u8;
            }
        }
        "visibility" => {
            let v = value.trim().to_ascii_lowercase();
            if v == "hidden" {
                style.opacity = 0;
            }
        }
        "box-sizing" => {
            let v = value.trim().to_ascii_lowercase();
            style.box_sizing = match v.as_str() {
                "border-box" => BoxSizing::BorderBox,
                _ => BoxSizing::ContentBox,
            };
        }
        "overflow" => {
            style.overflow = parse_overflow(value);
        }
        "overflow-x" | "overflow-y" => {
            // Use the more restrictive one
            let ov = parse_overflow(value);
            if ov != Overflow::Visible {
                style.overflow = ov;
            }
        }
        "list-style-type" => {
            style.list_style_type = parse_list_style_type(value);
        }
        "list-style" => {
            // simple: just look for known list-style-type tokens
            style.list_style_type = parse_list_style_type(value);
        }
        "box-shadow" => {
            let v = value.trim().to_ascii_lowercase();
            if v == "none" {
                style.box_shadow = None;
            } else {
                style.box_shadow = parse_box_shadow(value);
            }
        }
        "cursor" => {
            let v = value.trim().to_ascii_lowercase();
            style.cursor_pointer = v == "pointer";
        }
        _ => {}
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// default_display / default_margin
// ─────────────────────────────────────────────────────────────────────────────

fn default_display(tag_name: &str) -> Display {
    match tag_name {
        "document" | "html" | "body" | "main" | "section" | "article" | "div" | "header"
        | "footer" | "nav" | "aside" | "p" | "ul" | "ol" | "li" | "pre" | "blockquote" | "h1"
        | "h2" | "h3" | "h4" | "h5" | "h6" | "table" | "tbody" | "thead" | "tfoot" | "tr"
        | "td" | "th" | "center" | "frameset" | "hr" => {
            if tag_name == "li" {
                Display::ListItem
            } else {
                Display::Block
            }
        }
        "script" | "style" | "title" | "head" | "meta" | "link" | "noscript" => Display::None,
        _ => Display::Inline,
    }
}

fn default_margin(tag_name: &str) -> EdgeSizes {
    match tag_name {
        "p" => EdgeSizes::vertical(0, 12),
        "ul" | "ol" => EdgeSizes::vertical(0, 12),
        "li" => EdgeSizes::vertical(0, 4),
        "table" | "tr" => EdgeSizes::vertical(0, 8),
        "td" | "th" => EdgeSizes::vertical(0, 6),
        "hr" => EdgeSizes::vertical(10, 10),
        "blockquote" => EdgeSizes {
            top: 0,
            right: 0,
            bottom: 12,
            left: 18,
        },
        _ => EdgeSizes::default(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Legacy HTML attributes
// ─────────────────────────────────────────────────────────────────────────────

fn apply_legacy_attributes(style: &mut ComputedStyle, element: &Element, parent_font_size: u32) {
    if let Some(width) = element
        .attribute("width")
        .and_then(|value| parse_length_value(value, parent_font_size))
    {
        style.width = Some(width);
    }

    if let Some(height) = element
        .attribute("height")
        .and_then(|value| parse_length_value(value, parent_font_size))
    {
        style.height = Some(height);
    }

    if let Some(text_align) = element.attribute("align").and_then(parse_text_align) {
        style.text_align = text_align;
    }

    if let Some(vertical_align) = element.attribute("valign").and_then(parse_vertical_align) {
        style.vertical_align = vertical_align;
    }

    if let Some(background_color) = element.attribute("bgcolor").and_then(parse_color) {
        style.background_color = Some(background_color);
    }

    if let Some(color) = element.attribute("text").and_then(parse_color) {
        style.color = color;
    }

    if element.tag_name == "font" {
        if let Some(color) = element.attribute("color").and_then(parse_color) {
            style.color = color;
        }

        if let Some(size) = element.attribute("size")
            && let Some(font_size_px) = parse_legacy_font_size(size, parent_font_size)
        {
            style.font_size_px = font_size_px;
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Selector parsing
// ─────────────────────────────────────────────────────────────────────────────

fn parse_selector(input: &str) -> Option<Selector> {
    let mut raw_parts: Vec<(Option<Combinator>, String)> = Vec::new();
    let mut current = String::new();
    let mut combinator: Option<Combinator> = None;
    let chars: Vec<char> = input.trim().chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let ch = chars[i];

        if ch == '>' {
            if !current.trim().is_empty() {
                raw_parts.push((combinator.take(), current.trim().to_string()));
                current.clear();
            }
            combinator = Some(Combinator::Child);
            i += 1;
            continue;
        }

        if ch == '+' {
            if !current.trim().is_empty() {
                raw_parts.push((combinator.take(), current.trim().to_string()));
                current.clear();
            }
            combinator = Some(Combinator::AdjacentSibling);
            i += 1;
            continue;
        }

        if ch == '~' {
            if !current.trim().is_empty() {
                raw_parts.push((combinator.take(), current.trim().to_string()));
                current.clear();
            }
            combinator = Some(Combinator::GeneralSibling);
            i += 1;
            continue;
        }

        if ch.is_whitespace() {
            if !current.trim().is_empty() {
                raw_parts.push((combinator.take(), current.trim().to_string()));
                current.clear();
            }
            if !raw_parts.is_empty() && combinator.is_none() {
                combinator = Some(Combinator::Descendant);
            }
            i += 1;
            continue;
        }

        // Check for [ attribute selector ] — consume till matching ]
        if ch == '[' {
            let start = i;
            i += 1;
            let mut depth = 1;
            while i < chars.len() && depth > 0 {
                if chars[i] == '[' {
                    depth += 1;
                }
                if chars[i] == ']' {
                    depth -= 1;
                }
                i += 1;
            }
            // include the full [...]
            current.push_str(&chars[start..i].iter().collect::<String>());
            continue;
        }

        // Check for pseudo-class / pseudo-element :
        if ch == ':' {
            current.push(ch);
            i += 1;
            // double colon? (::before, ::after)
            if i < chars.len() && chars[i] == ':' {
                current.push(':');
                i += 1;
            }
            // collect ident or function (with parens)
            while i < chars.len()
                && (chars[i].is_alphanumeric() || chars[i] == '-' || chars[i] == '_')
            {
                current.push(chars[i]);
                i += 1;
            }
            // if function call with parens
            if i < chars.len() && chars[i] == '(' {
                let start = i;
                i += 1;
                let mut depth = 1;
                while i < chars.len() && depth > 0 {
                    if chars[i] == '(' {
                        depth += 1;
                    }
                    if chars[i] == ')' {
                        depth -= 1;
                    }
                    i += 1;
                }
                current.push_str(&chars[start..i].iter().collect::<String>());
            }
            continue;
        }

        current.push(ch);
        i += 1;
    }

    if !current.trim().is_empty() {
        raw_parts.push((combinator.take(), current.trim().to_string()));
    }

    let parts = raw_parts
        .into_iter()
        .filter_map(|(part_combinator, value)| {
            let simple = parse_simple_selector(&value)?;
            Some(SelectorPart {
                simple,
                combinator: part_combinator,
            })
        })
        .collect::<Vec<_>>();

    if parts.is_empty() {
        None
    } else {
        Some(Selector { parts })
    }
}

fn parse_simple_selector(input: &str) -> Option<SimpleSelector> {
    let mut selector = SimpleSelector::default();
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    let mut buffer = String::new();
    let mut mode = SelectorMode::Tag;

    while i < chars.len() {
        let ch = chars[i];

        match ch {
            '#' => {
                flush_selector_buffer(&mut selector, &mut buffer, mode);
                mode = SelectorMode::Id;
                i += 1;
            }
            '.' => {
                flush_selector_buffer(&mut selector, &mut buffer, mode);
                mode = SelectorMode::Class;
                i += 1;
            }
            '*' => {
                selector.universal = true;
                i += 1;
            }
            '[' => {
                // Attribute selector
                flush_selector_buffer(&mut selector, &mut buffer, mode);
                mode = SelectorMode::Tag; // reset
                i += 1; // skip '['
                let mut attr_content = String::new();
                while i < chars.len() && chars[i] != ']' {
                    attr_content.push(chars[i]);
                    i += 1;
                }
                if i < chars.len() {
                    i += 1;
                } // skip ']'
                if let Some(cond) = parse_attribute_condition(&attr_content) {
                    selector.attributes.push(cond);
                }
            }
            ':' => {
                flush_selector_buffer(&mut selector, &mut buffer, mode);
                mode = SelectorMode::Tag;
                i += 1;
                // pseudo-element ::
                if i < chars.len() && chars[i] == ':' {
                    // ::before / ::after → never match
                    selector.never_match = true;
                    // consume rest
                    while i < chars.len() {
                        i += 1;
                    }
                    continue;
                }
                // collect pseudo-class name
                let mut pseudo_name = String::new();
                while i < chars.len()
                    && (chars[i].is_alphanumeric() || chars[i] == '-' || chars[i] == '_')
                {
                    pseudo_name.push(chars[i]);
                    i += 1;
                }
                // function args?
                let mut args = None;
                if i < chars.len() && chars[i] == '(' {
                    i += 1; // skip (
                    let mut paren_content = String::new();
                    let mut depth = 1;
                    while i < chars.len() && depth > 0 {
                        if chars[i] == '(' {
                            depth += 1;
                        }
                        if chars[i] == ')' {
                            depth -= 1;
                        }
                        if depth > 0 {
                            paren_content.push(chars[i]);
                        }
                        i += 1;
                    }
                    args = Some(paren_content);
                }
                if let Some(pc) = parse_pseudo_class(&pseudo_name, args.as_deref()) {
                    selector.pseudo_classes.push(pc);
                }
                // ignore unknown pseudo-classes (hover, focus, etc.)
            }
            _ => {
                buffer.push(ch);
                i += 1;
            }
        }
    }

    flush_selector_buffer(&mut selector, &mut buffer, mode);

    if selector.tag_name.is_none()
        && selector.id.is_none()
        && selector.classes.is_empty()
        && !selector.universal
        && selector.pseudo_classes.is_empty()
        && selector.attributes.is_empty()
        && !selector.never_match
    {
        None
    } else {
        Some(selector)
    }
}

fn parse_pseudo_class(name: &str, args: Option<&str>) -> Option<PseudoClass> {
    match name.to_ascii_lowercase().as_str() {
        "first-child" => Some(PseudoClass::FirstChild),
        "last-child" => Some(PseudoClass::LastChild),
        "nth-child" => {
            let arg = args.unwrap_or("").trim();
            let (a, b) = parse_nth(arg);
            Some(PseudoClass::NthChild(a, b))
        }
        "not" => {
            let arg = args.unwrap_or("").trim();
            let selectors = split_at_top_level(arg, ',')
                .into_iter()
                .map(|part| parse_simple_selector(part.trim()))
                .collect::<Option<Vec<_>>>()?;
            if selectors.is_empty() {
                None
            } else {
                Some(PseudoClass::Not(selectors))
            }
        }
        // Ignored pseudo-classes (no-op)
        "hover" | "focus" | "active" | "visited" | "checked" | "disabled" | "enabled" | "link"
        | "root" | "empty" | "focus-within" | "focus-visible" | "placeholder" => None,
        _ => None,
    }
}

/// Parse CSS :nth-child argument like "odd", "even", "3", "2n", "2n+1", etc.
/// Returns (a, b) where matching condition is (1-based-index - b) % a == 0 for a != 0,
/// or index == b for a == 0.
fn parse_nth(arg: &str) -> (i32, i32) {
    let s = arg.trim().to_ascii_lowercase();
    match s.as_str() {
        "odd" => (2, 1),
        "even" => (2, 0),
        "n" => (1, 0),
        _ => {
            // try plain number
            if let Ok(n) = s.parse::<i32>() {
                return (0, n);
            }
            // try "an+b", "an-b", "an"
            if let Some(n_pos) = s.find('n') {
                let a_part = s[..n_pos].trim();
                let b_part = s[n_pos + 1..].trim();
                let a: i32 = if a_part.is_empty() || a_part == "+" {
                    1
                } else if a_part == "-" {
                    -1
                } else {
                    a_part.parse().unwrap_or(1)
                };
                let b: i32 = if b_part.is_empty() {
                    0
                } else {
                    b_part.replace('+', "").parse().unwrap_or(0)
                };
                (a, b)
            } else {
                (0, 1)
            }
        }
    }
}

fn parse_attribute_condition(content: &str) -> Option<AttributeCondition> {
    // Parse [name], [name=val], [name*=val], [name^=val], [name$=val], [name~=val], [name|=val]
    let content = content.trim();

    // Find operator
    let operators = [
        ("~=", AttrOperator::Word),
        ("|=", AttrOperator::DashPrefix),
        ("^=", AttrOperator::StartsWith),
        ("$=", AttrOperator::EndsWith),
        ("*=", AttrOperator::Contains),
        ("=", AttrOperator::Equals),
    ];

    for (op_str, op) in &operators {
        if let Some(pos) = content.find(op_str) {
            let name = content[..pos].trim().to_ascii_lowercase();
            let val = content[pos + op_str.len()..]
                .trim()
                .trim_matches('"')
                .trim_matches('\'')
                .to_string();
            return Some(AttributeCondition {
                name,
                operator: op.clone(),
                value: val,
            });
        }
    }

    // Exists only
    let name = content.trim().to_ascii_lowercase();
    if !name.is_empty() {
        Some(AttributeCondition {
            name,
            operator: AttrOperator::Exists,
            value: String::new(),
        })
    } else {
        None
    }
}

fn flush_selector_buffer(selector: &mut SimpleSelector, buffer: &mut String, mode: SelectorMode) {
    let value = buffer.trim();
    if value.is_empty() {
        buffer.clear();
        return;
    }

    match mode {
        SelectorMode::Tag => selector.tag_name = Some(value.to_ascii_lowercase()),
        SelectorMode::Id => selector.id = Some(value.to_string()),
        SelectorMode::Class => selector.classes.push(value.to_string()),
    }

    buffer.clear();
}

#[derive(Debug, Clone, Copy)]
enum SelectorMode {
    Tag,
    Id,
    Class,
}

// ─────────────────────────────────────────────────────────────────────────────
// Selector matching
// ─────────────────────────────────────────────────────────────────────────────

impl Selector {
    fn specificity(&self) -> usize {
        self.parts.iter().map(|part| part.simple.specificity()).sum()
    }

    fn matches(
        &self,
        element: &ElementIdentity,
        ancestors: &[AncestorSlot],
        sibling_index: usize,
        sibling_count: usize,
        preceding_siblings: &[ElementIdentity],
    ) -> bool {
        let Some(last_index) = self.parts.len().checked_sub(1) else {
            return false;
        };
        // Synthetic AncestorSlot for the element being matched.
        // `siblings` is intentionally left empty and `prec_count` is 0 because this slot is
        // only used to match the rightmost selector part against the element itself (tag, id,
        // class, pseudo-class, etc.).  The element's actual preceding siblings are passed
        // separately as `preceding_siblings` to `matches_part`, which is the authoritative
        // source for sibling-combinator lookups (`+`, `~`).
        // Calling `current.preceding_siblings()` would return `&[]` — always use the
        // `current_preceding_siblings` parameter in `matches_part` for the current element's
        // siblings.
        let current = AncestorSlot {
            element: element.clone(),
            sibling_index,
            sibling_count,
            siblings: empty_siblings_rc(), // shared empty Rc — no allocation per call
            prec_count: 0,
        };
        self.matches_part(last_index, &current, ancestors, preceding_siblings)
    }

    fn matches_part(
        &self,
        part_index: usize,
        current: &AncestorSlot,
        ancestors: &[AncestorSlot],
        current_preceding_siblings: &[ElementIdentity],
    ) -> bool {
        if !self.parts[part_index].simple.matches_slot(current) {
            return false;
        }

        if part_index == 0 {
            return true;
        }

        match self.parts[part_index]
            .combinator
            .unwrap_or(Combinator::Descendant)
        {
            Combinator::Descendant => {
                ancestors.iter().enumerate().rev().any(|(index, ancestor)| {
                    self.matches_part(
                        part_index - 1,
                        ancestor,
                        &ancestors[..index],
                        ancestor.preceding_siblings(),
                    )
                })
            }
            Combinator::Child => ancestors.last().is_some_and(|parent| {
                self.matches_part(
                    part_index - 1,
                    parent,
                    &ancestors[..ancestors.len() - 1],
                    parent.preceding_siblings(),
                )
            }),
            Combinator::AdjacentSibling => current_preceding_siblings
                .last()
                .is_some_and(|sibling| {
                    let sibling_index = current.sibling_index.saturating_sub(1);
                    let sibling_slot = AncestorSlot {
                        element: sibling.clone(),
                        sibling_index,
                        sibling_count: current.sibling_count,
                        siblings: empty_siblings_rc(),
                        prec_count: 0,
                    };
                    self.matches_part(
                        part_index - 1,
                        &sibling_slot,
                        ancestors,
                        &current_preceding_siblings[..sibling_index],
                    )
                }),
            Combinator::GeneralSibling => current_preceding_siblings
                .iter()
                .enumerate()
                .rev()
                .any(|(sibling_index, sibling)| {
                    let sibling_slot = AncestorSlot {
                        element: sibling.clone(),
                        sibling_index,
                        sibling_count: current.sibling_count,
                        siblings: empty_siblings_rc(),
                        prec_count: 0,
                    };
                    self.matches_part(
                        part_index - 1,
                        &sibling_slot,
                        ancestors,
                        &current_preceding_siblings[..sibling_index],
                    )
                }),
        }
    }
}

impl SimpleSelector {
    fn specificity(&self) -> usize {
        let id_score = self.id.is_some() as usize * 100;
        let non_not_pseudo_count = self
            .pseudo_classes
            .iter()
            .filter(|pc| !matches!(pc, PseudoClass::Not(_)))
            .count();
        let not_score: usize = self
            .pseudo_classes
            .iter()
            .filter_map(|pc| {
                if let PseudoClass::Not(selectors) = pc {
                    selectors.iter().map(|s| s.specificity()).max()
                } else {
                    None
                }
            })
            .sum();
        let class_score =
            (self.classes.len() + non_not_pseudo_count + self.attributes.len()) * 10;
        let tag_score = self.tag_name.is_some() as usize;
        id_score + class_score + not_score + tag_score
    }

    fn matches_slot(&self, slot: &AncestorSlot) -> bool {
        if self.never_match {
            return false;
        }

        let element = &slot.element;

        if let Some(tag_name) = &self.tag_name {
            if &element.tag_name != tag_name {
                return false;
            }
        }

        if let Some(id) = &self.id {
            if element.id.as_ref() != Some(id) {
                return false;
            }
        }

        if !self
            .classes
            .iter()
            .all(|class_name| element.classes.iter().any(|c| c == class_name))
        {
            return false;
        }

        // Attribute conditions
        for cond in &self.attributes {
            let attr_val = element
                .attributes
                .get(&cond.name)
                .map(String::as_str)
                .unwrap_or("");
            let matches = match &cond.operator {
                AttrOperator::Exists => element.attributes.contains_key(&cond.name),
                AttrOperator::Equals => attr_val == cond.value,
                AttrOperator::Contains => attr_val.contains(&cond.value),
                AttrOperator::StartsWith => attr_val.starts_with(&cond.value),
                AttrOperator::EndsWith => attr_val.ends_with(&cond.value),
                AttrOperator::Word => attr_val.split_whitespace().any(|w| w == cond.value),
                AttrOperator::DashPrefix => {
                    attr_val == cond.value || attr_val.starts_with(&format!("{}-", cond.value))
                }
            };
            if !matches {
                return false;
            }
        }

        // Pseudo-classes
        let one_based_index = slot.sibling_index + 1;
        for pc in &self.pseudo_classes {
            let matched = match pc {
                PseudoClass::FirstChild => slot.sibling_index == 0,
                PseudoClass::LastChild => slot.sibling_index + 1 == slot.sibling_count,
                PseudoClass::NthChild(a, b) => {
                    let idx = one_based_index as i32;
                    if *a == 0 {
                        idx == *b
                    } else {
                        let rem = (idx - b) % a;
                        rem == 0 && (idx - b) / a >= 0
                    }
                }
                PseudoClass::Not(selectors) => {
                    !selectors.iter().any(|selector| selector.matches_slot(slot))
                }
            };
            if !matched {
                return false;
            }
        }

        true
    }
}

impl From<&Element> for ElementIdentity {
    fn from(value: &Element) -> Self {
        let id = value.attribute("id").map(str::to_string);
        let classes = value
            .attribute("class")
            .map(|class_names| {
                class_names
                    .split_whitespace()
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        Self {
            tag_name: value.tag_name.clone(),
            id,
            classes,
            attributes: value.attributes.clone(),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Property parsers
// ─────────────────────────────────────────────────────────────────────────────

fn parse_display(input: &str) -> Option<Display> {
    match input.trim().to_ascii_lowercase().as_str() {
        "block" | "flow-root" | "flex" | "inline-flex" | "grid" | "inline-grid" | "table"
        | "table-row" => Some(Display::Block),
        "inline" | "inline-block" | "table-cell" | "contents" => Some(Display::Inline),
        "list-item" => Some(Display::ListItem),
        "none" => Some(Display::None),
        _ => None,
    }
}

fn parse_font_weight(input: &str) -> Option<bool> {
    let value = input.trim().to_ascii_lowercase();
    match value.as_str() {
        "normal" => Some(false),
        "bold" | "bolder" => Some(true),
        _ => value.parse::<u32>().ok().map(|weight| weight >= 600),
    }
}

fn parse_text_align(input: &str) -> Option<TextAlign> {
    match input.trim().to_ascii_lowercase().as_str() {
        "left" | "start" => Some(TextAlign::Left),
        "center" => Some(TextAlign::Center),
        "right" | "end" => Some(TextAlign::Right),
        _ => None,
    }
}

fn parse_vertical_align(input: &str) -> Option<VerticalAlign> {
    match input.trim().to_ascii_lowercase().as_str() {
        "top" | "text-top" => Some(VerticalAlign::Top),
        "middle" | "center" => Some(VerticalAlign::Middle),
        "bottom" | "text-bottom" => Some(VerticalAlign::Bottom),
        _ => None,
    }
}

fn parse_white_space(input: &str) -> Option<WhiteSpaceMode> {
    match input.trim().to_ascii_lowercase().as_str() {
        "normal" => Some(WhiteSpaceMode::Normal),
        "pre" | "pre-wrap" | "pre-line" => Some(WhiteSpaceMode::Pre),
        _ => None,
    }
}

fn parse_text_transform(input: &str) -> TextTransform {
    match input.trim().to_ascii_lowercase().as_str() {
        "uppercase" => TextTransform::Uppercase,
        "lowercase" => TextTransform::Lowercase,
        "capitalize" => TextTransform::Capitalize,
        _ => TextTransform::None,
    }
}

fn parse_overflow(input: &str) -> Overflow {
    match input.trim().to_ascii_lowercase().as_str() {
        "hidden" => Overflow::Hidden,
        "auto" => Overflow::Auto,
        "scroll" => Overflow::Scroll,
        _ => Overflow::Visible,
    }
}

fn parse_list_style_type(input: &str) -> ListStyleType {
    let lower = input.trim().to_ascii_lowercase();
    if lower.contains("disc") {
        return ListStyleType::Disc;
    }
    if lower.contains("circle") {
        return ListStyleType::Circle;
    }
    if lower.contains("square") {
        return ListStyleType::Square;
    }
    if lower.contains("decimal") {
        return ListStyleType::Decimal;
    }
    if lower.contains("none") {
        return ListStyleType::None;
    }
    ListStyleType::Disc
}

fn parse_box_shadow(value: &str) -> Option<BoxShadow> {
    let v = value.trim();
    if v.to_ascii_lowercase() == "none" {
        return None;
    }
    // Split tokens at spaces, respecting parentheses (for rgba/rgb colors)
    let tokens: Vec<String> = split_at_top_level(v, ' ')
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    // Skip "inset" keyword
    let tokens: Vec<&str> = tokens
        .iter()
        .filter(|t| t.to_ascii_lowercase() != "inset")
        .map(|s| s.as_str())
        .collect();

    if tokens.len() < 2 {
        return None;
    }

    let mut offset_x: i32 = 0;
    let mut offset_y: i32 = 0;
    let mut blur: u32 = 0;
    let mut color: u32 = 0;
    let mut length_count = 0;

    for token in &tokens {
        if let Some(val) = parse_signed_length(token, 16) {
            let val_i32 = val as i32;
            match length_count {
                0 => offset_x = val_i32,
                1 => offset_y = val_i32,
                2 => blur = val_i32.max(0) as u32,
                _ => {}
            }
            length_count += 1;
        } else if let Some(c) = parse_color(token) {
            color = c;
        }
    }

    if length_count < 2 {
        return None;
    }

    Some(BoxShadow {
        offset_x,
        offset_y,
        blur,
        color,
    })
}

fn parse_line_height(input: &str, parent_font_size: u32) -> u32 {
    let v = input.trim().to_ascii_lowercase();
    if v == "normal" {
        return 0;
    }
    // unitless multiplier
    if let Ok(f) = v.parse::<f32>() {
        return (f * 1000.0).round() as u32;
    }
    // px
    if let Some(rest) = v.strip_suffix("px") {
        if let Some(px) = parse_float(rest) {
            // store as em thousandths relative to parent_font_size
            let em = if parent_font_size > 0 {
                px / parent_font_size as f32
            } else {
                px / 16.0
            };
            return (em * 1000.0).round() as u32;
        }
    }
    // em
    if let Some(rest) = v.strip_suffix("em") {
        if let Some(f) = parse_float(rest) {
            return (f * 1000.0).round() as u32;
        }
    }
    // %
    if let Some(rest) = v.strip_suffix('%') {
        if let Some(f) = parse_float(rest) {
            return (f * 10.0).round() as u32; // percent/100 * 1000
        }
    }
    0
}

/// Parse a border shorthand like "1px solid red" or "none"
fn parse_border_shorthand(style: &mut ComputedStyle, value: &str, parent_font_size: u32) {
    let v = value.trim().to_ascii_lowercase();
    if v == "none" || v == "0" {
        style.border = EdgeSizes::default();
        style.border_style_none = true;
        return;
    }
    // Parse tokens: find width, color; style keyword
    for token in v.split_whitespace() {
        if token == "none" {
            style.border_style_none = true;
            continue;
        }
        if matches!(
            token,
            "solid" | "dashed" | "dotted" | "double" | "groove" | "ridge" | "inset" | "outset"
        ) {
            style.border_style_none = false;
            continue;
        }
        if let Some(px) = parse_length(token, parent_font_size) {
            style.border = EdgeSizes::all(px);
            continue;
        }
        if let Some(color) = parse_color(token) {
            style.border_color = color;
            continue;
        }
    }
}

fn parse_border_side_shorthand(
    style: &mut ComputedStyle,
    value: &str,
    parent_font_size: u32,
    side: &str,
) {
    let v = value.trim().to_ascii_lowercase();
    for token in v.split_whitespace() {
        if token == "none" {
            match side {
                "top" => style.border.top = 0,
                "right" => style.border.right = 0,
                "bottom" => style.border.bottom = 0,
                "left" => style.border.left = 0,
                _ => {}
            }
            continue;
        }
        if let Some(px) = parse_length(token, parent_font_size) {
            match side {
                "top" => style.border.top = px,
                "right" => style.border.right = px,
                "bottom" => style.border.bottom = px,
                "left" => style.border.left = px,
                _ => {}
            }
        }
    }
}

fn parse_outline_shorthand(style: &mut ComputedStyle, value: &str, parent_font_size: u32) {
    let v = value.trim().to_ascii_lowercase();
    if v == "none" {
        style.outline_width = 0;
        return;
    }
    for token in v.split_whitespace() {
        if matches!(token, "solid" | "dashed" | "dotted" | "none") {
            continue;
        }
        if let Some(px) = parse_length(token, parent_font_size) {
            style.outline_width = px;
            continue;
        }
        if let Some(color) = parse_color(token) {
            style.outline_color = Some(color);
        }
    }
}

/// Parse `font` shorthand: "bold 16px/1.5 sans-serif" or "italic bold 14px Arial"
fn parse_font_shorthand(style: &mut ComputedStyle, value: &str, parent_font_size: u32) {
    let v = value.trim().to_ascii_lowercase();
    // Split by whitespace, handle size/line-height together
    let tokens: Vec<&str> = v.split_whitespace().collect();
    for token in &tokens {
        if let Some(bold_result) = parse_font_weight(token) {
            style.font_weight = bold_result;
            continue;
        }
        if *token == "italic" || *token == "oblique" {
            style.font_style_italic = true;
            continue;
        }
        if *token == "normal" {
            continue;
        }
        // size/line-height
        if token.contains('/') {
            let parts: Vec<&str> = token.splitn(2, '/').collect();
            if let Some(size) = parse_font_size(parts[0], parent_font_size) {
                style.font_size_px = size.max(8);
            }
            if parts.len() > 1 {
                style.line_height = parse_line_height(parts[1], style.font_size_px);
            }
            continue;
        }
        // plain size
        if let Some(size) = parse_font_size(token, parent_font_size) {
            style.font_size_px = size.max(8);
            continue;
        }
        // font-family
        if let Some(ff) = parse_font_family(token) {
            style.font_family = ff;
        }
    }
}

fn parse_box_shorthand(input: &str, parent_font_size: u32) -> Option<EdgeSizes> {
    let values = input
        .split_whitespace()
        .filter_map(|part| parse_length(part, parent_font_size))
        .collect::<Vec<_>>();

    match values.as_slice() {
        [all] => Some(EdgeSizes::all(*all)),
        [vertical, horizontal] => Some(EdgeSizes {
            top: *vertical,
            right: *horizontal,
            bottom: *vertical,
            left: *horizontal,
        }),
        [top, horizontal, bottom] => Some(EdgeSizes {
            top: *top,
            right: *horizontal,
            bottom: *bottom,
            left: *horizontal,
        }),
        [top, right, bottom, left] => Some(EdgeSizes {
            top: *top,
            right: *right,
            bottom: *bottom,
            left: *left,
        }),
        _ => None,
    }
}

fn parse_font_size(input: &str, parent_font_size: u32) -> Option<u32> {
    let value = input.trim().to_ascii_lowercase();
    match value.as_str() {
        "xx-small" => Some(9),
        "x-small" => Some(10),
        "small" => Some(13),
        "medium" => Some(16),
        "large" => Some(20),
        "x-large" => Some(24),
        "xx-large" => Some(32),
        "smaller" => Some(parent_font_size.saturating_sub(2).max(8)),
        "larger" => Some(parent_font_size.saturating_add(2)),
        _ => parse_length(&value, parent_font_size),
    }
}

fn parse_legacy_font_size(input: &str, parent_font_size: u32) -> Option<u32> {
    match input.trim() {
        "1" => Some(10),
        "2" => Some(13),
        "3" => Some(16),
        "4" => Some(18),
        "5" => Some(24),
        "6" => Some(32),
        "7" => Some(48),
        value if value.starts_with('+') || value.starts_with('-') => {
            let delta = value.parse::<i32>().ok()?;
            let adjusted = parent_font_size as i32 + delta * 2;
            Some(adjusted.max(8) as u32)
        }
        _ => parse_font_size(input, parent_font_size),
    }
}

fn parse_font_family(input: &str) -> Option<FontFamilyKind> {
    let value = input.trim().to_ascii_lowercase();
    if value.contains("mono") || value.contains("code") || value.contains("console") {
        Some(FontFamilyKind::Monospace)
    } else if !value.is_empty() {
        Some(FontFamilyKind::Sans)
    } else {
        None
    }
}

/// Parse a CSS length. Handles calc(), vw/vh, px, em, rem, %
pub fn parse_length(input: &str, parent_font_size: u32) -> Option<u32> {
    let value = input.trim().to_ascii_lowercase();
    if value == "0" {
        return Some(0);
    }

    // calc()
    if let Some(inner) = value
        .strip_prefix("calc(")
        .and_then(|s| s.strip_suffix(')'))
    {
        return parse_calc(inner, parent_font_size);
    }

    if let Some(number) = value.strip_suffix("px") {
        return parse_float(number).map(|p| p.round().max(0.0) as u32);
    }

    if let Some(number) = value.strip_suffix("vw") {
        return parse_float(number).map(|p| (p * 1280.0 / 100.0).round() as u32);
    }

    if let Some(number) = value.strip_suffix("vh") {
        return parse_float(number).map(|p| (p * 800.0 / 100.0).round() as u32);
    }

    // rem must be checked before em
    if let Some(number) = value.strip_suffix("rem") {
        return parse_float(number).map(|p| (p * 16.0).round() as u32);
    }

    if let Some(number) = value.strip_suffix("em") {
        return parse_float(number).map(|p| (p * parent_font_size as f32).round() as u32);
    }

    if let Some(number) = value.strip_suffix('%') {
        return parse_float(number).map(|p| ((p / 100.0) * parent_font_size as f32).round() as u32);
    }

    parse_float(&value).map(|p| p.round().max(0.0) as u32)
}

/// Like parse_length but allows negative values; returns i16.
fn parse_signed_length(input: &str, parent_font_size: u32) -> Option<i16> {
    let value = input.trim().to_ascii_lowercase();
    if value == "0" {
        return Some(0);
    }

    if value.starts_with('-') {
        let positive = &value[1..];
        let px = parse_length(positive, parent_font_size)? as i16;
        return Some(-px);
    }

    parse_length(input, parent_font_size).map(|v| v.min(i16::MAX as u32) as i16)
}

/// Simple calc() evaluator: left-to-right, no precedence.
fn parse_calc(expr: &str, parent_font_size: u32) -> Option<u32> {
    let expr = expr.trim();

    // Tokenize: collect (operator, f32_value) pairs.
    // The first token has no operator (treated as +).
    let mut values: Vec<f32> = Vec::new();
    let mut ops: Vec<char> = Vec::new();
    let mut buf = String::new();

    let chars: Vec<char> = expr.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        match ch {
            '+' | '*' | '/' => {
                if !buf.trim().is_empty() {
                    values.push(resolve_calc_operand_f32(buf.trim(), parent_font_size)?);
                    buf.clear();
                }
                ops.push(ch);
                i += 1;
            }
            '-' if !buf.trim().is_empty() => {
                values.push(resolve_calc_operand_f32(buf.trim(), parent_font_size)?);
                buf.clear();
                ops.push('-');
                i += 1;
            }
            _ => {
                buf.push(ch);
                i += 1;
            }
        }
    }
    if !buf.trim().is_empty() {
        values.push(resolve_calc_operand_f32(buf.trim(), parent_font_size)?);
    }

    if values.is_empty() {
        return None;
    }

    // Pass 1: collapse * and / (higher precedence than + and -)
    let mut i = 0;
    while i < ops.len() {
        match ops[i] {
            '*' => {
                values[i] *= values[i + 1];
                values.remove(i + 1);
                ops.remove(i);
            }
            '/' if values[i + 1] != 0.0 => {
                values[i] /= values[i + 1];
                values.remove(i + 1);
                ops.remove(i);
            }
            _ => i += 1,
        }
    }

    // Pass 2: evaluate + and -
    let mut result = values[0];
    for (op, val) in ops.iter().zip(values[1..].iter()) {
        match op {
            '+' => result += val,
            '-' => result -= val,
            _ => {}
        }
    }

    Some(result.round().max(0.0) as u32)
}

fn resolve_calc_operand_f32(token: &str, parent_font_size: u32) -> Option<f32> {
    let t = token.trim().to_ascii_lowercase();
    // Plain number used as multiplier in * or /
    if let Ok(f) = t.parse::<f32>() {
        return Some(f);
    }
    if let Some(n) = t.strip_suffix("px") {
        return parse_float(n);
    }
    if let Some(n) = t.strip_suffix("em") {
        return parse_float(n).map(|f| f * parent_font_size as f32);
    }
    if let Some(n) = t.strip_suffix("rem") {
        return parse_float(n).map(|f| f * 16.0);
    }
    if let Some(n) = t.strip_suffix("vw") {
        return parse_float(n).map(|f| f * 12.8); // viewport 1280px wide
    }
    if let Some(n) = t.strip_suffix("vh") {
        return parse_float(n).map(|f| f * 8.0); // viewport 800px tall (matches parse_length)
    }
    if let Some(n) = t.strip_suffix('%') {
        return parse_float(n).map(|f| f * parent_font_size as f32 / 100.0);
    }
    None
}

fn parse_length_value(input: &str, parent_font_size: u32) -> Option<LengthValue> {
    let value = input.trim().to_ascii_lowercase();
    if let Some(number) = value.strip_suffix('%') {
        return parse_float(number).map(|p| LengthValue::Percent(p.round().max(0.0) as u32));
    }

    parse_length(&value, parent_font_size).map(LengthValue::Pixels)
}

fn parse_float(input: &str) -> Option<f32> {
    input.trim().parse::<f32>().ok()
}

// ─────────────────────────────────────────────────────────────────────────────
// Color parsing
// ─────────────────────────────────────────────────────────────────────────────

pub fn parse_color(input: &str) -> Option<Color> {
    let value = input.trim().to_ascii_lowercase();
    if value == "transparent" || value == "none" {
        return None;
    }
    if value == "currentcolor" {
        return None; // treat as transparent for now
    }

    if let Some(hex) = value.strip_prefix('#') {
        return parse_hex_color(hex);
    }

    if let Some(arguments) = value
        .strip_prefix("rgba(")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        let parts: Vec<&str> = arguments.split(',').collect();
        if parts.len() >= 4 {
            let r = parts[0].trim().parse::<u8>().ok()?;
            let g = parts[1].trim().parse::<u8>().ok()?;
            let b = parts[2].trim().parse::<u8>().ok()?;
            let a = parts[3].trim().parse::<f32>().ok()?.clamp(0.0, 1.0);
            if a == 0.0 {
                return None;
            }
            return Some(blend_with_white(r, g, b, a));
        }
    }

    if let Some(arguments) = value
        .strip_prefix("rgb(")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        let parts: Vec<&str> = arguments.split(',').collect();
        if parts.len() >= 3 {
            let r = parts[0].trim().parse::<u8>().ok()?;
            let g = parts[1].trim().parse::<u8>().ok()?;
            let b = parts[2].trim().parse::<u8>().ok()?;
            return Some(rgb(r, g, b));
        }
    }

    if let Some(arguments) = value
        .strip_prefix("hsla(")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        let parts: Vec<&str> = arguments.split(',').collect();
        if parts.len() >= 4 {
            let h = parts[0].trim().parse::<f32>().ok()?;
            let s = parts[1].trim().trim_end_matches('%').parse::<f32>().ok()? / 100.0;
            let l = parts[2].trim().trim_end_matches('%').parse::<f32>().ok()? / 100.0;
            let a = parts[3].trim().parse::<f32>().ok()?.clamp(0.0, 1.0);
            if a == 0.0 {
                return None;
            }
            let (r, g, b) = hsl_to_rgb(h, s, l);
            return Some(blend_with_white(r, g, b, a));
        }
    }

    if let Some(arguments) = value
        .strip_prefix("hsl(")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        let parts: Vec<&str> = arguments.split(',').collect();
        if parts.len() >= 3 {
            let h = parts[0].trim().parse::<f32>().ok()?;
            let s = parts[1].trim().trim_end_matches('%').parse::<f32>().ok()? / 100.0;
            let l = parts[2].trim().trim_end_matches('%').parse::<f32>().ok()? / 100.0;
            let (r, g, b) = hsl_to_rgb(h, s, l);
            return Some(rgb(r, g, b));
        }
    }

    parse_named_color(&value)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let h_prime = h / 60.0;
    let x = c * (1.0 - (h_prime % 2.0 - 1.0).abs());
    let (r1, g1, b1) = if h_prime < 1.0 {
        (c, x, 0.0)
    } else if h_prime < 2.0 {
        (x, c, 0.0)
    } else if h_prime < 3.0 {
        (0.0, c, x)
    } else if h_prime < 4.0 {
        (0.0, x, c)
    } else if h_prime < 5.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };
    let m = l - c / 2.0;
    let to_byte = |v: f32| ((v + m).clamp(0.0, 1.0) * 255.0).round() as u8;
    (to_byte(r1), to_byte(g1), to_byte(b1))
}

fn parse_hex_color(value: &str) -> Option<Color> {
    match value.len() {
        3 => {
            let red = u8::from_str_radix(&value[0..1].repeat(2), 16).ok()?;
            let green = u8::from_str_radix(&value[1..2].repeat(2), 16).ok()?;
            let blue = u8::from_str_radix(&value[2..3].repeat(2), 16).ok()?;
            Some(rgb(red, green, blue))
        }
        4 => {
            let red = u8::from_str_radix(&value[0..1].repeat(2), 16).ok()?;
            let green = u8::from_str_radix(&value[1..2].repeat(2), 16).ok()?;
            let blue = u8::from_str_radix(&value[2..3].repeat(2), 16).ok()?;
            let alpha = u8::from_str_radix(&value[3..4].repeat(2), 16).ok()?;
            if alpha < 128 {
                None
            } else {
                Some(rgb(red, green, blue))
            }
        }
        6 => {
            let red = u8::from_str_radix(&value[0..2], 16).ok()?;
            let green = u8::from_str_radix(&value[2..4], 16).ok()?;
            let blue = u8::from_str_radix(&value[4..6], 16).ok()?;
            Some(rgb(red, green, blue))
        }
        8 => {
            let red = u8::from_str_radix(&value[0..2], 16).ok()?;
            let green = u8::from_str_radix(&value[2..4], 16).ok()?;
            let blue = u8::from_str_radix(&value[4..6], 16).ok()?;
            let alpha = u8::from_str_radix(&value[6..8], 16).ok()?;
            if alpha < 128 {
                None
            } else {
                Some(rgb(red, green, blue))
            }
        }
        _ => None,
    }
}

fn blend_with_white(r: u8, g: u8, b: u8, alpha: f32) -> Color {
    let blend =
        |channel: u8| -> u8 { (channel as f32 * alpha + 255.0 * (1.0 - alpha)).round() as u8 };
    rgb(blend(r), blend(g), blend(b))
}

fn rgb(red: u8, green: u8, blue: u8) -> Color {
    (red as u32) << 16 | (green as u32) << 8 | blue as u32
}

fn parse_named_color(name: &str) -> Option<Color> {
    Some(match name {
        "aliceblue" => rgb(240, 248, 255),
        "antiquewhite" => rgb(250, 235, 215),
        "aqua" | "cyan" => rgb(0, 255, 255),
        "aquamarine" => rgb(127, 255, 212),
        "azure" => rgb(240, 255, 255),
        "beige" => rgb(245, 245, 220),
        "bisque" => rgb(255, 228, 196),
        "black" => rgb(0, 0, 0),
        "blanchedalmond" => rgb(255, 235, 205),
        "blue" => rgb(0, 0, 255),
        "blueviolet" => rgb(138, 43, 226),
        "brown" => rgb(165, 42, 42),
        "burlywood" => rgb(222, 184, 135),
        "cadetblue" => rgb(95, 158, 160),
        "chartreuse" => rgb(127, 255, 0),
        "chocolate" => rgb(210, 105, 30),
        "coral" => rgb(255, 127, 80),
        "cornflowerblue" => rgb(100, 149, 237),
        "cornsilk" => rgb(255, 248, 220),
        "crimson" => rgb(220, 20, 60),
        "darkblue" => rgb(0, 0, 139),
        "darkcyan" => rgb(0, 139, 139),
        "darkgoldenrod" => rgb(184, 134, 11),
        "darkgray" | "darkgrey" => rgb(169, 169, 169),
        "darkgreen" => rgb(0, 100, 0),
        "darkkhaki" => rgb(189, 183, 107),
        "darkmagenta" => rgb(139, 0, 139),
        "darkolivegreen" => rgb(85, 107, 47),
        "darkorange" => rgb(255, 140, 0),
        "darkorchid" => rgb(153, 50, 204),
        "darkred" => rgb(139, 0, 0),
        "darksalmon" => rgb(233, 150, 122),
        "darkseagreen" => rgb(143, 188, 143),
        "darkslateblue" => rgb(72, 61, 139),
        "darkslategray" | "darkslategrey" => rgb(47, 79, 79),
        "darkturquoise" => rgb(0, 206, 209),
        "darkviolet" => rgb(148, 0, 211),
        "deeppink" => rgb(255, 20, 147),
        "deepskyblue" => rgb(0, 191, 255),
        "dimgray" | "dimgrey" => rgb(105, 105, 105),
        "dodgerblue" => rgb(30, 144, 255),
        "firebrick" => rgb(178, 34, 34),
        "floralwhite" => rgb(255, 250, 240),
        "forestgreen" => rgb(34, 139, 34),
        "fuchsia" | "magenta" => rgb(255, 0, 255),
        "gainsboro" => rgb(220, 220, 220),
        "ghostwhite" => rgb(248, 248, 255),
        "gold" => rgb(255, 215, 0),
        "goldenrod" => rgb(218, 165, 32),
        "gray" | "grey" => rgb(128, 128, 128),
        "green" => rgb(0, 128, 0),
        "greenyellow" => rgb(173, 255, 47),
        "honeydew" => rgb(240, 255, 240),
        "hotpink" => rgb(255, 105, 180),
        "indianred" => rgb(205, 92, 92),
        "indigo" => rgb(75, 0, 130),
        "ivory" => rgb(255, 255, 240),
        "khaki" => rgb(240, 230, 140),
        "lavender" => rgb(230, 230, 250),
        "lavenderblush" => rgb(255, 240, 245),
        "lawngreen" => rgb(124, 252, 0),
        "lemonchiffon" => rgb(255, 250, 205),
        "lightblue" => rgb(173, 216, 230),
        "lightcoral" => rgb(240, 128, 128),
        "lightcyan" => rgb(224, 255, 255),
        "lightgoldenrodyellow" => rgb(250, 250, 210),
        "lightgray" | "lightgrey" => rgb(211, 211, 211),
        "lightgreen" => rgb(144, 238, 144),
        "lightpink" => rgb(255, 182, 193),
        "lightsalmon" => rgb(255, 160, 122),
        "lightseagreen" => rgb(32, 178, 170),
        "lightskyblue" => rgb(135, 206, 250),
        "lightslategray" | "lightslategrey" => rgb(119, 136, 153),
        "lightsteelblue" => rgb(176, 196, 222),
        "lightyellow" => rgb(255, 255, 224),
        "lime" => rgb(0, 255, 0),
        "limegreen" => rgb(50, 205, 50),
        "linen" => rgb(250, 240, 230),
        "maroon" => rgb(128, 0, 0),
        "mediumaquamarine" => rgb(102, 205, 170),
        "mediumblue" => rgb(0, 0, 205),
        "mediumorchid" => rgb(186, 85, 211),
        "mediumpurple" => rgb(147, 112, 219),
        "mediumseagreen" => rgb(60, 179, 113),
        "mediumslateblue" => rgb(123, 104, 238),
        "mediumspringgreen" => rgb(0, 250, 154),
        "mediumturquoise" => rgb(72, 209, 204),
        "mediumvioletred" => rgb(199, 21, 133),
        "midnightblue" => rgb(25, 25, 112),
        "mintcream" => rgb(245, 255, 250),
        "mistyrose" => rgb(255, 228, 225),
        "moccasin" => rgb(255, 228, 181),
        "navajowhite" => rgb(255, 222, 173),
        "navy" => rgb(0, 0, 128),
        "oldlace" => rgb(253, 245, 230),
        "olive" => rgb(128, 128, 0),
        "olivedrab" => rgb(107, 142, 35),
        "orange" => rgb(255, 165, 0),
        "orangered" => rgb(255, 69, 0),
        "orchid" => rgb(218, 112, 214),
        "palegoldenrod" => rgb(238, 232, 170),
        "palegreen" => rgb(152, 251, 152),
        "paleturquoise" => rgb(175, 238, 238),
        "palevioletred" => rgb(219, 112, 147),
        "papayawhip" => rgb(255, 239, 213),
        "peachpuff" => rgb(255, 218, 185),
        "peru" => rgb(205, 133, 63),
        "pink" => rgb(255, 192, 203),
        "plum" => rgb(221, 160, 221),
        "powderblue" => rgb(176, 224, 230),
        "purple" => rgb(128, 0, 128),
        "rebeccapurple" => rgb(102, 51, 153),
        "red" => rgb(255, 0, 0),
        "rosybrown" => rgb(188, 143, 143),
        "royalblue" => rgb(65, 105, 225),
        "saddlebrown" => rgb(139, 69, 19),
        "salmon" => rgb(250, 128, 114),
        "sandybrown" => rgb(244, 164, 96),
        "seagreen" => rgb(46, 139, 87),
        "seashell" => rgb(255, 245, 238),
        "sienna" => rgb(160, 82, 45),
        "silver" => rgb(192, 192, 192),
        "skyblue" => rgb(135, 206, 235),
        "slateblue" => rgb(106, 90, 205),
        "slategray" | "slategrey" => rgb(112, 128, 144),
        "snow" => rgb(255, 250, 250),
        "springgreen" => rgb(0, 255, 127),
        "steelblue" => rgb(70, 130, 180),
        "tan" => rgb(210, 180, 140),
        "teal" => rgb(0, 128, 128),
        "thistle" => rgb(216, 191, 216),
        "tomato" => rgb(255, 99, 71),
        "turquoise" => rgb(64, 224, 208),
        "violet" => rgb(238, 130, 238),
        "wheat" => rgb(245, 222, 179),
        "white" => rgb(255, 255, 255),
        "whitesmoke" => rgb(245, 245, 245),
        "yellow" => rgb(255, 255, 0),
        "yellowgreen" => rgb(154, 205, 50),
        _ => return None,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Comment stripping
// ─────────────────────────────────────────────────────────────────────────────

fn strip_comments(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut result = String::new();
    let mut index = 0;

    while index < bytes.len() {
        if index + 1 < bytes.len() && bytes[index] == b'/' && bytes[index + 1] == b'*' {
            index += 2;
            while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/') {
                index += 1;
            }
            index = (index + 2).min(bytes.len());
            continue;
        }

        result.push(bytes[index] as char);
        index += 1;
    }

    result
}

// ─────────────────────────────────────────────────────────────────────────────
// Text transform helper (used by layout.rs)
// ─────────────────────────────────────────────────────────────────────────────

pub fn apply_text_transform(text: &str, transform: TextTransform) -> String {
    match transform {
        TextTransform::None => text.to_string(),
        TextTransform::Uppercase => text.to_uppercase(),
        TextTransform::Lowercase => text.to_lowercase(),
        TextTransform::Capitalize => {
            let mut result = String::with_capacity(text.len());
            let mut capitalize_next = true;
            for ch in text.chars() {
                if ch.is_whitespace() {
                    capitalize_next = true;
                    result.push(ch);
                } else if capitalize_next {
                    for upper in ch.to_uppercase() {
                        result.push(upper);
                    }
                    capitalize_next = false;
                } else {
                    result.push(ch);
                }
            }
            result
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::{
        Display, LengthValue, StyledElement, StyledNode, VerticalAlign, WhiteSpaceMode,
        build_styled_tree, parse_color, parse_length, parse_stylesheet, split_at_top_level,
    };
    use crate::html::{Node, parse_document};

    fn find_first_element<'a>(
        node: &'a StyledNode,
        tag_name: &str,
    ) -> Option<&'a super::StyledElement> {
        match node {
            StyledNode::Text(_) => None,
            StyledNode::Element(element) => {
                if element.tag_name == tag_name {
                    return Some(element);
                }

                element
                    .children
                    .iter()
                    .find_map(|child| find_first_element(child, tag_name))
            }
        }
    }

    fn find_element_by_id<'a>(node: &'a StyledNode, id: &str) -> Option<&'a super::StyledElement> {
        match node {
            StyledNode::Text(_) => None,
            StyledNode::Element(element) => {
                if element
                    .attributes
                    .get("id")
                    .is_some_and(|value| value == id)
                {
                    return Some(element);
                }

                element
                    .children
                    .iter()
                    .find_map(|child| find_element_by_id(child, id))
            }
        }
    }

    #[test]
    fn parses_colors() {
        assert_eq!(parse_color("#ff00aa"), Some(0xFF00AA));
        assert_eq!(parse_color("#0fa"), Some(0x00FFAA));
        assert_eq!(parse_color("rgb(10, 20, 30)"), Some(0x0A141E));
        assert_eq!(parse_color("navy"), Some(0x000080));
    }

    #[test]
    fn applies_specificity_and_inline_styles() {
        let document = parse_document(
            "<div><p id=\"hero\" class=\"callout\" style=\"color:#00aa00; margin: 6px;\">Hello</p></div>",
        );
        let stylesheet = parse_stylesheet(
            "p { color: blue; } .callout { color: red; } #hero { font-size: 24px; white-space: pre; }",
        );

        let styled = build_styled_tree(&document, &stylesheet, 1280);
        let paragraph = find_first_element(&styled, "p").expect("paragraph should exist");

        assert_eq!(paragraph.style.color, 0x00AA00);
        assert_eq!(paragraph.style.font_size_px, 24);
        assert_eq!(paragraph.style.margin.top, 6);
        assert_eq!(paragraph.style.white_space, WhiteSpaceMode::Pre);
    }

    #[test]
    fn supports_descendant_and_child_selectors() {
        let document = parse_document(
            "<section class=\"outer\"><div><p id=\"direct\">A</p></div><p id=\"nested\">B</p></section>",
        );
        let stylesheet =
            parse_stylesheet(".outer > p { color: red; } .outer div p { display: none; }");
        let styled = build_styled_tree(&document, &stylesheet, 1280);

        let Node::Element(root) = document else {
            panic!("document root should be an element");
        };
        assert_eq!(root.tag_name, "document");

        let direct = find_first_element(&styled, "p").expect("paragraph should exist");
        assert_eq!(direct.style.display, Display::None);

        let second = match &styled {
            StyledNode::Element(root) => root
                .children
                .iter()
                .find_map(|child| find_second_paragraph(child))
                .expect("second paragraph should exist"),
            StyledNode::Text(_) => panic!("root should be an element"),
        };

        assert_eq!(second.style.color, 0xFF0000);
    }

    #[test]
    fn supports_adjacent_sibling_selector_on_target() {
        let document = parse_document(
            "<div><h1>Title</h1><p id=\"lead\">Lead</p><p id=\"body\">Body</p></div>",
        );
        let stylesheet = parse_stylesheet("h1 + p { color: #ff0000; }");
        let styled = build_styled_tree(&document, &stylesheet, 1280);

        let lead = find_element_by_id(&styled, "lead").expect("lead paragraph should exist");
        let body = find_element_by_id(&styled, "body").expect("body paragraph should exist");

        assert_eq!(lead.style.color, 0xFF0000);
        assert_ne!(body.style.color, 0xFF0000);
    }

    #[test]
    fn supports_adjacent_sibling_selector_on_ancestor_chain() {
        let document = parse_document(
            "<div><h1 id=\"heading\">Title</h1><section id=\"content\"><p id=\"text\">Hello</p></section></div>",
        );
        let stylesheet = parse_stylesheet("h1 + section p { color: #00aa00; }");
        let styled = build_styled_tree(&document, &stylesheet, 1280);

        let text = find_element_by_id(&styled, "text").expect("nested paragraph should exist");
        assert_eq!(text.style.color, 0x00AA00);
    }

    #[test]
    fn supports_chained_adjacent_and_general_sibling_selectors() {
        let document = parse_document(
            "<div><p id=\"a\">A</p><p id=\"b\">B</p><p id=\"c\">C</p><p id=\"d\">D</p></div>",
        );
        let stylesheet = parse_stylesheet(
            "p + p + p { color: #ff0000; } p#a ~ p { background-color: #0000ff; }",
        );
        let styled = build_styled_tree(&document, &stylesheet, 1280);

        let a = find_element_by_id(&styled, "a").expect("first paragraph should exist");
        let b = find_element_by_id(&styled, "b").expect("second paragraph should exist");
        let c = find_element_by_id(&styled, "c").expect("third paragraph should exist");
        let d = find_element_by_id(&styled, "d").expect("fourth paragraph should exist");

        assert_ne!(a.style.background_color, Some(0x0000FF));
        assert_eq!(b.style.background_color, Some(0x0000FF));
        assert_eq!(c.style.color, 0xFF0000);
        assert_eq!(d.style.color, 0xFF0000);
        assert_eq!(d.style.background_color, Some(0x0000FF));
    }

    #[test]
    fn supports_adjacent_sibling_then_child_combinator() {
        let document = parse_document(
            "<body><div></div><section><p id=\"target\"></p><div></div></section></body>",
        );
        let stylesheet = parse_stylesheet("div + section > p { color: #ff0000; }");
        let styled = build_styled_tree(&document, &stylesheet, 1280);

        let target = find_element_by_id(&styled, "target").expect("target paragraph should exist");
        assert_eq!(target.style.color, 0xFF0000);
    }

    #[test]
    fn supports_general_sibling_then_child_combinator() {
        let document = parse_document(
            "<body><h1></h1><p></p><div><span id=\"target\"></span></div></body>",
        );
        let stylesheet = parse_stylesheet("h1 ~ div > span { color: #00ff00; }");
        let styled = build_styled_tree(&document, &stylesheet, 1280);

        let target = find_element_by_id(&styled, "target").expect("target span should exist");
        assert_eq!(target.style.color, 0x00FF00);
    }

    #[test]
    fn applies_legacy_html_attributes() {
        let document = parse_document(
            "<body bgcolor=\"#f0f0ff\"><h1 align=\"center\">Title</h1><font color=\"#ff0000\">red</font></body>",
        );
        let styled = build_styled_tree(&document, &super::Stylesheet::default(), 1280);

        let body = find_first_element(&styled, "body").expect("body should exist");
        let heading = find_first_element(&styled, "h1").expect("heading should exist");
        let font = find_first_element(&styled, "font").expect("font should exist");

        assert_eq!(body.style.background_color, Some(0xF0F0FF));
        assert_eq!(heading.style.text_align, super::TextAlign::Center);
        assert_eq!(font.style.color, 0xFF0000);
    }

    #[test]
    fn applies_css_and_legacy_width_height_and_valign() {
        let document = parse_document(
            "<table><tr><td width=\"120\" height=\"40\" valign=\"bottom\" style=\"width: 60%;\">Hello</td></tr></table>",
        );
        let stylesheet = parse_stylesheet("td { vertical-align: middle; }");
        let styled = build_styled_tree(&document, &stylesheet, 1280);
        let cell = find_first_element(&styled, "td").expect("cell should exist");

        assert_eq!(cell.style.width, Some(LengthValue::Percent(60)));
        assert_eq!(cell.style.height, Some(LengthValue::Pixels(40)));
        assert_eq!(cell.style.vertical_align, VerticalAlign::Middle);
    }

    fn find_second_paragraph<'a>(node: &'a StyledNode) -> Option<&'a super::StyledElement> {
        fn collect<'a>(node: &'a StyledNode, output: &mut Vec<&'a super::StyledElement>) {
            match node {
                StyledNode::Text(_) => {}
                StyledNode::Element(element) => {
                    if element.tag_name == "p" {
                        output.push(element);
                    }
                    for child in &element.children {
                        collect(child, output);
                    }
                }
            }
        }

        let mut paragraphs = Vec::new();
        collect(node, &mut paragraphs);
        paragraphs.get(1).copied()
    }

    // ── Attribute selector tests ──────────────────────────────────────────────

    #[test]
    fn attribute_exists_selector_matches() {
        let document = parse_document("<div><a href=\"#\">link</a><span>plain</span></div>");
        let stylesheet = parse_stylesheet("[href] { color: #ff0000; }");
        let styled = build_styled_tree(&document, &stylesheet, 1280);
        let a = find_first_element(&styled, "a").expect("a should exist");
        let span = find_first_element(&styled, "span").expect("span should exist");
        assert_eq!(a.style.color, 0xFF0000);
        assert_ne!(span.style.color, 0xFF0000);
    }

    #[test]
    fn attribute_equals_selector_matches() {
        let document = parse_document("<input type=\"text\"><input type=\"checkbox\">");
        let stylesheet = parse_stylesheet("[type=text] { color: #00ff00; }");
        let styled = build_styled_tree(&document, &stylesheet, 1280);
        let inputs: Vec<_> = {
            fn collect_inputs<'a>(node: &'a StyledNode, out: &mut Vec<&'a StyledElement>) {
                if let StyledNode::Element(el) = node {
                    if el.tag_name == "input" {
                        out.push(el);
                    }
                    for c in &el.children {
                        collect_inputs(c, out);
                    }
                }
            }
            let mut v = Vec::new();
            collect_inputs(&styled, &mut v);
            v
        };
        assert_eq!(inputs[0].style.color, 0x00FF00);
        assert_ne!(inputs[1].style.color, 0x00FF00);
    }

    #[test]
    fn attribute_starts_with_selector_matches() {
        let document =
            parse_document("<a href=\"https://example.com\">A</a><a href=\"http://x.com\">B</a>");
        let stylesheet = parse_stylesheet("[href^=\"https\"] { color: #0000ff; }");
        let styled = build_styled_tree(&document, &stylesheet, 1280);
        fn nth_a(node: &StyledNode, n: usize) -> Option<&StyledElement> {
            let mut found = Vec::new();
            fn collect<'a>(node: &'a StyledNode, out: &mut Vec<&'a StyledElement>) {
                if let StyledNode::Element(el) = node {
                    if el.tag_name == "a" {
                        out.push(el);
                    }
                    for c in &el.children {
                        collect(c, out);
                    }
                }
            }
            collect(node, &mut found);
            found.into_iter().nth(n)
        }
        assert_eq!(nth_a(&styled, 0).unwrap().style.color, 0x0000FF);
        assert_ne!(nth_a(&styled, 1).unwrap().style.color, 0x0000FF);
    }

    // ── Pseudo-class tests ────────────────────────────────────────────────────

    #[test]
    fn first_child_selector_matches() {
        let document = parse_document("<ul><li>first</li><li>second</li><li>third</li></ul>");
        let stylesheet = parse_stylesheet("li:first-child { color: #ff0000; }");
        let styled = build_styled_tree(&document, &stylesheet, 1280);
        fn collect_li(node: &StyledNode, out: &mut Vec<u32>) {
            if let StyledNode::Element(el) = node {
                if el.tag_name == "li" {
                    out.push(el.style.color);
                }
                for c in &el.children {
                    collect_li(c, out);
                }
            }
        }
        let mut colors = Vec::new();
        collect_li(&styled, &mut colors);
        assert_eq!(colors[0], 0xFF0000, "first-child should be red");
        assert_ne!(colors[1], 0xFF0000, "second child should not be red");
    }

    #[test]
    fn last_child_selector_matches() {
        let document = parse_document("<ul><li>first</li><li>second</li><li>last</li></ul>");
        let stylesheet = parse_stylesheet("li:last-child { color: #0000ff; }");
        let styled = build_styled_tree(&document, &stylesheet, 1280);
        fn collect_li(node: &StyledNode, out: &mut Vec<u32>) {
            if let StyledNode::Element(el) = node {
                if el.tag_name == "li" {
                    out.push(el.style.color);
                }
                for c in &el.children {
                    collect_li(c, out);
                }
            }
        }
        let mut colors = Vec::new();
        collect_li(&styled, &mut colors);
        assert_ne!(colors[0], 0x0000FF, "first should not be blue");
        assert_eq!(
            *colors.last().unwrap(),
            0x0000FF,
            "last-child should be blue"
        );
    }

    #[test]
    fn nth_child_odd_even_matches() {
        let document = parse_document("<ul><li>1</li><li>2</li><li>3</li><li>4</li></ul>");
        let stylesheet = parse_stylesheet(
            "li:nth-child(odd) { color: #ff0000; } li:nth-child(even) { color: #0000ff; }",
        );
        let styled = build_styled_tree(&document, &stylesheet, 1280);
        fn collect_li(node: &StyledNode, out: &mut Vec<u32>) {
            if let StyledNode::Element(el) = node {
                if el.tag_name == "li" {
                    out.push(el.style.color);
                }
                for c in &el.children {
                    collect_li(c, out);
                }
            }
        }
        let mut colors = Vec::new();
        collect_li(&styled, &mut colors);
        assert_eq!(colors[0], 0xFF0000, "1st (odd) should be red");
        assert_eq!(colors[1], 0x0000FF, "2nd (even) should be blue");
        assert_eq!(colors[2], 0xFF0000, "3rd (odd) should be red");
        assert_eq!(colors[3], 0x0000FF, "4th (even) should be blue");
    }

    #[test]
    fn not_selector_excludes_matching_elements() {
        let document = parse_document("<ul><li class=\"skip\">A</li><li>B</li><li>C</li></ul>");
        let stylesheet = parse_stylesheet("li:not(.skip) { color: #00ff00; }");
        let styled = build_styled_tree(&document, &stylesheet, 1280);
        fn collect_li(node: &StyledNode, out: &mut Vec<u32>) {
            if let StyledNode::Element(el) = node {
                if el.tag_name == "li" {
                    out.push(el.style.color);
                }
                for c in &el.children {
                    collect_li(c, out);
                }
            }
        }
        let mut colors = Vec::new();
        collect_li(&styled, &mut colors);
        assert_ne!(colors[0], 0x00FF00, ".skip li should not match :not(.skip)");
        assert_eq!(colors[1], 0x00FF00, "plain li should match :not(.skip)");
    }

    #[test]
    fn not_selector_list_excludes_any_matching_selector() {
        let document =
            parse_document("<ul><li class=\"skip\">A</li><li class=\"omit\">B</li><li>C</li></ul>");
        let stylesheet = parse_stylesheet("li:not(.skip, .omit) { color: #00ff00; }");
        let styled = build_styled_tree(&document, &stylesheet, 1280);
        fn collect_li(node: &StyledNode, out: &mut Vec<u32>) {
            if let StyledNode::Element(el) = node {
                if el.tag_name == "li" {
                    out.push(el.style.color);
                }
                for c in &el.children {
                    collect_li(c, out);
                }
            }
        }
        let mut colors = Vec::new();
        collect_li(&styled, &mut colors);
        assert_ne!(colors[0], 0x00FF00, ".skip li should not match selector list in :not()");
        assert_ne!(colors[1], 0x00FF00, ".omit li should not match selector list in :not()");
        assert_eq!(colors[2], 0x00FF00, "plain li should match selector list in :not()");
    }

    // ── @media tests ─────────────────────────────────────────────────────────

    #[test]
    fn media_max_width_filters_rules_by_viewport() {
        let document = parse_document("<p>Hello</p>");
        // Base rule first, then media rule — at narrow viewport the media rule
        // comes later in source order so it wins (same specificity).
        let stylesheet = parse_stylesheet(
            "p { color: #0000ff; } @media (max-width: 600px) { p { color: #ff0000; } }",
        );
        // Viewport 1280 → max-width 600 rule should NOT apply, base rule wins
        let styled_wide = build_styled_tree(&document, &stylesheet, 1280);
        let p_wide = find_first_element(&styled_wide, "p").unwrap();
        assert_eq!(
            p_wide.style.color, 0x0000FF,
            "wide viewport: plain rule wins"
        );

        // Viewport 400 → max-width 600 rule SHOULD apply and wins (later in source)
        let styled_narrow = build_styled_tree(&document, &stylesheet, 400);
        let p_narrow = find_first_element(&styled_narrow, "p").unwrap();
        assert_eq!(
            p_narrow.style.color, 0xFF0000,
            "narrow viewport: media rule wins"
        );
    }

    #[test]
    fn media_nested_braces_are_parsed_correctly() {
        // @media with multiple rules inside — previously the first } broke the parse
        let document = parse_document("<p class=\"a\">A</p><p class=\"b\">B</p>");
        let stylesheet =
            parse_stylesheet("@media screen { .a { color: #ff0000; } .b { color: #0000ff; } }");
        let styled = build_styled_tree(&document, &stylesheet, 1280);
        let a = find_first_element(&styled, "p").unwrap();
        // Both rules inside @media screen should be parsed (screen always applies)
        assert_eq!(
            a.style.color, 0xFF0000,
            "first rule inside @media should apply"
        );
    }

    // ── calc() tests ──────────────────────────────────────────────────────────

    #[test]
    fn calc_addition_and_subtraction() {
        let document = parse_document("<p>text</p>");
        let stylesheet = parse_stylesheet("p { font-size: calc(10px + 6px); }");
        let styled = build_styled_tree(&document, &stylesheet, 1280);
        let p = find_first_element(&styled, "p").unwrap();
        assert_eq!(p.style.font_size_px, 16);
    }

    #[test]
    fn calc_multiplication_has_higher_precedence_than_addition() {
        // calc(2px + 3 * 4px) should be 2 + 12 = 14, NOT (2+3)*4 = 20
        let document = parse_document("<p>text</p>");
        let stylesheet = parse_stylesheet("p { font-size: calc(2px + 3 * 4px); }");
        let styled = build_styled_tree(&document, &stylesheet, 1280);
        let p = find_first_element(&styled, "p").unwrap();
        assert_eq!(
            p.style.font_size_px, 14,
            "multiplication must bind tighter than addition"
        );
    }

    #[test]
    fn calc_em_multiplication() {
        // calc(1.5 * 1em) at 16px parent → 24px
        let document = parse_document("<p>text</p>");
        let stylesheet = parse_stylesheet("p { font-size: calc(1.5 * 1em); }");
        let styled = build_styled_tree(&document, &stylesheet, 1280);
        let p = find_first_element(&styled, "p").unwrap();
        assert_eq!(p.style.font_size_px, 24);
    }

    #[test]
    fn calc_vh_uses_800px_base() {
        // 50vh should resolve to 400px (50% of 800px viewport height)
        // This locks the vh base against parse_length's viewport-unit handling
        let result = parse_length("calc(50vh)", 16);
        assert_eq!(result, Some(400));
    }

    // ── rgba() blending tests ─────────────────────────────────────────────────

    #[test]
    fn rgba_fully_opaque_returns_color() {
        assert_eq!(parse_color("rgba(255, 0, 0, 1.0)"), Some(0xFF0000));
    }

    #[test]
    fn rgba_fully_transparent_returns_none() {
        assert_eq!(parse_color("rgba(255, 0, 0, 0.0)"), None);
    }

    #[test]
    fn rgba_half_transparent_blends_with_white() {
        // rgba(0, 0, 0, 0.5) should blend 50% black with white → rgb(128, 128, 128)
        let color = parse_color("rgba(0, 0, 0, 0.5)").expect("should return a color");
        let r = (color >> 16) & 0xFF;
        let g = (color >> 8) & 0xFF;
        let b = color & 0xFF;
        assert!((r as i32 - 128).abs() <= 1, "r should be ~128, got {r}");
        assert!((g as i32 - 128).abs() <= 1, "g should be ~128, got {g}");
        assert!((b as i32 - 128).abs() <= 1, "b should be ~128, got {b}");
    }

    // ── split_at_top_level tests ──────────────────────────────────────────────

    #[test]
    fn split_comma_at_top_level_ignores_parens() {
        // :not(.a, .b) must NOT be split on the inner comma
        let result = split_at_top_level(":not(.a, .b), .c", ',');
        assert_eq!(result, vec![":not(.a, .b)".to_string(), " .c".to_string()]);
    }

    #[test]
    fn split_semicolon_at_top_level_ignores_string() {
        // content: "a; b" must NOT be split inside the string
        let result = split_at_top_level(r#"color: red; content: "a; b""#, ';');
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].trim(), "color: red");
        assert_eq!(result[1].trim(), r#"content: "a; b""#);
    }

    #[test]
    fn not_pseudo_class_selector_matches() {
        let document = parse_document("<p class=\"a\">A</p><p class=\"b\">B</p>");
        let stylesheet = parse_stylesheet("p:not(.a) { color: #ff0000; }");
        let styled = build_styled_tree(&document, &stylesheet, 1280);
        let pa = find_first_element(&styled, "p").unwrap();
        // first p has class "a" so :not(.a) should NOT match it
        assert_ne!(pa.style.color, 0xFF0000, "p.a should not match :not(.a)");
    }

    #[test]
    fn nested_inline_opacity_stacking_context_resets() {
        // CSS spec: opacity < 1 creates a stacking context for ALL elements, including inline.
        // The span (opacity: 0.5) is a stacking context boundary; em resets to its own opacity.
        //
        // Note: inline elements do not emit a LayerCommand, so the span's 50% opacity is NOT
        // applied via offscreen compositing — it is an approximation.  The em's effective_opacity
        // is reset to its own opacity (128) at the stacking context boundary, matching the
        // block-element path for consistency.  Pixel-perfect inline group compositing would
        // require a LayerCommand for inline opacity runs (future work).
        let document = parse_document("<body><span><em>hi</em></span></body>");
        let stylesheet = parse_stylesheet("span { opacity: 0.5; } em { opacity: 0.5; }");
        let styled = build_styled_tree(&document, &stylesheet, 1280);
        let em = find_first_element(&styled, "em").expect("em element should exist");
        // em.effective_opacity == em.opacity (128) because span is a stacking context boundary.
        assert_eq!(
            em.style.effective_opacity, 128,
            "inline stacking context should reset effective_opacity to child's own opacity"
        );
    }
    #[test]
    fn test_root_css_variable_inheritance() {
        use crate::html::parse_document;
        let css_text = r#":root { --color: #ff0000; } p { color: var(--color); }"#;
        let html = r#"<html><head></head><body><p>Hello</p></body></html>"#;
        let doc = parse_document(html);
        let stylesheet = parse_stylesheet(css_text);
        let styled = build_styled_tree(&doc, &stylesheet, 800);

        fn find_p(node: &StyledNode) -> Option<&StyledElement> {
            match node {
                StyledNode::Element(el) if el.tag_name == "p" => Some(el),
                StyledNode::Element(el) => el.children.iter().find_map(find_p),
                _ => None,
            }
        }

        let p_el = find_p(&styled).expect("Should find <p> element");
        assert_eq!(p_el.style.color, 0xff0000, "p color should be #ff0000 from :root var");
    }
}