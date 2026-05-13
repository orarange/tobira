use std::collections::BTreeMap;

use crate::html::{Element, Node};

pub type Color = u32;

pub const DEFAULT_TEXT_COLOR: Color = 0x1D232E;
pub const DEFAULT_BACKGROUND_COLOR: Color = 0xFFFDF8;
pub const DEFAULT_LINK_COLOR: Color = 0x2A5DB0;

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
}

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
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SimpleSelector {
    tag_name: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
    universal: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    property: String,
    value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ElementIdentity {
    tag_name: String,
    id: Option<String>,
    classes: Vec<String>,
}

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
pub enum WhiteSpaceMode {
    Normal,
    Pre,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FontFamilyKind {
    Sans,
    Monospace,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComputedStyle {
    pub display: Display,
    pub color: Color,
    pub background_color: Option<Color>,
    pub margin: EdgeSizes,
    pub padding: EdgeSizes,
    pub font_size_px: u32,
    pub font_family: FontFamilyKind,
    pub text_align: TextAlign,
    pub font_weight: bool,
    pub underline: bool,
    pub white_space: WhiteSpaceMode,
}

impl ComputedStyle {
    fn for_element(tag_name: &str, parent: Option<&Self>) -> Self {
        let parent_font_size = parent.map(|style| style.font_size_px).unwrap_or(16);
        let mut style = Self {
            display: default_display(tag_name),
            color: parent
                .map(|style| style.color)
                .unwrap_or(DEFAULT_TEXT_COLOR),
            background_color: None,
            margin: default_margin(tag_name),
            padding: EdgeSizes::default(),
            font_size_px: parent_font_size,
            font_family: parent
                .map(|style| style.font_family)
                .unwrap_or(FontFamilyKind::Sans),
            text_align: parent
                .map(|style| style.text_align)
                .unwrap_or(TextAlign::Left),
            font_weight: parent.map(|style| style.font_weight).unwrap_or(false),
            underline: parent.map(|style| style.underline).unwrap_or(false),
            white_space: parent
                .map(|style| style.white_space)
                .unwrap_or(WhiteSpaceMode::Normal),
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

pub fn parse_stylesheet(input: &str) -> Stylesheet {
    let mut rules = Vec::new();
    let source = strip_comments(input);
    let mut cursor = 0;

    while let Some(open_offset) = source[cursor..].find('{') {
        let selector_start = cursor;
        let selector_end = cursor + open_offset;
        let block_start = selector_end + 1;

        let Some(close_offset) = source[block_start..].find('}') else {
            break;
        };
        let block_end = block_start + close_offset;

        let selector_text = source[selector_start..selector_end].trim();
        let block_text = source[block_start..block_end].trim();
        cursor = block_end + 1;

        if selector_text.is_empty() || block_text.is_empty() {
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
            });
        }
    }

    Stylesheet { rules }
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

pub fn build_styled_tree(document: &Node, stylesheet: &Stylesheet) -> StyledNode {
    let ancestors = Vec::new();
    build_node(document, stylesheet, None, &ancestors)
}

fn build_node(
    node: &Node,
    stylesheet: &Stylesheet,
    parent_style: Option<&ComputedStyle>,
    ancestors: &[ElementIdentity],
) -> StyledNode {
    match node {
        Node::Text(text) => StyledNode::Text(StyledText {
            text: text.clone(),
            style: parent_style
                .cloned()
                .unwrap_or_else(|| ComputedStyle::for_element("body", None)),
        }),
        Node::Element(element) => {
            let style = compute_style(element, stylesheet, parent_style, ancestors);
            let mut next_ancestors = ancestors.to_vec();
            next_ancestors.push(ElementIdentity::from(element));

            let children = element
                .children
                .iter()
                .map(|child| build_node(child, stylesheet, Some(&style), &next_ancestors))
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

fn compute_style(
    element: &Element,
    stylesheet: &Stylesheet,
    parent_style: Option<&ComputedStyle>,
    ancestors: &[ElementIdentity],
) -> ComputedStyle {
    let mut style = ComputedStyle::for_element(&element.tag_name, parent_style);
    let parent_font_size = parent_style
        .map(|computed| computed.font_size_px)
        .unwrap_or(16);
    apply_legacy_attributes(&mut style, element, parent_font_size);
    let mut applicable = Vec::new();
    let identity = ElementIdentity::from(element);

    for (rule_index, rule) in stylesheet.rules.iter().enumerate() {
        for selector in &rule.selectors {
            if selector.matches(&identity, ancestors) {
                applicable.extend(rule.declarations.iter().cloned().enumerate().map(
                    |(declaration_index, declaration)| {
                        (
                            selector.specificity(),
                            rule_index * 100 + declaration_index,
                            declaration,
                        )
                    },
                ));
            }
        }
    }

    if let Some(inline_style) = element.attribute("style") {
        applicable.extend(
            parse_inline_declarations(inline_style)
                .into_iter()
                .enumerate()
                .map(|(index, declaration)| (1_000, usize::MAX - 1_000 + index, declaration)),
        );
    }

    applicable.sort_by_key(|(specificity, order, _)| (*specificity, *order));

    for (_, _, declaration) in applicable {
        apply_declaration(&mut style, &declaration, parent_font_size);
    }

    style
}

fn apply_declaration(style: &mut ComputedStyle, declaration: &Declaration, parent_font_size: u32) {
    match declaration.property.as_str() {
        "color" => {
            if let Some(color) = parse_color(&declaration.value) {
                style.color = color;
            }
        }
        "background" | "background-color" => {
            style.background_color = parse_color(&declaration.value);
        }
        "display" => {
            if let Some(display) = parse_display(&declaration.value) {
                style.display = display;
            }
        }
        "font-size" => {
            if let Some(font_size) = parse_font_size(&declaration.value, parent_font_size) {
                style.font_size_px = font_size.max(8);
            }
        }
        "font-family" => {
            if let Some(font_family) = parse_font_family(&declaration.value) {
                style.font_family = font_family;
            }
        }
        "font-weight" => {
            style.font_weight = parse_font_weight(&declaration.value).unwrap_or(style.font_weight);
        }
        "text-align" => {
            if let Some(text_align) = parse_text_align(&declaration.value) {
                style.text_align = text_align;
            }
        }
        "text-decoration" => {
            style.underline = parse_underline(&declaration.value).unwrap_or(style.underline);
        }
        "white-space" => {
            if let Some(white_space) = parse_white_space(&declaration.value) {
                style.white_space = white_space;
            }
        }
        "margin" => {
            if let Some(edges) = parse_box_shorthand(&declaration.value, parent_font_size) {
                style.margin = edges;
            }
        }
        "padding" => {
            if let Some(edges) = parse_box_shorthand(&declaration.value, parent_font_size) {
                style.padding = edges;
            }
        }
        "margin-top" => {
            if let Some(value) = parse_length(&declaration.value, parent_font_size) {
                style.margin.top = value;
            }
        }
        "margin-right" => {
            if let Some(value) = parse_length(&declaration.value, parent_font_size) {
                style.margin.right = value;
            }
        }
        "margin-bottom" => {
            if let Some(value) = parse_length(&declaration.value, parent_font_size) {
                style.margin.bottom = value;
            }
        }
        "margin-left" => {
            if let Some(value) = parse_length(&declaration.value, parent_font_size) {
                style.margin.left = value;
            }
        }
        "padding-top" => {
            if let Some(value) = parse_length(&declaration.value, parent_font_size) {
                style.padding.top = value;
            }
        }
        "padding-right" => {
            if let Some(value) = parse_length(&declaration.value, parent_font_size) {
                style.padding.right = value;
            }
        }
        "padding-bottom" => {
            if let Some(value) = parse_length(&declaration.value, parent_font_size) {
                style.padding.bottom = value;
            }
        }
        "padding-left" => {
            if let Some(value) = parse_length(&declaration.value, parent_font_size) {
                style.padding.left = value;
            }
        }
        _ => {}
    }
}

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

fn apply_legacy_attributes(style: &mut ComputedStyle, element: &Element, parent_font_size: u32) {
    if let Some(text_align) = element.attribute("align").and_then(parse_text_align) {
        style.text_align = text_align;
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

fn parse_selector(input: &str) -> Option<Selector> {
    let mut raw_parts = Vec::new();
    let mut current = String::new();
    let mut combinator = None;

    for character in input.trim().chars() {
        if character == '>' {
            if !current.trim().is_empty() {
                raw_parts.push((combinator.take(), current.trim().to_string()));
                current.clear();
            }
            combinator = Some(Combinator::Child);
            continue;
        }

        if character.is_whitespace() {
            if !current.trim().is_empty() {
                raw_parts.push((combinator.take(), current.trim().to_string()));
                current.clear();
            }
            if !raw_parts.is_empty() && combinator.is_none() {
                combinator = Some(Combinator::Descendant);
            }
            continue;
        }

        current.push(character);
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
    let mut buffer = String::new();
    let mut mode = SelectorMode::Tag;

    for character in input.chars() {
        match character {
            '#' => {
                flush_selector_buffer(&mut selector, &mut buffer, mode);
                mode = SelectorMode::Id;
            }
            '.' => {
                flush_selector_buffer(&mut selector, &mut buffer, mode);
                mode = SelectorMode::Class;
            }
            '*' => {
                selector.universal = true;
            }
            _ => buffer.push(character),
        }
    }

    flush_selector_buffer(&mut selector, &mut buffer, mode);

    if selector.tag_name.is_none()
        && selector.id.is_none()
        && selector.classes.is_empty()
        && !selector.universal
    {
        None
    } else {
        Some(selector)
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

impl Selector {
    fn specificity(&self) -> usize {
        self.parts
            .iter()
            .map(|part| {
                part.simple.id.iter().count() * 100
                    + part.simple.classes.len() * 10
                    + part.simple.tag_name.iter().count()
            })
            .sum()
    }

    fn matches(&self, element: &ElementIdentity, ancestors: &[ElementIdentity]) -> bool {
        let Some(last_part) = self.parts.last() else {
            return false;
        };
        if !last_part.simple.matches(element) {
            return false;
        }

        if self.parts.len() == 1 {
            return true;
        }

        let mut ancestor_index = ancestors.len();
        for part_index in (0..self.parts.len() - 1).rev() {
            let part = &self.parts[part_index];
            match self.parts[part_index + 1]
                .combinator
                .unwrap_or(Combinator::Descendant)
            {
                Combinator::Descendant => {
                    let mut found = false;
                    while ancestor_index > 0 {
                        ancestor_index -= 1;
                        if part.simple.matches(&ancestors[ancestor_index]) {
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
                    if !part.simple.matches(&ancestors[ancestor_index]) {
                        return false;
                    }
                }
            }
        }

        true
    }
}

impl SimpleSelector {
    fn matches(&self, element: &ElementIdentity) -> bool {
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

        self.classes
            .iter()
            .all(|class_name| element.classes.iter().any(|class| class == class_name))
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
        }
    }
}

fn parse_display(input: &str) -> Option<Display> {
    match input.trim().to_ascii_lowercase().as_str() {
        "block" => Some(Display::Block),
        "inline" | "inline-block" => Some(Display::Inline),
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

fn parse_underline(input: &str) -> Option<bool> {
    let value = input.trim().to_ascii_lowercase();
    if value.contains("underline") {
        Some(true)
    } else if value.contains("none") {
        Some(false)
    } else {
        None
    }
}

fn parse_white_space(input: &str) -> Option<WhiteSpaceMode> {
    match input.trim().to_ascii_lowercase().as_str() {
        "normal" => Some(WhiteSpaceMode::Normal),
        "pre" | "pre-wrap" | "pre-line" => Some(WhiteSpaceMode::Pre),
        _ => None,
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

fn parse_length(input: &str, parent_font_size: u32) -> Option<u32> {
    let value = input.trim().to_ascii_lowercase();
    if value == "0" {
        return Some(0);
    }

    if let Some(number) = value.strip_suffix("px") {
        return parse_float(number).map(|parsed| parsed.round().max(0.0) as u32);
    }

    if let Some(number) = value.strip_suffix("em") {
        return parse_float(number).map(|parsed| (parsed * parent_font_size as f32).round() as u32);
    }

    if let Some(number) = value.strip_suffix("rem") {
        return parse_float(number).map(|parsed| (parsed * 16.0).round() as u32);
    }

    if let Some(number) = value.strip_suffix('%') {
        return parse_float(number)
            .map(|parsed| ((parsed / 100.0) * parent_font_size as f32).round() as u32);
    }

    parse_float(&value).map(|parsed| parsed.round().max(0.0) as u32)
}

fn parse_float(input: &str) -> Option<f32> {
    input.trim().parse::<f32>().ok()
}

fn parse_color(input: &str) -> Option<Color> {
    let value = input.trim().to_ascii_lowercase();
    if value == "transparent" {
        return None;
    }

    if let Some(hex) = value.strip_prefix('#') {
        return parse_hex_color(hex);
    }

    if let Some(arguments) = value
        .strip_prefix("rgb(")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        let parts = arguments
            .split(',')
            .filter_map(|part| part.trim().parse::<u8>().ok())
            .collect::<Vec<_>>();
        if let [red, green, blue] = parts.as_slice() {
            return Some(rgb(*red, *green, *blue));
        }
    }

    match value.as_str() {
        "black" => Some(rgb(0, 0, 0)),
        "white" => Some(rgb(255, 255, 255)),
        "red" => Some(rgb(255, 0, 0)),
        "green" => Some(rgb(0, 128, 0)),
        "blue" => Some(rgb(0, 0, 255)),
        "navy" => Some(rgb(0, 0, 128)),
        "teal" => Some(rgb(0, 128, 128)),
        "yellow" => Some(rgb(255, 255, 0)),
        "orange" => Some(rgb(255, 165, 0)),
        "purple" => Some(rgb(128, 0, 128)),
        "gray" | "grey" => Some(rgb(128, 128, 128)),
        "silver" => Some(rgb(192, 192, 192)),
        "maroon" => Some(rgb(128, 0, 0)),
        "lime" => Some(rgb(0, 255, 0)),
        "aqua" => Some(rgb(0, 255, 255)),
        _ => None,
    }
}

fn parse_hex_color(value: &str) -> Option<Color> {
    match value.len() {
        3 => {
            let red = u8::from_str_radix(&value[0..1].repeat(2), 16).ok()?;
            let green = u8::from_str_radix(&value[1..2].repeat(2), 16).ok()?;
            let blue = u8::from_str_radix(&value[2..3].repeat(2), 16).ok()?;
            Some(rgb(red, green, blue))
        }
        6 => {
            let red = u8::from_str_radix(&value[0..2], 16).ok()?;
            let green = u8::from_str_radix(&value[2..4], 16).ok()?;
            let blue = u8::from_str_radix(&value[4..6], 16).ok()?;
            Some(rgb(red, green, blue))
        }
        _ => None,
    }
}

fn rgb(red: u8, green: u8, blue: u8) -> Color {
    (red as u32) << 16 | (green as u32) << 8 | blue as u32
}

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

#[cfg(test)]
mod tests {
    use super::{
        Display, StyledNode, WhiteSpaceMode, build_styled_tree, parse_color, parse_stylesheet,
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

        let styled = build_styled_tree(&document, &stylesheet);
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
        let styled = build_styled_tree(&document, &stylesheet);

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
        let styled = build_styled_tree(&document, &super::Stylesheet::default());

        let body = find_first_element(&styled, "body").expect("body should exist");
        let heading = find_first_element(&styled, "h1").expect("heading should exist");
        let font = find_first_element(&styled, "font").expect("font should exist");

        assert_eq!(body.style.background_color, Some(0xF0F0FF));
        assert_eq!(heading.style.text_align, super::TextAlign::Center);
        assert_eq!(font.style.color, 0xFF0000);
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
