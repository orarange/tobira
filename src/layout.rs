use crate::css::{
    Color, ComputedStyle, DEFAULT_BACKGROUND_COLOR, Display, FontFamilyKind, LengthValue,
    StyledElement, StyledNode, TextAlign, TextTransform, VerticalAlign, WhiteSpaceMode,
    apply_text_transform,
};
use crate::font::FontContext;
use crate::image::ImageStore;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutDocument {
    pub background_color: Color,
    pub content_height: u32,
    pub rects: Vec<RectCommand>,
    pub texts: Vec<TextCommand>,
    pub images: Vec<ImageCommand>,
    pub links: Vec<LinkCommand>,
    pub controls: Vec<FormControlCommand>,
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
    pub italic: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageCommand {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub src: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkCommand {
    pub node_id: Option<usize>,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub href: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormControlKind {
    TextInput,
    Button,
    Hidden,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormControlCommand {
    pub id: usize,
    pub node_id: Option<usize>,
    pub form_node_id: Option<usize>,
    pub kind: FormControlKind,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub name: Option<String>,
    pub value: String,
    pub label: String,
    pub placeholder: Option<String>,
    pub form_id: Option<usize>,
    pub form_action: Option<String>,
    pub form_method: String,
    pub activates_submit: bool,
    pub disabled: bool,
    pub masked: bool,
    pub font_size_px: u32,
    pub font_family: FontFamilyKind,
    pub text_color: Color,
    pub background_color: Color,
    pub border_color: Color,
}

pub fn layout_styled_document(
    document: &StyledNode,
    images: &ImageStore,
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
        images,
        fonts,
        None,
    );

    LayoutDocument {
        background_color,
        content_height: cursor_y,
        rects: context.rects,
        texts: context.texts,
        images: context.images,
        links: context.links,
        controls: context.controls,
    }
}

struct LayoutContext {
    rects: Vec<RectCommand>,
    texts: Vec<TextCommand>,
    images: Vec<ImageCommand>,
    links: Vec<LinkCommand>,
    controls: Vec<FormControlCommand>,
    next_control_id: usize,
    next_form_id: usize,
}

impl Default for LayoutContext {
    fn default() -> Self {
        Self {
            rects: Vec::new(),
            texts: Vec::new(),
            images: Vec::new(),
            links: Vec::new(),
            controls: Vec::new(),
            next_control_id: 0,
            next_form_id: 0,
        }
    }
}

impl LayoutContext {
    fn allocate_control_id(&mut self) -> usize {
        let id = self.next_control_id;
        self.next_control_id = self
            .next_control_id
            .checked_add(1)
            .expect("control id counter overflowed");
        id
    }

    fn allocate_form_id(&mut self) -> usize {
        let id = self.next_form_id;
        self.next_form_id = self
            .next_form_id
            .checked_add(1)
            .expect("form id counter overflowed");
        id
    }
}

#[derive(Debug, Clone)]
enum InlineFragment {
    Text {
        text: String,
        style: ComputedStyle,
        link_href: Option<String>,
        link_node_id: Option<usize>,
    },
    Control(FormControlSpec),
    LineBreak,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LineSpan {
    text: String,
    width: u32,
    height: u32,
    style: ComputedStyle,
    link_href: Option<String>,
    link_node_id: Option<usize>,
    control: Option<FormControlSpec>,
}

#[derive(Debug, Default)]
struct LineBuilder {
    spans: Vec<LineSpan>,
    width: u32,
    line_height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FormContext {
    id: usize,
    node_id: Option<usize>,
    action: Option<String>,
    method: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FormControlSpec {
    id: usize,
    node_id: Option<usize>,
    form_node_id: Option<usize>,
    kind: FormControlKind,
    style: ComputedStyle,
    name: Option<String>,
    value: String,
    placeholder: Option<String>,
    label: String,
    form_id: Option<usize>,
    form_action: Option<String>,
    form_method: String,
    activates_submit: bool,
    disabled: bool,
    masked: bool,
    size_chars: Option<u32>,
}

impl LineBuilder {
    fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    fn push_span(
        &mut self,
        text: &str,
        style: &ComputedStyle,
        fonts: &mut FontContext,
        link_href: Option<&str>,
        link_node_id: Option<usize>,
    ) {
        if text.is_empty() {
            return;
        }

        let width = text_width(style, text, fonts);
        self.width = self.width.saturating_add(width);
        let height = text_line_height(style, fonts);
        self.line_height = self.line_height.max(height);

        if let Some(last) = self.spans.last_mut() {
            if last.control.is_none()
                && last.style == *style
                && last.link_href.as_deref() == link_href
                && last.link_node_id == link_node_id
            {
                last.text.push_str(text);
                last.width = last.width.saturating_add(width);
                return;
            }
        }

        self.spans.push(LineSpan {
            text: text.to_string(),
            width,
            height,
            style: style.clone(),
            link_href: link_href.map(str::to_string),
            link_node_id,
            control: None,
        });
    }

    fn push_control(&mut self, control: &FormControlSpec, fonts: &mut FontContext) {
        let (width, height) = measure_form_control(control, fonts);
        self.width = self.width.saturating_add(width);
        self.line_height = self.line_height.max(height);
        self.spans.push(LineSpan {
            text: control.label.clone(),
            width,
            height,
            style: control.style.clone(),
            link_href: None,
            link_node_id: None,
            control: Some(control.clone()),
        });
    }
}

fn form_context_for_element(
    element: &StyledElement,
    context: &mut LayoutContext,
    current_form: Option<FormContext>,
) -> Option<FormContext> {
    if element.tag_name != "form" {
        return current_form;
    }

    Some(FormContext {
        id: context.allocate_form_id(),
        node_id: element_node_id(element),
        action: element
            .attributes
            .get("action")
            .cloned()
            .filter(|value| !value.trim().is_empty()),
        method: element
            .attributes
            .get("method")
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "get".to_string()),
    })
}

fn build_form_control_spec(
    element: &StyledElement,
    current_form: Option<&FormContext>,
    context: &mut LayoutContext,
) -> Option<FormControlSpec> {
    let disabled = element.attributes.contains_key("disabled");
    let form_id = current_form.map(|form| form.id);
    let node_id = element_node_id(element);
    let form_node_id = current_form.and_then(|form| form.node_id);
    let form_action = current_form.and_then(|form| form.action.clone());
    let form_method = current_form
        .map(|form| form.method.clone())
        .unwrap_or_else(|| "get".to_string());

    match element.tag_name.as_str() {
        "input" => {
            let input_type = element
                .attributes
                .get("type")
                .map(|value| value.trim().to_ascii_lowercase())
                .unwrap_or_else(|| "text".to_string());

            match input_type.as_str() {
                "hidden" => Some(FormControlSpec {
                    id: context.allocate_control_id(),
                    node_id,
                    form_node_id,
                    kind: FormControlKind::Hidden,
                    style: element.style.clone(),
                    name: element.attributes.get("name").cloned(),
                    value: element.attributes.get("value").cloned().unwrap_or_default(),
                    placeholder: None,
                    label: String::new(),
                    form_id,
                    form_action,
                    form_method,
                    activates_submit: false,
                    disabled,
                    masked: false,
                    size_chars: None,
                }),
                "checkbox" | "radio" | "file" | "image" | "reset" => None,
                "submit" | "button" => Some(FormControlSpec {
                    id: context.allocate_control_id(),
                    node_id,
                    form_node_id,
                    kind: FormControlKind::Button,
                    style: element.style.clone(),
                    name: element.attributes.get("name").cloned(),
                    value: element.attributes.get("value").cloned().unwrap_or_default(),
                    placeholder: None,
                    label: element
                        .attributes
                        .get("value")
                        .cloned()
                        .filter(|value| !value.trim().is_empty())
                        .unwrap_or_else(|| {
                            if input_type == "submit" {
                                "Submit".to_string()
                            } else {
                                "Button".to_string()
                            }
                        }),
                    form_id,
                    form_action,
                    form_method,
                    activates_submit: input_type == "submit",
                    disabled,
                    masked: false,
                    size_chars: None,
                }),
                "password" => Some(FormControlSpec {
                    id: context.allocate_control_id(),
                    node_id,
                    form_node_id,
                    kind: FormControlKind::TextInput,
                    style: element.style.clone(),
                    name: element.attributes.get("name").cloned(),
                    value: element.attributes.get("value").cloned().unwrap_or_default(),
                    placeholder: element.attributes.get("placeholder").cloned(),
                    label: String::new(),
                    form_id,
                    form_action,
                    form_method,
                    activates_submit: false,
                    disabled,
                    masked: true,
                    size_chars: element
                        .attributes
                        .get("size")
                        .and_then(|value| value.parse::<u32>().ok()),
                }),
                _ => Some(FormControlSpec {
                    id: context.allocate_control_id(),
                    node_id,
                    form_node_id,
                    kind: FormControlKind::TextInput,
                    style: element.style.clone(),
                    name: element.attributes.get("name").cloned(),
                    value: element.attributes.get("value").cloned().unwrap_or_default(),
                    placeholder: element.attributes.get("placeholder").cloned(),
                    label: String::new(),
                    form_id,
                    form_action,
                    form_method,
                    activates_submit: false,
                    disabled,
                    masked: false,
                    size_chars: element
                        .attributes
                        .get("size")
                        .and_then(|value| value.parse::<u32>().ok()),
                }),
            }
        }
        "textarea" => Some(FormControlSpec {
            id: context.allocate_control_id(),
            node_id,
            form_node_id,
            kind: FormControlKind::TextInput,
            style: element.style.clone(),
            name: element.attributes.get("name").cloned(),
            value: collect_raw_text_content(&element.children),
            placeholder: element.attributes.get("placeholder").cloned(),
            label: String::new(),
            form_id,
            form_action,
            form_method,
            activates_submit: false,
            disabled,
            masked: false,
            size_chars: element
                .attributes
                .get("cols")
                .and_then(|value| value.parse::<u32>().ok()),
        }),
        "button" => {
            let button_type = element
                .attributes
                .get("type")
                .map(|value| value.trim().to_ascii_lowercase())
                .unwrap_or_else(|| "submit".to_string());
            let label = {
                let text = collect_text_content(&element.children);
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    "Button".to_string()
                } else {
                    trimmed.to_string()
                }
            };
            Some(FormControlSpec {
                id: context.allocate_control_id(),
                node_id,
                form_node_id,
                kind: FormControlKind::Button,
                style: element.style.clone(),
                name: element.attributes.get("name").cloned(),
                value: element.attributes.get("value").cloned().unwrap_or_default(),
                placeholder: None,
                label,
                form_id,
                form_action,
                form_method,
                activates_submit: button_type != "button" && button_type != "reset",
                disabled,
                masked: false,
                size_chars: None,
            })
        }
        _ => None,
    }
}

fn element_node_id(element: &StyledElement) -> Option<usize> {
    element
        .attributes
        .get("data-tobira-node-id")
        .and_then(|value| value.parse::<usize>().ok())
}

fn measure_form_control(control: &FormControlSpec, fonts: &mut FontContext) -> (u32, u32) {
    let line_height = text_line_height(&control.style, fonts);
    let height = line_height.saturating_add(10).max(28);
    match control.kind {
        FormControlKind::Hidden => (0, 0),
        FormControlKind::TextInput => {
            let size_chars = control.size_chars.unwrap_or(20).max(4);
            let char_width = char_width(&control.style, 'M', fonts).max(7);
            let text_width = char_width.saturating_mul(size_chars);
            (text_width.saturating_add(18).max(120), height)
        }
        FormControlKind::Button => {
            let label = control.label.trim();
            let label_width = if label.is_empty() {
                char_width(&control.style, 'M', fonts).saturating_mul(6)
            } else {
                text_width(&control.style, label, fonts)
            };
            (label_width.saturating_add(24).max(64), height)
        }
    }
}

fn collect_text_content(children: &[StyledNode]) -> String {
    let mut text = String::new();
    for child in children {
        match child {
            StyledNode::Text(node) => text.push_str(&node.text),
            StyledNode::Element(element) => text.push_str(&collect_text_content(&element.children)),
        }
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn collect_raw_text_content(children: &[StyledNode]) -> String {
    let mut text = String::new();
    for child in children {
        match child {
            StyledNode::Text(node) => text.push_str(&node.text),
            StyledNode::Element(element) => {
                text.push_str(&collect_raw_text_content(&element.children))
            }
        }
    }
    text
}

fn layout_node(
    node: &StyledNode,
    x: u32,
    width: u32,
    cursor_y: &mut u32,
    context: &mut LayoutContext,
    images: &ImageStore,
    fonts: &mut FontContext,
    current_form: Option<FormContext>,
) {
    match node {
        StyledNode::Text(text) => {
            let fragments = [InlineFragment::Text {
                text: text.text.clone(),
                style: text.style.clone(),
                link_href: None,
                link_node_id: None,
            }];
            layout_inline_fragments(&fragments, &text.style, x, width, cursor_y, context, fonts);
        }
        StyledNode::Element(element) => {
            if element.tag_name == "img" {
                layout_image_element(element, x, width, cursor_y, context, images, fonts);
                return;
            }

            match element.style.display {
                Display::None => {}
                Display::Inline => {
                    let fragments = flatten_inline_fragments(node, context, current_form.clone());
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
                    let current_form = form_context_for_element(element, context, current_form);
                    let link_href = if element.tag_name == "a" {
                        element.attributes.get("href").cloned()
                    } else {
                        None
                    };
                    let link_node_id = if element.tag_name == "a" {
                        element_node_id(element)
                    } else {
                        None
                    };
                    let y_before = *cursor_y;
                    layout_block_element(
                        element,
                        x,
                        width,
                        cursor_y,
                        context,
                        images,
                        fonts,
                        current_form,
                    );
                    if let Some(href) = link_href {
                        let link_height = cursor_y.saturating_sub(y_before);
                        if link_height > 0 {
                            context.links.push(LinkCommand {
                                node_id: link_node_id,
                                x,
                                y: y_before,
                                width,
                                height: link_height,
                                href,
                            });
                        }
                    }
                }
            }
        }
    }
}

fn layout_block_element(
    element: &StyledElement,
    x: u32,
    width: u32,
    cursor_y: &mut u32,
    context: &mut LayoutContext,
    images: &ImageStore,
    fonts: &mut FontContext,
    current_form: Option<FormContext>,
) {
    if element.tag_name == "br" {
        *cursor_y = cursor_y.saturating_add(text_line_height(&element.style, fonts));
        return;
    }

    if element.tag_name == "table" {
        layout_table_element(
            element,
            x,
            width,
            cursor_y,
            context,
            images,
            fonts,
            current_form,
        );
        return;
    }

    *cursor_y = cursor_y.saturating_add(element.style.margin.top);

    let outer_x = x.saturating_add(element.style.margin.left);
    let raw_outer_width =
        width.saturating_sub(element.style.margin.left + element.style.margin.right);
    // Apply min/max-width
    let outer_width = raw_outer_width
        .min(element.style.max_width.unwrap_or(u32::MAX))
        .max(element.style.min_width);
    let background_top = *cursor_y;
    let background_index = if let Some(background_color) = element.style.background_color {
        context.rects.push(RectCommand {
            x: outer_x,
            y: background_top,
            width: outer_width.max(1),
            height: 1,
            color: background_color,
        });
        Some(context.rects.len() - 1)
    } else {
        None
    };

    *cursor_y = cursor_y.saturating_add(element.style.padding.top);

    let bullet_indent = if element.style.display == Display::ListItem {
        16
    } else {
        0
    };

    let border_left = if !element.style.border_style_none {
        element.style.border.left
    } else {
        0
    };
    let border_right = if !element.style.border_style_none {
        element.style.border.right
    } else {
        0
    };

    let content_x = outer_x
        .saturating_add(border_left)
        .saturating_add(element.style.padding.left)
        .saturating_add(bullet_indent);
    let content_width = outer_width
        .saturating_sub(
            border_left
                + border_right
                + element.style.padding.left
                + element.style.padding.right
                + bullet_indent,
        )
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
            images,
            fonts,
            current_form,
        );
    }

    *cursor_y = cursor_y.saturating_add(element.style.padding.bottom);
    let background_height = cursor_y.saturating_sub(background_top).max(1);

    if let Some(background_index) = background_index {
        if let Some(rect) = context.rects.get_mut(background_index) {
            rect.height = background_height;
        }
    }

    // Draw borders if present
    if !element.style.border_style_none {
        let bc = element.style.border_color;
        let border_top_h = element.style.border.top;
        let border_bottom_h = element.style.border.bottom;
        let border_left_w = element.style.border.left;
        let border_right_w = element.style.border.right;

        if border_top_h > 0 {
            context.rects.push(RectCommand {
                x: outer_x,
                y: background_top,
                width: outer_width.max(1),
                height: border_top_h,
                color: bc,
            });
        }
        if border_bottom_h > 0 {
            context.rects.push(RectCommand {
                x: outer_x,
                y: cursor_y.saturating_sub(border_bottom_h),
                width: outer_width.max(1),
                height: border_bottom_h,
                color: bc,
            });
        }
        if border_left_w > 0 {
            context.rects.push(RectCommand {
                x: outer_x,
                y: background_top,
                width: border_left_w,
                height: background_height,
                color: bc,
            });
        }
        if border_right_w > 0 {
            context.rects.push(RectCommand {
                x: outer_x
                    .saturating_add(outer_width)
                    .saturating_sub(border_right_w),
                y: background_top,
                width: border_right_w,
                height: background_height,
                color: bc,
            });
        }
    }

    *cursor_y = cursor_y.saturating_add(element.style.margin.bottom);
}

fn layout_image_element(
    element: &StyledElement,
    x: u32,
    width: u32,
    cursor_y: &mut u32,
    context: &mut LayoutContext,
    images: &ImageStore,
    fonts: &mut FontContext,
) {
    *cursor_y = cursor_y.saturating_add(element.style.margin.top);

    let Some(src) = resolved_image_source(element) else {
        layout_image_fallback(element, x, width, cursor_y, context, fonts);
        return;
    };

    let Some(image) = images.get(src) else {
        layout_image_fallback(element, x, width, cursor_y, context, fonts);
        return;
    };

    let (draw_width, draw_height) =
        image_dimensions(element, image.width, image.height, width.max(1));
    let draw_x = match element.style.text_align {
        TextAlign::Center => x.saturating_add(width.saturating_sub(draw_width) / 2),
        TextAlign::Right => x.saturating_add(width.saturating_sub(draw_width)),
        TextAlign::Left => x,
    };

    context.images.push(ImageCommand {
        x: draw_x,
        y: *cursor_y,
        width: draw_width,
        height: draw_height,
        src: src.to_string(),
    });

    *cursor_y = cursor_y.saturating_add(draw_height);
    *cursor_y = cursor_y.saturating_add(element.style.margin.bottom);
}

fn layout_image_fallback(
    element: &StyledElement,
    x: u32,
    width: u32,
    cursor_y: &mut u32,
    context: &mut LayoutContext,
    fonts: &mut FontContext,
) {
    let alt = element
        .attributes
        .get("alt")
        .filter(|text| !text.trim().is_empty())
        .cloned()
        .unwrap_or_else(|| "[image]".to_string());
    let fragments = [InlineFragment::Text {
        text: alt,
        style: element.style.clone(),
        link_href: None,
        link_node_id: None,
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
}

fn resolved_image_source(element: &StyledElement) -> Option<&str> {
    element
        .attributes
        .get("data-scratch-src")
        .map(String::as_str)
        .or_else(|| element.attributes.get("src").map(String::as_str))
}

fn image_dimensions(
    element: &StyledElement,
    intrinsic_width: u32,
    intrinsic_height: u32,
    max_width: u32,
) -> (u32, u32) {
    let width_spec = specified_length(element, element.style.width, "width");
    let height_spec = specified_length(element, element.style.height, "height");
    let width_attr = width_spec.map(|length| resolve_length_value(length, max_width.max(1)));
    let height_attr =
        height_spec.map(|length| resolve_length_value(length, intrinsic_height.max(1)));

    let mut width = width_attr.unwrap_or(intrinsic_width.max(1));
    let mut height = height_attr.unwrap_or_else(|| {
        scaled_dimension(intrinsic_height.max(1), width, intrinsic_width.max(1))
    });

    if width > max_width && width > 0 {
        height = scaled_dimension(height.max(1), max_width.max(1), width);
        width = max_width.max(1);
    }

    if height_attr.is_some() && width_attr.is_none() {
        width = scaled_dimension(
            intrinsic_width.max(1),
            height.max(1),
            intrinsic_height.max(1),
        );
    }

    (width.max(1), height.max(1))
}

fn scaled_dimension(source: u32, target_basis: u32, source_basis: u32) -> u32 {
    if source_basis == 0 {
        return source.max(1);
    }
    ((source as u64 * target_basis as u64) / source_basis as u64)
        .max(1)
        .try_into()
        .unwrap_or(u32::MAX)
}

fn layout_table_element(
    element: &StyledElement,
    x: u32,
    width: u32,
    cursor_y: &mut u32,
    context: &mut LayoutContext,
    images: &ImageStore,
    fonts: &mut FontContext,
    current_form: Option<FormContext>,
) {
    *cursor_y = cursor_y.saturating_add(element.style.margin.top);

    let rows = collect_table_rows(element);
    if rows.is_empty() {
        *cursor_y = cursor_y.saturating_add(element.style.margin.bottom);
        return;
    }

    let placements = build_table_placements(&rows);
    let column_count = placements
        .iter()
        .map(|placement| placement.column_index + placement.colspan)
        .max()
        .unwrap_or(0);
    if column_count == 0 {
        *cursor_y = cursor_y.saturating_add(element.style.margin.bottom);
        return;
    }

    let spacing = parse_dimension_attribute(element.attributes.get("cellspacing")).unwrap_or(0);
    let padding = parse_dimension_attribute(element.attributes.get("cellpadding")).unwrap_or(0);
    let available_width = width.max(1);
    let track_total_spacing = spacing.saturating_mul(column_count.saturating_sub(1) as u32);
    let content_limit = available_width.saturating_sub(track_total_spacing).max(1);
    let mut sizing =
        compute_column_widths(element, &placements, content_limit, padding, images, fonts);
    let preferred_content_width = sizing.widths.iter().sum::<u32>();
    let preferred_table_width = preferred_content_width
        .saturating_add(track_total_spacing)
        .max(1);
    let table_width = resolve_table_width(element, available_width, preferred_table_width);
    let target_content_width = table_width.saturating_sub(track_total_spacing).max(1);
    expand_column_widths(
        &mut sizing,
        target_content_width.saturating_sub(preferred_content_width),
    );
    let column_widths = sizing.widths;
    let table_width = column_widths
        .iter()
        .sum::<u32>()
        .saturating_add(track_total_spacing);
    let table_x = match element
        .attributes
        .get("align")
        .map(|value| value.to_ascii_lowercase())
    {
        Some(value) if value == "center" => x.saturating_add(width.saturating_sub(table_width) / 2),
        Some(value) if value == "right" => x.saturating_add(width.saturating_sub(table_width)),
        _ => x,
    };

    let mut cell_layouts = Vec::with_capacity(placements.len());
    let mut next_control_id = context.next_control_id;
    let mut next_form_id = context.next_form_id;
    for placement in &placements {
        let span_width = span_width(&column_widths, placement.column_index, placement.colspan)
            .saturating_add(spacing.saturating_mul(placement.colspan.saturating_sub(1) as u32));
        let inner_width = span_width.saturating_sub(padding.saturating_mul(2)).max(1);
        let layout = layout_table_cell(
            placement.cell,
            inner_width,
            images,
            fonts,
            current_form.clone(),
            next_control_id,
            next_form_id,
        );
        next_control_id = layout.next_control_id;
        next_form_id = layout.next_form_id;
        cell_layouts.push(layout);
    }

    let row_count = rows.len();
    let mut row_heights = vec![0_u32; row_count];
    for (placement, layout) in placements.iter().zip(cell_layouts.iter()) {
        if placement.rowspan == 1 {
            row_heights[placement.row_index] =
                row_heights[placement.row_index].max(layout.content_height);
        }
    }
    for (placement, layout) in placements.iter().zip(cell_layouts.iter()) {
        if placement.rowspan > 1 {
            let start = placement.row_index;
            let end = (placement.row_index + placement.rowspan).min(row_heights.len());
            let current = row_heights[start..end].iter().sum::<u32>();
            if current < layout.content_height && end > start {
                row_heights[end - 1] =
                    row_heights[end - 1].saturating_add(layout.content_height - current);
            }
        }
    }
    for height in &mut row_heights {
        *height = (*height).max(1);
    }

    let mut row_offsets = vec![0_u32; row_count];
    for index in 1..row_count {
        row_offsets[index] = row_offsets[index - 1]
            .saturating_add(row_heights[index - 1])
            .saturating_add(spacing);
    }

    for (placement, layout) in placements.iter().zip(cell_layouts.iter()) {
        let cell_x = table_x
            .saturating_add(span_width(&column_widths, 0, placement.column_index))
            .saturating_add(spacing.saturating_mul(placement.column_index as u32));
        let cell_y = cursor_y.saturating_add(row_offsets[placement.row_index]);
        let cell_width = span_width(&column_widths, placement.column_index, placement.colspan)
            .saturating_add(spacing.saturating_mul(placement.colspan.saturating_sub(1) as u32));
        let cell_height = cell_span_height(&row_heights, placement.row_index, placement.rowspan)
            .saturating_add(spacing.saturating_mul(placement.rowspan.saturating_sub(1) as u32));

        if let Some(background_color) = placement.cell.style.background_color {
            context.rects.push(RectCommand {
                x: cell_x,
                y: cell_y,
                width: cell_width.max(1),
                height: cell_height.max(1),
                color: background_color,
            });
        }

        let content_area_height = cell_height.saturating_sub(padding.saturating_mul(2));
        let vertical_offset = match placement.cell.style.vertical_align {
            VerticalAlign::Top => 0,
            VerticalAlign::Middle => content_area_height.saturating_sub(layout.content_height) / 2,
            VerticalAlign::Bottom => content_area_height.saturating_sub(layout.content_height),
        };

        merge_fragment(
            context,
            layout,
            cell_x.saturating_add(padding),
            cell_y
                .saturating_add(padding)
                .saturating_add(vertical_offset),
        );
    }

    let table_height = row_heights.iter().sum::<u32>()
        + spacing.saturating_mul(row_count.saturating_sub(1) as u32);
    context.next_control_id = next_control_id;
    context.next_form_id = next_form_id;
    *cursor_y = cursor_y.saturating_add(table_height);
    *cursor_y = cursor_y.saturating_add(element.style.margin.bottom);
}

#[derive(Debug, Clone)]
struct TablePlacement<'a> {
    row_index: usize,
    column_index: usize,
    colspan: usize,
    rowspan: usize,
    cell: &'a StyledElement,
}

#[derive(Debug, Clone, Default)]
struct FragmentLayout {
    content_height: u32,
    rects: Vec<RectCommand>,
    texts: Vec<TextCommand>,
    images: Vec<ImageCommand>,
    links: Vec<LinkCommand>,
    controls: Vec<FormControlCommand>,
    next_control_id: usize,
    next_form_id: usize,
}

#[derive(Debug, Clone)]
struct TableColumnSizing {
    widths: Vec<u32>,
    locked: Vec<bool>,
}

fn collect_table_rows(element: &StyledElement) -> Vec<&StyledElement> {
    let mut rows = Vec::new();
    collect_table_rows_into(element, &mut rows);
    rows
}

fn collect_table_rows_into<'a>(element: &'a StyledElement, output: &mut Vec<&'a StyledElement>) {
    for child in &element.children {
        if let StyledNode::Element(child_element) = child {
            match child_element.tag_name.as_str() {
                "tr" => output.push(child_element),
                "tbody" | "thead" | "tfoot" => collect_table_rows_into(child_element, output),
                _ => {}
            }
        }
    }
}

fn build_table_placements<'a>(rows: &[&'a StyledElement]) -> Vec<TablePlacement<'a>> {
    let mut placements = Vec::new();
    let mut row_spans = Vec::<usize>::new();

    for (row_index, row) in rows.iter().enumerate() {
        for span in &mut row_spans {
            *span = span.saturating_sub(1);
        }

        let cells = row
            .children
            .iter()
            .filter_map(|child| match child {
                StyledNode::Element(element)
                    if matches!(element.tag_name.as_str(), "td" | "th") =>
                {
                    Some(element)
                }
                _ => None,
            })
            .collect::<Vec<_>>();

        let mut column_index = 0;
        for cell in cells {
            while row_spans.get(column_index).copied().unwrap_or(0) > 0 {
                column_index += 1;
            }

            let colspan = parse_span_attribute(cell.attributes.get("colspan"));
            let rowspan = parse_span_attribute(cell.attributes.get("rowspan"));

            if row_spans.len() < column_index + colspan {
                row_spans.resize(column_index + colspan, 0);
            }

            for span in row_spans.iter_mut().skip(column_index).take(colspan) {
                *span = rowspan;
            }

            placements.push(TablePlacement {
                row_index,
                column_index,
                colspan,
                rowspan,
                cell,
            });
            column_index += colspan;
        }
    }

    placements
}

fn compute_column_widths(
    _table: &StyledElement,
    placements: &[TablePlacement<'_>],
    available_width: u32,
    padding: u32,
    images: &ImageStore,
    fonts: &mut FontContext,
) -> TableColumnSizing {
    let column_count = placements
        .iter()
        .map(|placement| placement.column_index + placement.colspan)
        .max()
        .unwrap_or(0);
    let mut widths = vec![0_u32; column_count];
    let mut locked = vec![false; column_count];

    for placement in placements {
        if placement.colspan != 1 {
            continue;
        }

        let column = placement.column_index;
        if let Some(length) = specified_length(placement.cell, placement.cell.style.width, "width")
        {
            let resolved = resolve_length_value(length, available_width);
            widths[column] = widths[column].max(resolved);
            locked[column] = true;
            continue;
        }

        let measured = measure_cell_preferred_width(placement.cell, padding, images, fonts);
        widths[column] = widths[column].max(measured);
    }

    TableColumnSizing {
        widths: widths.into_iter().map(|value| value.max(1)).collect(),
        locked,
    }
}

fn expand_column_widths(sizing: &mut TableColumnSizing, extra: u32) {
    if extra == 0 || sizing.widths.is_empty() {
        return;
    }

    let flex_columns = sizing.locked.iter().filter(|&&value| !value).count().max(1) as u32;
    let flex_share = (extra / flex_columns).max(1);
    let mut remaining = extra;

    for (index, width) in sizing.widths.iter_mut().enumerate() {
        if sizing.locked[index] {
            continue;
        }
        let add = flex_share.min(remaining);
        *width = width.saturating_add(add);
        remaining = remaining.saturating_sub(add);
    }

    if remaining > 0 {
        let target_index = sizing
            .locked
            .iter()
            .position(|locked| !locked)
            .unwrap_or(sizing.widths.len() - 1);
        sizing.widths[target_index] = sizing.widths[target_index].saturating_add(remaining);
    }
}

fn layout_table_cell(
    cell: &StyledElement,
    width: u32,
    images: &ImageStore,
    fonts: &mut FontContext,
    current_form: Option<FormContext>,
    control_id_seed: usize,
    form_id_seed: usize,
) -> FragmentLayout {
    let mut context = LayoutContext {
        rects: Vec::new(),
        texts: Vec::new(),
        images: Vec::new(),
        links: Vec::new(),
        controls: Vec::new(),
        next_control_id: control_id_seed,
        next_form_id: form_id_seed,
    };
    let mut cursor_y = 0_u32;

    for child in &cell.children {
        if is_hidden(child) {
            continue;
        }

        layout_node(
            child,
            0,
            width,
            &mut cursor_y,
            &mut context,
            images,
            fonts,
            current_form.clone(),
        );
    }

    FragmentLayout {
        content_height: cursor_y.max(1),
        rects: context.rects,
        texts: context.texts,
        images: context.images,
        links: context.links,
        controls: context.controls,
        next_control_id: context.next_control_id,
        next_form_id: context.next_form_id,
    }
}

fn merge_fragment(
    context: &mut LayoutContext,
    fragment: &FragmentLayout,
    offset_x: u32,
    offset_y: u32,
) {
    context
        .rects
        .extend(fragment.rects.iter().map(|rect| RectCommand {
            x: rect.x.saturating_add(offset_x),
            y: rect.y.saturating_add(offset_y),
            width: rect.width,
            height: rect.height,
            color: rect.color,
        }));
    context
        .texts
        .extend(fragment.texts.iter().map(|text| TextCommand {
            x: text.x.saturating_add(offset_x),
            y: text.y.saturating_add(offset_y),
            width: text.width,
            text: text.text.clone(),
            font_size_px: text.font_size_px,
            font_family: text.font_family,
            color: text.color,
            underline: text.underline,
            bold: text.bold,
            italic: text.italic,
        }));
    context
        .images
        .extend(fragment.images.iter().map(|image| ImageCommand {
            x: image.x.saturating_add(offset_x),
            y: image.y.saturating_add(offset_y),
            width: image.width,
            height: image.height,
            src: image.src.clone(),
        }));
    context
        .links
        .extend(fragment.links.iter().map(|link| LinkCommand {
            node_id: link.node_id,
            x: link.x.saturating_add(offset_x),
            y: link.y.saturating_add(offset_y),
            width: link.width,
            height: link.height,
            href: link.href.clone(),
        }));
    context
        .controls
        .extend(fragment.controls.iter().map(|control| FormControlCommand {
            id: control.id,
            node_id: control.node_id,
            form_node_id: control.form_node_id,
            kind: control.kind,
            x: control.x.saturating_add(offset_x),
            y: control.y.saturating_add(offset_y),
            width: control.width,
            height: control.height,
            name: control.name.clone(),
            value: control.value.clone(),
            label: control.label.clone(),
            placeholder: control.placeholder.clone(),
            form_id: control.form_id,
            form_action: control.form_action.clone(),
            form_method: control.form_method.clone(),
            activates_submit: control.activates_submit,
            disabled: control.disabled,
            masked: control.masked,
            font_size_px: control.font_size_px,
            font_family: control.font_family,
            text_color: control.text_color,
            background_color: control.background_color,
            border_color: control.border_color,
        }));
}

fn span_width(widths: &[u32], start: usize, span: usize) -> u32 {
    widths.iter().skip(start).take(span).sum()
}

fn cell_span_height(heights: &[u32], start: usize, span: usize) -> u32 {
    heights.iter().skip(start).take(span).sum()
}

fn layout_mixed_children(
    element: &StyledElement,
    x: u32,
    width: u32,
    cursor_y: &mut u32,
    context: &mut LayoutContext,
    needs_bullet: bool,
    images: &ImageStore,
    fonts: &mut FontContext,
    current_form: Option<FormContext>,
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
                            link_href: None,
                            link_node_id: None,
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

            layout_node(
                child,
                x,
                width,
                cursor_y,
                context,
                images,
                fonts,
                current_form.clone(),
            );
        } else {
            if bullet_pending {
                inline_fragments.push(InlineFragment::Text {
                    text: "- ".to_string(),
                    style: element.style.clone(),
                    link_href: None,
                    link_node_id: None,
                });
                bullet_pending = false;
            }
            collect_inline_fragments(
                child,
                &mut inline_fragments,
                None,
                None,
                current_form.clone(),
                context,
            );
        }
    }

    if !inline_fragments.is_empty() || bullet_pending {
        if bullet_pending {
            inline_fragments.push(InlineFragment::Text {
                text: "- ".to_string(),
                style: element.style.clone(),
                link_href: None,
                link_node_id: None,
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

fn collect_inline_fragments(
    node: &StyledNode,
    output: &mut Vec<InlineFragment>,
    link_href: Option<&str>,
    link_node_id: Option<usize>,
    current_form: Option<FormContext>,
    context: &mut LayoutContext,
) {
    match node {
        StyledNode::Text(text) => {
            output.push(InlineFragment::Text {
                text: text.text.clone(),
                style: text.style.clone(),
                link_href: link_href.map(str::to_string),
                link_node_id,
            });
        }
        StyledNode::Element(element) => {
            let current_form = form_context_for_element(element, context, current_form);
            let current_link = if element.tag_name == "a" {
                element
                    .attributes
                    .get("href")
                    .map(String::as_str)
                    .or(link_href)
            } else {
                link_href
            };
            let current_link_node_id = if element.tag_name == "a" {
                element_node_id(element).or(link_node_id)
            } else {
                link_node_id
            };

            match element.style.display {
                Display::None => {}
                Display::Inline => {
                    if element.tag_name == "br" {
                        output.push(InlineFragment::LineBreak);
                        return;
                    }

                    if let Some(control) =
                        build_form_control_spec(element, current_form.as_ref(), context)
                    {
                        output.push(InlineFragment::Control(control));
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
                            link_href: current_link.map(str::to_string),
                            link_node_id: current_link_node_id,
                        });
                        return;
                    }

                    for child in &element.children {
                        collect_inline_fragments(
                            child,
                            output,
                            current_link,
                            current_link_node_id,
                            current_form.clone(),
                            context,
                        );
                    }
                }
                Display::Block | Display::ListItem => {}
            }
        }
    }
}

fn flatten_inline_fragments(
    node: &StyledNode,
    context: &mut LayoutContext,
    current_form: Option<FormContext>,
) -> Vec<InlineFragment> {
    let mut fragments = Vec::new();
    collect_inline_fragments(node, &mut fragments, None, None, current_form, context);
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
    let text_indent = container_style.text_indent;
    let mut first_line = true;

    for fragment in fragments {
        match fragment {
            InlineFragment::LineBreak => {
                emit_line_with_indent(
                    &mut line,
                    container_style,
                    x,
                    width,
                    cursor_y,
                    context,
                    fonts,
                    if first_line { text_indent } else { 0 },
                );
                first_line = false;
                pending_space = false;
            }
            InlineFragment::Control(control) => {
                let (control_width, _) = measure_form_control(control, fonts);
                let effective_width = if first_line && line.is_empty() {
                    width.saturating_sub(text_indent)
                } else {
                    width
                };

                let pending_space_before_control = pending_space && !line.is_empty();
                if pending_space_before_control {
                    let space_width = char_width(&control.style, ' ', fonts);
                    if line.width.saturating_add(space_width) > effective_width {
                        emit_line_with_indent(
                            &mut line,
                            container_style,
                            x,
                            width,
                            cursor_y,
                            context,
                            fonts,
                            if first_line { text_indent } else { 0 },
                        );
                        first_line = false;
                    } else {
                        line.push_span(" ", &control.style, fonts, None, None);
                    }
                }

                let effective_width = if first_line && line.is_empty() {
                    width.saturating_sub(text_indent)
                } else {
                    width
                };
                if !line.is_empty() && line.width.saturating_add(control_width) > effective_width {
                    emit_line_with_indent(
                        &mut line,
                        container_style,
                        x,
                        width,
                        cursor_y,
                        context,
                        fonts,
                        if first_line { text_indent } else { 0 },
                    );
                    first_line = false;
                }
                line.push_control(control, fonts);
                pending_space = true;
            }
            InlineFragment::Text {
                text,
                style,
                link_href,
                link_node_id,
            } => {
                let had_whitespace = text.chars().any(char::is_whitespace);

                for word in text.split_whitespace() {
                    let effective_width = if first_line && line.is_empty() {
                        width.saturating_sub(text_indent)
                    } else {
                        width
                    };

                    if pending_space && !line.is_empty() {
                        let space_width = char_width(style, ' ', fonts);
                        if line.width.saturating_add(space_width) > effective_width {
                            emit_line_with_indent(
                                &mut line,
                                container_style,
                                x,
                                width,
                                cursor_y,
                                context,
                                fonts,
                                if first_line { text_indent } else { 0 },
                            );
                            first_line = false;
                        } else {
                            line.push_span(" ", style, fonts, link_href.as_deref(), *link_node_id);
                        }
                    }

                    let effective_width2 = if first_line && line.is_empty() {
                        width.saturating_sub(text_indent)
                    } else {
                        width
                    };

                    push_wrapped_word(
                        word,
                        style,
                        link_href.as_deref(),
                        *link_node_id,
                        container_style,
                        x,
                        effective_width2,
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

    emit_line_with_indent(
        &mut line,
        container_style,
        x,
        width,
        cursor_y,
        context,
        fonts,
        if first_line { text_indent } else { 0 },
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
            InlineFragment::Control(control) => {
                let (control_width, _) = measure_form_control(control, fonts);
                if !line.is_empty() && line.width.saturating_add(control_width) > width {
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
                line.push_control(control, fonts);
            }
            InlineFragment::Text {
                text,
                style,
                link_href,
                link_node_id,
            } => {
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
                    line.push_span(
                        character.encode_utf8(&mut buffer),
                        style,
                        fonts,
                        link_href.as_deref(),
                        *link_node_id,
                    );
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
    link_href: Option<&str>,
    link_node_id: Option<usize>,
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
        line.push_span(word, style, fonts, link_href, link_node_id);
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
            line.push_span(&chunk, style, fonts, link_href, link_node_id);
            emit_line(line, container_style, x, width, cursor_y, context, fonts);
            chunk.clear();
        }
    }

    if !chunk.is_empty() {
        if !line.is_empty() && line.width.saturating_add(text_width(style, &chunk, fonts)) > width {
            emit_line(line, container_style, x, width, cursor_y, context, fonts);
        }
        line.push_span(&chunk, style, fonts, link_href, link_node_id);
    }
}

fn emit_line_with_indent(
    line: &mut LineBuilder,
    container_style: &ComputedStyle,
    x: u32,
    width: u32,
    cursor_y: &mut u32,
    context: &mut LayoutContext,
    fonts: &mut FontContext,
    indent: u32,
) {
    emit_line_impl(
        line,
        container_style,
        x,
        width,
        cursor_y,
        context,
        fonts,
        indent,
    );
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
    emit_line_impl(line, container_style, x, width, cursor_y, context, fonts, 0);
}

fn emit_line_impl(
    line: &mut LineBuilder,
    container_style: &ComputedStyle,
    x: u32,
    width: u32,
    cursor_y: &mut u32,
    context: &mut LayoutContext,
    fonts: &mut FontContext,
    indent: u32,
) {
    if line.is_empty() {
        *cursor_y = cursor_y.saturating_add(text_line_height(container_style, fonts));
        return;
    }

    let effective_x = x.saturating_add(indent);
    let effective_width = width.saturating_sub(indent);

    let line_width = line.width.min(effective_width);
    let offset_x = match container_style.text_align {
        TextAlign::Left => 0,
        TextAlign::Center => effective_width.saturating_sub(line_width) / 2,
        TextAlign::Right => effective_width.saturating_sub(line_width),
    };

    let mut cursor_x = effective_x.saturating_add(offset_x);
    let line_height = line
        .line_height
        .max(text_line_height(container_style, fonts));

    for span in &line.spans {
        if let Some(control) = &span.control {
            let control_y = cursor_y.saturating_add(line_height.saturating_sub(span.height) / 2);
            let background_color = if control.disabled {
                0xE4E6EA
            } else if matches!(control.kind, FormControlKind::Button) {
                0xE7EBF2
            } else {
                0xFFFFFF
            };
            let border_color = if control.disabled { 0xA9AFB8 } else { 0x7F8B9C };
            context.controls.push(FormControlCommand {
                id: control.id,
                node_id: control.node_id,
                form_node_id: control.form_node_id,
                kind: control.kind,
                x: cursor_x,
                y: control_y,
                width: span.width.max(1),
                height: span.height.max(1),
                name: control.name.clone(),
                value: control.value.clone(),
                label: control.label.clone(),
                placeholder: control.placeholder.clone(),
                form_id: control.form_id,
                form_action: control.form_action.clone(),
                form_method: control.form_method.clone(),
                activates_submit: control.activates_submit,
                disabled: control.disabled,
                masked: control.masked,
                font_size_px: span.style.font_size_px,
                font_family: span.style.font_family,
                text_color: span.style.color,
                background_color,
                border_color,
            });

            cursor_x = cursor_x.saturating_add(span.width);
            continue;
        }

        if let Some(background_color) = span.style.background_color {
            context.rects.push(RectCommand {
                x: cursor_x,
                y: *cursor_y,
                width: span.width,
                height: line_height,
                color: background_color,
            });
        }

        let display_text = if span.style.text_transform != TextTransform::None {
            apply_text_transform(&span.text, span.style.text_transform)
        } else {
            span.text.clone()
        };
        context.texts.push(TextCommand {
            x: cursor_x,
            y: *cursor_y,
            width: span.width,
            text: display_text,
            font_size_px: span.style.font_size_px,
            font_family: span.style.font_family,
            color: span.style.color,
            underline: span.style.underline,
            bold: span.style.font_weight,
            italic: span.style.font_style_italic,
        });

        if let Some(href) = &span.link_href {
            context.links.push(LinkCommand {
                node_id: span.link_node_id,
                x: cursor_x,
                y: *cursor_y,
                width: span.width,
                height: line_height,
                href: href.clone(),
            });
        }

        cursor_x = cursor_x.saturating_add(span.width);
    }

    *cursor_y = cursor_y.saturating_add(line_height);
    line.spans.clear();
    line.width = 0;
    line.line_height = 0;
}

fn is_block_level(node: &StyledNode) -> bool {
    if matches!(
        node,
        StyledNode::Element(StyledElement { tag_name, .. }) if tag_name == "img"
    ) {
        return true;
    }

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
    if style.line_height > 0 {
        (style.font_size_px as u64 * style.line_height as u64 / 1000) as u32
    } else {
        fonts.line_height_px(style.font_size_px, style.font_family)
    }
}

fn text_width(style: &ComputedStyle, text: &str, fonts: &mut FontContext) -> u32 {
    let base = fonts.text_width_px(text, style.font_size_px, style.font_family);
    let char_count = text.chars().count() as i32;
    let spacing = style.letter_spacing as i32 * char_count;
    if spacing >= 0 {
        base.saturating_add(spacing as u32)
    } else {
        base.saturating_sub((-spacing) as u32)
    }
}

fn parse_dimension_attribute(value: Option<&String>) -> Option<u32> {
    value
        .map(String::as_str)
        .and_then(|raw| raw.trim_end_matches('%').parse::<u32>().ok())
}

fn parse_span_attribute(value: Option<&String>) -> usize {
    value
        .and_then(|raw| raw.parse::<usize>().ok())
        .unwrap_or(1)
        .max(1)
}

fn resolve_table_width(element: &StyledElement, available_width: u32, preferred_width: u32) -> u32 {
    specified_length(element, element.style.width, "width")
        .map(|length| resolve_length_value(length, available_width))
        .map(|resolved| resolved.max(preferred_width))
        .unwrap_or(preferred_width)
        .min(available_width.max(1))
}

fn measure_cell_preferred_width(
    cell: &StyledElement,
    padding: u32,
    images: &ImageStore,
    fonts: &mut FontContext,
) -> u32 {
    if let Some(LengthValue::Pixels(width)) = specified_length(cell, cell.style.width, "width") {
        return width.max(1);
    }

    let mut max_width = 1_u32;
    for child in &cell.children {
        max_width = max_width.max(measure_node_preferred_width(child, images, fonts));
    }

    max_width.saturating_add(padding.saturating_mul(2))
}

fn measure_node_preferred_width(
    node: &StyledNode,
    images: &ImageStore,
    fonts: &mut FontContext,
) -> u32 {
    match node {
        StyledNode::Text(text) => text_width(&text.style, text.text.trim(), fonts).max(1),
        StyledNode::Element(element) => {
            if element.tag_name == "img"
                && let Some(src) = resolved_image_source(element)
                && let Some(image) = images.get(src)
            {
                return image_dimensions(element, image.width, image.height, u32::MAX / 2).0;
            }

            if element.tag_name == "table" {
                return specified_length(element, element.style.width, "width")
                    .map(|length| match length {
                        LengthValue::Pixels(value) => value,
                        LengthValue::Percent(value) => value.saturating_mul(8),
                    })
                    .unwrap_or_else(|| {
                        collect_table_rows(element)
                            .into_iter()
                            .flat_map(|row| row.children.iter())
                            .map(|child| measure_node_preferred_width(child, images, fonts))
                            .sum::<u32>()
                            .max(1)
                    });
            }

            let child_width = element
                .children
                .iter()
                .map(|child| measure_node_preferred_width(child, images, fonts))
                .max()
                .unwrap_or(1);

            child_width
                .saturating_add(element.style.padding.left + element.style.padding.right)
                .max(1)
        }
    }
}

fn specified_length(
    element: &StyledElement,
    from_style: Option<LengthValue>,
    attribute_name: &str,
) -> Option<LengthValue> {
    from_style.or_else(|| parse_attribute_length_value(element.attributes.get(attribute_name)))
}

fn parse_attribute_length_value(value: Option<&String>) -> Option<LengthValue> {
    let raw = value?.trim();
    if let Some(percent) = raw.strip_suffix('%') {
        return percent.parse::<u32>().ok().map(LengthValue::Percent);
    }

    raw.parse::<u32>().ok().map(LengthValue::Pixels)
}

fn resolve_length_value(length: LengthValue, available_width: u32) -> u32 {
    match length {
        LengthValue::Pixels(value) => value,
        LengthValue::Percent(value) => available_width.saturating_mul(value) / 100,
    }
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
    use crate::image::{DecodedImage, ImageStore};

    #[test]
    fn hides_display_none_content() {
        let document = parse_document("<div><p>Hello</p><span class=\"hide\">Nope</span></div>");
        let stylesheet = parse_stylesheet(".hide { display: none; } p { color: #ff0000; }");
        let styled = build_styled_tree(&document, &stylesheet, 1280);
        let mut fonts = FontContext::load();
        let layout = layout_styled_document(&styled, &ImageStore::default(), 320, &mut fonts);

        assert!(layout.texts.iter().any(|text| text.text.contains("Hello")));
        assert!(layout.texts.iter().all(|text| !text.text.contains("Nope")));
        assert!(layout.texts.iter().any(|text| text.color == 0xFF0000));
    }

    #[test]
    fn centers_text_when_requested() {
        let document = parse_document("<p>Hello</p>");
        let stylesheet = parse_stylesheet("p { text-align: center; font-size: 16px; }");
        let styled = build_styled_tree(&document, &stylesheet, 1280);
        let mut fonts = FontContext::load();
        let layout = layout_styled_document(&styled, &ImageStore::default(), 200, &mut fonts);

        let text = layout.texts.first().expect("text command should exist");
        let expected_left_offset = (200 - text.width) / 2;

        assert_eq!(text.x, expected_left_offset);
    }

    #[test]
    fn wraps_text_across_multiple_lines() {
        let document = parse_document("<p>alpha beta gamma delta epsilon</p>");
        let stylesheet = parse_stylesheet("p { font-size: 16px; }");
        let styled = build_styled_tree(&document, &stylesheet, 1280);
        let mut fonts = FontContext::load();
        let layout = layout_styled_document(&styled, &ImageStore::default(), 90, &mut fonts);

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
        let styled = build_styled_tree(&document, &stylesheet, 1280);

        let paragraph = match styled {
            crate::css::StyledNode::Element(ref root) => {
                find_paragraph(root).expect("paragraph should be present")
            }
            crate::css::StyledNode::Text(_) => panic!("root should be an element"),
        };

        assert_eq!(paragraph.style.text_align, TextAlign::Right);
    }

    #[test]
    fn places_table_cells_side_by_side() {
        let document = parse_document("<table><tr><td>Left</td><td>Right</td></tr></table>");
        let styled = build_styled_tree(&document, &parse_stylesheet(""), 1280);
        let mut fonts = FontContext::load();
        let layout = layout_styled_document(&styled, &ImageStore::default(), 320, &mut fonts);
        let left = layout
            .texts
            .iter()
            .find(|text| text.text.contains("Left"))
            .expect("left cell text should exist");
        let right = layout
            .texts
            .iter()
            .find(|text| text.text.contains("Right"))
            .expect("right cell text should exist");

        assert_eq!(left.y, right.y);
        assert!(right.x > left.x);
    }

    #[test]
    fn emits_image_commands_for_loaded_images() {
        let document = parse_document(
            "<div><img src=\"https://example.com/pic.jpg\" data-scratch-src=\"https://example.com/pic.jpg\" width=\"40\" height=\"20\"></div>",
        );
        let styled = build_styled_tree(&document, &parse_stylesheet(""), 1280);
        let mut fonts = FontContext::load();
        let mut images = ImageStore::default();
        images.insert(
            "https://example.com/pic.jpg".to_string(),
            DecodedImage {
                width: 80,
                height: 40,
                rgba: vec![255; 80 * 40 * 4],
            },
        );

        let layout = layout_styled_document(&styled, &images, 320, &mut fonts);

        assert_eq!(layout.images.len(), 1);
        assert_eq!(layout.images[0].width, 40);
        assert_eq!(layout.images[0].height, 20);
    }

    #[test]
    fn auto_width_tables_do_not_expand_to_full_container() {
        let document =
            parse_document("<table align=\"center\"><tr><td>Hello</td><td>World</td></tr></table>");
        let styled = build_styled_tree(&document, &parse_stylesheet(""), 1280);
        let mut fonts = FontContext::load();
        let layout = layout_styled_document(&styled, &ImageStore::default(), 500, &mut fonts);
        let hello = layout
            .texts
            .iter()
            .find(|text| text.text.contains("Hello"))
            .expect("hello text should exist");
        let world = layout
            .texts
            .iter()
            .find(|text| text.text.contains("World"))
            .expect("world text should exist");

        assert!(hello.x > 40);
        assert!(world.x.saturating_sub(hello.x) < 220);
    }

    #[test]
    fn vertical_align_middle_offsets_cell_content() {
        let document = parse_document(
            "<table><tr><td valign=\"middle\">short</td><td><br><br><br><br><br>tall</td></tr></table>",
        );
        let styled = build_styled_tree(&document, &parse_stylesheet(""), 1280);
        let mut fonts = FontContext::load();
        let layout = layout_styled_document(&styled, &ImageStore::default(), 320, &mut fonts);
        let short = layout
            .texts
            .iter()
            .find(|text| text.text.contains("short"))
            .expect("short text should exist");

        assert!(short.y > 0);
    }

    #[test]
    fn keeps_rowspan_cells_from_colliding_with_next_row() {
        let document = parse_document(
            "<table><tr><td rowspan=\"2\">Left</td><td>Top</td></tr><tr><td>Bottom</td></tr></table>",
        );
        let styled = build_styled_tree(&document, &parse_stylesheet(""), 1280);
        let mut fonts = FontContext::load();
        let layout = layout_styled_document(&styled, &ImageStore::default(), 320, &mut fonts);
        let top = layout
            .texts
            .iter()
            .find(|text| text.text.contains("Top"))
            .expect("top cell text should exist");
        let bottom = layout
            .texts
            .iter()
            .find(|text| text.text.contains("Bottom"))
            .expect("bottom cell text should exist");

        assert!(bottom.y > top.y);
        assert_eq!(top.x, bottom.x);
    }

    #[test]
    fn emits_form_controls_for_inputs_and_buttons() {
        let document = parse_document(
            "<form action=\"/search\"><input name=\"q\" value=\"rust\"><button type=\"submit\">Go</button></form>",
        );
        let styled = build_styled_tree(&document, &parse_stylesheet(""), 1280);
        let mut fonts = FontContext::load();
        let layout = layout_styled_document(&styled, &ImageStore::default(), 320, &mut fonts);

        assert_eq!(layout.controls.len(), 2);
        assert!(
            layout
                .controls
                .iter()
                .any(|control| control.kind == super::FormControlKind::TextInput
                    && control.name.as_deref() == Some("q"))
        );
        assert!(
            layout
                .controls
                .iter()
                .any(|control| control.kind == super::FormControlKind::Button
                    && control.label == "Go")
        );
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
