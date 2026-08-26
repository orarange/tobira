use crate::css::{
    BackgroundRepeat, BackgroundSize, BoxSizing, Color, ComputedStyle, CursorKind, DEFAULT_BACKGROUND_COLOR, Display,
    FontFamilyKind, GridEdge, GridTrackSize, LengthValue, ListStyleType, ObjectFit, Overflow, Position, FlexDirection,
    FlexWrap, AlignItems, AlignSelf, JustifyContent, StyledElement, StyledNode, TextAlign, TextTransform,
    VerticalAlign, WhiteSpaceMode, apply_text_transform, ClearSide, FloatSide,
};
use crate::font::FontContext;
use crate::image::ImageStore;
use std::sync::Arc;

/// Width reserved to the left of a list item's content for its marker.
const MARKER_INDENT: u32 = 16;

/// The marker string for a list item, or `None` when it renders without one.
///
/// `list-style-type` was parsed into the computed style but never read back
/// here: every `display: list-item` box got a hardcoded `"- "`. That is wrong
/// in both directions. Navigation menus are `<ul>` markup with
/// `list-style: none` -- structure for assistive tech, no bullets on screen --
/// and rendered as a column of stray dashes; ordered lists lost their numbers.
fn list_marker_text(style: &ComputedStyle, ordinal: u32) -> Option<String> {
    if style.display != Display::ListItem {
        return None;
    }
    Some(match style.list_style_type {
        ListStyleType::None => return None,
        ListStyleType::Disc => "\u{2022} ".to_string(),
        ListStyleType::Circle => "\u{25e6} ".to_string(),
        ListStyleType::Square => "\u{25aa} ".to_string(),
        ListStyleType::Decimal => format!("{ordinal}. "),
    })
}

/// Where a box's border edge sits, honouring `margin-left/right: auto`.
///
/// Two auto margins share the leftover space equally -- the idiom that centres
/// a fixed-width band on the page. Only the block path did this; a flex or grid
/// container with `width: 990px; margin: 0 auto` was pinned to the left edge,
/// which on Yahoo! JAPAN put the masthead band 145px off and dragged the logo
/// pinned inside it along with it.
fn outer_x_with_auto_margins(
    style: &ComputedStyle,
    container_x: u32,
    container_width: u32,
    outer_width: u32,
) -> u32 {
    if style.margin_left_auto && style.margin_right_auto && outer_width < container_width {
        return container_x.saturating_add(container_width.saturating_sub(outer_width) / 2);
    }
    if style.margin_left_auto && !style.margin_right_auto && outer_width < container_width {
        return container_x.saturating_add(container_width.saturating_sub(outer_width));
    }
    offset_x_by_margin(container_x, style.margin.left)
}

fn advance_by_margin(cursor: u32, m: i32) -> u32 {
    (cursor as i64 + m as i64).max(0) as u32
}

fn offset_x_by_margin(x: u32, m: i32) -> u32 {
    (x as i64 + m as i64).max(0) as u32
}

fn outer_width_with_margins(width: u32, ml: i32, mr: i32) -> u32 {
    (width as i64 - (ml as i64 + mr as i64)).max(1) as u32
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GradientStop {
    pub color: u32,
    pub position: u32, // 0-1000 (thousandths)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GradientCommand {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub border_radius: u32,
    pub angle_deg_x1000: i32,
    pub stops: Vec<GradientStop>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrawCommand {
    Rect(RectCommand),
    Text(TextCommand),
    Image(ImageCommand),
    Layer(LayerCommand),
    Gradient(GradientCommand),
    Sticky(StickyCommand),
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
    // CSS transform (applied during composite)
    pub scale_x: u32,          // millis: 1000 = 1.0. 0 = no scale (treated as 1000)
    pub scale_y: u32,          // millis: 1000 = 1.0. 0 = no scale (treated as 1000)
    pub rotate_millideg: i32,  // rotation in millidegrees. 0 = no rotation
    pub origin_x: u32,         // transform-origin X as permille of width (500 = 50% = center)
    pub origin_y: u32,         // transform-origin Y as permille of height (500 = 50% = center)
    pub commands: Vec<DrawCommand>,
}

/// A sticky-positioned element. It lays out in normal flow (at `normal_y`) but is rendered
/// at `max(normal_y, min(scroll_y + sticky_top, container_bottom - layer.height))` so it
/// pins near the viewport top when scrolled past.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StickyCommand {
    /// Element's y-coordinate in normal flow (content space).
    pub normal_y: u32,
    /// CSS `top` value in pixels — distance from viewport content-top when sticking.
    pub sticky_top: u32,
    /// Bottom boundary of the containing block in content space. Use `u32::MAX` when unknown
    /// (element sticks indefinitely until the end of the page).
    pub container_bottom: u32,
    /// The element's rendering data. `layer.y` equals `normal_y`; commands are element-relative
    /// (rebased to origin (0,0) relative to layer.x / layer.y).
    pub layer: LayerCommand,
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
        DrawCommand::Gradient(g) => {
            g.x = g.x.saturating_add(dx);
            g.y = g.y.saturating_add(dy);
        }
        DrawCommand::Sticky(s) => {
            s.layer.x = s.layer.x.saturating_add(dx);
            s.layer.y = s.layer.y.saturating_add(dy);
            s.normal_y = s.normal_y.saturating_add(dy);
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
        DrawCommand::Gradient(g) => {
            g.x = (g.x as i64 + dx as i64).max(0) as u32;
            g.y = (g.y as i64 + dy as i64).max(0) as u32;
        }
        DrawCommand::Sticky(s) => {
            s.layer.x = (s.layer.x as i64 + dx as i64).max(0) as u32;
            s.layer.y = (s.layer.y as i64 + dy as i64).max(0) as u32;
            s.normal_y = (s.normal_y as i64 + dy as i64).max(0) as u32;
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
            DrawCommand::Sticky(s) => {
                out.extend(collect_texts(&s.layer.commands, offset_x.saturating_add(s.layer.x), offset_y.saturating_add(s.layer.y)));
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
            DrawCommand::Sticky(s) => {
                out.extend(collect_rects(&s.layer.commands, offset_x.saturating_add(s.layer.x), offset_y.saturating_add(s.layer.y)));
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
            DrawCommand::Sticky(s) => {
                out.extend(collect_images(&s.layer.commands, offset_x.saturating_add(s.layer.x), offset_y.saturating_add(s.layer.y)));
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
    pub line_through: bool,
    pub bold: bool,
    pub italic: bool,
    pub text_shadow: Option<crate::css::TextShadow>,
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
    pub tile: bool,  // true = background-repeat tile at natural size
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
    /// True when colors fell back to the native widget chrome (no CSS
    /// background/border). The renderer applies its own hover/focus affordance
    /// only for these; CSS-styled controls keep their authored colors.
    pub native_chrome: bool,
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
    /// Content height of the nearest ancestor with a definite (pixel) height,
    /// used to resolve a child's `height: <percent>`. `None` when no ancestor
    /// has a definite height (then percentage heights are treated as auto).
    container_height: Option<u32>,
    /// The main size the flex algorithm settled on for the item about to be
    /// laid out, which that item must use rather than working its own out
    /// again. A percentage width on a flex item resolves against the flex
    /// container, and the item is then handed a slot of that size; re-resolving
    /// the percentage against the slot shrinks it every time. firefox.com's
    /// footer columns ask for `calc(25% - 12px)` of a 1050px row -- 250px --
    /// and came out 31px wide, so every link stacked one character per line and
    /// the footer ran to six screens.
    flex_item_main_size: Option<u32>,
    /// Ordinal of the list item about to be laid out, set by its container.
    /// `None` for anything that is not a numbered item.
    list_ordinal: Option<u32>,
    /// Size of the containing block, which percentage offsets resolve against.
    /// The height is only known when an ancestor states one, so a percentage
    /// `top` falls back to zero rather than to a guess.
    containing_block_size: (u32, u32),
    /// Boxes placed by `bottom` with `top` auto, waiting for the height of the
    /// block that contains them.
    pending_bottom: Vec<PendingBottom>,
}

/// A box anchored to the bottom of its containing block.
///
/// `bottom` cannot be resolved when the box is laid out: the containing block's
/// height is not known until its own children are done, and the box is one of
/// them. So the box is drawn at its static position, its output is remembered,
/// and the containing block moves it once it knows how tall it turned out.
#[derive(Debug, Clone)]
struct PendingBottom {
    /// Index into `positioned_commands` of this box's drawing.
    slot: usize,
    /// Where the box's links, controls and hitboxes start in the context, so
    /// they move with it.
    links_from: usize,
    controls_from: usize,
    hitboxes_from: usize,
    /// Where the box was drawn, and how tall it is.
    drawn_top: u32,
    height: u32,
    offset: LengthValue,
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
            container_height: None,
            flex_item_main_size: None,
            list_ordinal: None,
            containing_block_size: (0, 0),
            pending_bottom: Vec::new(),
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

/// An `inline-block` box, laid out into its own coordinate space.
///
/// It is inline-level on the outside -- it sits on a line beside text -- but a
/// block container on the inside, so its contents cannot be flattened into
/// inline fragments. Laying it out up front and carrying the result lets the
/// line breaker treat it as one indivisible box, like an image.
#[derive(Debug, Clone)]
struct AtomicInline {
    commands: Vec<DrawCommand>,
    links: Vec<LinkCommand>,
    controls: Vec<FormControlCommand>,
    hitboxes: Vec<ElementHitbox>,
    width: u32,
    height: u32,
}

#[derive(Debug, Clone)]
enum InlineFragment {
    Atomic(Box<AtomicInline>),
    Text {
        text: String,
        style: Arc<ComputedStyle>,
        link_href: Option<String>,
        link_node_id: Option<usize>,
    },
    Image {
        src: String,
        draw_width: u32,
        draw_height: u32,
        style: Arc<ComputedStyle>,
        link_href: Option<String>,
        link_node_id: Option<usize>,
    },
    Control(Box<FormControlSpec>),
    LineBreak,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InlineImageSpec {
    src: String,
    draw_width: u32,
    draw_height: u32,
    style: ComputedStyle,
    link_href: Option<String>,
    link_node_id: Option<usize>,
}

#[derive(Debug, Clone)]
struct LineSpan {
    text: String,
    width: u32,
    height: u32,
    /// Shared with the fragment this span was cut from. Inline, a `ComputedStyle`
    /// is 520 bytes and one span is produced per word, so copying it per word
    /// dominated both layout allocation and memory for text-heavy pages.
    style: Arc<ComputedStyle>,
    link_href: Option<String>,
    link_node_id: Option<usize>,
    /// Boxed: both carry a full `ComputedStyle`, and inline they made every
    /// plain text span pay ~1.3 KB for cases it never uses. Controls and inline
    /// images are rare enough that one allocation each is the better trade.
    control: Option<Box<FormControlSpec>>,
    image: Option<Box<InlineImageSpec>>,
    atomic: Option<Box<AtomicInline>>,
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
    style: Arc<ComputedStyle>,
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
        style: &Arc<ComputedStyle>,
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
                && last.image.is_none()
                && last.atomic.is_none()
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
            image: None,
            atomic: None,
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
            control: Some(Box::new(control.clone())),
            image: None,
            atomic: None,
        });
    }

    fn push_image(
        &mut self,
        src: &str,
        draw_width: u32,
        draw_height: u32,
        style: &Arc<ComputedStyle>,
        link_href: Option<&str>,
        link_node_id: Option<usize>,
    ) {
        self.width = self.width.saturating_add(draw_width);
        self.line_height = self.line_height.max(draw_height);
        let image = InlineImageSpec {
            src: src.to_string(),
            draw_width,
            draw_height,
            style: (**style).clone(),
            link_href: link_href.map(str::to_string),
            link_node_id,
        };
        self.spans.push(LineSpan {
            text: String::new(),
            width: draw_width,
            height: draw_height,
            style: style.clone(),
            link_href: link_href.map(str::to_string),
            link_node_id,
            control: None,
            image: Some(Box::new(image)),
            atomic: None,
        });
    }

    fn push_atomic(&mut self, atomic: Box<AtomicInline>, style: &ComputedStyle) {
        self.width = self.width.saturating_add(atomic.width);
        self.line_height = self.line_height.max(atomic.height);
        self.spans.push(LineSpan {
            text: String::new(),
            width: atomic.width,
            height: atomic.height,
            style: Arc::new(style.clone()),
            link_href: None,
            link_node_id: None,
            control: None,
            image: None,
            atomic: Some(atomic),
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

/// Background + border colors for an interactive control. Authored CSS wins
/// (`background` on the element; `border` when an actual border was set);
/// otherwise fall back to the native-widget chrome. Disabled controls always
/// use the grayed-out chrome. The returned bool is `native_chrome`: true when
/// neither background nor border came from CSS, so the renderer may apply its
/// native hover/focus affordance (a CSS-styled control keeps its own colors,
/// and its `:hover` rule already flows through here on the hover relayout).
fn control_colors(spec: &FormControlSpec) -> (Color, Color, bool) {
    if spec.disabled {
        return (0xE4E6EA, 0xA9AFB8, true);
    }
    let css_bg = spec.style.background_color;
    let background = css_bg.unwrap_or(if matches!(spec.kind, FormControlKind::Button) {
        0xE7EBF2
    } else {
        0xFFFFFF
    });
    let has_css_border = !spec.style.border_style_none
        && (spec.style.border.left > 0
            || spec.style.border.top > 0
            || spec.style.border.right > 0
            || spec.style.border.bottom > 0);
    let border = if has_css_border {
        spec.style.border_color
    } else {
        0x7F8B9C
    };
    let native_chrome = css_bg.is_none() && !has_css_border;
    (background, border, native_chrome)
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

/// The nodes that actually take part in `element`'s formatting context.
///
/// `display: contents` makes an element generate no box at all: its children
/// stand in its place. MDN's navigation depends on it -- `.navigation__popup`
/// is `display:contents`, so its three children are the grid's items. Treating
/// the wrapper as a box of its own stacked them vertically and made the sticky
/// header 606px tall instead of the 66px its `--navigation-height` asks for.
fn formatting_context_children(element: &StyledElement) -> Vec<&StyledNode> {
    fn walk<'a>(nodes: &'a [StyledNode], out: &mut Vec<&'a StyledNode>) {
        for node in nodes {
            match node {
                StyledNode::Element(el) if el.style.display == Display::Contents => {
                    walk(&el.children, out);
                }
                _ => out.push(node),
            }
        }
    }
    let mut out = Vec::new();
    walk(&element.children, &mut out);
    out
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

            // An absolutely positioned box is placed relative to the padding
            // box of its nearest *positioned* ancestor, so every box that is
            // not `position: static` becomes the containing block for the
            // subtree beneath it. Nothing ever set this, leaving it at the page
            // origin: every `position: absolute` element on every page was laid
            // out against (0, 0). On Yahoo! JAPAN that dropped the ranking
            // badges of its trending-keyword list onto the masthead.
            match element.style.display {
                Display::None => {}
                // No box of its own: the children take this element's place.
                Display::Contents => {
                    for child in &element.children {
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
                    }
                }
                Display::Inline => {
                    let fragments =
                        flatten_inline_fragments(node, context, current_form.clone(), images, fonts, width);
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
                // Reached only when a container lays its children out
                // directly (a flex or grid item, say); in an inline formatting
                // context an `inline-block` becomes an atomic inline instead.
                Display::Block | Display::ListItem | Display::InlineBlock => {
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
                    // Inline-level: as wide as its contents, not as wide as the
                    // space on offer, and placed by the surrounding
                    // `text-align`. Given the whole line instead, firefox.com's
                    // download button spanned the hero from edge to edge rather
                    // than sitting centred at 246px, and its 32px border radius
                    // turned the 100px-tall result into an oval.
                    let current_form = form_context_for_element(element, context, current_form);
                    let inline_width = element.style.width
                        .map(|w| match w {
                            LengthValue::Pixels(px) => px,
                            LengthValue::Percent(pct) => (width as f32 * pct as f32 / 100.0) as u32,
                            LengthValue::MinContent => 0,
                            LengthValue::MaxContent => width,
                            LengthValue::FitContent(max_px) => width.min(max_px),
                            LengthValue::Calc { percent_hundredths, px } => crate::css::resolve_calc(percent_hundredths, px, width),
                        })
                        .unwrap_or_else(|| {
                            // A plain measurement, not a trial layout: laying the
                            // box out to size it re-enters this very arm and the
                            // recursion never bottoms out.
                            let surround = element.style.padding.left
                                + element.style.padding.right
                                + if element.style.border_style_none {
                                    0
                                } else {
                                    element.style.border.left + element.style.border.right
                                };
                            measure_cell_preferred_width(element, 0, images, fonts)
                                .saturating_add(surround)
                                .min(width)
                                .max(1)
                        });
                    let offset = match element.style.text_align {
                        TextAlign::Center => width.saturating_sub(inline_width) / 2,
                        TextAlign::Right => width.saturating_sub(inline_width),
                        TextAlign::Left => 0,
                    };
                    layout_flex_container(element, x.saturating_add(offset), inline_width, cursor_y, context, images, fonts, current_form.clone());
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
                            LengthValue::Calc { percent_hundredths, px } => crate::css::resolve_calc(percent_hundredths, px, width),
                        })
                        .unwrap_or(width);
                    layout_grid_container(element, x, inline_width, cursor_y, context, images, fonts, current_form);
                }
            }
        }
    }
}

/// Honor an explicit CSS `height` (pixel value) as a *minimum* box height.
///
/// Block layout otherwise sizes a box purely from its content, so
/// `<section style="height: 1200px">` with little content collapses to a few
/// lines. If the element specifies a pixel height taller than the laid-out
/// content, expand the box so that height is reserved in the normal flow and
/// advance `cursor_y` to the bottom of the expanded box (so following siblings
/// and the page's scroll height account for it).
///
/// Smaller explicit heights are ignored rather than clipping content — content
/// is never hidden by this path (real `overflow: hidden` clipping is handled
/// separately). Percent / min-/max-/fit-content heights are left to the
/// content-based height because resolving them needs the containing block's
/// height, which is not threaded through here.
/// The element's definite content height in px (a pixel value, or a percentage
/// resolved against the containing block's definite height), or `None` for an
/// auto height. Used to set the containing block for descendant percent heights.
fn resolve_definite_height(style: &ComputedStyle, container_height: Option<u32>) -> Option<u32> {
    match style.height {
        Some(LengthValue::Pixels(px)) => Some(px),
        Some(LengthValue::Percent(pct)) => {
            container_height.map(|ch| (ch as u64 * pct as u64 / 100) as u32)
        }
        _ => None,
    }
}

fn explicit_box_height(
    style: &ComputedStyle,
    background_top: u32,
    content_height: u32,
    cursor_y: &mut u32,
    container_height: Option<u32>,
) -> u32 {
    let height = specified_box_height(style, background_top, content_height, cursor_y, container_height);

    // `max-height` caps whatever we arrived at, including a purely content-based
    // height. It was parsed into the style and then never consulted, so MDN's
    // decorative mandala -- `max-height: 20rem; overflow: hidden` around a 35rem
    // drawing -- claimed its full 608px instead of the 320px it asks for.
    //
    // Capping pulls the following content up, which is what a browser does too:
    // overflow does not participate in layout. Anything spilling past the cap is
    // painted over what follows unless the box also clips.
    let Some(max_content) = style.max_height else {
        return height;
    };
    let cap = max_content
        .saturating_add(style.padding.top)
        .saturating_add(style.padding.bottom)
        .max(1);
    if height > cap {
        *cursor_y = background_top.saturating_add(cap);
        return cap;
    }
    height
}

/// The height `height` alone asks for, before `max-height` gets a say.
fn specified_box_height(
    style: &ComputedStyle,
    background_top: u32,
    content_height: u32,
    cursor_y: &mut u32,
    container_height: Option<u32>,
) -> u32 {
    // Resolve the specified content-box height to pixels: a pixel value directly,
    // or a percentage against the containing block's definite height.
    let px = match style.height {
        Some(LengthValue::Pixels(px)) => px,
        Some(LengthValue::Percent(pct)) => match container_height {
            Some(ch) => (ch as u64 * pct as u64 / 100) as u32,
            None => return content_height, // % against an auto height → auto
        },
        _ => return content_height,
    };
    // `height` is the content-box height; this engine's box height also spans the
    // vertical padding (borders are drawn overlapping it), so add the padding to
    // arrive at the box height the surrounding code accounts for.
    let desired = px
        .saturating_add(style.padding.top)
        .saturating_add(style.padding.bottom)
        .max(1);
    if desired > content_height {
        *cursor_y = background_top.saturating_add(desired);
        desired
    } else {
        content_height
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
    // Taken here, before any dispatch, so it cannot survive into a descendant
    // and be spent on the wrong box.
    let settled_main_size = context.flex_item_main_size.take();

    if element.tag_name == "br" {
        *cursor_y = cursor_y.saturating_add(text_line_height(&element.style, fonts));
        return;
    }

    // A box that is itself a flex or grid container must not be laid out as a
    // plain block. Only `layout_node` dispatched on `display`; every caller that
    // places children itself -- a flex container sizing its items, a grid, a
    // table cell, an absolutely positioned box -- came straight here and so
    // ignored the box's own `display`. Yahoo! JAPAN's masthead is a flex row
    // nested inside another flex row, and the inner one stacked its two service
    // groups vertically instead of putting them side by side.
    match element.style.display {
        Display::Flex | Display::InlineFlex | Display::Grid | Display::InlineGrid => {
            // An `inline-flex` or `inline-grid` box is inline-level: it is as
            // wide as its contents, not as wide as the space it is offered, and
            // the surrounding `text-align` places it. Laid out as a block it
            // spanned the whole line -- firefox.com's download button filled the
            // hero from edge to edge instead of sitting centred at 246px, and
            // its 32px border radius made a 100px-tall oval of it.
            let inline_level = matches!(
                element.style.display,
                Display::InlineFlex | Display::InlineGrid
            );
            let (x, width) = if inline_level && settled_main_size.is_none() {
                let shrunk = flex_item_content_width(
                    element,
                    width,
                    images,
                    fonts,
                    context.background_color,
                )
                .min(width)
                .max(1);
                let free = width.saturating_sub(shrunk);
                let offset = match element.style.text_align {
                    TextAlign::Center => free / 2,
                    TextAlign::Right => free,
                    TextAlign::Left => 0,
                };
                (x.saturating_add(offset), shrunk)
            } else {
                (x, settled_main_size.unwrap_or(width))
            };
            if matches!(element.style.display, Display::Flex | Display::InlineFlex) {
                layout_flex_container(element, x, width, cursor_y, context, images, fonts, current_form);
            } else {
                layout_grid_container(element, x, width, cursor_y, context, images, fonts, current_form);
            }
            return;
        }
        _ => {}
    }

    if element.tag_name == "table" {
        let needs_layer = element.style.opacity < 255
            || element.style.filter_blur_px > 0
            || element.style.filter_brightness != 10000
            || element.style.transform_scale_x != 0
            || element.style.transform_scale_y != 0
            || element.style.transform_rotate_millideg != 0;
        if needs_layer {
            // Table with opacity/filter/transform: render into sub-context and wrap in a LayerCommand
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
                scale_x: element.style.transform_scale_x,
                scale_y: element.style.transform_scale_y,
                rotate_millideg: element.style.transform_rotate_millideg,
                origin_x: element.style.transform_origin_x,
                origin_y: element.style.transform_origin_y,
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

    // Form controls (button / input / textarea) can reach block layout as flex
    // items (a flex container lays each child out via `layout_block_element`) or
    // as `display:block` elements. The inline path registers them as interactive
    // controls via `push_control`; without mirroring that here, a control inside
    // `display:flex` paints but is dead to hit-testing (no FormControlCommand →
    // hit-test never returns it → clicks/focus do nothing). Emit the control at
    // this block position so it stays clickable, matching the inline behavior.
    if matches!(element.tag_name.as_str(), "input" | "textarea" | "button") {
        if let Some(spec) = build_form_control_spec(element, current_form.as_ref(), context) {
            if matches!(spec.kind, FormControlKind::Hidden) {
                // Hidden inputs take no space and need no geometry.
                return;
            }
            let (ctrl_w, ctrl_h) = measure_form_control(&spec, fonts);
            *cursor_y = advance_by_margin(*cursor_y, element.style.margin.top);
            let control_x = offset_x_by_margin(x, element.style.margin.left);
            // Shrink-to-fit the control, but never exceed the slot width.
            let final_w = ctrl_w.min(width.max(1)).max(1);
            let (background_color, border_color, native_chrome) = control_colors(&spec);
            context.controls.push(FormControlCommand {
                id: spec.id,
                node_id: spec.node_id,
                form_node_id: spec.form_node_id,
                kind: spec.kind,
                x: control_x,
                y: *cursor_y,
                width: final_w,
                height: ctrl_h.max(1),
                name: spec.name.clone(),
                value: spec.value.clone(),
                label: spec.label.clone(),
                placeholder: spec.placeholder.clone(),
                form_id: spec.form_id,
                form_action: spec.form_action.clone(),
                form_method: spec.form_method.clone(),
                activates_submit: spec.activates_submit,
                disabled: spec.disabled,
                masked: spec.masked,
                font_size_px: element.style.font_size_px,
                font_family: element.style.font_family,
                text_color: element.style.color,
                background_color,
                border_color,
                native_chrome,
            });
            *cursor_y = advance_by_margin(*cursor_y, ctrl_h as i32);
            *cursor_y = advance_by_margin(*cursor_y, element.style.margin.bottom);
            return;
        }
    }

    let block_cmd_start = context.commands.len();

    *cursor_y = advance_by_margin(*cursor_y, element.style.margin.top);

    // Resolve explicit width from style.width (LengthValue → px)
    let explicit_width: Option<u32> = settled_main_size.or_else(|| element.style.width.map(|w| match w {
        LengthValue::Pixels(px) => px,
        LengthValue::Percent(pct) => (width as u64 * pct as u64 / 100).min(width as u64) as u32,
        LengthValue::MinContent => 0,
        LengthValue::MaxContent => width,
        LengthValue::FitContent(max_px) => width.min(max_px),
        LengthValue::Calc { percent_hundredths, px } => crate::css::resolve_calc(percent_hundredths, px, width),
    }));

    // Container-derived width (what the element would be without explicit width)
    let container_derived_width = {
        let ml = if element.style.margin_left_auto { 0 } else { element.style.margin.left };
        let mr = if element.style.margin_right_auto { 0 } else { element.style.margin.right };
        // `max-width` / `min-width` percentages resolve against the containing
        // block, not the font size. Resolving them at parse time against the
        // font size turned the extremely common `max-width: 100%` into 16px,
        // which squeezed text down to one character per line.
        let max = element
            .style
            .max_width
            .map(|length| resolve_length_value(length, width))
            .unwrap_or(u32::MAX);
        let min = element
            .style
            .min_width
            .map(|length| resolve_length_value(length, width))
            .unwrap_or(0);
        outer_width_with_margins(width, ml, mr).min(max).max(min)
    };

    // Compute outer_width: only use explicit width when it actually constrains (is narrower).
    // This prevents HTML width="" attributes from incorrectly shrinking table-allocated cells.
    let (outer_width, width_is_constrained) = if let Some(ew) = explicit_width {
        let max = element
            .style
            .max_width
            .map(|length| resolve_length_value(length, width))
            .unwrap_or(u32::MAX);
        let min = element
            .style
            .min_width
            .map(|length| resolve_length_value(length, width))
            .unwrap_or(0);
        let clamped = ew.min(max).max(min);
        if clamped < container_derived_width {
            (clamped, true)
        } else {
            (container_derived_width, false)
        }
    } else {
        (container_derived_width, false)
    };

    // Compute outer_x: center when both margins are auto AND width is actually constrained.
    let outer_x = if element.style.margin_left_auto && element.style.margin_right_auto && width_is_constrained {
        let total_margin = width.saturating_sub(outer_width);
        x.saturating_add(total_margin / 2)
    } else if element.style.margin_right_auto && !element.style.margin_left_auto {
        offset_x_by_margin(x, element.style.margin.left)
    } else {
        offset_x_by_margin(x, element.style.margin.left)
    };

    let background_top = *cursor_y;

    // Detect stacking context: element has opacity < 255, filter: blur(), or CSS transform scale/rotate
    let needs_layer = element.style.opacity < 255
        || element.style.filter_blur_px > 0
        || element.style.filter_brightness != 10000
        || element.style.transform_scale_x != 0
        || element.style.transform_scale_y != 0
        || element.style.transform_rotate_millideg != 0;
    if needs_layer {
        layout_block_element_as_layer(
            element, outer_x, outer_width, background_top, cursor_y, context, images, fonts, current_form,
        );
        *cursor_y = advance_by_margin(*cursor_y, element.style.margin.bottom);
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

    // Insert background image placeholder BEFORE children so it renders behind them.
    let bg_img_tile = matches!(element.style.background_repeat,
        BackgroundRepeat::Repeat | BackgroundRepeat::RepeatX | BackgroundRepeat::RepeatY);
    let bg_img_object_fit = if bg_img_tile { ObjectFit::None } else {
        match element.style.background_size {
            BackgroundSize::Cover => ObjectFit::Cover,
            BackgroundSize::Contain => ObjectFit::Contain,
            BackgroundSize::Auto => ObjectFit::Fill,
        }
    };
    let bg_image_cmd_idx: Option<usize> = element.style.background_image_url.as_ref().map(|url| {
        let idx = context.commands.len();
        context.commands.push(DrawCommand::Image(ImageCommand {
            x: outer_x,
            y: background_top,
            width: outer_width.max(1),
            height: 1, // placeholder; updated after children
            src: url.clone(),
            object_fit: bg_img_object_fit,
            object_position_x: element.style.background_position_x,
            object_position_y: element.style.background_position_y,
            tile: bg_img_tile,
        }));
        idx
    });

    // Capture clip start BEFORE children are laid out, so overflow:hidden can correctly
    // filter commands added by children (even when there is no background rect).
    let clip_start_idx = context.commands.len();

    *cursor_y = cursor_y.saturating_add(element.style.padding.top);

    let marker = list_marker_text(&element.style, context.list_ordinal.unwrap_or(1));
    let bullet_indent = if marker.is_some() { MARKER_INDENT } else { 0 };

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

    // An absolutely positioned box is placed against the padding box of its
    // nearest *positioned* ancestor, so any box that is not `position: static`
    // becomes the containing block for the subtree under it. Nothing ever set
    // this, leaving it at the page origin: every `position: absolute` element
    // on every page was laid out against (0, 0). Yahoo! JAPAN's trending list
    // wraps each rank badge in a `position: relative` inline span, so all five
    // badges piled up on the masthead instead of sitting beside their entries.
    let saved_origin = context.containing_block_origin;
    let saved_cb_size = context.containing_block_size;
    // Anything anchored to this block's bottom edge is recorded from here on,
    // and settled below once the block's height is known.
    let pending_mark = context.pending_bottom.len();
    let establishes_containing_block = element.style.position != Position::Static;
    if establishes_containing_block {
        context.containing_block_origin = (outer_x, background_top);
        context.containing_block_size = (outer_width, definite_height(&element.style));
    }

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

    // Resolve this block's definite content height (pixel, or percent against the
    // current containing block) and make it the containing block for descendants'
    // `height: <percent>`. Auto-height blocks leave the inherited value so a
    // percent height passes through wrapper divs to the nearest definite ancestor.
    let parent_container_height = context.container_height;
    let this_content_height = resolve_definite_height(&element.style, parent_container_height);
    let saved_container_height = context.container_height;
    if let Some(h) = this_content_height {
        context.container_height = Some(h);
    }

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
            marker,
            images,
            fonts,
            current_form,
        );
    }
    context.containing_block_origin = saved_origin;
    context.containing_block_size = saved_cb_size;
    context.container_height = saved_container_height;

    *cursor_y = cursor_y.saturating_add(element.style.padding.bottom);
    let content_height = cursor_y.saturating_sub(background_top).max(1);
    // Honor an explicit CSS `height` (px or percent); expands a short box.
    let background_height = explicit_box_height(
        &element.style,
        background_top,
        content_height,
        cursor_y,
        parent_container_height,
    );

    // The block's own bottom padding is part of its height, and it is exactly
    // the room a page reserves for a box anchored there -- so settle against
    // the finished box height, not against the content height.
    if establishes_containing_block {
        settle_bottom_anchored(context, pending_mark, background_top, background_height);
    }

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
    if let Some(idx) = bg_image_cmd_idx {
        if let DrawCommand::Image(ref mut img) = context.commands[idx] {
            img.height = background_height;
        }
    }

    // Emit gradient overlay if background_gradient is set
    if let Some(ref gradient) = element.style.background_gradient {
        let stops: Vec<GradientStop> = gradient.stops.iter().map(|(c, p)| GradientStop {
            color: *c,
            position: *p,
        }).collect();
        context.commands.push(DrawCommand::Gradient(GradientCommand {
            x: outer_x,
            y: background_top,
            width: outer_width.max(1),
            height: background_height,
            border_radius: element.style.border_radius,
            angle_deg_x1000: gradient.angle_deg_x1000,
            stops,
        }));
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
                LengthValue::Calc { percent_hundredths, px } => crate::css::resolve_calc(percent_hundredths, px, background_height),
            })
            .unwrap_or(background_height);
        clip_commands_to_box(
            &mut context.commands,
            clip_start_idx,
            outer_x,
            background_top,
            outer_width,
            clip_height,
            fonts,
        );
    }

    // Draw borders if present
    if !element.style.border_style_none && !element.style.border_color_transparent {
        let bc = apply_opacity(
            element.style.border_color,
            context.background_color,
            element.style.effective_opacity,
        );
        let border_top_h = element.style.border.top;
        let border_bottom_h = element.style.border.bottom;
        let border_left_w = element.style.border.left;
        let border_right_w = element.style.border.right;

        // Which element drew this? Border rects all look alike in the command
        // dump, and tracking one back to its rule is otherwise guesswork.
        if std::env::var_os("TOBIRA_DEBUG_BORDERS").is_some()
            && border_top_h + border_bottom_h + border_left_w + border_right_w > 0
        {
            eprintln!(
                "border <{}> class={:?} t={border_top_h} r={border_right_w} b={border_bottom_h} l={border_left_w} color={bc:#08x}",
                element.tag_name,
                element
                    .attributes
                    .get("class")
                    .map(|c| c.chars().take(30).collect::<String>()),
            );
        }

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

    // position: relative — apply visual offset without affecting flow
    if element.style.position == Position::Relative {
        let (cb_width, cb_height) = context.containing_block_size;
        let offset = |length: Option<LengthValue>, basis: u32| {
            length.map_or(0, |length| resolve_offset(length, basis))
        };
        let dx = offset(element.style.left, cb_width) - offset(element.style.right, cb_width);
        let dy = offset(element.style.top, cb_height) - offset(element.style.bottom, cb_height);
        if dx != 0 || dy != 0 {
            for cmd in &mut context.commands[block_cmd_start..] {
                shift_command_signed(cmd, dx, dy);
            }
        }
    }

    // position: sticky — wrap commands in a StickyCommand for scroll-aware rendering
    if element.style.position == Position::Sticky {
        if let Some(top_px) = element
            .style
            .top
            .map(|length| resolve_offset(length, context.containing_block_size.1))
        {
            let height = cursor_y.saturating_sub(background_top).max(1);
            let mut sticky_cmds: Vec<DrawCommand> = context.commands.drain(block_cmd_start..).collect();
            rebase_commands(&mut sticky_cmds, outer_x, background_top);
            context.commands.push(DrawCommand::Sticky(StickyCommand {
                normal_y: background_top,
                sticky_top: top_px.max(0) as u32,
                container_bottom: u32::MAX,
                layer: LayerCommand {
                    x: outer_x,
                    y: background_top,
                    width: outer_width.max(1),
                    height,
                    opacity: 255,
                    blur_px: 0,
                    brightness: 10000,
                    scale_x: 0,
                    scale_y: 0,
                    rotate_millideg: 0,
                    origin_x: 500,
                    origin_y: 500,
                    commands: sticky_cmds,
                },
            }));
        }
        // If no `top` is set, sticky behaves like static — leave commands as-is.
    }

    // CSS transform: translate — shift all commands for this element by (tx, ty)
    let tx = element.style.transform_translate_x;
    let ty = element.style.transform_translate_y;
    if tx != 0 || ty != 0 {
        for cmd in &mut context.commands[block_cmd_start..] {
            shift_command_signed(cmd, tx, ty);
        }
    }

    *cursor_y = advance_by_margin(*cursor_y, element.style.margin.bottom);
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
/// Trim a text run to the part of it that survives a clip box.
///
/// Returns `None` when nothing of the run is left. Keeping a run whole because
/// it merely *touches* the clip box is how the visually-hidden idiom leaked on
/// screen: `position:absolute; width:1px; height:1px; overflow:hidden` is on
/// almost every real page to expose label text to screen readers only, and a
/// full-width word intersects that 1px box, so the word was painted in full.
fn clip_text_to_box(
    text: TextCommand,
    clip_x: u32,
    clip_x2: u32,
    fonts: &mut FontContext,
) -> Option<TextCommand> {
    if text.x >= clip_x && text.x.saturating_add(text.width) <= clip_x2 {
        return Some(text);
    }

    let mut kept = String::new();
    let mut kept_x = text.x;
    let mut pen = text.x;
    let mut started = false;
    for character in text.text.chars() {
        let advance = fonts.glyph_advance_px(character, text.font_size_px, text.font_family);
        let next = pen.saturating_add(advance);
        // A glyph counts as visible only if it fits entirely inside the box:
        // the renderer draws whole glyphs, so a partly-covered one would spill.
        if pen >= clip_x && next <= clip_x2 {
            if !started {
                kept_x = pen;
                started = true;
            }
            kept.push(character);
        } else if started {
            break;
        }
        pen = next;
    }

    if kept.is_empty() {
        return None;
    }
    let width = fonts.text_width_px(&kept, text.font_size_px, text.font_family);
    Some(TextCommand {
        text: kept,
        x: kept_x,
        width,
        ..text
    })
}

fn clip_commands_to_box(
    commands: &mut Vec<DrawCommand>,
    start: usize,
    clip_x: u32,
    clip_y: u32,
    clip_w: u32,
    clip_h: u32,
    fonts: &mut FontContext,
) {
    let clip_x2 = clip_x.saturating_add(clip_w);
    let clip_y2 = clip_y.saturating_add(clip_h);

    let tail = commands.split_off(start);
    let clamped: Vec<DrawCommand> = tail.into_iter().filter_map(|cmd| {
        let fonts = &mut *fonts;
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
                    clip_text_to_box(t, clip_x, clip_x2, fonts).map(DrawCommand::Text)
                }
            }
            DrawCommand::Gradient(mut g) => {
                let gx2 = g.x.saturating_add(g.width);
                let gy2 = g.y.saturating_add(g.height);
                if g.x >= clip_x2 || g.y >= clip_y2 || gx2 <= clip_x || gy2 <= clip_y {
                    return None;
                }
                let new_x = g.x.max(clip_x);
                let new_y = g.y.max(clip_y);
                let new_x2 = gx2.min(clip_x2);
                let new_y2 = gy2.min(clip_y2);
                g.x = new_x; g.y = new_y;
                g.width = new_x2.saturating_sub(new_x).max(1);
                g.height = new_y2.saturating_sub(new_y).max(1);
                Some(DrawCommand::Gradient(g))
            }
            DrawCommand::Sticky(mut s) => {
                let lx2 = s.layer.x.saturating_add(s.layer.width);
                let ly2 = s.layer.y.saturating_add(s.layer.height);
                if s.layer.x >= clip_x2 || s.layer.y >= clip_y2 || lx2 <= clip_x || ly2 <= clip_y {
                    return None;
                }
                // Clamp width/height only — same as Layer arm
                s.layer.width = lx2.min(clip_x2).saturating_sub(s.layer.x).max(1);
                s.layer.height = ly2.min(clip_y2).saturating_sub(s.layer.y).max(1);
                Some(DrawCommand::Sticky(s))
            }
        }
    }).collect();
    commands.extend(clamped);
}

/// Lay an `inline-block` out into its own coordinate space.
///
/// The box is measured shrink-to-fit -- an `inline-block` is only as wide as
/// its content unless it says otherwise -- and rendered from (0, 0) so the line
/// breaker can place the finished result anywhere.
fn layout_atomic_inline(
    element: &StyledElement,
    available_width: u32,
    context: &mut LayoutContext,
    images: &ImageStore,
    fonts: &mut FontContext,
    current_form: Option<FormContext>,
) -> Option<AtomicInline> {
    // The box has to hold its content *and* its own padding and borders: the
    // width handed to `layout_block_element` is an outer width, and it carves
    // those out again. Sizing the box to the bare content width therefore left
    // the content short by exactly the padding, so Yahoo! JAPAN's weather badge
    // -- 60px of text in a box with 6px of padding either side -- got 48px and
    // wrapped its last character onto a second line.
    // Margins count too: the width handed to `layout_block_element` is the
    // margin box, and it subtracts them before anything else. Yahoo! JAPAN's
    // weather section headings carry 12px of side margins, so the heading text
    // lost twelve pixels and dropped its last character to a second line.
    let surround = element.style.padding.left
        + element.style.padding.right
        + element.style.margin.left.max(0) as u32
        + element.style.margin.right.max(0) as u32
        + if element.style.border_style_none {
            0
        } else {
            element.style.border.left + element.style.border.right
        };
    let width = match element.style.width {
        // An explicit width is the content box unless `box-sizing` says
        // otherwise, so the surround is added on top of it.
        Some(length) => {
            let width = resolve_length_value(length, available_width);
            if element.style.box_sizing == BoxSizing::BorderBox {
                width
            } else {
                width.saturating_add(surround)
            }
        }
        None => measure_cell_preferred_width(element, 0, images, fonts).saturating_add(surround),
    }
    .min(available_width)
    .max(1);

    if std::env::var_os("TOBIRA_DEBUG_ATOMIC").is_some() {
        eprintln!(
            "atomic <{}> class={:?} avail={available_width} -> {width}",
            element.tag_name,
            element.attributes.get("class").map(|c| c.chars().take(24).collect::<String>()),
        );
    }

    let mut sub_context = LayoutContext {
        background_color: context.background_color,
        next_control_id: context.next_control_id,
        next_form_id: context.next_form_id,
        ..LayoutContext::default()
    };
    let mut cursor_y = 0;
    layout_block_element(
        element,
        0,
        width,
        &mut cursor_y,
        &mut sub_context,
        images,
        fonts,
        current_form,
    );
    context.next_control_id = sub_context.next_control_id;
    context.next_form_id = sub_context.next_form_id;
    // Out-of-flow descendants were collected against the page, not this box;
    // they are already positioned and must not be shifted with it.
    for (_, commands) in sub_context.positioned_commands {
        context.positioned_commands.push((0, commands));
    }

    Some(AtomicInline {
        commands: sub_context.commands,
        links: sub_context.links,
        controls: sub_context.controls,
        hitboxes: sub_context.element_hitboxes,
        width,
        height: cursor_y.max(1),
    })
}

/// Shift already-laid-out commands to a new origin -- the inverse of
/// `rebase_commands`.
fn offset_commands(commands: &mut [DrawCommand], dx: u32, dy: u32) {
    for command in commands.iter_mut() {
        match command {
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
                // Inner commands are layer-relative already.
            }
            DrawCommand::Gradient(g) => {
                g.x = g.x.saturating_add(dx);
                g.y = g.y.saturating_add(dy);
            }
            DrawCommand::Sticky(s) => {
                s.layer.x = s.layer.x.saturating_add(dx);
                s.layer.y = s.layer.y.saturating_add(dy);
            }
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
            DrawCommand::Gradient(g) => {
                g.x = g.x.saturating_sub(origin_x);
                g.y = g.y.saturating_sub(origin_y);
            }
            DrawCommand::Sticky(s) => {
                s.layer.x = s.layer.x.saturating_sub(origin_x);
                s.layer.y = s.layer.y.saturating_sub(origin_y);
                s.normal_y = s.normal_y.saturating_sub(origin_y);
                // Do NOT recurse into s.layer.commands — they're already layer-relative
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

    let marker = list_marker_text(&element.style, context.list_ordinal.unwrap_or(1));
    let bullet_indent = if marker.is_some() { MARKER_INDENT } else { 0 };

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

    // An absolutely positioned box is placed against the padding box of its
    // nearest *positioned* ancestor, so any box that is not `position: static`
    // becomes the containing block for the subtree under it. Nothing ever set
    // this, leaving it at the page origin: every `position: absolute` element
    // on every page was laid out against (0, 0). Yahoo! JAPAN's trending list
    // wraps each rank badge in a `position: relative` inline span, so all five
    // badges piled up on the masthead instead of sitting beside their entries.
    let saved_origin = context.containing_block_origin;
    let saved_cb_size = context.containing_block_size;
    // Anything anchored to this block's bottom edge is recorded from here on,
    // and settled below once the block's height is known.
    let pending_mark = context.pending_bottom.len();
    let establishes_containing_block = element.style.position != Position::Static;
    if establishes_containing_block {
        context.containing_block_origin = (outer_x, background_top);
        context.containing_block_size = (outer_width, definite_height(&element.style));
    }

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

    // Containing block height for descendant percent heights (see the non-layer
    // path). Children of a layer use `sub_context`, so seed it from the parent.
    let parent_container_height = context.container_height;
    let this_content_height = resolve_definite_height(&element.style, parent_container_height);
    sub_context.container_height = this_content_height.or(parent_container_height);

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
            marker,
            images,
            fonts,
            current_form,
        );
    }
    context.containing_block_origin = saved_origin;
    context.containing_block_size = saved_cb_size;

    *cursor_y = cursor_y.saturating_add(element.style.padding.bottom);
    let content_height = cursor_y.saturating_sub(background_top).max(1);
    // Honor an explicit CSS `height` (px or percent); expands a short box.
    let final_height = explicit_box_height(
        &element.style,
        background_top,
        content_height,
        cursor_y,
        parent_container_height,
    );

    // The block's own bottom padding is part of its height, and it is exactly
    // the room a page reserves for a box anchored there -- so settle against
    // the finished box height, not against the content height.
    if establishes_containing_block {
        settle_bottom_anchored(context, pending_mark, background_top, final_height);
    }

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

    // Emit gradient overlay if background_gradient is set
    if let Some(ref gradient) = element.style.background_gradient {
        let stops: Vec<GradientStop> = gradient.stops.iter().map(|(c, p)| GradientStop {
            color: *c,
            position: *p,
        }).collect();
        sub_context.commands.push(DrawCommand::Gradient(GradientCommand {
            x: outer_x,
            y: background_top,
            width: outer_width.max(1),
            height: final_height,
            border_radius: element.style.border_radius,
            angle_deg_x1000: gradient.angle_deg_x1000,
            stops,
        }));
    }

    // Emit background image if background_image_url is set
    if let Some(ref url) = element.style.background_image_url {
        let tile = matches!(element.style.background_repeat, BackgroundRepeat::Repeat | BackgroundRepeat::RepeatX | BackgroundRepeat::RepeatY);
        let object_fit = if tile {
            ObjectFit::None
        } else {
            match element.style.background_size {
                BackgroundSize::Cover => ObjectFit::Cover,
                BackgroundSize::Contain => ObjectFit::Contain,
                BackgroundSize::Auto => ObjectFit::Fill,
            }
        };
        sub_context.commands.push(DrawCommand::Image(ImageCommand {
            x: outer_x,
            y: background_top,
            width: outer_width.max(1),
            height: final_height,
            src: url.clone(),
            object_fit,
            object_position_x: element.style.background_position_x,
            object_position_y: element.style.background_position_y,
            tile,
        }));
    }

    // Draw borders into the sub-context (they are part of the composited layer)
    if !element.style.border_style_none && !element.style.border_color_transparent {
        // Borders use raw border_color since they're inside the layer
        let bc = element.style.border_color;
        let border_top_h = element.style.border.top;
        let border_bottom_h = element.style.border.bottom;
        let border_left_w = element.style.border.left;
        let border_right_w = element.style.border.right;

        // Which element drew this? Border rects all look alike in the command
        // dump, and tracking one back to its rule is otherwise guesswork.
        if std::env::var_os("TOBIRA_DEBUG_BORDERS").is_some()
            && border_top_h + border_bottom_h + border_left_w + border_right_w > 0
        {
            eprintln!(
                "border <{}> class={:?} t={border_top_h} r={border_right_w} b={border_bottom_h} l={border_left_w} color={bc:#08x}",
                element.tag_name,
                element
                    .attributes
                    .get("class")
                    .map(|c| c.chars().take(30).collect::<String>()),
            );
        }

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
                LengthValue::Calc { percent_hundredths, px } => crate::css::resolve_calc(percent_hundredths, px, final_height),
            })
            .unwrap_or(final_height);
        clip_commands_to_box(
            &mut sub_context.commands,
            clip_start_idx,
            outer_x,
            background_top,
            outer_width,
            clip_height,
            fonts,
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
        scale_x: element.style.transform_scale_x,
        scale_y: element.style.transform_scale_y,
        rotate_millideg: element.style.transform_rotate_millideg,
        origin_x: element.style.transform_origin_x,
        origin_y: element.style.transform_origin_y,
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
    *cursor_y = advance_by_margin(*cursor_y, element.style.margin.top);

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

    // Images reach this path whether they are inline or block level, so the
    // inherited indent has to be honoured here as well as in the line builder.
    // A negative one puts the image left of the content edge; once it clears the
    // canvas entirely it is invisible, and the unsigned coordinates below cannot
    // say where it went. That is how firefox.com hides the `<img>` inside its
    // header logo link -- painted, it doubled up with the background image the
    // link draws the same logo with.
    let indented_x = i64::from(draw_x) + i64::from(element.style.text_indent);
    let off_canvas = indented_x + i64::from(draw_width) <= 0;
    let draw_x = indented_x.clamp(0, i64::from(u32::MAX)) as u32;

    if off_canvas {
        // Indented clear off the canvas: nothing to paint, though the box still
        // takes up its height below.
    } else if element.style.opacity < 255 || element.style.filter_blur_px > 0 || element.style.filter_brightness != 10000 {
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
            tile: false,
        });
        context.commands.push(DrawCommand::Layer(LayerCommand {
            x: draw_x,
            y: *cursor_y,
            width: draw_width,
            height: draw_height,
            opacity: element.style.opacity,
            blur_px: element.style.filter_blur_px,
            brightness: element.style.filter_brightness,
            scale_x: element.style.transform_scale_x,
            scale_y: element.style.transform_scale_y,
            rotate_millideg: element.style.transform_rotate_millideg,
            origin_x: element.style.transform_origin_x,
            origin_y: element.style.transform_origin_y,
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
            tile: false,
        }));
    }

    *cursor_y = cursor_y.saturating_add(draw_height);
    *cursor_y = advance_by_margin(*cursor_y, element.style.margin.bottom);
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
    *cursor_y = advance_by_margin(*cursor_y, element.style.margin.bottom);
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
    *cursor_y = advance_by_margin(*cursor_y, element.style.margin.top);

    let rows = collect_table_rows(element);
    if rows.is_empty() {
        *cursor_y = advance_by_margin(*cursor_y, element.style.margin.bottom);
        return;
    }

    let placements = build_table_placements(&rows);
    let column_count = placements
        .iter()
        .map(|placement| placement.column_index + placement.colspan)
        .max()
        .unwrap_or(0);
    if column_count == 0 {
        *cursor_y = advance_by_margin(*cursor_y, element.style.margin.bottom);
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
    if preferred_content_width > target_content_width {
        shrink_column_widths(&mut sizing, preferred_content_width - target_content_width);
    } else {
        expand_column_widths(&mut sizing, target_content_width - preferred_content_width);
    }
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
            if let Some(ref url) = placement.cell.style.background_image_url {
                let tile = matches!(placement.cell.style.background_repeat, BackgroundRepeat::Repeat | BackgroundRepeat::RepeatX | BackgroundRepeat::RepeatY);
                let object_fit = if tile {
                    ObjectFit::None
                } else {
                    match placement.cell.style.background_size {
                        BackgroundSize::Cover => ObjectFit::Cover,
                        BackgroundSize::Contain => ObjectFit::Contain,
                        BackgroundSize::Auto => ObjectFit::Fill,
                    }
                };
                layer_commands.push(DrawCommand::Image(ImageCommand {
                    x: 0,
                    y: 0,
                    width: layer_w,
                    height: layer_h,
                    src: url.clone(),
                    object_fit,
                    object_position_x: placement.cell.style.background_position_x,
                    object_position_y: placement.cell.style.background_position_y,
                    tile,
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
                scale_x: placement.cell.style.transform_scale_x,
                scale_y: placement.cell.style.transform_scale_y,
                rotate_millideg: placement.cell.style.transform_rotate_millideg,
                origin_x: placement.cell.style.transform_origin_x,
                origin_y: placement.cell.style.transform_origin_y,
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
                native_chrome: ctrl.native_chrome,
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
            if let Some(ref url) = placement.cell.style.background_image_url {
                let tile = matches!(placement.cell.style.background_repeat, BackgroundRepeat::Repeat | BackgroundRepeat::RepeatX | BackgroundRepeat::RepeatY);
                let object_fit = if tile {
                    ObjectFit::None
                } else {
                    match placement.cell.style.background_size {
                        BackgroundSize::Cover => ObjectFit::Cover,
                        BackgroundSize::Contain => ObjectFit::Contain,
                        BackgroundSize::Auto => ObjectFit::Fill,
                    }
                };
                context.commands.push(DrawCommand::Image(ImageCommand {
                    x: cell_x,
                    y: cell_y,
                    width: cell_width.max(1),
                    height: cell_height.max(1),
                    src: url.clone(),
                    object_fit,
                    object_position_x: placement.cell.style.background_position_x,
                    object_position_y: placement.cell.style.background_position_y,
                    tile,
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
    *cursor_y = advance_by_margin(*cursor_y, element.style.margin.bottom);
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
    mins: Vec<u32>,
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
    let mut mins = vec![0_u32; column_count];
    let mut locked = vec![false; column_count];

    for placement in placements {
        if placement.colspan != 1 {
            continue;
        }

        let column = placement.column_index;
        let min_width = measure_cell_min_width(placement.cell, padding, images, fonts);
        mins[column] = mins[column].max(min_width);
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
        mins: mins.into_iter().map(|value| value.max(1)).collect(),
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

fn shrink_column_widths(sizing: &mut TableColumnSizing, overflow: u32) {
    if overflow == 0 || sizing.widths.is_empty() {
        return;
    }

    let remaining = shrink_column_widths_for_lock_state(sizing, overflow, false);
    if remaining > 0 {
        shrink_column_widths_for_lock_state(sizing, remaining, true);
    }
}

fn shrink_column_widths_for_lock_state(
    sizing: &mut TableColumnSizing,
    overflow: u32,
    locked: bool,
) -> u32 {
    if overflow == 0 {
        return 0;
    }

    let candidates = sizing
        .widths
        .iter()
        .enumerate()
        .filter(|(index, width)| sizing.locked[*index] == locked && **width > sizing.mins[*index])
        .map(|(index, width)| (index, width.saturating_sub(sizing.mins[index])))
        .collect::<Vec<_>>();
    let total_capacity = candidates
        .iter()
        .map(|(_, capacity)| *capacity)
        .sum::<u32>();
    if total_capacity == 0 {
        return overflow;
    }

    let target = overflow.min(total_capacity);
    let mut reductions = vec![0_u32; sizing.widths.len()];
    let mut applied = 0_u32;
    for (index, capacity) in &candidates {
        let reduce = ((*capacity as u64 * target as u64) / total_capacity as u64) as u32;
        reductions[*index] = reduce.min(*capacity);
        applied = applied.saturating_add(reductions[*index]);
    }

    let mut remainder = target.saturating_sub(applied);
    while remainder > 0 {
        let Some((index, _)) = candidates
            .iter()
            .filter_map(|(index, capacity)| {
                let spare = capacity.saturating_sub(reductions[*index]);
                (spare > 0).then_some((*index, spare))
            })
            .max_by_key(|(_, spare)| *spare)
        else {
            break;
        };
        reductions[index] = reductions[index].saturating_add(1);
        remainder -= 1;
    }

    for (index, reduction) in reductions.into_iter().enumerate() {
        if reduction > 0 {
            sizing.widths[index] = sizing.widths[index].saturating_sub(reduction);
        }
    }

    overflow.saturating_sub(target)
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

    layout_mixed_children(
        cell,
        0,
        width,
        &mut cursor_y,
        &mut context,
        None,
        images,
        fonts,
        current_form,
    );

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
            native_chrome: control.native_chrome,
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
            line_through: text.line_through,
            bold: text.bold,
            italic: text.italic,
            text_shadow: text.text_shadow.clone(),
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
            tile: image.tile,
        }),
        DrawCommand::Layer(layer) => DrawCommand::Layer(LayerCommand {
            x: layer.x.saturating_add(offset_x),
            y: layer.y.saturating_add(offset_y),
            width: layer.width,
            height: layer.height,
            opacity: layer.opacity,
            blur_px: layer.blur_px,
            brightness: layer.brightness,
            scale_x: layer.scale_x,
            scale_y: layer.scale_y,
            rotate_millideg: layer.rotate_millideg,
            origin_x: layer.origin_x,
            origin_y: layer.origin_y,
            commands: layer.commands.clone(),
        }),
        DrawCommand::Gradient(g) => DrawCommand::Gradient(GradientCommand {
            x: g.x.saturating_add(offset_x),
            y: g.y.saturating_add(offset_y),
            width: g.width,
            height: g.height,
            border_radius: g.border_radius,
            angle_deg_x1000: g.angle_deg_x1000,
            stops: g.stops.clone(),
        }),
        DrawCommand::Sticky(sticky) => DrawCommand::Sticky(StickyCommand {
            normal_y: sticky.normal_y.saturating_add(offset_y),
            sticky_top: sticky.sticky_top,
            container_bottom: sticky.container_bottom,
            layer: LayerCommand {
                x: sticky.layer.x.saturating_add(offset_x),
                y: sticky.layer.y.saturating_add(offset_y),
                width: sticky.layer.width,
                height: sticky.layer.height,
                opacity: sticky.layer.opacity,
                blur_px: sticky.layer.blur_px,
                brightness: sticky.layer.brightness,
                scale_x: sticky.layer.scale_x,
                scale_y: sticky.layer.scale_y,
                rotate_millideg: sticky.layer.rotate_millideg,
                origin_x: sticky.layer.origin_x,
                origin_y: sticky.layer.origin_y,
                commands: sticky.layer.commands.clone(),
            },
        }),
    }
}

fn span_width(widths: &[u32], start: usize, span: usize) -> u32 {
    widths.iter().skip(start).take(span).sum()
}

fn cell_span_height(heights: &[u32], start: usize, span: usize) -> u32 {
    heights.iter().skip(start).take(span).sum()
}

#[derive(Debug, Clone, Copy)]
struct ActiveFloat {
    side: FloatSide,
    x: u32,
    top: u32,
    bottom: u32,
    width: u32,
}

fn active_float_edges(active_floats: &[ActiveFloat], cursor_y: u32, x: u32, width: u32) -> (u32, u32) {
    let mut left_edge = x;
    let mut right_edge = x.saturating_add(width);
    for float in active_floats.iter().filter(|f| f.top <= cursor_y && cursor_y < f.bottom) {
        match float.side {
            FloatSide::Left => left_edge = left_edge.max(float.x.saturating_add(float.width)),
            FloatSide::Right => right_edge = right_edge.min(float.x),
            FloatSide::None => {}
        }
    }
    if right_edge <= left_edge {
        right_edge = left_edge.saturating_add(1);
    }
    (left_edge, right_edge)
}

fn clear_cursor_y(cursor_y: u32, clear: ClearSide, active_floats: &[ActiveFloat]) -> u32 {
    let mut target = cursor_y;
    for float in active_floats {
        let affects = match clear {
            ClearSide::Left => matches!(float.side, FloatSide::Left),
            ClearSide::Right => matches!(float.side, FloatSide::Right),
            ClearSide::Both => matches!(float.side, FloatSide::Left | FloatSide::Right),
            ClearSide::None => false,
        };
        if affects {
            target = target.max(float.bottom);
        }
    }
    target
}

fn max_float_bottom(active_floats: &[ActiveFloat]) -> u32 {
    active_floats.iter().map(|f| f.bottom).max().unwrap_or(0)
}

fn layout_mixed_children(
    element: &StyledElement,
    x: u32,
    width: u32,
    cursor_y: &mut u32,
    context: &mut LayoutContext,
    marker: Option<String>,
    images: &ImageStore,
    fonts: &mut FontContext,
    current_form: Option<FormContext>,
) {
    let mut inline_fragments = Vec::new();
    let mut bullet_pending = marker;
    let mut list_ordinal = 0_u32;
    let mut active_floats: Vec<ActiveFloat> = Vec::new();

    let flush_inline = |inline_fragments: &mut Vec<InlineFragment>,
                        bullet_pending: &mut Option<String>,
                        cursor_y: &mut u32,
                        context: &mut LayoutContext,
                        fonts: &mut FontContext,
                        x: u32,
                        width: u32,
                        element_style: &Arc<ComputedStyle>,
                        active_floats: &[ActiveFloat]| {
        if inline_fragments.is_empty() && bullet_pending.is_none() {
            return;
        }
        // A run of nothing but whitespace generates no line box -- CSS drops it.
        // Emitting one put a phantom line wherever a page sets an empty inline
        // between two blocks: MDN's header has three such gaps (an empty
        // `<mdn-search-modal>` and the newlines around it), and together they
        // made the sticky bar 71px taller than the 98px its own variables ask
        // for, so the nav sat over the article instead of above it.
        //
        // Under `white-space: pre` the spaces are content, so leave those alone.
        if bullet_pending.is_none()
            && element_style.white_space != WhiteSpaceMode::Pre
            && inline_fragments.iter().all(|fragment| {
                matches!(fragment, InlineFragment::Text { text, .. } if text.trim().is_empty())
            })
        {
            inline_fragments.clear();
            return;
        }
        if let Some(marker) = bullet_pending.take() {
            inline_fragments.insert(
                0,
                InlineFragment::Text {
                    text: marker,
                    style: element_style.clone(),
                    link_href: None,
                    link_node_id: None,
                },
            );
        }
        let (avail_x, avail_right) = active_float_edges(active_floats, *cursor_y, x, width);
        layout_inline_fragments(
            inline_fragments,
            element_style,
            avail_x,
            avail_right.saturating_sub(avail_x).max(1),
            cursor_y,
            context,
            fonts,
        );
        inline_fragments.clear();
        *bullet_pending = None;
    };

    // See through `display: contents`. This is the block container's own child
    // walk, separate from `layout_node`, and it was the one path the original
    // `display: contents` support missed -- MDN's article pages put their whole
    // body under `<main class="layout__content">`, which is `display: contents`,
    // so every docs page laid out its header and footer and nothing in between.
    for child in formatting_context_children(element) {
        if is_hidden(child) {
            continue;
        }

        let child_style = match child {
            StyledNode::Element(element) => Some(&element.style),
            _ => None,
        };
        let child_float = child_style.map(|s| s.float).unwrap_or(FloatSide::None);
        let child_clear = child_style.map(|s| s.clear).unwrap_or(ClearSide::None);
        let child_is_block = is_block_level(child);

        if child_is_block && child_float != FloatSide::None {
            flush_inline(
                &mut inline_fragments,
                &mut bullet_pending,
                cursor_y,
                context,
                fonts,
                x,
                width,
                &element.style,
                &active_floats,
            );

            let Some(style) = child_style else {
                layout_node(child, x, width, cursor_y, context, images, fonts, current_form.clone());
                continue;
            };

            let fw = match style.width {
                Some(LengthValue::Pixels(px)) => px.min(width).max(1),
                Some(LengthValue::Percent(pct)) => (width as u64 * pct as u64 / 100).min(width as u64) as u32,
                Some(LengthValue::MinContent) | Some(LengthValue::MaxContent) | Some(LengthValue::FitContent(_)) => width.max(1),
                Some(LengthValue::Calc { percent_hundredths, px }) => crate::css::resolve_calc(percent_hundredths, px, width).max(1),
                None => {
                    if matches!(child, StyledNode::Element(StyledElement { tag_name, .. }) if tag_name == "img") {
                        width.max(1)
                    } else {
                        layout_node(child, x, width, cursor_y, context, images, fonts, current_form.clone());
                        continue;
                    }
                }
            }
            .min(width.max(1));

            let mut top = *cursor_y;
            loop {
                let (left_edge, right_edge) = active_float_edges(&active_floats, top, x, width);
                let slot_width = right_edge.saturating_sub(left_edge);
                if slot_width >= fw.max(1) {
                    let fx = match style.float {
                        FloatSide::Left => left_edge,
                        FloatSide::Right => right_edge.saturating_sub(fw.max(1)),
                        FloatSide::None => left_edge,
                    };
                    let mut f_y = top;
                    layout_node(
                        child,
                        fx,
                        fw.max(1),
                        &mut f_y,
                        context,
                        images,
                        fonts,
                        current_form.clone(),
                    );
                    let bottom = f_y.max(top + 1);
                    active_floats.push(ActiveFloat {
                        side: style.float,
                        x: fx,
                        top,
                        bottom,
                        width: fw.max(1),
                    });
                    break;
                }
                top = active_floats
                    .iter()
                    .filter(|f| f.top <= top && top < f.bottom)
                    .map(|f| f.bottom)
                    .max()
                    .unwrap_or_else(|| top.saturating_add(1));
            }
            continue;
        }

        if child_is_block {
            flush_inline(
                &mut inline_fragments,
                &mut bullet_pending,
                cursor_y,
                context,
                fonts,
                x,
                width,
                &element.style,
                &active_floats,
            );
            *cursor_y = clear_cursor_y(*cursor_y, child_clear, &active_floats);
            let (avail_x, avail_right) = active_float_edges(&active_floats, *cursor_y, x, width);
            // Number this container's list items so `list-style-type: decimal`
            // has an ordinal to render. Counting here keeps nested lists
            // independent: each container runs its own pass over its children.
            context.list_ordinal = if matches!(
                child,
                StyledNode::Element(child) if child.style.display == Display::ListItem
            ) {
                list_ordinal = list_ordinal.saturating_add(1);
                Some(list_ordinal)
            } else {
                None
            };
            layout_node(
                child,
                avail_x,
                avail_right.saturating_sub(avail_x).max(1),
                cursor_y,
                context,
                images,
                fonts,
                current_form.clone(),
            );
            context.list_ordinal = None;
        } else {
            if let Some(marker) = bullet_pending.take() {
                inline_fragments.push(InlineFragment::Text {
                    text: marker,
                    style: element.style.clone(),
                    link_href: None,
                    link_node_id: None,
                });
            }
            collect_inline_fragments(
                child,
                &mut inline_fragments,
                None,
                None,
                current_form.clone(),
                context,
                images,
                fonts,
                width,
            );
        }
    }

    flush_inline(
        &mut inline_fragments,
        &mut bullet_pending,
        cursor_y,
        context,
        fonts,
        x,
        width,
        &element.style,
        &active_floats,
    );
    *cursor_y = (*cursor_y).max(max_float_bottom(&active_floats));
}

#[allow(clippy::too_many_arguments)]
fn collect_inline_fragments(
    node: &StyledNode,
    output: &mut Vec<InlineFragment>,
    link_href: Option<&str>,
    link_node_id: Option<usize>,
    current_form: Option<FormContext>,
    context: &mut LayoutContext,
    images: &ImageStore,
    fonts: &mut FontContext,
    available_width: u32,
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

            // An out-of-flow box takes no part in the inline flow, but it does
            // have to be laid out. The arms below drop every block-level box
            // inside an inline formatting context, and blockification makes
            // every absolutely positioned box block-level -- so a positioned
            // element nested in an inline one vanished from the page entirely.
            // (An *in-flow* block inside an inline box is still skipped;
            // splitting the inline around it is a separate feature.)
            if matches!(element.style.position, Position::Absolute | Position::Fixed)
                && element.style.display != Display::None
            {
                let mut cursor_y = 0;
                layout_positioned_element(
                    element,
                    0,
                    available_width,
                    &mut cursor_y,
                    context,
                    images,
                    fonts,
                    current_form.clone(),
                );
                return;
            }

            match element.style.display {
                Display::None => {}
                // Transparent wrapper: flatten straight through to the children.
                Display::Contents => {
                    for child in &element.children {
                        collect_inline_fragments(
                            child,
                            output,
                            link_href,
                            link_node_id,
                            current_form.clone(),
                            context,
                            images,
                            fonts,
                            available_width,
                        );
                    }
                }
                Display::Inline => {
                    if element.tag_name == "br" {
                        output.push(InlineFragment::LineBreak);
                        return;
                    }

                    if let Some(control) =
                        build_form_control_spec(element, current_form.as_ref(), context)
                    {
                        output.push(InlineFragment::Control(Box::new(control)));
                        return;
                    }

                    if element.tag_name == "img" {
                        if let Some(src) = resolved_image_source(element) {
                            if let Some(image) = images.get(src) {
                                let (draw_width, draw_height) =
                                    image_dimensions(element, image.width, image.height, available_width.max(1));
                                output.push(InlineFragment::Image {
                                    src: src.to_string(),
                                    draw_width,
                                    draw_height,
                                    style: element.style.clone(),
                                    link_href: current_link.map(str::to_string),
                                    link_node_id: current_link_node_id,
                                });
                                return;
                            }
                        }

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
                            images,
                            fonts,
                            available_width,
                        );
                    }
                }
                Display::InlineBlock => {
                    if let Some(atomic) = layout_atomic_inline(
                        element,
                        available_width,
                        context,
                        images,
                        fonts,
                        current_form,
                    ) {
                        output.push(InlineFragment::Atomic(Box::new(atomic)));
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
    images: &ImageStore,
    fonts: &mut FontContext,
    available_width: u32,
) -> Vec<InlineFragment> {
    let mut fragments = Vec::new();
    collect_inline_fragments(
        node,
        &mut fragments,
        None,
        None,
        current_form,
        context,
        images,
        fonts,
        available_width,
    );
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
    } else if container_style.white_space == WhiteSpaceMode::NoWrap {
        layout_nowrap_fragments(
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

fn layout_nowrap_fragments(
    fragments: &[InlineFragment],
    container_style: &ComputedStyle,
    x: u32,
    width: u32,
    cursor_y: &mut u32,
    context: &mut LayoutContext,
    fonts: &mut FontContext,
) {
    let mut line = LineBuilder::default();
    let text_indent = container_style.text_indent;
    let mut pending_space = false;

    for fragment in fragments {
        match fragment {
            InlineFragment::Atomic(atomic) => {
                line.push_atomic(atomic.clone(), container_style);
                pending_space = true;
            }
            InlineFragment::LineBreak => {
                // nowrap: ignore line breaks
            }
            InlineFragment::Control(control) => {
                if pending_space && !line.is_empty() {
                    line.push_span(" ", &control.style, fonts, None, None);
                }
                line.push_control(control, fonts);
                pending_space = true;
            }
            InlineFragment::Image {
                src,
                draw_width,
                draw_height,
                style,
                link_href,
                link_node_id,
            } => {
                if pending_space && !line.is_empty() {
                    line.push_span(" ", style, fonts, link_href.as_deref(), *link_node_id);
                }
                line.push_image(
                    src,
                    *draw_width,
                    *draw_height,
                    style,
                    link_href.as_deref(),
                    *link_node_id,
                );
                pending_space = true;
            }
            InlineFragment::Text { text, style, link_href, link_node_id } => {
                let starts_with_whitespace = text.chars().next().map(char::is_whitespace).unwrap_or(false);
                let ends_with_whitespace = text.chars().last().map(char::is_whitespace).unwrap_or(false);
                let words: Vec<&str> = text.split_whitespace().collect();
                let mut needs_space = pending_space || starts_with_whitespace;
                for word in words {
                    if needs_space && !line.is_empty() {
                        line.push_span(" ", style, fonts, link_href.as_deref(), *link_node_id);
                    }
                    line.push_span(word, style, fonts, link_href.as_deref(), *link_node_id);
                    needs_space = true;
                }
                pending_space = ends_with_whitespace || (text.chars().any(char::is_whitespace) && line.is_empty());
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
        // nowrap emits a single line, so the indent always applies
        text_indent,
    );
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
    let ellipsis_mode = container_style.text_overflow_ellipsis && container_style.overflow == Overflow::Hidden;
    let mut line = LineBuilder::default();
    let mut pending_space = false;
    let text_indent = container_style.text_indent;
    let mut first_line = true;
    let mut ellipsis_done = false; // in ellipsis mode, once we clip, we're done

    'outer: for fragment in fragments {
        if ellipsis_mode && ellipsis_done {
            break;
        }
        match fragment {
            InlineFragment::Atomic(atomic) => {
                line.push_atomic(atomic.clone(), container_style);
            }
            InlineFragment::LineBreak => {
                if ellipsis_mode {
                    // In ellipsis mode, ignore line breaks
                } else {
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
            }
            InlineFragment::Control(control) => {
                let (control_width, _) = measure_form_control(control, fonts);
                let effective_width = if first_line && line.is_empty() {
                    width_after_indent(width, text_indent)
                } else {
                    width
                };

                let pending_space_before_control = pending_space && !line.is_empty();
                if pending_space_before_control {
                    let space_width = char_width(&control.style, ' ', fonts);
                    if line.width.saturating_add(space_width) > effective_width {
                        if ellipsis_mode {
                            // Apply ellipsis and stop
                            apply_ellipsis_to_line(&mut line, effective_width, container_style, fonts);
                            ellipsis_done = true;
                            break 'outer;
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
                        first_line = false;
                    } else {
                        line.push_span(" ", &control.style, fonts, None, None);
                    }
                }

                let effective_width = if first_line && line.is_empty() {
                    width_after_indent(width, text_indent)
                } else {
                    width
                };
                if !line.is_empty() && line.width.saturating_add(control_width) > effective_width {
                    if ellipsis_mode {
                        apply_ellipsis_to_line(&mut line, effective_width, container_style, fonts);
                        ellipsis_done = true;
                        break 'outer;
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
                    first_line = false;
                }
                line.push_control(control, fonts);
                pending_space = true;
            }
            InlineFragment::Image {
                src,
                draw_width,
                draw_height,
                style,
                link_href,
                link_node_id,
            } => {
                let effective_width = if first_line && line.is_empty() {
                    width_after_indent(width, text_indent)
                } else {
                    width
                };

                if pending_space && !line.is_empty() {
                    let space_width = char_width(style, ' ', fonts);
                    if line.width.saturating_add(space_width) > effective_width {
                        if ellipsis_mode {
                            apply_ellipsis_to_line(&mut line, effective_width, container_style, fonts);
                            ellipsis_done = true;
                            break 'outer;
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
                        first_line = false;
                    } else {
                        line.push_span(" ", style, fonts, link_href.as_deref(), *link_node_id);
                    }
                }

                let effective_width = if first_line && line.is_empty() {
                    width_after_indent(width, text_indent)
                } else {
                    width
                };
                if !line.is_empty() && line.width.saturating_add(*draw_width) > effective_width {
                    if ellipsis_mode {
                        apply_ellipsis_to_line(&mut line, effective_width, container_style, fonts);
                        ellipsis_done = true;
                        break 'outer;
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
                    first_line = false;
                }
                line.push_image(
                    src,
                    *draw_width,
                    *draw_height,
                    style,
                    link_href.as_deref(),
                    *link_node_id,
                );
                pending_space = true;
            }
            InlineFragment::Text {
                text,
                style,
                link_href,
                link_node_id,
            } => {
                let starts_with_whitespace = text.chars().next().map(char::is_whitespace).unwrap_or(false);
                let ends_with_whitespace = text.chars().last().map(char::is_whitespace).unwrap_or(false);
                let words: Vec<&str> = text.split_whitespace().collect();
                let mut needs_space = pending_space || starts_with_whitespace;

                for word in words {
                    if ellipsis_mode && ellipsis_done {
                        break;
                    }
                    let effective_width = if first_line && line.is_empty() {
                        width_after_indent(width, text_indent)
                    } else {
                        width
                    };

                    if needs_space && !line.is_empty() {
                        let space_width = char_width(style, ' ', fonts);
                        if line.width.saturating_add(space_width) > effective_width {
                            if ellipsis_mode {
                                apply_ellipsis_to_line(&mut line, effective_width, container_style, fonts);
                                ellipsis_done = true;
                                break;
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
                            first_line = false;
                        } else {
                            line.push_span(" ", style, fonts, link_href.as_deref(), *link_node_id);
                        }
                    }

                    if ellipsis_done { break; }

                    let effective_width2 = if first_line && line.is_empty() {
                        width_after_indent(width, text_indent)
                    } else {
                        width
                    };

                    if ellipsis_mode {
                        // Check if word fits; if not, apply ellipsis
                        let word_width = text_width(style, word, fonts);
                        let ellipsis_width = text_width(style, "...", fonts);
                        if line.width.saturating_add(word_width) > effective_width2 {
                            // Word doesn't fit - apply ellipsis to current line
                            apply_ellipsis_to_line(&mut line, effective_width2, container_style, fonts);
                            ellipsis_done = true;
                            break;
                        } else if line.width.saturating_add(word_width).saturating_add(ellipsis_width) > effective_width2 {
                            // Word fits but we can't guarantee another word will fit - add it
                            line.push_span(word, style, fonts, link_href.as_deref(), *link_node_id);
                            needs_space = true;
                        } else {
                            line.push_span(word, style, fonts, link_href.as_deref(), *link_node_id);
                            needs_space = true;
                        }
                    } else {
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
                        needs_space = true;
                    }
                }

                pending_space = ends_with_whitespace || (text.chars().any(char::is_whitespace) && line.is_empty());
            }
        }
    }

    if ellipsis_mode && !ellipsis_done && !line.is_empty() {
        let eff_width = width_after_indent(width, text_indent);
        apply_ellipsis_to_line(&mut line, eff_width, container_style, fonts);
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

/// Truncate the last span in `line` so total width fits within `max_width`, then append "...".
fn apply_ellipsis_to_line(
    line: &mut LineBuilder,
    max_width: u32,
    container_style: &ComputedStyle,
    fonts: &mut FontContext,
) {
    let ellipsis = "...";
    // Find a style to use for ellipsis (last span or container style)
    let ellipsis_style = line
        .spans
        .last()
        .map(|s| Arc::clone(&s.style))
        .unwrap_or_else(|| Arc::new(container_style.clone()));
    let ellipsis_width = text_width(&ellipsis_style, ellipsis, fonts);
    let target = max_width.saturating_sub(ellipsis_width);

    // Trim spans to fit within `target` width
    let mut used = 0u32;
    for span in &mut line.spans {
        if used >= target {
            span.text.clear();
            span.width = 0;
        } else {
            let available = target.saturating_sub(used);
            if span.width <= available {
                used = used.saturating_add(span.width);
            } else {
                // Truncate this span
                let mut truncated_text = String::new();
                let mut tw = 0u32;
                for ch in span.text.chars() {
                    let cw = fonts.glyph_advance_px(ch, span.style.font_size_px, span.style.font_family);
                    if tw.saturating_add(cw) > available {
                        break;
                    }
                    tw += cw;
                    truncated_text.push(ch);
                }
                span.text = truncated_text;
                span.width = tw;
                used = used.saturating_add(tw);
            }
        }
    }
    // Remove empty trailing spans
    line.spans
        .retain(|s| !s.text.is_empty() || s.control.is_some() || s.image.is_some());
    // Append ellipsis as a new span
    let ellipsis_span = LineSpan {
        text: ellipsis.to_string(),
        width: ellipsis_width,
        height: text_line_height(&ellipsis_style, fonts),
        style: ellipsis_style,
        link_href: None,
        link_node_id: None,
        control: None,
        image: None,
        atomic: None,
    };
    line.spans.push(ellipsis_span);
    // Recompute line width
    line.width = line.spans.iter().map(|s| s.width).sum();
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
            InlineFragment::Atomic(atomic) => {
                // An atomic inline never splits, so it moves to the next line
                // whole rather than overflowing the current one.
                if !line.is_empty() && line.width.saturating_add(atomic.width) > width {
                    emit_line_with_indent(
                        &mut line, container_style, x, width, cursor_y, context, fonts,
                        if first_line { text_indent } else { 0 },
                    );
                    first_line = false;
                }
                line.push_atomic(atomic.clone(), container_style);
            }
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
                    width_after_indent(width, text_indent)
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
            InlineFragment::Image {
                src,
                draw_width,
                draw_height,
                style,
                link_href,
                link_node_id,
            } => {
                line.push_image(
                    src,
                    *draw_width,
                    *draw_height,
                    style,
                    link_href.as_deref(),
                    *link_node_id,
                );
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
                    let eff_w = if first_line { width_after_indent(width, text_indent) } else { width };
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
    style: &Arc<ComputedStyle>,
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

/// The width left for a line once the indent has eaten into it. A negative
/// indent widens it: the line starts left of the content edge but still ends at
/// the right one.
fn width_after_indent(width: u32, indent: i32) -> u32 {
    (i64::from(width) - i64::from(indent)).clamp(0, i64::from(u32::MAX)) as u32
}

fn emit_line_with_indent(
    line: &mut LineBuilder,
    container_style: &ComputedStyle,
    x: u32,
    width: u32,
    cursor_y: &mut u32,
    context: &mut LayoutContext,
    fonts: &mut FontContext,
    indent: i32,
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
    indent: i32,
) {
    if line.is_empty() {
        *cursor_y = cursor_y.saturating_add(text_line_height(container_style, fonts));
        return;
    }

    // A negative indent starts the line left of the content edge. The painter
    // works in unsigned coordinates, and a line pushed clear off the left of the
    // canvas is invisible in any case, so drop its content and keep only the
    // height its line box occupies.
    let line_start = i64::from(x) + i64::from(indent);
    let effective_width = width_after_indent(width, indent);
    let line_width = line.width.min(effective_width);
    if line_start + i64::from(line_width) <= 0 {
        *cursor_y = cursor_y.saturating_add(
            line.line_height
                .max(text_line_height(container_style, fonts)),
        );
        line.spans.clear();
        line.width = 0;
        line.line_height = 0;
        return;
    }
    let effective_x = line_start.max(0) as u32;
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
            let (background_color, border_color, native_chrome) = control_colors(control);
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
                native_chrome,
            });

            cursor_x = cursor_x.saturating_add(span.width);
            continue;
        }

        if let Some(atomic) = &span.atomic {
            // The box was laid out from its own origin, so shift the whole
            // result to where the line breaker put it.
            let box_y = cursor_y.saturating_add(line_height.saturating_sub(span.height));
            let mut commands = atomic.commands.clone();
            offset_commands(&mut commands, cursor_x, box_y);
            context.commands.append(&mut commands);
            for mut link in atomic.links.iter().cloned() {
                link.x = link.x.saturating_add(cursor_x);
                link.y = link.y.saturating_add(box_y);
                context.links.push(link);
            }
            for mut control in atomic.controls.iter().cloned() {
                control.x = control.x.saturating_add(cursor_x);
                control.y = control.y.saturating_add(box_y);
                context.controls.push(control);
            }
            for mut hitbox in atomic.hitboxes.iter().cloned() {
                hitbox.x = hitbox.x.saturating_add(cursor_x);
                hitbox.y = hitbox.y.saturating_add(box_y);
                context.element_hitboxes.push(hitbox);
            }
            cursor_x = cursor_x.saturating_add(span.width);
            continue;
        }

        if let Some(image) = &span.image {
            let image_y = cursor_y.saturating_add(line_height.saturating_sub(span.height));
            if image.style.opacity < 255
                || image.style.filter_blur_px > 0
                || image.style.filter_brightness != 10000
            {
                let img_cmd = DrawCommand::Image(ImageCommand {
                    x: 0,
                    y: 0,
                    width: image.draw_width,
                    height: image.draw_height,
                    src: image.src.clone(),
                    object_fit: image.style.object_fit,
                    object_position_x: image.style.object_position_x,
                    object_position_y: image.style.object_position_y,
                    tile: false,
                });
                context.commands.push(DrawCommand::Layer(LayerCommand {
                    x: cursor_x,
                    y: image_y,
                    width: image.draw_width,
                    height: image.draw_height,
                    opacity: image.style.opacity,
                    blur_px: image.style.filter_blur_px,
                    brightness: image.style.filter_brightness,
                    scale_x: image.style.transform_scale_x,
                    scale_y: image.style.transform_scale_y,
                    rotate_millideg: image.style.transform_rotate_millideg,
                    origin_x: image.style.transform_origin_x,
                    origin_y: image.style.transform_origin_y,
                    commands: vec![img_cmd],
                }));
            } else {
                context.commands.push(DrawCommand::Image(ImageCommand {
                    x: cursor_x,
                    y: image_y,
                    width: image.draw_width,
                    height: image.draw_height,
                    src: image.src.clone(),
                    object_fit: image.style.object_fit,
                    object_position_x: image.style.object_position_x,
                    object_position_y: image.style.object_position_y,
                    tile: false,
                }));
            }

            if let Some(href) = &image.link_href {
                if !image.style.pointer_events_none {
                    context.links.push(LinkCommand {
                        node_id: image.link_node_id,
                        x: cursor_x,
                        y: image_y,
                        width: image.draw_width,
                        height: image.draw_height,
                        href: href.clone(),
                    });
                }
            }

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
            line_through: span.style.line_through,
            bold: span.style.font_weight,
            italic: span.style.font_style_italic,
            text_shadow: span.style.text_shadow.clone(),
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

    match node {
        StyledNode::Element(element) => matches!(
            element.style.display,
            Display::Block
                | Display::ListItem
                | Display::Flex
                | Display::InlineFlex
                | Display::Grid
                | Display::InlineGrid
        ),
        StyledNode::Text(_) => false,
    }
}

fn is_hidden(node: &StyledNode) -> bool {
    match node {
        StyledNode::Element(element) => element.style.display == Display::None,
        StyledNode::Text(_) => false,
    }
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

    // A flex row lays its items out side by side however each item declares
    // itself, so a block-level child does not start a new line there. Treating
    // it as one measured a row by its widest item instead of by their sum:
    // rust-lang.org's eight navigation links came to 149px between them and the
    // list wrapped to one link a line.
    let in_a_row = lays_children_out_in_a_row(&cell.style);
    let mut max_width = 1_u32;
    let mut inline_width = 0_u32;
    for child in &cell.children {
        if is_hidden(child) {
            continue;
        }

        if matches!(child, StyledNode::Element(StyledElement { tag_name, .. }) if tag_name == "br") {
            max_width = max_width.max(inline_width);
            inline_width = 0;
            continue;
        }

        let child_width = measure_node_preferred_width(child, images, fonts);
        if is_block_level(child) && !in_a_row {
            max_width = max_width.max(inline_width).max(child_width);
            inline_width = 0;
        } else {
            // A row holds its items apart by `gap`, so that space is part of
            // what the row needs. Left out, the box came up exactly the gaps
            // short and its contents wrapped inside a box measured to fit them:
            // firefox.com's header menu titles are a label and a chevron 4px
            // apart, and every one of them broke over two lines.
            if in_a_row && inline_width > 0 {
                inline_width = inline_width.saturating_add(cell.style.gap);
            }
            inline_width = inline_width.saturating_add(child_width);
        }
    }
    max_width = max_width.max(inline_width);

    max_width.saturating_add(padding.saturating_mul(2))
}

fn measure_cell_min_width(
    cell: &StyledElement,
    padding: u32,
    images: &ImageStore,
    fonts: &mut FontContext,
) -> u32 {
    let mut max_width = 1_u32;
    for child in &cell.children {
        if is_hidden(child) {
            continue;
        }

        max_width = max_width.max(measure_node_min_width(child, images, fonts));
    }

    max_width.saturating_add(padding.saturating_mul(2)).max(1)
}

/// Move the boxes anchored to this block's bottom edge, now that its height is
/// known. Anything recorded before `mark` belongs to an outer block.
fn settle_bottom_anchored(
    context: &mut LayoutContext,
    mark: usize,
    block_top: u32,
    block_height: u32,
) {
    if context.pending_bottom.len() <= mark {
        return;
    }
    let pending: Vec<PendingBottom> = context.pending_bottom.drain(mark..).collect();
    for box_ in pending {
        let offset = resolve_offset(box_.offset, block_height);
        let target = (block_top as i64 + block_height as i64)
            .saturating_sub(offset as i64)
            .saturating_sub(box_.height as i64)
            .max(0) as u32;
        let dy = target as i64 - box_.drawn_top as i64;
        if dy == 0 {
            continue;
        }
        if let Some((_, commands)) = context.positioned_commands.get_mut(box_.slot) {
            for command in commands.iter_mut() {
                shift_command_signed(command, 0, dy as i32);
            }
        }
        let shift = |value: &mut u32| {
            *value = (*value as i64 + dy).max(0) as u32;
        };
        for link in context.links.iter_mut().skip(box_.links_from) {
            shift(&mut link.y);
        }
        for control in context.controls.iter_mut().skip(box_.controls_from) {
            shift(&mut control.y);
        }
        for hitbox in context.element_hitboxes.iter_mut().skip(box_.hitboxes_from) {
            shift(&mut hitbox.y);
        }
    }
}

/// The height a box states outright, if it states one in pixels.
///
/// A percentage `top` needs a definite containing-block height; when no
/// ancestor gives one there is nothing honest to resolve against, so callers
/// fall back to zero rather than guessing.
fn definite_height(style: &ComputedStyle) -> u32 {
    match style.height {
        Some(LengthValue::Pixels(px)) => px,
        _ => 0,
    }
}

/// Whether this box puts all of its children on one axis, so that its
/// max-content width is the sum of theirs rather than the widest of them.
fn lays_children_out_in_a_row(style: &ComputedStyle) -> bool {
    match style.display {
        Display::Inline | Display::InlineBlock => true,
        Display::Flex | Display::InlineFlex => matches!(
            style.flex_direction,
            FlexDirection::Row | FlexDirection::RowReverse
        ),
        _ => false,
    }
}

fn measure_node_preferred_width(
    node: &StyledNode,
    images: &ImageStore,
    fonts: &mut FontContext,
) -> u32 {
    match node {
        StyledNode::Text(text) => text_width(&text.style, text.text.trim(), fonts).max(1),
        StyledNode::Element(element) => {
            // A box that states its own width is that wide whatever is inside
            // it -- including when nothing is. An icon is an empty span sized by
            // CSS and painted with a background image; measuring only its
            // children reported one pixel, so every icon collapsed and the label
            // beside it wrapped for want of the space the icon should have held.
            if let Some(LengthValue::Pixels(width)) = element.style.width {
                let extra = if element.style.box_sizing == BoxSizing::BorderBox {
                    0
                } else {
                    element.style.padding.left + element.style.padding.right
                };
                return width.saturating_add(extra).max(1);
            }

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
                        LengthValue::Calc { percent_hundredths, px } => crate::css::resolve_calc(percent_hundredths, px, u32::MAX / 2),
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

            let child_width = if lays_children_out_in_a_row(&element.style) {
                // Skip `display: none`, the same as the block branch below does.
                // Summing hidden children counted MDN's mega-menu panel -- a
                // hidden 600px block -- into the width of the little nav tab that
                // owns it, so every tab measured 721px instead of about 80. The
                // flex row then overflowed by fivefold and shrank every item
                // proportionally, which crushed the one tab that had measured
                // correctly down to a single character per line.
                // A row holds its items apart by `gap`, so that space counts
                // towards what the row needs. Left out, the box came up exactly
                // the gaps short and its own contents wrapped inside a box
                // measured to fit them: firefox.com's header menu titles are a
                // label and a chevron 4px apart, and every one broke over two
                // lines, taking the header from 68px to 117px tall.
                {
                    let visible: Vec<u32> = element
                        .children
                        .iter()
                        .filter(|child| !is_hidden(child))
                        .map(|child| measure_node_preferred_width(child, images, fonts))
                        .collect();
                    let gaps = element
                        .style
                        .gap
                        .saturating_mul(visible.len().saturating_sub(1) as u32);
                    visible.iter().fold(gaps, |total, width| total.saturating_add(*width))
                }
                    .max(1)
            } else {
                // A block box breaks a line only at a block-level child, so its
                // max-content width is the widest *line*, not the widest child.
                // Taking a plain maximum reported the widest word instead: a box
                // holding `38` and `℃` measured 10px, and shrink-to-fit sizing
                // then laid its contents out one character to a line.
                let mut widest = 1_u32;
                let mut line = 0_u32;
                for child in &element.children {
                    if is_hidden(child) {
                        continue;
                    }
                    if matches!(
                        child,
                        StyledNode::Element(StyledElement { tag_name, .. }) if tag_name == "br"
                    ) {
                        widest = widest.max(line);
                        line = 0;
                        continue;
                    }
                    let width = measure_node_preferred_width(child, images, fonts);
                    if is_block_level(child) {
                        widest = widest.max(line).max(width);
                        line = 0;
                    } else {
                        line = line.saturating_add(width);
                    }
                }
                widest.max(line)
            };

            // `min-width` is a floor on the box, so it is a floor on the
            // width it reports as well.
            let minimum = match element.style.min_width {
                Some(LengthValue::Pixels(px)) => px,
                _ => 0,
            };
            // The box takes up its borders and margins too. Reporting only the
            // content plus padding left each item a few pixels short, and a row
            // of them added up to enough that the row no longer fitted: Yahoo!
            // JAPAN's top-right navigation separates its items with a 1px rule
            // and an 8px margin, and lost 27px across four of them.
            let border = if element.style.border_style_none {
                0
            } else {
                element.style.border.left + element.style.border.right
            };
            let margins =
                element.style.margin.left.max(0) as u32 + element.style.margin.right.max(0) as u32;
            child_width
                .saturating_add(element.style.padding.left + element.style.padding.right)
                .max(minimum)
                .saturating_add(border)
                .saturating_add(margins)
                .max(1)
        }
    }
}

fn measure_node_min_width(
    node: &StyledNode,
    images: &ImageStore,
    fonts: &mut FontContext,
) -> u32 {
    match node {
        StyledNode::Text(text) => text
            .text
            .chars()
            .find(|ch| !ch.is_whitespace())
            .map(|ch| char_width(&text.style, ch, fonts))
            .unwrap_or(1)
            .max(1),
        StyledNode::Element(element) => {
            if element.tag_name == "img"
                && let Some(src) = resolved_image_source(element)
                && let Some(image) = images.get(src)
            {
                return image_dimensions(element, image.width, image.height, u32::MAX / 2).0;
            }

            let mut form_context = LayoutContext::default();
            if let Some(spec) = build_form_control_spec(element, None, &mut form_context) {
                return measure_form_control(&spec, fonts).0.max(1);
            }

            if element.tag_name == "table" {
                return specified_length(element, element.style.width, "width")
                    .and_then(|length| match length {
                        LengthValue::Pixels(value) => Some(value),
                        _ => None,
                    })
                    .unwrap_or_else(|| {
                        let rows = collect_table_rows(element);
                        let placements = build_table_placements(&rows);
                        let spacing =
                            parse_dimension_attribute(element.attributes.get("cellspacing"))
                                .unwrap_or(0);
                        let padding =
                            parse_dimension_attribute(element.attributes.get("cellpadding"))
                                .unwrap_or(0);
                        let sizing = compute_column_widths(
                            element,
                            &placements,
                            u32::MAX / 2,
                            padding,
                            images,
                            fonts,
                        );
                        sizing
                            .mins
                            .iter()
                            .sum::<u32>()
                            .saturating_add(
                                spacing.saturating_mul(sizing.mins.len().saturating_sub(1) as u32),
                            )
                            .max(1)
                    });
            }

            let child_width = if element.style.display == Display::Inline {
                element
                    .children
                    .iter()
                    .map(|child| measure_node_min_width(child, images, fonts))
                    .max()
                    .unwrap_or(1)
            } else {
                element
                    .children
                    .iter()
                    .map(|child| measure_node_min_width(child, images, fonts))
                    .max()
                    .unwrap_or(1)
            };

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

/// Resolve a box offset against its containing block.
///
/// Signed, unlike a width: `left: -20px` and `top: -1px` are everyday values,
/// and so is `left: 50%`, whose percentage is of the containing block and not
/// of the font size.
fn resolve_offset(length: LengthValue, basis: u32) -> i32 {
    match length {
        LengthValue::Pixels(px) => px.min(i32::MAX as u32) as i32,
        LengthValue::Percent(percent) => ((basis as i64 * percent as i64) / 100) as i32,
        LengthValue::Calc { percent_hundredths, px } => {
            ((basis as i64 * percent_hundredths as i64) / 10_000 + px as i64)
                .clamp(i32::MIN as i64, i32::MAX as i64) as i32
        }
        LengthValue::MinContent => 0,
        LengthValue::MaxContent => basis.min(i32::MAX as u32) as i32,
        LengthValue::FitContent(px) => basis.min(px).min(i32::MAX as u32) as i32,
    }
}

fn resolve_length_value(length: LengthValue, available_width: u32) -> u32 {
    match length {
        LengthValue::Pixels(value) => value,
        LengthValue::Percent(value) => available_width.saturating_mul(value) / 100,
        LengthValue::MinContent => 0,
        LengthValue::MaxContent => available_width,
        LengthValue::FitContent(max_px) => available_width.min(max_px),
        LengthValue::Calc { percent_hundredths, px } => crate::css::resolve_calc(percent_hundredths, px, available_width),
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
    static_x: u32,
    container_width: u32,
    static_y: &mut u32,
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
    // With `top`/`left` auto the box keeps its *static position* -- where it
    // would have sat in flow -- rather than jumping to the containing block's
    // corner. Yahoo! JAPAN marks each headline with an absolutely positioned
    // `::before` dot that sets only `left`, so pinning it to the top of the
    // article drew the dot a line above the headline it belongs to.
    let static_y = *static_y;

    let border_x = if element.style.border_style_none {
        0
    } else {
        element.style.border.left + element.style.border.right
    };
    let surround = element.style.padding.left + element.style.padding.right + border_x;

    let specified_width = element.style.width.as_ref().map(|lv| match lv {
        LengthValue::Pixels(px) => *px,
        LengthValue::Percent(p) => (container_width as f32 * (*p as f32) / 100.0) as u32,
        LengthValue::MinContent => 0,
        LengthValue::MaxContent => container_width,
        LengthValue::FitContent(max_px) => container_width.min(*max_px),
        LengthValue::Calc { percent_hundredths, px } => crate::css::resolve_calc(*percent_hundredths, *px, container_width),
    });

    // `left` / `right` resolve against the containing block's width, `top` /
    // `bottom` against its height. The height is only known when an ancestor
    // states one, so a percentage `top` falls back to zero.
    let (cb_width, cb_height) = {
        let (width, height) = context.containing_block_size;
        (if width == 0 { container_width } else { width }, height)
    };
    let left = element.style.left.map(|length| resolve_offset(length, cb_width));
    let right = element.style.right.map(|length| resolve_offset(length, cb_width));
    let top = element.style.top.map(|length| resolve_offset(length, cb_height));

    let elem_width = match (specified_width, left, right) {
        (Some(width), _, _) => width,
        // Both edges pinned: the offsets themselves determine the width.
        (None, Some(left), Some(right)) => container_width
            .saturating_sub(left.max(0) as u32)
            .saturating_sub(right.max(0) as u32),
        // CSS 2.1 10.3.7: with `width: auto` an absolutely positioned box
        // shrinks to fit its content -- it does not fill its containing block.
        // Filling it made `text-align: center` inside such a box centre against
        // the whole page. Yahoo! JAPAN's trending-list rank badges are
        // `position:absolute; min-width:16px; text-align:center`, so their
        // digits were flung a hundred pixels from the number they label.
        (None, _, _) => measure_cell_preferred_width(element, 0, images, fonts)
            .saturating_add(surround)
            .min(container_width),
    };
    let min_width = element
        .style
        .min_width
        .map(|length| resolve_length_value(length, container_width))
        .unwrap_or(0);
    let max_width = element
        .style
        .max_width
        .map(|length| resolve_length_value(length, container_width))
        .unwrap_or(u32::MAX);
    let elem_width = elem_width.min(max_width).max(min_width.min(max_width)).max(1);

    let x = match (left, right) {
        (Some(left), _) => (base_x as i64 + left as i64).max(0) as u32,
        // Only `right` is given, so it is the box's *right* edge that is placed,
        // that far in from the containing block's right edge. Ignoring `right`
        // pinned every such box to the left edge instead.
        (None, Some(right)) => (base_x as i64 + container_width as i64
            - right as i64
            - elem_width as i64)
            .max(0) as u32,
        (None, None) => static_x.max(base_x),
    };
    // Keep the unclamped position: page coordinates are unsigned, so a box with
    // a negative `top` cannot be drawn where it belongs, and we need to know how
    // far above the origin it wanted to sit before deciding what to do with it.
    let signed_top = match top {
        Some(top) => base_y as i64 + top as i64,
        None => static_y.max(base_y) as i64,
    };
    let mut cursor_y = signed_top.max(0) as u32;
    if std::env::var_os("TOBIRA_DEBUG_POS").is_some() {
        eprintln!(
            "abspos <{}> class={:?} base=({base_x},{base_y}) cb=({cb_width},{cb_height}) left={left:?} top={top:?} right={right:?} bottom={:?} -> {x},{cursor_y}",
            element.tag_name,
            element.attributes.get("class").map(|c| c.chars().take(30).collect::<String>()),
            element.style.bottom,
        );
    }

    let mut sub_context = LayoutContext {
        background_color: context.background_color,
        next_control_id: context.next_control_id,
        next_form_id: context.next_form_id,
        // A positioned box is itself the containing block for anything
        // positioned inside it, and its subtree is laid out in absolute page
        // coordinates -- so hand the descendants this box's own origin rather
        // than the default (0, 0).
        containing_block_origin: (x, cursor_y),
        containing_block_size: (elem_width, definite_height(&element.style)),
        ..LayoutContext::default()
    };
    // Use sub_context for form allocation so next_form_id counter stays consistent
    // when propagated back — avoids form_id going backwards if element is a <form>
    let current_form = form_context_for_element(element, &mut sub_context, current_form);
    let box_top = cursor_y;
    layout_block_element(element, x, elem_width, &mut cursor_y, &mut sub_context, images, fonts, current_form);
    let box_height = cursor_y.saturating_sub(box_top);

    // Parking a box above the viewport is the standard way to hide a skip link:
    // MDN's is `top: calc(var(--offset) * -1)`. With coordinates clamped at zero
    // it landed on y=0 instead and sat on top of the page. Nothing of it would be
    // on screen in a browser either, so drop it rather than draw it in the wrong
    // place. Only an explicitly negative `top` qualifies, so an ordinary
    // zero-height box at the top of the page is left alone.
    if top.is_some_and(|top| top < 0) && signed_top + box_height as i64 <= 0 {
        context.next_control_id = sub_context.next_control_id;
        context.next_form_id = sub_context.next_form_id;
        return;
    }

    let z = element.style.z_index.unwrap_or(0);
    let slot = context.positioned_commands.len();
    let links_from = context.links.len();
    let controls_from = context.controls.len();
    let hitboxes_from = context.element_hitboxes.len();
    context.positioned_commands.push((z, sub_context.commands));
    context.links.extend(sub_context.links);
    context.controls.extend(sub_context.controls);
    context.element_hitboxes.extend(sub_context.element_hitboxes);

    // Anchored to the bottom: hand it to the containing block to finish once it
    // knows its own height.
    if top.is_none()
        && let Some(offset) = element.style.bottom
        && element.style.position != Position::Fixed
    {
        context.pending_bottom.push(PendingBottom {
            slot,
            links_from,
            controls_from,
            hitboxes_from,
            drawn_top: box_top,
            height: box_height,
            offset,
        });
    }
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
    *cursor_y = advance_by_margin(*cursor_y, element.style.margin.top);
    let outer_x = offset_x_by_margin(x, element.style.margin.left);
    let outer_width = outer_width_with_margins(available_width, element.style.margin.left, element.style.margin.right);
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
    let areas = element.style.grid_template_areas.as_deref();
    let line_names = element.style.grid_line_names.as_deref();
    let col_tracks = &element.style.grid_template_columns;

    // The explicit grid is as wide as the larger of the two definitions, and a
    // column the track list does not size falls back to `grid-auto-columns`. So
    // a template that names areas but sizes nothing is an all-`auto` track list
    // -- not the single full-width column an empty track list would give, which
    // is what used to squeeze named-area pages into one narrow strip.
    let explicit_cols = col_tracks.len().max(areas.map_or(0, |a| a.columns));
    let tracks: Vec<GridTrackSize> = (0..explicit_cols)
        .map(|i| {
            col_tracks
                .get(i)
                .cloned()
                .unwrap_or_else(|| element.style.grid_auto_columns.clone())
        })
        .collect();
    // Widths are resolved after placement, because a content-sized track cannot
    // be measured until we know which items are in it.
    let n_cols = tracks.len().max(1);

    // ── Collect grid items ─────────────────────────────────────────────────
    // Absolutely positioned children are out of flow and take no grid slot.
    // Grid was the last container still placing them as items, so MDN's skip
    // link -- a `position:absolute; top:calc(var(--offset)*-1)` list sitting
    // under a `display:grid` body -- was laid out as a grid item and never had
    // its negative offset applied at all.
    let mut out_of_flow: Vec<&StyledElement> = Vec::new();
    let children: Vec<&StyledElement> = formatting_context_children(element)
        .into_iter()
        .filter_map(|child| {
            let StyledNode::Element(el) = child else {
                return None;
            };
            if el.style.display == Display::None {
                return None;
            }
            if matches!(el.style.position, Position::Absolute | Position::Fixed) {
                out_of_flow.push(el);
                return None;
            }
            Some(el)
        })
        .collect();

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
        // `grid-area: <name>` names an area of *this* container's template. The
        // item cannot see that template from its own style, so the lookup has to
        // happen here; an unknown name simply falls through to normal placement.
        let named_area = child
            .style
            .grid_area_name
            .as_deref()
            .and_then(|name| areas.and_then(|template| template.area(name)));

        // `<custom-ident>` line references (`grid-column: content`) resolve
        // against *this* container's track list, which the item cannot see from
        // its own style. A start with no matching end spans a single track.
        let (named_col, named_row) = match (child.style.grid_placement_names.as_deref(), line_names)
        {
            (Some(refs), Some(names)) => {
                let span_between = |start: Option<usize>, end: Option<usize>| match (start, end) {
                    (Some(start), Some(end)) => Some((start, end.saturating_sub(start).max(1))),
                    (Some(start), None) => Some((start, 1)),
                    _ => None,
                };
                let col = span_between(
                    refs.column_start
                        .as_deref()
                        .and_then(|n| names.column_line(n, GridEdge::Start)),
                    refs.column_end
                        .as_deref()
                        .and_then(|n| names.column_line(n, GridEdge::End)),
                );
                let row = span_between(
                    refs.row_start
                        .as_deref()
                        .and_then(|n| names.row_line(n, GridEdge::Start)),
                    refs.row_end
                        .as_deref()
                        .and_then(|n| names.row_line(n, GridEdge::End)),
                );
                (col, row)
            }
            _ => (None, None),
        };

        let (col_start, col_span) = if let Some((_, col_start, _, col_end)) = named_area {
            (Some(col_start), col_end - col_start)
        } else if let Some((start, span)) = named_col {
            (Some(start), span)
        } else {
            let p = &child.style.grid_column;
            let span = p.span.unwrap_or(1) as usize;
            let start = p.start.map(|s| (s - 1).max(0) as usize);
            (start, span)
        };
        let (row_start, row_span) = if let Some((row_start, _, row_end, _)) = named_area {
            (Some(row_start), row_end - row_start)
        } else if let Some((start, span)) = named_row {
            (Some(start), span)
        } else {
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

    // ── Resolve column widths ──────────────────────────────────────────────
    // `auto` / `min-content` / `max-content` tracks are sized by what is in
    // them, so they have to wait for placement. Without a measurement they used
    // to fall back to an equal share of the free space, which handed a third of
    // the page to the narrow tools rail in Wikipedia's `minmax(0,1fr)
    // min-content` body grid and squeezed the article into what was left.
    let content_sized = |track: Option<&GridTrackSize>| {
        matches!(
            track,
            Some(GridTrackSize::Auto | GridTrackSize::MinContent | GridTrackSize::MaxContent)
        )
    };
    let intrinsic_widths: Vec<u32> = if (0..n_cols).any(|c| content_sized(tracks.get(c))) {
        (0..n_cols)
            .map(|c| {
                if !content_sized(tracks.get(c)) {
                    return 0;
                }
                // Only single-column items speak for a column; a spanning item
                // would need the space distributed across the columns it covers.
                //
                // Each track kind asks its own question. `min-content` wants the
                // narrowest the item can get, `max-content` and `auto` the width
                // it would like. Measuring all three by laying the item out at the
                // container's full width -- which is what this did at first --
                // reports the *stretched* width of any block child, so a
                // `min-content` column claimed the whole row and the `1fr` beside
                // it collapsed. That is what crushed MDN's navigation menu.
                let want_min = matches!(tracks.get(c), Some(GridTrackSize::MinContent));
                placed
                    .iter()
                    .filter(|item| item.col == c && item.col_span <= 1)
                    .map(|item| {
                        let width = if want_min {
                            measure_cell_min_width(item.element, 0, images, fonts)
                        } else {
                            measure_cell_preferred_width(item.element, 0, images, fonts)
                        };
                        // A track can never need to be wider than the grid it is in.
                        width.min(content_width)
                    })
                    .max()
                    .unwrap_or(0)
            })
            .collect()
    } else {
        Vec::new()
    };

    let col_widths: Vec<u32> = if tracks.is_empty() {
        vec![content_width]
    } else {
        resolve_grid_tracks_with_intrinsic(&tracks, content_width, gap, &intrinsic_widths)
    };

    // ── Compute row heights ────────────────────────────────────────────────
    let max_row = placed
        .iter()
        .map(|p| p.row + p.row_span)
        .max()
        .unwrap_or(0)
        .max(areas.map_or(0, |a| a.rows));
    let auto_row_tracks = &element.style.grid_template_rows;
    let mut row_heights: Vec<u32> = vec![0u32; max_row];

    // Measure pass
    struct MeasuredItem<'a> {
        element: &'a StyledElement,
        col: usize,
        row: usize,
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
            row_span: item.row_span,
            measured_height: h,
            cell_width,
        });
    }

    // Items spanning several rows still have to make those rows tall enough to
    // hold them (CSS Grid Â§12.5). The measure pass above only folded in
    // single-row items, so any shortfall left by a spanning item is spread over
    // the rows it covers. Explicit track sizes below still win.
    for item in &measured {
        if item.row_span <= 1 {
            continue;
        }
        let end_row = (item.row + item.row_span).min(row_heights.len());
        if end_row <= item.row {
            continue;
        }
        let spanned = end_row - item.row;
        let gaps = gap * (spanned.saturating_sub(1)) as u32;
        let current = row_heights[item.row..end_row]
            .iter()
            .sum::<u32>()
            .saturating_add(gaps);
        let deficit = item.measured_height.saturating_sub(current);
        if deficit == 0 {
            continue;
        }
        let share = deficit / spanned as u32;
        let mut remainder = deficit % spanned as u32;
        for height in &mut row_heights[item.row..end_row] {
            *height = height.saturating_add(share);
            if remainder > 0 {
                *height = height.saturating_add(1);
                remainder -= 1;
            }
        }
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


    // The gradient goes down with the background, before any child command, so
    // it sits *under* the content. Emitting it after the children -- which is
    // where the border is drawn -- buries them: the cards on firefox.com turned
    // into blank white boxes. Its height is only known once the box is measured,
    // so it is patched up alongside the background rect.
    let gradient_cmd_index = if let Some(ref gradient) = element.style.background_gradient {
        let stops: Vec<GradientStop> = gradient
            .stops
            .iter()
            .map(|(color, position)| GradientStop {
                color: *color,
                position: *position,
            })
            .collect();
        context.commands.push(DrawCommand::Gradient(GradientCommand {
            x: outer_x,
            y: background_top,
            width: outer_width.max(1),
            height: 1,
            border_radius: element.style.border_radius,
            angle_deg_x1000: gradient.angle_deg_x1000,
            stops,
        }));
        Some(context.commands.len() - 1)
    } else {
        None
    };

    let content_top = background_top
        .saturating_add(if !element.style.border_style_none { element.style.border.top } else { 0 })
        .saturating_add(element.style.padding.top);

    // A stated height taller than the rows themselves is shared out over them
    // (`align-content: stretch`), which is what gives `align-items` room to work
    // with. MDN's nav bar is one 33px row inside a 4.125rem box; without this the
    // row stayed 33px and centring had nothing to centre in.
    let stated_height = definite_height(&element.style);
    if stated_height > 0 && !row_heights.is_empty() {
        let rows_total: u32 =
            row_heights.iter().sum::<u32>() + gap * max_row.saturating_sub(1) as u32;
        if stated_height > rows_total {
            let share = (stated_height - rows_total) / row_heights.len() as u32;
            for height in row_heights.iter_mut() {
                *height = height.saturating_add(share);
            }
        }
    }

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

        // `align-items` / `align-self` place the item within its row. Grid
        // ignored both, so every item sat at the top of its row: MDN's nav tabs
        // hugged the top edge of the bar instead of sitting in the middle of it.
        let align = match item.element.style.align_self {
            AlignSelf::Auto => element.style.align_items,
            AlignSelf::Stretch => AlignItems::Stretch,
            AlignSelf::FlexStart => AlignItems::FlexStart,
            AlignSelf::FlexEnd => AlignItems::FlexEnd,
            AlignSelf::Center => AlignItems::Center,
            AlignSelf::Baseline => AlignItems::Baseline,
        };
        let row_height: u32 = {
            let end = (item.row + item.row_span).min(row_heights.len());
            let rows: u32 = row_heights[item.row.min(row_heights.len())..end].iter().sum();
            rows + gap * end.saturating_sub(item.row).saturating_sub(1) as u32
        };
        let free = row_height.saturating_sub(item.measured_height);
        let mut item_y = cell_y
            + match align {
                AlignItems::Center => free / 2,
                AlignItems::FlexEnd => free,
                // `stretch` and `baseline` both start at the top here; a real
                // stretch would also resize the item, which block layout already
                // does for an auto height.
                AlignItems::Stretch | AlignItems::FlexStart | AlignItems::Baseline => 0,
            };
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

    // Total content height. A stated `height` wins over the rows' own sum, the
    // way it already does for a flex container -- grid was the one container
    // still sizing itself purely by content. MDN's `.navigation` asks for
    // `height: var(--navigation-height)` (4.125rem) and got whatever its items
    // added up to instead, which left a tall empty band under the nav bar.
    let rows_h: u32 = row_heights.iter().sum::<u32>()
        + gap * max_row.saturating_sub(1) as u32;
    let total_h = if stated_height > 0 { stated_height } else { rows_h };
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
    if let Some(index) = gradient_cmd_index
        && let Some(DrawCommand::Gradient(gradient)) = context.commands.get_mut(index)
    {
        gradient.height = (background_bottom - background_top).max(1);
    }


    // Draw border
    if !element.style.border_style_none && !element.style.border_color_transparent {
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

        // Which element drew this? Border rects all look alike in the command
        // dump, and tracking one back to its rule is otherwise guesswork.
        if std::env::var_os("TOBIRA_DEBUG_BORDERS").is_some()
            && border_top_h + border_bottom_h + border_left_w + border_right_w > 0
        {
            eprintln!(
                "border <{}> class={:?} t={border_top_h} r={border_right_w} b={border_bottom_h} l={border_left_w} color={bc:#08x}",
                element.tag_name,
                element
                    .attributes
                    .get("class")
                    .map(|c| c.chars().take(30).collect::<String>()),
            );
        }
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

    // Placed last, from the grid's content box, the same way a flex container
    // finishes with its own out-of-flow children.
    for child in out_of_flow {
        let mut static_y = content_top;
        layout_positioned_element(
            child,
            content_x,
            content_width,
            &mut static_y,
            context,
            images,
            fonts,
            current_form.clone(),
        );
    }

    *cursor_y = advance_by_margin(background_bottom, element.style.margin.bottom);
}

/// Resolve grid track sizes into pixel widths, distributing fr units.
fn resolve_grid_tracks(tracks: &[GridTrackSize], available_px: u32, gap: u32) -> Vec<u32> {
    resolve_grid_tracks_with_intrinsic(tracks, available_px, gap, &[])
}

/// As `resolve_grid_tracks`, but with measured widths for the content-sized
/// tracks.
///
/// A track with a measured width is treated as fixed, so the `fr` tracks split
/// what genuinely remains. `intrinsic` is indexed by track; a missing or zero
/// entry means "not measured" and falls back to the old even-share behaviour.
fn resolve_grid_tracks_with_intrinsic(
    tracks: &[GridTrackSize],
    available_px: u32,
    gap: u32,
    intrinsic: &[u32],
) -> Vec<u32> {
    let n = tracks.len();
    let total_gap = gap * n.saturating_sub(1) as u32;
    let remaining_after_gap = available_px.saturating_sub(total_gap);

    let measured = |i: usize| intrinsic.get(i).copied().filter(|width| *width > 0);

    let mut widths = vec![0u32; n];
    // Which tracks are already final, so the stretch pass below leaves them be.
    let mut content_fixed = vec![false; n];
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
            // `min-content` / `max-content` are sized purely by their contents
            // and do *not* stretch, so a measurement makes them fixed.
            GridTrackSize::MinContent | GridTrackSize::MaxContent if measured(i).is_some() => {
                let width = measured(i).unwrap_or(0);
                widths[i] = width;
                content_fixed[i] = true;
                fixed_total += width;
            }
            // `auto` is content-sized *and then stretched* to fill what is left,
            // so it takes a share of the free space and uses its measurement
            // only as a floor. Unmeasured content tracks fall in here too.
            GridTrackSize::Auto | GridTrackSize::MinContent | GridTrackSize::MaxContent => {
                auto_count += 1;
            }
        }
    }

    let remaining = remaining_after_gap.saturating_sub(fixed_total);

    // Split the free space between the `fr` tracks and the stretching tracks.
    // When only one kind is present it takes all of it: reserving a share for a
    // kind that is not there used to throw that share away, because the `fr`
    // payout is skipped entirely when `fr_total` is zero. An all-`auto` list
    // therefore kept only a third of the width -- `auto auto` across 1200px got
    // 200px per column instead of 600px.
    let (fr_space, auto_space) = match (fr_total > 0, auto_count > 0) {
        (true, true) => {
            let fr_space = remaining * 2 / 3;
            (fr_space, remaining - fr_space)
        }
        (true, false) => (remaining, 0),
        (false, true) => (0, remaining),
        (false, false) => (0, 0),
    };

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
            if content_fixed[i] {
                continue;
            }
            if matches!(
                track,
                GridTrackSize::Auto | GridTrackSize::MinContent | GridTrackSize::MaxContent
            ) {
                // Never stretch a track below what it has to hold.
                widths[i] = per_auto.max(measured(i).unwrap_or(0));
            }
        }
    }

    widths
}

/// Intrinsic main-axis content width of a flex item: lay it out in a throwaway
/// context at the container's content width and take the farthest right edge of
/// what it produced (rects/text/images/controls). Used to size flex items that
/// have neither an explicit `width` nor `flex-basis`, so they shrink-to-fit
/// instead of stretching to fill.
fn flex_item_content_width(
    child: &StyledElement,
    avail_width: u32,
    images: &ImageStore,
    fonts: &mut FontContext,
    bg: Color,
) -> u32 {
    fn max_right(cmds: &[DrawCommand]) -> u32 {
        let mut m = 0;
        for cmd in cmds {
            let r = match cmd {
                DrawCommand::Rect(r) => r.x.saturating_add(r.width),
                DrawCommand::Text(t) => t.x.saturating_add(t.width),
                DrawCommand::Image(i) => i.x.saturating_add(i.width),
                DrawCommand::Gradient(g) => g.x.saturating_add(g.width),
                DrawCommand::Layer(l) => l.x.saturating_add(l.width).max(max_right(&l.commands)),
                DrawCommand::Sticky(s) => s.layer.x.saturating_add(s.layer.width),
            };
            m = m.max(r);
        }
        m
    }
    // Lay the item out at *its own* max-content width, not at the container's.
    // Measuring the painted extent inside a container-wide box counts the empty
    // space that `text-align: center` (or `right`) leaves before the content:
    // rust-lang.org centres each navigation label, so every one of them
    // measured about half the page wide and only a single item fitted per line.
    let surround = child.style.padding.left
        + child.style.padding.right
        + if child.style.border_style_none {
            0
        } else {
            child.style.border.left + child.style.border.right
        }
        + child.style.margin.left.max(0) as u32
        + child.style.margin.right.max(0) as u32;
    let intrinsic = measure_cell_preferred_width(child, 0, images, fonts)
        .saturating_add(surround)
        .min(avail_width.max(1))
        .max(1);

    let mut dummy_y = 0u32;
    let mut ctx = LayoutContext {
        background_color: bg,
        ..LayoutContext::default()
    };
    layout_block_element(
        child,
        0,
        intrinsic,
        &mut dummy_y,
        &mut ctx,
        images,
        fonts,
        None,
    );
    let mut w = max_right(&ctx.commands);
    for c in &ctx.controls {
        w = w.max(c.x.saturating_add(c.width));
    }
    // `max_right` already includes margin.left (the child was laid out at x=0 and
    // block layout offsets content by it), but the right edge of painted content
    // doesn't cover margin.right — add it, or the item's slot is too narrow and a
    // later height-measure at that width wraps the content (a one-line span
    // ballooned to two lines, pushing every other flex item down).
    // The layout pass can still report more than the estimate -- a form
    // control or a nested flex row the intrinsic measure cannot see through --
    // so take whichever is larger.
    let w = w.max(intrinsic);
    let result = (w as i64 + child.style.margin.right as i64).max(1) as u32;
    if std::env::var_os("TOBIRA_DEBUG_ITEM").is_some() {
        eprintln!(
            "item <{}> class={:?} avail={avail_width} intrinsic={intrinsic} painted={w} -> {result} pad=({},{}) border=({},{}) margin=({},{})",
            child.tag_name,
            child.attributes.get("class").map(|c| c.chars().take(24).collect::<String>()),
            child.style.padding.left, child.style.padding.right,
            child.style.border.left, child.style.border.right,
            child.style.margin.left, child.style.margin.right,
        );
    }
    result
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
    *cursor_y = advance_by_margin(*cursor_y, element.style.margin.top);
    let avail_width = outer_width_with_margins(width, element.style.margin.left, element.style.margin.right);
    // Honor an explicit width on the flex container (needed for flex-wrap to know
    // where lines break); otherwise take the available width.
    let outer_width = match element.style.width {
        Some(LengthValue::Pixels(px)) => px,
        Some(LengthValue::Percent(p)) => (avail_width as u64 * p as u64 / 100) as u32,
        _ => avail_width,
    }
    .max(1);
    let outer_x = outer_x_with_auto_margins(&element.style, x, width, outer_width);
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

    // Collect visible flex items (only element children, not text nodes).
    // An out-of-flow child is not a flex item -- it takes no space on the line
    // and no share of the free space -- but it still has to be drawn, so it is
    // set aside and positioned after the line is laid out. Counting one as an
    // item gave it a slot: Yahoo! JAPAN's masthead pins its logo with
    // `position: absolute`, and the logo's 213px slot pushed the two groups of
    // service shortcuts apart and sat between them.
    // Text sitting directly inside a flex container is a flex item too -- an
    // anonymous one. Collecting only element children dropped such text
    // outright, so `<div style="display:flex">hello</div>` rendered empty.
    let anonymous: Vec<StyledElement> = element
        .children
        .iter()
        .filter_map(|child| match child {
            StyledNode::Text(text) if !text.text.trim().is_empty() => {
                // The wrapper is a block container, not another flex one: a
                // text node carries its parent's computed style, so copying it
                // verbatim would make the wrapper a flex container that wraps
                // its own text again, for ever.
                //
                // Everything else the container styles its own box with has to
                // go for the same reason -- an anonymous box takes the
                // inherited properties and nothing more. Left in place, the
                // padding was charged again one level down, so the text was
                // laid out in a strip narrower than the space measured for it
                // and wrapped where it fits perfectly: firefox.com's header
                // menu titles each broke across two lines and pushed the header
                // from 68px to 117px tall.
                let mut style = (*text.style).clone();
                style.display = Display::Block;
                style.padding = crate::css::EdgeSizes::default();
                style.margin = crate::css::SignedEdgeSizes::default();
                style.border = crate::css::EdgeSizes::default();
                style.border_radius = 0;
                style.background_color = None;
                style.background_gradient = None;
                style.background_image_url = None;
                style.width = None;
                style.height = None;
                style.min_width = None;
                style.max_width = None;
                style.min_height = 0;
                style.max_height = None;
                Some(StyledElement {
                    tag_name: String::new(),
                    attributes: std::collections::BTreeMap::new(),
                    style: Arc::new(style),
                    children: vec![child.clone()],
                })
            }
            _ => None,
        })
        .collect();

    let mut out_of_flow: Vec<&StyledElement> = Vec::new();
    let mut anonymous_next = anonymous.iter();
    let mut children: Vec<&StyledElement> = element.children.iter().filter_map(|child| {
        let StyledNode::Element(el) = child else {
            return match child {
                StyledNode::Text(text) if !text.text.trim().is_empty() => anonymous_next.next(),
                _ => None,
            };
        };
        if el.style.display == Display::None {
            return None;
        }
        if matches!(el.style.position, Position::Absolute | Position::Fixed) {
            out_of_flow.push(el);
            return None;
        }
        Some(el)
    }).collect();

    // `order` re-sequences the items without touching the document, and a
    // reverse direction lays them out from the far end. Neither was
    // implemented, so `flex-direction: row-reverse` -- which Yahoo! JAPAN uses
    // to put the icon before the label in its service list -- came out in
    // document order and, with `justify-content: flex-end` also unflipped,
    // packed against the wrong edge: the whole left rail read right-aligned.
    children.sort_by_key(|child| child.style.order);
    if matches!(
        element.style.flex_direction,
        FlexDirection::RowReverse | FlexDirection::ColumnReverse
    ) {
        children.reverse();
    }
    let children = children;

    // Everything drawn from here on belongs to this container's subtree, which
    // is what `overflow: hidden` has to clip.
    let clip_start_idx = context.commands.len();

    // A flex container that is not `position: static` is the containing block
    // for the positioned boxes under it, exactly as a block one is. Only the
    // block path established this, which was invisible while every positioned
    // child of a flex container was mistakenly laid out as a flex item; now
    // that they are positioned properly, the gap shows -- Yahoo! JAPAN's
    // masthead logo is pinned inside a `position: relative` flex row, and
    // without this it resolved its offsets against the page and flew to the
    // far left.
    let saved_origin = context.containing_block_origin;
    let saved_cb_size = context.containing_block_size;
    // Anything anchored to this block's bottom edge is recorded from here on,
    // and settled below once the block's height is known.
    let pending_mark = context.pending_bottom.len();
    let establishes_containing_block = element.style.position != Position::Static;
    if establishes_containing_block {
        context.containing_block_origin = (outer_x, background_top);
        context.containing_block_size = (outer_width, definite_height(&element.style));
    }

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

    // The gradient goes down with the background, before any child command, so
    // it sits *under* the content. Emitting it after the children -- where the
    // border is drawn -- buries them: the cards on firefox.com turned into blank
    // white boxes. Its height is only known once the box is measured, so it is
    // patched up alongside the background rect.
    let gradient_cmd_index = if let Some(ref gradient) = element.style.background_gradient {
        let stops: Vec<GradientStop> = gradient
            .stops
            .iter()
            .map(|(color, position)| GradientStop {
                color: *color,
                position: *position,
            })
            .collect();
        context.commands.push(DrawCommand::Gradient(GradientCommand {
            x: outer_x,
            y: background_top,
            width: outer_width.max(1),
            height: 1,
            border_radius: element.style.border_radius,
            angle_deg_x1000: gradient.angle_deg_x1000,
            stops,
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

            // Resolve a LengthValue against the flex content width.
            let resolve = |lv: &LengthValue| -> u32 {
                match lv {
                    LengthValue::Pixels(px) => *px,
                    LengthValue::Percent(p) => (content_width as f32 * (*p as f32) / 100.0) as u32,
                    LengthValue::MinContent => 0,
                    LengthValue::MaxContent => content_width,
                    LengthValue::FitContent(max_px) => content_width.min(*max_px),
                    LengthValue::Calc { percent_hundredths, px } => crate::css::resolve_calc(*percent_hundredths, *px, content_width),
                }
            };

            // Base main size per item: explicit `width`, else `flex-basis`, else
            // the item's intrinsic content width. Items grow past their base only
            // when `flex-grow > 0` (default 0); otherwise they stay content-sized
            // and the leftover space is distributed by justify-content. (The old
            // code stretched every auto-width item to `remaining / n`, i.e. it
            // treated everything as `flex-grow: 1`, which spread out the React
            // demo's buttons and pushed `space-between` items off-screen.)
            let base_widths: Vec<u32> = children
                .iter()
                .map(|child| {
                    // Flex arithmetic works in margin boxes: the placement loop
                    // advances the cursor by the slot width, and the child then
                    // carves its own margins out of that slot. The intrinsic
                    // measurement already reports a margin box, so strip the
                    // margins off it here -- the min/max clamps below apply to
                    // the box itself -- and add them back at the end. An
                    // explicit `width` is not a margin box and needs them added.
                    let margins = child.style.margin.left.max(0) as u32
                        + child.style.margin.right.max(0) as u32;
                    let base = if let Some(w) = child.style.width.as_ref() {
                        resolve(w)
                    } else if let Some(b) = child.style.flex_basis.as_ref() {
                        resolve(b)
                    } else {
                        flex_item_content_width(
                            child,
                            content_width,
                            images,
                            fonts,
                            context.background_color,
                        )
                        .min(content_width)
                        .saturating_sub(margins)
                    };
                    // `min-width` / `max-width` bound a flex item's base size
                    // just as they bound any other box. Skipping them let a
                    // column that asks for `calc(47.47475% - 20px)` but insists
                    // on `min-width: 450px` collapse to nothing, and the column
                    // after it was then drawn on top of it.
                    let min = child
                        .style
                        .min_width
                        .as_ref()
                        .map(|length| resolve(length))
                        .unwrap_or(0);
                    let max = child
                        .style
                        .max_width
                        .as_ref()
                        .map(|length| resolve(length))
                        .unwrap_or(u32::MAX);
                    base.min(max).max(min.min(max)).max(1).saturating_add(margins)
                })
                .collect();

            // Every width below is a margin box, so margins must not be
            // subtracted again. Counting them twice shrank rows that already
            // fitted: Yahoo! JAPAN's top-right navigation lost 25px this way and
            // wrapped "ホームページに設定する" onto two lines.
            let base_sum: u32 = base_widths.iter().sum();
            let mut item_widths = base_widths.clone();
            if base_sum.saturating_add(total_gap) > content_width
                && element.style.flex_wrap == FlexWrap::NoWrap
                && base_sum > 0
            {
                // Single-line overflow. CSS Flexbox 9.7 weights the
                // shrinkage by `flex-shrink` times the base size, so an item
                // that says `flex-shrink: 0` keeps its width and the others
                // absorb the whole overflow. `flex-shrink` was parsed and then
                // never read: everything shrank proportionally, and Yahoo!
                // JAPAN's topics column -- `flex: 1 0 240px`, i.e. "at least
                // 240px, never shrink" -- was squeezed to 132px, which wrapped
                // every headline onto three lines.
                let avail = content_width.saturating_sub(total_gap).max(1);
                let overflow = base_sum.saturating_sub(avail) as u64;
                let weights: Vec<u64> = children
                    .iter()
                    .zip(base_widths.iter())
                    .map(|(child, base)| child.style.flex_shrink as u64 * *base as u64)
                    .collect();
                let total_weight: u64 = weights.iter().sum();
                if total_weight > 0 {
                    for (i, w) in item_widths.iter_mut().enumerate() {
                        let taken = overflow.saturating_mul(weights[i]) / total_weight;
                        *w = (*w as u64).saturating_sub(taken).max(1) as u32;
                    }
                }
                // With nothing allowed to shrink the row simply overflows,
                // which is what the spec asks for.
            } else {
                // Distribute free space to growers (flex-grow), proportionally.
                let free = content_width
                    .saturating_sub(base_sum)
                    .saturating_sub(total_gap);
                let total_grow: u32 = children.iter().map(|c| c.style.flex_grow).sum();
                if free > 0 && total_grow > 0 {
                    for (i, child) in children.iter().enumerate() {
                        if child.style.flex_grow > 0 {
                            item_widths[i] = item_widths[i].saturating_add(
                                free.saturating_mul(child.style.flex_grow) / total_grow,
                            );
                        }
                    }
                }
            }
            let total_fixed: i64 = item_widths.iter().map(|w| *w as i64).sum::<i64>();

            // Measure heights at the final item widths (wrapping depends on width).
            let item_heights: Vec<u32> = children
                .iter()
                .enumerate()
                .map(|(i, child)| {
                    let child_w = item_widths[i].max(1);
                    let mut dummy_y = content_y;
                    let mut dummy_ctx = LayoutContext {
                        background_color: context.background_color,
                        ..LayoutContext::default()
                    };
                    layout_block_element(
                        child,
                        content_x,
                        child_w,
                        &mut dummy_y,
                        &mut dummy_ctx,
                        images,
                        fonts,
                        current_form.clone(),
                    );
                    dummy_y.saturating_sub(content_y)
                })
                .collect();
            let content_max_height = *item_heights.iter().max().unwrap_or(&0);
            // The flex line's cross size is the container's definite content
            // height (if any), expanded to fit the tallest item — so
            // `align-items: center`/`flex-end` center within the container, not
            // just within the items (vertical centering in a fixed-height row).
            let container_cross_height = match element.style.height {
                Some(LengthValue::Pixels(px)) => Some(px),
                Some(LengthValue::Percent(p)) => {
                    context.container_height.map(|ch| (ch as u64 * p as u64 / 100) as u32)
                }
                _ => None,
            };
            let max_height = container_cross_height
                .map(|h| h.max(content_max_height))
                .unwrap_or(content_max_height);

            let child_cross_offset = |child: &StyledElement, line_h: u32, item_h: u32| -> u32 {
                let self_align = match child.style.align_self {
                    AlignSelf::Auto => element.style.align_items,
                    AlignSelf::FlexStart => AlignItems::FlexStart,
                    AlignSelf::FlexEnd => AlignItems::FlexEnd,
                    AlignSelf::Center => AlignItems::Center,
                    AlignSelf::Stretch => AlignItems::Stretch,
                    AlignSelf::Baseline => AlignItems::Baseline,
                };
                match self_align {
                    AlignItems::Center => line_h.saturating_sub(item_h) / 2,
                    AlignItems::FlexEnd => line_h.saturating_sub(item_h),
                    _ => 0,
                }
            };

            if element.style.flex_wrap == FlexWrap::NoWrap {
                // Single line: honor justify-content + cross-axis alignment.
                let (start_offset, item_gap) = justify_content_offsets(
                    justify_for_direction(
                        element.style.justify_content,
                        element.style.flex_direction,
                    ),
                    content_width,
                    total_fixed.max(0) as u32,
                    total_gap,
                    n as u32,
                );
                let mut cursor_x = content_x.saturating_add(start_offset);
                if std::env::var_os("TOBIRA_DEBUG_FLEX").is_some() {
                    eprintln!(
                        "flexrow <{}> class={:?} x={x} avail={width} outer_x={outer_x} outer_w={outer_width} auto=({},{}) content_x={content_x} w={content_width} items={:?}",
                        element.tag_name,
                        element.attributes.get("class").map(|c| c.chars().take(28).collect::<String>()),
                        element.style.margin_left_auto,
                        element.style.margin_right_auto,
                        children.iter().zip(item_widths.iter()).map(|(c, w)| (
                            c.tag_name.clone(),
                            c.attributes.get("class").map(|s| s.chars().take(16).collect::<String>()).unwrap_or_default(),
                            *w,
                        )).collect::<Vec<_>>(),
                    );
                }
                for (i, child) in children.iter().enumerate() {
                    let child_w = item_widths[i];
                    let child_y_offset = child_cross_offset(child, max_height, item_heights[i]);
                    let mut child_y = content_y.saturating_add(child_y_offset);
                    let child_form = form_context_for_element(child, context, current_form.clone());
                    context.flex_item_main_size = Some(child_w);
                    layout_block_element(child, cursor_x, child_w, &mut child_y, context, images, fonts, child_form);
                    cursor_x = cursor_x.saturating_add(child_w).saturating_add(item_gap);
                }
                *cursor_y = content_y.saturating_add(max_height)
                    .saturating_add(element.style.padding.bottom)
                    .saturating_add(border_bottom_sz);
            } else {
                // flex-wrap: greedily break items into lines that fit the
                // container width, then lay each line out flex-start with
                // per-line cross-axis alignment.
                let gap = element.style.gap;
                let widths: Vec<u32> = item_widths.clone();
                if std::env::var_os("TOBIRA_DEBUG_FLEX").is_some() {
                    eprintln!(
                        "flexwrap <{}> class={:?} w={content_width} gap={gap} widths={widths:?}",
                        element.tag_name,
                        element.attributes.get("class").map(|c| c.chars().take(28).collect::<String>()),
                    );
                }
                let mut lines: Vec<Vec<usize>> = vec![Vec::new()];
                let mut line_w = 0u32;
                for (i, &w) in widths.iter().enumerate() {
                    let line = lines.last_mut().expect("at least one line");
                    let add = if line.is_empty() { w } else { w.saturating_add(gap) };
                    if !line.is_empty() && line_w.saturating_add(add) > content_width {
                        lines.push(vec![i]);
                        line_w = w;
                    } else {
                        line.push(i);
                        line_w = line_w.saturating_add(add);
                    }
                }
                let mut line_y = content_y;
                for line in &lines {
                    let line_h = line.iter().map(|&i| item_heights[i]).max().unwrap_or(0);
                    let mut cx = content_x;
                    for &i in line {
                        let child = &children[i];
                        let w = widths[i];
                        let yoff = child_cross_offset(child, line_h, item_heights[i]);
                        let mut cy = line_y.saturating_add(yoff);
                        let child_form = form_context_for_element(child, context, current_form.clone());
                        context.flex_item_main_size = Some(w);
                        layout_block_element(child, cx, w, &mut cy, context, images, fonts, child_form);
                        cx = cx.saturating_add(w).saturating_add(gap);
                    }
                    line_y = line_y.saturating_add(line_h).saturating_add(gap);
                }
                if !lines.is_empty() {
                    line_y = line_y.saturating_sub(gap); // drop trailing row gap
                }
                *cursor_y = line_y
                    .saturating_add(element.style.padding.bottom)
                    .saturating_add(border_bottom_sz);
            }
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
    if let Some(index) = gradient_cmd_index
        && let Some(DrawCommand::Gradient(gradient)) = context.commands.get_mut(index)
    {
        gradient.height = background_height;
    }

    context.background_color = saved_bg;

    // Only the block path clipped `overflow: hidden`, so a flex container never
    // did. That was invisible while a nested flex box was mistakenly laid out as
    // a block; once it dispatches on its own `display`, the gap shows. It
    // matters for the same reason it does on the block path: the
    // visually-hidden idiom (`position:absolute; width:1px; height:1px;
    // overflow:hidden`) is on almost every page, and several of Yahoo! JAPAN's
    // screen-reader headings are flex containers -- unclipped they printed down
    // the left edge, one character to a line.
    if element.style.overflow == Overflow::Hidden {
        let clip_height = element
            .style
            .height
            .map(|length| match length {
                LengthValue::Pixels(px) => px,
                other => resolve_length_value(other, background_height),
            })
            .unwrap_or(background_height);
        clip_commands_to_box(
            &mut context.commands,
            clip_start_idx,
            outer_x,
            background_top,
            outer_width,
            clip_height,
            fonts,
        );
    }

    // Draw borders
    if !element.style.border_style_none && !element.style.border_color_transparent {
        let bc = apply_opacity(element.style.border_color, context.background_color, element.style.effective_opacity);
        if std::env::var_os("TOBIRA_DEBUG_BORDERS").is_some()
            && border_top + border_bottom_sz + border_left + border_right > 0
        {
            eprintln!(
                "border(flex) <{}> class={:?} t={border_top} r={border_right} b={border_bottom_sz} l={border_left} color={bc:#08x}",
                element.tag_name,
                element
                    .attributes
                    .get("class")
                    .map(|c| c.chars().take(30).collect::<String>()),
            );
        }
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

    // Out-of-flow children take their static position from where the container's
    // content begins; `layout_positioned_element` resolves the rest. This runs
    // before the containing block is restored, because for these children this
    // container *is* it.
    for child in out_of_flow {
        let mut static_y = content_y;
        layout_positioned_element(
            child,
            content_x,
            content_width,
            &mut static_y,
            context,
            images,
            fonts,
            current_form.clone(),
        );
    }

    if establishes_containing_block {
        settle_bottom_anchored(
            context,
            pending_mark,
            background_top,
            cursor_y.saturating_sub(background_top),
        );
    }
    context.containing_block_origin = saved_origin;
    context.containing_block_size = saved_cb_size;

    *cursor_y = advance_by_margin(*cursor_y, element.style.margin.bottom);
}

/// In a reverse direction the main axis starts at the far edge, so
/// `flex-start` and `flex-end` swap. The distributed values are symmetric and
/// need no adjustment.
fn justify_for_direction(justify: JustifyContent, direction: FlexDirection) -> JustifyContent {
    if !matches!(
        direction,
        FlexDirection::RowReverse | FlexDirection::ColumnReverse
    ) {
        return justify;
    }
    match justify {
        JustifyContent::FlexStart => JustifyContent::FlexEnd,
        JustifyContent::FlexEnd => JustifyContent::FlexStart,
        other => other,
    }
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
mod percentage_sizing_tests {
    use super::*;
    use crate::css::{InteractiveState, build_styled_tree, parse_stylesheet};
    use crate::html::parse_document;

    fn text_runs(css: &str, html: &str) -> Vec<TextCommand> {
        let doc = parse_document(html);
        let sheet = parse_stylesheet(css);
        let styled = build_styled_tree(&doc, &sheet, 1280, &InteractiveState::default());
        let mut fonts = FontContext::load();
        let images = ImageStore::default();
        layout_styled_document(&styled, &images, 1280, &mut fonts).texts()
    }

    /// `max-width` percentages resolve against the containing block. They used
    /// to be resolved at parse time against the *font size*, so the extremely
    /// common `max-width: 100%` became 16px and squeezed text down to one
    /// character per line -- which is how Yahoo! JAPAN rendered.
    #[test]
    fn max_width_percent_resolves_against_the_container() {
        let runs = text_runs(
            ".wide { max-width: 100%; }",
            "<div style=\"width:600px\"><div class=\"wide\">ホームページに設定する</div></div>",
        );
        assert_eq!(runs.len(), 1, "the line must not be broken up: {runs:?}");
        assert_eq!(runs[0].text, "ホームページに設定する");
    }

    /// A percentage that genuinely constrains still does.
    #[test]
    fn a_constraining_max_width_percent_still_applies() {
        let narrow = text_runs(
            ".half { max-width: 10%; }",
            "<div style=\"width:600px\"><div class=\"half\">ホームページに設定する</div></div>",
        );
        assert!(
            narrow.len() > 1,
            "10% of 600px should force a wrap, got {narrow:?}"
        );
    }

    /// `min-width` percentages resolve the same way.
    #[test]
    fn min_width_percent_resolves_against_the_container() {
        let runs = text_runs(
            ".w { min-width: 100%; }",
            "<div style=\"width:600px\"><div class=\"w\">ホームページに設定する</div></div>",
        );
        assert_eq!(runs.len(), 1);
    }

    /// `:before` / `:after` with a single colon are the legacy spelling of the
    /// pseudo-elements. Treated as unknown pseudo-classes they were dropped, so
    /// the rule applied to the host and `width: 0` collapsed it.
    #[test]
    fn single_colon_pseudo_elements_do_not_style_the_host() {
        let runs = text_runs(
            ".x:after, .x:before { display: block; width: 0; height: 0; content: \"\"; }",
            "<div style=\"width:600px\"><div class=\"x\">ホームページに設定する</div></div>",
        );
        assert_eq!(
            runs.len(),
            1,
            "an :after rule must not shrink the element itself: {runs:?}"
        );
        assert_eq!(runs[0].text, "ホームページに設定する");
    }

    /// The double-colon form was already excluded and must stay that way.
    #[test]
    fn double_colon_pseudo_elements_still_do_not_style_the_host() {
        let runs = text_runs(
            ".x::after { width: 0; content: \"\"; }",
            "<div style=\"width:600px\"><div class=\"x\">ホームページに設定する</div></div>",
        );
        assert_eq!(runs.len(), 1);
    }

    /// The visually-hidden idiom is on almost every real page: it hides text
    /// from sight while leaving it for screen readers. Only block-level boxes
    /// clip `overflow: hidden`, and `position: absolute` is supposed to make a
    /// `<span>` block-level. Without that blockification the text was never
    /// clipped and rendered as a 1px-wide column, one character per line.
    #[test]
    fn the_visually_hidden_idiom_is_clipped() {
        const VH: &str = ".vh{position:absolute;width:1px;height:1px;padding:0;overflow:hidden;clip:rect(1px,1px,1px,1px);border:0}";
        for tag in ["div", "span", "p"] {
            let html = format!(
                "<div style=\"width:600px\"><{tag} class=\"vh\">キーワード入力補助を開く</{tag}></div>"
            );
            let runs = text_runs(VH, &html);
            // Nothing at all: the renderer draws whole glyphs, so a 1px-wide box
            // has room for none of them. Keeping the run because it merely
            // touched the box is what put stray characters at the page origin.
            assert!(
                runs.is_empty(),
                "<{tag}> should be clipped away entirely, got {runs:?}"
            );
        }
    }

    /// Comment stripping used to copy a stylesheet one byte at a time via
    /// `byte as char`, reading each byte as a Latin-1 code point. That explodes
    /// every multi-byte character into one bogus character per byte, and the
    /// stripper runs twice over a stylesheet, so the damage compounded: Yahoo!
    /// JAPAN's news separators reached the screen as six garbage characters
    /// instead of one middle dot.
    #[test]
    fn non_ascii_survives_comment_stripping() {
        for text in ["\u{30fb}", "\u{5bfe}", "\u{30e1}\u{30a4}\u{30ea}\u{30aa}"] {
            let runs = text_runs(&format!("p::before{{content:\"{text}\"}}"), "<p>x</p>");
            let shown: String = runs.iter().map(|run| run.text.as_str()).collect();
            assert_eq!(shown, format!("{text}x"));
        }
    }

    /// Comments still go away, and one sitting next to a multi-byte character
    /// must not take part of that character with it.
    #[test]
    fn comments_are_stripped_around_multibyte_text() {
        let runs = text_runs("p::before{/* a */content:\"\u{30fb}\"/* b */}", "<p>x</p>");
        let shown: String = runs.iter().map(|run| run.text.as_str()).collect();
        assert_eq!(shown, "\u{30fb}x");
    }

    /// Clipping trims a run rather than discarding it: a box narrower than its
    /// text keeps the glyphs that fit and drops only the ones that spill.
    #[test]
    fn a_partly_covered_run_is_truncated_not_dropped() {
        let runs = text_runs(
            ".narrow{width:40px;height:20px;overflow:hidden}",
            "<div style=\"width:600px\"><div class=\"narrow\">キーワード入力補助を開く</div></div>",
        );
        let shown: String = runs.iter().map(|run| run.text.as_str()).collect();
        assert!(!shown.is_empty(), "some text must survive: {runs:?}");
        assert!(
            "キーワード入力補助を開く".starts_with(&shown),
            "what survives must be a prefix of the original, got {shown:?}"
        );
        assert!(
            shown.chars().count() < 12,
            "a 40px box cannot show the whole string, got {shown:?}"
        );
    }

    /// Floats blockify too, and an inline box with no out-of-flow positioning
    /// must stay inline -- clipping does not apply to it.
    #[test]
    fn only_out_of_flow_boxes_are_blockified() {
        let floated = text_runs(
            ".f{float:left;width:1px;height:1px;overflow:hidden}",
            "<div style=\"width:600px\"><span class=\"f\">キーワード入力補助を開く</span></div>",
        );
        let shown: usize = floated.iter().map(|r| r.text.chars().count()).sum();
        assert!(shown <= 2, "a floated span should clip, showed {shown}");

        let inline = text_runs(
            ".i{display:inline;width:1px;height:1px;overflow:hidden}",
            "<div style=\"width:600px\"><span class=\"i\">キーワード入力補助を開く</span></div>",
        );
        assert_eq!(
            inline.iter().map(|r| r.text.chars().count()).sum::<usize>(),
            12,
            "a plain inline box is not clipped"
        );
    }

    /// A row holds its items apart by `gap`, so that space counts towards the
    /// width the row needs. Left out, a shrink-to-fit box came up exactly the
    /// gaps short and its own contents then wrapped inside a box that had been
    /// measured to fit them. firefox.com's header menu titles are a label and a
    /// chevron 4px apart; every one broke over two lines and took the header
    /// from 68px to 117px tall.
    #[test]
    fn a_rows_gaps_count_towards_the_width_it_needs() {
        let runs = text_runs(
            ".menu{display:flex}.title{display:inline-block}.head{display:flex;gap:40px}.pad{width:40px}",
            "<div class=\"menu\" style=\"width:600px\"><a class=\"title\"><span class=\"head\">ブラウザー<span class=\"pad\"></span></span></a></div>",
        );
        assert_eq!(runs.len(), 1, "the label must not be broken up: {runs:?}");
    }

    /// An `inline-flex` box is inline-level: as wide as its contents, not as
    /// wide as the space on offer, and placed by the surrounding `text-align`.
    /// Given the whole line instead, firefox.com's download button spanned the
    /// hero from edge to edge rather than sitting centred, and its border radius
    /// turned the over-tall result into an oval.
    #[test]
    fn an_inline_flex_box_shrinks_to_its_contents() {
        let centred = text_runs(
            ".b{display:inline-flex;padding:0 32px}",
            "<div style=\"width:600px;text-align:center\"><a class=\"b\">HELLO</a></div>",
        );
        assert_eq!(centred.len(), 1, "{centred:?}");
        let wide = text_runs(
            ".b{display:flex;padding:0 32px}",
            "<div style=\"width:600px;text-align:center\"><a class=\"b\">HELLO</a></div>",
        );
        assert_eq!(wide.len(), 1, "{wide:?}");
        assert!(
            centred[0].x > wide[0].x,
            "the inline one is centred, the block one starts at the edge: {} vs {}",
            centred[0].x,
            wide[0].x
        );
        assert!(
            centred[0].x > 200,
            "and sits near the middle of 600px: {}",
            centred[0].x
        );
    }

    /// A percentage width on a flex item resolves against the flex container,
    /// and the item is then handed a slot of exactly that size. Resolving the
    /// percentage a second time against the slot shrinks it on every pass:
    /// firefox.com's footer columns ask for a quarter of a 1050px row and came
    /// out 31px wide, so every link stacked one character per line and the
    /// footer ran to six screens.
    #[test]
    fn a_flex_items_percentage_width_is_not_resolved_twice() {
        let runs = text_runs(
            ".row{display:flex;width:400px}.cell{width:25%}",
            "<div class=\"row\"><div class=\"cell\">A</div><div class=\"cell\">B</div>             <div class=\"cell\">C</div><div class=\"cell\">D</div></div>",
        );
        let xs: Vec<u32> = runs.iter().map(|run| run.x).collect();
        assert_eq!(xs, vec![0, 100, 200, 300], "quarters of 400px: {runs:?}");
    }

    /// A `<select>` shows one option at a time; the rest exist to be chosen
    /// from, not to be laid out. Rendering them as content spilled every entry
    /// onto the page -- firefox.com's footer language picker lists over a
    /// hundred, and it alone made the footer 6120px tall where a browser gives
    /// it 975px.
    #[test]
    fn a_selects_options_are_not_page_content() {
        let runs = text_runs(
            "",
            "<div style=\"width:600px\">before<select><option>ALPHA</option>             <option>BETA</option></select>after</div>",
        );
        let shown: String = runs.iter().map(|run| run.text.as_str()).collect();
        assert!(
            !shown.contains("ALPHA") && !shown.contains("BETA"),
            "no option should be laid out as text: {shown:?}"
        );
        assert!(
            shown.contains("before") && shown.contains("after"),
            "the text around it still is: {shown:?}"
        );
    }

    /// The anonymous box wrapping text in a flex container takes the inherited
    /// half of its parent's style and nothing else. Carrying the padding over
    /// charged it a second time one level down, so the text was laid out in a
    /// strip narrower than the width measured for it and wrapped where it fits
    /// exactly -- firefox.com's header menu titles each broke over two lines.
    #[test]
    fn an_anonymous_flex_item_does_not_repeat_its_parents_padding() {
        let runs = text_runs(
            ".row{display:flex;padding:0 8px;width:96px}",
            "<div style=\"width:600px\"><div class=\"row\">ブラウザー</div></div>",
        );
        assert_eq!(
            runs.len(),
            1,
            "the text fits the padded row on one line: {runs:?}"
        );
        assert_eq!(runs[0].x, 8, "and starts just inside the padding");
    }

    /// `text-indent` inherits, so a nested block starts its first line indented
    /// as well. Held as a non-inherited property it never reached the content
    /// it was meant to move.
    #[test]
    fn text_indent_reaches_a_nested_block() {
        let runs = text_runs(
            ".outer{text-indent:40px}",
            "<div style=\"width:600px\" class=\"outer\"><div>Firefox</div></div>",
        );
        assert_eq!(runs.len(), 1, "{runs:?}");
        assert_eq!(runs[0].x, 40, "the inherited indent must move the line");
    }

    /// A negative indent pushes the first line clear off the canvas. That is the
    /// image-replacement idiom -- `overflow:hidden` plus `text-indent:-9999px`
    /// -- which shows a logo as a background image while leaving real text in
    /// the markup for anyone not seeing it.
    #[test]
    fn a_negative_indent_pushes_the_line_off_the_canvas() {
        let runs = text_runs(
            ".logo{text-indent:-9999px;white-space:nowrap;overflow:hidden}",
            "<div style=\"width:600px\"><div class=\"logo\">Firefox</div></div>",
        );
        assert!(runs.is_empty(), "nothing should paint: {runs:?}");
    }

    /// Blockification is what lets that idiom work on a flex item: `text-indent`
    /// applies to block containers, and firefox.com's header logo link is an
    /// inline `<a>` that only becomes one by being an item of the header flex
    /// container.
    #[test]
    fn a_flex_item_is_blockified_enough_to_take_an_indent() {
        let runs = text_runs(
            ".row{display:flex}.logo{width:120px;overflow:hidden;text-indent:-9999px;white-space:nowrap}",
            "<div style=\"width:600px\" class=\"row\"><a class=\"logo\">Firefox</a></div>",
        );
        assert!(runs.is_empty(), "the flex item must take the indent: {runs:?}");
    }

    fn images_painted(css: &str, html: &str) -> usize {
        let doc = parse_document(html);
        let sheet = parse_stylesheet(css);
        let styled = build_styled_tree(&doc, &sheet, 1280, &InteractiveState::default());
        let mut images = ImageStore::default();
        images.insert(
            "logo.png".to_string(),
            crate::image::DecodedImage {
                width: 120,
                height: 40,
                rgba: vec![0; 120 * 40 * 4],
            },
        );
        let mut fonts = FontContext::load();
        layout_styled_document(&styled, &images, 1280, &mut fonts)
            .commands
            .iter()
            .filter(|command| matches!(command, DrawCommand::Image(_)))
            .count()
    }

    /// The indent has to move an image too, not just text. firefox.com draws its
    /// header logo as a background on the link *and* keeps an `<img>` of the
    /// same logo inside it; with the `<img>` left in place the two painted one
    /// on top of the other.
    #[test]
    fn a_negative_indent_hides_an_image_too() {
        const HTML: &str =
            "<div style=\"width:600px\"><div class=\"logo\"><img src=\"logo.png\" width=\"120\" height=\"40\"></div></div>";
        assert_eq!(
            images_painted(".logo{width:120px}", HTML),
            1,
            "control: an unindented image paints"
        );
        assert_eq!(
            images_painted(".logo{width:120px;overflow:hidden;text-indent:-9999px}", HTML),
            0,
            "an indented image must not paint"
        );
    }

    /// A real pseudo-class on the same shape must keep working.
    #[test]
    fn pseudo_classes_still_match_the_host() {
        let runs = text_runs(
            "div:first-child { max-width: 10%; }",
            "<div style=\"width:600px\"><div>ホームページに設定する</div></div>",
        );
        assert!(runs.len() > 1, "first-child should still apply: {runs:?}");
    }

    /// With `top` / `left` auto an absolutely positioned box keeps its *static
    /// position* -- where it would have sat in flow -- instead of jumping to the
    /// corner of its containing block.
    #[test]
    fn an_auto_offset_keeps_the_static_position() {
        let runs = text_runs(
            ".host{position:relative}.pin{position:absolute;left:0}",
            "<div class=\"host\" style=\"width:600px\">\
             <div style=\"height:40px\">\u{4e0a}</div>\
             <div class=\"pin\">\u{5370}</div></div>",
        );
        let pin = runs.iter().find(|run| run.text == "\u{5370}").expect("pin");
        assert!(
            pin.y >= 40,
            "the box stays below the block it follows: {pin:?}"
        );
    }

    /// Text sitting directly inside a flex container is a flex item too -- an
    /// anonymous one. Collecting only element children dropped it outright, so
    /// `<div style="display:flex">hello</div>` rendered empty.
    #[test]
    fn text_directly_inside_a_flex_container_is_not_dropped() {
        let runs = text_runs(
            ".row{display:flex}",
            "<div class=\"row\" style=\"width:600px\">\u{88f8}<span>\u{4ed8}</span></div>",
        );
        let shown: String = runs.iter().map(|run| run.text.as_str()).collect();
        assert!(shown.contains('\u{88f8}'), "the bare text must survive: {runs:?}");
        assert!(shown.contains('\u{4ed8}'));
    }

    /// A pseudo-class narrows what a selector matches, so dropping one that is
    /// not modelled *widens* it -- the opposite of what it says. Wikipedia
    /// scopes its edit-link brackets with `a:has(+ a.mw-editsection-visualeditor)`
    /// and, with `:has()` discarded, that became "every link on the page": a
    /// stray `]` was drawn after every menu entry and every row of the contents.
    #[test]
    fn an_unmodelled_pseudo_class_matches_nothing() {
        let runs = text_runs(
            "a:has(+ b)::after{content:\"X\"}",
            "<div><a>L</a></div>",
        );
        let shown: String = runs.iter().map(|run| run.text.as_str()).collect();
        assert_eq!(shown, "L", "the rule must not apply: {runs:?}");
    }

    /// `:is()` is a grouping construct, not a wildcard: it matches only what its
    /// argument matches.
    ///
    /// It used to be skipped over and its argument thrown away, which made
    /// `a:is(.x, .y)` behave like a bare `a`. On MDN that turned
    /// `:is(.homepage-hero h1)::after { content: "_" }` -- one heading's cursor --
    /// into `*::after`, stamping an underscore after the text of every element.
    #[test]
    fn a_grouping_pseudo_class_matches_only_its_argument() {
        let unmatched = text_runs(
            "a:is(.x, .y)::after{content:\"X\"}",
            "<div><a>L</a></div>",
        );
        let shown: String = unmatched.iter().map(|run| run.text.as_str()).collect();
        assert_eq!(shown, "L", "a class-less <a> must not match: {unmatched:?}");

        let matched = text_runs(
            "a:is(.x, .y)::after{content:\"X\"}",
            "<div><a class=\"y\">L</a></div>",
        );
        let shown: String = matched.iter().map(|run| run.text.as_str()).collect();
        assert_eq!(shown, "LX", "one of the alternatives must match: {matched:?}");
    }

    /// The argument may be a whole descendant chain, and may sit on its own with
    /// the rest of the compound after it -- the shape every one of MDN's uses has.
    #[test]
    fn a_grouping_pseudo_class_carries_a_descendant_chain() {
        let inside = text_runs(
            ":is(.hero h1)::after{content:\"X\"}",
            "<div class=\"hero\"><h1>L</h1></div>",
        );
        let shown: String = inside.iter().map(|run| run.text.as_str()).collect();
        assert_eq!(shown, "LX", "{inside:?}");

        let outside = text_runs(
            ":is(.hero h1)::after{content:\"X\"}",
            "<div><h1>L</h1><p>P</p></div>",
        );
        let shown: String = outside.iter().map(|run| run.text.as_str()).collect();
        assert_eq!(shown, "LP", "an h1 outside .hero must not match: {outside:?}");
    }

    /// The `content` value is a list that may end with `/ <string>`, which is
    /// alternative text for speech and is never drawn. Wikipedia writes its
    /// brackets as `content: ']' / ''`; stripping the outer quotes and keeping
    /// the rest drew `]' / '` after every one of them.
    #[test]
    fn content_alternative_text_is_not_drawn() {
        let runs = text_runs(
            "a::after{content:']' / 'closing bracket'}",
            "<div><a>L</a></div>",
        );
        let shown: String = runs.iter().map(|run| run.text.as_str()).collect();
        assert_eq!(shown, "L]", "{runs:?}");
    }

    /// Stylesheets write separators as escapes rather than embedding the
    /// characters: `\a0` is a non-breaking space.
    #[test]
    fn content_escapes_are_resolved() {
        let runs = text_runs(
            "a::after{content:\"\\a0 \\2022 \"}",
            "<div><a>L</a></div>",
        );
        let shown: String = runs.iter().map(|run| run.text.as_str()).collect();
        assert!(shown.contains('\u{2022}'), "expected a bullet, got {shown:?}");
        assert!(!shown.contains('\\'), "no backslash survives: {shown:?}");
    }

    /// `bottom` anchors a box to the bottom edge of its containing block. It
    /// cannot be resolved while the box is laid out -- the containing block's
    /// height is not known until its children are done, and the box is one of
    /// them -- so the box is drawn at its static position and moved once the
    /// block knows how tall it turned out. Ignoring `bottom` outright left
    /// Yahoo! JAPAN's "more" links at the top of its topics box.
    #[test]
    fn bottom_anchors_a_box_to_the_bottom_of_its_containing_block() {
        let runs = text_runs(
            ".host{position:relative}.pin{position:absolute;bottom:0;left:0}",
            "<div class=\"host\"><div style=\"height:200px\">\u{4e2d}</div>\
             <div class=\"pin\">\u{4e0b}</div></div>",
        );
        let pinned = runs.iter().find(|run| run.text == "\u{4e0b}").expect("pinned");
        let above = runs.iter().find(|run| run.text == "\u{4e2d}").expect("above");
        assert!(
            pinned.y > above.y + 100,
            "the box belongs at the foot of a 200px block: {runs:?}"
        );
    }

    /// A percentage offset resolves against the containing block -- `left` and
    /// `right` against its width -- not against the font size. The oldest way to
    /// centre a fixed-width box is `left: 50%` with a negative margin of half
    /// its width; resolved against a 14px font that read as `left: 7px`, and
    /// Yahoo! JAPAN's masthead logo sat against the left edge of the page with
    /// its -106px margin still applied.
    #[test]
    fn a_percentage_offset_resolves_against_the_containing_block() {
        let runs = text_runs(
            ".host{position:relative;width:400px}.pin{position:absolute;left:50%}",
            "<div class=\"host\"><span class=\"pin\">\u{5370}</span></div>",
        );
        let pin = runs.iter().find(|run| run.text == "\u{5370}").expect("pin");
        assert!(
            (190..=210).contains(&pin.x),
            "50% of a 400px block is 200px, got x={}",
            pin.x
        );
    }

    /// A negative offset still works -- it is an everyday value here, and the
    /// type that carries the percentage has to hold one.
    #[test]
    fn a_negative_offset_moves_the_box_back() {
        let runs = text_runs(
            ".host{position:relative;width:400px}.pin{position:absolute;left:50%;margin-left:-40px}",
            "<div class=\"host\"><span class=\"pin\">\u{5370}</span></div>",
        );
        let pin = runs.iter().find(|run| run.text == "\u{5370}").expect("pin");
        assert!(
            (150..=170).contains(&pin.x),
            "200px less a 40px margin, got x={}",
            pin.x
        );
    }

    /// An out-of-flow child is not a flex item: it takes no slot on the line and
    /// no share of the free space. Counting one gave it a slot -- Yahoo! JAPAN
    /// pins its masthead logo with `position: absolute`, and the logo's slot
    /// pushed the two groups of service shortcuts apart and sat between them.
    #[test]
    fn an_out_of_flow_child_takes_no_slot_in_a_flex_row() {
        let runs = text_runs(
            ".row{display:flex;position:relative}.pin{position:absolute;left:0;top:0}",
            "<div class=\"row\" style=\"width:600px\">\
             <div>\u{4e00}</div><div class=\"pin\">\u{5370}</div><div>\u{4e8c}</div></div>",
        );
        let first = runs.iter().find(|run| run.text == "\u{4e00}").expect("first");
        let second = runs.iter().find(|run| run.text == "\u{4e8c}").expect("second");
        assert!(
            second.x.saturating_sub(first.x) < 40,
            "the two items sit next to each other: {runs:?}"
        );
    }

    /// `margin-left: auto; margin-right: auto` centres a fixed-width box. Only
    /// the block path did this, so a flex container with `width: 990px;
    /// margin: 0 auto` -- Yahoo! JAPAN's masthead band -- was pinned to the left
    /// edge, 145px from where it belongs.
    #[test]
    fn a_flex_container_with_auto_margins_is_centred() {
        let runs = text_runs(
            ".band{display:flex;width:200px;margin-left:auto;margin-right:auto}",
            "<div style=\"width:600px\"><div class=\"band\">\u{5e2f}</div></div>",
        );
        let band = runs.iter().find(|run| run.text == "\u{5e2f}").expect("band");
        assert!(
            (190..=210).contains(&band.x),
            "a 200px band in 600px starts at 200px, got x={}",
            band.x
        );
    }

    /// A box that states its own width is that wide whatever is inside it --
    /// including when nothing is. An icon is an empty element sized by CSS and
    /// painted with a background image; measuring only its children reported one
    /// pixel, so every icon in Yahoo! JAPAN's service list collapsed and the
    /// label beside it wrapped for want of the space the icon should have held.
    #[test]
    fn an_empty_box_with_a_width_is_measured_by_that_width() {
        let runs = text_runs(
            ".row{display:flex}.icon{width:120px;height:20px}",
            "<div class=\"row\" style=\"width:600px\">\
             <div><span class=\"icon\"></span></div><div>\u{5f8c}</div></div>",
        );
        let after = runs.iter().find(|run| run.text == "\u{5f8c}").expect("label");
        assert!(
            after.x >= 120,
            "the empty icon holds its 120px, so the label starts past it: {after:?}"
        );
    }

    /// A box takes up its borders and margins as well as its padding. Reporting
    /// only content plus padding left each item a few pixels short, and a row of
    /// them added up to enough that the row no longer fitted -- Yahoo! JAPAN's
    /// top-right navigation separates items with a 1px rule and an 8px margin
    /// and lost 27px across four of them, which wrapped the first label.
    #[test]
    fn intrinsic_width_includes_borders_and_margins() {
        let runs = text_runs(
            ".row{display:flex}.sep{margin-left:20px;border-left:5px solid #000}",
            "<div class=\"row\" style=\"width:200px\">\
             <div>\u{4e00}</div><div class=\"sep\">\u{4e8c}</div>\
             <div class=\"sep\">\u{4e09}</div></div>",
        );
        let ys: Vec<u32> = runs.iter().map(|run| run.y).collect();
        assert!(
            ys.windows(2).all(|pair| pair[0] == pair[1]),
            "16 + 25 + 16 + 25 + 16 fits in 200px: {runs:?}"
        );
        let last = runs.iter().find(|run| run.text == "\u{4e09}").expect("third");
        assert!(
            last.x >= 82,
            "each separator contributes its margin and border: {last:?}"
        );
    }

    /// A flex item was measured by laying it out across the whole container and
    /// reading how far the paint reached. That counts the empty space
    /// `text-align: center` leaves *before* the content, so a centred label came
    /// out about half the container wide. rust-lang.org centres every
    /// navigation label, and its eight links each claimed half the page.
    #[test]
    fn a_centred_flex_item_is_measured_by_its_content() {
        let runs = text_runs(
            ".row{display:flex}.cell{text-align:center}",
            "<div class=\"row\" style=\"width:600px\">\
             <div class=\"cell\">\u{4e00}</div><div class=\"cell\">\u{4e8c}</div>\
             <div class=\"cell\">\u{4e09}</div></div>",
        );
        let first = runs.iter().find(|run| run.text == "\u{4e00}").expect("first");
        let last = runs.iter().find(|run| run.text == "\u{4e09}").expect("last");
        assert_eq!(first.y, last.y, "all three belong on one line: {runs:?}");
        assert!(
            last.x < 200,
            "each item is one glyph wide, so the third starts early: {runs:?}"
        );
    }

    /// A flex row's max-content width is the sum of its items, whatever each
    /// item's own `display` says. Breaking a line at every block-level child
    /// measured rust-lang.org's eight navigation links at 149px between them,
    /// and the list wrapped to one link a line.
    #[test]
    fn a_flex_rows_intrinsic_width_is_the_sum_of_its_items() {
        let runs = text_runs(
            ".outer{display:flex}.row{display:flex}.item{display:block}",
            "<div class=\"outer\" style=\"width:600px\"><div class=\"row\">\
             <div class=\"item\">\u{4e00}</div><div class=\"item\">\u{4e8c}</div>\
             <div class=\"item\">\u{4e09}</div></div></div>",
        );
        let ys: Vec<u32> = runs.iter().map(|run| run.y).collect();
        assert!(
            ys.windows(2).all(|pair| pair[0] == pair[1]),
            "the inner row must not wrap: {runs:?}"
        );
    }

    /// `flex-shrink` was parsed and then never read: an overflowing row shrank
    /// every item proportionally, whatever each one asked for. Yahoo! JAPAN's
    /// topics column says `flex: 1 0 240px` -- at least 240px, never shrink --
    /// and was squeezed to 132px, which wrapped every headline onto three lines.
    #[test]
    fn flex_shrink_zero_keeps_an_items_width() {
        const HEADLINE: &str = "\u{3042}\u{3044}\u{3046}\u{3048}\u{304a}\u{304b}\u{304d}\u{304f}\u{3051}\u{3053}";
        let html = format!(
            "<div class=\"row\" style=\"width:300px\">\
             <div class=\"fixed\">{HEADLINE}</div>\
             <div class=\"rest\">{HEADLINE}{HEADLINE}{HEADLINE}</div></div>"
        );
        let runs = text_runs(".row{display:flex}.fixed{flex:1 0 240px}", &html);
        let headline = runs
            .iter()
            .find(|run| run.text == HEADLINE)
            .unwrap_or_else(|| panic!("the fixed item must not wrap: {runs:?}"));
        assert!(
            headline.width >= 160,
            "the fixed item keeps its 240px slot: {headline:?}"
        );
    }

    /// Flex arithmetic works in margin boxes, and an item's margins were counted
    /// twice: once inside its measured base size and again in the row's total.
    /// A row that fitted was shrunk anyway -- Yahoo! JAPAN's top-right
    /// navigation lost 25px that way and wrapped its first label onto a second
    /// line.
    #[test]
    fn flex_item_margins_are_counted_once() {
        let runs = text_runs(
            ".row{display:flex}.b{margin-left:40px}",
            "<div class=\"row\" style=\"width:280px\">\
             <span class=\"a\">\u{30db}\u{30fc}\u{30e0}\u{30da}\u{30fc}\u{30b8}\u{306b}\u{8a2d}\u{5b9a}\u{3059}\u{308b}</span>\
             <span class=\"b\">\u{30d8}\u{30eb}\u{30d7}</span></div>",
        );
        let texts: Vec<&str> = runs.iter().map(|run| run.text.as_str()).collect();
        assert_eq!(
            texts,
            vec!["\u{30db}\u{30fc}\u{30e0}\u{30da}\u{30fc}\u{30b8}\u{306b}\u{8a2d}\u{5b9a}\u{3059}\u{308b}", "\u{30d8}\u{30eb}\u{30d7}"],
            "the row fits in 280px, so nothing may wrap"
        );
    }

    /// Only `layout_node` dispatched on `display`; a flex container placed its
    /// items by calling the block path directly, so a flex box nested inside
    /// another was laid out as a plain block and stacked its children. Yahoo!
    /// JAPAN's masthead is exactly that shape, and its two service groups sat
    /// one above the other instead of side by side.
    #[test]
    fn a_flex_container_nested_in_a_flex_row_still_lays_out_as_flex() {
        let runs = text_runs(
            ".outer{display:flex}.inner{display:flex}",
            "<div class=\"outer\" style=\"width:600px\"><div class=\"inner\">\
             <span>\u{4e00}</span><span>\u{4e8c}</span></div></div>",
        );
        let first = runs.iter().find(|run| run.text == "\u{4e00}").expect("first");
        let second = runs.iter().find(|run| run.text == "\u{4e8c}").expect("second");
        assert_eq!(
            first.y, second.y,
            "the inner container is a flex row, not a block: {runs:?}"
        );
    }

    /// `overflow: hidden` was only ever clipped on the block path. Several of
    /// Yahoo! JAPAN's screen-reader-only headings are flex containers, so once
    /// they stopped being mistaken for blocks they printed down the left edge.
    #[test]
    fn a_flex_container_clips_overflow_hidden() {
        let runs = text_runs(
            ".vh{display:flex;position:absolute;width:1px;height:1px;overflow:hidden}",
            "<div style=\"width:600px\"><div class=\"vh\">\
             <span>\u{30ad}\u{30fc}\u{30ef}\u{30fc}\u{30c9}\u{5165}\u{529b}\u{88dc}\u{52a9}</span></div></div>",
        );
        assert!(
            runs.is_empty(),
            "a 1px box has room for no glyph at all: {runs:?}"
        );
    }

    /// `flex-direction: row-reverse` lays items out from the far edge, which
    /// also swaps what `flex-start` and `flex-end` mean. Neither was
    /// implemented: Yahoo! JAPAN uses `row-reverse` with `justify-content:
    /// flex-end` to put the icon before the label in its service list, and
    /// without the flip the whole left rail came out right-aligned.
    #[test]
    fn a_reverse_row_runs_from_the_far_edge() {
        let runs = text_runs(
            ".row{display:flex;flex-direction:row-reverse;justify-content:flex-end}",
            "<div class=\"row\" style=\"width:600px\">\
             <span>\u{4e00}</span><span>\u{4e8c}</span></div>",
        );
        let first = runs.iter().find(|run| run.text == "\u{4e00}").expect("first");
        let second = runs.iter().find(|run| run.text == "\u{4e8c}").expect("second");
        assert!(
            second.x < first.x,
            "document order is reversed on screen: {runs:?}"
        );
        assert!(
            second.x < 60,
            "flex-end packs against the left edge in a reverse row: {runs:?}"
        );
    }

    /// `order` re-sequences flex items without touching the document.
    #[test]
    fn the_order_property_resequences_flex_items() {
        let runs = text_runs(
            ".row{display:flex}.a{order:2}.b{order:1}",
            "<div class=\"row\" style=\"width:600px\">\
             <span class=\"a\">\u{5f8c}</span><span class=\"b\">\u{5148}</span></div>",
        );
        let later = runs.iter().find(|run| run.text == "\u{5f8c}").expect("a");
        let earlier = runs.iter().find(|run| run.text == "\u{5148}").expect("b");
        assert!(
            earlier.x < later.x,
            "the lower `order` comes first: {runs:?}"
        );
    }

    /// A block box breaks a line only at a block-level child, so its max-content
    /// width is that of its widest *line*. Measuring it as the widest single
    /// child reported the widest word instead: Yahoo! JAPAN's weather panel puts
    /// `38` and the degree sign in one block, which measured 10px, so
    /// shrink-to-fit laid the temperatures out one character to a line.
    #[test]
    fn max_content_width_measures_whole_lines() {
        let runs = text_runs(
            ".card{display:inline-block}",
            "<div style=\"width:600px\"><span class=\"card\">\
             <div><span>38</span><span>\u{2103}</span></div></span></div>",
        );
        let value = runs.iter().find(|run| run.text.contains("38")).expect("value");
        let unit = runs
            .iter()
            .find(|run| run.text.contains('\u{2103}'))
            .expect("unit");
        assert_eq!(value.y, unit.y, "both belong on one line: {runs:?}");
    }

    /// `display: inline-block` was collapsed to plain `inline`. An inline
    /// formatting context drops block-level children, so everything nested
    /// inside such a wrapper was deleted outright -- which is how Yahoo!
    /// JAPAN's news headlines went missing: each sits under an
    /// `<article style="display:inline-block">`.
    #[test]
    fn an_inline_block_keeps_its_block_children() {
        let runs = text_runs(
            ".card{display:inline-block}.body{display:block}",
            "<span class=\"card\"><div class=\"body\">見出し</div></span>",
        );
        let shown: String = runs.iter().map(|run| run.text.as_str()).collect();
        assert_eq!(shown, "見出し", "got {runs:?}");
    }

    /// An `inline-block` has to hold its content *and* its own padding: the
    /// width it is laid out at is an outer width, and the block path carves the
    /// padding out of it again. Sizing the box to the bare content width left
    /// the content short by exactly the padding -- Yahoo! JAPAN's weather badge
    /// is 60px of text in a box with 6px either side, got 48px, and wrapped its
    /// last character onto a second line.
    #[test]
    fn an_inline_block_reserves_room_for_its_own_padding() {
        let runs = text_runs(
            ".badge{display:inline-block;padding:0 6px}",
            "<div style=\"width:600px\"><span class=\"badge\">\
             \u{6975}\u{3081}\u{3066}\u{5371}\u{967a}</span></div>",
        );
        let texts: Vec<&str> = runs.iter().map(|run| run.text.as_str()).collect();
        assert_eq!(
            texts,
            vec!["\u{6975}\u{3081}\u{3066}\u{5371}\u{967a}"],
            "the badge must not wrap"
        );
    }

    /// Margins count towards an `inline-block`'s width for the same reason its
    /// padding does: the width it is laid out at is the margin box. Yahoo!
    /// JAPAN's weather headings carry 12px of side margins, and without them
    /// the heading lost twelve pixels and dropped its last character.
    #[test]
    fn an_inline_block_reserves_room_for_its_own_margins() {
        let runs = text_runs(
            ".head{display:inline-block;margin-left:7px;margin-right:5px}",
            "<div style=\"width:600px\"><span class=\"head\">\
             \u{5730}\u{57df}\u{60c5}\u{5831}</span></div>",
        );
        let texts: Vec<&str> = runs.iter().map(|run| run.text.as_str()).collect();
        assert_eq!(texts, vec!["\u{5730}\u{57df}\u{60c5}\u{5831}"], "must not wrap");
    }

    /// It stays inline-level on the outside: two of them sit on one line.
    #[test]
    fn inline_blocks_sit_side_by_side() {
        let runs = text_runs(
            ".card{display:inline-block}",
            "<div style=\"width:600px\">\
             <span class=\"card\">左</span><span class=\"card\">右</span></div>",
        );
        let left = runs.iter().find(|run| run.text == "左").expect("left");
        let right = runs.iter().find(|run| run.text == "右").expect("right");
        assert_eq!(left.y, right.y, "both belong on the same line: {runs:?}");
        assert!(right.x > left.x, "the second follows the first: {runs:?}");
    }

    /// Its children still lay out as blocks, one under the next.
    #[test]
    fn blocks_inside_an_inline_block_stack() {
        let runs = text_runs(
            ".card{display:inline-block}",
            "<span class=\"card\"><div>上</div><div>下</div></span>",
        );
        let top = runs.iter().find(|run| run.text == "上").expect("top");
        let bottom = runs.iter().find(|run| run.text == "下").expect("bottom");
        assert!(bottom.y > top.y, "the second block goes below: {runs:?}");
    }

    /// A percentage inside `calc()` resolves against the containing block, like
    /// any other percentage. It used to be resolved at parse time against the
    /// font size, so `calc(47.47475% - 20px)` -- the width of Yahoo! JAPAN's
    /// centre column -- came out as a single pixel and the column to its right
    /// was drawn on top of it.
    #[test]
    fn calc_percentages_resolve_against_the_containing_block() {
        let runs = text_runs(
            ".col{width:calc(50% - 20px)}",
            "<div style=\"width:600px\"><div class=\"col\">\
             ホームページに設定する</div></div>",
        );
        // 280px fits the 198px string on one line; a font-size-relative
        // resolution would give single digits and wrap it to one glyph a line.
        assert_eq!(runs.len(), 1, "expected one line, got {runs:?}");
    }

    /// The offset really is subtracted -- the percentage is not just passed
    /// through on its own.
    #[test]
    fn a_calc_offset_narrows_the_box() {
        let runs = text_runs(
            ".col{width:calc(100% - 480px)}",
            "<div style=\"width:600px\"><div class=\"col\">\
             ホームページに設定する</div></div>",
        );
        assert!(
            runs.len() > 1,
            "120px must wrap the 198px string, got {runs:?}"
        );
    }

    /// A flex item is bounded by `min-width` like any other box. Yahoo! JAPAN's
    /// centre column asks for `calc(47.47475% - 20px)` but insists on
    /// `min-width: 450px`; without the clamp it collapsed.
    #[test]
    fn flex_items_are_bounded_by_min_width() {
        let runs = text_runs(
            ".row{display:flex}.narrow{width:10px;min-width:300px}",
            "<div class=\"row\" style=\"width:600px\">\
             <div class=\"narrow\">ホームページに設定する</div>\
             <div>後</div></div>",
        );
        let after = runs.iter().find(|run| run.text == "後").expect("second item");
        assert!(
            after.x >= 300,
            "the second item must start past the clamped first one, got x={}",
            after.x
        );
    }

    /// An absolutely positioned box is placed against its nearest *positioned*
    /// ancestor. Nothing established that containing block, so every such box
    /// on every page was laid out from the page origin: Yahoo! JAPAN's
    /// trending-list rank badges all landed on the masthead at y = 0.
    #[test]
    fn absolute_boxes_are_placed_against_their_positioned_ancestor() {
        let runs = text_runs(
            ".host{position:relative}.pin{position:absolute;top:0;left:0}",
            "<div style=\"height:50px\">spacer</div>\
             <div class=\"host\"><span class=\"pin\">badge</span></div>",
        );
        let badge = runs.iter().find(|run| run.text == "badge").expect("badge");
        assert!(
            badge.y >= 50,
            "the badge belongs below the spacer, not at the page origin: {badge:?}"
        );
    }

    /// An out-of-flow box nested inside an inline box used to disappear: every
    /// block-level box in an inline formatting context is dropped, and
    /// blockification makes absolutely positioned boxes block-level, so the two
    /// rules combined to delete the content outright.
    ///
    /// Where such a box *lands* is a separate matter. An inline box has no
    /// position until line breaking has run, and out-of-flow boxes are laid out
    /// during fragment collection, before that -- so an inline
    /// `position: relative` ancestor does not yet act as the containing block,
    /// and the box falls back to the nearest block-level positioned one.
    #[test]
    fn an_out_of_flow_box_inside_an_inline_box_is_not_dropped() {
        let runs = text_runs(
            ".host{position:relative}.pin{position:absolute;top:0}",
            "<div style=\"height:40px\">spacer</div>\
             <span class=\"host\"><span class=\"pin\">badge</span></span>",
        );
        assert!(
            runs.iter().any(|run| run.text == "badge"),
            "the positioned box must still be drawn: {runs:?}"
        );
    }

    /// CSS 2.1 10.3.7: with `width: auto` an absolutely positioned box shrinks
    /// to fit. Filling the containing block instead made `text-align: center`
    /// centre the content against the whole page.
    #[test]
    fn an_auto_width_absolute_box_shrinks_to_fit() {
        let runs = text_runs(
            ".host{position:relative}\
             .pin{position:absolute;top:0;left:0;min-width:16px;text-align:center}",
            "<div style=\"width:600px\" class=\"host\"><span class=\"pin\">1</span></div>",
        );
        let digit = runs.iter().find(|run| run.text == "1").expect("digit");
        assert!(
            digit.x < 16,
            "a shrink-to-fit box is 16px wide, so its centred digit stays near \
             its left edge, got x={}",
            digit.x
        );
    }

    /// `right` places the box's right edge. It was ignored outright, which
    /// pinned every right-anchored box to the left edge instead.
    #[test]
    fn the_right_offset_places_the_right_edge() {
        let runs = text_runs(
            ".host{position:relative}.pin{position:absolute;top:0;right:0}",
            "<div style=\"width:600px\" class=\"host\"><span class=\"pin\">edge</span></div>",
        );
        let pinned = runs.iter().find(|run| run.text == "edge").expect("edge");
        assert!(
            pinned.x + pinned.width >= 560,
            "expected the box against the right edge of a 600px block, got \
             x={} width={}",
            pinned.x,
            pinned.width
        );
    }

    /// `rem` is relative to the root element's computed font size, not to a
    /// constant. `html { font-size: 62.5% }` -- so that `1.4rem` reads as
    /// "14px" -- is one of the most widespread idioms in production CSS, and a
    /// hardcoded 16px basis inflated every length on such a page by 1.6x. On
    /// Yahoo! JAPAN it turned 12px tab labels into 19px ones that overlapped.
    #[test]
    fn rem_resolves_against_the_root_font_size() {
        let runs = text_runs(
            "html { font-size: 62.5%; } p { font-size: 1.2rem; }",
            "<html><body><p>hello</p></body></html>",
        );
        assert_eq!(runs[0].font_size_px, 12);
    }

    /// A page that leaves the root alone still gets the initial 16px basis.
    #[test]
    fn rem_falls_back_to_the_initial_font_size() {
        let runs = text_runs(
            "p { font-size: 1.5rem; }",
            "<html><body><p>hello</p></body></html>",
        );
        assert_eq!(runs[0].font_size_px, 24);
    }

    /// The basis must not leak from one document into the next: these run on
    /// the same thread, and the root font size lives in thread-local state.
    #[test]
    fn the_rem_basis_does_not_leak_between_documents() {
        let shrunk = text_runs(
            "html { font-size: 50%; } p { font-size: 2rem; }",
            "<html><body><p>hello</p></body></html>",
        );
        assert_eq!(shrunk[0].font_size_px, 16);
        let plain = text_runs(
            "p { font-size: 2rem; }",
            "<html><body><p>hello</p></body></html>",
        );
        assert_eq!(plain[0].font_size_px, 32, "the previous root leaked");
    }

    /// `list-style: none` is how every navigation menu on the web keeps `<ul>`
    /// markup for structure while dropping the bullets on screen. The computed
    /// `list-style-type` was parsed but never read by layout, so each item got a
    /// hardcoded `"- "` and Yahoo! JAPAN's nav rendered as a column of dashes.
    #[test]
    fn list_style_none_suppresses_the_marker() {
        let runs = text_runs(
            "ul { list-style: none; }",
            "<ul><li>Home</li><li>Help</li></ul>",
        );
        let texts: Vec<&str> = runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(texts, vec!["Home", "Help"], "no marker may be drawn");
    }

    /// A list that does not opt out still gets its bullet -- and it is the disc
    /// the spec asks for, not a hyphen.
    #[test]
    fn a_default_list_item_keeps_its_disc() {
        let runs = text_runs("", "<ul><li>plain</li></ul>");
        let texts: Vec<&str> = runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(texts, vec!["\u{2022} plain"], "expected a disc marker");
    }

    /// `list-style-type: decimal` numbers the items of its own container, so a
    /// nested list restarts at 1 rather than continuing the outer count.
    #[test]
    fn decimal_markers_are_numbered_per_container() {
        let runs = text_runs(
            "ol { list-style-type: decimal; }",
            "<ol><li>a</li><li>b<ol><li>inner</li></ol></li><li>c</li></ol>",
        );
        let texts: Vec<&str> = runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(texts, vec!["1. a", "2. b", "1. inner", "3. c"]);
    }

    /// The marker only indents content when there is a marker to make room for.
    #[test]
    fn a_markerless_item_does_not_reserve_marker_space() {
        let with_marker = text_runs("", "<ul><li>plain</li></ul>");
        let without = text_runs("ul { list-style: none; }", "<ul><li>plain</li></ul>");
        assert_eq!(
            without[0].x + MARKER_INDENT,
            with_marker[0].x,
            "suppressing the marker must also drop its indent"
        );
    }

}


#[cfg(test)]
mod tests {
    use super::{DrawCommand, layout_styled_document};
    use crate::css::{TextAlign, build_styled_tree, parse_stylesheet};
    use crate::font::FontContext;
    use crate::html::parse_document;
    use crate::image::{DecodedImage, ImageStore};

    // Empirical layout probe: runs real-world CSS layout patterns through the
    // full parse → style → layout pipeline and checks the resulting geometry, to
    // surface flexbox/positioning/box-model gaps. Diagnostic — prints a report,
    // never fails. Run: cargo test --bin tobira layout_probe -- --nocapture
    /// Regression: form controls inside a flex container (and a flex `<li>`) must
    /// still register as interactive `FormControlCommand`s. Before the fix, flex
    /// children were laid out via `layout_block_element` (block path), which never
    /// emitted controls, so buttons/inputs inside `display:flex` painted but were
    /// dead to hit-testing (the React demo's counter / todo controls).
    #[test]
    fn flex_child_controls_are_registered() {
        let l = probe_layout(
            r#"<html><body><button>nonflex</button>
               <div style="display:flex;align-items:center"><button>minus</button><span>0</span><button>plus</button><button>reset</button></div>
               <div style="display:flex"><input><button>add</button></div>
               <ul><li style="display:flex;justify-content:space-between"><span>item one</span><button>del</button></li></ul></body></html>"#,
            640,
        );
        let labels: Vec<&str> = l.controls.iter().map(|c| c.label.as_str()).collect();
        for want in ["nonflex", "minus", "plus", "reset", "add", "del"] {
            assert!(
                labels.contains(&want),
                "control {want:?} not registered (flex controls dead to hit-test); got {labels:?}"
            );
        }
        // The flex <input> must register as a TextInput control too.
        assert!(
            l.controls.iter().any(|c| matches!(c.kind, super::FormControlKind::TextInput)),
            "flex <input> did not register as a text-input control"
        );
    }

    /// Regression: flex items without an explicit width / flex-basis are
    /// content-sized (default flex-grow: 0), not stretched to fill. A row of
    /// buttons packs left; a `space-between` row keeps its last item inside the
    /// container. Before the fix, every auto item got `remaining / n`, spreading
    /// the demo's buttons out and pushing space-between items off-screen.
    #[test]
    fn flex_items_are_content_sized_not_stretched() {
        let l = probe_layout(
            r#"<html><body><div style="display:flex;align-items:center"><button>minus</button><span>0</span><button>plus</button><button>reset</button></div>
               <ul><li style="display:flex;justify-content:space-between"><span>item one</span><button>del</button></li></ul></body></html>"#,
            640,
        );
        let by_label = |label: &str| -> super::FormControlCommand {
            l.controls.iter().find(|c| c.label == label).cloned()
                .unwrap_or_else(|| panic!("control {label:?} missing"))
        };
        let minus = by_label("minus");
        let plus = by_label("plus");
        let reset = by_label("reset");
        // Packed left in order, each only as wide as its content (< 120px).
        assert!(minus.x < plus.x && plus.x < reset.x, "buttons not in row order: {} {} {}", minus.x, plus.x, reset.x);
        assert!(plus.x < 200, "buttons stretched/spread: plus.x={}", plus.x);
        assert!(minus.width < 120 && plus.width < 120, "button too wide: {} {}", minus.width, plus.width);
        // space-between: the delete button stays within the 640px container.
        let del = by_label("del");
        assert!(del.x + del.width <= 640, "space-between item off-screen: del.x={} w={}", del.x, del.width);
        assert!(del.x > 400, "space-between item not pushed right: del.x={}", del.x);
    }

    /// Diagnostic: dump form-control + element-hitbox geometry for flex-laid-out
    /// buttons/inputs, to check whether interactive controls inside a flex
    /// container get their flex position (so hit-test matches paint).
    /// Run: cargo test --bin tobira probe_flex_control_geometry -- --nocapture
    #[test]
    fn probe_flex_control_geometry() {
        // Mirrors the React demo's counter row and todo row (flex containers).
        let l = probe_layout(
            r#"<html><body><button>nonflex</button><div style="display:flex;align-items:center"><button>minus</button><span>0</span><button>plus</button><button>reset</button></div>
               <ul><li style="display:flex;justify-content:space-between"><span>item one</span><button>del</button></li></ul></body></html>"#,
            640,
        );
        println!("=== controls (hit-test targets): {} ===", l.controls.len());
        for c in &l.controls {
            println!(
                "kind={:?} label={:?} x={} y={} w={} h={} node_id={:?}",
                c.kind, c.label, c.x, c.y, c.width, c.height, c.node_id
            );
        }
        println!("=== element_hitboxes: {} ===", l.element_hitboxes.len());
        for h in &l.element_hitboxes {
            println!(
                "node_id={} x={} y={} w={} h={}",
                h.node_id, h.x, h.y, h.width, h.height
            );
        }
        println!("=== all rects (paint): {} ===", l.commands.len());
        for cmd in &l.commands {
            if let DrawCommand::Rect(r) = cmd {
                println!("rect x={} y={} w={} h={} color=#{:06x}", r.x, r.y, r.width, r.height, r.color & 0xFFFFFF);
            }
        }
    }

    /// Regression: a `button:hover` rule recolors the control when that button
    /// is the hovered node. Controls are not in `element_hitboxes`, so the GUI
    /// resolves hover against the controls list; here we confirm the CSS side —
    /// styling the tree with the button marked hovered flows its :hover
    /// background into the emitted FormControlCommand (native_chrome stays false
    /// so the renderer won't override it with the gray chrome hover).
    #[test]
    fn button_hover_rule_recolors_control() {
        let html = r#"<html><body><button data-tobira-node-id="2">Go</button></body></html>"#;
        let css = "button { background: #3457d5; color: #fff; border: 1px solid #3457d5; } button:hover { background: #2742a8; }";
        let document = parse_document(html);
        let mut fonts = FontContext::load();

        let normal = {
            let styled = build_styled_tree(&document, &parse_stylesheet(css), 700, &crate::css::InteractiveState::default());
            layout_styled_document(&styled, &ImageStore::default(), 700, &mut fonts)
        };
        let nb = normal.controls.iter().find(|c| c.label == "Go").expect("button");
        assert_eq!(nb.background_color, 0x3457D5, "resting background from CSS");
        assert!(!nb.native_chrome, "CSS-styled button must not be native chrome");

        let hovered = {
            let interactive = crate::css::InteractiveState { hovered_node_id: Some(2), ..Default::default() };
            let styled = build_styled_tree(&document, &parse_stylesheet(css), 700, &interactive);
            layout_styled_document(&styled, &ImageStore::default(), 700, &mut fonts)
        };
        let hb = hovered.controls.iter().find(|c| c.label == "Go").expect("button");
        assert_eq!(hb.background_color, 0x2742A8, "hover background from :hover rule");
    }

    /// Diagnostic: replicate the React demo's counter section exactly (flex row
    /// with −/＋/リセット buttons + a count span, with the demo's real CSS) and
    /// dump every text command + control, to locate the stray "−" rendered above
    /// the row. Run: cargo test --bin tobira probe_demo_counter_row -- --nocapture
    #[test]
    fn probe_demo_counter_row() {
        let html = r#"<html><head><style>
            section { border: 1px solid #e2e2e2; border-radius: 10px; padding: 16px 18px; margin: 16px 0; background: #fff; }
            h2 { font-size: 17px; margin: 0 0 10px; }
            button { font-size: 15px; padding: 8px 14px; border: 1px solid #3457d5; background: #3457d5; color: #fff; border-radius: 6px; cursor: pointer; margin-right: 6px; }
            button.ghost { background: #fff; color: #3457d5; }
            .count { font-size: 28px; font-weight: 700; margin: 0 12px; }
        </style></head><body>
            <section><h2>① カウンター (useState + onClick)</h2><div style="display: flex; align-items: center"><button>−</button><span class="count">4</span><button>＋</button><button class="ghost">リセット</button></div></section>
        </body></html>"#;
        // Apply the demo's real CSS (probe_layout uses an empty stylesheet).
        let css = r#"
            section { border: 1px solid #e2e2e2; border-radius: 10px; padding: 16px 18px; margin: 16px 0; background: #fff; }
            h2 { font-size: 17px; margin: 0 0 10px; }
            button { font-size: 15px; padding: 8px 14px; border: 1px solid #3457d5; background: #3457d5; color: #fff; border-radius: 6px; cursor: pointer; margin-right: 6px; }
            button.ghost { background: #fff; color: #3457d5; }
            .count { font-size: 28px; font-weight: 700; margin: 0 12px; }
        "#;
        let document = parse_document(html);
        let styled = build_styled_tree(
            &document,
            &parse_stylesheet(css),
            1280,
            &crate::css::InteractiveState::default(),
        );
        let mut fonts = FontContext::load();
        let l = layout_styled_document(&styled, &ImageStore::default(), 700, &mut fonts);
        fn dump(cmds: &[super::DrawCommand], depth: usize) {
            for cmd in cmds {
                match cmd {
                    super::DrawCommand::Text(t) => println!(
                        "{:indent$}TEXT {:?} x={} y={} w={} size={}",
                        "", t.text, t.x, t.y, t.width, t.font_size_px, indent = depth * 2
                    ),
                    super::DrawCommand::Layer(layer) => {
                        println!("{:indent$}LAYER x={} y={}", "", layer.x, layer.y, indent = depth * 2);
                        dump(&layer.commands, depth + 1);
                    }
                    _ => {}
                }
            }
        }
        dump(&l.commands, 0);
        for c in &l.controls {
            println!(
                "CONTROL {:?} label={:?} x={} y={} w={} h={} font={} bg=#{:06x} text=#{:06x} border=#{:06x}",
                c.kind, c.label, c.x, c.y, c.width, c.height, c.font_size_px,
                c.background_color, c.text_color, c.border_color
            );
        }
        // CSS-authored colors must reach the control: the demo's blue buttons
        // (background #3457d5, white text, border #3457d5) and the ghost variant
        // (white bg, blue text).
        let minus = l.controls.iter().find(|c| c.label == "−").expect("minus button");
        assert_eq!(minus.background_color, 0x3457D5, "button background from CSS");
        assert_eq!(minus.text_color, 0xFFFFFF, "button text color from CSS");
        assert_eq!(minus.border_color, 0x3457D5, "button border from CSS");
        let ghost = l.controls.iter().find(|c| c.label == "リセット").expect("ghost button");
        assert_eq!(ghost.background_color, 0xFFFFFF, "ghost background from CSS");
        assert_eq!(ghost.text_color, 0x3457D5, "ghost text from CSS");
    }

    #[test]
    fn block_stack_with_padding_places_children_below_padding() {
        let l = probe_layout(
            r#"<html><body style="margin:0"><div style="padding:10px;background:#bb0001"><div style="height:20px;background:#bb0002"></div><div style="height:20px;background:#bb0003"></div></div></body></html>"#,
            400,
        );
        let first = probe_rect(&l, 0xBB0002).expect("first rect");
        let second = probe_rect(&l, 0xBB0003).expect("second rect");
        assert_eq!(first.y, 10, "first child should start after 10px padding");
        assert!(second.y >= first.y + first.height, "second child should stack below the first");
    }

    #[test]
    fn block_width_percent_resolves_against_parent_width() {
        let l = probe_layout(
            r#"<html><body style="margin:0"><div style="width:400px"><div style="width:50%;height:20px;background:#bb0004"></div></div></body></html>"#,
            400,
        );
        let box_ = probe_rect(&l, 0xBB0004).expect("percent-width rect");
        assert!((box_.width as i32 - 200).abs() <= 2, "50% of 400px should be about 200px, got {}", box_.width);
    }

    #[test]
    fn padding_left_offsets_child_content_start() {
        let l = probe_layout(
            r#"<html><body style="margin:0"><div style="padding-left:20px;background:#bb0005"><div style="width:30px;height:20px;background:#bb0006"></div></div></body></html>"#,
            400,
        );
        let child = probe_rect(&l, 0xBB0006).expect("child rect");
        assert_eq!(child.x, 20, "padding-left should shift child content start");
    }

    #[test]
    fn flex_row_places_items_side_by_side() {
        let l = probe_layout(
            r#"<html><body style="margin:0"><div style="display:flex;background:#bb0007"><div style="width:80px;height:20px;background:#bb0008"></div><div style="width:80px;height:20px;background:#bb0009"></div></div></body></html>"#,
            400,
        );
        let a = probe_rect(&l, 0xBB0008).expect("first flex item");
        let b = probe_rect(&l, 0xBB0009).expect("second flex item");
        assert_eq!(a.x, 0, "flex row should start at x=0");
        assert!((b.x as i32 - (a.x + 80) as i32).abs() <= 2, "second item should follow the first horizontally");
    }

    #[test]
    fn flex_gap_is_reflected_between_items() {
        let l = probe_layout(
            r#"<html><body style="margin:0"><div style="display:flex;gap:10px;background:#bb000a"><div style="width:60px;height:20px;background:#bb000b"></div><div style="width:60px;height:20px;background:#bb000c"></div></div></body></html>"#,
            400,
        );
        let a = probe_rect(&l, 0xBB000B).expect("first flex item");
        let b = probe_rect(&l, 0xBB000C).expect("second flex item");
        assert!((b.x as i32 - (a.x + a.width + 10) as i32).abs() <= 2, "flex gap should add 10px between items");
    }

    #[test]
    fn grid_template_columns_places_cells_on_expected_tracks() {
        let l = probe_layout(
            r#"<html><body style="margin:0"><div style="display:grid;width:200px;grid-template-columns:100px 100px;background:#bb000d"><div style="height:20px;background:#bb000e"></div><div style="height:20px;background:#bb000f"></div></div></body></html>"#,
            400,
        );
        let a = probe_rect(&l, 0xBB000E).expect("first grid cell");
        let b = probe_rect(&l, 0xBB000F).expect("second grid cell");
        assert_eq!(a.x, 0, "first grid cell should start at x=0");
        assert!((b.x as i32 - 100).abs() <= 2, "second grid cell should start at x=100, got {}", b.x);
    }

    #[test]
    fn position_absolute_is_placed_relative_to_nearest_positioned_parent() {
        let l = probe_layout(
            r#"<html><body style="margin:0"><div style="position:relative;width:200px;height:120px;background:#bb0010"><div style="position:absolute;left:30px;top:40px;width:40px;height:20px;background:#bb0011"></div></div></body></html>"#,
            400,
        );
        let child = probe_rect(&l, 0xBB0011).expect("absolute child");
        assert_eq!(child.x, 30, "absolute child x should be relative to parent");
        assert_eq!(child.y, 40, "absolute child y should be relative to parent");
    }

    /// A box parked entirely above the page origin is not drawn.
    ///
    /// This is the skip-link idiom: MDN hides "skip to main content" with
    /// `top: calc(var(--offset) * -1)`. Page coordinates are unsigned, so
    /// clamping that to zero put the link on top of the page instead of off it.
    #[test]
    fn an_absolute_box_above_the_page_origin_is_not_drawn() {
        let l = probe_layout(
            r#"<html><body style="margin:0"><div style="position:absolute;top:-320px;left:0;width:200px;height:40px;background:#bb0021"></div><div style="position:absolute;top:-10px;left:0;width:200px;height:40px;background:#bb0022"></div></body></html>"#,
            400,
        );
        assert!(
            probe_rect(&l, 0xBB0021).is_err(),
            "a box entirely above the origin should not be drawn"
        );
        // One that only overhangs the top still has something on screen.
        assert!(
            probe_rect(&l, 0xBB0022).is_ok(),
            "a partly visible box should still be drawn"
        );
    }

    /// An absolutely positioned child takes no grid slot.
    ///
    /// Grid was the last container that still placed such a child as an item,
    /// so its offsets never applied. MDN's skip link is exactly this shape: a
    /// `position:absolute` list directly under a `display:grid` body.
    #[test]
    fn an_out_of_flow_child_takes_no_slot_in_a_grid() {
        let l = probe_layout(
            r#"<html><body style="margin:0"><div style="display:grid;grid-template-columns:100px 100px"><div style="position:absolute;top:5px;left:7px;width:20px;height:10px;background:#bb0031"></div><div style="height:10px;background:#bb0032"></div><div style="height:10px;background:#bb0033"></div></div></body></html>"#,
            400,
        );

        let first = probe_rect(&l, 0xBB0032).expect("first in-flow item");
        let second = probe_rect(&l, 0xBB0033).expect("second in-flow item");
        assert_eq!(first.x, 0, "the in-flow items should own both columns");
        assert_eq!(second.x, 100, "the positioned child must not hold a slot");

        let positioned = probe_rect(&l, 0xBB0031).expect("positioned child");
        assert_eq!(
            (positioned.x, positioned.y),
            (7, 5),
            "the positioned child should honour its own offsets"
        );
    }

    /// `display: contents` generates no box: the wrapper's children become the
    /// container's items.
    ///
    /// MDN's navigation is built this way -- `.navigation__popup` is
    /// `display:contents` -- and treating the wrapper as a real box stacked its
    /// three children vertically instead of laying them across the columns,
    /// which is what made the sticky header 606px tall.
    #[test]
    fn display_contents_hands_its_children_to_the_grid() {
        let l = probe_layout(
            r#"<html><body style="margin:0"><div style="display:grid;grid-template-columns:100px 100px"><div style="display:contents"><div style="height:10px;background:#bb0041"></div><div style="height:10px;background:#bb0042"></div></div></div></body></html>"#,
            400,
        );

        let first = probe_rect(&l, 0xBB0041).expect("first child");
        let second = probe_rect(&l, 0xBB0042).expect("second child");
        assert_eq!(first.x, 0);
        assert_eq!(
            second.x, 100,
            "the wrapper's children should be separate grid items"
        );
        assert_eq!(first.y, second.y, "and share a row instead of stacking");
    }

    /// A grid container honours a stated height, the way a flex one already did.
    ///
    /// MDN's nav bar asks for `height: var(--navigation-height)`; sizing the
    /// grid purely by its rows left a tall empty band under the tabs.
    #[test]
    fn a_grid_container_honours_a_stated_height() {
        let l = probe_layout(
            r#"<html><body style="margin:0"><div style="display:grid;height:60px;background:#bb0051"><div style="height:10px"></div></div><div style="height:5px;background:#bb0052"></div></body></html>"#,
            400,
        );

        let grid = probe_rect(&l, 0xBB0051).expect("grid background");
        assert_eq!(
            grid.height, 60,
            "the stated height should win over the row's 10px"
        );
        let after = probe_rect(&l, 0xBB0052).expect("the box after the grid");
        assert_eq!(after.y, 60, "the next box starts below the stated height");
    }

    /// `max-height` caps a box, including one sized purely by its content.
    ///
    /// It was parsed and then never consulted, so MDN's decorative mandala --
    /// `max-height: 20rem` around a 35rem drawing -- kept its full height and
    /// pushed the page down.
    #[test]
    fn max_height_caps_a_box() {
        let l = probe_layout(
            r#"<html><body style="margin:0"><div style="max-height:40px;background:#bb0061"><div style="height:200px"></div></div><div style="height:5px;background:#bb0062"></div></body></html>"#,
            400,
        );

        let capped = probe_rect(&l, 0xBB0061).expect("capped box");
        assert_eq!(capped.height, 40, "content is 200px but max-height is 40px");
        let after = probe_rect(&l, 0xBB0062).expect("the box after it");
        assert_eq!(
            after.y, 40,
            "overflow does not take part in layout, so the next box follows the cap"
        );
    }

    /// A box shorter than its cap is left alone.
    #[test]
    fn max_height_leaves_a_shorter_box_alone() {
        let l = probe_layout(
            r#"<html><body style="margin:0"><div style="max-height:200px;background:#bb0063"><div style="height:30px"></div></div></body></html>"#,
            400,
        );
        let box_ = probe_rect(&l, 0xBB0063).expect("box");
        assert_eq!(box_.height, 30);
    }

    /// Whitespace between two blocks generates no line box.
    ///
    /// An empty inline (a custom element that renders nothing, say) plus the
    /// newlines around it used to add a phantom line. MDN's sticky header has
    /// three such gaps and came out 71px taller than it asks to be, which put
    /// the nav bar on top of the article instead of above it.
    #[test]
    fn whitespace_between_blocks_makes_no_line_box() {
        let with_gap = probe_layout(
            r#"<html><body style="margin:0"><div style="height:20px;background:#bb0071"></div>
            <my-thing></my-thing>
            <div style="height:20px;background:#bb0072"></div></body></html>"#,
            400,
        );
        let without_gap = probe_layout(
            r#"<html><body style="margin:0"><div style="height:20px;background:#bb0071"></div><div style="height:20px;background:#bb0072"></div></body></html>"#,
            400,
        );
        assert_eq!(
            probe_rect(&with_gap, 0xBB0072).unwrap().y,
            probe_rect(&without_gap, 0xBB0072).unwrap().y,
            "an empty inline between blocks must not take a line"
        );
    }

    /// ...but under `white-space: pre` the blank line is content.
    #[test]
    fn pre_keeps_a_blank_line() {
        let l = probe_layout(
            r#"<html><body style="margin:0"><div style="white-space:pre;height:auto">
</div><div style="height:20px;background:#bb0073"></div></body></html>"#,
            400,
        );
        assert!(
            probe_rect(&l, 0xBB0073).unwrap().y > 0,
            "the preformatted blank line should still occupy space"
        );
    }

    /// A grid centres its items in the row when asked to.
    ///
    /// `align-items` was read by the flex container and ignored by the grid, so
    /// every grid item hugged the top of its row. MDN's nav bar is a 33px row of
    /// tabs inside a 4.125rem bar, and they sat against its top edge.
    #[test]
    fn a_grid_centres_items_in_a_taller_row() {
        let l = probe_layout(
            r#"<html><body style="margin:0"><div style="display:grid;height:100px;align-items:center"><div style="height:20px;background:#bb0081"></div></div></body></html>"#,
            400,
        );
        let item = probe_rect(&l, 0xBB0081).expect("grid item");
        assert_eq!(item.y, 40, "a 20px item in a 100px row centres at y=40");
    }

    /// `align-items: end` puts it at the bottom, and the default leaves it at
    /// the top.
    #[test]
    fn a_grid_honours_the_other_alignments() {
        let bottom = probe_layout(
            r#"<html><body style="margin:0"><div style="display:grid;height:100px;align-items:flex-end"><div style="height:20px;background:#bb0082"></div></div></body></html>"#,
            400,
        );
        assert_eq!(probe_rect(&bottom, 0xBB0082).unwrap().y, 80);

        let top = probe_layout(
            r#"<html><body style="margin:0"><div style="display:grid;height:100px"><div style="height:20px;background:#bb0083"></div></div></body></html>"#,
            400,
        );
        assert_eq!(probe_rect(&top, 0xBB0083).unwrap().y, 0);
    }

    /// Flex and grid containers paint a gradient background, and paint it under
    /// their content.
    ///
    /// Only the block paths emitted gradients, so `background: linear-gradient()`
    /// on a flex box produced nothing -- firefox.com's cards are `display:flex`,
    /// and their light fill never appeared, leaving dark text on the page's dark
    /// background. Emitting it after the children instead buries them.
    #[test]
    fn flex_and_grid_paint_a_gradient_under_their_content() {
        for display in ["flex", "grid"] {
            let l = probe_layout(
                &format!(
                    r#"<html><body style="margin:0"><div style="display:{display};background:linear-gradient(180deg,#ffffff 0%,#eeeeee 100%)"><div style="height:20px;background:#bb0091"></div></div></body></html>"#
                ),
                400,
            );

            let gradient_at = l
                .commands
                .iter()
                .position(|c| matches!(c, DrawCommand::Gradient(_)))
                .unwrap_or_else(|| panic!("{display} should paint a gradient: {:?}", l.commands));
            let child_at = l
                .commands
                .iter()
                .position(|c| matches!(c, DrawCommand::Rect(r) if r.color == 0xBB0091))
                .expect("child rect");
            assert!(
                gradient_at < child_at,
                "{display} must paint the gradient under its content"
            );

            if let DrawCommand::Gradient(g) = &l.commands[gradient_at] {
                assert!(g.height > 1, "{display} gradient height should be measured");
            }
        }
    }

    #[test]
    fn position_relative_shifts_box_from_normal_flow() {
        let l = probe_layout(
            r#"<html><body style="margin:0"><div style="height:10px"></div><div style="position:relative;left:15px;width:40px;height:20px;background:#bb0012"></div></body></html>"#,
            400,
        );
        let shifted = probe_rect(&l, 0xBB0012).expect("relative box");
        assert!(shifted.x >= 15, "relative left offset should shift box right, got x={}", shifted.x);
    }

    #[test]
    fn adjacent_block_margins_are_added_without_collapsing() {
        let l = probe_layout(
            r#"<html><body style="margin:0"><div style="height:20px;margin-bottom:30px;background:#bb0013"></div><div style="height:20px;margin-top:20px;background:#bb0014"></div></body></html>"#,
            400,
        );
        let first = probe_rect(&l, 0xBB0013).expect("first block");
        let second = probe_rect(&l, 0xBB0014).expect("second block");
        assert_eq!(second.y, first.y + first.height + 50, "adjacent margins should add, not collapse");
    }

    fn probe_layout(html: &str, width: u32) -> super::LayoutDocument {
        probe_layout_with_images(html, width, &ImageStore::default())
    }

    fn probe_layout_with_images(
        html: &str,
        width: u32,
        images: &ImageStore,
    ) -> super::LayoutDocument {
        let document = parse_document(html);
        let styled = build_styled_tree(
            &document,
            &parse_stylesheet(""),
            1280,
            &crate::css::InteractiveState::default(),
        );
        let mut fonts = FontContext::load();
        layout_styled_document(&styled, images, width, &mut fonts)
    }

    fn probe_rect(layout: &super::LayoutDocument, color: u32) -> Result<super::RectCommand, String> {
        layout
            .rects()
            .into_iter()
            .find(|r| r.color == color)
            .ok_or_else(|| format!("rect #{color:06x} not found"))
    }

    fn assert_drawn_commands_within(layout: &super::LayoutDocument, width: u32) {
        for text in layout.texts() {
            assert!(
                text.x.saturating_add(text.width) <= width,
                "text overflows: {:?} x={} width={} limit={}",
                text.text,
                text.x,
                text.width,
                width
            );
        }
        for rect in layout.rects() {
            assert!(
                rect.x.saturating_add(rect.width) <= width,
                "rect overflows: x={} width={} limit={}",
                rect.x,
                rect.width,
                width
            );
        }
        for image in layout.images() {
            assert!(
                image.x.saturating_add(image.width) <= width,
                "image overflows: {} x={} width={} limit={}",
                image.src,
                image.x,
                image.width,
                width
            );
        }
    }

    #[test]
    fn table_columns_shrink_and_wrap_long_cell_text() {
        let long_text = "LONGTEXT".repeat(90);
        let html = format!(
            r#"<html><body style="margin:0"><table cellspacing="0" cellpadding="0"><tr><td>{}</td></tr></table></body></html>"#,
            long_text
        );
        let layout = probe_layout(&html, 400);

        assert_drawn_commands_within(&layout, 400);
        let ys = layout
            .texts()
            .into_iter()
            .map(|text| text.y)
            .collect::<std::collections::BTreeSet<_>>();
        assert!(ys.len() > 1, "table text should wrap to multiple lines");
    }

    #[test]
    fn table_column_shrink_preserves_loaded_image_floor() {
        let mut images = ImageStore::default();
        images.insert(
            "https://example.com/table.jpg".to_string(),
            DecodedImage {
                width: 600,
                height: 200,
                rgba: vec![255; 600 * 200 * 4],
            },
        );
        let long_text = "TEXTCOLUMN".repeat(60);
        let html = format!(
            r#"<html><body style="margin:0"><table cellspacing="0" cellpadding="0"><tr><td><img src="https://example.com/table.jpg" width="300" height="100"></td><td>{}</td></tr></table></body></html>"#,
            long_text
        );
        let layout = probe_layout_with_images(&html, 400, &images);
        let image = layout
            .images()
            .into_iter()
            .find(|image| image.src == "https://example.com/table.jpg")
            .expect("table image should be drawn");

        assert_eq!(image.width, 300);
        assert_drawn_commands_within(&layout, 400);
        assert!(
            layout
                .texts()
                .into_iter()
                .any(|text| text.x >= image.x.saturating_add(image.width)),
            "text column should remain after the image column"
        );
    }

    #[test]
    fn float_left_pushes_following_block_right() {
        let l = probe_layout(
            r#"<html><body style="margin:0"><div style="float:left;width:100px;height:40px;background:#aa0001"></div><div style="background:#aa0002;height:20px"></div></body></html>"#,
            320,
        );
        let float_box = probe_rect(&l, 0xAA0001).expect("float rect");
        let flow_box = probe_rect(&l, 0xAA0002).expect("flow rect");
        assert_eq!(float_box.x, 0);
        assert_eq!(float_box.y, 0);
        assert!(flow_box.x >= 100, "flow box not shortened by float: x={}", flow_box.x);
        assert!(flow_box.y <= float_box.height, "flow box should stay in the float band or just below it");
    }

    #[test]
    fn clear_both_drops_below_floats() {
        let l = probe_layout(
            r#"<html><body style="margin:0"><div style="float:left;width:100px;height:40px;background:#aa0003"></div><div style="clear:both;background:#aa0004;height:20px"></div></body></html>"#,
            320,
        );
        let float_box = probe_rect(&l, 0xAA0003).expect("float rect");
        let cleared = probe_rect(&l, 0xAA0004).expect("cleared rect");
        assert!(cleared.y >= float_box.y.saturating_add(float_box.height), "clear:both did not move below float");
    }

    #[test]
    fn block_stack_without_floats_is_unchanged() {
        let l = probe_layout(
            r#"<html><body style="margin:0"><div style="background:#aa0005;height:20px"></div><div style="background:#aa0006;height:30px"></div></body></html>"#,
            320,
        );
        let a = probe_rect(&l, 0xAA0005).expect("first rect");
        let b = probe_rect(&l, 0xAA0006).expect("second rect");
        assert!(b.y >= a.y.saturating_add(a.height), "blocks no longer stack vertically");
        assert_eq!(a.x, 0);
        assert_eq!(b.x, 0);
    }

    #[test]
    fn negative_margin_top_pulls_element_up() {
        let l = probe_layout(
            r#"<html><body style="margin:0"><div style="background:#aa0009;height:20px"></div><div style="margin-top:-20px;background:#aa000a;height:20px"></div></body></html>"#,
            320,
        );
        let first = probe_rect(&l, 0xAA0009).expect("first rect");
        let second = probe_rect(&l, 0xAA000A).expect("second rect");
        assert!(second.y < first.y.saturating_add(first.height), "negative top margin did not pull the second block up");
        assert_eq!(second.y, first.y, "expected the second block to overlap the first by 20px");
    }

    #[test]
    fn negative_margin_left_shifts_left_with_clamp() {
        let l = probe_layout(
            r#"<html><body style="margin:0"><div style="margin-left:-10px;width:40px;height:20px;background:#aa000b"></div></body></html>"#,
            320,
        );
        let box_ = probe_rect(&l, 0xAA000B).expect("box rect");
        assert_eq!(box_.x, 0, "negative left margin should clamp at the viewport edge");
    }

    #[test]
    fn positive_margin_top_still_adds_spacing() {
        let l = probe_layout(
            r#"<html><body style="margin:0"><div style="background:#aa000c;height:20px"></div><div style="margin-top:30px;background:#aa000d;height:20px"></div></body></html>"#,
            320,
        );
        let first = probe_rect(&l, 0xAA000C).expect("first rect");
        let second = probe_rect(&l, 0xAA000D).expect("second rect");
        assert_eq!(second.y, first.y.saturating_add(first.height).saturating_add(30), "positive margin-top changed");
    }

    #[test]
    fn negative_horizontal_margin_does_not_overflow_left_of_zero() {
        let l = probe_layout(
            r#"<html><body style="margin:0"><div style="display:flex;width:100px"><div style="margin-left:-30px;margin-right:-10px;width:40px;height:20px;background:#aa000e"></div></div></body></html>"#,
            320,
        );
        let box_ = probe_rect(&l, 0xAA000E).expect("box rect");
        assert_eq!(box_.x, 0, "negative horizontal margin should clamp x at zero");
        assert!(box_.width >= 40, "negative margins should not shrink the available width");
    }

    #[test]
    fn auto_width_non_img_float_degrades_to_block() {
        let l = probe_layout(
            r#"<html><body style="margin:0"><div style="float:left;background:#aa0007;height:20px">auto width float?</div><div style="background:#aa0008;height:20px"></div></body></html>"#,
            320,
        );
        let degraded = probe_rect(&l, 0xAA0007).expect("degraded rect");
        let next = probe_rect(&l, 0xAA0008).expect("next rect");
        assert_eq!(degraded.x, 0, "auto-width non-img float should fall back to normal block flow");
        assert!(next.y >= degraded.y.saturating_add(degraded.height), "fallback block should keep vertical flow");
    }

    #[test]
    fn layout_probe_report() {
        type Case = (&'static str, fn() -> Result<(), String>);
        fn near(a: u32, b: u32, tol: u32) -> bool {
            a.abs_diff(b) <= tol
        }
        let cases: Vec<Case> = vec![
            ("flex-row-side-by-side", || {
                let l = probe_layout(
                    r#"<div style="display:flex"><div style="width:60px;height:40px;background:#aa0001"></div><div style="width:60px;height:40px;background:#aa0002"></div></div>"#,
                    600,
                );
                let a = probe_rect(&l, 0xAA0001)?;
                let b = probe_rect(&l, 0xAA0002)?;
                if b.x <= a.x {
                    return Err(format!("not side by side: a.x={} b.x={}", a.x, b.x));
                }
                if !near(a.y, b.y, 4) {
                    return Err(format!("rows misaligned: a.y={} b.y={}", a.y, b.y));
                }
                Ok(())
            }),
            ("flex-justify-center", || {
                let l = probe_layout(
                    r#"<div style="display:flex;justify-content:center;width:600px"><div style="width:100px;height:20px;background:#aa0003"></div></div>"#,
                    600,
                );
                let a = probe_rect(&l, 0xAA0003)?;
                if !near(a.x, 250, 40) {
                    return Err(format!("not centered: x={} (want ~250)", a.x));
                }
                Ok(())
            }),
            ("flex-column-stacked", || {
                let l = probe_layout(
                    r#"<div style="display:flex;flex-direction:column"><div style="width:60px;height:40px;background:#aa0004"></div><div style="width:60px;height:40px;background:#aa0005"></div></div>"#,
                    600,
                );
                let a = probe_rect(&l, 0xAA0004)?;
                let b = probe_rect(&l, 0xAA0005)?;
                if b.y <= a.y {
                    return Err(format!("not stacked: a.y={} b.y={}", a.y, b.y));
                }
                Ok(())
            }),
            ("flex-justify-space-between", || {
                let l = probe_layout(
                    r#"<div style="display:flex;justify-content:space-between;width:600px"><div style="width:60px;height:20px;background:#aa0006"></div><div style="width:60px;height:20px;background:#aa0007"></div></div>"#,
                    600,
                );
                let a = probe_rect(&l, 0xAA0006)?;
                let b = probe_rect(&l, 0xAA0007)?;
                if !near(a.x, 0, 30) {
                    return Err(format!("first not at start: x={}", a.x));
                }
                if !near(b.x, 540, 60) {
                    return Err(format!("last not at end: x={} (want ~540)", b.x));
                }
                Ok(())
            }),
            ("flex-grow-equal", || {
                let l = probe_layout(
                    r#"<div style="display:flex;width:600px"><div style="flex:1;height:20px;background:#aa0008"></div><div style="flex:1;height:20px;background:#aa0009"></div></div>"#,
                    600,
                );
                let a = probe_rect(&l, 0xAA0008)?;
                let b = probe_rect(&l, 0xAA0009)?;
                if !near(a.width, 300, 80) {
                    return Err(format!("grow item width={} (want ~300)", a.width));
                }
                if b.x <= a.x {
                    return Err(format!("second item not after first: a.x={} b.x={}", a.x, b.x));
                }
                Ok(())
            }),
            ("flex-align-items-center", || {
                let l = probe_layout(
                    r#"<div style="display:flex;align-items:center;height:100px"><div style="width:40px;height:20px;background:#aa000a"></div></div>"#,
                    600,
                );
                let a = probe_rect(&l, 0xAA000A)?;
                if !near(a.y, 40, 25) {
                    return Err(format!("cross-axis not centered: y={} (want ~40)", a.y));
                }
                Ok(())
            }),
            ("position-absolute", || {
                let l = probe_layout(
                    r#"<div style="position:relative;height:200px"><div style="position:absolute;top:50px;left:80px;width:40px;height:40px;background:#aa000b"></div></div>"#,
                    600,
                );
                let a = probe_rect(&l, 0xAA000B)?;
                if !near(a.x, 80, 12) || !near(a.y, 50, 12) {
                    return Err(format!("absolute pos wrong: ({},{}) want (~80,~50)", a.x, a.y));
                }
                Ok(())
            }),
            ("position-relative-offset", || {
                let l = probe_layout(
                    r#"<div style="height:10px"></div><div style="position:relative;top:30px;width:40px;height:40px;background:#aa000c"></div>"#,
                    600,
                );
                let a = probe_rect(&l, 0xAA000C)?;
                if a.y < 30 {
                    return Err(format!("relative top offset not applied: y={}", a.y));
                }
                Ok(())
            }),
            ("box-sizing-border-box", || {
                let l = probe_layout(
                    r#"<div style="box-sizing:border-box;width:200px;padding:20px;background:#aa000d">x</div>"#,
                    600,
                );
                let a = probe_rect(&l, 0xAA000D)?;
                if !near(a.width, 200, 8) {
                    return Err(format!("border-box width={} (want ~200)", a.width));
                }
                Ok(())
            }),
            ("block-margin-auto-center", || {
                let l = probe_layout(
                    r#"<div style="width:200px;margin-left:auto;margin-right:auto;height:20px;background:#aa000e"></div>"#,
                    600,
                );
                let a = probe_rect(&l, 0xAA000E)?;
                if !near(a.x, 200, 40) {
                    return Err(format!("margin auto not centered: x={} (want ~200)", a.x));
                }
                Ok(())
            }),
            ("flex-gap", || {
                let l = probe_layout(
                    r#"<div style="display:flex;gap:20px"><div style="width:60px;height:20px;background:#aa000f"></div><div style="width:60px;height:20px;background:#aa0010"></div></div>"#,
                    600,
                );
                let a = probe_rect(&l, 0xAA000F)?;
                let b = probe_rect(&l, 0xAA0010)?;
                if !near(b.x, a.x + a.width + 20, 12) {
                    return Err(format!("gap not applied: a.x+w={} b.x={}", a.x + a.width, b.x));
                }
                Ok(())
            }),
            ("flex-wrap", || {
                let l = probe_layout(
                    r#"<div style="display:flex;flex-wrap:wrap;width:140px"><div style="width:60px;height:30px;background:#aa0011"></div><div style="width:60px;height:30px;background:#aa0012"></div><div style="width:60px;height:30px;background:#aa0013"></div></div>"#,
                    600,
                );
                let a = probe_rect(&l, 0xAA0011)?;
                let c = probe_rect(&l, 0xAA0013)?;
                if c.y <= a.y {
                    return Err(format!("third item did not wrap: a.y={} c.y={}", a.y, c.y));
                }
                Ok(())
            }),
            ("grid-two-columns", || {
                let l = probe_layout(
                    r#"<div style="display:grid;grid-template-columns:1fr 1fr;width:400px"><div style="height:20px;background:#aa0014"></div><div style="height:20px;background:#aa0015"></div></div>"#,
                    600,
                );
                let a = probe_rect(&l, 0xAA0014)?;
                let b = probe_rect(&l, 0xAA0015)?;
                if b.x <= a.x {
                    return Err(format!("grid columns not side by side: a.x={} b.x={}", a.x, b.x));
                }
                Ok(())
            }),
            ("min-width", || {
                let l = probe_layout(
                    r#"<div style="width:50px;min-width:200px;height:20px;background:#aa0016"></div>"#,
                    600,
                );
                let a = probe_rect(&l, 0xAA0016)?;
                if a.width < 190 {
                    return Err(format!("min-width not applied: width={} (want >=200)", a.width));
                }
                Ok(())
            }),
            ("max-width", || {
                let l = probe_layout(
                    r#"<div style="width:100%;max-width:300px;height:20px;background:#aa0017"></div>"#,
                    600,
                );
                let a = probe_rect(&l, 0xAA0017)?;
                if a.width > 320 {
                    return Err(format!("max-width not capping: width={} (want ~300)", a.width));
                }
                Ok(())
            }),
            ("position-fixed-top", || {
                let l = probe_layout(
                    r#"<div style="height:50px"></div><div style="position:fixed;top:0;left:0;width:40px;height:30px;background:#aa0018"></div>"#,
                    600,
                );
                let a = probe_rect(&l, 0xAA0018)?;
                if a.y > 12 {
                    return Err(format!("fixed top not at viewport top: y={}", a.y));
                }
                Ok(())
            }),
            ("nested-flex-navbar", || {
                // logo on the left, two nav links pushed right via space-between
                let l = probe_layout(
                    r#"<div style="display:flex;justify-content:space-between;width:600px"><div style="width:80px;height:24px;background:#aa0019"></div><div style="display:flex;gap:10px"><div style="width:50px;height:24px;background:#aa001a"></div><div style="width:50px;height:24px;background:#aa001b"></div></div></div>"#,
                    600,
                );
                let logo = probe_rect(&l, 0xAA0019)?;
                let link2 = probe_rect(&l, 0xAA001B)?;
                if !near(logo.x, 0, 20) {
                    return Err(format!("logo not at left: x={}", logo.x));
                }
                if link2.x + link2.width < 520 {
                    return Err(format!("nav links not pushed right: last right edge={}", link2.x + link2.width));
                }
                Ok(())
            }),
        ];

        let mut failures: Vec<(&str, String)> = Vec::new();
        for (name, case) in &cases {
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(case)) {
                Ok(Ok(())) => {}
                Ok(Err(e)) => failures.push((name, e)),
                Err(_) => failures.push((name, "PANIC".to_string())),
            }
        }
        let total = cases.len();
        println!("\n=== layout probe: {}/{} passed ===", total - failures.len(), total);
        for (name, err) in &failures {
            println!("  [layout] {name}: {err}");
        }
        println!();
    }

    #[test]
    fn inline_code_in_paragraph_does_not_overlap_lines() {
        // A paragraph mixing text + an inline <code> that wraps to several lines
        // must place each line below the previous (no overlapping text).
        let html = r#"<p style="width:300px">Press each button and if the display changes <code>TOBIRA_ENGINE</code> events are flowing through and the banner turns green which proves scripts ran.</p>"#;
        let l = probe_layout(html, 320);
        let mut ys: Vec<u32> = l.texts().iter().map(|t| t.y).collect();
        ys.sort_unstable();
        ys.dedup();
        assert!(ys.len() >= 2, "paragraph should wrap to multiple lines, got ys={ys:?}");
        // Consecutive distinct line tops must differ by at least half a line —
        // overlapping lines would sit within a few px of each other.
        let line_h = l.texts().iter().map(|t| t.line_height_px).max().unwrap_or(16);
        for pair in ys.windows(2) {
            let gap = pair[1] - pair[0];
            assert!(
                gap >= line_h / 2,
                "lines overlap: gap {gap} < {} (line_h); ys={ys:?}",
                line_h / 2
            );
        }
    }

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
    fn honors_explicit_pixel_height() {
        // A block with an explicit pixel height taller than its content should
        // reserve that height (so the page scrolls), not collapse to content.
        let document = parse_document("<div style=\"height: 600px;\">x</div>");
        let styled = build_styled_tree(
            &document,
            &parse_stylesheet(""),
            1280,
            &crate::css::InteractiveState::default(),
        );
        let mut fonts = FontContext::load();
        let layout = layout_styled_document(&styled, &ImageStore::default(), 320, &mut fonts);
        assert!(
            layout.content_height >= 600,
            "explicit height should expand the box (content_height = {})",
            layout.content_height
        );
    }

    #[test]
    fn resolves_percent_height_against_definite_parent() {
        // A child with height:100% inside a definite-height parent fills it
        // (e.g. a progress-bar fill). Without this it collapses to content height
        // (0 for an empty element) and renders invisible.
        let document = parse_document(
            "<div style=\"height: 40px\"><div style=\"height: 100%; background: #123456\"></div></div>",
        );
        let styled = build_styled_tree(
            &document,
            &parse_stylesheet(""),
            1280,
            &crate::css::InteractiveState::default(),
        );
        let mut fonts = FontContext::load();
        let layout = layout_styled_document(&styled, &ImageStore::default(), 320, &mut fonts);
        let inner = layout
            .rects()
            .into_iter()
            .find(|r| r.color == 0x123456)
            .expect("inner background rect should exist");
        assert!(
            inner.height >= 40,
            "percent height should resolve to the parent's 40px, got {}",
            inner.height
        );
    }

    #[test]
    fn short_block_without_explicit_height_stays_short() {
        // Control: identical content with no explicit height stays content-sized,
        // so the height honoring above is what produced the tall box.
        let document = parse_document("<div>x</div>");
        let styled = build_styled_tree(
            &document,
            &parse_stylesheet(""),
            1280,
            &crate::css::InteractiveState::default(),
        );
        let mut fonts = FontContext::load();
        let layout = layout_styled_document(&styled, &ImageStore::default(), 320, &mut fonts);
        assert!(
            layout.content_height < 200,
            "a one-line block should stay short (content_height = {})",
            layout.content_height
        );
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
    fn table_align_centers_box_without_centering_cell_text() {
        let layout = probe_layout(
            r##"<html><body style="margin:0"><table align="center" width="200" cellspacing="0" cellpadding="0"><tr><td bgcolor="#bb0020">text</td></tr></table></body></html>"##,
            500,
        );
        let cell = probe_rect(&layout, 0xBB0020).expect("cell rect should exist");
        let text = layout
            .texts()
            .into_iter()
            .find(|text| text.text == "text")
            .expect("text should exist");

        assert_eq!(cell.x, 150);
        assert_eq!(cell.width, 200);
        assert_eq!(text.x, cell.x);
    }

    #[test]
    fn legacy_div_align_still_centers_text() {
        let layout = probe_layout(
            r#"<html><body style="margin:0"><div align="center">text</div></body></html>"#,
            200,
        );
        let text = layout
            .texts()
            .into_iter()
            .find(|text| text.text == "text")
            .expect("text should exist");

        assert_eq!(text.x, (200 - text.width) / 2);
    }

    #[test]
    fn legacy_td_align_still_centers_cell_text() {
        let layout = probe_layout(
            r##"<html><body style="margin:0"><table width="200" cellspacing="0" cellpadding="0"><tr><td align="center" bgcolor="#bb0021">text</td></tr></table></body></html>"##,
            500,
        );
        let cell = probe_rect(&layout, 0xBB0021).expect("cell rect should exist");
        let text = layout
            .texts()
            .into_iter()
            .find(|text| text.text == "text")
            .expect("text should exist");

        assert_eq!(cell.x, 0);
        assert_eq!(cell.width, 200);
        assert_eq!(text.x, cell.x + (cell.width - text.width) / 2);
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
    fn table_cell_br_line_spacing_matches_div() {
        let div_layout = probe_layout(
            r#"<html><body style="margin:0"><div>L1<br>L2<br>L3</div></body></html>"#,
            1570,
        );
        let table_layout = probe_layout(
            r#"<html><body style="margin:0"><table><tr><td>L1<br>L2<br>L3</td></tr></table></body></html>"#,
            1570,
        );

        let line_gaps = |layout: &super::LayoutDocument| -> Vec<u32> {
            let mut ys: Vec<u32> = ["L1", "L2", "L3"]
                .iter()
                .map(|label| {
                    layout
                        .texts()
                        .into_iter()
                        .find(|text| text.text == *label)
                        .unwrap_or_else(|| panic!("missing text {label}"))
                        .y
                })
                .collect();
            ys.sort_unstable();
            ys.windows(2).map(|pair| pair[1] - pair[0]).collect()
        };

        assert_eq!(line_gaps(&table_layout), line_gaps(&div_layout));
    }

    #[test]
    fn table_cell_inline_children_share_line() {
        let layout = probe_layout(
            r#"<html><body style="margin:0"><table><tr><td>Left<strong>:</strong></td></tr></table></body></html>"#,
            1570,
        );
        let texts = layout.texts();
        let left = texts
            .iter()
            .find(|text| text.text == "Left")
            .expect("Left text should exist");
        let colon = texts
            .iter()
            .find(|text| text.text == ":")
            .expect("colon text should exist");

        assert_eq!(left.y, colon.y, "texts: {texts:?}");
    }

    #[test]
    fn table_cell_mixed_block_children_stack_vertically() {
        let layout = probe_layout(
            r#"<html><body style="margin:0"><table><tr><td>text<table><tr><td>X</td></tr></table><div>after</div></td></tr></table></body></html>"#,
            1570,
        );
        let texts = layout.texts();
        let text = texts
            .iter()
            .find(|text| text.text == "text")
            .expect("cell text should exist");
        let nested = texts
            .iter()
            .find(|text| text.text == "X")
            .expect("nested table text should exist");
        let after = texts
            .iter()
            .find(|text| text.text == "after")
            .expect("block text should exist");

        assert!(nested.y > text.y, "nested table should stack below inline text");
        assert!(after.y > nested.y, "following block should stack below nested table");
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
    fn inline_linked_image_emits_image_command_and_link_hitbox() {
        let mut images = ImageStore::default();
        images.insert(
            "https://example.com/a.jpg".to_string(),
            DecodedImage {
                width: 4,
                height: 4,
                rgba: vec![255; 4 * 4 * 4],
            },
        );
        let layout = probe_layout_with_images(
            r#"<html><body style="margin:0"><div><a href="https://example.com/x"><img src="https://example.com/a.jpg" width="100" height="140"></a> No.1</div></body></html>"#,
            320,
            &images,
        );

        let images_list = layout.images();
        let image = images_list
            .iter()
            .find(|image| image.src == "https://example.com/a.jpg")
            .expect("inline linked image should emit ImageCommand");
        assert_eq!(image.width, 100);
        assert_eq!(image.height, 140);
        assert!(
            !layout.texts().iter().any(|text| text.text.contains("[image]")),
            "loaded inline image should not fall back to [image] text"
        );
        assert!(
            layout.links.iter().any(|link| {
                link.href == "https://example.com/x"
                    && link.x == image.x
                    && link.y == image.y
                    && link.width == image.width
                    && link.height == image.height
            }),
            "linked inline image should register an image-sized link hitbox"
        );
    }

    #[test]
    fn missing_inline_image_keeps_alt_text_fallback() {
        let layout = probe_layout(
            r#"<html><body style="margin:0"><div><a href="https://example.com/x"><img src="https://example.com/missing.jpg"></a></div></body></html>"#,
            320,
        );

        assert!(layout.images().is_empty());
        assert!(
            layout.texts().iter().any(|text| text.text.contains("[image]")),
            "missing inline image should keep the [image] fallback"
        );
    }

    #[test]
    fn inline_image_advances_line_by_image_height() {
        let mut images = ImageStore::default();
        images.insert(
            "https://example.com/tall.jpg".to_string(),
            DecodedImage {
                width: 4,
                height: 4,
                rgba: vec![255; 4 * 4 * 4],
            },
        );
        let layout = probe_layout_with_images(
            r#"<html><body style="margin:0"><div><span><img src="https://example.com/tall.jpg" width="30" height="140"></span> No.1</div><div>Next</div></body></html>"#,
            320,
            &images,
        );

        let image = layout
            .images()
            .into_iter()
            .find(|image| image.src == "https://example.com/tall.jpg")
            .expect("inline image should be drawn");
        let next = layout
            .texts()
            .into_iter()
            .find(|text| text.text.contains("Next"))
            .expect("following line should be drawn");
        assert!(
            next.y >= image.y.saturating_add(image.height),
            "following block should start after the inline image height"
        );
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
    fn table_cells_default_to_middle_vertical_alignment() {
        let layout = probe_layout(
            r#"<html><body style="margin:0"><table cellspacing="0" cellpadding="0"><tr><td>A<br>B<br>C</td><td>short</td></tr></table></body></html>"#,
            320,
        );
        let texts = layout.texts();
        let a = texts
            .iter()
            .find(|text| text.text == "A")
            .expect("A should exist");
        let c = texts
            .iter()
            .find(|text| text.text == "C")
            .expect("C should exist");
        let short = texts
            .iter()
            .find(|text| text.text == "short")
            .expect("short should exist");

        assert!(short.y > a.y, "short should not be top-aligned: {texts:?}");
        assert!(short.y < c.y, "short should be within the tall cell middle band: {texts:?}");
    }

    #[test]
    fn td_valign_top_keeps_cell_content_top_aligned() {
        let layout = probe_layout(
            r#"<html><body style="margin:0"><table cellspacing="0" cellpadding="0"><tr><td>A<br>B<br>C</td><td valign="top">short</td></tr></table></body></html>"#,
            320,
        );
        let texts = layout.texts();
        let a = texts
            .iter()
            .find(|text| text.text == "A")
            .expect("A should exist");
        let short = texts
            .iter()
            .find(|text| text.text == "short")
            .expect("short should exist");

        assert_eq!(short.y, a.y);
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

    /// One `LineSpan` is produced per word and one `InlineFragment` per inline
    /// run, so anything inlined into them is paid for by the whole page. Both
    /// used to carry a `ComputedStyle` (520 bytes) plus inline `FormControlSpec`
    /// and `InlineImageSpec` variants, putting `LineSpan` at ~1.9 KB. The style
    /// is shared through an `Rc` now and the rare variants are boxed. This guard
    /// exists so a future field does not quietly re-inline something large.
    #[test]
    fn inline_layout_structs_stay_small() {
        use super::{InlineFragment, LineSpan};
        use std::mem::size_of;

        let span = size_of::<LineSpan>();
        let fragment = size_of::<InlineFragment>();
        assert!(
            span <= 128,
            "LineSpan grew to {span} bytes; box or share whatever was added"
        );
        assert!(
            fragment <= 128,
            "InlineFragment grew to {fragment} bytes; box or share whatever was added"
        );
    }

    #[test]
    fn grid_row_spanning_item_grows_the_rows_it_covers() {
        use crate::css::{parse_stylesheet, build_styled_tree};
        use crate::html::parse_document;

        // Column 1 holds a 200px item spanning both rows; column 2 holds two
        // short items. The spanning item has to push the two rows apart, so C
        // (row 2) ends up far below B (row 1). Before rows grew for spanning
        // items, C sat directly under B and the tall item overflowed the grid.
        let html = r#"<div style="display:grid;grid-template-columns:100px 100px;gap:0px;"><div style="grid-row:span 2;height:200px;">TALL</div><div>B</div><div>C</div></div>"#;
        let doc = parse_document(html);
        let stylesheet = parse_stylesheet("");
        let styled = build_styled_tree(&doc, &stylesheet, 400, &crate::css::InteractiveState::default());
        let mut fonts = FontContext::load();
        let images = ImageStore::default();
        let layout = layout_styled_document(&styled, &images, 400, &mut fonts);

        let texts = layout.texts();
        let b = texts.iter().find(|t| t.text.contains('B')).expect("B should be rendered");
        let c = texts.iter().find(|t| t.text.contains('C')).expect("C should be rendered");

        assert!(c.y > b.y, "C should be on the row below B");
        let row_gap = c.y - b.y;
        assert!(
            row_gap >= 80,
            "the 200px row-spanning item should have grown both rows, but C is only {row_gap}px below B"
        );
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

    /// `auto` tracks must share *all* the free space.
    ///
    /// The split used to hand two thirds to the `fr` tracks unconditionally,
    /// but the `fr` payout is skipped when there are none, so that share was
    /// simply discarded -- `auto auto` across 1200px produced 200px columns
    /// instead of 600px. Named-area grids hit this constantly, because a
    /// template with no `grid-template-columns` is an all-`auto` track list.
    #[test]
    fn auto_grid_tracks_receive_the_whole_free_space() {
        use crate::css::GridTrackSize;

        assert_eq!(
            super::resolve_grid_tracks(&[GridTrackSize::Auto, GridTrackSize::Auto], 1200, 0),
            vec![600, 600]
        );

        // Fixed tracks come off the top; the remainder is still fully spent.
        assert_eq!(
            super::resolve_grid_tracks(
                &[
                    GridTrackSize::Pixels(200),
                    GridTrackSize::Auto,
                    GridTrackSize::Auto
                ],
                1200,
                0
            ),
            vec![200, 500, 500]
        );

        // With both kinds present the existing 2/3 fr, 1/3 auto split stands.
        assert_eq!(
            super::resolve_grid_tracks(&[GridTrackSize::Fr(1000), GridTrackSize::Auto], 900, 0),
            vec![600, 300]
        );
    }

    /// Items land on the rectangle their `grid-area` name points at: the header
    /// spans both columns, and nav/main sit side by side beneath it.
    #[test]
    fn grid_named_areas_place_items_on_their_rectangle() {
        use crate::css::{build_styled_tree, parse_stylesheet};
        use crate::html::parse_document;

        let html = r#"<div class="page"><div class="h">HEAD</div><div class="n">NAV</div><div class="m">MAIN</div></div>"#;
        let stylesheet = parse_stylesheet(
            r#".page { display: grid; grid-template-areas: "head head" "nav main"; }
               .h { grid-area: head; }
               .n { grid-area: nav; }
               .m { grid-area: main; }"#,
        );
        let doc = parse_document(html);
        let styled =
            build_styled_tree(&doc, &stylesheet, 800, &crate::css::InteractiveState::default());
        let mut fonts = FontContext::load();
        let images = ImageStore::default();
        let layout = layout_styled_document(&styled, &images, 800, &mut fonts);

        let texts = layout.texts();
        let find = |needle: &str| {
            texts
                .iter()
                .find(|t| t.text.contains(needle))
                .unwrap_or_else(|| panic!("{needle} should be rendered"))
                .clone()
        };
        let head = find("HEAD");
        let nav = find("NAV");
        let main = find("MAIN");

        // Two columns across 800px, so the second one starts near the middle.
        assert_eq!(head.x, nav.x, "head and nav both start in column 1");
        assert!(
            main.x >= 380 && main.x <= 420,
            "main should start in column 2, got x={}",
            main.x
        );
        // The header owns its own row above the other two.
        assert!(head.y < nav.y, "head sits above nav");
        assert_eq!(nav.y, main.y, "nav and main share a row");
    }

    /// The regression that motivated named areas: a grid laid out purely with
    /// `grid-template-areas` used to collapse to one full-width column, so an
    /// item in the second column got no width of its own and its text wrapped
    /// one character per line.
    #[test]
    fn named_area_columns_are_wide_enough_to_hold_their_text() {
        use crate::css::{build_styled_tree, parse_stylesheet};
        use crate::html::parse_document;

        let html = r#"<div class="page"><div class="s">SIDE</div><div class="m">Resources for developers</div></div>"#;
        let stylesheet = parse_stylesheet(
            r#".page { display: grid; grid-template-areas: "side main"; }
               .s { grid-area: side; }
               .m { grid-area: main; }"#,
        );
        let doc = parse_document(html);
        let styled =
            build_styled_tree(&doc, &stylesheet, 1000, &crate::css::InteractiveState::default());
        let mut fonts = FontContext::load();
        let images = ImageStore::default();
        let layout = layout_styled_document(&styled, &images, 1000, &mut fonts);

        let texts = layout.texts();
        let main_runs: Vec<_> = texts.iter().filter(|t| t.x >= 400).collect();
        assert!(!main_runs.is_empty(), "the main area should render something");

        // Half of 1000px is plenty for this phrase; if the column had collapsed
        // every run would be a single character.
        let longest = main_runs.iter().map(|t| t.text.trim().len()).max().unwrap();
        assert!(
            longest > 1,
            "main column collapsed -- text broke into single characters"
        );
    }

    /// The MDN regression in miniature: a container whose track list names its
    /// lines, with a child placed by `grid-column: <name>`. The child must land
    /// on the wide middle track, not auto-place into the narrow first one.
    #[test]
    fn named_line_placement_puts_the_item_on_the_named_track() {
        use crate::css::{build_styled_tree, parse_stylesheet};
        use crate::html::parse_document;

        let html = r#"<div class="page"><div class="c">CONTENT</div></div>"#;
        let stylesheet = parse_stylesheet(
            r#".page { display: grid;
                      grid-template-columns: [pad-start] 40px [content-start] 1fr [content-end] 40px [pad-end]; }
               .c { grid-column: content; }"#,
        );
        let doc = parse_document(html);
        let styled =
            build_styled_tree(&doc, &stylesheet, 1000, &crate::css::InteractiveState::default());
        let mut fonts = FontContext::load();
        let images = ImageStore::default();
        let layout = layout_styled_document(&styled, &images, 1000, &mut fonts);

        let content = layout
            .texts()
            .into_iter()
            .find(|t| t.text.contains("CONTENT"))
            .expect("CONTENT should be rendered");

        // Track 0 is the 40px pad, so the named track starts right after it.
        assert_eq!(
            content.x, 40,
            "item should sit on the content track, not auto-place into the pad"
        );
    }

    /// `min-content` is sized by what is in it and does not stretch, so a
    /// narrow neighbour must not take an equal share from an `fr` track. This
    /// is Wikipedia's `... / minmax(0,Nrem) min-content` article grid.
    #[test]
    fn min_content_track_does_not_steal_from_fr() {
        use crate::css::{build_styled_tree, parse_stylesheet};
        use crate::html::parse_document;

        let html = r#"<div class="page"><div class="a">MAIN</div><div class="b">.</div></div>"#;
        let stylesheet = parse_stylesheet(
            r#".page { display: grid; grid-template-columns: 1fr min-content; }"#,
        );
        let doc = parse_document(html);
        let styled =
            build_styled_tree(&doc, &stylesheet, 1000, &crate::css::InteractiveState::default());
        let mut fonts = FontContext::load();
        let images = ImageStore::default();
        let layout = layout_styled_document(&styled, &images, 1000, &mut fonts);

        let narrow = layout
            .texts()
            .into_iter()
            .find(|t| t.text.trim() == ".")
            .expect("the min-content item should be rendered");

        // A third of 1000px would put the narrow column's start at ~667. Sized
        // by its contents it starts far to the right of that.
        assert!(
            narrow.x > 800,
            "min-content column took too much room; it starts at x={}",
            narrow.x
        );
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

    #[test]
    fn block_with_explicit_width_is_constrained() {
        let document = parse_document(r#"<div style="width:200px;background:red;">x</div>"#);
        let stylesheet = parse_stylesheet("");
        let styled = build_styled_tree(&document, &stylesheet, 1280, &crate::css::InteractiveState::default());
        let mut fonts = FontContext::load();
        let layout = layout_styled_document(&styled, &ImageStore::default(), 800, &mut fonts);

        let rects = layout.rects();
        let bg = rects.iter().find(|r| r.color == 0xFF0000)
            .expect("red background rect should exist");
        assert_eq!(bg.width, 200, "div should be 200px wide, got {}", bg.width);
    }

    #[test]
    fn margin_auto_centers_block_element() {
        let document = parse_document(r#"<div style="width:200px;margin:0 auto;background:red;">x</div>"#);
        let stylesheet = parse_stylesheet("");
        let styled = build_styled_tree(&document, &stylesheet, 1280, &crate::css::InteractiveState::default());
        let mut fonts = FontContext::load();
        let layout = layout_styled_document(&styled, &ImageStore::default(), 800, &mut fonts);

        let rects = layout.rects();
        let bg = rects.iter().find(|r| r.color == 0xFF0000)
            .expect("red background rect should exist");
        // (800 - 200) / 2 = 300
        assert_eq!(bg.x, 300, "div should be centered at x=300, got {}", bg.x);
        assert_eq!(bg.width, 200, "div width should be 200px");
    }
}
