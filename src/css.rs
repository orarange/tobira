use std::collections::BTreeMap;

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
}

impl Stylesheet {
    pub fn extend(&mut self, other: Stylesheet) {
        self.rules.extend(other.rules);
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
    Not(Box<SimpleSelector>),
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
            font_family: parent.map(|s| s.font_family).unwrap_or(FontFamilyKind::Sans),
            text_align: parent.map(|s| s.text_align).unwrap_or(TextAlign::Left),
            vertical_align: VerticalAlign::Top,
            font_weight: parent.map(|s| s.font_weight).unwrap_or(false),
            underline: parent.map(|s| s.underline).unwrap_or(false),
            white_space: parent.map(|s| s.white_space).unwrap_or(WhiteSpaceMode::Normal),
            // new fields – most not inherited
            border: EdgeSizes::default(),
            border_color: parent.map(|s| s.color).unwrap_or(DEFAULT_TEXT_COLOR),
            border_style_none: false,
            border_radius: 0,
            outline_width: 0,
            outline_color: None,
            line_height: parent.map(|s| s.line_height).unwrap_or(0),
            opacity: 255,
            font_style_italic: parent.map(|s| s.font_style_italic).unwrap_or(false),
            text_transform: parent.map(|s| s.text_transform).unwrap_or(TextTransform::None),
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

pub fn parse_stylesheet(input: &str) -> Stylesheet {
    let mut rules = Vec::new();
    let source = strip_comments(input);
    let mut cursor = 0;

    while let Some(open_offset) = source[cursor..].find('{') {
        let selector_start = cursor;
        let selector_end = cursor + open_offset;
        let block_start = selector_end + 1;

        // We must find the matching closing brace (possibly nested for @media)
        let block_text_raw = &source[block_start..];
        let Some(close_offset) = block_text_raw.find('}') else {
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

        let selectors = selector_text
            .split(',')
            .filter_map(parse_selector)
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

    Stylesheet { rules }
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
    strip_comments(input)
        .split(';')
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

pub fn build_styled_tree(document: &Node, stylesheet: &Stylesheet, viewport_width: u32) -> StyledNode {
    let ancestors = Vec::new();
    build_node(document, stylesheet, None, &ancestors, 0, 0, &[], viewport_width)
}

fn build_node(
    node: &Node,
    stylesheet: &Stylesheet,
    parent_style: Option<&ComputedStyle>,
    ancestors: &[ElementIdentity],
    sibling_index: usize,
    sibling_count: usize,
    preceding_siblings: &[ElementIdentity],
    viewport_width: u32,
) -> StyledNode {
    match node {
        Node::Text(text) => StyledNode::Text(StyledText {
            text: text.clone(),
            style: parent_style
                .cloned()
                .unwrap_or_else(|| ComputedStyle::for_element("body", None)),
        }),
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
            let mut next_ancestors = ancestors.to_vec();
            next_ancestors.push(ElementIdentity::from(element));

            // Count element children and build sibling info
            let element_children: Vec<&Node> = element
                .children
                .iter()
                .filter(|c| matches!(c, Node::Element(_)))
                .collect();
            let child_element_count = element_children.len();

            let mut elem_sibling_idx = 0;
            let mut preceding: Vec<ElementIdentity> = Vec::new();

            let children = element
                .children
                .iter()
                .map(|child| {
                    let (idx, count, prec) = if matches!(child, Node::Element(_)) {
                        let idx = elem_sibling_idx;
                        let prec_snap = preceding.clone();
                        if let Node::Element(e) = child {
                            preceding.push(ElementIdentity::from(e));
                        }
                        elem_sibling_idx += 1;
                        (idx, child_element_count, prec_snap)
                    } else {
                        (0, 0, Vec::new())
                    };
                    build_node(
                        child,
                        stylesheet,
                        Some(&style),
                        &next_ancestors,
                        idx,
                        count,
                        &prec,
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
    ancestors: &[ElementIdentity],
    sibling_index: usize,
    sibling_count: usize,
    preceding_siblings: &[ElementIdentity],
    viewport_width: u32,
) -> ComputedStyle {
    let mut style = ComputedStyle::for_element(&element.tag_name, parent_style);
    let parent_font_size = parent_style.map(|c| c.font_size_px).unwrap_or(16);
    apply_legacy_attributes(&mut style, element, parent_font_size);

    let identity = ElementIdentity::from(element);
    let mut css_variables: BTreeMap<String, String> = BTreeMap::new();
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
                applicable.extend(
                    rule.declarations.iter().cloned().enumerate().map(
                        |(declaration_index, declaration)| {
                            (
                                selector.specificity(),
                                rule_index * 100 + declaration_index,
                                declaration,
                            )
                        },
                    ),
                );
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
        result = format!("{}{}{}", &result[..start], replacement, &result[inner_start + end + 1..]);
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
                if chars[i] == '[' { depth += 1; }
                if chars[i] == ']' { depth -= 1; }
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
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '-' || chars[i] == '_') {
                current.push(chars[i]);
                i += 1;
            }
            // if function call with parens
            if i < chars.len() && chars[i] == '(' {
                let start = i;
                i += 1;
                let mut depth = 1;
                while i < chars.len() && depth > 0 {
                    if chars[i] == '(' { depth += 1; }
                    if chars[i] == ')' { depth -= 1; }
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
                if i < chars.len() { i += 1; } // skip ']'
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
                    while i < chars.len() { i += 1; }
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
                        if chars[i] == '(' { depth += 1; }
                        if chars[i] == ')' { depth -= 1; }
                        if depth > 0 { paren_content.push(chars[i]); }
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
            let inner = parse_simple_selector(arg)?;
            Some(PseudoClass::Not(Box::new(inner)))
        }
        // Ignored pseudo-classes (no-op)
        "hover" | "focus" | "active" | "visited" | "checked" | "disabled" | "enabled"
        | "link" | "root" | "empty" | "focus-within" | "focus-visible" | "placeholder" => None,
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
    let operators = [("~=", AttrOperator::Word), ("|=", AttrOperator::DashPrefix),
                     ("^=", AttrOperator::StartsWith), ("$=", AttrOperator::EndsWith),
                     ("*=", AttrOperator::Contains), ("=", AttrOperator::Equals)];

    for (op_str, op) in &operators {
        if let Some(pos) = content.find(op_str) {
            let name = content[..pos].trim().to_ascii_lowercase();
            let val = content[pos + op_str.len()..].trim().trim_matches('"').trim_matches('\'').to_string();
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
        self.parts
            .iter()
            .map(|part| {
                part.simple.id.iter().count() * 100
                    + (part.simple.classes.len()
                        + part.simple.pseudo_classes.len()
                        + part.simple.attributes.len())
                        * 10
                    + part.simple.tag_name.iter().count()
            })
            .sum()
    }

    fn matches(
        &self,
        element: &ElementIdentity,
        ancestors: &[ElementIdentity],
        sibling_index: usize,
        sibling_count: usize,
        preceding_siblings: &[ElementIdentity],
    ) -> bool {
        let Some(last_part) = self.parts.last() else {
            return false;
        };
        if !last_part.simple.matches_element(element, sibling_index, sibling_count, preceding_siblings) {
            return false;
        }

        if self.parts.len() == 1 {
            return true;
        }

        let mut ancestor_index = ancestors.len();
        // preceding_siblings not threaded into ancestor matching for simplicity
        for part_index in (0..self.parts.len() - 1).rev() {
            let part = &self.parts[part_index];
            let combinator = self.parts[part_index + 1]
                .combinator
                .unwrap_or(Combinator::Descendant);
            match combinator {
                Combinator::Descendant => {
                    let mut found = false;
                    while ancestor_index > 0 {
                        ancestor_index -= 1;
                        if part.simple.matches_element(&ancestors[ancestor_index], 0, 0, &[]) {
                            found = true;
                            break;
                        }
                    }
                    if !found {
                        return false;
                    }
                }
                Combinator::Child => {
                    if ancestor_index == 0 {
                        return false;
                    }
                    ancestor_index -= 1;
                    if !part.simple.matches_element(&ancestors[ancestor_index], 0, 0, &[]) {
                        return false;
                    }
                }
                // Sibling combinators: for now we only partially support them
                // (we'd need sibling data threaded into ancestor matching)
                Combinator::AdjacentSibling | Combinator::GeneralSibling => {
                    // Check preceding_siblings only for the immediately preceding element
                    if part_index == self.parts.len() - 2 {
                        // this is directly preceding the target
                        let found = match combinator {
                            Combinator::AdjacentSibling => {
                                preceding_siblings.last().map_or(false, |sib| {
                                    part.simple.matches_element(sib, 0, 0, &[])
                                })
                            }
                            Combinator::GeneralSibling => {
                                preceding_siblings.iter().any(|sib| {
                                    part.simple.matches_element(sib, 0, 0, &[])
                                })
                            }
                            _ => false,
                        };
                        if !found {
                            return false;
                        }
                    } else {
                        return false;
                    }
                }
            }
        }

        true
    }
}

impl SimpleSelector {
    fn matches_element(
        &self,
        element: &ElementIdentity,
        sibling_index: usize,
        sibling_count: usize,
        preceding_siblings: &[ElementIdentity],
    ) -> bool {
        if self.never_match {
            return false;
        }

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
            let attr_val = element.attributes.get(&cond.name).map(String::as_str).unwrap_or("");
            let matches = match &cond.operator {
                AttrOperator::Exists => element.attributes.contains_key(&cond.name),
                AttrOperator::Equals => attr_val == cond.value,
                AttrOperator::Contains => attr_val.contains(&cond.value),
                AttrOperator::StartsWith => attr_val.starts_with(&cond.value),
                AttrOperator::EndsWith => attr_val.ends_with(&cond.value),
                AttrOperator::Word => attr_val.split_whitespace().any(|w| w == cond.value),
                AttrOperator::DashPrefix => {
                    attr_val == cond.value
                        || attr_val.starts_with(&format!("{}-", cond.value))
                }
            };
            if !matches {
                return false;
            }
        }

        // Pseudo-classes
        let one_based_index = sibling_index + 1;
        for pc in &self.pseudo_classes {
            let matched = match pc {
                PseudoClass::FirstChild => sibling_index == 0,
                PseudoClass::LastChild => sibling_index + 1 == sibling_count,
                PseudoClass::NthChild(a, b) => {
                    let idx = one_based_index as i32;
                    if *a == 0 {
                        idx == *b
                    } else {
                        let rem = (idx - b) % a;
                        rem == 0 && (idx - b) / a >= 0
                    }
                }
                PseudoClass::Not(inner) => {
                    !inner.matches_element(element, sibling_index, sibling_count, preceding_siblings)
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
        "block" | "flow-root" | "flex" | "inline-flex" | "grid" | "inline-grid"
        | "table" | "table-row" => Some(Display::Block),
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
    if lower.contains("disc") { return ListStyleType::Disc; }
    if lower.contains("circle") { return ListStyleType::Circle; }
    if lower.contains("square") { return ListStyleType::Square; }
    if lower.contains("decimal") { return ListStyleType::Decimal; }
    if lower.contains("none") { return ListStyleType::None; }
    ListStyleType::Disc
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
            let em = if parent_font_size > 0 { px / parent_font_size as f32 } else { px / 16.0 };
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
        if matches!(token, "solid" | "dashed" | "dotted" | "double" | "groove" | "ridge" | "inset" | "outset") {
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
    if let Some(inner) = value.strip_prefix("calc(").and_then(|s| s.strip_suffix(')')) {
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
        return parse_float(number)
            .map(|p| ((p / 100.0) * parent_font_size as f32).round() as u32);
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
    // Tokenize into numbers and operators
    let mut tokens: Vec<CalcToken> = Vec::new();
    let mut buf = String::new();

    let chars: Vec<char> = expr.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch == '+' || ch == '*' || ch == '/' {
            if !buf.trim().is_empty() {
                let num = resolve_calc_operand(buf.trim(), parent_font_size)?;
                tokens.push(CalcToken::Value(num));
                buf.clear();
            }
            tokens.push(CalcToken::Op(ch));
            i += 1;
        } else if ch == '-' && !buf.trim().is_empty() {
            // minus between operands
            let num = resolve_calc_operand(buf.trim(), parent_font_size)?;
            tokens.push(CalcToken::Value(num));
            buf.clear();
            tokens.push(CalcToken::Op('-'));
            i += 1;
        } else {
            buf.push(ch);
            i += 1;
        }
    }
    if !buf.trim().is_empty() {
        let num = resolve_calc_operand(buf.trim(), parent_font_size)?;
        tokens.push(CalcToken::Value(num));
    }

    // Evaluate left-to-right
    let mut result: f32 = 0.0;
    let mut pending_op = '+';
    for token in &tokens {
        match token {
            CalcToken::Value(v) => {
                let f = *v as f32;
                match pending_op {
                    '+' => result += f,
                    '-' => result -= f,
                    '*' => result *= f,
                    '/' => {
                        if f != 0.0 {
                            result /= f;
                        }
                    }
                    _ => {}
                }
            }
            CalcToken::Op(op) => {
                pending_op = *op;
            }
        }
    }

    Some(result.round().max(0.0) as u32)
}

enum CalcToken {
    Value(u32),
    Op(char),
}

fn resolve_calc_operand(token: &str, parent_font_size: u32) -> Option<u32> {
    // numbers without units (used in multiplication/division)
    if let Ok(f) = token.parse::<f32>() {
        return Some(f.round().max(0.0) as u32);
    }
    parse_length(token, parent_font_size)
}

fn parse_length_value(input: &str, parent_font_size: u32) -> Option<LengthValue> {
    let value = input.trim().to_ascii_lowercase();
    if let Some(number) = value.strip_suffix('%') {
        return parse_float(number)
            .map(|p| LengthValue::Percent(p.round().max(0.0) as u32));
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
            let a = parts[3].trim().parse::<f32>().ok()?;
            if a < 0.5 {
                return None;
            }
            return Some(rgb(r, g, b));
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
            let a = parts[3].trim().parse::<f32>().ok()?;
            if a < 0.5 {
                return None;
            }
            let (r, g, b) = hsl_to_rgb(h, s, l);
            return Some(rgb(r, g, b));
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
            if alpha < 128 { None } else { Some(rgb(red, green, blue)) }
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
            if alpha < 128 { None } else { Some(rgb(red, green, blue)) }
        }
        _ => None,
    }
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
        Display, LengthValue, StyledNode, VerticalAlign, WhiteSpaceMode, build_styled_tree,
        parse_color, parse_stylesheet,
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
}
