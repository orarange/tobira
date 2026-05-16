use crate::css::{
    Color, ComputedStyle, CursorKind, DEFAULT_BACKGROUND_COLOR, Display, FontFamilyKind, GridTrackSize,
    LengthValue, ObjectFit, Overflow, Position, FlexDirection, AlignItems, AlignSelf,
    JustifyContent, StyledElement, StyledNode, TextAlign, TextTransform, VerticalAlign,
    WhiteSpaceMode, apply_text_transform,
};
use crate::font::FontContext;
use crate::image::ImageStore;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrawCommand {
    Rect(RectCommand),
    Text(TextCommand),
    Image(ImageCommand),
    Layer(LayerCommand),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerCommand {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub opacity: u8,
    pub blur_px: u32,       // CSS filter: blur() radius; 0 = no blur
    pub brightness: u32,    // CSS filter: brightness() in 1/10000; 10000 = no change
    pub commands: Vec<DrawCommand>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElementHitbox {
    pub node_id: usize,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub cursor_kind: CursorKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutDocument {
    pub background_color: Color,
    pub content_height: u32,
    pub commands: Vec<DrawCommand>,
    pub links: Vec<LinkCommand>,
    pub controls: Vec<FormControlCommand>,
    pub element_hitboxes: Vec<ElementHitbox>,
}

// Convenience accessors for consumers that need flat lists
impl LayoutDocument {
    /// Flatten all text commands across the command tree, including those inside layers.
    ///
    /// **Note:** these methods recurse into `LayerCommand` children but ignore the layer's
    /// `opacity` value.  Colors/positions returned reflect the *raw* (pre-compositor) values
    /// stored in the draw commands.  If a stacking context sets `opacity < 1`, the colors
    /// you see here are the unblended source colors — the actual on-screen appearance
    /// depends on the compositor blending them at render time.  Use these accessors for
    /// structural inspection (e.g. unit tests), not for pixel-accurate color assertions.
    pub fn texts(&self) -> Vec<TextCommand> {
        collect_texts(&self.commands, 0, 0)
    }
    /// Flatten all rect commands across the command tree, including those inside layers.
    ///
    /// See [`texts`](Self::texts) for the note on opacity and unblended colors.
    pub fn rects(&self) -> Vec<RectCommand> {
        collect_rects(&self.commands, 0, 0)
    }
    /// Flatten all image commands across the command tree, including those inside layers.
    ///
    /// See [`texts`](Self::texts) for the note on opacity and unblended colors.
    pub fn images(&self) -> Vec<ImageCommand> {
        collect_images(&self.commands, 0, 0)
    }
}

/// Shift a DrawCommand by (dx, dy), saturating on overflow.
fn shift_command(cmd: &mut DrawCommand, dx: u32, dy: u32) {
    match cmd {
        DrawCommand::Rect(r) => {
            r.x = r.x.saturating_add(dx);
            r.y = r.y.saturating_add(dy);
        }
        DrawCommand::Text(t) => {
            t.x = t.x.saturating_add(dx);
            t.y = t.y.saturating_add(dy);
        }
        DrawCommand::Image(i) => {
            i.x = i.x.saturating_add(dx);
            i.y = i.y.saturating_add(dy);
        }
        DrawCommand::Layer(l) => {
            l.x = l.x.saturating_add(dx);
            l.y = l.y.saturating_add(dy);
        }
    }
}

fn shift_command_signed(cmd: &mut DrawCommand, dx: i32, dy: i32) {
    match cmd {
        DrawCommand::Rect(r) => {
            r.x = (r.x as i64 + dx as i64).max(0) as u32;
            r.y = (r.y as i64 + dy as i64).max(0) as u32;
        }
        DrawCommand::Text(t) => {
            t.x = (t.x as i64 + dx as i64).max(0) as u32;
            t.y = (t.y as i64 + dy as i64).max(0) as u32;
        }
        DrawCommand::Image(i) => {
            i.x = (i.x as i64 + dx as i64).max(0) as u32;
            i.y = (i.y as i64 + dy as i64).max(0) as u32;
        }
        DrawCommand::Layer(l) => {
            l.x = (l.x as i64 + dx as i64).max(0) as u32;
            l.y = (l.y as i64 + dy as i64).max(0) as u32;
        }
    }
}

fn collect_texts(commands: &[DrawCommand], offset_x: u32, offset_y: u32) -> Vec<TextCommand> {
    let mut out = Vec::new();
    for cmd in commands {
        match cmd {
            DrawCommand::Text(t) => {
                let mut t2 = t.clone();
                t2.x = t2.x.saturating_add(offset_x);
                t2.y = t2.y.saturating_add(offset_y);
                out.push(t2);
            }
            DrawCommand::Layer(l) => {
                out.extend(collect_texts(&l.commands, offset_x.saturating_add(l.x), offset_y.saturating_add(l.y)));
            }
            _ => {}
        }
    }
    out
}

fn collect_rects(commands: &[DrawCommand], offset_x: u32, offset_y: u32) -> Vec<RectCommand> {
    let mut out = Vec::new();
    for cmd in commands {
        match cmd {
            DrawCommand::Rect(r) => {
                let mut r2 = r.clone();
                r2.x = r2.x.saturating_add(offset_x);
                r2.y = r2.y.saturating_add(offset_y);
                out.push(r2);
            }
            DrawCommand::Layer(l) => {
                out.extend(collect_rects(&l.commands, offset_x.saturating_add(l.x), offset_y.saturating_add(l.y)));
            }
            _ => {}
        }
    }
    out
}

fn collect_images(commands: &[DrawCommand], offset_x: u32, offset_y: u32) -> Vec<ImageCommand> {
    let mut out = Vec::new();
    for cmd in commands {
        match cmd {
            DrawCommand::Image(i) => {
                let mut i2 = i.clone();
                i2.x = i2.x.saturating_add(offset_x);
                i2.y = i2.y.saturating_add(offset_y);
                out.push(i2);
            }
            DrawCommand::Layer(l) => {
                out.extend(collect_images(&l.commands, offset_x.saturating_add(l.x), offset_y.saturating_add(l.y)));
            }
            _ => {}
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RectCommand {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub color: Color,
    pub border_radius: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextCommand {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub text: String,
    pub font_size_px: u32,
    pub line_height_px: u32,
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
    pub object_fit: ObjectFit,
    pub object_position_x: u32,
    pub object_position_y: u32,
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

/// Scan the document tree for a body/html element with a solid background color.
/// Used to fill canvas margins without double-applying opacity.
///
/// Typical documents are `document > html > body`, so `body` is a grandchild of the
/// root node. We recurse the full tree rather than only checking direct children.
fn extract_body_background(node: &StyledNode) -> Option<u32> {
    if let StyledNode::Element(el) = node {
        // Check this element itself
        if (el.tag_name == "body" || el.tag_name == "html") && el.style.opacity == 255 {
            if let Some(bg) = el.style.background_color {
                return Some(bg);
            }
        }
        // Breadth-first: check direct children before recursing deeper
        for child in &el.children {
            if let StyledNode::Element(child_el) = child {
                if (child_el.tag_name == "body" || child_el.tag_name == "html")
                    && child_el.style.opacity == 255
                {
                    if let Some(bg) = child_el.style.background_color {
                        return Some(bg);
                    }
                }
            }
        }
        // Recurse deeper for documents with extra nesting layers
        for child in &el.children {
            if let Some(bg) = extract_body_background(child) {
                return Some(bg);
            }
        }
    }
    None
}


pub fn layout_styled_document(
    document: &StyledNode,
    images: &ImageStore,
    viewport_width: u32,
    fonts: &mut FontContext,
) -> LayoutDocument {
    // Use body/html background if available and fully opaque.
    // When body has opacity < 1, layout_block_element_as_layer wraps it in a LayerCommand
    // which composites at render time, so we keep DEFAULT_BACKGROUND_COLOR to avoid
    // double-applying opacity.
    let canvas_bg = extract_body_background(document).unwrap_or(DEFAULT_BACKGROUND_COLOR);
    let mut context = LayoutContext {
        background_color: canvas_bg,
        ..LayoutContext::default()
    };
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

    // Append absolutely/fixed positioned elements sorted by z-index
    let mut positioned = std::mem::take(&mut context.positioned_commands);
    positioned.sort_by_key(|(z, _)| *z);
    for (_, cmds) in positioned {
        context.commands.extend(cmds);
    }

    LayoutDocument {
        background_color: canvas_bg,
        content_height: cursor_y,
        commands: context.commands,
        links: context.links,
        controls: context.controls,
        element_hitboxes: context.element_hitboxes,
    }
}

struct LayoutContext {
    background_color: Color,
    commands: Vec<DrawCommand>,
    links: Vec<LinkCommand>,
    controls: Vec<FormControlCommand>,
    element_hitboxes: Vec<ElementHitbox>,
    next_control_id: usize,
    next_form_id: usize,
    containing_block_origin: (u32, u32),
    scroll_y_for_fixed: u32,
    positioned_commands: Vec<(i32, Vec<DrawCommand>)>,
}

impl Default for LayoutContext {
    fn default() -> Self {
        Self {
            background_color: DEFAULT_BACKGROUND_COLOR,
            commands: Vec::new(),
            links: Vec::new(),
            controls: Vec::new(),
            element_hitboxes: Vec::new(),
            next_control_id: 0,
            next_form_id: 0,
            containing_block_origin: (0, 0),
            scroll_y_for_fixed: 0,
            positioned_commands: Vec::new(),
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

            // Handle positioned elements (absolute/fixed) — they don't contribute to flow
            if element.style.position == Position::Absolute || element.style.position == Position::Fixed {
                layout_positioned_element(element, x, width, cursor_y, context, images, fonts, current_form.clone());
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
                        if link_height > 0 && !element.style.pointer_events_none {
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
                Display::Flex => {
                    let current_form = form_context_for_element(element, context, current_form);
                    layout_flex_container(element, x, width, cursor_y, context, images, fonts, current_form.clone());
                }
                Display::InlineFlex => {
                    // Inline-flex: behaves like flex internally but inline in parent flow.
                    // Treat it as a flex container with full available width here (block-level fallback).
                    let current_form = form_context_for_element(element, context, current_form);
                    let inline_width = element.style.width
                        .map(|w| match w {
                            LengthValue::Pixels(px) => px,
                            LengthValue::Percent(pct) => (width as f32 * pct as f32 / 100.0) as u32,
                            LengthValue::MinContent => 0,
                            LengthValue::MaxContent => width,
                            LengthValue::FitContent(max_px) => width.min(max_px),
                        })
                        .unwrap_or(width);
                    layout_flex_container(element, x, inline_width, cursor_y, context, images, fonts, current_form.clone());
                }
                Display::Grid => {
                    let current_form = form_context_for_element(element, context, current_form);
                    layout_grid_container(element, x, width, cursor_y, context, images, fonts, current_form);
                }
                Display::InlineGrid => {
                    let current_form = form_context_for_element(element, context, current_form);
                    let inline_width = element.style.width
                        .map(|w| match w {
                            LengthValue::Pixels(px) => px,
                            LengthValue::Percent(pct) => (width as f32 * pct as f32 / 100.0) as u32,
                            LengthValue::MinContent => 0,
                            LengthValue::MaxContent => width,
                            LengthValue::FitContent(max_px) => width.min(max_px),
                        })
                        .unwrap_or(width);
                    layout_grid_container(element, x, inline_width, cursor_y, context, images, fonts, current_form);
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
        if element.style.opacity < 255 || element.style.filter_blur_px > 0 || element.style.filter_brightness != 10000 {
            // Table with opacity/filter: render into sub-context and wrap in a LayerCommand
            let mut sub_context = LayoutContext {
                background_color: context.background_color,
                ..LayoutContext::default()
            };
            sub_context.next_control_id = context.next_control_id;
            sub_context.next_form_id = context.next_form_id;

            let y_before = *cursor_y;
            layout_table_element(element, x, width, cursor_y, &mut sub_context, images, fonts, current_form.clone());
            let table_height = cursor_y.saturating_sub(y_before).max(1);
            rebase_commands(&mut sub_context.commands, x, y_before);
            context.commands.push(DrawCommand::Layer(LayerCommand {
                x,
                y: y_before,
                width: width.max(1),
                height: table_height,
                opacity: element.style.opacity,
                blur_px: element.style.filter_blur_px,
                brightness: element.style.filter_brightness,
                commands: sub_context.commands,
            }));
            context.links.extend(sub_context.links);
            context.controls.extend(sub_context.controls);
            context.element_hitboxes.extend(sub_context.element_hitboxes);
            context.next_control_id = sub_context.next_control_id;
            context.next_form_id = sub_context.next_form_id;
        } else {
            layout_table_element(element, x, width, cursor_y, context, images, fonts, current_form);
        }
        return;
    }

    let block_cmd_start = context.commands.len();

    *cursor_y = cursor_y.saturating_add(element.style.margin.top);

    let outer_x = x.saturating_add(element.style.margin.left);
    let raw_outer_width =
        width.saturating_sub(element.style.margin.left + element.style.margin.right);
    // Apply min/max-width
    let outer_width = raw_outer_width
        .min(element.style.max_width.unwrap_or(u32::MAX))
        .max(element.style.min_width);
    let background_top = *cursor_y;

    // Detect stacking context: element has opacity < 255 or a filter effect
    if element.style.opacity < 255 || element.style.filter_blur_px > 0 || element.style.filter_brightness != 10000 {
        layout_block_element_as_layer(
            element, outer_x, outer_width, background_top, cursor_y, context, images, fonts, current_form,
        );
        *cursor_y = cursor_y.saturating_add(element.style.margin.bottom);
        return;
    }

    let saved_bg = context.background_color;

    // box-shadow: push shadow rect before background (so it renders behind it)
    let shadow_cmd_index = if let Some(ref shadow) = element.style.box_shadow {
        let blur = shadow.blur;
        // Expand shadow rect by blur amount in all directions for approximate blur spread
        let sx = (outer_x as i64 + shadow.offset_x as i64 - blur as i64).max(0) as u32;
        let sy = (background_top as i64 + shadow.offset_y as i64 - blur as i64).max(0) as u32;
        let sw = outer_width.saturating_add(blur.saturating_mul(2)).max(1);
        context.commands.push(DrawCommand::Rect(RectCommand {
            x: sx,
            y: sy,
            width: sw,
            height: 1,
            color: shadow.color.unwrap_or(element.style.color),
            border_radius: element.style.border_radius.saturating_add(blur),
        }));
        Some(context.commands.len() - 1)
    } else {
        None
    };

    let background_cmd_index = if let Some(background_color) = element.style.background_color {
        // Use effective_opacity for the actual drawn rect color (correct visual result)
        let blended_for_rect = apply_opacity(
            background_color,
            context.background_color,
            element.style.effective_opacity,
        );
        context.commands.push(DrawCommand::Rect(RectCommand {
            x: outer_x,
            y: background_top,
            width: outer_width.max(1),
            height: 1,
            color: blended_for_rect,
            border_radius: element.style.border_radius,
        }));
        if element.style.effective_opacity == 255 {
            // Fully opaque: children blend against this element's solid background
            context.background_color = background_color;
        }
        // If opacity < 255: don't update — children keep the parent/canvas backdrop
        Some(context.commands.len() - 1)
    } else {
        None
    };

    // Capture clip start BEFORE children are laid out, so overflow:hidden can correctly
    // filter commands added by children (even when there is no background rect).
    let clip_start_idx = context.commands.len();

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
        context.commands.push(DrawCommand::Rect(RectCommand {
            x: content_x,
            y: *cursor_y,
            width: content_width,
            height: 2,
            color: element.style.color,
            border_radius: 0,
        }));
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

    // Emit element hitbox for interactive state (hover/focus) detection
    if let Some(node_id) = element_node_id(element) {
        if background_height > 0 && !element.style.pointer_events_none {
            context.element_hitboxes.push(ElementHitbox {
                node_id,
                x: outer_x,
                y: background_top,
                width: outer_width.max(1),
                height: background_height,
                cursor_kind: element.style.cursor_kind,
            });
        }
    }

    if let Some(shadow_idx) = shadow_cmd_index {
        if let Some(DrawCommand::Rect(rect)) = context.commands.get_mut(shadow_idx) {
            let blur = element.style.box_shadow.as_ref().map(|s| s.blur).unwrap_or(0);
            rect.height = background_height.saturating_add(blur.saturating_mul(2));
        }
    }
    if let Some(background_cmd_index) = background_cmd_index {
        if let Some(DrawCommand::Rect(rect)) = context.commands.get_mut(background_cmd_index) {
            rect.height = background_height;
        }
    }

    // Restore parent background color after children are rendered
    context.background_color = saved_bg;

    // overflow: hidden — clip commands that fall outside the element box
    // Use clip_start_idx (captured before children were laid out) so that child
    // commands are correctly filtered even when there is no background rect.
    if element.style.overflow == Overflow::Hidden {
        let clip_height = element.style.height
            .map(|lv| match lv {
                LengthValue::Pixels(px) => px,
                LengthValue::Percent(_) => background_height, // can't resolve % without context
                LengthValue::MinContent | LengthValue::MaxContent | LengthValue::FitContent(_) => background_height,
            })
            .unwrap_or(background_height);
        clip_commands_to_box(
            &mut context.commands,
            clip_start_idx,
            outer_x,
            background_top,
            outer_width,
            clip_height,
        );
    }

    // Draw borders if present
    if !element.style.border_style_none {
        let bc = apply_opacity(
            element.style.border_color,
            context.background_color,
            element.style.effective_opacity,
        );
        let border_top_h = element.style.border.top;
        let border_bottom_h = element.style.border.bottom;
        let border_left_w = element.style.border.left;
        let border_right_w = element.style.border.right;

        if border_top_h > 0 {
            context.commands.push(DrawCommand::Rect(RectCommand {
                x: outer_x,
                y: background_top,
                width: outer_width.max(1),
                height: border_top_h,
                color: bc,
                border_radius: element.style.border_radius,
            }));
        }
        if border_bottom_h > 0 {
            context.commands.push(DrawCommand::Rect(RectCommand {
                x: outer_x,
                y: cursor_y.saturating_sub(border_bottom_h),
                width: outer_width.max(1),
                height: border_bottom_h,
                color: bc,
                border_radius: element.style.border_radius,
            }));
        }
        if border_left_w > 0 {
            context.commands.push(DrawCommand::Rect(RectCommand {
                x: outer_x,
                y: background_top,
                width: border_left_w,
                height: background_height,
                color: bc,
                border_radius: 0,
            }));
        }
        if border_right_w > 0 {
            context.commands.push(DrawCommand::Rect(RectCommand {
                x: outer_x
                    .saturating_add(outer_width)
                    .saturating_sub(border_right_w),
                y: background_top,
                width: border_right_w,
                height: background_height,
                color: bc,
                border_radius: 0,
            }));
        }
    }

    // position: relative / sticky — apply visual offset without affecting flow
    // sticky: lay out in normal flow then apply top/bottom as minimum offsets (same as relative
    // for now; true scroll-based stickiness requires scroll state propagation).
    if element.style.position == Position::Relative || element.style.position == Position::Sticky {
        let dx = element.style.left.unwrap_or(0) - element.style.right.unwrap_or(0);
        let dy = element.style.top.unwrap_or(0) - element.style.bottom.unwrap_or(0);
        if dx != 0 || dy != 0 {
            for cmd in &mut context.commands[block_cmd_start..] {
                shift_command_signed(cmd, dx, dy);
            }
        }
    }

    *cursor_y = cursor_y.saturating_add(element.style.margin.bottom);
}

/// Remove or clamp draw commands (from index `start`) to the given clip box.
/// Commands entirely outside the box are removed.
/// Rect, Image, and Layer commands that partially overlap are clamped to the clip bounds.
/// Text commands are filtered by bounding box only (not clamped horizontally).
///
/// Note: Image clamping changes the destination rect size, which will rescale the image
/// at render time rather than cropping it. For pixel-perfect image overflow, a source-rect
/// crop would be needed.
/// Note: Layer clamping adjusts width/height but does not rebase inner commands; the
/// compositor clips at the layer's new dimensions.
fn clip_commands_to_box(
    commands: &mut Vec<DrawCommand>,
    start: usize,
    clip_x: u32,
    clip_y: u32,
    clip_w: u32,
    clip_h: u32,
) {
    let clip_x2 = clip_x.saturating_add(clip_w);
    let clip_y2 = clip_y.saturating_add(clip_h);

    let tail = commands.split_off(start);
    let clamped: Vec<DrawCommand> = tail.into_iter().filter_map(|cmd| {
        match cmd {
            DrawCommand::Rect(mut r) => {
                let rx2 = r.x.saturating_add(r.width);
                let ry2 = r.y.saturating_add(r.height);
                // entirely outside?
                if r.x >= clip_x2 || r.y >= clip_y2 || rx2 <= clip_x || ry2 <= clip_y {
                    return None;
                }
                // clamp to clip box
                let new_x = r.x.max(clip_x);
                let new_y = r.y.max(clip_y);
                let new_x2 = rx2.min(clip_x2);
                let new_y2 = ry2.min(clip_y2);
                r.x = new_x; r.y = new_y;
                r.width = new_x2.saturating_sub(new_x).max(1);
                r.height = new_y2.saturating_sub(new_y).max(1);
                Some(DrawCommand::Rect(r))
            }
            DrawCommand::Image(img) => {
                let ix2 = img.x.saturating_add(img.width);
                let iy2 = img.y.saturating_add(img.height);
                // Only discard entirely-outside images; don't resize (clamping x/y/width/height
                // would rescale the full image into a smaller rect instead of cropping it).
                // Pixel-accurate cropping would require source-rect support in the renderer.
                if img.x >= clip_x2 || img.y >= clip_y2 || ix2 <= clip_x || iy2 <= clip_y {
                    None
                } else {
                    Some(DrawCommand::Image(img))
                }
            }
            DrawCommand::Layer(mut l) => {
                let lx2 = l.x.saturating_add(l.width);
                let ly2 = l.y.saturating_add(l.height);
                if l.x >= clip_x2 || l.y >= clip_y2 || lx2 <= clip_x || ly2 <= clip_y {
                    return None;
                }
                // Clamp width/height only — do NOT change x/y.
                // Changing x/y would shift the layer's screen position without rebasing inner
                // commands (which are layer-relative), causing them to render at the wrong position.
                // The compositor clips at the layer's dimensions, so reducing width/height is enough
                // to limit the visible area.
                l.width = lx2.min(clip_x2).saturating_sub(l.x).max(1);
                l.height = ly2.min(clip_y2).saturating_sub(l.y).max(1);
                Some(DrawCommand::Layer(l))
            }
            DrawCommand::Text(t) => {
                let ty2 = t.y.saturating_add(t.line_height_px);
                let tx2 = t.x.saturating_add(t.width);
                if t.x >= clip_x2 || t.y >= clip_y2 || tx2 <= clip_x || ty2 <= clip_y {
                    None
                } else {
                    Some(DrawCommand::Text(t))
                }
            }
        }
    }).collect();
    commands.extend(clamped);
}

fn rebase_commands(commands: &mut Vec<DrawCommand>, origin_x: u32, origin_y: u32) {
    for cmd in commands.iter_mut() {
        match cmd {
            DrawCommand::Rect(r) => {
                r.x = r.x.saturating_sub(origin_x);
                r.y = r.y.saturating_sub(origin_y);
            }
            DrawCommand::Text(t) => {
                t.x = t.x.saturating_sub(origin_x);
                t.y = t.y.saturating_sub(origin_y);
            }
            DrawCommand::Image(i) => {
                i.x = i.x.saturating_sub(origin_x);
                i.y = i.y.saturating_sub(origin_y);
            }
            DrawCommand::Layer(l) => {
                l.x = l.x.saturating_sub(origin_x);
                l.y = l.y.saturating_sub(origin_y);
                // Do NOT recurse into l.commands — they're already layer-relative
            }
        }
    }
}

/// Layout a block element that needs opacity compositing via a LayerCommand.
///
/// TODO(refactor): This function duplicates ~200 lines from `layout_block_element`:
/// padding, bullet indent, hr handling, background rect fixup, border drawing,
/// box-shadow, and overflow clipping. Changes to either path must be manually
/// mirrored to the other or the two paths will silently diverge.
/// A shared helper taking a `&mut LayoutContext` (sub-context vs parent context)
/// would eliminate the duplication.
fn layout_block_element_as_layer(
    element: &StyledElement,
    outer_x: u32,
    outer_width: u32,
    background_top: u32,
    cursor_y: &mut u32,
    context: &mut LayoutContext,
    images: &ImageStore,
    fonts: &mut FontContext,
    current_form: Option<FormContext>,
) {
    // Create a sub-context for the element's subtree
    let mut sub_context = LayoutContext {
        background_color: context.background_color,
        next_control_id: context.next_control_id,
        next_form_id: context.next_form_id,
        ..LayoutContext::default()
    };

    // box-shadow: push shadow rect before background (so it renders behind it)
    let shadow_cmd_index = if let Some(ref shadow) = element.style.box_shadow {
        let blur = shadow.blur;
        // Clamp shadow origin to the element's own top-left corner.
        // Without clamping, a shadow with a negative offset or large blur can produce
        // sx < outer_x or sy < background_top. The subsequent rebase_commands call uses
        // saturating_sub(outer_x, background_top), which clamps negative offsets to 0 and
        // corrupts the shadow position. By clamping to the element box we lose shadow that
        // extends above/left of the element, but avoid rebase corruption.
        let sx = (outer_x as i64 + shadow.offset_x as i64 - blur as i64)
            .max(outer_x as i64) as u32; // don't go left of element
        let sy = (background_top as i64 + shadow.offset_y as i64 - blur as i64)
            .max(background_top as i64) as u32; // don't go above element
        let sw = outer_width.saturating_add(blur.saturating_mul(2)).max(1);
        sub_context.commands.push(DrawCommand::Rect(RectCommand {
            x: sx,
            y: sy,
            width: sw,
            height: 1,
            color: shadow.color.unwrap_or(element.style.color),
            border_radius: element.style.border_radius.saturating_add(blur),
        }));
        Some(sub_context.commands.len() - 1)
    } else {
        None
    };

    // The element's own background rect goes into the sub-context (raw color, no opacity blend)
    let background_cmd_index = if let Some(background_color) = element.style.background_color {
        // Use raw background color — opacity is applied by the layer compositor
        sub_context.commands.push(DrawCommand::Rect(RectCommand {
            x: outer_x,
            y: background_top,
            width: outer_width.max(1),
            height: 1,
            color: background_color,
            border_radius: element.style.border_radius,
        }));
        // Update sub_context backdrop for children
        sub_context.background_color = background_color;
        Some(sub_context.commands.len() - 1)
    } else {
        None
    };

    // Capture clip index BEFORE children so overflow:hidden can clip child commands
    let clip_start_idx = sub_context.commands.len();

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
        sub_context.commands.push(DrawCommand::Rect(RectCommand {
            x: content_x,
            y: *cursor_y,
            width: content_width,
            height: 2,
            color: element.style.color,
            border_radius: 0,
        }));
        *cursor_y = cursor_y.saturating_add(10);
    } else {
        layout_mixed_children(
            element,
            content_x,
            content_width,
            cursor_y,
            &mut sub_context,
            bullet_indent > 0,
            images,
            fonts,
            current_form,
        );
    }

    *cursor_y = cursor_y.saturating_add(element.style.padding.bottom);
    let final_height = cursor_y.saturating_sub(background_top).max(1);

    if let Some(shadow_idx) = shadow_cmd_index {
        if let Some(DrawCommand::Rect(rect)) = sub_context.commands.get_mut(shadow_idx) {
            let blur = element.style.box_shadow.as_ref().map(|s| s.blur).unwrap_or(0);
            rect.height = final_height.saturating_add(blur.saturating_mul(2));
        }
    }
    if let Some(background_cmd_index) = background_cmd_index {
        if let Some(DrawCommand::Rect(rect)) = sub_context.commands.get_mut(background_cmd_index) {
            rect.height = final_height;
        }
    }

    // Draw borders into the sub-context (they are part of the composited layer)
    if !element.style.border_style_none {
        // Borders use raw border_color since they're inside the layer
        let bc = element.style.border_color;
        let border_top_h = element.style.border.top;
        let border_bottom_h = element.style.border.bottom;
        let border_left_w = element.style.border.left;
        let border_right_w = element.style.border.right;

        if border_top_h > 0 {
            sub_context.commands.push(DrawCommand::Rect(RectCommand {
                x: outer_x,
                y: background_top,
                width: outer_width.max(1),
                height: border_top_h,
                color: bc,
                border_radius: element.style.border_radius,
            }));
        }
        if border_bottom_h > 0 {
            sub_context.commands.push(DrawCommand::Rect(RectCommand {
                x: outer_x,
                y: cursor_y.saturating_sub(border_bottom_h),
                width: outer_width.max(1),
                height: border_bottom_h,
                color: bc,
                border_radius: element.style.border_radius,
            }));
        }
        if border_left_w > 0 {
            sub_context.commands.push(DrawCommand::Rect(RectCommand {
                x: outer_x,
                y: background_top,
                width: border_left_w,
                height: final_height,
                color: bc,
                border_radius: 0,
            }));
        }
        if border_right_w > 0 {
            sub_context.commands.push(DrawCommand::Rect(RectCommand {                x: outer_x
                    .saturating_add(outer_width)
                    .saturating_sub(border_right_w),
                y: background_top,
                width: border_right_w,
                height: final_height,
                color: bc,
                border_radius: 0,
            }));
        }
    }

    // overflow: hidden — clip child commands within the element box
    if element.style.overflow == Overflow::Hidden {
        let clip_height = element.style.height
            .map(|lv| match lv {
                LengthValue::Pixels(px) => px,
                LengthValue::Percent(_) => final_height,
                LengthValue::MinContent | LengthValue::MaxContent | LengthValue::FitContent(_) => final_height,
            })
            .unwrap_or(final_height);
        clip_commands_to_box(
            &mut sub_context.commands,
            clip_start_idx,
            outer_x,
            background_top,
            outer_width,
            clip_height,
        );
    }

    // Rebase sub-commands to layer-relative coordinates before wrapping
    rebase_commands(&mut sub_context.commands, outer_x, background_top);

    // Wrap sub-context commands in a LayerCommand and push to parent
    context.commands.push(DrawCommand::Layer(LayerCommand {
        x: outer_x,
        y: background_top,
        width: outer_width.max(1),
        height: final_height,
        opacity: element.style.opacity,
        blur_px: element.style.filter_blur_px,
        brightness: element.style.filter_brightness,
        commands: sub_context.commands,
    }));

    // Propagate links, controls, and element hitboxes from sub_context to parent
    context.links.extend(sub_context.links);
    context.controls.extend(sub_context.controls);
    context.element_hitboxes.extend(sub_context.element_hitboxes);
    context.next_control_id = sub_context.next_control_id;
    context.next_form_id = sub_context.next_form_id;
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

    if element.style.opacity < 255 || element.style.filter_blur_px > 0 || element.style.filter_brightness != 10000 {
        // Wrap the image in a LayerCommand so opacity/filters are applied correctly
        let img_cmd = DrawCommand::Image(ImageCommand {
            x: 0,
            y: 0,
            width: draw_width,
            height: draw_height,
            src: src.to_string(),
            object_fit: element.style.object_fit,
            object_position_x: element.style.object_position_x,
            object_position_y: element.style.object_position_y,
        });
        context.commands.push(DrawCommand::Layer(LayerCommand {
            x: draw_x,
            y: *cursor_y,
            width: draw_width,
            height: draw_height,
            opacity: element.style.opacity,
            blur_px: element.style.filter_blur_px,
            brightness: element.style.filter_brightness,
            commands: vec![img_cmd],
        }));
    } else {
        context.commands.push(DrawCommand::Image(ImageCommand {
            x: draw_x,
            y: *cursor_y,
            width: draw_width,
            height: draw_height,
            src: src.to_string(),
            object_fit: element.style.object_fit,
            object_position_x: element.style.object_position_x,
            object_position_y: element.style.object_position_y,
        }));
    }

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
    let mut height = if let Some(ratio_milli) = element.style.aspect_ratio {
        // CSS aspect-ratio overrides intrinsic ratio for height calculation
        let ratio = ratio_milli as f32 / 1000.0;
        height_attr.unwrap_or_else(|| (width as f32 / ratio).round().max(1.0) as u32)
    } else {
        height_attr.unwrap_or_else(|| {
            scaled_dimension(intrinsic_height.max(1), width, intrinsic_width.max(1))
        })
    };

    if width > max_width && width > 0 {
        height = scaled_dimension(height.max(1), max_width.max(1), width);
        width = max_width.max(1);
    }

    if height_attr.is_some() && width_attr.is_none() && element.style.aspect_ratio.is_none() {
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
        let cell_backdrop = placement.cell.style.background_color
            .unwrap_or(context.background_color);
        let layout = layout_table_cell(
            placement.cell,
            inner_width,
            images,
            fonts,
            cell_backdrop,
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

        let content_area_height = cell_height.saturating_sub(padding.saturating_mul(2));
        let vertical_offset = match placement.cell.style.vertical_align {
            VerticalAlign::Top => 0,
            VerticalAlign::Middle => content_area_height.saturating_sub(layout.content_height) / 2,
            VerticalAlign::Bottom => content_area_height.saturating_sub(layout.content_height),
        };

        let content_x = cell_x.saturating_add(padding);
        let content_y = cell_y.saturating_add(padding).saturating_add(vertical_offset);

        if placement.cell.style.opacity < 255 {
            // Wrap cell content in a LayerCommand for opacity compositing.
            // Emit the background rect INSIDE the layer with the raw (unblended) color so
            // it is composited once by the LayerCommand — not pre-blended into the parent.
            let layer_w = cell_width.max(1);
            let layer_h = cell_height.max(1);
            let mut layer_commands = Vec::new();
            if let Some(background_color) = placement.cell.style.background_color {
                layer_commands.push(DrawCommand::Rect(RectCommand {
                    x: 0,
                    y: 0,
                    width: layer_w,
                    height: layer_h,
                    color: background_color,
                    border_radius: 0,
                }));
            }
            // Content commands are (0,0)-relative within the cell; offset by padding/valign
            let pad_x = padding;
            let pad_y = padding.saturating_add(vertical_offset);
            for cmd in &layout.commands {
                let mut shifted = cmd.clone();
                shift_command(&mut shifted, pad_x, pad_y);
                layer_commands.push(shifted);
            }
            context.commands.push(DrawCommand::Layer(LayerCommand {
                x: cell_x,
                y: cell_y,
                width: layer_w,
                height: layer_h,
                opacity: placement.cell.style.opacity,
                blur_px: placement.cell.style.filter_blur_px,
                brightness: placement.cell.style.filter_brightness,
                commands: layer_commands,
            }));
            // Links are content-relative; shift by cell position + padding/valign
            context.links.extend(layout.links.iter().map(|link| LinkCommand {
                node_id: link.node_id,
                x: link.x.saturating_add(cell_x).saturating_add(padding),
                y: link.y.saturating_add(cell_y).saturating_add(padding).saturating_add(vertical_offset),
                width: link.width,
                height: link.height,
                href: link.href.clone(),
            }));
            context.controls.extend(layout.controls.iter().map(|ctrl| FormControlCommand {
                id: ctrl.id,
                node_id: ctrl.node_id,
                form_node_id: ctrl.form_node_id,
                kind: ctrl.kind,
                x: ctrl.x.saturating_add(cell_x).saturating_add(padding),
                y: ctrl.y.saturating_add(cell_y).saturating_add(padding).saturating_add(vertical_offset),
                width: ctrl.width,
                height: ctrl.height,
                name: ctrl.name.clone(),
                value: ctrl.value.clone(),
                label: ctrl.label.clone(),
                placeholder: ctrl.placeholder.clone(),
                form_id: ctrl.form_id,
                form_action: ctrl.form_action.clone(),
                form_method: ctrl.form_method.clone(),
                activates_submit: ctrl.activates_submit,
                disabled: ctrl.disabled,
                masked: ctrl.masked,
                font_size_px: ctrl.font_size_px,
                font_family: ctrl.font_family,
                text_color: ctrl.text_color,
                background_color: ctrl.background_color,
                border_color: ctrl.border_color,
            }));
            context.element_hitboxes.extend(layout.element_hitboxes.iter().map(|h| ElementHitbox {
                node_id: h.node_id,
                x: h.x.saturating_add(cell_x).saturating_add(padding),
                y: h.y.saturating_add(cell_y).saturating_add(padding).saturating_add(vertical_offset),
                width: h.width,
                height: h.height,
                cursor_kind: h.cursor_kind,
            }));
        } else {
            // opacity == 255: emit background rect directly into parent context
            if let Some(background_color) = placement.cell.style.background_color {
                let blended = apply_opacity(
                    background_color,
                    context.background_color,
                    placement.cell.style.effective_opacity,
                );
                context.commands.push(DrawCommand::Rect(RectCommand {
                    x: cell_x,
                    y: cell_y,
                    width: cell_width.max(1),
                    height: cell_height.max(1),
                    color: blended,
                    border_radius: 0,
                }));
            }
            merge_fragment(context, layout, content_x, content_y);
        }
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
    commands: Vec<DrawCommand>,
    links: Vec<LinkCommand>,
    controls: Vec<FormControlCommand>,
    element_hitboxes: Vec<ElementHitbox>,
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
    background_color: Color,
    current_form: Option<FormContext>,
    control_id_seed: usize,
    form_id_seed: usize,
) -> FragmentLayout {
    let mut context = LayoutContext {
        background_color,
        commands: Vec::new(),
        links: Vec::new(),
        controls: Vec::new(),
        next_control_id: control_id_seed,
        next_form_id: form_id_seed,
        ..LayoutContext::default()
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
        commands: context.commands,
        links: context.links,
        controls: context.controls,
        element_hitboxes: context.element_hitboxes,
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
    for cmd in &fragment.commands {
        context.commands.push(offset_draw_command(cmd, offset_x, offset_y));
    }
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
    context
        .element_hitboxes
        .extend(fragment.element_hitboxes.iter().map(|h| ElementHitbox {
            node_id: h.node_id,
            x: h.x.saturating_add(offset_x),
            y: h.y.saturating_add(offset_y),
            width: h.width,
            height: h.height,
            cursor_kind: h.cursor_kind,
        }));
}

fn offset_draw_command(cmd: &DrawCommand, offset_x: u32, offset_y: u32) -> DrawCommand {
    match cmd {
        DrawCommand::Rect(rect) => DrawCommand::Rect(RectCommand {
            x: rect.x.saturating_add(offset_x),
            y: rect.y.saturating_add(offset_y),
            width: rect.width,
            height: rect.height,
            color: rect.color,
            border_radius: rect.border_radius,
        }),
        DrawCommand::Text(text) => DrawCommand::Text(TextCommand {
            x: text.x.saturating_add(offset_x),
            y: text.y.saturating_add(offset_y),
            width: text.width,
            text: text.text.clone(),
            font_size_px: text.font_size_px,
            line_height_px: text.line_height_px,
            font_family: text.font_family,
            color: text.color,
            underline: text.underline,
            bold: text.bold,
            italic: text.italic,
        }),
        DrawCommand::Image(image) => DrawCommand::Image(ImageCommand {
            x: image.x.saturating_add(offset_x),
            y: image.y.saturating_add(offset_y),
            width: image.width,
            height: image.height,
            src: image.src.clone(),
            object_fit: image.object_fit,
            object_position_x: image.object_position_x,
            object_position_y: image.object_position_y,
        }),
        DrawCommand::Layer(layer) => DrawCommand::Layer(LayerCommand {
            x: layer.x.saturating_add(offset_x),
            y: layer.y.saturating_add(offset_y),
            width: layer.width,
            height: layer.height,
            opacity: layer.opacity,
            blur_px: layer.blur_px,
            brightness: layer.brightness,
            commands: layer.commands.clone(),
        }),
    }
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
                Display::Block
                | Display::ListItem
                | Display::Flex
                | Display::InlineFlex
                | Display::Grid
                | Display::InlineGrid => {}
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
                link_node_id,            } => {
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
    let mut first_line = true;
    let text_indent = container_style.text_indent;

    for fragment in fragments {
        match fragment {
            InlineFragment::LineBreak => {
                emit_line_with_indent(
                    &mut line, container_style, x, width, cursor_y, context, fonts,
                    if first_line { text_indent } else { 0 },
                );
                first_line = false;
            }
            InlineFragment::Control(control) => {
                let (control_width, _) = measure_form_control(control, fonts);
                let effective_width = if first_line {
                    width.saturating_sub(text_indent)
                } else {
                    width
                };
                // Only emit current line if the control won't fit inline
                if !line.is_empty() && line.width.saturating_add(control_width) > effective_width {
                    emit_line_with_indent(
                        &mut line, container_style, x, width, cursor_y, context, fonts,
                        if first_line { text_indent } else { 0 },
                    );
                    first_line = false;
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
                        emit_line_with_indent(
                            &mut line, container_style, x, width, cursor_y, context, fonts,
                            if first_line { text_indent } else { 0 },
                        );
                        first_line = false;
                        continue;
                    }

                    let character_width = char_width(style, character, fonts);
                    let eff_w = if first_line { width.saturating_sub(text_indent) } else { width };
                    if !line.is_empty() && line.width.saturating_add(character_width) > eff_w {
                        emit_line_with_indent(
                            &mut line, container_style, x, width, cursor_y, context, fonts,
                            if first_line { text_indent } else { 0 },
                        );
                        first_line = false;
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

        let span_opacity = span.style.effective_opacity;
        // Note: apply_opacity here blends span colors against context.background_color,
        // which tracks the nearest solid block-level backdrop. For spans inside a block
        // with opacity < 1, the block emits a LayerCommand and effective_opacity is reset
        // to 255 (via compute_style stacking-context rule), so blending is correct.
        // For a bare inline <span style="opacity:0.5"> with no surrounding stacking-context
        // block, effective_opacity accumulates multiplicatively and blending is done against
        // the block-level backdrop — ignoring any inline content painted underneath.
        // This is an intentional approximation (see css.rs nested_inline_opacity test).
        if let Some(background_color) = span.style.background_color {
            let blended_bg = apply_opacity(background_color, context.background_color, span_opacity);
            context.commands.push(DrawCommand::Rect(RectCommand {
                x: cursor_x,
                y: *cursor_y,
                width: span.width,
                height: line_height,
                color: blended_bg,
                border_radius: 0,
            }));
        }

        let display_text = if span.style.text_transform != TextTransform::None {
            apply_text_transform(&span.text, span.style.text_transform)
        } else {
            span.text.clone()
        };
        context.commands.push(DrawCommand::Text(TextCommand {
            x: cursor_x,
            y: *cursor_y,
            width: span.width,
            text: display_text,
            font_size_px: span.style.font_size_px,
            line_height_px: line_height,
            font_family: span.style.font_family,
            color: apply_opacity(span.style.color, context.background_color, span_opacity),
            underline: span.style.underline,
            bold: span.style.font_weight,
            italic: span.style.font_style_italic,
        }));

        if let Some(href) = &span.link_href {
            if !span.style.pointer_events_none {
                context.links.push(LinkCommand {
                    node_id: span.link_node_id,
                    x: cursor_x,
                    y: *cursor_y,
                    width: span.width,
                    height: line_height,
                    href: href.clone(),
                });
            }
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
                display: Display::Block
                    | Display::ListItem
                    | Display::Flex
                    | Display::InlineFlex
                    | Display::Grid
                    | Display::InlineGrid,
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
                        LengthValue::MinContent => 0,
                        LengthValue::MaxContent | LengthValue::FitContent(_) => u32::MAX / 2,
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
        LengthValue::MinContent => 0,
        LengthValue::MaxContent => available_width,
        LengthValue::FitContent(max_px) => available_width.min(max_px),
    }
}

/// Blend `color` with `background` using `opacity` (255 = fully opaque).
fn apply_opacity(color: Color, background: Color, opacity: u8) -> Color {
    if opacity == 255 {
        return color;
    }
    if opacity == 0 {
        return background;
    }
    let a = opacity as u32;
    let blend = |fg: u32, bg: u32| -> u32 { (fg * a + bg * (255 - a) + 127) / 255 };
    let fr = (color >> 16) & 0xFF;
    let fg = (color >> 8) & 0xFF;
    let fb = color & 0xFF;
    let br = (background >> 16) & 0xFF;
    let bg_g = (background >> 8) & 0xFF;
    let bb = background & 0xFF;
    (blend(fr, br) << 16) | (blend(fg, bg_g) << 8) | blend(fb, bb)
}

// find_document_background was removed: we no longer pre-blend body background
// in layout_styled_document (Issue 4 — double compositing fix).

fn layout_positioned_element(
    element: &StyledElement,
    _x: u32,
    container_width: u32,
    _cursor_y: &mut u32,
    context: &mut LayoutContext,
    images: &ImageStore,
    fonts: &mut FontContext,
    current_form: Option<FormContext>,
) {
    let (base_x, base_y) = if element.style.position == Position::Fixed {
        (0u32, context.scroll_y_for_fixed)
    } else {
        context.containing_block_origin
    };

    let elem_width = element.style.width
        .as_ref()
        .and_then(|lv| match lv {
            LengthValue::Pixels(px) => Some(*px),
            LengthValue::Percent(p) => Some((container_width as f32 * (*p as f32) / 100.0) as u32),
            LengthValue::MinContent => Some(0),
            LengthValue::MaxContent => Some(container_width),
            LengthValue::FitContent(max_px) => Some(container_width.min(*max_px)),
        })
        .unwrap_or(container_width);

    let x = (base_x as i64 + element.style.left.unwrap_or(0) as i64).max(0) as u32;
    let mut cursor_y = (base_y as i64 + element.style.top.unwrap_or(0) as i64).max(0) as u32;

    let mut sub_context = LayoutContext {
        background_color: context.background_color,
        next_control_id: context.next_control_id,
        next_form_id: context.next_form_id,
        ..LayoutContext::default()
    };
    // Use sub_context for form allocation so next_form_id counter stays consistent
    // when propagated back — avoids form_id going backwards if element is a <form>
    let current_form = form_context_for_element(element, &mut sub_context, current_form);
    layout_block_element(element, x, elem_width, &mut cursor_y, &mut sub_context, images, fonts, current_form);
    let z = element.style.z_index.unwrap_or(0);
    context.positioned_commands.push((z, sub_context.commands));
    context.links.extend(sub_context.links);
    context.controls.extend(sub_context.controls);
    context.element_hitboxes.extend(sub_context.element_hitboxes);
    context.next_control_id = sub_context.next_control_id;
    context.next_form_id = sub_context.next_form_id;
}

// ─────────────────────────────────────────────────────────────────────────────
// Grid layout
// ─────────────────────────────────────────────────────────────────────────────

fn layout_grid_container(
    element: &StyledElement,
    x: u32,
    available_width: u32,
    cursor_y: &mut u32,
    context: &mut LayoutContext,
    images: &ImageStore,
    fonts: &mut FontContext,
    current_form: Option<FormContext>,
) {
    *cursor_y = cursor_y.saturating_add(element.style.margin.top);
    let outer_x = x.saturating_add(element.style.margin.left);
    let outer_width = available_width
        .saturating_sub(element.style.margin.left + element.style.margin.right);
    let background_top = *cursor_y;

    let border_h = if !element.style.border_style_none {
        element.style.border.top + element.style.border.bottom
    } else {
        0
    };
    let border_v = if !element.style.border_style_none {
        element.style.border.left + element.style.border.right
    } else {
        0
    };
    let content_x = outer_x
        .saturating_add(if !element.style.border_style_none { element.style.border.left } else { 0 })
        .saturating_add(element.style.padding.left);
    let content_width = outer_width
        .saturating_sub(border_v + element.style.padding.left + element.style.padding.right)
        .max(1);

    // ── Resolve column widths ──────────────────────────────────────────────
    let gap = element.style.gap;
    let col_tracks = &element.style.grid_template_columns;
    let col_widths: Vec<u32> = if col_tracks.is_empty() {
        vec![content_width]
    } else {
        resolve_grid_tracks(col_tracks, content_width, gap)
    };
    let n_cols = col_widths.len().max(1);

    // ── Collect grid items ─────────────────────────────────────────────────
    let children: Vec<&StyledElement> = element.children.iter().filter_map(|c| {
        if let StyledNode::Element(el) = c {
            if el.style.display != Display::None { Some(el) } else { None }
        } else {
            None
        }
    }).collect();

    // ── Auto-place items into grid cells ──────────────────────────────────
    let mut col_cursor = 0usize;
    let mut row_cursor = 0usize;
    let mut occupied: std::collections::HashSet<(usize, usize)> = std::collections::HashSet::new();

    struct PlacedItem<'a> {
        element: &'a StyledElement,
        col: usize,
        row: usize,
        col_span: usize,
        row_span: usize,
    }

    let mut placed: Vec<PlacedItem> = Vec::new();

    for child in &children {
        let (col_start, col_span) = {
            let p = &child.style.grid_column;
            let span = p.span.unwrap_or(1) as usize;
            let start = p.start.map(|s| (s - 1).max(0) as usize);
            (start, span)
        };
        let (row_start, row_span) = {
            let p = &child.style.grid_row;
            let span = p.span.unwrap_or(1) as usize;
            let start = p.start.map(|s| (s - 1).max(0) as usize);
            (start, span)
        };

        let (final_col, final_row) = if let Some(c) = col_start {
            let r = row_start.unwrap_or_else(|| {
                let mut r = row_cursor;
                loop {
                    let fits = (c..c + col_span).all(|cc| {
                        (r..r + row_span).all(|rr| !occupied.contains(&(rr, cc)))
                    });
                    if fits { return r; }
                    r += 1;
                }
            });
            (c, r)
        } else {
            let mut c = col_cursor;
            let mut r = row_cursor;
            loop {
                if c + col_span > n_cols {
                    c = 0;
                    r += 1;
                }
                let fits = (c..c + col_span).all(|cc| {
                    (r..r + row_span).all(|rr| !occupied.contains(&(rr, cc)))
                });
                if fits { break; }
                c += 1;
                if c + col_span > n_cols {
                    c = 0;
                    r += 1;
                }
            }
            col_cursor = c + col_span;
            row_cursor = r;
            if col_cursor >= n_cols {
                col_cursor = 0;
                row_cursor += 1;
            }
            (c, r)
        };

        // Mark cells as occupied
        for rr in final_row..final_row + row_span {
            for cc in final_col..final_col + col_span.min(n_cols) {
                occupied.insert((rr, cc));
            }
        }

        placed.push(PlacedItem {
            element: child,
            col: final_col,
            row: final_row,
            col_span: col_span.min(n_cols.saturating_sub(final_col).max(1)),
            row_span,
        });
    }

    // ── Compute row heights ────────────────────────────────────────────────
    let max_row = placed.iter().map(|p| p.row + p.row_span).max().unwrap_or(0);
    let auto_row_tracks = &element.style.grid_template_rows;
    let mut row_heights: Vec<u32> = vec![0u32; max_row];

    // Measure pass
    struct MeasuredItem<'a> {
        element: &'a StyledElement,
        col: usize,
        row: usize,
        col_span: usize,
        row_span: usize,
        measured_height: u32,
        cell_width: u32,
    }
    let mut measured: Vec<MeasuredItem> = Vec::new();

    for item in &placed {
        let cell_width: u32 = {
            let end_col = (item.col + item.col_span).min(n_cols);
            let w: u32 = if end_col > item.col {
                col_widths[item.col..end_col].iter().sum()
            } else {
                col_widths.get(item.col).copied().unwrap_or(0)
            };
            let gaps = gap * (end_col.saturating_sub(item.col).saturating_sub(1)) as u32;
            w + gaps
        };

        // Measure item height via a throwaway context
        let mut dummy_y = 0u32;
        let mut dummy_ctx = LayoutContext {
            background_color: context.background_color,
            next_control_id: context.next_control_id,
            next_form_id: context.next_form_id,
            ..LayoutContext::default()
        };
        layout_block_element(
            item.element,
            0,
            cell_width,
            &mut dummy_y,
            &mut dummy_ctx,
            images,
            fonts,
            None,
        );
        let h = dummy_y;

        if item.row_span == 1 {
            if item.row < row_heights.len() {
                row_heights[item.row] = row_heights[item.row].max(h);
            }
        }

        measured.push(MeasuredItem {
            element: item.element,
            col: item.col,
            row: item.row,
            col_span: item.col_span,
            row_span: item.row_span,
            measured_height: h,
            cell_width,
        });
    }

    // Override with explicit row track sizes
    for (ri, track) in auto_row_tracks.iter().enumerate() {
        if ri < row_heights.len() {
            if let GridTrackSize::Pixels(px) = track {
                row_heights[ri] = *px;
            }
        }
    }
    // Apply grid-auto-rows to rows beyond the explicit template
    for ri in auto_row_tracks.len()..max_row {
        if ri < row_heights.len() {
            if let GridTrackSize::Pixels(px) = &element.style.grid_auto_rows {
                row_heights[ri] = row_heights[ri].max(*px);
            }
        }
    }

    // Background placeholder
    let bg_cmd_index = if let Some(bg) = element.style.background_color {
        let blended = apply_opacity(bg, context.background_color, element.style.effective_opacity);
        context.commands.push(DrawCommand::Rect(RectCommand {
            x: outer_x,
            y: background_top,
            width: outer_width.max(1),
            height: 1,
            color: blended,
            border_radius: element.style.border_radius,
        }));
        Some(context.commands.len() - 1)
    } else {
        None
    };

    let content_top = background_top
        .saturating_add(if !element.style.border_style_none { element.style.border.top } else { 0 })
        .saturating_add(element.style.padding.top);

    // ── Render items ──────────────────────────────────────────────────────
    for item in &measured {
        let cell_x: u32 = {
            let x_offset: u32 = col_widths[..item.col].iter().sum::<u32>()
                + gap * item.col as u32;
            content_x + x_offset
        };
        let cell_y: u32 = {
            let y_offset: u32 = row_heights[..item.row].iter().sum::<u32>()
                + gap * item.row as u32;
            content_top + y_offset
        };

        let mut item_y = cell_y;
        let item_form = form_context_for_element(item.element, context, current_form.clone());
        layout_block_element(
            item.element,
            cell_x,
            item.cell_width,
            &mut item_y,
            context,
            images,
            fonts,
            item_form,
        );
    }

    // Total content height
    let total_h: u32 = row_heights.iter().sum::<u32>()
        + gap * max_row.saturating_sub(1) as u32;
    let content_bottom = content_top + total_h;
    let background_bottom = content_bottom
        .saturating_add(element.style.padding.bottom)
        .saturating_add(if !element.style.border_style_none { element.style.border.bottom } else { 0 });

    // Fix background rect height
    if let Some(idx) = bg_cmd_index {
        if let DrawCommand::Rect(r) = &mut context.commands[idx] {
            r.height = (background_bottom - background_top).max(1);
        }
    }

    // Draw border
    if !element.style.border_style_none {
        let bc = apply_opacity(
            element.style.border_color,
            context.background_color,
            element.style.effective_opacity,
        );
        let background_height = background_bottom.saturating_sub(background_top).max(1);
        let border_top_h = if border_h > 0 { element.style.border.top } else { 0 };
        let border_bottom_h = if border_h > 0 { element.style.border.bottom } else { 0 };
        let border_left_w = if border_v > 0 { element.style.border.left } else { 0 };
        let border_right_w = if border_v > 0 { element.style.border.right } else { 0 };
        if border_top_h > 0 {
            context.commands.push(DrawCommand::Rect(RectCommand {
                x: outer_x,
                y: background_top,
                width: outer_width.max(1),
                height: border_top_h,
                color: bc,
                border_radius: element.style.border_radius,
            }));
        }
        if border_bottom_h > 0 {
            context.commands.push(DrawCommand::Rect(RectCommand {
                x: outer_x,
                y: background_bottom.saturating_sub(border_bottom_h),
                width: outer_width.max(1),
                height: border_bottom_h,
                color: bc,
                border_radius: element.style.border_radius,
            }));
        }
        if border_left_w > 0 {
            context.commands.push(DrawCommand::Rect(RectCommand {
                x: outer_x,
                y: background_top,
                width: border_left_w,
                height: background_height,
                color: bc,
                border_radius: 0,
            }));
        }
        if border_right_w > 0 {
            context.commands.push(DrawCommand::Rect(RectCommand {
                x: outer_x.saturating_add(outer_width).saturating_sub(border_right_w),
                y: background_top,
                width: border_right_w,
                height: background_height,
                color: bc,
                border_radius: 0,
            }));
        }
    }

    *cursor_y = background_bottom + element.style.margin.bottom;
}

/// Resolve grid track sizes into pixel widths, distributing fr units.
fn resolve_grid_tracks(tracks: &[GridTrackSize], available_px: u32, gap: u32) -> Vec<u32> {
    let n = tracks.len();
    let total_gap = gap * n.saturating_sub(1) as u32;
    let remaining_after_gap = available_px.saturating_sub(total_gap);

    let mut widths = vec![0u32; n];
    let mut fixed_total = 0u32;
    let mut fr_total = 0u32;
    let mut auto_count = 0u32;

    for (i, track) in tracks.iter().enumerate() {
        match track {
            GridTrackSize::Pixels(px) => {
                widths[i] = *px;
                fixed_total += px;
            }
            GridTrackSize::Percent(pct_x100) => {
                let px = (remaining_after_gap as u64 * *pct_x100 as u64 / 10000) as u32;
                widths[i] = px;
                fixed_total += px;
            }
            GridTrackSize::Fr(fr_x1000) => {
                fr_total += fr_x1000;
            }
            GridTrackSize::Auto | GridTrackSize::MinContent | GridTrackSize::MaxContent => {
                auto_count += 1;
            }
        }
    }

    let remaining = remaining_after_gap.saturating_sub(fixed_total);
    let fr_space = if auto_count == 0 { remaining } else { remaining * 2 / 3 };
    let auto_space = if auto_count > 0 { remaining - fr_space } else { 0 };

    if fr_total > 0 {
        for (i, track) in tracks.iter().enumerate() {
            if let GridTrackSize::Fr(fr_x1000) = track {
                widths[i] = (fr_space as u64 * *fr_x1000 as u64 / fr_total as u64) as u32;
            }
        }
    }
    if auto_count > 0 {
        let per_auto = auto_space / auto_count;
        for (i, track) in tracks.iter().enumerate() {
            if matches!(
                track,
                GridTrackSize::Auto | GridTrackSize::MinContent | GridTrackSize::MaxContent
            ) {
                widths[i] = per_auto;
            }
        }
    }

    widths
}

fn layout_flex_container(
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
    let outer_x = x.saturating_add(element.style.margin.left);
    let outer_width = width.saturating_sub(
        element.style.margin.left + element.style.margin.right
    );
    let background_top = *cursor_y;

    let border_left = if !element.style.border_style_none { element.style.border.left } else { 0 };
    let border_right = if !element.style.border_style_none { element.style.border.right } else { 0 };
    let border_top = if !element.style.border_style_none { element.style.border.top } else { 0 };
    let border_bottom_sz = if !element.style.border_style_none { element.style.border.bottom } else { 0 };

    let content_x = outer_x
        .saturating_add(border_left)
        .saturating_add(element.style.padding.left);
    let content_width = outer_width
        .saturating_sub(border_left + border_right + element.style.padding.left + element.style.padding.right)
        .max(1);
    let content_y = background_top
        .saturating_add(border_top)
        .saturating_add(element.style.padding.top);

    let gap = element.style.gap;
    let is_row = matches!(element.style.flex_direction, FlexDirection::Row | FlexDirection::RowReverse);

    // Collect visible flex items (only element children, not text nodes)
    let children: Vec<&StyledElement> = element.children.iter().filter_map(|child| {
        if let StyledNode::Element(el) = child {
            if el.style.display != Display::None { Some(el) } else { None }
        } else { None }
    }).collect();

    // Reserve a slot for background rect — insert placeholder now, update height later
    let bg_cmd_index = if let Some(background_color) = element.style.background_color {
        let blended = apply_opacity(background_color, context.background_color, element.style.effective_opacity);
        context.commands.push(DrawCommand::Rect(RectCommand {
            x: outer_x, y: background_top,
            width: outer_width.max(1), height: 1,
            color: blended,
            border_radius: element.style.border_radius,
        }));
        Some(context.commands.len() - 1)
    } else {
        None
    };

    let saved_bg = context.background_color;
    if let Some(bg) = element.style.background_color {
        if element.style.effective_opacity == 255 {
            context.background_color = bg;
        }
    }

    if !children.is_empty() {
        if is_row {
            let n = children.len();
            let total_gap = gap.saturating_mul((n.saturating_sub(1)) as u32);

            // Calculate total width of children with explicit widths (+ margins)
            let total_fixed: u32 = children.iter().map(|child| {
                child.style.width.as_ref().and_then(|lv| match lv {
                    LengthValue::Pixels(px) => Some(*px),
                    LengthValue::Percent(p) => Some((content_width as f32 * (*p as f32) / 100.0) as u32),
                    LengthValue::MinContent => Some(0),
                    LengthValue::MaxContent => Some(content_width),
                    LengthValue::FitContent(max_px) => Some(content_width.min(*max_px)),
                }).unwrap_or(0)
                + child.style.margin.left + child.style.margin.right
            }).sum();
            let n_auto = children.iter().filter(|child| child.style.width.is_none()).count() as u32;
            let remaining = content_width.saturating_sub(total_fixed).saturating_sub(total_gap);
            let auto_width = if n_auto > 0 { remaining / n_auto } else { 0 };

            // First pass: measure heights for alignment
            let item_heights: Vec<u32> = children.iter().map(|child| {
                let child_w = child.style.width.as_ref().and_then(|lv| match lv {
                    LengthValue::Pixels(px) => Some(*px),
                    LengthValue::Percent(p) => Some((content_width as f32 * (*p as f32) / 100.0) as u32),
                    LengthValue::MinContent => Some(0),
                    LengthValue::MaxContent => Some(content_width),
                    LengthValue::FitContent(max_px) => Some(content_width.min(*max_px)),
                }).unwrap_or(auto_width).max(1);
                let mut dummy_y = content_y;
                let mut dummy_ctx = LayoutContext { background_color: context.background_color, ..LayoutContext::default() };
                layout_block_element(child, content_x, child_w, &mut dummy_y, &mut dummy_ctx, images, fonts, current_form.clone());
                dummy_y.saturating_sub(content_y)
            }).collect();
            let max_height = *item_heights.iter().max().unwrap_or(&0);

            // Compute justify-content offsets
            let (start_offset, item_gap) = justify_content_offsets(
                element.style.justify_content, content_width, total_fixed, total_gap, n as u32
            );

            // Second pass: actual layout
            let mut cursor_x = content_x.saturating_add(start_offset);
            for (i, child) in children.iter().enumerate() {
                let child_w = child.style.width.as_ref().and_then(|lv| match lv {
                    LengthValue::Pixels(px) => Some(*px),
                    LengthValue::Percent(p) => Some((content_width as f32 * (*p as f32) / 100.0) as u32),
                    LengthValue::MinContent => Some(0),
                    LengthValue::MaxContent => Some(content_width),
                    LengthValue::FitContent(max_px) => Some(content_width.min(*max_px)),
                }).unwrap_or(auto_width).max(1);

                let self_align = match child.style.align_self {
                    AlignSelf::Auto => element.style.align_items,
                    AlignSelf::FlexStart => AlignItems::FlexStart,
                    AlignSelf::FlexEnd => AlignItems::FlexEnd,
                    AlignSelf::Center => AlignItems::Center,
                    AlignSelf::Stretch => AlignItems::Stretch,
                    AlignSelf::Baseline => AlignItems::Baseline,
                };
                let child_y_offset = match self_align {
                    AlignItems::Center => (max_height.saturating_sub(item_heights[i])) / 2,
                    AlignItems::FlexEnd => max_height.saturating_sub(item_heights[i]),
                    _ => 0,
                };

                let mut child_y = content_y.saturating_add(child_y_offset);
                let child_form = form_context_for_element(child, context, current_form.clone());
                layout_block_element(child, cursor_x, child_w, &mut child_y, context, images, fonts, child_form);
                cursor_x = cursor_x.saturating_add(child_w).saturating_add(item_gap);
            }

            *cursor_y = content_y.saturating_add(max_height)
                .saturating_add(element.style.padding.bottom)
                .saturating_add(border_bottom_sz);
        } else {
            // Column direction: stack children vertically with gap
            *cursor_y = content_y;
            for (i, child) in children.iter().enumerate() {
                let child_form = form_context_for_element(child, context, current_form.clone());
                layout_block_element(child, content_x, content_width, cursor_y, context, images, fonts, child_form);
                if i < children.len() - 1 {
                    *cursor_y = cursor_y.saturating_add(gap);
                }
            }
            *cursor_y = cursor_y.saturating_add(element.style.padding.bottom)
                .saturating_add(border_bottom_sz);
        }
    } else {
        *cursor_y = content_y.saturating_add(element.style.padding.bottom).saturating_add(border_bottom_sz);
    }

    // Update background rect height
    let background_height = cursor_y.saturating_sub(background_top).max(1);
    if let Some(idx) = bg_cmd_index {
        if let Some(DrawCommand::Rect(rect)) = context.commands.get_mut(idx) {
            rect.height = background_height;
        }
    }

    context.background_color = saved_bg;

    // Draw borders
    if !element.style.border_style_none {
        let bc = apply_opacity(element.style.border_color, context.background_color, element.style.effective_opacity);
        if border_top > 0 {
            context.commands.push(DrawCommand::Rect(RectCommand {
                x: outer_x, y: background_top,
                width: outer_width.max(1), height: border_top,
                color: bc, border_radius: element.style.border_radius,
            }));
        }
        if border_bottom_sz > 0 {
            context.commands.push(DrawCommand::Rect(RectCommand {
                x: outer_x, y: cursor_y.saturating_sub(border_bottom_sz),
                width: outer_width.max(1), height: border_bottom_sz,
                color: bc, border_radius: element.style.border_radius,
            }));
        }
        if border_left > 0 {
            context.commands.push(DrawCommand::Rect(RectCommand {
                x: outer_x, y: background_top,
                width: border_left, height: background_height,
                color: bc, border_radius: 0,
            }));
        }
        if border_right > 0 {
            context.commands.push(DrawCommand::Rect(RectCommand {
                x: outer_x.saturating_add(outer_width).saturating_sub(border_right),
                y: background_top,
                width: border_right, height: background_height,
                color: bc, border_radius: 0,
            }));
        }
    }

    *cursor_y = cursor_y.saturating_add(element.style.margin.bottom);
}

fn justify_content_offsets(
    justify: JustifyContent,
    container_w: u32,
    total_fixed: u32,
    total_gap: u32,
    n: u32,
) -> (u32, u32) {
    // Returns (start_offset, gap_between_items)
    let free = container_w.saturating_sub(total_fixed).saturating_sub(total_gap);
    let base_gap = if n > 1 { total_gap / (n - 1) } else { 0 };
    match justify {
        JustifyContent::FlexStart => (0, base_gap),
        JustifyContent::FlexEnd => (free, base_gap),
        JustifyContent::Center => (free / 2, base_gap),
        JustifyContent::SpaceBetween => (0, if n > 1 { (free + total_gap) / (n - 1) } else { 0 }),
        JustifyContent::SpaceAround => {
            let per = free / n.max(1);
            (per / 2, per + base_gap)
        }
        JustifyContent::SpaceEvenly => {
            let per = free / (n + 1).max(1);
            (per, per + base_gap)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DrawCommand, layout_styled_document};
    use crate::css::{TextAlign, build_styled_tree, parse_stylesheet};
    use crate::font::FontContext;
    use crate::html::parse_document;
    use crate::image::{DecodedImage, ImageStore};

    #[test]
    fn hides_display_none_content() {
        let document = parse_document("<div><p>Hello</p><span class=\"hide\">Nope</span></div>");
        let stylesheet = parse_stylesheet(".hide { display: none; } p { color: #ff0000; }");
        let styled = build_styled_tree(&document, &stylesheet, 1280, &crate::css::InteractiveState::default());
        let mut fonts = FontContext::load();
        let layout = layout_styled_document(&styled, &ImageStore::default(), 320, &mut fonts);

        let texts = layout.texts();
        assert!(texts.iter().any(|text| text.text.contains("Hello")));
        assert!(texts.iter().all(|text| !text.text.contains("Nope")));
        assert!(texts.iter().any(|text| text.color == 0xFF0000));
    }

    #[test]
    fn centers_text_when_requested() {
        let document = parse_document("<p>Hello</p>");
        let stylesheet = parse_stylesheet("p { text-align: center; font-size: 16px; }");
        let styled = build_styled_tree(&document, &stylesheet, 1280, &crate::css::InteractiveState::default());
        let mut fonts = FontContext::load();
        let layout = layout_styled_document(&styled, &ImageStore::default(), 200, &mut fonts);

        let texts = layout.texts();
        let text = texts.first().expect("text command should exist");
        let expected_left_offset = (200 - text.width) / 2;

        assert_eq!(text.x, expected_left_offset);
    }

    #[test]
    fn wraps_text_across_multiple_lines() {
        let document = parse_document("<p>alpha beta gamma delta epsilon</p>");
        let stylesheet = parse_stylesheet("p { font-size: 16px; }");
        let styled = build_styled_tree(&document, &stylesheet, 1280, &crate::css::InteractiveState::default());
        let mut fonts = FontContext::load();
        let layout = layout_styled_document(&styled, &ImageStore::default(), 90, &mut fonts);

        let distinct_rows = layout
            .texts()
            .into_iter()
            .map(|text| text.y)
            .collect::<std::collections::BTreeSet<_>>();

        assert!(distinct_rows.len() >= 2);
    }

    #[test]
    fn keeps_text_align_inherited() {
        let document = parse_document("<div><p>Hello</p></div>");
        let stylesheet = parse_stylesheet("div { text-align: right; }");
        let styled = build_styled_tree(&document, &stylesheet, 1280, &crate::css::InteractiveState::default());

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
        let styled = build_styled_tree(&document, &parse_stylesheet(""), 1280, &crate::css::InteractiveState::default());
        let mut fonts = FontContext::load();
        let layout = layout_styled_document(&styled, &ImageStore::default(), 320, &mut fonts);
        let texts = layout.texts();
        let left = texts
            .iter()
            .find(|text| text.text.contains("Left"))
            .expect("left cell text should exist");
        let right = texts
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
        let styled = build_styled_tree(&document, &parse_stylesheet(""), 1280, &crate::css::InteractiveState::default());
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

        let images_list = layout.images();
        assert_eq!(images_list.len(), 1);
        assert_eq!(images_list[0].width, 40);
        assert_eq!(images_list[0].height, 20);
    }

    #[test]
    fn auto_width_tables_do_not_expand_to_full_container() {
        let document =
            parse_document("<table align=\"center\"><tr><td>Hello</td><td>World</td></tr></table>");
        let styled = build_styled_tree(&document, &parse_stylesheet(""), 1280, &crate::css::InteractiveState::default());
        let mut fonts = FontContext::load();
        let layout = layout_styled_document(&styled, &ImageStore::default(), 500, &mut fonts);
        let texts = layout.texts();
        let hello = texts
            .iter()
            .find(|text| text.text.contains("Hello"))
            .expect("hello text should exist");
        let world = texts
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
        let styled = build_styled_tree(&document, &parse_stylesheet(""), 1280, &crate::css::InteractiveState::default());
        let mut fonts = FontContext::load();
        let layout = layout_styled_document(&styled, &ImageStore::default(), 320, &mut fonts);
        let texts = layout.texts();
        let short = texts
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
        let styled = build_styled_tree(&document, &parse_stylesheet(""), 1280, &crate::css::InteractiveState::default());
        let mut fonts = FontContext::load();
        let layout = layout_styled_document(&styled, &ImageStore::default(), 320, &mut fonts);
        let texts = layout.texts();
        let top = texts
            .iter()
            .find(|text| text.text.contains("Top"))
            .expect("top cell text should exist");
        let bottom = texts
            .iter()
            .find(|text| text.text.contains("Bottom"))
            .expect("bottom cell text should exist");

        assert!(bottom.y > top.y);
        assert_eq!(top.x, bottom.x);
    }

    #[test]
    fn uses_document_background_for_opacity_blending() {
        let document = parse_document("<body><div>Hi</div></body>");
        let stylesheet =
            parse_stylesheet("body { background-color: #000000; } div { background-color: #ff0000; opacity: 0.5; }");
        let styled = build_styled_tree(&document, &stylesheet, 1280, &crate::css::InteractiveState::default());
        let mut fonts = FontContext::load();
        let layout = layout_styled_document(&styled, &ImageStore::default(), 320, &mut fonts);

        // With stacking contexts, the div with opacity: 0.5 becomes a LayerCommand.
        // Its background rect inside the layer uses the raw red (#ff0000), not a pre-blended value.
        // The compositor blends it at render time.
        let has_layer = layout.commands.iter().any(|cmd| {
            matches!(cmd, DrawCommand::Layer(layer) if layer.opacity == 128 || layer.opacity == 127)
        });
        assert!(
            has_layer,
            "div with opacity: 0.5 should produce a LayerCommand with ~50% opacity"
        );
        // The raw red rect should be inside the layer
        let has_raw_red = layout.commands.iter().any(|cmd| {
            if let DrawCommand::Layer(layer) = cmd {
                layer.commands.iter().any(|inner| {
                    matches!(inner, DrawCommand::Rect(r) if r.color == 0xFF0000)
                })
            } else {
                false
            }
        });
        assert!(
            has_raw_red,
            "raw red rect should be inside the LayerCommand"
        );
    }

    #[test]
    fn accumulates_parent_opacity_for_text() {
        let document = parse_document("<body><div><span>Hi</span></div></body>");
        let stylesheet = parse_stylesheet(
            "body { background-color: #000000; } div { opacity: 0.5; } span { opacity: 0.5; color: #ffffff; }",
        );
        let styled = build_styled_tree(&document, &stylesheet, 1280, &crate::css::InteractiveState::default());
        let mut fonts = FontContext::load();
        let layout = layout_styled_document(&styled, &ImageStore::default(), 320, &mut fonts);

        // With proper stacking contexts, div creates a LayerCommand with its own opacity.
        // The span inside has its own effective_opacity (reset at stacking context boundary).
        // The text color inside the layer is pre-blended with the span's own opacity (0.5)
        // against the layer's local backdrop (black #000000 from body background).
        // span.opacity=0.5=128, color=white=#ffffff blended against black => ~0x808080
        let has_layer = layout.commands.iter().any(|cmd| {
            matches!(cmd, DrawCommand::Layer(_))
        });
        assert!(has_layer, "div with opacity: 0.5 should produce a LayerCommand");

        // Text color inside layer should be pre-blended with span's own opacity against the
        // layer's local backdrop color. The layer's backdrop is black (body bg).
        // span effective_opacity = 128 (its own opacity, reset at stacking context boundary)
        // color = apply_opacity(0xFFFFFF, 0x000000, 128) = ~0x808080
        let texts = layout.texts();
        let text = texts.first().expect("text command should exist");
        // The text should be blended with span's 50% opacity against the layer backdrop (black)
        assert_eq!(text.color, 0x808080,
            "text inside stacking context should be pre-blended with span's own opacity against layer backdrop");
    }

    #[test]
    fn emits_form_controls_for_inputs_and_buttons() {
        let document = parse_document(
            r#"<form action="/search"><input name="q" value="rust"><button type="submit">Go</button></form>"#,
        );
        let styled = build_styled_tree(&document, &parse_stylesheet(""), 1280, &crate::css::InteractiveState::default());
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
    #[test]
    fn test_overflow_hidden_clips_commands() {
        use crate::css::{parse_stylesheet, build_styled_tree};
        use crate::html::parse_document;
        use crate::font::FontContext;
        use crate::image::ImageStore;

        let html = r#"<div style="overflow:hidden;height:50px;background:#ffffff"><div style="height:100px;background:#ff0000">Content</div></div>"#;
        let doc = parse_document(html);
        let stylesheet = parse_stylesheet("");
        let styled = build_styled_tree(&doc, &stylesheet, 800, &crate::css::InteractiveState::default());
        let mut fonts = FontContext::load();
        let images = ImageStore::default();
        let layout = layout_styled_document(&styled, &images, 800, &mut fonts);

        // The outer div is at y=8 (body margin), height=50, so max_y=58
        let div_top = 8u32;
        let max_y = div_top + 50;
        for rect in layout.rects() {
            if rect.y >= div_top && rect.y < max_y {
                assert!(
                    rect.y.saturating_add(rect.height) <= max_y + 2,
                    "Rect y={} height={} exceeds overflow:hidden boundary y={}",
                    rect.y, rect.height, max_y
                );
            }
        }
    }
    #[test]
    fn test_border_radius_in_rect_command() {
        use crate::css::{parse_stylesheet, build_styled_tree};
        use crate::html::parse_document;

        let html = r#"<div style="background:#ff0000;border-radius:10px">Hello</div>"#;
        let doc = parse_document(html);
        let stylesheet = parse_stylesheet("");
        let styled = build_styled_tree(&doc, &stylesheet, 800, &crate::css::InteractiveState::default());
        let mut fonts = FontContext::load();
        let images = ImageStore::default();
        let layout = layout_styled_document(&styled, &images, 800, &mut fonts);

        let rects = layout.rects();
        let bg_rect = rects.iter().find(|r| r.border_radius == 10);
        assert!(bg_rect.is_some(), "Should have a rect with border_radius=10");
        assert_eq!(bg_rect.unwrap().border_radius, 10);
    }
    #[test]
    fn test_box_shadow_generates_shadow_rect() {
        use crate::css::{parse_stylesheet, build_styled_tree};
        use crate::html::parse_document;

        let html = r#"<div style="background:#ffffff;box-shadow:2px 2px #000000">Hello</div>"#;
        let doc = parse_document(html);
        let stylesheet = parse_stylesheet("");
        let styled = build_styled_tree(&doc, &stylesheet, 800, &crate::css::InteractiveState::default());
        let mut fonts = FontContext::load();
        let images = ImageStore::default();
        let layout = layout_styled_document(&styled, &images, 800, &mut fonts);

        // Should have a black shadow rect
        let rects = layout.rects();
        let shadow_rect = rects.iter().find(|r| r.color == 0x000000);
        assert!(shadow_rect.is_some(), "Should have a shadow rect with black color");
    }

    #[test]
    fn grid_children_placed_side_by_side() {
        use crate::css::{parse_stylesheet, build_styled_tree};
        use crate::html::parse_document;

        // 2-column grid: two children should be placed side by side (different x values)
        let html = r#"<div style="display:grid;grid-template-columns:200px 200px;gap:0px;"><div>Left</div><div>Right</div></div>"#;
        let doc = parse_document(html);
        let stylesheet = parse_stylesheet("");
        let styled = build_styled_tree(&doc, &stylesheet, 800, &crate::css::InteractiveState::default());
        let mut fonts = FontContext::load();
        let images = ImageStore::default();
        let layout = layout_styled_document(&styled, &images, 800, &mut fonts);

        let texts = layout.texts();
        let left = texts.iter().find(|t| t.text.contains("Left")).expect("Left text should be rendered");
        let right = texts.iter().find(|t| t.text.contains("Right")).expect("Right text should be rendered");

        // Left and Right should have different x positions (side by side)
        assert_ne!(left.x, right.x, "Grid children should be placed at different x positions");
        // Right should be to the right of left
        assert!(right.x > left.x, "Right item should have a larger x than Left item");
        // They should be on the same row (same y)
        assert_eq!(left.y, right.y, "Grid children in the same row should have the same y");
    }

    #[test]
    fn grid_three_column_equal_fr_layout() {
        use crate::css::{parse_stylesheet, build_styled_tree};
        use crate::html::parse_document;

        let html = r#"<div style="display:grid;grid-template-columns:repeat(3,1fr);"><div>A</div><div>B</div><div>C</div></div>"#;
        let doc = parse_document(html);
        let stylesheet = parse_stylesheet("");
        let styled = build_styled_tree(&doc, &stylesheet, 600, &crate::css::InteractiveState::default());
        let mut fonts = FontContext::load();
        let images = ImageStore::default();
        let layout = layout_styled_document(&styled, &images, 600, &mut fonts);

        let texts = layout.texts();
        let a = texts.iter().find(|t| t.text.contains('A')).expect("A should be rendered");
        let b = texts.iter().find(|t| t.text.contains('B')).expect("B should be rendered");
        let c = texts.iter().find(|t| t.text.contains('C')).expect("C should be rendered");

        // All three should be on the same row
        assert_eq!(a.y, b.y, "A and B should be on the same row");
        assert_eq!(b.y, c.y, "B and C should be on the same row");
        // They should be at different x positions
        assert!(b.x > a.x, "B should be to the right of A");
        assert!(c.x > b.x, "C should be to the right of B");
    }

    #[test]
    fn filter_blur_emits_layer_command_with_blur_px() {
        use super::LayerCommand;

        let document = parse_document(r#"<div style="filter: blur(4px);">Hello</div>"#);
        let stylesheet = parse_stylesheet("");
        let styled = build_styled_tree(&document, &stylesheet, 1280, &crate::css::InteractiveState::default());
        let mut fonts = FontContext::load();
        let images = ImageStore::default();
        let layout = layout_styled_document(&styled, &images, 320, &mut fonts);

        // Find all LayerCommands recursively
        fn find_layers(cmds: &[DrawCommand]) -> Vec<&LayerCommand> {
            let mut result = Vec::new();
            for cmd in cmds {
                if let DrawCommand::Layer(layer) = cmd {
                    result.push(layer);
                    result.extend(find_layers(&layer.commands));
                }
            }
            result
        }

        let layers = find_layers(&layout.commands);
        assert!(!layers.is_empty(), "Expected at least one LayerCommand for filter: blur()");
        assert!(
            layers.iter().any(|l| l.blur_px > 0),
            "Expected a LayerCommand with blur_px > 0, got: {:?}",
            layers.iter().map(|l| l.blur_px).collect::<Vec<_>>()
        );
    }

    #[test]
    fn filter_brightness_emits_layer_command_with_brightness() {
        use super::LayerCommand;

        let document = parse_document(r#"<div style="filter: brightness(0.5);">Hello</div>"#);
        let stylesheet = parse_stylesheet("");
        let styled = build_styled_tree(&document, &stylesheet, 1280, &crate::css::InteractiveState::default());
        let mut fonts = FontContext::load();
        let images = ImageStore::default();
        let layout = layout_styled_document(&styled, &images, 320, &mut fonts);

        fn find_layers(cmds: &[DrawCommand]) -> Vec<&LayerCommand> {
            let mut result = Vec::new();
            for cmd in cmds {
                if let DrawCommand::Layer(layer) = cmd {
                    result.push(layer);
                    result.extend(find_layers(&layer.commands));
                }
            }
            result
        }

        let layers = find_layers(&layout.commands);
        assert!(!layers.is_empty(), "Expected at least one LayerCommand for filter: brightness()");
        assert!(
            layers.iter().any(|l| l.brightness != 10000),
            "Expected a LayerCommand with brightness != 10000, got: {:?}",
            layers.iter().map(|l| l.brightness).collect::<Vec<_>>()
        );
        // brightness(0.5) => 5000
        assert!(
            layers.iter().any(|l| l.brightness == 5000),
            "Expected brightness = 5000 (50%), got: {:?}",
            layers.iter().map(|l| l.brightness).collect::<Vec<_>>()
        );
    }
}