use crate::css::{
    Color, ComputedStyle, DEFAULT_BACKGROUND_COLOR, Display, FontFamilyKind, LengthValue,
    Overflow, StyledElement, StyledNode, TextAlign, TextTransform, VerticalAlign, WhiteSpaceMode,
    apply_text_transform,
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
    pub commands: Vec<DrawCommand>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutDocument {
    pub background_color: Color,
    pub content_height: u32,
    pub commands: Vec<DrawCommand>,
    pub links: Vec<LinkCommand>,
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
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub href: String,
}

pub fn layout_styled_document(
    document: &StyledNode,
    images: &ImageStore,
    viewport_width: u32,
    fonts: &mut FontContext,
) -> LayoutDocument {
    // Do NOT pre-blend the body background colour here.
    // When body has an opacity < 1, layout_block_element_as_layer wraps the body in a
    // LayerCommand which composites it at render time.  Pre-blending here AND compositing
    // in render_layer would double-apply the opacity (Issue 4).
    // canvas_bg stays as the default so the body's LayerCommand is the sole source of truth.
    let canvas_bg = DEFAULT_BACKGROUND_COLOR;
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
    );

    LayoutDocument {
        background_color: canvas_bg,
        content_height: cursor_y,
        commands: context.commands,
        links: context.links,
    }
}

#[derive(Default)]
struct LayoutContext {
    background_color: Color,
    commands: Vec<DrawCommand>,
    links: Vec<LinkCommand>,
}

#[derive(Debug, Clone)]
enum InlineFragment {
    Text {
        text: String,
        style: ComputedStyle,
        link_href: Option<String>,
    },
    LineBreak,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LineSpan {
    text: String,
    width: u32,
    style: ComputedStyle,
    link_href: Option<String>,
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

    fn push_span(
        &mut self,
        text: &str,
        style: &ComputedStyle,
        fonts: &mut FontContext,
        link_href: Option<&str>,
    ) {
        if text.is_empty() {
            return;
        }

        let width = text_width(style, text, fonts);
        self.width = self.width.saturating_add(width);
        self.line_height = self.line_height.max(text_line_height(style, fonts));

        if let Some(last) = self.spans.last_mut() {
            if last.style == *style && last.link_href.as_deref() == link_href {
                last.text.push_str(text);
                last.width = last.width.saturating_add(width);
                return;
            }
        }

        self.spans.push(LineSpan {
            text: text.to_string(),
            width,
            style: style.clone(),
            link_href: link_href.map(str::to_string),
        });
    }
}

fn layout_node(
    node: &StyledNode,
    x: u32,
    width: u32,
    cursor_y: &mut u32,
    context: &mut LayoutContext,
    images: &ImageStore,
    fonts: &mut FontContext,
) {
    match node {
        StyledNode::Text(text) => {
            let fragments = [InlineFragment::Text {
                text: text.text.clone(),
                style: text.style.clone(),
                link_href: None,
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
                    let link_href = if element.tag_name == "a" {
                        element.attributes.get("href").cloned()
                    } else {
                        None
                    };
                    let y_before = *cursor_y;
                    layout_block_element(element, x, width, cursor_y, context, images, fonts);
                    if let Some(href) = link_href {
                        let link_height = cursor_y.saturating_sub(y_before);
                        if link_height > 0 {
                            context.links.push(LinkCommand {
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
) {
    if element.tag_name == "br" {
        *cursor_y = cursor_y.saturating_add(text_line_height(&element.style, fonts));
        return;
    }

    if element.tag_name == "table" {
        if element.style.opacity < 255 {
            // Table with opacity: render into sub-context and wrap in a LayerCommand
            let mut sub_context = LayoutContext {
                background_color: context.background_color,
                ..LayoutContext::default()
            };
            let y_before = *cursor_y;
            layout_table_element(element, x, width, cursor_y, &mut sub_context, images, fonts);
            let table_height = cursor_y.saturating_sub(y_before).max(1);
            rebase_commands(&mut sub_context.commands, x, y_before);
            context.commands.push(DrawCommand::Layer(LayerCommand {
                x,
                y: y_before,
                width: width.max(1),
                height: table_height,
                opacity: element.style.opacity,
                commands: sub_context.commands,
            }));
            context.links.extend(sub_context.links);
        } else {
            layout_table_element(element, x, width, cursor_y, context, images, fonts);
        }
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

    // Detect stacking context: element has opacity < 255
    if element.style.opacity < 255 {
        layout_block_element_as_layer(
            element, outer_x, outer_width, background_top, cursor_y, context, images, fonts,
        );
        *cursor_y = cursor_y.saturating_add(element.style.margin.bottom);
        return;
    }

    let saved_bg = context.background_color;

    // box-shadow: push shadow rect before background (so it renders behind it)
    let shadow_cmd_index = if let Some(ref shadow) = element.style.box_shadow {
        let sx = (outer_x as i64 + shadow.offset_x as i64).max(0) as u32;
        let sy = (background_top as i64 + shadow.offset_y as i64).max(0) as u32;
        context.commands.push(DrawCommand::Rect(RectCommand {
            x: sx,
            y: sy,
            width: outer_width.max(1),
            height: 1,
            color: shadow.color,
            border_radius: element.style.border_radius,
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
        );
    }

    *cursor_y = cursor_y.saturating_add(element.style.padding.bottom);
    let background_height = cursor_y.saturating_sub(background_top).max(1);

    if let Some(shadow_idx) = shadow_cmd_index {
        if let Some(DrawCommand::Rect(rect)) = context.commands.get_mut(shadow_idx) {
            rect.height = background_height;
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
    if element.style.overflow == Overflow::Hidden {
        let start_clip_idx = background_cmd_index
            .map(|i| i + 1)
            .unwrap_or(context.commands.len());
        clip_commands_to_box(
            &mut context.commands,
            start_clip_idx,
            outer_x,
            background_top,
            outer_width,
            background_height,
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
                border_radius: 0,
            }));
        }
        if border_bottom_h > 0 {
            context.commands.push(DrawCommand::Rect(RectCommand {
                x: outer_x,
                y: cursor_y.saturating_sub(border_bottom_h),
                width: outer_width.max(1),
                height: border_bottom_h,
                color: bc,
                border_radius: 0,
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

    *cursor_y = cursor_y.saturating_add(element.style.margin.bottom);
}

/// Remove draw commands (from index start) that fall entirely outside the given box.
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

    let mut i = start;
    while i < commands.len() {
        let outside = match &commands[i] {
            DrawCommand::Rect(r) => {
                r.x >= clip_x2
                    || r.y >= clip_y2
                    || r.x.saturating_add(r.width) <= clip_x
                    || r.y.saturating_add(r.height) <= clip_y
            }
            DrawCommand::Text(t) => {
                t.y >= clip_y2
                    || t.y.saturating_add(t.font_size_px) <= clip_y
                    || t.x >= clip_x2
                    || t.x.saturating_add(t.width) <= clip_x
            }
            DrawCommand::Image(img) => {
                img.x >= clip_x2
                    || img.y >= clip_y2
                    || img.x.saturating_add(img.width) <= clip_x
                    || img.y.saturating_add(img.height) <= clip_y
            }
            DrawCommand::Layer(l) => {
                l.x >= clip_x2
                    || l.y >= clip_y2
                    || l.x.saturating_add(l.width) <= clip_x
                    || l.y.saturating_add(l.height) <= clip_y
            }
        };
        if outside {
            commands.remove(i);
        } else {
            i += 1;
        }
    }
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

fn layout_block_element_as_layer(
    element: &StyledElement,
    outer_x: u32,
    outer_width: u32,
    background_top: u32,
    cursor_y: &mut u32,
    context: &mut LayoutContext,
    images: &ImageStore,
    fonts: &mut FontContext,
) {
    // Create a sub-context for the element's subtree
    let mut sub_context = LayoutContext {
        background_color: context.background_color,
        ..LayoutContext::default()
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
        );
    }

    *cursor_y = cursor_y.saturating_add(element.style.padding.bottom);
    let final_height = cursor_y.saturating_sub(background_top).max(1);

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
                border_radius: 0,
            }));
        }
        if border_bottom_h > 0 {
            sub_context.commands.push(DrawCommand::Rect(RectCommand {
                x: outer_x,
                y: cursor_y.saturating_sub(border_bottom_h),
                width: outer_width.max(1),
                height: border_bottom_h,
                color: bc,
                border_radius: 0,
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
            sub_context.commands.push(DrawCommand::Rect(RectCommand {
                x: outer_x
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

    // Rebase sub-commands to layer-relative coordinates before wrapping
    rebase_commands(&mut sub_context.commands, outer_x, background_top);

    // Wrap sub-context commands in a LayerCommand and push to parent
    context.commands.push(DrawCommand::Layer(LayerCommand {
        x: outer_x,
        y: background_top,
        width: outer_width.max(1),
        height: final_height,
        opacity: element.style.opacity,
        commands: sub_context.commands,
    }));

    // Propagate links from sub_context to parent (links are for hit-testing, not compositing)
    context.links.extend(sub_context.links);
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

    if element.style.opacity < 255 {
        // Wrap the image in a LayerCommand so opacity is applied correctly
        let img_cmd = DrawCommand::Image(ImageCommand {
            x: 0,
            y: 0,
            width: draw_width,
            height: draw_height,
            src: src.to_string(),
        });
        context.commands.push(DrawCommand::Layer(LayerCommand {
            x: draw_x,
            y: *cursor_y,
            width: draw_width,
            height: draw_height,
            opacity: element.style.opacity,
            commands: vec![img_cmd],
        }));
    } else {
        context.commands.push(DrawCommand::Image(ImageCommand {
            x: draw_x,
            y: *cursor_y,
            width: draw_width,
            height: draw_height,
            src: src.to_string(),
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

    let cell_layouts = placements
        .iter()
        .map(|placement| {
            let span_width = span_width(&column_widths, placement.column_index, placement.colspan)
                .saturating_add(spacing.saturating_mul(placement.colspan.saturating_sub(1) as u32));
            let inner_width = span_width.saturating_sub(padding.saturating_mul(2)).max(1);
            let cell_backdrop = placement.cell.style.background_color
                .unwrap_or(context.background_color);
            layout_table_cell(placement.cell, inner_width, images, fonts, cell_backdrop)
        })
        .collect::<Vec<_>>();

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
                commands: layer_commands,
            }));
            // Links are content-relative; shift by cell position + padding/valign
            context.links.extend(layout.links.iter().map(|link| LinkCommand {
                x: link.x.saturating_add(cell_x).saturating_add(padding),
                y: link.y.saturating_add(cell_y).saturating_add(padding).saturating_add(vertical_offset),
                width: link.width,
                height: link.height,
                href: link.href.clone(),
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
) -> FragmentLayout {
    let mut context = LayoutContext { background_color, ..LayoutContext::default() };
    let mut cursor_y = 0_u32;

    for child in &cell.children {
        if is_hidden(child) {
            continue;
        }

        layout_node(child, 0, width, &mut cursor_y, &mut context, images, fonts);
    }

    FragmentLayout {
        content_height: cursor_y.max(1),
        commands: context.commands,
        links: context.links,
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
            x: link.x.saturating_add(offset_x),
            y: link.y.saturating_add(offset_y),
            width: link.width,
            height: link.height,
            href: link.href.clone(),
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
        }),
        DrawCommand::Layer(layer) => DrawCommand::Layer(LayerCommand {
            x: layer.x.saturating_add(offset_x),
            y: layer.y.saturating_add(offset_y),
            width: layer.width,
            height: layer.height,
            opacity: layer.opacity,
            // Do NOT recurse into layer.commands — they're already layer-relative.
            // TODO: wrap commands in Rc<[DrawCommand]> for O(1) clone when deep/large
            //       layer trees are encountered during fragment merge (perf, not correctness).
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

            layout_node(child, x, width, cursor_y, context, images, fonts);
        } else {
            if bullet_pending {
                inline_fragments.push(InlineFragment::Text {
                    text: "- ".to_string(),
                    style: element.style.clone(),
                    link_href: None,
                });
                bullet_pending = false;
            }
            collect_inline_fragments(child, &mut inline_fragments, None);
        }
    }

    if !inline_fragments.is_empty() || bullet_pending {
        if bullet_pending {
            inline_fragments.push(InlineFragment::Text {
                text: "- ".to_string(),
                style: element.style.clone(),
                link_href: None,
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
) {
    match node {
        StyledNode::Text(text) => {
            output.push(InlineFragment::Text {
                text: text.text.clone(),
                style: text.style.clone(),
                link_href: link_href.map(str::to_string),
            });
        }
        StyledNode::Element(element) => {
            let current_link = if element.tag_name == "a" {
                element
                    .attributes
                    .get("href")
                    .map(String::as_str)
                    .or(link_href)
            } else {
                link_href
            };

            match element.style.display {
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
                            link_href: current_link.map(str::to_string),
                        });
                        return;
                    }

                    for child in &element.children {
                        collect_inline_fragments(child, output, current_link);
                    }
                }
                Display::Block | Display::ListItem => {}
            }
        }
    }
}

fn flatten_inline_fragments(node: &StyledNode) -> Vec<InlineFragment> {
    let mut fragments = Vec::new();
    collect_inline_fragments(node, &mut fragments, None);
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
            InlineFragment::Text {
                text,
                style,
                link_href,
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
                            line.push_span(" ", style, fonts, link_href.as_deref());
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
            InlineFragment::Text {
                text,
                style,
                link_href,
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
        line.push_span(word, style, fonts, link_href);
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
            line.push_span(&chunk, style, fonts, link_href);
            emit_line(line, container_style, x, width, cursor_y, context, fonts);
            chunk.clear();
        }
    }

    if !chunk.is_empty() {
        if !line.is_empty() && line.width.saturating_add(text_width(style, &chunk, fonts)) > width {
            emit_line(line, container_style, x, width, cursor_y, context, fonts);
        }
        line.push_span(&chunk, style, fonts, link_href);
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
            font_family: span.style.font_family,
            color: apply_opacity(span.style.color, context.background_color, span_opacity),
            underline: span.style.underline,
            bold: span.style.font_weight,
            italic: span.style.font_style_italic,
        }));

        if let Some(href) = &span.link_href {
            context.links.push(LinkCommand {
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
        let styled = build_styled_tree(&document, &stylesheet, 1280);
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
        let styled = build_styled_tree(&document, &stylesheet, 1280);
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
        let styled = build_styled_tree(&document, &stylesheet, 1280);
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

        let images_list = layout.images();
        assert_eq!(images_list.len(), 1);
        assert_eq!(images_list[0].width, 40);
        assert_eq!(images_list[0].height, 20);
    }

    #[test]
    fn auto_width_tables_do_not_expand_to_full_container() {
        let document =
            parse_document("<table align=\"center\"><tr><td>Hello</td><td>World</td></tr></table>");
        let styled = build_styled_tree(&document, &parse_stylesheet(""), 1280);
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
        let styled = build_styled_tree(&document, &parse_stylesheet(""), 1280);
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
        let styled = build_styled_tree(&document, &parse_stylesheet(""), 1280);
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
        let styled = build_styled_tree(&document, &stylesheet, 1280);
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
        let styled = build_styled_tree(&document, &stylesheet, 1280);
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
        let styled = build_styled_tree(&doc, &stylesheet, 800);
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
        let styled = build_styled_tree(&doc, &stylesheet, 800);
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
        let styled = build_styled_tree(&doc, &stylesheet, 800);
        let mut fonts = FontContext::load();
        let images = ImageStore::default();
        let layout = layout_styled_document(&styled, &images, 800, &mut fonts);

        // Should have a black shadow rect
        let rects = layout.rects();
        let shadow_rect = rects.iter().find(|r| r.color == 0x000000);
        assert!(shadow_rect.is_some(), "Should have a shadow rect with black color");
    }
}