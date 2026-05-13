use crate::css::{
    Color, ComputedStyle, DEFAULT_BACKGROUND_COLOR, Display, FontFamilyKind, StyledElement,
    StyledNode, TextAlign, WhiteSpaceMode,
};
use crate::font::FontContext;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutDocument {
    pub background_color: Color,
    pub content_height: u32,
    pub rects: Vec<RectCommand>,
    pub texts: Vec<TextCommand>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RectCommand {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub color: Color,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextCommand {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub text: String,
    pub font_size_px: u32,
    pub font_family: FontFamilyKind,
    pub color: Color,
    pub underline: bool,
    pub bold: bool,
}

pub fn layout_styled_document(
    document: &StyledNode,
    viewport_width: u32,
    fonts: &mut FontContext,
) -> LayoutDocument {
    let mut context = LayoutContext::default();
    let background_color = find_document_background(document).unwrap_or(DEFAULT_BACKGROUND_COLOR);
    let mut cursor_y = 0;

    layout_node(
        document,
        0,
        viewport_width,
        &mut cursor_y,
        &mut context,
        fonts,
    );

    LayoutDocument {
        background_color,
        content_height: cursor_y,
        rects: context.rects,
        texts: context.texts,
    }
}

#[derive(Default)]
struct LayoutContext {
    rects: Vec<RectCommand>,
    texts: Vec<TextCommand>,
}

#[derive(Debug, Clone)]
enum InlineFragment {
    Text { text: String, style: ComputedStyle },
    LineBreak,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LineSpan {
    text: String,
    width: u32,
    style: ComputedStyle,
}

#[derive(Debug, Default)]
struct LineBuilder {
    spans: Vec<LineSpan>,
    width: u32,
    line_height: u32,
}

impl LineBuilder {
    fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    fn push_span(&mut self, text: &str, style: &ComputedStyle, fonts: &mut FontContext) {
        if text.is_empty() {
            return;
        }

        let width = text_width(style, text, fonts);
        self.width = self.width.saturating_add(width);
        self.line_height = self.line_height.max(text_line_height(style, fonts));

        if let Some(last) = self.spans.last_mut() {
            if last.style == *style {
                last.text.push_str(text);
                last.width = last.width.saturating_add(width);
                return;
            }
        }

        self.spans.push(LineSpan {
            text: text.to_string(),
            width,
            style: style.clone(),
        });
    }
}

fn layout_node(
    node: &StyledNode,
    x: u32,
    width: u32,
    cursor_y: &mut u32,
    context: &mut LayoutContext,
    fonts: &mut FontContext,
) {
    match node {
        StyledNode::Text(text) => {
            let fragments = [InlineFragment::Text {
                text: text.text.clone(),
                style: text.style.clone(),
            }];
            layout_inline_fragments(&fragments, &text.style, x, width, cursor_y, context, fonts);
        }
        StyledNode::Element(element) => match element.style.display {
            Display::None => {}
            Display::Inline => {
                let fragments = flatten_inline_fragments(node);
                layout_inline_fragments(
                    &fragments,
                    &element.style,
                    x,
                    width,
                    cursor_y,
                    context,
                    fonts,
                );
            }
            Display::Block | Display::ListItem => {
                layout_block_element(element, x, width, cursor_y, context, fonts);
            }
        },
    }
}

fn layout_block_element(
    element: &StyledElement,
    x: u32,
    width: u32,
    cursor_y: &mut u32,
    context: &mut LayoutContext,
    fonts: &mut FontContext,
) {
    if element.tag_name == "br" {
        *cursor_y = cursor_y.saturating_add(text_line_height(&element.style, fonts));
        return;
    }

    if element.tag_name == "img" {
        let alt = element
            .attributes
            .get("alt")
            .filter(|text| !text.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| "[image]".to_string());
        let fragments = [InlineFragment::Text {
            text: alt,
            style: element.style.clone(),
        }];
        layout_inline_fragments(
            &fragments,
            &element.style,
            x,
            width,
            cursor_y,
            context,
            fonts,
        );
        *cursor_y = cursor_y.saturating_add(element.style.margin.bottom);
        return;
    }

    *cursor_y = cursor_y.saturating_add(element.style.margin.top);

    let outer_x = x.saturating_add(element.style.margin.left);
    let outer_width = width.saturating_sub(element.style.margin.left + element.style.margin.right);
    let background_top = *cursor_y;

    *cursor_y = cursor_y.saturating_add(element.style.padding.top);

    let bullet_indent = if element.style.display == Display::ListItem {
        16
    } else {
        0
    };

    let content_x = outer_x
        .saturating_add(element.style.padding.left)
        .saturating_add(bullet_indent);
    let content_width = outer_width
        .saturating_sub(element.style.padding.left + element.style.padding.right + bullet_indent)
        .max(1);

    if element.tag_name == "hr" {
        context.rects.push(RectCommand {
            x: content_x,
            y: *cursor_y,
            width: content_width,
            height: 2,
            color: element.style.color,
        });
        *cursor_y = cursor_y.saturating_add(10);
    } else {
        layout_mixed_children(
            element,
            content_x,
            content_width,
            cursor_y,
            context,
            bullet_indent > 0,
            fonts,
        );
    }

    *cursor_y = cursor_y.saturating_add(element.style.padding.bottom);
    let background_height = cursor_y.saturating_sub(background_top).max(1);

    if let Some(background_color) = element.style.background_color {
        context.rects.push(RectCommand {
            x: outer_x,
            y: background_top,
            width: outer_width.max(1),
            height: background_height,
            color: background_color,
        });
    }

    *cursor_y = cursor_y.saturating_add(element.style.margin.bottom);
}

fn layout_mixed_children(
    element: &StyledElement,
    x: u32,
    width: u32,
    cursor_y: &mut u32,
    context: &mut LayoutContext,
    needs_bullet: bool,
    fonts: &mut FontContext,
) {
    let mut inline_fragments = Vec::new();
    let mut bullet_pending = needs_bullet;

    for child in &element.children {
        if is_hidden(child) {
            continue;
        }

        if is_block_level(child) {
            if !inline_fragments.is_empty() || bullet_pending {
                if bullet_pending {
                    inline_fragments.insert(
                        0,
                        InlineFragment::Text {
                            text: "- ".to_string(),
                            style: element.style.clone(),
                        },
                    );
                }
                layout_inline_fragments(
                    &inline_fragments,
                    &element.style,
                    x,
                    width,
                    cursor_y,
                    context,
                    fonts,
                );
                inline_fragments.clear();
                bullet_pending = false;
            }

            layout_node(child, x, width, cursor_y, context, fonts);
        } else {
            if bullet_pending {
                inline_fragments.push(InlineFragment::Text {
                    text: "- ".to_string(),
                    style: element.style.clone(),
                });
                bullet_pending = false;
            }
            collect_inline_fragments(child, &mut inline_fragments);
        }
    }

    if !inline_fragments.is_empty() || bullet_pending {
        if bullet_pending {
            inline_fragments.push(InlineFragment::Text {
                text: "- ".to_string(),
                style: element.style.clone(),
            });
        }
        layout_inline_fragments(
            &inline_fragments,
            &element.style,
            x,
            width,
            cursor_y,
            context,
            fonts,
        );
    }
}

fn collect_inline_fragments(node: &StyledNode, output: &mut Vec<InlineFragment>) {
    match node {
        StyledNode::Text(text) => {
            output.push(InlineFragment::Text {
                text: text.text.clone(),
                style: text.style.clone(),
            });
        }
        StyledNode::Element(element) => match element.style.display {
            Display::None => {}
            Display::Inline => {
                if element.tag_name == "br" {
                    output.push(InlineFragment::LineBreak);
                    return;
                }

                if element.tag_name == "img" {
                    let alt = element
                        .attributes
                        .get("alt")
                        .filter(|text| !text.trim().is_empty())
                        .cloned()
                        .unwrap_or_else(|| "[image]".to_string());
                    output.push(InlineFragment::Text {
                        text: alt,
                        style: element.style.clone(),
                    });
                    return;
                }

                for child in &element.children {
                    collect_inline_fragments(child, output);
                }
            }
            Display::Block | Display::ListItem => {}
        },
    }
}

fn flatten_inline_fragments(node: &StyledNode) -> Vec<InlineFragment> {
    let mut fragments = Vec::new();
    collect_inline_fragments(node, &mut fragments);
    fragments
}

fn layout_inline_fragments(
    fragments: &[InlineFragment],
    container_style: &ComputedStyle,
    x: u32,
    width: u32,
    cursor_y: &mut u32,
    context: &mut LayoutContext,
    fonts: &mut FontContext,
) {
    if fragments.is_empty() {
        return;
    }

    if container_style.white_space == WhiteSpaceMode::Pre {
        layout_preformatted_fragments(
            fragments,
            container_style,
            x,
            width,
            cursor_y,
            context,
            fonts,
        );
    } else {
        layout_normal_fragments(
            fragments,
            container_style,
            x,
            width,
            cursor_y,
            context,
            fonts,
        );
    }
}

fn layout_normal_fragments(
    fragments: &[InlineFragment],
    container_style: &ComputedStyle,
    x: u32,
    width: u32,
    cursor_y: &mut u32,
    context: &mut LayoutContext,
    fonts: &mut FontContext,
) {
    let mut line = LineBuilder::default();
    let mut pending_space = false;

    for fragment in fragments {
        match fragment {
            InlineFragment::LineBreak => {
                emit_line(
                    &mut line,
                    container_style,
                    x,
                    width,
                    cursor_y,
                    context,
                    fonts,
                );
                pending_space = false;
            }
            InlineFragment::Text { text, style } => {
                let had_whitespace = text.chars().any(char::is_whitespace);

                for word in text.split_whitespace() {
                    if pending_space && !line.is_empty() {
                        let space_width = char_width(style, ' ', fonts);
                        if line.width.saturating_add(space_width) > width {
                            emit_line(
                                &mut line,
                                container_style,
                                x,
                                width,
                                cursor_y,
                                context,
                                fonts,
                            );
                        } else {
                            line.push_span(" ", style, fonts);
                        }
                    }

                    push_wrapped_word(
                        word,
                        style,
                        container_style,
                        x,
                        width,
                        cursor_y,
                        context,
                        &mut line,
                        fonts,
                    );
                    pending_space = true;
                }

                if had_whitespace {
                    pending_space = true;
                }
            }
        }
    }

    emit_line(
        &mut line,
        container_style,
        x,
        width,
        cursor_y,
        context,
        fonts,
    );
}

fn layout_preformatted_fragments(
    fragments: &[InlineFragment],
    container_style: &ComputedStyle,
    x: u32,
    width: u32,
    cursor_y: &mut u32,
    context: &mut LayoutContext,
    fonts: &mut FontContext,
) {
    let mut line = LineBuilder::default();

    for fragment in fragments {
        match fragment {
            InlineFragment::LineBreak => emit_line(
                &mut line,
                container_style,
                x,
                width,
                cursor_y,
                context,
                fonts,
            ),
            InlineFragment::Text { text, style } => {
                for character in text.chars() {
                    if character == '\n' {
                        emit_line(
                            &mut line,
                            container_style,
                            x,
                            width,
                            cursor_y,
                            context,
                            fonts,
                        );
                        continue;
                    }

                    let character_width = char_width(style, character, fonts);
                    if !line.is_empty() && line.width.saturating_add(character_width) > width {
                        emit_line(
                            &mut line,
                            container_style,
                            x,
                            width,
                            cursor_y,
                            context,
                            fonts,
                        );
                    }

                    let mut buffer = [0_u8; 4];
                    line.push_span(character.encode_utf8(&mut buffer), style, fonts);
                }
            }
        }
    }

    emit_line(
        &mut line,
        container_style,
        x,
        width,
        cursor_y,
        context,
        fonts,
    );
}

fn push_wrapped_word(
    word: &str,
    style: &ComputedStyle,
    container_style: &ComputedStyle,
    x: u32,
    width: u32,
    cursor_y: &mut u32,
    context: &mut LayoutContext,
    line: &mut LineBuilder,
    fonts: &mut FontContext,
) {
    let word_width = text_width(style, word, fonts);
    if word_width <= width {
        if !line.is_empty() && line.width.saturating_add(word_width) > width {
            emit_line(line, container_style, x, width, cursor_y, context, fonts);
        }
        line.push_span(word, style, fonts);
        return;
    }

    let avg_char_width = char_width(style, 'M', fonts).max(1);
    let max_chars = (width / avg_char_width).max(1) as usize;
    let mut chunk = String::new();

    for character in word.chars() {
        chunk.push(character);
        if chunk.chars().count() == max_chars {
            if !line.is_empty() {
                emit_line(line, container_style, x, width, cursor_y, context, fonts);
            }
            line.push_span(&chunk, style, fonts);
            emit_line(line, container_style, x, width, cursor_y, context, fonts);
            chunk.clear();
        }
    }

    if !chunk.is_empty() {
        if !line.is_empty() && line.width.saturating_add(text_width(style, &chunk, fonts)) > width {
            emit_line(line, container_style, x, width, cursor_y, context, fonts);
        }
        line.push_span(&chunk, style, fonts);
    }
}

fn emit_line(
    line: &mut LineBuilder,
    container_style: &ComputedStyle,
    x: u32,
    width: u32,
    cursor_y: &mut u32,
    context: &mut LayoutContext,
    fonts: &mut FontContext,
) {
    if line.is_empty() {
        *cursor_y = cursor_y.saturating_add(text_line_height(container_style, fonts));
        return;
    }

    let line_width = line.width.min(width);
    let offset_x = match container_style.text_align {
        TextAlign::Left => 0,
        TextAlign::Center => width.saturating_sub(line_width) / 2,
        TextAlign::Right => width.saturating_sub(line_width),
    };

    let mut cursor_x = x.saturating_add(offset_x);
    let line_height = line
        .line_height
        .max(text_line_height(container_style, fonts));

    for span in &line.spans {
        if let Some(background_color) = span.style.background_color {
            context.rects.push(RectCommand {
                x: cursor_x,
                y: *cursor_y,
                width: span.width,
                height: line_height,
                color: background_color,
            });
        }

        context.texts.push(TextCommand {
            x: cursor_x,
            y: *cursor_y,
            width: span.width,
            text: span.text.clone(),
            font_size_px: span.style.font_size_px,
            font_family: span.style.font_family,
            color: span.style.color,
            underline: span.style.underline,
            bold: span.style.font_weight,
        });

        cursor_x = cursor_x.saturating_add(span.width);
    }

    *cursor_y = cursor_y.saturating_add(line_height);
    line.spans.clear();
    line.width = 0;
    line.line_height = 0;
}

fn is_block_level(node: &StyledNode) -> bool {
    matches!(
        node,
        StyledNode::Element(StyledElement {
            style: ComputedStyle {
                display: Display::Block | Display::ListItem,
                ..
            },
            ..
        })
    )
}

fn is_hidden(node: &StyledNode) -> bool {
    matches!(
        node,
        StyledNode::Element(StyledElement {
            style: ComputedStyle {
                display: Display::None,
                ..
            },
            ..
        })
    )
}

fn char_width(style: &ComputedStyle, character: char, fonts: &mut FontContext) -> u32 {
    fonts.glyph_advance_px(character, style.font_size_px, style.font_family)
}

fn text_line_height(style: &ComputedStyle, fonts: &mut FontContext) -> u32 {
    fonts.line_height_px(style.font_size_px, style.font_family)
}

fn text_width(style: &ComputedStyle, text: &str, fonts: &mut FontContext) -> u32 {
    fonts.text_width_px(text, style.font_size_px, style.font_family)
}

fn find_document_background(node: &StyledNode) -> Option<Color> {
    match node {
        StyledNode::Text(_) => None,
        StyledNode::Element(element) => {
            if matches!(element.tag_name.as_str(), "body" | "html" | "document") {
                if let Some(background_color) = element.style.background_color {
                    return Some(background_color);
                }
            }

            element.children.iter().find_map(find_document_background)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::layout_styled_document;
    use crate::css::{TextAlign, build_styled_tree, parse_stylesheet};
    use crate::font::FontContext;
    use crate::html::parse_document;

    #[test]
    fn hides_display_none_content() {
        let document = parse_document("<div><p>Hello</p><span class=\"hide\">Nope</span></div>");
        let stylesheet = parse_stylesheet(".hide { display: none; } p { color: #ff0000; }");
        let styled = build_styled_tree(&document, &stylesheet);
        let mut fonts = FontContext::load();
        let layout = layout_styled_document(&styled, 320, &mut fonts);

        assert!(layout.texts.iter().any(|text| text.text.contains("Hello")));
        assert!(layout.texts.iter().all(|text| !text.text.contains("Nope")));
        assert!(layout.texts.iter().any(|text| text.color == 0xFF0000));
    }

    #[test]
    fn centers_text_when_requested() {
        let document = parse_document("<p>Hello</p>");
        let stylesheet = parse_stylesheet("p { text-align: center; font-size: 16px; }");
        let styled = build_styled_tree(&document, &stylesheet);
        let mut fonts = FontContext::load();
        let layout = layout_styled_document(&styled, 200, &mut fonts);

        let text = layout.texts.first().expect("text command should exist");
        let expected_left_offset = (200 - text.width) / 2;

        assert_eq!(text.x, expected_left_offset);
    }

    #[test]
    fn wraps_text_across_multiple_lines() {
        let document = parse_document("<p>alpha beta gamma delta epsilon</p>");
        let stylesheet = parse_stylesheet("p { font-size: 16px; }");
        let styled = build_styled_tree(&document, &stylesheet);
        let mut fonts = FontContext::load();
        let layout = layout_styled_document(&styled, 90, &mut fonts);

        let distinct_rows = layout
            .texts
            .iter()
            .map(|text| text.y)
            .collect::<std::collections::BTreeSet<_>>();

        assert!(distinct_rows.len() >= 2);
    }

    #[test]
    fn keeps_text_align_inherited() {
        let document = parse_document("<div><p>Hello</p></div>");
        let stylesheet = parse_stylesheet("div { text-align: right; }");
        let styled = build_styled_tree(&document, &stylesheet);

        let paragraph = match styled {
            crate::css::StyledNode::Element(ref root) => {
                find_paragraph(root).expect("paragraph should be present")
            }
            crate::css::StyledNode::Text(_) => panic!("root should be an element"),
        };

        assert_eq!(paragraph.style.text_align, TextAlign::Right);
    }

    fn find_paragraph(element: &crate::css::StyledElement) -> Option<&crate::css::StyledElement> {
        if element.tag_name == "p" {
            return Some(element);
        }

        element.children.iter().find_map(|child| match child {
            crate::css::StyledNode::Text(_) => None,
            crate::css::StyledNode::Element(child) => find_paragraph(child),
        })
    }
}
