use std::collections::BTreeMap;
use std::rc::Rc;

use crate::html::{Element, Node};

pub type Color = u32;

pub const DEFAULT_TEXT_COLOR: Color = 0x1D232E;
pub const DEFAULT_BACKGROUND_COLOR: Color = 0xFFFDF8;
pub const DEFAULT_LINK_COLOR: Color = 0x2A5DB0;

// ─────────────────────────────────────────────────────────────────────────────
// Animation / Transition / Keyframe types
// ─────────────────────────────────────────────────────────────────────────────

/// A single keyframe position with its CSS declarations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyframeStop {
    /// Position 0–1000 (permille). 0 = "from"/0%, 1000 = "to"/100%
    pub position: u32,
    /// The CSS declarations at this keyframe position (property → value pairs)
    pub declarations: Vec<(String, String)>,
}

/// A parsed `@keyframes` rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyframeRule {
    pub name: String,
    pub stops: Vec<KeyframeStop>,
}

/// Easing function for both animation and transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimingFunction {
    Linear,
    Ease,       // default
    EaseIn,
    EaseOut,
    EaseInOut,
    StepStart,
    StepEnd,
}

impl Default for TimingFunction {
    fn default() -> Self {
        TimingFunction::Ease
    }
}

/// CSS `animation-fill-mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimFillMode {
    None,
    Forwards,
    Backwards,
    Both,
}

impl Default for AnimFillMode {
    fn default() -> Self {
        AnimFillMode::None
    }
}

/// CSS `animation-direction`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimDirection {
    Normal,
    Reverse,
    Alternate,
    AlternateReverse,
}

impl Default for AnimDirection {
    fn default() -> Self {
        AnimDirection::Normal
    }
}

/// A fully parsed `animation` layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnimationSpec {
    pub name: String,
    pub duration_ms: u32,
    pub delay_ms: i32,
    /// 0 = infinite (stored as u32::MAX), default 1
    pub iteration_count: u32,
    pub fill_mode: AnimFillMode,
    pub timing_function: TimingFunction,
    pub direction: AnimDirection,
}

/// A fully parsed `transition` layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionSpec {
    /// CSS property name, or "all"
    pub property: String,
    pub duration_ms: u32,
    pub delay_ms: i32,
    pub timing_function: TimingFunction,
}

impl TimingFunction {
    /// Apply the easing curve to a linear progress value `t` in [0, 1].
    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            TimingFunction::Linear => t,
            // Approximate cubic-bezier(0.25, 0.1, 0.25, 1.0)
            TimingFunction::Ease => cubic_bezier(t, 0.25, 0.1, 0.25, 1.0),
            // cubic-bezier(0.42, 0, 1, 1)
            TimingFunction::EaseIn => cubic_bezier(t, 0.42, 0.0, 1.0, 1.0),
            // cubic-bezier(0, 0, 0.58, 1)
            TimingFunction::EaseOut => cubic_bezier(t, 0.0, 0.0, 0.58, 1.0),
            // cubic-bezier(0.42, 0, 0.58, 1)
            TimingFunction::EaseInOut => cubic_bezier(t, 0.42, 0.0, 0.58, 1.0),
            TimingFunction::StepStart => if t > 0.0 { 1.0 } else { 0.0 },
            TimingFunction::StepEnd => if t >= 1.0 { 1.0 } else { 0.0 },
        }
    }
}

/// Approximate cubic-bezier(P1.x, P1.y, P2.x, P2.y) for t in [0, 1].
/// Uses Newton's method on the X parameterization to invert, then evaluates Y.
fn cubic_bezier(t: f32, x1: f32, y1: f32, x2: f32, y2: f32) -> f32 {
    // Solve for parametric u given x(u) = t.
    let mut u = t;
    for _ in 0..6 {
        let xu = 3.0 * (1.0 - u).powi(2) * u * x1
            + 3.0 * (1.0 - u) * u.powi(2) * x2
            + u.powi(3);
        let dxu = 3.0 * (1.0 - u).powi(2) * x1
            + 6.0 * (1.0 - u) * u * (x2 - x1)
            + 3.0 * u.powi(2) * (1.0 - x2);
        let denom = if dxu.abs() < 1e-6 { 1e-6 } else { dxu };
        u -= (xu - t) / denom;
        u = u.clamp(0.0, 1.0);
    }
    3.0 * (1.0 - u).powi(2) * u * y1
        + 3.0 * (1.0 - u) * u.powi(2) * y2
        + u.powi(3)
}

/// Linear interpolation helper.
pub fn lerp_u32(a: u32, b: u32, t: f32) -> u32 {
    let af = a as f32;
    let bf = b as f32;
    (af + (bf - af) * t).round().max(0.0) as u32
}

pub fn lerp_i32(a: i32, b: i32, t: f32) -> i32 {
    let af = a as f32;
    let bf = b as f32;
    (af + (bf - af) * t).round() as i32
}

/// Interpolate two packed `0xRRGGBB` colors component-wise.
pub fn lerp_color(a: u32, b: u32, t: f32) -> u32 {
    let ar = ((a >> 16) & 0xFF) as f32;
    let ag = ((a >> 8) & 0xFF) as f32;
    let ab = (a & 0xFF) as f32;
    let br = ((b >> 16) & 0xFF) as f32;
    let bg = ((b >> 8) & 0xFF) as f32;
    let bb = (b & 0xFF) as f32;
    let r = (ar + (br - ar) * t).round().clamp(0.0, 255.0) as u32;
    let g = (ag + (bg - ag) * t).round().clamp(0.0, 255.0) as u32;
    let bl = (ab + (bb - ab) * t).round().clamp(0.0, 255.0) as u32;
    (r << 16) | (g << 8) | bl
}

/// Compute progress (0..=1) for an `animation` at the given monotonic time (ms).
/// Returns None if the animation has not started or has finished and fill-mode forbids it.
pub fn animation_progress(spec: &AnimationSpec, start_ms: u64, now_ms: u64) -> Option<f32> {
    let now = now_ms as i64;
    let start = start_ms as i64 + spec.delay_ms as i64;
    let elapsed = now - start;
    if elapsed < 0 {
        return if matches!(spec.fill_mode, AnimFillMode::Backwards | AnimFillMode::Both) {
            Some(0.0)
        } else {
            None
        };
    }
    let dur = spec.duration_ms.max(1) as i64;
    let iter = (elapsed / dur) as u32;
    let infinite = spec.iteration_count == u32::MAX;
    if !infinite && iter >= spec.iteration_count {
        return if matches!(spec.fill_mode, AnimFillMode::Forwards | AnimFillMode::Both) {
            Some(spec.timing_function.apply(1.0))
        } else {
            None
        };
    }
    let in_iter = (elapsed % dur) as f32 / dur as f32;
    // Honor animation-direction (alternate flips on odd iterations).
    let t_linear = match spec.direction {
        AnimDirection::Normal => in_iter,
        AnimDirection::Reverse => 1.0 - in_iter,
        AnimDirection::Alternate => {
            if iter % 2 == 0 { in_iter } else { 1.0 - in_iter }
        }
        AnimDirection::AlternateReverse => {
            if iter % 2 == 0 { 1.0 - in_iter } else { in_iter }
        }
    };
    Some(spec.timing_function.apply(t_linear))
}

/// Walk a styled tree and apply currently-animating values to each element's style
/// using the provided keyframe rules and the monotonic time (ms). `start_times` maps
/// element identity (tag_name + node id) → animation start time; the caller is
/// expected to populate this as animations begin (a future repaint loop responsibility).
///
/// MVP: supports `opacity`, `color`, `background-color`, `transform: translateX/Y`,
/// `transform: scale*`, `transform: rotate` keyframe interpolation.
pub fn apply_animations_to_tree(
    node: &mut StyledNode,
    keyframes: &[KeyframeRule],
    now_ms: u64,
    default_start_ms: u64,
) {
    fn walk(node: &mut StyledNode, kf: &[KeyframeRule], now: u64, start: u64) {
        match node {
            StyledNode::Element(el) => {
                if !el.style.animations.is_empty() {
                    apply_animations_to_style(&mut el.style, kf, now, start);
                }
                for child in el.children.iter_mut() {
                    walk(child, kf, now, start);
                }
            }
            StyledNode::Text(_) => {}
        }
    }
    walk(node, keyframes, now_ms, default_start_ms);
}

/// Apply each `AnimationSpec` on `style` by looking up its `@keyframes` rule and
/// blending the surrounding keyframe stop declarations into `style` at the current
/// progress. Later animations win for overlapping properties (CSS cascade order).
pub fn apply_animations_to_style(
    style: &mut ComputedStyle,
    keyframes: &[KeyframeRule],
    now_ms: u64,
    start_ms: u64,
) {
    let specs = style.animations.clone();
    for spec in &specs {
        let progress = match animation_progress(spec, start_ms, now_ms) {
            Some(p) => p,
            None => continue,
        };
        let rule = match keyframes.iter().find(|r| r.name == spec.name) {
            Some(r) => r,
            None => continue,
        };
        let (from, to, t) = match find_keyframe_segment(rule, progress) {
            Some(seg) => seg,
            None => continue,
        };
        blend_keyframe_declarations(style, &from.declarations, &to.declarations, t);
    }
}

/// Blend two keyframe declaration sets into `style` at progress `t`.
fn blend_keyframe_declarations(
    style: &mut ComputedStyle,
    from: &[(String, String)],
    to: &[(String, String)],
    t: f32,
) {
    // Build a lookup of `to` declarations for matching properties.
    let to_map: std::collections::HashMap<&str, &str> =
        to.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    for (prop, from_val) in from {
        let to_val = match to_map.get(prop.as_str()) {
            Some(v) => *v,
            None => from_val.as_str(),
        };
        match prop.as_str() {
            "opacity" => {
                let a = from_val.trim().parse::<f32>().unwrap_or(1.0);
                let b = to_val.trim().parse::<f32>().unwrap_or(1.0);
                let v = (a + (b - a) * t).clamp(0.0, 1.0);
                style.opacity = (v * 255.0).round() as u8;
                style.effective_opacity = style.opacity;
            }
            "color" => {
                if let (Some(a), Some(b)) = (parse_color(from_val), parse_color(to_val)) {
                    style.color = lerp_color(a, b, t);
                }
            }
            "background-color" => {
                if let (Some(a), Some(b)) = (parse_color(from_val), parse_color(to_val)) {
                    style.background_color = Some(lerp_color(a, b, t));
                }
            }
            "transform" => {
                // Limited support: parse the same single-function transform on both sides.
                blend_transform(style, from_val, to_val, t);
            }
            _ => {} // unsupported property — silently skip
        }
    }
}

fn blend_transform(style: &mut ComputedStyle, from: &str, to: &str, t: f32) {
    // Try scale(x, y?) / scaleX / scaleY
    if let (Some(a), Some(b)) = (extract_scale(from), extract_scale(to)) {
        // extract_scale returns (sx_millis, sy_millis); blend linearly in millis.
        let sx_lin = ((a.0 as f32) + ((b.0 as f32) - (a.0 as f32)) * t).round() as u32;
        let sy_lin = ((a.1 as f32) + ((b.1 as f32) - (a.1 as f32)) * t).round() as u32;
        style.transform_scale_x = sx_lin;
        style.transform_scale_y = sy_lin;
    }
    // rotate(Ndeg)
    if let (Some(a), Some(b)) = (extract_rotate_millideg(from), extract_rotate_millideg(to)) {
        style.transform_rotate_millideg = lerp_i32(a, b, t);
    }
    // translate(Xpx, Ypx) — supports only px units
    if let (Some((ax, ay)), Some((bx, by))) = (extract_translate_px(from), extract_translate_px(to)) {
        style.transform_translate_x = lerp_i32(ax, bx, t);
        style.transform_translate_y = lerp_i32(ay, by, t);
    }
}

fn extract_scale(s: &str) -> Option<(u32, u32)> {
    // Returns (sx_millis, sy_millis)
    let lower = s.to_ascii_lowercase();
    if let Some(start) = lower.find("scale(") {
        let after = &lower[start + "scale(".len()..];
        let close = after.find(')')?;
        let inside = &after[..close];
        let parts: Vec<&str> = inside.split(',').collect();
        let sx = parts.first()?.trim().parse::<f32>().ok()?;
        let sy = parts.get(1).and_then(|s| s.trim().parse::<f32>().ok()).unwrap_or(sx);
        return Some(((sx * 1000.0) as u32, (sy * 1000.0) as u32));
    }
    if let Some(start) = lower.find("scalex(") {
        let after = &lower[start + "scalex(".len()..];
        let close = after.find(')')?;
        let sx = after[..close].trim().parse::<f32>().ok()?;
        return Some(((sx * 1000.0) as u32, 1000));
    }
    if let Some(start) = lower.find("scaley(") {
        let after = &lower[start + "scaley(".len()..];
        let close = after.find(')')?;
        let sy = after[..close].trim().parse::<f32>().ok()?;
        return Some((1000, (sy * 1000.0) as u32));
    }
    None
}

fn extract_rotate_millideg(s: &str) -> Option<i32> {
    let lower = s.to_ascii_lowercase();
    let start = lower.find("rotate(")?;
    let after = &lower[start + "rotate(".len()..];
    let close = after.find(')')?;
    let token = after[..close].trim();
    let num_str = token.trim_end_matches("deg").trim();
    let deg = num_str.parse::<f32>().ok()?;
    Some((deg * 1000.0) as i32)
}

fn extract_translate_px(s: &str) -> Option<(i32, i32)> {
    let lower = s.to_ascii_lowercase();
    let start = lower.find("translate(")?;
    let after = &lower[start + "translate(".len()..];
    let close = after.find(')')?;
    let inside = &after[..close];
    let parts: Vec<&str> = inside.split(',').collect();
    let parse_px = |t: &str| -> Option<i32> {
        let t = t.trim().trim_end_matches("px").trim();
        t.parse::<f32>().ok().map(|v| v as i32)
    };
    let x = parse_px(parts.first()?)?;
    let y = parts.get(1).and_then(|s| parse_px(s)).unwrap_or(0);
    Some((x, y))
}

/// Find the two surrounding keyframe stops around `progress` (0..=1) and the local t.
/// Returns (from_stop, to_stop, local_t).
pub fn find_keyframe_segment(rule: &KeyframeRule, progress: f32) -> Option<(&KeyframeStop, &KeyframeStop, f32)> {
    if rule.stops.is_empty() {
        return None;
    }
    let p = (progress.clamp(0.0, 1.0) * 1000.0) as u32;
    let mut from = &rule.stops[0];
    let mut to = &rule.stops[rule.stops.len() - 1];
    for window in rule.stops.windows(2) {
        if window[0].position <= p && p <= window[1].position {
            from = &window[0];
            to = &window[1];
            break;
        }
    }
    let span = (to.position as i64 - from.position as i64).max(1);
    let local = ((p as i64 - from.position as i64) as f32 / span as f32).clamp(0.0, 1.0);
    Some((from, to, local))
}

/// Apply currently-running CSS transitions to `current` style, blending each animated
/// property between the previous and current values per the transition spec. `start_ms`
/// is when the property change was observed; `now_ms` is the current time. Property
/// changes prior to the recorded start are considered complete.
///
/// MVP scope: opacity, color, background_color, transform translate/scale/rotate.
pub fn apply_transitions_to_style(
    current: &mut ComputedStyle,
    previous: &ComputedStyle,
    now_ms: u64,
    start_ms: u64,
) {
    let specs = current.transitions.clone();
    if specs.is_empty() {
        return;
    }
    for spec in &specs {
        let elapsed = (now_ms as i64) - (start_ms as i64) - (spec.delay_ms as i64);
        if elapsed < 0 {
            continue;
        }
        let dur = spec.duration_ms.max(1) as i64;
        let raw_t = ((elapsed as f32) / (dur as f32)).clamp(0.0, 1.0);
        let t = spec.timing_function.apply(raw_t);
        let prop = spec.property.as_str();
        let matches_prop = |name: &str| prop == "all" || prop == name;

        if matches_prop("opacity") && previous.opacity != current.opacity {
            let v = (previous.opacity as f32)
                + ((current.opacity as f32) - (previous.opacity as f32)) * t;
            current.opacity = v.round().clamp(0.0, 255.0) as u8;
            current.effective_opacity = current.opacity;
        }
        if matches_prop("color") && previous.color != current.color {
            current.color = lerp_color(previous.color, current.color, t);
        }
        if matches_prop("background-color")
            && previous.background_color != current.background_color
        {
            if let (Some(a), Some(b)) = (previous.background_color, current.background_color) {
                current.background_color = Some(lerp_color(a, b, t));
            }
        }
        if matches_prop("transform") {
            if previous.transform_translate_x != current.transform_translate_x {
                current.transform_translate_x =
                    lerp_i32(previous.transform_translate_x, current.transform_translate_x, t);
            }
            if previous.transform_translate_y != current.transform_translate_y {
                current.transform_translate_y =
                    lerp_i32(previous.transform_translate_y, current.transform_translate_y, t);
            }
            if previous.transform_scale_x != current.transform_scale_x {
                current.transform_scale_x =
                    lerp_u32(previous.transform_scale_x, current.transform_scale_x, t);
            }
            if previous.transform_scale_y != current.transform_scale_y {
                current.transform_scale_y =
                    lerp_u32(previous.transform_scale_y, current.transform_scale_y, t);
            }
            if previous.transform_rotate_millideg != current.transform_rotate_millideg {
                current.transform_rotate_millideg = lerp_i32(
                    previous.transform_rotate_millideg,
                    current.transform_rotate_millideg,
                    t,
                );
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Stylesheet / Rule types
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stylesheet {
    pub rules: Vec<Rule>,
    /// CSS custom properties declared on `:root` or `html` outside any `@media` block.
    /// Shared via `Rc` so that cloning into each element's `css_variables` map is O(1).
    pub root_vars: Rc<BTreeMap<String, String>>,
    /// CSS custom properties declared on `:root` or `html` inside an `@media` block.
    /// Each entry is `(condition, vars)` and is only applied when the condition matches
    /// the current viewport width at style-computation time.
    pub media_root_vars: Vec<(MediaCondition, BTreeMap<String, String>)>,
    /// All `@keyframes` rules found in this stylesheet.
    pub keyframes: Vec<KeyframeRule>,
}

impl Stylesheet {
    pub fn extend(&mut self, other: Stylesheet) {
        self.rules.extend(other.rules);
        // Merge unconditional root_vars: make a mutable copy, extend it, then wrap back in Rc
        let mut merged = (*self.root_vars).clone();
        merged.extend((*other.root_vars).clone());
        self.root_vars = Rc::new(merged);
        // Merge media-conditional root vars
        self.media_root_vars.extend(other.media_root_vars);
        // Merge keyframes
        self.keyframes.extend(other.keyframes);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    selectors: Vec<Selector>,
    declarations: Vec<Declaration>,
    /// None = always apply; Some(cond) = apply only when cond matches
    media: Option<MediaCondition>,
    pub pseudo_element: Option<PseudoElement>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MediaCondition {
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
    pseudo_element: Option<PseudoElement>,
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
    never_match: bool,
    pseudo_element: Option<PseudoElement>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PseudoElement {
    Before,
    After,
    Placeholder,
    Selection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PseudoClass {
    FirstChild,
    LastChild,
    NthChild(i32, i32), // (a, b) → matches when (index - b) % a == 0 (1-based index)
    Not(Vec<SimpleSelector>),
    Hover,
    Focus,
    Active,
    Checked,
    Disabled,
    Enabled,
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
    node_id: Option<usize>,
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

/// Tracks which elements are in interactive states for :hover/:focus/:active matching.
#[derive(Debug, Clone, Default)]
pub struct InteractiveState {
    pub hovered_node_id: Option<usize>,
    pub focused_node_id: Option<usize>,
    pub active_node_ids: std::collections::HashSet<usize>,
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
    Flex,
    InlineFlex,
    Grid,
    InlineGrid,
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
    NoWrap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WordBreak {
    Normal,
    BreakAll,
    KeepAll,
}

impl Default for WordBreak {
    fn default() -> Self { WordBreak::Normal }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FontFamilyKind {
    Sans,
    Serif,
    Monospace,
}

// ─────────────────────────────────────────────────────────────────────────────
// TextShadow
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextShadow {
    pub offset_x: i32,
    pub offset_y: i32,
    pub blur: u32,
    pub color: u32,
}

// ─────────────────────────────────────────────────────────────────────────────
// GradientKind / CssGradient (shared by linear + radial)
// ─────────────────────────────────────────────────────────────────────────────

/// Distinguishes gradient types in the CSS parse layer and draw-command layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GradientKind {
    /// angle_deg_x1000: degrees × 1000 (e.g. 180_000 = 180°, top-to-bottom).
    Linear { angle_deg_x1000: i32 },
    /// center_x / center_y in permille of element dimensions (500 = 50 %).
    Radial { center_x: u32, center_y: u32 },
}

/// A gradient value parsed from CSS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CssGradient {
    pub kind: GradientKind,
    pub stops: Vec<(u32, u32)>, // (color, position 0-1000)
}

// Keep old name as a type alias for backward compatibility.
pub type LinearGradient = CssGradient;

// ─────────────────────────────────────────────────────────────────────────────
// BackgroundSize / BackgroundRepeat
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackgroundSize {
    Auto,
    Cover,
    Contain,
}

impl Default for BackgroundSize {
    fn default() -> Self {
        BackgroundSize::Auto
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundRepeat {
    Repeat,
    NoRepeat,
    RepeatX,
    RepeatY,
}

impl Default for BackgroundRepeat {
    fn default() -> Self {
        BackgroundRepeat::Repeat
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LengthValue {
    Pixels(u32),
    Percent(u32),
    MinContent,
    MaxContent,
    FitContent(u32), // argument in pixels
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
    pub spread: i32,
    pub color: Option<u32>,
    pub inset: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overflow {
    Visible,
    Hidden,
    Auto,
    Scroll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position {
    Static,
    Relative,
    Absolute,
    Fixed,
    Sticky,
}

impl Default for Position {
    fn default() -> Self {
        Position::Static
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlexDirection {
    Row,
    Column,
    RowReverse,
    ColumnReverse,
}
impl Default for FlexDirection {
    fn default() -> Self {
        FlexDirection::Row
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlexWrap {
    NoWrap,
    Wrap,
    WrapReverse,
}
impl Default for FlexWrap {
    fn default() -> Self {
        FlexWrap::NoWrap
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignItems {
    Stretch,
    FlexStart,
    FlexEnd,
    Center,
    Baseline,
}
impl Default for AlignItems {
    fn default() -> Self {
        AlignItems::Stretch
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JustifyContent {
    FlexStart,
    FlexEnd,
    Center,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}
impl Default for JustifyContent {
    fn default() -> Self {
        JustifyContent::FlexStart
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignSelf {
    Auto,
    Stretch,
    FlexStart,
    FlexEnd,
    Center,
    Baseline,
}
impl Default for AlignSelf {
    fn default() -> Self {
        AlignSelf::Auto
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignContent {
    FlexStart,
    FlexEnd,
    Center,
    SpaceBetween,
    SpaceAround,
    Stretch,
}
impl Default for AlignContent {
    fn default() -> Self {
        AlignContent::Stretch
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorKind {
    #[default]
    Auto,
    Default,
    Pointer,
    Text,
    Move,
    Crosshair,
    Wait,
    Help,
    NotAllowed,
    Grab,
    Grabbing,
    ZoomIn,
    ZoomOut,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ObjectFit {
    #[default]
    Fill,
    Contain,
    Cover,
    ScaleDown,
    None,
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
// Grid types
// ─────────────────────────────────────────────────────────────────────────────

/// A single grid track definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GridTrackSize {
    Pixels(u32),
    /// Stored as percent * 100 to keep Eq (e.g. 50% → 5000)
    Percent(u32),
    /// Fractional unit * 1000 (1fr → 1000, 0.5fr → 500)
    Fr(u32),
    Auto,
    MinContent,
    MaxContent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridPlacement {
    pub start: Option<i32>, // grid line number (1-based), None = auto
    pub span: Option<u32>,  // span count, None = 1
}

impl Default for GridPlacement {
    fn default() -> Self {
        GridPlacement {
            start: None,
            span: None,
        }
    }
}

/// `grid-auto-flow` direction and packing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GridAutoFlow {
    Row,
    Column,
    RowDense,
    ColumnDense,
}

/// `writing-mode` — direction of inline progression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WritingMode {
    HorizontalTb,   // default: left-to-right top-to-bottom
    VerticalRl,     // top-to-bottom right-to-left columns
    VerticalLr,     // top-to-bottom left-to-right columns
}

impl Default for WritingMode {
    fn default() -> Self {
        WritingMode::HorizontalTb
    }
}

/// `direction` — base direction for inline text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Ltr,
    Rtl,
}

impl Default for Direction {
    fn default() -> Self {
        Direction::Ltr
    }
}

/// Supported `clip-path` shapes. Coordinates are stored as permille (0–1000)
/// of the element's content box so values survive Eq/Hash on `ComputedStyle`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipPath {
    /// `circle(r at cx cy)` — radius and center as permille of min(width, height).
    Circle { radius_permille: u32, cx_permille: u32, cy_permille: u32 },
    /// `inset(top right bottom left)` — distances from each edge as permille.
    Inset { top: u32, right: u32, bottom: u32, left: u32 },
    /// `polygon((x y), (x y), ...)` — points as (x, y) permille pairs.
    Polygon { points: Vec<(u32, u32)> },
}

impl Default for GridAutoFlow {
    fn default() -> Self {
        GridAutoFlow::Row
    }
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
    pub margin_left_auto: bool,
    pub margin_right_auto: bool,
    pub padding: EdgeSizes,
    pub width: Option<LengthValue>,
    pub height: Option<LengthValue>,
    pub font_size_px: u32,
    pub font_family: FontFamilyKind,
    pub text_align: TextAlign,
    pub vertical_align: VerticalAlign,
    pub font_weight: bool,
    pub underline: bool,
    pub line_through: bool,
    pub white_space: WhiteSpaceMode,
    pub text_overflow_ellipsis: bool,
    pub text_shadow: Option<TextShadow>,
    pub background_gradient: Option<LinearGradient>,
    pub background_image_url: Option<String>,
    pub background_size: BackgroundSize,
    pub background_repeat: BackgroundRepeat,
    pub background_position_x: u32,
    pub background_position_y: u32,
    // ── new fields ──
    pub border: EdgeSizes,
    pub border_color: Color,
    pub border_style_none: bool,
    pub border_radius: u32,
    pub outline_width: u32,
    pub outline_color: Option<Color>,
    /// Space between element border and outline (can be negative)
    pub outline_offset: i32,
    /// Whether the outline style is visible (false = none/hidden)
    pub outline_visible: bool,
    /// line-height in thousandths of em; 0 = "normal"
    pub line_height: u32,
    /// opacity 0–255; 255 = opaque
    pub opacity: u8,
    pub effective_opacity: u8,
    pub font_style_italic: bool,
    pub text_transform: TextTransform,
    pub text_indent: u32,
    pub letter_spacing: i32,
    pub max_width: Option<u32>,
    pub min_width: u32,
    pub max_height: Option<u32>,
    pub min_height: u32,
    pub box_sizing: BoxSizing,
    pub overflow: Overflow,
    pub list_style_type: ListStyleType,
    pub cursor_pointer: bool,
    pub cursor_kind: CursorKind,
    pub pointer_events_none: bool,
    pub text_decoration_color: Option<Color>,
    pub box_shadows: Vec<BoxShadow>,
    pub content: Option<String>,
    // Position
    pub position: Position,
    pub z_index: Option<i32>,
    pub top: Option<i32>,
    pub right: Option<i32>,
    pub bottom: Option<i32>,
    pub left: Option<i32>,
    // Flexbox
    pub flex_direction: FlexDirection,
    pub flex_wrap: FlexWrap,
    pub align_items: AlignItems,
    pub justify_content: JustifyContent,
    pub align_self: AlignSelf,
    pub align_content: AlignContent,
    pub flex_grow: u32,
    pub flex_shrink: u32,
    pub flex_basis: Option<LengthValue>,
    pub gap: u32,
    pub order: i32,
    /// aspect-ratio as milliratio (ratio * 1000, e.g. 16/9 → 1778); None = auto
    pub aspect_ratio: Option<u32>,
    pub object_fit: ObjectFit,
    /// object-position x, 0–100 (percentage), default 50 = center
    pub object_position_x: u32,
    /// object-position y, 0–100 (percentage), default 50 = center
    pub object_position_y: u32,
    // Grid container fields
    pub grid_template_columns: Vec<GridTrackSize>,
    pub grid_template_rows: Vec<GridTrackSize>,
    pub grid_auto_rows: GridTrackSize,
    pub grid_auto_columns: GridTrackSize,
    pub grid_auto_flow: GridAutoFlow,
    /// `scroll-behavior: smooth` — when true, programmatic scroll animates over multiple frames.
    pub scroll_behavior_smooth: bool,
    /// `clip-path` shape, if any.
    pub clip_path: Option<ClipPath>,
    pub writing_mode: WritingMode,
    pub direction: Direction,
    // Grid container areas
    pub grid_template_areas: Vec<Vec<String>>,
    // Grid item fields
    pub grid_column: GridPlacement,
    pub grid_row: GridPlacement,
    pub grid_area_name: Option<String>,
    // Filter effects
    pub filter_blur_px: u32,       // blur() value in pixels, 0 = no blur
    pub filter_brightness: u32,    // brightness() in percent * 100 (10000 = 100% = no change)
    pub filter_opacity: u8,        // opacity() as 0-255, 255 = no change
    // CSS transform (integer fixed-point to keep Eq/Hash)
    pub transform_translate_x: i32,  // pixels (signed)
    pub transform_translate_y: i32,
    pub transform_scale_x: u32,      // millis: 1000=1.0, 0=identity
    pub transform_scale_y: u32,
    pub transform_rotate_millideg: i32,
    pub transform_origin_x: u32,     // permille: 500=50%
    pub transform_origin_y: u32,
    // Text breaking
    pub word_break: WordBreak,
    pub overflow_wrap_break_word: bool,
    // CSS counters
    pub counter_reset: Vec<(String, i32)>,      // counter-reset: name value
    pub counter_increment: Vec<(String, i32)>,  // counter-increment: name value
    // Animations and transitions (structured, for future repaint loop use)
    pub animations: Vec<AnimationSpec>,
    pub transitions: Vec<TransitionSpec>,
}

impl ComputedStyle {
    fn for_element(tag_name: &str, parent: Option<&Self>) -> Self {
        let parent_font_size = parent.map(|s| s.font_size_px).unwrap_or(16);
        let mut style = Self {
            display: default_display(tag_name),
            color: parent.map(|s| s.color).unwrap_or(DEFAULT_TEXT_COLOR),
            background_color: None,
            margin: default_margin(tag_name),
            margin_left_auto: false,
            margin_right_auto: false,
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
            line_through: parent.map(|s| s.line_through).unwrap_or(false),
            white_space: parent
                .map(|s| s.white_space)
                .unwrap_or(WhiteSpaceMode::Normal),
            text_overflow_ellipsis: false,
            text_shadow: None,
            background_gradient: None,
            background_image_url: None,
            background_size: BackgroundSize::Auto,
            background_repeat: BackgroundRepeat::Repeat,
            background_position_x: 50,
            background_position_y: 50,
            // new fields – most not inherited
            border: EdgeSizes::default(),
            border_color: parent.map(|s| s.color).unwrap_or(DEFAULT_TEXT_COLOR),
            border_style_none: false,
            border_radius: 0,
            outline_width: 0,
            outline_color: None,
            outline_offset: 0,
            outline_visible: true,
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
            cursor_kind: CursorKind::Auto,
            pointer_events_none: false,
            text_decoration_color: None,
            box_shadows: Vec::new(),
            content: None,
            // Position fields
            position: Position::Static,
            z_index: None,
            top: None,
            right: None,
            bottom: None,
            left: None,
            // Flexbox fields
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::NoWrap,
            align_items: AlignItems::Stretch,
            justify_content: JustifyContent::FlexStart,
            align_self: AlignSelf::Auto,
            align_content: AlignContent::Stretch,
            flex_grow: 0,
            flex_shrink: 100,
            flex_basis: None,
            gap: 0,
            order: 0,
            aspect_ratio: None,
            object_fit: ObjectFit::Fill,
            object_position_x: 50,
            object_position_y: 50,
            // Grid fields
            grid_template_columns: Vec::new(),
            grid_template_rows: Vec::new(),
            grid_auto_rows: GridTrackSize::Auto,
            grid_auto_columns: GridTrackSize::Auto,
            grid_auto_flow: GridAutoFlow::Row,
            scroll_behavior_smooth: false,
            clip_path: None,
            writing_mode: WritingMode::HorizontalTb,
            direction: Direction::Ltr,
            grid_template_areas: Vec::new(),
            grid_column: GridPlacement::default(),
            grid_row: GridPlacement::default(),
            grid_area_name: None,
            // Filter effects
            filter_blur_px: 0,
            filter_brightness: 10000,
            filter_opacity: 255,
            // CSS transform
            transform_translate_x: 0,
            transform_translate_y: 0,
            transform_scale_x: 0,
            transform_scale_y: 0,
            transform_rotate_millideg: 0,
            transform_origin_x: 500,
            transform_origin_y: 500,
            word_break: WordBreak::Normal,
            overflow_wrap_break_word: false,
            // CSS counters
            counter_reset: vec![],
            counter_increment: vec![],
            // Animations / transitions
            animations: vec![],
            transitions: vec![],
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
    /// Pre-computed ::placeholder style for `<input>` / `<textarea>`, when any matching rule exists.
    pub placeholder_style: Option<ComputedStyle>,
    /// Pre-computed ::selection style for text selection highlights, when any matching rule exists.
    pub selection_style: Option<ComputedStyle>,
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
    let mut depth_paren: u32 = 0;
    let mut depth_bracket: u32 = 0;
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
            '(' => {
                depth_paren += 1;
            }
            ')' if depth_paren > 0 => {
                depth_paren -= 1;
            }
            '[' => {
                depth_bracket += 1;
            }
            ']' if depth_bracket > 0 => {
                depth_bracket -= 1;
            }
            c if c == delimiter && depth_paren == 0 && depth_bracket == 0 => {
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
    let mut root_vars = BTreeMap::new();
    let mut media_root_vars: Vec<(MediaCondition, BTreeMap<String, String>)> = Vec::new();
    let mut keyframes: Vec<KeyframeRule> = Vec::new();
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
                // Store root vars declared inside this @media block separately so they
                // are only applied when the media condition matches at runtime.
                // Previously they were merged unconditionally into root_vars, which caused
                // `@media (max-width: 600px) { :root { --foo: bar; } }` to always apply.
                if !inner_stylesheet.root_vars.is_empty() {
                    let inner_map = (*inner_stylesheet.root_vars).clone();
                    media_root_vars.push((media_cond.clone(), inner_map));
                }
                // Also propagate any nested media_root_vars from the inner stylesheet.
                // Note: nested @media root vars are stored with the inner condition only.
                // The conjunction of outer + inner conditions is not computed. Nested @media
                // is uncommon (non-standard before CSS nesting) and practically rare, so
                // this approximation is acceptable for now.
                for (inner_cond, inner_map) in inner_stylesheet.media_root_vars {
                    media_root_vars.push((inner_cond, inner_map));
                }
                for mut rule in inner_stylesheet.rules {
                    rule.media = Some(media_cond.clone());
                    rules.push(rule);
                }
            } else if at_lower.starts_with("@keyframes") {
                // Extract animation name — everything after "@keyframes " before the block
                let name_part = selector_text["@keyframes".len()..].trim();
                // Strip vendor prefix if present (e.g. @-webkit-keyframes)
                let anim_name = name_part.trim_matches(|c: char| c == '"' || c == '\'').to_string();
                if !anim_name.is_empty() {
                    let stops = parse_keyframe_stops(block_text);
                    keyframes.push(KeyframeRule { name: anim_name, stops });
                }
            } else if at_lower.starts_with("@supports") || at_lower.starts_with("@layer") {
                // @supports: treat condition as always-true (optimistic: assume all features supported)
                // @layer: ignore layer name, parse rules as regular rules (no cascade layering)
                let inner_stylesheet = parse_stylesheet(block_text);
                if !inner_stylesheet.root_vars.is_empty() {
                    let inner_map = (*inner_stylesheet.root_vars).clone();
                    // Treat @supports/@layer root vars as unconditional
                    for (k, v) in inner_map {
                        root_vars.entry(k).or_insert(v);
                    }
                }
                for (inner_cond, inner_map) in inner_stylesheet.media_root_vars {
                    media_root_vars.push((inner_cond, inner_map));
                }
                rules.extend(inner_stylesheet.rules);
                keyframes.extend(inner_stylesheet.keyframes);
            }
            // other at-rules are skipped
            continue;
        }

        if block_text.is_empty() {
            continue;
        }

        let declarations = parse_inline_declarations(block_text);

        // Collect :root / html custom properties into root_vars.
        // Check the raw selector text because :root is not a recognized pseudo-class and
        // will be dropped by parse_selector — we must capture vars before that step.
        // Media conditions are already respected here: @media rules are handled in the
        // branch above and their inner stylesheets' root_vars are propagated separately.
        let is_root = split_at_top_level(selector_text, ',').iter().any(|s| {
            let s = s.trim().to_ascii_lowercase();
            s == ":root" || s == "html"
        });
        if is_root {
            for decl in &declarations {
                if decl.property.starts_with("--") {
                    root_vars.insert(decl.property.clone(), decl.value.clone());
                }
            }
        }

        let selectors = split_at_top_level(selector_text, ',')
            .iter()
            .filter_map(|s| parse_selector(s.trim()))
            .collect::<Vec<_>>();

        if !selectors.is_empty() && !declarations.is_empty() {
            let pseudo_element = selectors.iter().find_map(|sel| sel.pseudo_element.clone());
            rules.push(Rule {
                selectors,
                declarations,
                media: None,
                pseudo_element,
            });
        }
    }

    Stylesheet { rules, root_vars: Rc::new(root_vars), media_root_vars, keyframes }
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
    interactive: &InteractiveState,
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
        interactive,
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
    interactive: &InteractiveState,
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
                interactive,
            );
            // Pre-build the full sibling identity list once for all children to share.
            let all_sibling_ids: Rc<[ElementIdentity]> = element
                .children
                .iter()
                .filter_map(|c| {
                    if let Node::Element(e) = c {
                        Some(ElementIdentity::from(e))
                    } else {
                        None
                    }
                })
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

            let children: Vec<StyledNode> = element
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
                        interactive,
                    )
                })
                .collect();

            // Inject ::before and ::after pseudo-element content.
            // Use the pseudo-element rule's own ComputedStyle (color, font-size, etc.)
            // rather than the host element's style, so `p::before { color: red; }` works.
            let mut children = children;
            if let Some((before_text, pseudo_style)) = collect_pseudo_content(
                element,
                stylesheet,
                ancestors,
                sibling_index,
                sibling_count,
                preceding_siblings,
                viewport_width,
                &PseudoElement::Before,
                &style,
                interactive,
            ) {
                children.insert(
                    0,
                    StyledNode::Text(StyledText {
                        text: before_text,
                        style: pseudo_style,
                    }),
                );
            }
            if let Some((after_text, pseudo_style)) = collect_pseudo_content(
                element,
                stylesheet,
                ancestors,
                sibling_index,
                sibling_count,
                preceding_siblings,
                viewport_width,
                &PseudoElement::After,
                &style,
                interactive,
            ) {
                children.push(StyledNode::Text(StyledText {
                    text: after_text,
                    style: pseudo_style,
                }));
            }

            let placeholder_style = if matches!(element.tag_name.as_str(), "input" | "textarea") {
                compute_placeholder_style(element, stylesheet, &style, viewport_width)
            } else {
                None
            };
            let selection_style =
                compute_selection_style(element, stylesheet, &style, viewport_width);

            StyledNode::Element(StyledElement {
                tag_name: element.tag_name.clone(),
                attributes: element.attributes.clone(),
                style,
                children,
                placeholder_style,
                selection_style,
            })
        }
    }
}

/// Strip a matched pair of surrounding CSS string quotes (`"..."` or `'...'`).
/// Only removes quotes when the same quote character opens and closes the string.
/// Unbalanced quotes (e.g. `"foo'`) are left intact.
fn strip_css_string_quotes(s: &str) -> &str {
    let s = s.trim();
    if s.len() >= 2 {
        let first = s.as_bytes()[0];
        let last = s.as_bytes()[s.len() - 1];
        // Safety: `"` and `'` are single-byte ASCII characters, so checking
        // s.as_bytes()[0] and s.as_bytes()[s.len()-1] is always valid.
        // Slicing at byte offsets 1 and s.len()-1 is safe because the opening
        // and closing quotes are each exactly 1 byte, regardless of the content.
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return &s[1..s.len() - 1];
        }
    }
    s
}

#[allow(clippy::too_many_arguments)]
/// Returns `(content_string, pseudo_element_style)` for the last matching rule,
/// or `None` if no matching `::before`/`::after` rule with a non-empty `content` exists.
/// The returned `ComputedStyle` carries the pseudo-element's own declarations
/// (color, font-size, font-weight, etc.) so callers can apply them to the injected node.
fn collect_pseudo_content(
    element: &Element,
    stylesheet: &Stylesheet,
    ancestors: &[AncestorSlot],
    sibling_index: usize,
    sibling_count: usize,
    preceding_siblings: &[ElementIdentity],
    viewport_width: u32,
    which: &PseudoElement,
    host_style: &ComputedStyle,
    interactive: &InteractiveState,
) -> Option<(String, ComputedStyle)> {
    let identity = ElementIdentity::from(element);
    // Inherit from the host element so pseudo-elements pick up color, font-size, etc.
    // CSS requires pseudo-elements to inherit from their originating element.
    let mut pseudo_style = host_style.clone();
    let mut content_text: Option<String> = None;

    for rule in &stylesheet.rules {
        if let Some(cond) = &rule.media {
            if !cond.matches(viewport_width) {
                continue;
            }
        }
        // Check per-selector pseudo_element (not rule-level) to handle
        // comma-separated selectors like `p::before, div::after { ... }`
        let host_matches = rule.selectors.iter().any(|sel| {
            sel.pseudo_element.as_ref() == Some(which)
                && sel.matches(
                    &identity,
                    ancestors,
                    sibling_index,
                    sibling_count,
                    preceding_siblings,
                    interactive,
                )
        });
        if !host_matches {
            continue;
        }
        // Apply all declarations in cascade order.
        // Accumulate `content` text separately to avoid intermediate clones of pseudo_style —
        // the final (text, pseudo_style) pair is only constructed once at the end.
        for decl in &rule.declarations {
            if decl.property == "content" {
                let raw = decl.value.trim();
                if raw == "none" || raw == "normal" {
                    content_text = None;
                } else if let Some(inner) =
                    raw.strip_prefix("attr(").and_then(|s| s.strip_suffix(')'))
                {
                    // attr(name) — resolve from element attributes
                    let attr_name = inner.trim();
                    content_text = Some(element.attribute(attr_name).unwrap_or("").to_string());
                } else {
                    let v = strip_css_string_quotes(raw);
                    if !v.is_empty() {
                        content_text = Some(v.to_string());
                    }
                }
            } else {
                // Use host_style.font_size_px so em/% units in pseudo-element rules
                // resolve against the originating element's font size (not a hardcoded 16px).
                apply_declaration(&mut pseudo_style, decl, host_style.font_size_px);
            }
        }
    }
    content_text.map(|text| (text, pseudo_style))
}

/// Returns a `ComputedStyle` for the `::placeholder` pseudo-element applied to `element`,
/// or `None` if no `::placeholder` rule matches. The returned style inherits from
/// `host_style` and is further modified by matching `::placeholder` declarations.
pub fn compute_placeholder_style(
    element: &Element,
    stylesheet: &Stylesheet,
    host_style: &ComputedStyle,
    viewport_width: u32,
) -> Option<ComputedStyle> {
    let identity = ElementIdentity::from(element);
    let ancestors: &[AncestorSlot] = &[];
    let mut pseudo_style = host_style.clone();
    let mut has_match = false;

    for rule in &stylesheet.rules {
        if let Some(cond) = &rule.media {
            if !cond.matches(viewport_width) {
                continue;
            }
        }
        let host_matches = rule.selectors.iter().any(|sel| {
            sel.pseudo_element.as_ref() == Some(&PseudoElement::Placeholder)
                && sel.matches(
                    &identity,
                    ancestors,
                    0,
                    1,
                    &[],
                    &InteractiveState::default(),
                )
        });
        if !host_matches {
            continue;
        }
        has_match = true;
        for decl in &rule.declarations {
            apply_declaration(&mut pseudo_style, decl, host_style.font_size_px);
        }
    }
    if has_match { Some(pseudo_style) } else { None }
}

/// Returns a `ComputedStyle` for the `::selection` pseudo-element applied to `element`,
/// or `None` if no `::selection` rule matches. The returned style is used for highlighting
/// selected text in inputs/textareas.
pub fn compute_selection_style(
    element: &Element,
    stylesheet: &Stylesheet,
    host_style: &ComputedStyle,
    viewport_width: u32,
) -> Option<ComputedStyle> {
    let identity = ElementIdentity::from(element);
    let ancestors: &[AncestorSlot] = &[];
    let mut pseudo_style = host_style.clone();
    let mut has_match = false;

    for rule in &stylesheet.rules {
        if let Some(cond) = &rule.media {
            if !cond.matches(viewport_width) {
                continue;
            }
        }
        let host_matches = rule.selectors.iter().any(|sel| {
            sel.pseudo_element.as_ref() == Some(&PseudoElement::Selection)
                && sel.matches(
                    &identity,
                    ancestors,
                    0,
                    1,
                    &[],
                    &InteractiveState::default(),
                )
        });
        if !host_matches {
            continue;
        }
        has_match = true;
        for decl in &rule.declarations {
            apply_declaration(&mut pseudo_style, decl, host_style.font_size_px);
        }
    }
    if has_match { Some(pseudo_style) } else { None }
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
    interactive: &InteractiveState,
) -> ComputedStyle {
    let mut style = ComputedStyle::for_element(&element.tag_name, parent_style);
    let parent_font_size = parent_style.map(|c| c.font_size_px).unwrap_or(16);
    apply_legacy_attributes(&mut style, element, parent_font_size);

    let identity = ElementIdentity::from(element);
    // O(1) ref bump — we avoid cloning the full BTreeMap unless this element has its own vars.
    let root_vars = Rc::clone(&stylesheet.root_vars);
    let mut element_vars: BTreeMap<String, String> = BTreeMap::new();
    // Apply media-conditional root vars that match the current viewport width.
    // These are stored separately from unconditional root_vars so they are only
    // applied when their @media condition is satisfied.
    for (cond, vars) in &stylesheet.media_root_vars {
        if cond.matches(viewport_width) {
            // CSS cascade: last declaration wins, so use insert (not or_insert_with).
            // A later matching @media block should override an earlier one for the same var.
            for (k, v) in vars {
                element_vars.insert(k.clone(), v.clone());
            }
        }
    }
    let mut applicable: Vec<(usize, usize, Declaration)> = Vec::new();

    for (rule_index, rule) in stylesheet.rules.iter().enumerate() {
        // Skip rules where ALL selectors are pseudo-element rules — they are handled by collect_pseudo_content
        if rule
            .selectors
            .iter()
            .all(|sel| sel.pseudo_element.is_some())
        {
            continue;
        }
        // Check media condition
        if let Some(cond) = &rule.media {
            if !cond.matches(viewport_width) {
                continue;
            }
        }
        for selector in &rule.selectors {
            // Skip pseudo-element selectors (::before/::after) — they are handled
            // by collect_pseudo_content and must not apply to the host element.
            // This also prevents mixed rules like `p::before, span { color: red }`
            // from incorrectly contributing declarations to the host `<p>`.
            if selector.pseudo_element.is_some() {
                continue;
            }
            if selector.matches(
                &identity,
                ancestors,
                sibling_index,
                sibling_count,
                preceding_siblings,
                interactive,
            ) {
                // First pass: collect CSS variables
                for decl in &rule.declarations {
                    if decl.property.starts_with("--") {
                        element_vars.insert(decl.property.clone(), decl.value.clone());
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
                element_vars.insert(decl.property.clone(), decl.value.clone());
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

    // Merge root_vars into element_vars once, before the declaration loop.
    // element_vars (from matched rules + inline style) takes priority via or_insert_with.
    // Doing this once here avoids repeated merge attempts on every var()-containing declaration.
    if !element_vars.is_empty() {
        for (k, v) in root_vars.iter() {
            element_vars.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }
    let vars_ref: &BTreeMap<String, String> = if element_vars.is_empty() {
        &*root_vars
    } else {
        &element_vars
    };

    for (_, _, mut declaration) in applicable {
        // skip CSS custom properties
        if declaration.property.starts_with("--") {
            continue;
        }
        // substitute var() references
        if declaration.value.contains("var(") {
            declaration.value = substitute_vars(&declaration.value, vars_ref);
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

fn parse_filter_value(input: &str, style: &mut ComputedStyle) {
    let value = input.trim().to_ascii_lowercase();
    let mut rest = value.as_str();
    while !rest.is_empty() {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }

        if let Some(inner) = rest.strip_prefix("blur(") {
            if let Some(end) = inner.find(')') {
                let arg = &inner[..end];
                if let Some(px) = parse_length(arg.trim(), 16) {
                    style.filter_blur_px = px;
                }
                rest = &inner[end + 1..];
                continue;
            }
        }
        if let Some(inner) = rest.strip_prefix("brightness(") {
            if let Some(end) = inner.find(')') {
                let arg = inner[..end].trim().trim_end_matches('%');
                let pct = arg.parse::<f32>().ok().unwrap_or(100.0);
                // If value > 2.0 it's a percentage (e.g. "80%"), otherwise a factor (e.g. "0.8")
                let factor = if pct <= 2.0 { pct } else { pct / 100.0 };
                style.filter_brightness = (factor * 10000.0).round() as u32;
                rest = &inner[end + 1..];
                continue;
            }
        }
        if let Some(inner) = rest.strip_prefix("opacity(") {
            if let Some(end) = inner.find(')') {
                let arg = inner[..end].trim().trim_end_matches('%');
                let pct = arg.parse::<f32>().ok().unwrap_or(1.0);
                let factor = if pct <= 1.0 { pct } else { pct / 100.0 };
                style.filter_opacity = (factor.clamp(0.0, 1.0) * 255.0).round() as u8;
                rest = &inner[end + 1..];
                continue;
            }
        }
        if let Some(inner) = rest.strip_prefix("grayscale(") {
            if let Some(end) = inner.find(')') {
                rest = &inner[end + 1..];
                continue;
            }
        }
        // Unknown filter function — skip to next space or closing paren
        if let Some(pos) = rest.find(|c: char| c == ' ' || c == ')') {
            rest = rest[pos..].trim_start_matches(')');
        } else {
            break;
        }
    }
}

fn apply_declaration(style: &mut ComputedStyle, declaration: &Declaration, parent_font_size: u32) {
    let value = &declaration.value;
    match declaration.property.as_str() {
        "color" => {
            if let Some(color) = parse_color(value) {
                style.color = color;
            }
        }
        "background" => {
            let v = value.trim();
            let vl = v.to_ascii_lowercase();
            if vl.contains("radial-gradient(") {
                style.background_gradient = parse_radial_gradient(v);
            } else if vl.contains("linear-gradient(") {
                style.background_gradient = parse_linear_gradient(v);
            } else if vl.starts_with("url(") {
                style.background_image_url = extract_url(v);
            } else {
                style.background_color = parse_color(v);
            }
        }
        "background-color" => {
            style.background_color = parse_color(value);
        }
        "background-image" => {
            let v = value.trim();
            let vl = v.to_ascii_lowercase();
            if vl == "none" {
                style.background_gradient = None;
                style.background_image_url = None;
            } else if vl.contains("radial-gradient(") {
                style.background_gradient = parse_radial_gradient(v);
            } else if vl.contains("linear-gradient(") {
                style.background_gradient = parse_linear_gradient(v);
            } else if vl.starts_with("url(") {
                style.background_image_url = extract_url(v);
            }
        }
        "background-size" => {
            let v = value.trim().to_ascii_lowercase();
            style.background_size = match v.as_str() {
                "cover" => BackgroundSize::Cover,
                "contain" => BackgroundSize::Contain,
                _ => BackgroundSize::Auto,
            };
        }
        "background-repeat" => {
            let v = value.trim().to_ascii_lowercase();
            style.background_repeat = match v.as_str() {
                "no-repeat" => BackgroundRepeat::NoRepeat,
                "repeat-x" => BackgroundRepeat::RepeatX,
                "repeat-y" => BackgroundRepeat::RepeatY,
                _ => BackgroundRepeat::Repeat,
            };
        }
        "background-position" => {
            let parse_pct = |s: &str| -> u32 {
                match s.trim() {
                    "left" | "top" => 0,
                    "center" => 50,
                    "right" | "bottom" => 100,
                    other => other
                        .trim_end_matches('%')
                        .parse::<f32>()
                        .ok()
                        .map(|f| f.clamp(0.0, 100.0).round() as u32)
                        .unwrap_or(50),
                }
            };
            let parts: Vec<&str> = value.split_whitespace().collect();
            match parts.as_slice() {
                [x, y, ..] => {
                    style.background_position_x = parse_pct(x);
                    style.background_position_y = parse_pct(y);
                }
                [single] => {
                    let v = parse_pct(single);
                    style.background_position_x = v;
                    style.background_position_y = v;
                }
                _ => {}
            }
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
            // Shorthand: may contain line keywords, color, style.
            // e.g. "underline", "underline line-through", "underline red", "none"
            let v = value.trim().to_ascii_lowercase();
            if v.contains("none") {
                style.underline = false;
                style.line_through = false;
            } else {
                if v.contains("underline") {
                    style.underline = true;
                }
                if v.contains("line-through") {
                    style.line_through = true;
                }
                // Extract color token (any non-keyword token that parses as color)
                for token in v.split_whitespace() {
                    if !matches!(token, "underline" | "overline" | "line-through" | "blink"
                        | "solid" | "dashed" | "dotted" | "double" | "wavy" | "none") {
                        if let Some(c) = parse_color(token) {
                            style.text_decoration_color = Some(c);
                        }
                    }
                }
            }
        }
        "text-decoration-line" => {
            let v = value.trim().to_ascii_lowercase();
            if v.contains("none") {
                style.underline = false;
                style.line_through = false;
            } else {
                for token in v.split_whitespace() {
                    match token {
                        "underline" => style.underline = true,
                        "line-through" => style.line_through = true,
                        _ => {}
                    }
                }
            }
        }
        "text-decoration-color" => {
            style.text_decoration_color = parse_color(value);
        }
        "text-decoration-thickness" | "text-underline-offset" => {
            // Parsed but not yet applied; stored as no-op for future use
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
        "word-break" => {
            match value.trim() {
                "break-all" => style.word_break = WordBreak::BreakAll,
                "keep-all"  => style.word_break = WordBreak::KeepAll,
                _           => style.word_break = WordBreak::Normal,
            }
        }
        "overflow-wrap" | "word-wrap" => {
            style.overflow_wrap_break_word = value.trim() == "break-word";
        }
        "margin" => {
            parse_margin_shorthand(style, value, parent_font_size);
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
            let v = value.trim().to_ascii_lowercase();
            if v == "auto" {
                style.margin_right_auto = true;
                style.margin.right = 0;
            } else if let Some(v) = parse_length(value, parent_font_size) {
                style.margin_right_auto = false;
                style.margin.right = v;
            }
        }
        "margin-bottom" => {
            if let Some(v) = parse_length(value, parent_font_size) {
                style.margin.bottom = v;
            }
        }
        "margin-left" => {
            let v = value.trim().to_ascii_lowercase();
            if v == "auto" {
                style.margin_left_auto = true;
                style.margin.left = 0;
            } else if let Some(v) = parse_length(value, parent_font_size) {
                style.margin_left_auto = false;
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
        "outline-offset" => {
            if let Some(px) = parse_signed_length(value, parent_font_size) {
                style.outline_offset = px;
            }
        }
        "outline-style" => {
            let v = value.trim().to_ascii_lowercase();
            style.outline_visible = !matches!(v.as_str(), "none" | "hidden");
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
        "content" => {
            let v = strip_css_string_quotes(value.trim());
            if v == "none" || v == "normal" || v.is_empty() {
                style.content = None;
            } else {
                style.content = Some(v.to_string());
            }
        }
        "box-shadow" => {
            style.box_shadows = parse_box_shadows(value);
        }
        "cursor" => {
            style.cursor_kind = match value.trim().to_ascii_lowercase().as_str() {
                "pointer" => CursorKind::Pointer,
                "text" | "i-beam" => CursorKind::Text,
                "move" => CursorKind::Move,
                "crosshair" => CursorKind::Crosshair,
                "wait" | "progress" => CursorKind::Wait,
                "help" => CursorKind::Help,
                "not-allowed" | "no-drop" => CursorKind::NotAllowed,
                "grab" => CursorKind::Grab,
                "grabbing" => CursorKind::Grabbing,
                "zoom-in" => CursorKind::ZoomIn,
                "zoom-out" => CursorKind::ZoomOut,
                "none" => CursorKind::None,
                "default" => CursorKind::Default,
                _ => CursorKind::Auto,
            };
            style.cursor_pointer = matches!(style.cursor_kind, CursorKind::Pointer);
        }
        "pointer-events" => {
            style.pointer_events_none = value.trim().to_ascii_lowercase() == "none";
        }
        "position" => {
            style.position = match value.trim().to_ascii_lowercase().as_str() {
                "relative" => Position::Relative,
                "absolute" => Position::Absolute,
                "fixed" => Position::Fixed,
                "sticky" | "-webkit-sticky" => Position::Sticky,
                _ => Position::Static,
            };
        }
        "z-index" => {
            if let Ok(n) = value.trim().parse::<i32>() {
                style.z_index = Some(n);
            }
        }
        "top" => {
            style.top = parse_signed_length(value, parent_font_size);
        }
        "right" => {
            style.right = parse_signed_length(value, parent_font_size);
        }
        "bottom" => {
            style.bottom = parse_signed_length(value, parent_font_size);
        }
        "left" => {
            style.left = parse_signed_length(value, parent_font_size);
        }
        "flex-direction" => {
            style.flex_direction = match value.trim().to_ascii_lowercase().as_str() {
                "column" => FlexDirection::Column,
                "row-reverse" => FlexDirection::RowReverse,
                "column-reverse" => FlexDirection::ColumnReverse,
                _ => FlexDirection::Row,
            };
        }
        "flex-wrap" => {
            style.flex_wrap = match value.trim().to_ascii_lowercase().as_str() {
                "wrap" => FlexWrap::Wrap,
                "wrap-reverse" => FlexWrap::WrapReverse,
                _ => FlexWrap::NoWrap,
            };
        }
        "align-items" => {
            style.align_items = match value.trim().to_ascii_lowercase().as_str() {
                "flex-start" | "start" => AlignItems::FlexStart,
                "flex-end" | "end" => AlignItems::FlexEnd,
                "center" => AlignItems::Center,
                "baseline" => AlignItems::Baseline,
                _ => AlignItems::Stretch,
            };
        }
        "justify-content" => {
            style.justify_content = match value.trim().to_ascii_lowercase().as_str() {
                "flex-end" | "end" => JustifyContent::FlexEnd,
                "center" => JustifyContent::Center,
                "space-between" => JustifyContent::SpaceBetween,
                "space-around" => JustifyContent::SpaceAround,
                "space-evenly" => JustifyContent::SpaceEvenly,
                _ => JustifyContent::FlexStart,
            };
        }
        "align-self" => {
            style.align_self = match value.trim().to_ascii_lowercase().as_str() {
                "flex-start" | "start" => AlignSelf::FlexStart,
                "flex-end" | "end" => AlignSelf::FlexEnd,
                "center" => AlignSelf::Center,
                "baseline" => AlignSelf::Baseline,
                "stretch" => AlignSelf::Stretch,
                _ => AlignSelf::Auto,
            };
        }
        "align-content" => {
            style.align_content = match value.trim().to_ascii_lowercase().as_str() {
                "flex-start" | "start" => AlignContent::FlexStart,
                "flex-end" | "end" => AlignContent::FlexEnd,
                "center" => AlignContent::Center,
                "space-between" => AlignContent::SpaceBetween,
                "space-around" => AlignContent::SpaceAround,
                _ => AlignContent::Stretch,
            };
        }
        "flex-flow" => {
            // flex-flow: <direction> || <wrap>
            let parts: Vec<&str> = value.split_whitespace().collect();
            for part in &parts {
                match part.trim().to_ascii_lowercase().as_str() {
                    "row" => style.flex_direction = FlexDirection::Row,
                    "row-reverse" => style.flex_direction = FlexDirection::RowReverse,
                    "column" => style.flex_direction = FlexDirection::Column,
                    "column-reverse" => style.flex_direction = FlexDirection::ColumnReverse,
                    "nowrap" => style.flex_wrap = FlexWrap::NoWrap,
                    "wrap" => style.flex_wrap = FlexWrap::Wrap,
                    "wrap-reverse" => style.flex_wrap = FlexWrap::WrapReverse,
                    _ => {}
                }
            }
        }
        "flex-grow" => {
            if let Ok(f) = value.trim().parse::<f32>() {
                style.flex_grow = (f * 100.0).round() as u32;
            }
        }
        "flex-shrink" => {
            if let Ok(f) = value.trim().parse::<f32>() {
                style.flex_shrink = (f * 100.0).round() as u32;
            }
        }
        "flex-basis" => {
            if value.trim().to_ascii_lowercase() == "auto" {
                style.flex_basis = None;
            } else {
                style.flex_basis = parse_length_value(value, parent_font_size);
            }
        }
        "flex" => {
            let parts: Vec<&str> = value.split_whitespace().collect();
            if parts.len() >= 1 {
                if let Ok(g) = parts[0].parse::<f32>() {
                    style.flex_grow = (g * 100.0).round() as u32;
                }
            }
            if parts.len() >= 2 {
                if let Ok(s) = parts[1].parse::<f32>() {
                    style.flex_shrink = (s * 100.0).round() as u32;
                }
            }
            if parts.len() >= 3 {
                style.flex_basis = parse_length_value(parts[2], parent_font_size);
            }
        }
        "gap" | "grid-gap" => {
            if let Some(px) = parse_length(value, parent_font_size) {
                style.gap = px;
            }
        }
        "row-gap" => {
            if let Some(px) = parse_length(value, parent_font_size) {
                style.gap = px;
            }
        }
        "column-gap" => {}
        // ── Grid properties ──────────────────────────────────────────────────
        "grid-template-columns" => {
            style.grid_template_columns = parse_grid_track_list(value, parent_font_size);
        }
        "grid-template-rows" => {
            style.grid_template_rows = parse_grid_track_list(value, parent_font_size);
        }
        "grid-auto-rows" => {
            style.grid_auto_rows = parse_grid_track_size(value.trim(), parent_font_size)
                .unwrap_or(GridTrackSize::Auto);
        }
        "grid-auto-columns" => {
            style.grid_auto_columns = parse_grid_track_size(value.trim(), parent_font_size)
                .unwrap_or(GridTrackSize::Auto);
        }
        "grid-auto-flow" => {
            let v = value.trim().to_ascii_lowercase();
            let mut row = true;
            let mut dense = false;
            for token in v.split_whitespace() {
                match token {
                    "row" => row = true,
                    "column" => row = false,
                    "dense" => dense = true,
                    _ => {}
                }
            }
            style.grid_auto_flow = match (row, dense) {
                (true, false) => GridAutoFlow::Row,
                (true, true) => GridAutoFlow::RowDense,
                (false, false) => GridAutoFlow::Column,
                (false, true) => GridAutoFlow::ColumnDense,
            };
        }
        "grid-column" => {
            style.grid_column = parse_grid_placement(value);
        }
        "grid-row" => {
            style.grid_row = parse_grid_placement(value);
        }
        "grid-column-start" => {
            style.grid_column.start = parse_grid_line(value);
        }
        "grid-column-end" => {
            if let Some(end) = parse_grid_line(value) {
                if let Some(start) = style.grid_column.start {
                    let span = (end - start).max(1) as u32;
                    style.grid_column.span = Some(span);
                } else {
                    style.grid_column.start = Some(end);
                }
            }
        }
        "grid-row-start" => {
            style.grid_row.start = parse_grid_line(value);
        }
        "grid-row-end" => {
            if let Some(end) = parse_grid_line(value) {
                if let Some(start) = style.grid_row.start {
                    let span = (end - start).max(1) as u32;
                    style.grid_row.span = Some(span);
                } else {
                    style.grid_row.start = Some(end);
                }
            }
        }
        "grid-template-areas" => {
            style.grid_template_areas = parse_grid_template_areas(value);
        }
        "grid-area" => {
            let v = value.trim();
            if !v.contains('/') { style.grid_area_name = Some(v.to_string()); }
        }
        "grid-template" | "grid" => {
            // Simplified: skip complex shorthand
        }
        "order" => {
            if let Ok(n) = value.trim().parse::<i32>() {
                style.order = n;
            }
        }
        "aspect-ratio" => {
            let v = value.trim().to_ascii_lowercase();
            if v == "auto" {
                style.aspect_ratio = None;
            } else {
                let ratio = if let Some((num, den)) = v.split_once('/') {
                    num.trim()
                        .parse::<f32>()
                        .ok()
                        .zip(den.trim().parse::<f32>().ok())
                        .and_then(|(n, d)| if d != 0.0 { Some(n / d) } else { None })
                } else {
                    v.trim().parse::<f32>().ok().filter(|&r| r > 0.0)
                };
                if let Some(r) = ratio {
                    style.aspect_ratio = Some((r * 1000.0).round() as u32);
                }
            }
        }
        "object-fit" => {
            style.object_fit = match value.trim() {
                "contain" => ObjectFit::Contain,
                "cover" => ObjectFit::Cover,
                "scale-down" => ObjectFit::ScaleDown,
                "none" => ObjectFit::None,
                _ => ObjectFit::Fill,
            };
        }
        "filter" | "-webkit-filter" => {
            parse_filter_value(value, style);
        }
        "text-overflow" => {
            let v = value.trim().to_ascii_lowercase();
            style.text_overflow_ellipsis = v.contains("ellipsis");
        }
        "text-shadow" => {
            let v = value.trim();
            if v.to_ascii_lowercase() == "none" {
                style.text_shadow = None;
            } else {
                style.text_shadow = parse_text_shadow(v, parent_font_size);
            }
        }
        "scroll-behavior" => {
            style.scroll_behavior_smooth = value.trim().eq_ignore_ascii_case("smooth");
        }
        "clip-path" => {
            style.clip_path = parse_clip_path(value.trim());
        }
        "writing-mode" => {
            style.writing_mode = match value.trim().to_ascii_lowercase().as_str() {
                "vertical-rl" | "tb-rl" => WritingMode::VerticalRl,
                "vertical-lr" | "tb-lr" => WritingMode::VerticalLr,
                _ => WritingMode::HorizontalTb,
            };
        }
        "direction" => {
            style.direction = match value.trim().to_ascii_lowercase().as_str() {
                "rtl" => Direction::Rtl,
                _ => Direction::Ltr,
            };
        }
        // No-op properties — parsed to prevent warnings, not yet implemented
        "overscroll-behavior"
        | "overscroll-behavior-x"
        | "overscroll-behavior-y"
        | "resize"
        | "text-orientation"
        | "unicode-bidi"
        | "scroll-snap-type"
        | "scroll-snap-align"
        | "scroll-padding"
        | "scroll-padding-top"
        | "will-change"
        | "isolation"
        | "mix-blend-mode"
        | "backdrop-filter"
        | "-webkit-overflow-scrolling"
        | "touch-action"
        | "user-select"
        | "-webkit-user-select"
        | "appearance"
        | "-webkit-appearance"
        | "-moz-appearance"
        | "contain"
        | "content-visibility" => {
            // Parsed and ignored — no implementation yet
        }
        "object-position" => {
            let parse_pct = |s: &str| -> u32 {
                match s.trim() {
                    "left" | "top" => 0,
                    "center" => 50,
                    "right" | "bottom" => 100,
                    other => other
                        .trim_end_matches('%')
                        .parse::<f32>()
                        .ok()
                        .map(|f| f.clamp(0.0, 100.0).round() as u32)
                        .unwrap_or(50),
                }
            };
            let parts: Vec<&str> = value.split_whitespace().collect();
            match parts.as_slice() {
                [x, y, ..] => {
                    style.object_position_x = parse_pct(x);
                    style.object_position_y = parse_pct(y);
                }
                [single] => {
                    let v = parse_pct(single);
                    style.object_position_x = v;
                    style.object_position_y = v;
                }
                _ => {}
            }
        }
        "transform" => {
            parse_transform_into(value, style);
        }
        "transform-origin" => {
            let parts: Vec<&str> = value.split_whitespace().collect();
            if let Some(x) = parts.first() {
                style.transform_origin_x = parse_pct_permille(x, 500);
            }
            if let Some(y) = parts.get(1) {
                style.transform_origin_y = parse_pct_permille(y, 500);
            }
        }
        "counter-reset" => {
            style.counter_reset = parse_counter_list(value, 0);
        }
        "counter-increment" => {
            style.counter_increment = parse_counter_list(value, 1);
        }
        _ => {}
    }
}

/// Parse a CSS counter list value (for `counter-reset` or `counter-increment`).
/// Format: `<name> [<integer>]` pairs, e.g. `item 1` or `section 0 chapter`.
/// `default_val` is used when no integer follows the name (0 for reset, 1 for increment).
fn parse_counter_list(value: &str, default_val: i32) -> Vec<(String, i32)> {
    if value.trim().eq_ignore_ascii_case("none") {
        return vec![];
    }
    let mut result = Vec::new();
    let tokens: Vec<&str> = value.split_whitespace().collect();
    let mut i = 0;
    while i < tokens.len() {
        let name = tokens[i].to_string();
        i += 1;
        let val = if i < tokens.len() {
            if let Ok(n) = tokens[i].parse::<i32>() {
                i += 1;
                n
            } else {
                default_val
            }
        } else {
            default_val
        };
        if !name.is_empty() {
            result.push((name, val));
        }
    }
    result
}

/// Resolve `counter(name)` and `counter(name, style)` references in a content string.
/// Replaces each `counter(...)` call with the current integer value from `counters`.
pub fn resolve_content_counters(content: &str, counters: &std::collections::HashMap<String, i32>) -> String {
    let mut result = String::new();
    let mut s = content;
    while let Some(pos) = s.find("counter(") {
        result.push_str(&s[..pos]);
        let after = &s[pos + "counter(".len()..];
        if let Some(end) = after.find(')') {
            let args = &after[..end];
            // Take the first arg (counter name), ignore optional style arg
            let name = args.split(',').next().unwrap_or("").trim();
            let val = counters.get(name).copied().unwrap_or(0);
            result.push_str(&val.to_string());
            s = &after[end + 1..];
        } else {
            // Unbalanced paren — emit literally and stop
            result.push_str("counter(");
            s = after;
        }
    }
    result.push_str(s);
    result
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

    // <body background="..."> — annotate_resource_urls pre-resolves this to an absolute URL
    // stored in data-scratch-background; wire it up as background_image_url so it gets
    // fetched and drawn just like CSS background-image: url(...).
    if let Some(bg_url) = element.attribute("data-scratch-background") {
        style.background_image_url = Some(bg_url.to_string());
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
        // Extract pseudo_element from the last part's simple selector
        let pseudo_element = parts.last().and_then(|p| p.simple.pseudo_element.clone());
        Some(Selector {
            parts,
            pseudo_element,
        })
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
                    i += 1; // skip second ':'
                    // collect pseudo-element name
                    let mut pe_name = String::new();
                    while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '-') {
                        pe_name.push(chars[i]);
                        i += 1;
                    }
                    match pe_name.to_ascii_lowercase().as_str() {
                        "before" => selector.pseudo_element = Some(PseudoElement::Before),
                        "after" => selector.pseudo_element = Some(PseudoElement::After),
                        "placeholder" => selector.pseudo_element = Some(PseudoElement::Placeholder),
                        "selection" => selector.pseudo_element = Some(PseudoElement::Selection),
                        _ => selector.never_match = true,
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
        && selector.pseudo_element.is_none()
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
        "hover" => Some(PseudoClass::Hover),
        "focus" | "focus-visible" | "focus-within" => Some(PseudoClass::Focus),
        "active" => Some(PseudoClass::Active),
        "checked" => Some(PseudoClass::Checked),
        "disabled" => Some(PseudoClass::Disabled),
        "enabled" => Some(PseudoClass::Enabled),
        // Ignored pseudo-classes (no-op)
        "visited" | "link" | "root" | "empty" | "placeholder" => None,
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
        self.parts
            .iter()
            .map(|part| part.simple.specificity())
            .sum()
    }

    fn matches(
        &self,
        element: &ElementIdentity,
        ancestors: &[AncestorSlot],
        sibling_index: usize,
        sibling_count: usize,
        preceding_siblings: &[ElementIdentity],
        interactive: &InteractiveState,
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
        self.matches_part(
            last_index,
            &current,
            ancestors,
            preceding_siblings,
            interactive,
        )
    }

    fn matches_part(
        &self,
        part_index: usize,
        current: &AncestorSlot,
        ancestors: &[AncestorSlot],
        current_preceding_siblings: &[ElementIdentity],
        interactive: &InteractiveState,
    ) -> bool {
        if !self.parts[part_index]
            .simple
            .matches_slot(current, interactive)
        {
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
                        interactive,
                    )
                })
            }
            Combinator::Child => ancestors.last().is_some_and(|parent| {
                self.matches_part(
                    part_index - 1,
                    parent,
                    &ancestors[..ancestors.len() - 1],
                    parent.preceding_siblings(),
                    interactive,
                )
            }),
            Combinator::AdjacentSibling => {
                current_preceding_siblings.last().is_some_and(|sibling| {
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
                        interactive,
                    )
                })
            }
            Combinator::GeneralSibling => current_preceding_siblings.iter().enumerate().rev().any(
                |(sibling_index, sibling)| {
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
                        interactive,
                    )
                },
            ),
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
        let class_score = (self.classes.len() + non_not_pseudo_count + self.attributes.len()) * 10;
        let tag_score = self.tag_name.is_some() as usize;
        id_score + class_score + not_score + tag_score
    }

    fn matches_slot(&self, slot: &AncestorSlot, interactive: &InteractiveState) -> bool {
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
                PseudoClass::Not(selectors) => !selectors
                    .iter()
                    .any(|selector| selector.matches_slot(slot, interactive)),
                PseudoClass::Hover => {
                    slot.element.node_id.is_some()
                        && slot.element.node_id == interactive.hovered_node_id
                }
                PseudoClass::Focus => {
                    slot.element.node_id.is_some()
                        && slot.element.node_id == interactive.focused_node_id
                }
                PseudoClass::Active => slot
                    .element
                    .node_id
                    .is_some_and(|id| interactive.active_node_ids.contains(&id)),
                PseudoClass::Checked => slot.element.attributes.contains_key("checked"),
                PseudoClass::Disabled => slot.element.attributes.contains_key("disabled"),
                PseudoClass::Enabled => !slot.element.attributes.contains_key("disabled"),
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
        let node_id = value
            .attribute("data-tobira-node-id")
            .and_then(|v| v.parse::<usize>().ok());

        Self {
            tag_name: value.tag_name.clone(),
            id,
            classes,
            attributes: value.attributes.clone(),
            node_id,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Property parsers
// ─────────────────────────────────────────────────────────────────────────────

fn parse_display(input: &str) -> Option<Display> {
    match input.trim().to_ascii_lowercase().as_str() {
        "block" | "flow-root" | "table" | "table-row" => Some(Display::Block),
        "flex" => Some(Display::Flex),
        "inline-flex" => Some(Display::InlineFlex),
        "grid" => Some(Display::Grid),
        "inline-grid" => Some(Display::InlineGrid),
        "inline" | "inline-block" | "table-cell" | "contents" => Some(Display::Inline),
        "list-item" => Some(Display::ListItem),
        "none" => Some(Display::None),
        _ => None,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Grid parsing helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Parse a grid track list like "100px 1fr auto repeat(3, 200px)".
fn parse_grid_track_list(input: &str, parent_font_size: u32) -> Vec<GridTrackSize> {
    let mut tracks = Vec::new();
    let input = input.trim();
    let chars: Vec<char> = input.chars().collect();
    let mut buf = String::new();
    let mut depth = 0usize;

    for &ch in &chars {
        match ch {
            '(' => {
                depth += 1;
                buf.push(ch);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                buf.push(ch);
            }
            ' ' | '\t' if depth == 0 => {
                let token = buf.trim().to_string();
                if !token.is_empty() {
                    if token.starts_with("repeat(") {
                        tracks.extend(expand_grid_repeat(&token, parent_font_size));
                    } else if let Some(size) = parse_grid_track_size(&token, parent_font_size) {
                        tracks.push(size);
                    }
                }
                buf.clear();
            }
            _ => buf.push(ch),
        }
    }
    let token = buf.trim().to_string();
    if !token.is_empty() {
        if token.starts_with("repeat(") {
            tracks.extend(expand_grid_repeat(&token, parent_font_size));
        } else if let Some(size) = parse_grid_track_size(&token, parent_font_size) {
            tracks.push(size);
        }
    }
    tracks
}

/// Expand `repeat(N, track-list)` into N copies.
fn expand_grid_repeat(token: &str, parent_font_size: u32) -> Vec<GridTrackSize> {
    let inner = token
        .strip_prefix("repeat(")
        .and_then(|s| s.strip_suffix(')'));
    let inner = match inner {
        Some(s) => s,
        None => return Vec::new(),
    };
    let comma_pos = inner.find(',');
    let (count_str, track_str) = match comma_pos {
        Some(i) => (&inner[..i], &inner[i + 1..]),
        None => return Vec::new(),
    };
    let count: usize = match count_str.trim().parse::<usize>() {
        Ok(n) if n > 0 => n,
        _ => 1, // auto-fill/auto-fit: treat as 1
    };
    let track_sizes = parse_grid_track_list(track_str.trim(), parent_font_size);
    if track_sizes.is_empty() {
        return Vec::new();
    }
    track_sizes.into_iter().cycle().take(count).collect()
}

fn parse_grid_track_size(token: &str, parent_font_size: u32) -> Option<GridTrackSize> {
    let t = token.trim().to_ascii_lowercase();
    if t == "auto" {
        return Some(GridTrackSize::Auto);
    }
    if t == "min-content" {
        return Some(GridTrackSize::MinContent);
    }
    if t == "max-content" {
        return Some(GridTrackSize::MaxContent);
    }
    if let Some(n) = t.strip_suffix("fr") {
        return parse_float(n).map(|f| GridTrackSize::Fr((f * 1000.0).round() as u32));
    }
    if let Some(n) = t.strip_suffix('%') {
        return parse_float(n).map(|f| GridTrackSize::Percent((f * 100.0).round() as u32));
    }
    parse_length(&t, parent_font_size).map(GridTrackSize::Pixels)
}

fn parse_grid_placement(value: &str) -> GridPlacement {
    let parts: Vec<&str> = value.split('/').collect();
    match parts.as_slice() {
        [start_str, end_str] => {
            let start = parse_grid_line(start_str.trim());
            let end_val = end_str.trim();
            let span = if let Some(rest) = end_val.strip_prefix("span") {
                rest.trim().parse::<u32>().ok()
            } else if let Some(end_line) = parse_grid_line(end_val) {
                start.map(|s| (end_line - s).max(1) as u32)
            } else {
                None
            };
            GridPlacement { start, span }
        }
        [single] => {
            let s = single.trim();
            if let Some(rest) = s.strip_prefix("span") {
                GridPlacement {
                    start: None,
                    span: rest.trim().parse().ok(),
                }
            } else {
                GridPlacement {
                    start: parse_grid_line(s),
                    span: None,
                }
            }
        }
        _ => GridPlacement::default(),
    }
}

fn parse_grid_line(s: &str) -> Option<i32> {
    let s = s.trim();
    if s == "auto" {
        return None;
    }
    s.parse::<i32>().ok()
}


/// Parse a grid-template-areas value like "header header" "sidebar main".
/// Returns a Vec of rows, each row being a Vec of column area names.
/// Both single-quoted and double-quoted strings are supported.
fn parse_grid_template_areas(value: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' || c == '\'' {
            let quote = c;
            let mut row_str = String::new();
            for ch in chars.by_ref() {
                if ch == quote { break; }
                row_str.push(ch);
            }
            let cells: Vec<String> = row_str.split_whitespace().map(|s| s.to_string()).collect();
            if !cells.is_empty() { rows.push(cells); }
        }
    }
    rows
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
        "nowrap" => Some(WhiteSpaceMode::NoWrap),
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

fn parse_box_shadows(value: &str) -> Vec<BoxShadow> {
    if value.trim().to_ascii_lowercase() == "none" {
        return Vec::new();
    }
    // Split on commas at the top level (avoiding rgba(r,g,b,a) inner commas).
    split_at_top_level(value, ',')
        .into_iter()
        .filter_map(|s| parse_single_box_shadow(s.trim()))
        .collect()
}

fn parse_single_box_shadow(value: &str) -> Option<BoxShadow> {
    let v = value.trim();
    if v.is_empty() || v.to_ascii_lowercase() == "none" {
        return None;
    }

    // Split tokens at spaces (top-level only, respecting parentheses for rgb()/rgba() colors).
    // Note: only ASCII space is used as separator; tabs and other whitespace between
    // tokens are not treated as delimiters. This is an approximation that covers
    // standard CSS box-shadow syntax. Exotic whitespace (e.g. `2px\t2px`) would
    // produce unparseable tokens.
    let tokens: Vec<String> = split_at_top_level(v, ' ')
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    // Check for 'inset' keyword anywhere in the token list.
    let inset = tokens.iter().any(|t| t.to_ascii_lowercase() == "inset");

    // Filter out the 'inset' keyword, leaving only lengths and color tokens.
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
    let mut spread: i32 = 0;
    let mut color: Option<u32> = None;
    let mut length_count = 0;

    for token in &tokens {
        // Note: parse_signed_length uses a hardcoded font-size of 16px,
        // so `em`/`rem` units in box-shadow offsets resolve against 16px rather
        // than the element's actual font size. This is a known approximation.
        if let Some(val) = parse_signed_length(token, 16) {
            match length_count {
                0 => offset_x = val,
                1 => offset_y = val,
                2 => blur = val.max(0) as u32,
                3 => spread = val,
                _ => {}
            }
            length_count += 1;
        } else if let Some(c) = parse_color(token) {
            color = Some(c);
        }
    }

    if length_count < 2 {
        return None;
    }

    Some(BoxShadow {
        offset_x,
        offset_y,
        blur,
        spread,
        color,
        inset,
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
        style.outline_visible = false;
        return;
    }
    for token in v.split_whitespace() {
        match token {
            "none" | "hidden" => {
                style.outline_width = 0;
                style.outline_visible = false;
            }
            "solid" | "dashed" | "dotted" | "double" | "groove" | "ridge" | "inset" | "outset" | "auto" => {
                // style keyword — outline is visible
                style.outline_visible = true;
            }
            _ => {
                if let Some(px) = parse_length(token, parent_font_size) {
                    style.outline_width = px;
                } else if let Some(color) = parse_color(token) {
                    style.outline_color = Some(color);
                }
            }
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

fn parse_margin_shorthand(style: &mut ComputedStyle, input: &str, parent_font_size: u32) {
    // Reset auto flags
    style.margin_left_auto = false;
    style.margin_right_auto = false;

    let tokens: Vec<&str> = input.split_whitespace().collect();
    // Parse each token as length or auto (None means auto)
    let parsed: Vec<Option<u32>> = tokens
        .iter()
        .map(|t| {
            if t.to_ascii_lowercase() == "auto" {
                None // auto
            } else {
                parse_length(t, parent_font_size)
            }
        })
        .collect();

    // Apply CSS box shorthand rules (1/2/3/4 values)
    // None means "auto" (0px, flag set separately)
    let resolve = |v: Option<u32>| v.unwrap_or(0);
    match parsed.as_slice() {
        [all] => {
            let v = resolve(*all);
            style.margin = EdgeSizes::all(v);
            if all.is_none() {
                style.margin_left_auto = true;
                style.margin_right_auto = true;
            }
        }
        [vertical, horizontal] => {
            style.margin.top = resolve(*vertical);
            style.margin.bottom = resolve(*vertical);
            style.margin.left = resolve(*horizontal);
            style.margin.right = resolve(*horizontal);
            if horizontal.is_none() {
                style.margin_left_auto = true;
                style.margin_right_auto = true;
            }
        }
        [top, horizontal, bottom] => {
            style.margin.top = resolve(*top);
            style.margin.bottom = resolve(*bottom);
            style.margin.left = resolve(*horizontal);
            style.margin.right = resolve(*horizontal);
            if horizontal.is_none() {
                style.margin_left_auto = true;
                style.margin_right_auto = true;
            }
        }
        [top, right, bottom, left] => {
            style.margin.top = resolve(*top);
            style.margin.right = resolve(*right);
            style.margin.bottom = resolve(*bottom);
            style.margin.left = resolve(*left);
            if left.is_none() {
                style.margin_left_auto = true;
            }
            if right.is_none() {
                style.margin_right_auto = true;
            }
        }
        _ => {} // invalid, leave unchanged
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
    } else if value.contains("georgia") || value.contains("times") || value == "serif" {
        Some(FontFamilyKind::Serif)
    } else if !value.is_empty() {
        Some(FontFamilyKind::Sans)
    } else {
        None
    }
}

/// Split comma-separated CSS function arguments, respecting nested parentheses.
fn split_css_fn_args(expr: &str) -> Vec<&str> {
    let mut args: Vec<&str> = Vec::new();
    let mut depth: u32 = 0;
    let mut start = 0;
    for (i, ch) in expr.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                args.push(&expr[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    args.push(&expr[start..]);
    args
}

/// CSS `min(a, b, ...)` / `max(a, b, ...)` resolver (is_max=true for max).
fn parse_css_min_max(expr: &str, parent_font_size: u32, is_max: bool) -> Option<u32> {
    let mut result: Option<u32> = None;
    for arg in split_css_fn_args(expr) {
        if let Some(v) = parse_length(arg.trim(), parent_font_size) {
            result = Some(match result {
                None => v,
                Some(r) => {
                    if is_max {
                        r.max(v)
                    } else {
                        r.min(v)
                    }
                }
            });
        }
    }
    result
}

/// CSS `clamp(min, val, max)` resolver.
fn parse_css_clamp(expr: &str, parent_font_size: u32) -> Option<u32> {
    let args = split_css_fn_args(expr);
    if args.len() != 3 {
        return None;
    }
    let lo = parse_length(args[0].trim(), parent_font_size)? as f32;
    let val = parse_length(args[1].trim(), parent_font_size)? as f32;
    let hi = parse_length(args[2].trim(), parent_font_size)? as f32;
    Some(val.clamp(lo, hi).round() as u32)
}

/// Parse a CSS length. Handles calc(), clamp(), min(), max(), vw/vh, px, em, rem, %
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

    // min()
    if let Some(inner) = value.strip_prefix("min(").and_then(|s| s.strip_suffix(')')) {
        return parse_css_min_max(inner, parent_font_size, false);
    }
    // max()
    if let Some(inner) = value.strip_prefix("max(").and_then(|s| s.strip_suffix(')')) {
        return parse_css_min_max(inner, parent_font_size, true);
    }
    // clamp()
    if let Some(inner) = value
        .strip_prefix("clamp(")
        .and_then(|s| s.strip_suffix(')'))
    {
        return parse_css_clamp(inner, parent_font_size);
    }

    if let Some(number) = value.strip_suffix("px") {
        return parse_float(number).map(|p| p.round().max(0.0) as u32);
    }

    if let Some(number) = value.strip_suffix("vw") {
        return parse_float(number).map(|p| (p * 1280.0 / 100.0).round() as u32);
    }

    if let Some(number) = value.strip_suffix("vh") {
        return parse_float(number).map(|p| (p * 800.0 / 100.0).round() as u32); // viewport 800px tall — must match js.rs innerHeight
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

/// Like parse_length but allows negative values; returns i32.
/// Using i32 rather than i16 avoids silent truncation for large offsets
/// (e.g. box-shadow offsets > 32767px which are legal in CSS).
fn parse_signed_length(input: &str, parent_font_size: u32) -> Option<i32> {
    let value = input.trim().to_ascii_lowercase();
    if value == "0" {
        return Some(0);
    }

    if value.starts_with('-') {
        let positive = &value[1..];
        let px = parse_length(positive, parent_font_size)?.min(i32::MAX as u32) as i32;
        return Some(-px);
    }

    // Clamp to i32::MAX before casting so pathological lengths (>= 2^31 px) don't wrap.
    parse_length(input, parent_font_size).map(|v| v.min(i32::MAX as u32) as i32)
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
        if i + 1 >= values.len() {
            break; // malformed calc() — guard against index out of bounds
        }
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
    let mut result = *values.first()?;
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
    // nested min()/max()/clamp() inside calc()
    if let Some(inner) = t.strip_prefix("min(").and_then(|s| s.strip_suffix(')')) {
        return parse_css_min_max(inner, parent_font_size, false).map(|v| v as f32);
    }
    if let Some(inner) = t.strip_prefix("max(").and_then(|s| s.strip_suffix(')')) {
        return parse_css_min_max(inner, parent_font_size, true).map(|v| v as f32);
    }
    if let Some(inner) = t.strip_prefix("clamp(").and_then(|s| s.strip_suffix(')')) {
        return parse_css_clamp(inner, parent_font_size).map(|v| v as f32);
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
    match value.as_str() {
        "min-content" => return Some(LengthValue::MinContent),
        "max-content" => return Some(LengthValue::MaxContent),
        "auto" => return None,
        _ => {}
    }
    if let Some(inner) = value
        .strip_prefix("fit-content(")
        .and_then(|s| s.strip_suffix(')'))
    {
        if let Some(px) = parse_length(inner, parent_font_size) {
            return Some(LengthValue::FitContent(px));
        }
    }
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

/// Extract a URL from a CSS `url(...)` token.
fn extract_url(value: &str) -> Option<String> {
    let v = value.trim();
    let inner = v.strip_prefix("url(")?.strip_suffix(')')?;
    let inner = inner.trim();
    // Strip optional surrounding quotes
    let url = if (inner.starts_with('"') && inner.ends_with('"'))
        || (inner.starts_with('\'') && inner.ends_with('\''))
    {
        &inner[1..inner.len() - 1]
    } else {
        inner
    };
    if url.is_empty() {
        None
    } else {
        Some(url.to_string())
    }
}

/// Parse a `clip-path` value: `circle(...)`, `inset(...)`, `polygon(...)`.
/// All lengths are interpreted as percentages or px and stored as permille (0–1000)
/// of the element's reference dimension.
fn parse_clip_path(value: &str) -> Option<ClipPath> {
    let v = value.trim().to_ascii_lowercase();
    let inner = |fn_name: &str, src: &str| -> Option<String> {
        let start = src.find(fn_name)?;
        let after = &src[start + fn_name.len()..];
        let open = after.find('(')?;
        let close = after.rfind(')')?;
        if close <= open {
            return None;
        }
        Some(after[open + 1..close].to_string())
    };

    let parse_percent_to_permille = |s: &str| -> u32 {
        let t = s.trim();
        if let Some(stripped) = t.strip_suffix('%') {
            stripped.trim().parse::<f32>().ok().map(|p| (p * 10.0).round().clamp(0.0, 1000.0) as u32)
                .unwrap_or(0)
        } else if let Some(stripped) = t.strip_suffix("px") {
            // Treat px as an absolute fallback by hashing to 0–1000 via 1px=10permille (loose).
            stripped.trim().parse::<f32>().ok().map(|p| (p * 10.0).clamp(0.0, 1000.0) as u32).unwrap_or(0)
        } else {
            t.parse::<f32>().ok().map(|p| (p * 10.0).clamp(0.0, 1000.0) as u32).unwrap_or(0)
        }
    };

    if v.starts_with("circle(") {
        let body = inner("circle", &v)?;
        // syntax: `circle(R [at X Y])` or `circle()` (default = 50%)
        let lower = body.to_ascii_lowercase();
        let (r_part, center_part) = match lower.split_once(" at ") {
            Some((r, c)) => (r.to_string(), Some(c.to_string())),
            None => (lower.clone(), None),
        };
        let radius = if r_part.trim().is_empty() {
            500
        } else {
            parse_percent_to_permille(r_part.trim())
        };
        let (cx, cy) = if let Some(c) = center_part {
            let parts: Vec<&str> = c.split_whitespace().collect();
            let cx = parts.first().map(|s| parse_percent_to_permille(s)).unwrap_or(500);
            let cy = parts.get(1).map(|s| parse_percent_to_permille(s)).unwrap_or(500);
            (cx, cy)
        } else {
            (500, 500)
        };
        return Some(ClipPath::Circle { radius_permille: radius, cx_permille: cx, cy_permille: cy });
    }
    if v.starts_with("inset(") {
        let body = inner("inset", &v)?;
        let parts: Vec<&str> = body.split_whitespace().collect();
        let (t, r, b, l) = match parts.len() {
            1 => {
                let n = parse_percent_to_permille(parts[0]);
                (n, n, n, n)
            }
            2 => {
                let tb = parse_percent_to_permille(parts[0]);
                let lr = parse_percent_to_permille(parts[1]);
                (tb, lr, tb, lr)
            }
            3 => {
                let t = parse_percent_to_permille(parts[0]);
                let lr = parse_percent_to_permille(parts[1]);
                let b = parse_percent_to_permille(parts[2]);
                (t, lr, b, lr)
            }
            _ => {
                let t = parse_percent_to_permille(parts[0]);
                let r = parse_percent_to_permille(parts[1]);
                let b = parse_percent_to_permille(parts[2]);
                let l = parse_percent_to_permille(parts[3]);
                (t, r, b, l)
            }
        };
        return Some(ClipPath::Inset { top: t, right: r, bottom: b, left: l });
    }
    if v.starts_with("polygon(") {
        let body = inner("polygon", &v)?;
        let mut points = Vec::new();
        for pair in body.split(',') {
            let parts: Vec<&str> = pair.trim().split_whitespace().collect();
            if parts.len() >= 2 {
                let x = parse_percent_to_permille(parts[0]);
                let y = parse_percent_to_permille(parts[1]);
                points.push((x, y));
            }
        }
        if points.len() >= 3 {
            return Some(ClipPath::Polygon { points });
        }
    }
    None
}

/// Parse a `text-shadow` value. Format: offset-x offset-y [blur] color.
fn parse_text_shadow(value: &str, parent_font_size: u32) -> Option<TextShadow> {
    // Take only the first shadow (before any comma outside parens)
    let first_shadow = split_at_top_level(value, ',').into_iter().next()?;
    let tokens: Vec<String> = split_at_top_level(first_shadow.trim(), ' ')
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    if tokens.is_empty() {
        return None;
    }

    let mut lengths: Vec<i32> = Vec::new();
    let mut color: u32 = 0x000000;
    let mut found_color = false;

    for token in &tokens {
        if let Some(c) = parse_color(token) {
            color = c;
            found_color = true;
        } else if let Some(px) = parse_signed_length(token, parent_font_size) {
            lengths.push(px);
        }
    }

    if !found_color {
        // default shadow color is black
        color = 0x000000;
    }

    match lengths.as_slice() {
        [ox, oy] => Some(TextShadow {
            offset_x: *ox,
            offset_y: *oy,
            blur: 0,
            color,
        }),
        [ox, oy, blur, ..] => Some(TextShadow {
            offset_x: *ox,
            offset_y: *oy,
            blur: (*blur).max(0) as u32,
            color,
        }),
        _ => None,
    }
}

/// Parse the interior of a `@keyframes` block into a list of `KeyframeStop`s.
fn parse_keyframe_stops(block: &str) -> Vec<KeyframeStop> {
    let mut stops = Vec::new();
    // Find each "selector { declarations }" pair inside the block
    let mut rest = block.trim();
    while !rest.is_empty() {
        // Find the selector part (everything before '{')
        let brace = match rest.find('{') {
            Some(i) => i,
            None => break,
        };
        let sel = rest[..brace].trim();
        rest = &rest[brace + 1..];
        // Find matching closing brace
        let close = match rest.find('}') {
            Some(i) => i,
            None => break,
        };
        let decls_str = rest[..close].trim();
        rest = rest[close + 1..].trim();

        // Parse position(s): "from", "to", "0%", "50%", "0%, 100%"
        let positions: Vec<u32> = sel.split(',').filter_map(|s| {
            let s = s.trim();
            if s == "from" || s == "0%" { Some(0) }
            else if s == "to" || s == "100%" { Some(1000) }
            else if let Some(pct) = s.strip_suffix('%') {
                pct.trim().parse::<f32>().ok().map(|v| (v * 10.0) as u32)
            } else { None }
        }).collect();

        // Parse declarations as raw (property, value) pairs
        let declarations: Vec<(String, String)> = decls_str.split(';')
            .filter_map(|d| {
                let d = d.trim();
                if d.is_empty() { return None; }
                let colon = d.find(':')?;
                Some((d[..colon].trim().to_string(), d[colon+1..].trim().to_string()))
            })
            .collect();

        for pos in positions {
            stops.push(KeyframeStop { position: pos, declarations: declarations.clone() });
        }
    }
    stops
}

/// Parse a `linear-gradient(...)` value.
fn parse_linear_gradient(value: &str) -> Option<CssGradient> {
    // Find the linear-gradient(...) part
    let lower = value.to_ascii_lowercase();
    let start = lower.find("linear-gradient(")?;
    let after = &value[start + "linear-gradient(".len()..];
    // Find matching closing paren
    let mut depth = 1u32;
    let mut end = 0;
    for (i, ch) in after.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    end = i;
                    break;
                }
            }
            _ => {}
        }
    }
    let inner = &after[..end];

    // Split by top-level commas
    let args: Vec<String> = split_at_top_level(inner, ',');

    if args.is_empty() {
        return None;
    }

    let mut arg_iter = args.iter().peekable();

    // Determine angle from first arg
    let first_arg = arg_iter.peek()?.trim().to_ascii_lowercase();
    let angle_deg_x1000: i32;

    if first_arg.starts_with("to ") {
        let dir = first_arg[3..].trim();
        angle_deg_x1000 = match dir {
            "right" => 90_000,
            "left" => 270_000,
            "bottom" => 180_000,
            "top" => 0,
            "bottom right" | "right bottom" => 135_000,
            "bottom left" | "left bottom" => 225_000,
            "top right" | "right top" => 45_000,
            "top left" | "left top" => 315_000,
            _ => 180_000,
        };
        arg_iter.next(); // consume the direction arg
    } else if let Some(deg_str) = first_arg.strip_suffix("deg") {
        let deg: f64 = deg_str.trim().parse().unwrap_or(180.0);
        angle_deg_x1000 = (deg * 1000.0).round() as i32;
        arg_iter.next();
    } else if first_arg.starts_with("to")
        || first_arg.ends_with("deg")
        || first_arg.ends_with("turn")
        || first_arg.ends_with("rad")
        || first_arg.ends_with("grad")
    {
        // Other angle formats — skip and use 180
        angle_deg_x1000 = 180_000;
        arg_iter.next();
    } else {
        // No explicit angle, default to bottom (180deg)
        angle_deg_x1000 = 180_000;
    }

    // Parse color stops
    let mut raw_stops: Vec<(u32, Option<u32>)> = Vec::new();
    for arg in arg_iter {
        let arg_trimmed = arg.trim();
        // A color stop is "color [position%]"
        // Split by whitespace but be careful with rgb()/rgba()
        let parts: Vec<String> = split_at_top_level(arg_trimmed, ' ')
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        if parts.is_empty() {
            continue;
        }

        // Try to find which part is the color
        // Color could be a keyword, #hex, rgb(...), etc.
        // It's usually the first token but could be combined with a function
        // Reassemble function calls that were split
        let mut color_str = String::new();
        let mut pos_str: Option<String> = None;

        // Attempt: first join parts that belong to a function (rgb/rgba/hsl)
        let joined = parts.join(" ");
        // Try the whole joined string as color first, or look for position at end
        // Position is a numeric token ending with % or px
        let last = parts.last().unwrap();
        let second_last = if parts.len() >= 2 {
            Some(&parts[parts.len() - 2])
        } else {
            None
        };

        let last_is_position =
            last.ends_with('%') || (last.ends_with("px") && parse_length(last, 16).is_some());
        let second_last_is_position = second_last
            .map(|s| s.ends_with('%') || s.ends_with("px"))
            .unwrap_or(false);

        if last_is_position && parts.len() >= 2 {
            pos_str = Some(last.clone());
            color_str = parts[..parts.len() - 1].join(" ");
        } else if second_last_is_position && parts.len() >= 3 {
            pos_str = Some(second_last.unwrap().clone());
            color_str = parts[..parts.len() - 2].join(" ");
        } else {
            color_str = joined.clone();
        }

        if let Some(c) = parse_color(color_str.trim()) {
            let pos = pos_str.and_then(|p| {
                let p = p.trim();
                if p.ends_with('%') {
                    p[..p.len() - 1]
                        .parse::<f64>()
                        .ok()
                        .map(|v| (v * 10.0).round() as u32)
                } else {
                    parse_length(p, 16).map(|v| (v as f64 / 10.0).round() as u32) // rough conversion
                }
            });
            raw_stops.push((c, pos));
        }
    }

    if raw_stops.is_empty() {
        return None;
    }

    // Fill in missing positions by distributing evenly
    let count = raw_stops.len();
    let stops: Vec<(u32, u32)> = raw_stops
        .into_iter()
        .enumerate()
        .map(|(i, (c, p))| {
            let pos = p.unwrap_or_else(|| {
                if count == 1 {
                    0
                } else {
                    (1000 * i / (count - 1)) as u32
                }
            });
            (c, pos)
        })
        .collect();

    Some(CssGradient { kind: GradientKind::Linear { angle_deg_x1000 }, stops })
}

/// Parse a `radial-gradient(...)` value → CssGradient with GradientKind::Radial.
fn parse_radial_gradient(value: &str) -> Option<CssGradient> {
    let lower = value.to_ascii_lowercase();
    let start = lower.find("radial-gradient(")?;
    let after = &value[start + "radial-gradient(".len()..];
    let mut depth = 1u32;
    let mut end = 0;
    for (i, ch) in after.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => { depth -= 1; if depth == 0 { end = i; break; } }
            _ => {}
        }
    }
    let inner = &after[..end];

    // Split args respecting parens
    let args: Vec<String> = split_at_top_level(inner, ',');

    // Parse optional shape/center ("circle at 30% 70%", "ellipse at center")
    let mut stop_start = 0usize;
    let mut center_x = 500u32;
    let mut center_y = 500u32;

    if let Some(first) = args.first() {
        let fl = first.to_ascii_lowercase();
        if fl.contains("circle") || fl.contains("ellipse") || fl.contains(" at ") || fl.trim().starts_with("at ") {
            stop_start = 1;
            if let Some(at_pos) = fl.find(" at ") {
                let pos_str = &first[at_pos + 4..];
                let parts: Vec<&str> = pos_str.split_whitespace().collect();
                if let Some(x) = parts.first() { center_x = parse_pct_permille(x, 500); }
                if let Some(y) = parts.get(1) { center_y = parse_pct_permille(y, 500); }
            }
        } else {
            // Heuristic: if it doesn't look like a color stop, treat as shape spec
            let trimmed = fl.trim();
            if !trimmed.starts_with('#') && !trimmed.starts_with("rgb") && trimmed.parse::<f32>().is_err() {
                // Check if it's a known color keyword
                let looks_like_color = parse_color(trimmed).is_some();
                if !looks_like_color {
                    stop_start = 1;
                }
            }
        }
    }

    // Parse color stops
    let stop_args = &args[stop_start..];
    if stop_args.is_empty() { return None; }

    let mut raw_stops: Vec<(u32, Option<u32>)> = Vec::new();
    for arg in stop_args {
        let arg_trimmed = arg.trim();
        let parts: Vec<String> = split_at_top_level(arg_trimmed, ' ')
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if parts.is_empty() { continue; }

        let last = parts.last().unwrap();
        let last_is_position = last.ends_with('%') || last.ends_with("px");
        let (color_str, pos_str) = if last_is_position && parts.len() >= 2 {
            let c = parts[..parts.len() - 1].join(" ");
            (c, Some(last.clone()))
        } else {
            (parts.join(" "), None)
        };

        if let Some(c) = parse_color(color_str.trim()) {
            let pos = pos_str.and_then(|p| {
                let p = p.trim();
                if p.ends_with('%') {
                    p[..p.len()-1].parse::<f64>().ok().map(|v| (v * 10.0).round() as u32)
                } else {
                    None
                }
            });
            raw_stops.push((c, pos));
        }
    }

    if raw_stops.is_empty() { return None; }

    let count = raw_stops.len();
    let stops: Vec<(u32, u32)> = raw_stops.into_iter().enumerate().map(|(i, (c, p))| {
        let pos = p.unwrap_or_else(|| {
            if count == 1 { 0 } else { (1000 * i / (count - 1)) as u32 }
        });
        (c, pos)
    }).collect();

    Some(CssGradient { kind: GradientKind::Radial { center_x, center_y }, stops })
}

/// Accumulate CSS transform functions into ComputedStyle fields.
fn parse_transform_into(value: &str, style: &mut ComputedStyle) {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") { return; }
    let mut rest = v;
    while let Some(paren) = rest.find('(') {
        let func = rest[..paren].trim().to_ascii_lowercase();
        let after = &rest[paren + 1..];
        let close = after.find(')').unwrap_or(after.len());
        let args = &after[..close];
        rest = if close + 1 < after.len() { &after[close + 1..] } else { "" };

        match func.as_str() {
            "translate" => {
                let parts: Vec<&str> = args.split(',').collect();
                style.transform_translate_x = parse_transform_length(parts.first().unwrap_or(&"0"));
                style.transform_translate_y = parse_transform_length(parts.get(1).unwrap_or(&"0"));
            }
            "translatex" => {
                style.transform_translate_x = parse_transform_length(args);
            }
            "translatey" => {
                style.transform_translate_y = parse_transform_length(args);
            }
            "scale" => {
                let parts: Vec<&str> = args.split(',').collect();
                let sx = parse_transform_scale(parts.first().unwrap_or(&"1"));
                let sy = parts.get(1).map(|s| parse_transform_scale(s)).unwrap_or(sx);
                style.transform_scale_x = sx;
                style.transform_scale_y = sy;
            }
            "scalex" => { style.transform_scale_x = parse_transform_scale(args); }
            "scaley" => { style.transform_scale_y = parse_transform_scale(args); }
            "rotate" => {
                style.transform_rotate_millideg = parse_transform_angle_millideg(args);
            }
            _ => {} // skew, matrix, etc. — no-op
        }
    }
}

fn parse_transform_length(s: &str) -> i32 {
    let s = s.trim();
    if s.ends_with("px") {
        s[..s.len() - 2].trim().parse::<f32>().unwrap_or(0.0) as i32
    } else if s.ends_with('%') {
        0 // percent translate not supported in fixed-pixel layout
    } else {
        s.parse::<f32>().unwrap_or(0.0) as i32
    }
}

fn parse_transform_scale(s: &str) -> u32 {
    let v = s.trim().parse::<f32>().unwrap_or(1.0);
    (v * 1000.0).round() as u32
}

fn parse_transform_angle_millideg(s: &str) -> i32 {
    let s = s.trim();
    if s.ends_with("deg") {
        let v = s[..s.len() - 3].trim().parse::<f32>().unwrap_or(0.0);
        (v * 1000.0).round() as i32
    } else if s.ends_with("rad") {
        let v = s[..s.len() - 3].trim().parse::<f32>().unwrap_or(0.0);
        (v.to_degrees() * 1000.0).round() as i32
    } else if s.ends_with("turn") {
        let v = s[..s.len() - 4].trim().parse::<f32>().unwrap_or(0.0);
        (v * 360.0 * 1000.0).round() as i32
    } else {
        s.parse::<f32>().unwrap_or(0.0) as i32
    }
}

fn parse_pct_permille(s: &str, default: u32) -> u32 {
    let s = s.trim();
    match s {
        "center" => 500,
        "left" | "top" => 0,
        "right" | "bottom" => 1000,
        _ => {
            if s.ends_with('%') {
                let v = s[..s.len() - 1].trim().parse::<f32>().unwrap_or(50.0);
                (v * 10.0).round() as u32
            } else {
                default
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::{
        AlignItems, AlignSelf, Display, FlexDirection, FlexWrap, JustifyContent, LengthValue,
        Position, StyledElement, StyledNode, VerticalAlign, WhiteSpaceMode, build_styled_tree,
        compute_style, parse_color, parse_length, parse_stylesheet, split_at_top_level,
    };
    use crate::html::{Element, Node, parse_document};

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

        let styled = build_styled_tree(
            &document,
            &stylesheet,
            1280,
            &super::InteractiveState::default(),
        );
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
        let styled = build_styled_tree(
            &document,
            &stylesheet,
            1280,
            &super::InteractiveState::default(),
        );

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
        let styled = build_styled_tree(
            &document,
            &stylesheet,
            1280,
            &super::InteractiveState::default(),
        );

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
        let styled = build_styled_tree(
            &document,
            &stylesheet,
            1280,
            &super::InteractiveState::default(),
        );

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
        let styled = build_styled_tree(
            &document,
            &stylesheet,
            1280,
            &super::InteractiveState::default(),
        );

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
        let styled = build_styled_tree(
            &document,
            &stylesheet,
            1280,
            &super::InteractiveState::default(),
        );

        let target = find_element_by_id(&styled, "target").expect("target paragraph should exist");
        assert_eq!(target.style.color, 0xFF0000);
    }

    #[test]
    fn supports_general_sibling_then_child_combinator() {
        let document =
            parse_document("<body><h1></h1><p></p><div><span id=\"target\"></span></div></body>");
        let stylesheet = parse_stylesheet("h1 ~ div > span { color: #00ff00; }");
        let styled = build_styled_tree(
            &document,
            &stylesheet,
            1280,
            &super::InteractiveState::default(),
        );

        let target = find_element_by_id(&styled, "target").expect("target span should exist");
        assert_eq!(target.style.color, 0x00FF00);
    }

    #[test]
    fn applies_legacy_html_attributes() {
        let document = parse_document(
            "<body bgcolor=\"#f0f0ff\"><h1 align=\"center\">Title</h1><font color=\"#ff0000\">red</font></body>",
        );
        let styled = build_styled_tree(
            &document,
            &super::Stylesheet::default(),
            1280,
            &super::InteractiveState::default(),
        );

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
        let styled = build_styled_tree(
            &document,
            &stylesheet,
            1280,
            &super::InteractiveState::default(),
        );
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
        let styled = build_styled_tree(
            &document,
            &stylesheet,
            1280,
            &super::InteractiveState::default(),
        );
        let a = find_first_element(&styled, "a").expect("a should exist");
        let span = find_first_element(&styled, "span").expect("span should exist");
        assert_eq!(a.style.color, 0xFF0000);
        assert_ne!(span.style.color, 0xFF0000);
    }

    #[test]
    fn attribute_equals_selector_matches() {
        let document = parse_document("<input type=\"text\"><input type=\"checkbox\">");
        let stylesheet = parse_stylesheet("[type=text] { color: #00ff00; }");
        let styled = build_styled_tree(
            &document,
            &stylesheet,
            1280,
            &super::InteractiveState::default(),
        );
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
        let styled = build_styled_tree(
            &document,
            &stylesheet,
            1280,
            &super::InteractiveState::default(),
        );
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
        let styled = build_styled_tree(
            &document,
            &stylesheet,
            1280,
            &super::InteractiveState::default(),
        );
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
        let styled = build_styled_tree(
            &document,
            &stylesheet,
            1280,
            &super::InteractiveState::default(),
        );
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
        let styled = build_styled_tree(
            &document,
            &stylesheet,
            1280,
            &super::InteractiveState::default(),
        );
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
        let styled = build_styled_tree(
            &document,
            &stylesheet,
            1280,
            &super::InteractiveState::default(),
        );
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
        let styled = build_styled_tree(
            &document,
            &stylesheet,
            1280,
            &super::InteractiveState::default(),
        );
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
        assert_ne!(
            colors[0], 0x00FF00,
            ".skip li should not match selector list in :not()"
        );
        assert_ne!(
            colors[1], 0x00FF00,
            ".omit li should not match selector list in :not()"
        );
        assert_eq!(
            colors[2], 0x00FF00,
            "plain li should match selector list in :not()"
        );
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
        let styled_wide = build_styled_tree(
            &document,
            &stylesheet,
            1280,
            &super::InteractiveState::default(),
        );
        let p_wide = find_first_element(&styled_wide, "p").unwrap();
        assert_eq!(
            p_wide.style.color, 0x0000FF,
            "wide viewport: plain rule wins"
        );

        // Viewport 400 → max-width 600 rule SHOULD apply and wins (later in source)
        let styled_narrow = build_styled_tree(
            &document,
            &stylesheet,
            400,
            &super::InteractiveState::default(),
        );
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
        let styled = build_styled_tree(
            &document,
            &stylesheet,
            1280,
            &super::InteractiveState::default(),
        );
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
        let styled = build_styled_tree(
            &document,
            &stylesheet,
            1280,
            &super::InteractiveState::default(),
        );
        let p = find_first_element(&styled, "p").unwrap();
        assert_eq!(p.style.font_size_px, 16);
    }

    #[test]
    fn calc_multiplication_has_higher_precedence_than_addition() {
        // calc(2px + 3 * 4px) should be 2 + 12 = 14, NOT (2+3)*4 = 20
        let document = parse_document("<p>text</p>");
        let stylesheet = parse_stylesheet("p { font-size: calc(2px + 3 * 4px); }");
        let styled = build_styled_tree(
            &document,
            &stylesheet,
            1280,
            &super::InteractiveState::default(),
        );
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
        let styled = build_styled_tree(
            &document,
            &stylesheet,
            1280,
            &super::InteractiveState::default(),
        );
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
        let styled = build_styled_tree(
            &document,
            &stylesheet,
            1280,
            &super::InteractiveState::default(),
        );
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
        let styled = build_styled_tree(
            &document,
            &stylesheet,
            1280,
            &super::InteractiveState::default(),
        );
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
        let styled = build_styled_tree(&doc, &stylesheet, 800, &super::InteractiveState::default());

        fn find_p(node: &StyledNode) -> Option<&StyledElement> {
            match node {
                StyledNode::Element(el) if el.tag_name == "p" => Some(el),
                StyledNode::Element(el) => el.children.iter().find_map(find_p),
                _ => None,
            }
        }

        let p_el = find_p(&styled).expect("Should find <p> element");
        assert_eq!(
            p_el.style.color, 0xff0000,
            "p color should be #ff0000 from :root var"
        );
    }
    #[test]
    fn test_before_pseudo_element_content_injection() {
        use crate::html::parse_document;

        let css = r#"p::before { content: "-> "; }"#;
        let html = r#"<p>Hello</p>"#;
        let doc = parse_document(html);
        let stylesheet = parse_stylesheet(css);
        let styled = build_styled_tree(&doc, &stylesheet, 800, &super::InteractiveState::default());

        fn find_p(node: &StyledNode) -> Option<&StyledElement> {
            match node {
                StyledNode::Element(el) if el.tag_name == "p" => Some(el),
                StyledNode::Element(el) => el.children.iter().find_map(find_p),
                _ => None,
            }
        }

        let p_el = find_p(&styled).expect("Should find <p> element");
        assert!(!p_el.children.is_empty(), "p should have children");
        if let StyledNode::Text(first) = &p_el.children[0] {
            assert_eq!(first.text, "-> ", "First child should be ::before content");
        } else {
            panic!("First child should be a text node from ::before");
        }
    }

    #[test]
    fn test_position_relative_parsed() {
        let ss = parse_stylesheet("div { position: relative; top: 10px; left: 20px; }");
        let el = Element {
            tag_name: "div".into(),
            attributes: Default::default(),
            children: vec![],
        };
        let style = compute_style(
            &el,
            &ss,
            None,
            &[],
            0,
            1,
            &[],
            1280,
            &super::InteractiveState::default(),
        );
        assert_eq!(style.position, Position::Relative);
        assert_eq!(style.top, Some(10));
        assert_eq!(style.left, Some(20));
    }

    #[test]
    fn test_position_absolute_parsed() {
        let ss = parse_stylesheet("div { position: absolute; top: 0px; }");
        let el = Element {
            tag_name: "div".into(),
            attributes: Default::default(),
            children: vec![],
        };
        let style = compute_style(
            &el,
            &ss,
            None,
            &[],
            0,
            1,
            &[],
            1280,
            &super::InteractiveState::default(),
        );
        assert_eq!(style.position, Position::Absolute);
    }

    #[test]
    fn test_flex_display_parsed() {
        let ss = parse_stylesheet("div { display: flex; flex-direction: column; gap: 8px; }");
        let el = Element {
            tag_name: "div".into(),
            attributes: Default::default(),
            children: vec![],
        };
        let style = compute_style(
            &el,
            &ss,
            None,
            &[],
            0,
            1,
            &[],
            1280,
            &super::InteractiveState::default(),
        );
        assert_eq!(style.display, Display::Flex);
        assert_eq!(style.flex_direction, FlexDirection::Column);
        assert_eq!(style.gap, 8);
    }

    #[test]
    fn test_justify_content_parsed() {
        let ss = parse_stylesheet(
            "div { display: flex; justify-content: space-between; align-items: center; }",
        );
        let el = Element {
            tag_name: "div".into(),
            attributes: Default::default(),
            children: vec![],
        };
        let style = compute_style(
            &el,
            &ss,
            None,
            &[],
            0,
            1,
            &[],
            1280,
            &super::InteractiveState::default(),
        );
        assert_eq!(style.justify_content, JustifyContent::SpaceBetween);
        assert_eq!(style.align_items, AlignItems::Center);
    }

    #[test]
    fn test_z_index_parsed() {
        let ss = parse_stylesheet("div { position: absolute; z-index: 10; }");
        let el = Element {
            tag_name: "div".into(),
            attributes: Default::default(),
            children: vec![],
        };
        let style = compute_style(
            &el,
            &ss,
            None,
            &[],
            0,
            1,
            &[],
            1280,
            &super::InteractiveState::default(),
        );
        assert_eq!(style.z_index, Some(10));
    }

    // ── Phase 5: clamp / min / max ────────────────────────────────────────────

    #[test]
    fn clamp_resolves_clamped_value() {
        // clamp(10px, 50px, 100px) = 50px
        assert_eq!(parse_length("clamp(10px, 50px, 100px)", 16), Some(50));
        // clamp(10px, 5px, 100px) = 10px (below min)
        assert_eq!(parse_length("clamp(10px, 5px, 100px)", 16), Some(10));
        // clamp(10px, 200px, 100px) = 100px (above max)
        assert_eq!(parse_length("clamp(10px, 200px, 100px)", 16), Some(100));
    }

    #[test]
    fn min_max_resolve() {
        assert_eq!(parse_length("min(30px, 50px)", 16), Some(30));
        assert_eq!(parse_length("max(30px, 50px)", 16), Some(50));
        assert_eq!(parse_length("min(100px, 80px, 60px)", 16), Some(60));
    }

    #[test]
    fn aspect_ratio_parsed() {
        let html = r#"<div style="aspect-ratio: 16/9; width: 160px;"></div>"#;
        let doc = parse_document(html);
        let sheet = parse_stylesheet("");
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let div = find_first_element(&styled, "div").unwrap();
        // 16/9 * 1000 = 1778
        assert_eq!(div.style.aspect_ratio, Some(1778));
    }

    #[test]
    fn hover_pseudo_class_applies_when_node_hovered() {
        // Assign a node_id via the data-tobira-node-id attribute (same mechanism used at runtime).
        // The <a> element gets node_id 42 here so the test is independent of DFS order.
        let html = r##"<a href="#" id="link" data-tobira-node-id="42">text</a>"##;
        let css = r#"a:hover { color: #ff0000; }"#;
        let doc = parse_document(html);
        let sheet = parse_stylesheet(css);

        // Without hover: link color should be the default link color (not red)
        let styled_no_hover =
            build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let a_no_hover = find_first_element(&styled_no_hover, "a").expect("<a> should exist");
        assert_ne!(
            a_no_hover.style.color, 0xFF0000,
            "color should not be red without hover"
        );

        // With hover on node 42: link color should become red
        let interactive = super::InteractiveState {
            hovered_node_id: Some(42),
            ..Default::default()
        };
        let styled_hovered = build_styled_tree(&doc, &sheet, 1280, &interactive);
        let a_hovered = find_first_element(&styled_hovered, "a").expect("<a> should exist");
        assert_eq!(
            a_hovered.style.color, 0xFF0000,
            "color should be red when hovered"
        );
    }

    #[test]
    fn flex_flow_sets_direction_and_wrap() {
        let html = r#"<div style="flex-flow: column wrap;"></div>"#;
        let doc = parse_document(html);
        let sheet = parse_stylesheet("");
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let div = find_first_element(&styled, "div").unwrap();
        assert_eq!(div.style.flex_direction, FlexDirection::Column);
        assert_eq!(div.style.flex_wrap, FlexWrap::Wrap);
    }

    #[test]
    fn checked_pseudo_class_matches_checked_input() {
        let html = r#"<input type="checkbox" checked>"#;
        let css = "input:checked { color: #ff0000; }";
        let doc = parse_document(html);
        let sheet = parse_stylesheet(css);
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let input = find_first_element(&styled, "input").unwrap();
        assert_eq!(input.style.color, 0xff0000);
    }

    #[test]
    fn grid_template_columns_parsed() {
        use super::GridTrackSize;
        let html = r#"<div style="display:grid;grid-template-columns:100px 1fr 200px;"></div>"#;
        let doc = parse_document(html);
        let sheet = parse_stylesheet("");
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let div = find_first_element(&styled, "div").unwrap();
        assert_eq!(div.style.display, Display::Grid);
        assert_eq!(div.style.grid_template_columns.len(), 3);
        assert_eq!(
            div.style.grid_template_columns[0],
            GridTrackSize::Pixels(100)
        );
        assert_eq!(div.style.grid_template_columns[1], GridTrackSize::Fr(1000));
        assert_eq!(
            div.style.grid_template_columns[2],
            GridTrackSize::Pixels(200)
        );
    }

    #[test]
    fn grid_repeat_expands_tracks() {
        let html = r#"<div style="display:grid;grid-template-columns:repeat(3,1fr);"></div>"#;
        let doc = parse_document(html);
        let sheet = parse_stylesheet("");
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let div = find_first_element(&styled, "div").unwrap();
        assert_eq!(div.style.display, Display::Grid);
        assert_eq!(div.style.grid_template_columns.len(), 3);
    }

    #[test]
    fn grid_inline_grid_display_parsed() {
        let html = r#"<div style="display:inline-grid;"></div>"#;
        let doc = parse_document(html);
        let sheet = parse_stylesheet("");
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let div = find_first_element(&styled, "div").unwrap();
        assert_eq!(div.style.display, Display::InlineGrid);
    }

    #[test]
    fn grid_placement_parsed() {
        let html = r#"<div style="grid-column:1/3;grid-row:2;"></div>"#;
        let doc = parse_document(html);
        let sheet = parse_stylesheet("");
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let div = find_first_element(&styled, "div").unwrap();
        assert_eq!(div.style.grid_column.start, Some(1));
        assert_eq!(div.style.grid_column.span, Some(2));
        assert_eq!(div.style.grid_row.start, Some(2));
    }

    #[test]
    fn min_max_content_length_value_parsed() {
        let html = r#"<div style="width: min-content;"></div>"#;
        let doc = parse_document(html);
        let sheet = parse_stylesheet("");
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let div = find_first_element(&styled, "div").unwrap();
        assert_eq!(div.style.width, Some(LengthValue::MinContent));
    }

    #[test]
    fn fit_content_length_value_parsed() {
        let html = r#"<div style="width: fit-content(300px);"></div>"#;
        let doc = parse_document(html);
        let sheet = parse_stylesheet("");
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let div = find_first_element(&styled, "div").unwrap();
        assert_eq!(div.style.width, Some(LengthValue::FitContent(300)));
    }

    #[test]
    fn pointer_events_none_parsed() {
        let html = r#"<div style="pointer-events: none;"></div>"#;
        let doc = parse_document(html);
        let sheet = parse_stylesheet("");
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let div = find_first_element(&styled, "div").unwrap();
        assert!(div.style.pointer_events_none);
    }

    #[test]
    fn filter_blur_parsed() {
        let html = r#"<div style="filter: blur(4px);"></div>"#;
        let doc = parse_document(html);
        let sheet = parse_stylesheet("");
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let div = find_first_element(&styled, "div").unwrap();
        assert_eq!(div.style.filter_blur_px, 4);
    }

    #[test]
    fn filter_brightness_parsed() {
        let html = r#"<div style="filter: brightness(0.5);"></div>"#;
        let doc = parse_document(html);
        let sheet = parse_stylesheet("");
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let div = find_first_element(&styled, "div").unwrap();
        assert_eq!(div.style.filter_brightness, 5000); // 0.5 * 10000
    }

    #[test]
    fn filter_opacity_parsed() {
        let html = r#"<div style="filter: opacity(0.5);"></div>"#;
        let doc = parse_document(html);
        let sheet = parse_stylesheet("");
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let div = find_first_element(&styled, "div").unwrap();
        assert_eq!(div.style.filter_opacity, 128); // round(0.5 * 255) = 128
    }

    #[test]
    fn filter_multiple_functions_parsed() {
        let html = r#"<div style="filter: blur(2px) brightness(0.8);"></div>"#;
        let doc = parse_document(html);
        let sheet = parse_stylesheet("");
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let div = find_first_element(&styled, "div").unwrap();
        assert_eq!(div.style.filter_blur_px, 2);
        assert_eq!(div.style.filter_brightness, 8000);
    }

    #[test]
    fn at_supports_rules_applied() {
        // @supports is treated as always-true so inner rules should apply
        let html = r#"<div class="box"></div>"#;
        let css = r#"@supports (display: grid) { .box { color: #ff0000; } }"#;
        let doc = parse_document(html);
        let sheet = parse_stylesheet(css);
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let div = find_first_element(&styled, "div").unwrap();
        assert_eq!(div.style.color, 0xff0000);
    }

    #[test]
    fn at_layer_rules_applied() {
        // @layer contents are treated as regular rules
        let html = r#"<div class="box"></div>"#;
        let css = r#"@layer base { .box { color: #00ff00; } }"#;
        let doc = parse_document(html);
        let sheet = parse_stylesheet(css);
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let div = find_first_element(&styled, "div").unwrap();
        assert_eq!(div.style.color, 0x00ff00);
    }

    #[test]
    fn placeholder_pseudo_element_parsed() {
        // ::placeholder rules should be parsed without errors
        let css = r#"input::placeholder { color: #999999; }"#;
        let sheet = parse_stylesheet(css);
        // Should have one rule with Placeholder pseudo-element
        assert!(!sheet.rules.is_empty());
        let rule = &sheet.rules[0];
        assert!(
            rule.selectors
                .iter()
                .any(|s| s.pseudo_element == Some(super::PseudoElement::Placeholder))
        );
    }

    #[test]
    fn no_op_properties_do_not_panic() {
        // These properties should be silently accepted without panicking
        let html = r#"<div style="scroll-behavior: smooth; will-change: transform; user-select: none; writing-mode: horizontal-tb; touch-action: pan-y;"></div>"#;
        let doc = parse_document(html);
        let sheet = parse_stylesheet("");
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let div = find_first_element(&styled, "div").unwrap();
        // Just check it doesn't panic and the element is accessible
        assert_eq!(div.tag_name, "div");
    }
    #[test]
    fn margin_auto_sets_auto_flags() {
        // 5em at 16px = 80px; "auto" for horizontal means both auto flags set
        let html = r#"<div style="margin: 5em auto;"></div>"#;
        let doc = parse_document(html);
        let sheet = parse_stylesheet("");
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let div = find_first_element(&styled, "div").unwrap();
        assert_eq!(div.style.margin.top, 80, "5em at 16px base = 80px");
        assert_eq!(div.style.margin.bottom, 80, "5em at 16px base = 80px");
        assert_eq!(
            div.style.margin.left, 0,
            "auto resolves to 0 in parsed value"
        );
        assert_eq!(
            div.style.margin.right, 0,
            "auto resolves to 0 in parsed value"
        );
        assert!(div.style.margin_left_auto, "margin-left should be auto");
        assert!(div.style.margin_right_auto, "margin-right should be auto");
    }

    #[test]
    fn grid_template_areas_parsed() {
        let html = "<div style=\"display:grid;grid-template-areas:'header header' 'sidebar main' 'footer footer';\"></div>";
        let doc = parse_document(html);
        let sheet = parse_stylesheet("");
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let div = find_first_element(&styled, "div").unwrap();
        assert_eq!(div.style.grid_template_areas.len(), 3);
        assert_eq!(div.style.grid_template_areas[0], vec!["header", "header"]);
        assert_eq!(div.style.grid_template_areas[1], vec!["sidebar", "main"]);
        assert_eq!(div.style.grid_template_areas[2], vec!["footer", "footer"]);
    }

    #[test]
    fn grid_area_name_parsed() {
        let html = "<div style=\"grid-area: sidebar;\"></div>";
        let doc = parse_document(html);
        let sheet = parse_stylesheet("");
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let div = find_first_element(&styled, "div").unwrap();
        assert_eq!(div.style.grid_area_name, Some("sidebar".to_string()));
    }

    // ─── CSS counter tests ────────────────────────────────────────────────────

    #[test]
    fn counter_reset_parsed_with_value() {
        let css = "ol { counter-reset: item 0; }";
        let html = "<ol><li>test</li></ol>";
        let doc = parse_document(html);
        let sheet = parse_stylesheet(css);
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let ol = find_first_element(&styled, "ol").unwrap();
        assert_eq!(ol.style.counter_reset, vec![("item".to_string(), 0)]);
    }

    #[test]
    fn counter_reset_parsed_no_value() {
        // When no integer follows the name, default is 0
        let css = "ol { counter-reset: section; }";
        let html = "<ol></ol>";
        let doc = parse_document(html);
        let sheet = parse_stylesheet(css);
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let ol = find_first_element(&styled, "ol").unwrap();
        assert_eq!(ol.style.counter_reset, vec![("section".to_string(), 0)]);
    }

    #[test]
    fn counter_increment_parsed_default_step() {
        // counter-increment default step is 1
        let css = "li { counter-increment: item; }";
        let html = "<li>test</li>";
        let doc = parse_document(html);
        let sheet = parse_stylesheet(css);
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let li = find_first_element(&styled, "li").unwrap();
        assert_eq!(li.style.counter_increment, vec![("item".to_string(), 1)]);
    }

    #[test]
    fn counter_increment_parsed_with_step() {
        let css = "li { counter-increment: chapter 2; }";
        let html = "<li>test</li>";
        let doc = parse_document(html);
        let sheet = parse_stylesheet(css);
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let li = find_first_element(&styled, "li").unwrap();
        assert_eq!(li.style.counter_increment, vec![("chapter".to_string(), 2)]);
    }

    #[test]
    fn counter_reset_none_gives_empty_vec() {
        let css = "ol { counter-reset: none; }";
        let html = "<ol></ol>";
        let doc = parse_document(html);
        let sheet = parse_stylesheet(css);
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let ol = find_first_element(&styled, "ol").unwrap();
        assert_eq!(ol.style.counter_reset, vec![]);
    }

    #[test]
    fn resolve_content_counters_replaces_counter_ref() {
        let mut counters = std::collections::HashMap::new();
        counters.insert("item".to_string(), 3);
        let result = super::resolve_content_counters("counter(item). ", &counters);
        assert_eq!(result, "3. ");
    }

    #[test]
    fn resolve_content_counters_unknown_counter_gives_zero() {
        let counters = std::collections::HashMap::new();
        let result = super::resolve_content_counters("counter(missing)", &counters);
        assert_eq!(result, "0");
    }

    #[test]
    fn resolve_content_counters_no_counter_unchanged() {
        let counters = std::collections::HashMap::new();
        let result = super::resolve_content_counters("plain text", &counters);
        assert_eq!(result, "plain text");
    }
}
