use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::{Mutex, OnceLock};

use crate::html::{Element, Node};

mod color;
pub use color::parse_color;
mod media;
pub(crate) use media::{MediaCondition, parse_media_condition};

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
    /// CSS custom properties declared on `:root` or `html` outside any `@media` block.
    /// Shared via `Rc` so that cloning into each element's `css_variables` map is O(1).
    pub root_vars: Rc<BTreeMap<String, String>>,
    /// CSS custom properties declared on `:root` or `html` inside an `@media` block.
    /// Each entry is `(condition, vars)` and is only applied when the condition matches
    /// the current viewport width at style-computation time.
    pub media_root_vars: Vec<(MediaCondition, BTreeMap<String, String>)>,
    /// Selector index (id/class/tag/universal buckets) built once from `rules`,
    /// so style computation tests only candidate rules instead of every rule.
    /// Rebuilt by `extend` so it stays in sync with the rule set.
    rule_index: RuleIndex,
    /// Whether any rule in this sheet asks a `:has()` question. Gathering an
    /// element's children costs an allocation per element, and almost no page
    /// needs it, so the walk skips that work unless a rule will read it.
    uses_has: bool,
    /// Every cascade layer named by the document, in the order the layers were
    /// first declared. A layer declared later beats one declared earlier, and
    /// an unlayered rule beats them all, so this order is what the cascade
    /// sorts on -- not the order the rules happen to appear in.
    layer_order: Vec<Arc<str>>,
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
        // Layer order is a property of the document, not of one sheet: a layer
        // first named in an earlier sheet keeps its place when a later sheet
        // adds to it.
        for name in other.layer_order {
            if !self.layer_order.contains(&name) {
                self.layer_order.push(name);
            }
        }
        self.uses_has |= other.uses_has;
        self.rule_index.rebuild(&self.rules);
    }

    /// Makes every rule in this sheet conditional on `condition` as well as on
    /// whatever it already asked for.
    ///
    /// A `<link>` carries its own media query, and ignoring it applies sheets a
    /// browser would not load at all. firefox.com links its pre-layers base
    /// stylesheet as `media="all and (-ms-high-contrast: none)"` -- an IE-only
    /// test that matches nothing now. Applied anyway, its unlayered rules
    /// outranked the entire modern sheet: the page came out 700px wide with its
    /// navigation set to `display: none`.
    pub(crate) fn apply_media(&mut self, condition: MediaCondition) {
        for rule in &mut self.rules {
            rule.media = Some(match rule.media.take() {
                Some(existing) => MediaCondition::All(vec![condition.clone(), existing]),
                None => condition.clone(),
            });
        }
    }

    /// Where a rule sits in the cascade's layer ordering.
    ///
    /// Unlayered rules are the strongest normal declarations an author can
    /// write, which is exactly what firefox.com relies on: its base stylesheet
    /// sets `body { width: 700px }` outside any layer, and the
    /// `@layer defaults { body { inline-size: 100% } }` that comes later in the
    /// source does not override it.
    fn layer_rank(&self, layer: Option<&Arc<str>>) -> u32 {
        let Some(name) = layer else {
            return u32::MAX;
        };
        self.layer_order
            .iter()
            .position(|known| known == name)
            .map_or(u32::MAX, |index| index as u32)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct RuleIndex {
    by_id: HashMap<String, Vec<usize>>,
    by_class: HashMap<String, Vec<usize>>,
    by_tag: HashMap<String, Vec<usize>>,
    universal: Vec<usize>,
}

impl RuleIndex {
    fn rebuild(&mut self, rules: &[Rule]) {
        *self = Self::build(rules);
    }

    fn build(rules: &[Rule]) -> Self {
        let mut index = Self::default();
        for (rule_index, rule) in rules.iter().enumerate() {
            let mut saw_bucket = false;
            for selector in &rule.selectors {
                if let Some(bucket) = selector.key_bucket() {
                    saw_bucket = true;
                    bucket.insert(&mut index, rule_index);
                } else {
                    saw_bucket = true;
                    index.universal.push(rule_index);
                }
            }
            if !saw_bucket {
                continue;
            }
        }
        index.sort_dedup();
        index
    }

    fn sort_dedup(&mut self) {
        for values in self.by_id.values_mut() {
            values.sort_unstable();
            values.dedup();
        }
        for values in self.by_class.values_mut() {
            values.sort_unstable();
            values.dedup();
        }
        for values in self.by_tag.values_mut() {
            values.sort_unstable();
            values.dedup();
        }
        self.universal.sort_unstable();
        self.universal.dedup();
    }

    fn candidates_for(&self, element: &ElementIdentity) -> Vec<usize> {
        let mut candidates = Vec::new();
        if let Some(id) = &element.id && let Some(values) = self.by_id.get(id) {
            candidates.extend(values);
        }
        for class_name in &element.classes {
            if let Some(values) = self.by_class.get(class_name) {
                candidates.extend(values);
            }
        }
        if let Some(values) = self.by_tag.get(&element.tag_name) {
            candidates.extend(values);
        }
        candidates.extend(&self.universal);
        candidates.sort_unstable();
        candidates.dedup();
        candidates
    }
}

enum RuleBucket<'a> {
    Id(&'a str),
    Class(&'a str),
    Tag(&'a str),
    Universal,
}

impl<'a> RuleBucket<'a> {
    fn insert(self, index: &mut RuleIndex, rule_index: usize) {
        match self {
            RuleBucket::Id(id) => index.by_id.entry(id.to_string()).or_default().push(rule_index),
            RuleBucket::Class(class) => index.by_class.entry(class.to_string()).or_default().push(rule_index),
            RuleBucket::Tag(tag) => index.by_tag.entry(tag.to_string()).or_default().push(rule_index),
            RuleBucket::Universal => index.universal.push(rule_index),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    selectors: Vec<Selector>,
    declarations: Vec<Declaration>,
    /// None = always apply; Some(cond) = apply only when cond matches
    media: Option<MediaCondition>,
    /// The cascade layer this rule was written in, `None` for an unlayered
    /// rule. Nested layers are joined with a dot, as the spec names them.
    layer: Option<Arc<str>>,
    pub pseudo_element: Option<PseudoElement>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Selector types
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
struct Selector {
    parts: Vec<SelectorPart>,
    pseudo_element: Option<PseudoElement>,
    /// What this selector counts for, when that differs from what its parts add
    /// up to.
    ///
    /// `:where()` matches like `:is()` but contributes nothing to specificity.
    /// Splicing its argument in makes it match correctly and count wrongly, and
    /// counting wrongly changes which rule wins: firefox.com ends its front-page
    /// hero with `:where(:not(.conditional-display)) > .fl-intro:first-child`,
    /// which a browser scores below the `.fl-home-intro .fl-intro:first-child`
    /// written earlier. Scored above it, the later rule's `padding-block` put
    /// 128px under the hero where 64px belongs.
    specificity_override: Option<usize>,
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
    /// `:has(...)`, answered by looking at the element's children.
    ///
    /// Only the child form is exact: `:has(> .x)` asks precisely this question.
    /// The descendant form `:has(.x)` should search the whole subtree, and
    /// answering it from the children alone can only say "no" where a browser
    /// says "yes" -- the same answer this renderer gave before, when an
    /// unmodelled `:has()` made its rule match nothing at all.
    Has(Vec<SimpleSelector>),
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
    important: bool,
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
    /// This element's own element children, for `:has()`. Empty everywhere a
    /// slot is built for something other than the element being matched, so a
    /// `:has()` nested inside another selector's argument answers no rather
    /// than reaching for data that is not there.
    children: Rc<[ElementIdentity]>,
}

impl Selector {
    /// Whether any part of this selector asks a `:has()` question.
    fn mentions_has(&self) -> bool {
        self.parts
            .iter()
            .any(|part| part.simple.pseudo_classes.iter().any(|pseudo| {
                matches!(pseudo, PseudoClass::Has(_))
            }))
    }
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

/// Which part of a table an element plays, when CSS says so rather than the
/// markup.
///
/// `display: table` is a real layout, not a synonym for `block`: the cells of a
/// row share the row out between them, and one with a stated width leaves the
/// rest to the others. It is kept beside `display` rather than inside it so
/// that every box that is block-level on the outside still reads as
/// `Display::Block` and no existing match has to grow a case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TableRole {
    #[default]
    None,
    Table,
    /// `table-row-group`, `table-header-group`, `table-footer-group`.
    RowGroup,
    Row,
    Cell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Display {
    Block,
    Inline,
    /// Inline-level on the outside, a block container on the inside.
    ///
    /// Collapsing this to plain `Inline` looked harmless but silently deleted
    /// content: an inline formatting context drops block-level children, so
    /// everything nested inside an `inline-block` wrapper vanished.
    InlineBlock,
    ListItem,
    /// Generates no box at all: the element's children stand in its place.
    ///
    /// Collapsing this to `Inline` (which is what used to happen) turns a
    /// transparent wrapper into a real inline box, so its block children stack
    /// vertically instead of becoming items of the grid or flex container that
    /// should have adopted them.
    Contents,
    None,
    Flex,
    InlineFlex,
    Grid,
    InlineGrid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VerticalAlign {
    Top,
    Middle,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WhiteSpaceMode {
    Normal,
    Pre,
    NoWrap,
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TextShadow {
    pub offset_x: i32,
    pub offset_y: i32,
    pub blur: u32,
    pub color: u32,
}

// ─────────────────────────────────────────────────────────────────────────────
// LinearGradient
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LinearGradient {
    pub angle_deg_x1000: i32,
    pub stops: Vec<(u32, u32)>, // (color, position 0-1000)
    /// `radial-gradient()` rather than `linear-gradient()`: the stops run out
    /// from the centre instead of along an angle. The two share everything but
    /// that, so they share a parser and a command.
    pub radial: bool,
}

// ─────────────────────────────────────────────────────────────────────────────
// BackgroundSize / BackgroundRepeat
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LengthValue {
    Pixels(u32),
    Percent(u32),
    MinContent,
    MaxContent,
    FitContent(u32), // argument in pixels
    /// `calc()` reduced to "a share of the containing block, plus an offset".
    ///
    /// The percentage has to survive until layout, when the containing block is
    /// known; resolving it at parse time against the font size turned the very
    /// common `calc(100% - 20px)` into a handful of pixels. `percent_hundredths`
    /// keeps two decimal places, because real stylesheets write column widths
    /// like `calc(47.47475% - 20px)`.
    Calc {
        percent_hundredths: i32,
        px: i32,
    },
    /// `min()`, `max()` and `clamp()`: a linear form with optional bounds, each
    /// bound a linear form of its own. Like `calc()`, the percentages have to
    /// survive until the containing block is known.
    ///
    /// All three functions reduce to this shape -- `min(a, b)` is `a` bounded
    /// above by `b`, `max(a, b)` is `a` bounded below by `b`. Unparsed, they
    /// collapsed to almost nothing: firefox.com caps a banner's text column with
    /// `max-inline-size: min(600px, 100%)`, and at 16px wide it stacked a 64px
    /// heading one character to a line and ran to twelve hundred pixels.
    Bounded {
        lower: Option<(i32, i32)>,
        value: (i32, i32),
        upper: Option<(i32, i32)>,
    },
}

/// Resolves a `min()` / `max()` / `clamp()` against a known containing block.
pub fn resolve_bounded(
    lower: Option<(i32, i32)>,
    value: (i32, i32),
    upper: Option<(i32, i32)>,
    container: u32,
) -> u32 {
    let at = |(percent_hundredths, px): (i32, i32)| -> i64 {
        i64::from(percent_hundredths) * i64::from(container) / 10_000 + i64::from(px)
    };
    let mut resolved = at(value);
    if let Some(upper) = upper {
        resolved = resolved.min(at(upper));
    }
    // The lower bound wins a contradiction, as `clamp()` specifies.
    if let Some(lower) = lower {
        resolved = resolved.max(at(lower));
    }
    resolved.clamp(0, i64::from(u32::MAX)) as u32
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextTransform {
    None,
    Uppercase,
    Lowercase,
    Capitalize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BoxSizing {
    ContentBox,
    BorderBox,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BoxShadow {
    pub offset_x: i32,
    pub offset_y: i32,
    pub blur: u32,
    pub color: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Overflow {
    Visible,
    Hidden,
    Auto,
    Scroll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Position {
    Static,
    Relative,
    Absolute,
    Fixed,
    Sticky,
}

impl Default for Position {
    fn default() -> Self { Position::Static }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlexDirection { Row, Column, RowReverse, ColumnReverse }
impl Default for FlexDirection { fn default() -> Self { FlexDirection::Row } }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlexWrap { NoWrap, Wrap, WrapReverse }
impl Default for FlexWrap { fn default() -> Self { FlexWrap::NoWrap } }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AlignItems { Stretch, FlexStart, FlexEnd, Center, Baseline }
impl Default for AlignItems { fn default() -> Self { AlignItems::Stretch } }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JustifyContent { FlexStart, FlexEnd, Center, SpaceBetween, SpaceAround, SpaceEvenly }
impl Default for JustifyContent { fn default() -> Self { JustifyContent::FlexStart } }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AlignSelf { Auto, Stretch, FlexStart, FlexEnd, Center, Baseline }
impl Default for AlignSelf { fn default() -> Self { AlignSelf::Auto } }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AlignContent {
    FlexStart,
    FlexEnd,
    Center,
    SpaceBetween,
    SpaceAround,
    Stretch,
}
impl Default for AlignContent { fn default() -> Self { AlignContent::Stretch } }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum ObjectFit {
    #[default]
    Fill,
    Contain,
    Cover,
    ScaleDown,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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

/// Which edge of a grid area a `<custom-ident>` placement refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GridEdge {
    Start,
    End,
}

/// The names attached to grid lines by a track list, as `(name, line index)`.
///
/// Line indices are 0-based and count lines, not tracks, so the line before the
/// first track is 0 and a list of N tracks has lines 0..=N.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct GridLineNames {
    pub columns: Vec<(Box<str>, usize)>,
    pub rows: Vec<(Box<str>, usize)>,
}

fn lookup_grid_line(list: &[(Box<str>, usize)], name: &str, edge: GridEdge) -> Option<usize> {
    if let Some(&(_, index)) = list.iter().find(|(n, _)| &**n == name) {
        return Some(index);
    }
    // A bare `foo` also reaches the `foo-start` / `foo-end` pair, which is how
    // both a named area's implicit lines and an explicitly named line pair are
    // addressed by a single identifier.
    let suffixed = match edge {
        GridEdge::Start => format!("{name}-start"),
        GridEdge::End => format!("{name}-end"),
    };
    list.iter()
        .find(|(n, _)| &**n == suffixed.as_str())
        .map(|&(_, index)| index)
}

impl GridLineNames {
    pub fn column_line(&self, name: &str, edge: GridEdge) -> Option<usize> {
        lookup_grid_line(&self.columns, name, edge)
    }

    pub fn row_line(&self, name: &str, edge: GridEdge) -> Option<usize> {
        lookup_grid_line(&self.rows, name, edge)
    }

    pub fn is_empty(&self) -> bool {
        self.columns.is_empty() && self.rows.is_empty()
    }
}

/// `<custom-ident>` line references an item asked for, kept until layout can
/// look them up in the container's track list.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct GridPlacementNames {
    pub row_start: Option<Box<str>>,
    pub row_end: Option<Box<str>>,
    pub column_start: Option<Box<str>>,
    pub column_end: Option<Box<str>>,
}

impl GridPlacementNames {
    pub fn is_empty(&self) -> bool {
        self.row_start.is_none()
            && self.row_end.is_none()
            && self.column_start.is_none()
            && self.column_end.is_none()
    }
}

/// One `<grid-line>` value.
#[derive(Debug, Clone, PartialEq, Eq)]
enum GridLineRef {
    Auto,
    Line(i32),
    Named(Box<str>),
    Span(u32),
}

fn parse_grid_line_ref(s: &str) -> GridLineRef {
    let s = s.trim();
    if s.is_empty() || s.eq_ignore_ascii_case("auto") {
        return GridLineRef::Auto;
    }
    if let Some(rest) = s.strip_prefix("span") {
        return GridLineRef::Span(rest.trim().parse().unwrap_or(1));
    }
    if let Ok(n) = s.parse::<i32>() {
        // `0` is not a valid line number.
        return if n == 0 {
            GridLineRef::Auto
        } else {
            GridLineRef::Line(n)
        };
    }
    if is_grid_area_ident(s) {
        return GridLineRef::Named(s.to_string().into_boxed_str());
    }
    GridLineRef::Auto
}

/// A parsed `grid-template-areas` value: the size of the explicit grid the
/// strings describe, plus one rectangle per named area.
///
/// Only the rectangles are kept, never the cell grid, because a rectangle is
/// all layout ever asks for. That also puts the spec's validity rules --
/// every row has the same number of tokens, and every named area is a single
/// filled rectangle -- at parse time, which is where an invalid declaration
/// has to be dropped whole rather than partially honoured.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GridTemplateAreas {
    pub rows: usize,
    pub columns: usize,
    /// `(name, row_start, column_start, row_end, column_end)`, 0-based and
    /// half-open, so a one-cell area at the origin is `(_, 0, 0, 1, 1)`.
    pub areas: Vec<(Box<str>, usize, usize, usize, usize)>,
}

impl GridTemplateAreas {
    /// The rectangle named `name`, if the template defines one.
    pub fn area(&self, name: &str) -> Option<(usize, usize, usize, usize)> {
        self.areas
            .iter()
            .find(|(area_name, ..)| &**area_name == name)
            .map(|&(_, row_start, col_start, row_end, col_end)| {
                (row_start, col_start, row_end, col_end)
            })
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct SignedEdgeSizes {
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub left: i32,
}

impl SignedEdgeSizes {
    pub fn all(value: i32) -> Self {
        Self {
            top: value,
            right: value,
            bottom: value,
            left: value,
        }
    }

    pub fn vertical(top: i32, bottom: i32) -> Self {
        Self {
            top,
            right: 0,
            bottom,
            left: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FloatSide {
    None,
    Left,
    Right,
}

impl Default for FloatSide {
    fn default() -> Self {
        FloatSide::None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClearSide {
    None,
    Left,
    Right,
    Both,
}

impl Default for ClearSide {
    fn default() -> Self {
        ClearSide::None
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ComputedStyle
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ComputedStyle {
    pub display: Display,
    /// Set only when `display` named a table part.
    pub table_role: TableRole,
    pub color: Color,
    pub background_color: Option<Color>,
    pub margin: SignedEdgeSizes,
    pub margin_left_auto: bool,
    pub margin_right_auto: bool,
    pub padding: EdgeSizes,
    pub width: Option<LengthValue>,
    pub height: Option<LengthValue>,
    pub font_size_px: u32,
    pub font_family: FontFamilyKind,
    pub text_align: TextAlign,
    /// `text-wrap: balance` -- spread the run over the same number of lines,
    /// but as evenly as they will go.
    ///
    /// Inherited, like the rest of `text-wrap`. Only `balance` is modelled:
    /// `pretty` only pulls a short last line up, which is a subtler thing than
    /// this does, and the rest of the keywords say "wrap normally".
    pub text_wrap_balance: bool,
    pub vertical_align: VerticalAlign,
    pub font_weight: bool,
    pub underline: bool,
    pub line_through: bool,
    pub white_space: WhiteSpaceMode,
    /// Whether a word too long for its line may be broken mid-word.
    ///
    /// Off by default: a browser lets such a word overflow rather than cutting
    /// it. `overflow-wrap: break-word`, `word-wrap: break-word` and
    /// `word-break: break-all` each turn it on, and pages that hold long URLs
    /// or identifiers say so.
    pub break_long_words: bool,
    /// How far the text is lifted off the line's baseline, in pixels.
    ///
    /// Negative is up. `<sup>` and `<sub>` are the only things that set it, and
    /// they are why a footnote marker or the 2 in H2O sits where it does.
    pub baseline_shift: i32,
    pub text_overflow_ellipsis: bool,
    pub text_shadow: Option<TextShadow>,
    pub background_gradient: Option<LinearGradient>,
    pub background_image_url: Option<String>,
    /// `mask-image`: the shape the element's own colour is painted in.
    ///
    /// Icons on modern pages are an empty box with a mask and
    /// `background-color: currentColor`, so the same drawing takes the colour of
    /// the text around it. Without the mask the box was painted whole -- every
    /// icon on firefox.com came out as a filled square.
    pub mask_image_url: Option<String>,
    pub background_size: BackgroundSize,
    pub background_repeat: BackgroundRepeat,
    pub background_position_x: u32,
    pub background_position_y: u32,
    // ── new fields ──
    pub float: FloatSide,
    pub clear: ClearSide,
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
    /// May be negative: `text-indent: -9999px` inside an `overflow: hidden`
    /// box is the standard way to show a logo as a background image while
    /// keeping a real `<img>` in the markup for anyone not seeing it. Held
    /// unsigned, that indent parsed as nothing and both logos painted at once.
    pub text_indent: i32,
    pub letter_spacing: i32,
    pub max_width: Option<LengthValue>,
    pub min_width: Option<LengthValue>,
    pub max_height: Option<u32>,
    pub min_height: u32,
    pub box_sizing: BoxSizing,
    pub overflow: Overflow,
    pub list_style_type: ListStyleType,
    pub cursor_pointer: bool,
    pub cursor_kind: CursorKind,
    pub pointer_events_none: bool,
    pub text_decoration_color: Option<Color>,
    pub box_shadow: Option<BoxShadow>,
    pub content: Option<String>,
    // Position
    pub position: Position,
    pub z_index: Option<i32>,
    /// Box offsets. These keep their percentage until layout, because it
    /// resolves against the containing block -- `left`/`right` against its
    /// width, `top`/`bottom` against its height -- and not against the font
    /// size, which is what a plain pixel value here forced.
    pub top: Option<LengthValue>,
    pub right: Option<LengthValue>,
    pub bottom: Option<LengthValue>,
    pub left: Option<LengthValue>,
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
    /// Custom properties (`--x`) declared on this element or on an ancestor.
    ///
    /// They inherit like any other property, so a `--gap` set on a container is
    /// visible to everything inside it. Only the element's own declarations were
    /// consulted before, so MDN's `.menu { --menu-button-padding: ... }` never
    /// reached the `.menu__tab-link { padding: var(--menu-button-padding) }`
    /// beneath it and that padding silently vanished.
    ///
    /// `:root` and `@media` root variables are *not* copied in here: every
    /// element can see those already, so they stay on the stylesheet and are
    /// consulted as a fallback. Only what an ancestor actually declared travels
    /// with the style, which keeps this empty on most pages.
    ///
    /// `Arc`, not `Rc`: the finished style tree is handed to the render worker
    /// thread, so everything hanging off it has to be `Send`.
    /// The border colour resolved to something that paints nothing --
    /// `transparent`, `#0000`, `rgba(..., 0)`.
    ///
    /// The width still counts for layout, so this cannot be folded into
    /// `border_style_none` (which also zeroes the widths). Falling back to the
    /// default colour instead drew a solid black line wherever a page used
    /// `border: 1px solid transparent` to reserve space -- MDN rules its nav
    /// tabs that way, so the bar came out boxed in black.
    pub border_color_transparent: bool,
    pub custom_properties: Option<Arc<BTreeMap<String, String>>>,
    /// Line names from `grid-template-columns` / `grid-template-rows`. Boxed
    /// for the same reason as the areas below: most pages never name a line.
    pub grid_line_names: Option<Box<GridLineNames>>,
    /// `grid-template-areas`. Boxed because it is rare and `ComputedStyle` is
    /// ~520 bytes shared through `Arc` + interning -- cold fields stay behind a
    /// pointer so the common style pays 8 bytes, not the whole table.
    pub grid_template_areas: Option<Box<GridTemplateAreas>>,
    // Grid item fields
    pub grid_column: GridPlacement,
    pub grid_row: GridPlacement,
    /// `grid-area: <custom-ident>`, resolved against the containing grid's
    /// areas at layout time (the item cannot see them from here). Boxed for
    /// the same reason as `grid_template_areas`.
    pub grid_area_name: Option<Box<str>>,
    /// `<custom-ident>` line references on this item, resolved against the
    /// container's line names at layout time.
    pub grid_placement_names: Option<Box<GridPlacementNames>>,
    // Filter effects
    pub filter_blur_px: u32,       // blur() value in pixels, 0 = no blur
    pub filter_brightness: u32,    // brightness() in percent * 100 (10000 = 100% = no change)
    pub filter_opacity: u8,        // opacity() as 0-255, 255 = no change
    // CSS transform (all integer to keep ComputedStyle: Eq)
    /// translate X in pixels (0 = no translate)
    pub transform_translate_x: i32,
    /// translate Y in pixels (0 = no translate)
    pub transform_translate_y: i32,
    /// scaleX * 1000 (1000 = 1.0, no scale). 0 is treated as "not set" → 1000
    pub transform_scale_x: u32,
    /// scaleY * 1000 (1000 = 1.0, no scale). 0 is treated as "not set" → 1000
    pub transform_scale_y: u32,
    /// rotation in millidegrees clockwise (0 = no rotation)
    pub transform_rotate_millideg: i32,
    /// transform-origin X in permille of element width (500 = 50% = center)
    pub transform_origin_x: u32,
    /// transform-origin Y in permille of element height (500 = 50% = center)
    pub transform_origin_y: u32,
    /// The `cellpadding` attribute of the table this element sits in.
    ///
    /// Not a CSS property. It is carried down the tree because the
    /// attribute is written on the table and decides the padding of every
    /// cell under it -- the standard maps it to a UA rule scoped to that
    /// table, which is why a `td { padding }` in the page overrides it
    /// rather than adding to it.
    pub table_cellpadding: Option<u32>,
}

impl ComputedStyle {
    fn for_element(tag_name: &str, parent: Option<&Self>) -> Self {
        let parent_font_size = parent.map(|s| s.font_size_px).unwrap_or(16);
        let mut style = Self {
            // Custom properties inherit; the ancestors' map is shared, not copied.
            border_color_transparent: false,
            custom_properties: parent.and_then(|s| s.custom_properties.clone()),
            display: default_display(tag_name),
            table_role: TableRole::None,
            color: parent.map(|s| s.color).unwrap_or(DEFAULT_TEXT_COLOR),
            background_color: None,
            margin: default_margin(tag_name),
            margin_left_auto: false,
            margin_right_auto: false,
            padding: EdgeSizes::default(),
            table_cellpadding: parent.and_then(|parent| parent.table_cellpadding),
            width: None,
            height: None,
            font_size_px: parent_font_size,
            font_family: parent
                .map(|s| s.font_family)
                .unwrap_or(FontFamilyKind::Sans),
            text_align: parent.map(|s| s.text_align).unwrap_or(TextAlign::Left),
            text_wrap_balance: parent.map(|s| s.text_wrap_balance).unwrap_or(false),
            vertical_align: VerticalAlign::Top,
            font_weight: parent.map(|s| s.font_weight).unwrap_or(false),
            underline: parent.map(|s| s.underline).unwrap_or(false),
            line_through: parent.map(|s| s.line_through).unwrap_or(false),
            white_space: parent
                .map(|s| s.white_space)
                .unwrap_or(WhiteSpaceMode::Normal),
            break_long_words: parent.map(|s| s.break_long_words).unwrap_or(false),
            // Not inherited: a `<sup>` inside a `<sup>` is raised once more
            // from where the outer one put it, not twice from the baseline.
            baseline_shift: 0,
            text_overflow_ellipsis: false,
            text_shadow: None,
            background_gradient: None,
            background_image_url: None,
            mask_image_url: None,
            background_size: BackgroundSize::Auto,
            background_repeat: BackgroundRepeat::Repeat,
            background_position_x: 50,
            background_position_y: 50,
            // new fields – most not inherited
            float: FloatSide::None,
            clear: ClearSide::None,
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
            text_indent: parent.map(|s| s.text_indent).unwrap_or(0),
            letter_spacing: parent.map(|s| s.letter_spacing).unwrap_or(0),
            max_width: None,
            min_width: None,
            max_height: None,
            min_height: 0,
            box_sizing: BoxSizing::ContentBox,
            overflow: Overflow::Visible,
            list_style_type: default_list_style_type(tag_name, parent),
            cursor_pointer: false,
            cursor_kind: CursorKind::Auto,
            pointer_events_none: false,
            text_decoration_color: None,
            box_shadow: None,
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
            grid_line_names: None,
            grid_template_areas: None,
            grid_column: GridPlacement::default(),
            grid_row: GridPlacement::default(),
            grid_area_name: None,
            grid_placement_names: None,
            // Filter effects
            filter_blur_px: 0,
            filter_brightness: 10000,
            filter_opacity: 255,
            // CSS transform
            transform_translate_x: 0,
            transform_translate_y: 0,
            transform_scale_x: 0,  // 0 = "not set" → treated as 1000 at render time
            transform_scale_y: 0,
            transform_rotate_millideg: 0,
            transform_origin_x: 500,  // 50% center
            transform_origin_y: 500,
        };

        match tag_name {
            "body" => {
                style.margin = SignedEdgeSizes::all(8);
            }
            // The sizes and margins a browser's own sheet gives a heading,
            // in the em the standard states them in: 2em/0.67em down to
            // 0.67em/2.33em. Ours ran a step large from h2 down, so a page
            // that leaves its headings alone showed them all too big.
            "h1" => {
                style.font_size_px = 32;
                style.font_weight = true;
                style.margin = SignedEdgeSizes::vertical(21, 21);
            }
            "h2" => {
                style.font_size_px = 24;
                style.font_weight = true;
                style.margin = SignedEdgeSizes::vertical(20, 20);
            }
            "h3" => {
                style.font_size_px = 19;
                style.font_weight = true;
                style.margin = SignedEdgeSizes::vertical(19, 19);
            }
            "h4" => {
                style.font_size_px = 16;
                style.font_weight = true;
                style.margin = SignedEdgeSizes::vertical(21, 21);
            }
            "h5" => {
                style.font_size_px = 13;
                style.font_weight = true;
                style.margin = SignedEdgeSizes::vertical(22, 22);
            }
            "h6" => {
                style.font_size_px = 11;
                style.font_weight = true;
                style.margin = SignedEdgeSizes::vertical(25, 25);
            }
            "a" => {
                style.color = DEFAULT_LINK_COLOR;
                style.underline = true;
            }
            // A control is drawn in the platform's own colours, not the page's:
            // its box is a light field whatever the surrounding text is. Left
            // inheriting, firefox.com's footer picker drew white text on the
            // white box. Set here rather than at paint time so an authored
            // `color` still wins -- this is a default, not an override.
            "select" => style.color = DEFAULT_TEXT_COLOR,
            "pre" => {
                style.font_family = FontFamilyKind::Monospace;
                style.white_space = WhiteSpaceMode::Pre;
                style.margin = SignedEdgeSizes::vertical(12, 12);
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
            // Smaller type, lifted off the baseline. The size is the browser's
            // own `smaller`, and the lift is a third of the surrounding type --
            // which is what puts a footnote marker beside the word rather than
            // in the middle of it.
            "sup" | "sub" => {
                style.font_size_px = (parent_font_size * 83 / 100).max(1);
                let shift = (parent_font_size * 33 / 100) as i32;
                style.baseline_shift = if tag_name == "sup" { -shift } else { shift / 2 };
            }
            "small" => {
                style.font_size_px = parent_font_size.saturating_sub(2).max(12);
            }
            "big" => {
                style.font_size_px = parent_font_size.saturating_add(2);
            }
            "td" | "th" => {
                style.vertical_align = VerticalAlign::Middle;
                // The UA stylesheet gives a cell a pixel on every side,
                // and `cellpadding` on the table replaces that number.
                // Both sit below the page's own rules, so a `td { padding }`
                // anywhere in the page wins.
                let inset = style.table_cellpadding.unwrap_or(1);
                style.padding = EdgeSizes {
                    top: inset,
                    right: inset,
                    bottom: inset,
                    left: inset,
                };
            }
            // The room a list leaves for its own markers. Without it the
            // bullets sat in the margin of the page and the text started at
            // the left edge, which is not where any browser puts a list.
            "ul" | "ol" | "menu" | "dir" => {
                style.padding = EdgeSizes {
                    top: 0,
                    right: 0,
                    bottom: 0,
                    left: 40,
                };
            }
            "dd" => {
                style.margin.left = 40;
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
    /// Shared through [`intern_style`]: pages repeat a handful of computed
    /// styles across many nodes, so holding one inline per node cost 520 bytes
    /// each for values that are overwhelmingly duplicates.
    pub style: Arc<ComputedStyle>,
    pub children: Vec<StyledNode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyledText {
    pub text: String,
    pub style: Arc<ComputedStyle>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Public API
// ─────────────────────────────────────────────────────────────────────────────

/// Split `input` on `delimiter` but only at depth 0 (ignoring delimiters inside
/// parentheses/brackets and quoted strings).  This prevents `:not(.a, .b)` from
/// being split on the inner comma.
/// The same selector with every `:where(...)` group taken out, for scoring.
///
/// A group at the start of a compound leaves a `*` behind so the result is still
/// a selector; one after something else just goes.
fn without_where_groups(selector: &str) -> String {
    let mut out = selector.to_string();
    loop {
        let lower = out.to_ascii_lowercase();
        let Some(start) = lower.find(":where(") else {
            return out;
        };
        let mut depth = 0_usize;
        let mut end = None;
        for (index, character) in out[start..].char_indices() {
            match character {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(start + index);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(end) = end else {
            return out;
        };
        let starts_compound = out[..start]
            .chars()
            .next_back()
            .is_none_or(|previous| matches!(previous, ' ' | '>' | '+' | '~' | ','));
        let replacement = if starts_compound { "*" } else { "" };
        out.replace_range(start..=end, replacement);
    }
}

/// Rewrite `:is(...)` / `:where(...)` into a plain selector list.
///
/// Both were listed as "ignorable" pseudo-classes, meaning always satisfied with
/// their argument discarded. That turns `:is(.homepage-hero h1)::after` into
/// `*::after`, so MDN's `content: "_"` -- the little terminal cursor meant for
/// one heading -- was stamped after the text of every element on the page.
///
/// The argument is an ordinary selector list, so splicing it in at parse time is
/// enough; the matcher then only ever sees plain selectors, and nesting falls out
/// of the recursion. Run-time matching would instead need ancestor context inside
/// `SimpleSelector`, which it does not have.
///
/// One shape is deliberately left alone: with something before the group, as in
/// `.x:is(.y .z)`, a textual splice would say `.x.y .z` when the selector means
/// "a `.z` inside a `.y`, which is also `.x`". Those keep the old behaviour.
fn expand_selector_groups(selector: &str) -> Vec<String> {
    const GROUPS: [&str; 4] = [":is(", ":where(", ":matches(", ":any("];
    let lower = selector.to_ascii_lowercase();

    // The first group that is not itself inside brackets.
    let mut depth = 0usize;
    let mut opened: Option<(usize, usize)> = None;
    for (index, character) in selector.char_indices() {
        match character {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            ':' if depth == 0 => {
                if let Some(group) = GROUPS.iter().find(|g| lower[index..].starts_with(**g)) {
                    opened = Some((index, index + group.len()));
                    break;
                }
            }
            _ => {}
        }
    }
    let Some((start, args_start)) = opened else {
        return vec![selector.to_string()];
    };

    let mut depth = 1usize;
    let mut args_end = None;
    for (offset, character) in selector[args_start..].char_indices() {
        match character {
            '(' | '[' => depth += 1,
            ')' | ']' => {
                depth -= 1;
                if depth == 0 {
                    args_end = Some(args_start + offset);
                    break;
                }
            }
            _ => {}
        }
    }
    let Some(args_end) = args_end else {
        return vec![selector.to_string()];
    };

    let prefix = &selector[..start];
    let arguments = &selector[args_start..args_end];
    let suffix = &selector[args_end + 1..];

    let mut expanded = Vec::new();
    for alternative in split_at_top_level(arguments, ',') {
        let alternative = alternative.trim();
        if alternative.is_empty() {
            continue;
        }
        let is_compound = !alternative.contains([' ', '>', '+', '~']);
        if !prefix.trim().is_empty() && !is_compound {
            return vec![selector.to_string()];
        }
        expanded.extend(expand_selector_groups(&format!(
            "{prefix}{alternative}{suffix}"
        )));
    }

    if expanded.is_empty() {
        vec![selector.to_string()]
    } else {
        expanded
    }
}

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
    let mut in_string: Option<char> = None;
    let mut escaped = false;
    for (i, ch) in source.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
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

fn find_block_open(source: &str) -> Option<usize> {
    let mut in_string: Option<char> = None;
    let mut escaped = false;
    for (i, ch) in source.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
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
            '{' => return Some(i),
            _ => {}
        }
    }
    None
}

pub fn parse_stylesheet(input: &str) -> Stylesheet {
    let mut rules = Vec::new();
    let mut root_vars = BTreeMap::new();
    let mut media_root_vars: Vec<(MediaCondition, BTreeMap<String, String>)> = Vec::new();
    let mut layer_order: Vec<Arc<str>> = Vec::new();
    let source = strip_comments(input);
    let mut cursor = 0;

    while let Some(open_offset) = find_block_open(&source[cursor..]) {
        let selector_start = cursor;
        let selector_end = cursor + open_offset;
        let block_start = selector_end + 1;

        let block_text_raw = &source[block_start..];
        let Some(close_offset) = find_matching_close_brace(block_text_raw) else {
            // Every rule after this point is lost, so a sheet that trips here
            // goes quiet rather than wrong-looking. Say so when tracing.
            if css_debug_enabled() {
                eprintln!(
                    "[css] unbalanced braces at byte {block_start}; dropping the rest of a {}-byte sheet (after {:?})",
                    source.len(),
                    &source[selector_start..selector_end.min(selector_start + 60)]
                );
            }
            break;
        };
        let block_end = block_start + close_offset;

        let (statements, selector_text) =
            split_statement_prelude(source[selector_start..selector_end].trim());
        // `@layer a, b, c;` names an order without opening a block, and it is
        // the only way a sheet can put a layer ahead of one that appears
        // earlier in the source. It has to be read before the block it runs
        // into is parsed.
        register_layer_statements(statements, &mut layer_order);
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
                for name in inner_stylesheet.layer_order {
                    register_layer(&name, &mut layer_order);
                }
                for mut rule in inner_stylesheet.rules {
                    rule.media = Some(media_cond.clone());
                    rules.push(rule);
                }
            } else if at_lower.starts_with("@supports") || at_lower.starts_with("@layer") {
                // @layer: ignore layer name, parse rules as regular rules (no cascade layering)
                if let Some(condition) = at_lower.strip_prefix("@supports")
                    && !supports_condition(condition)
                {
                    continue;
                }
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

                // `@layer name { ... }` puts everything inside it in that layer;
                // an `@layer` nested in another joins their names with a dot.
                // `@supports` is not a layer and leaves its rules where they are.
                let outer = at_lower.strip_prefix("@layer").map(|name| {
                    let name = name.trim();
                    if name.is_empty() {
                        // An anonymous layer is its own layer, distinct from
                        // every other anonymous one, so give it a name nothing
                        // else can collide with.
                        format!("<anonymous {}>", layer_order.len())
                    } else {
                        name.to_string()
                    }
                });
                if let Some(ref outer) = outer {
                    register_layer(outer, &mut layer_order);
                }
                for name in inner_stylesheet.layer_order {
                    match outer {
                        Some(ref outer) => register_layer(&format!("{outer}.{name}"), &mut layer_order),
                        None => register_layer(&name, &mut layer_order),
                    }
                }
                for mut rule in inner_stylesheet.rules {
                    if let Some(ref outer) = outer {
                        rule.layer = Some(match rule.layer {
                            Some(inner) => Arc::from(format!("{outer}.{inner}").as_str()),
                            None => Arc::from(outer.as_str()),
                        });
                    }
                    rules.push(rule);
                }
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
            .flat_map(|part| {
                let part = part.trim();
                // Scored from the selector as written, minus the groups that
                // count for nothing -- the expansion below cannot tell them
                // apart afterwards.
                let override_score = part.to_ascii_lowercase().contains(":where(").then(|| {
                    parse_selector(without_where_groups(part).trim())
                        .map_or(0, |scored| scored.specificity())
                });
                expand_selector_groups(part)
                    .into_iter()
                    .map(move |expanded| (expanded, override_score))
            })
            .filter_map(|(expanded, override_score)| {
                let mut selector = parse_selector(expanded.trim())?;
                selector.specificity_override = override_score;
                Some(selector)
            })
            .collect::<Vec<_>>();

        if !selectors.is_empty() && !declarations.is_empty() {
            let pseudo_element = selectors.iter().find_map(|sel| sel.pseudo_element.clone());
            rules.push(Rule {
                selectors,
                declarations,
                media: None,
                layer: None,
                pseudo_element,
            });
        }
    }

    let uses_has = rules.iter().any(|rule| {
        rule.selectors
            .iter()
            .any(|selector| selector.mentions_has())
    });
    let rule_index = RuleIndex::build(&rules);
    Stylesheet {
        rules,
        uses_has,
        root_vars: Rc::new(root_vars),
        media_root_vars,
        rule_index,
        layer_order,
    }
}

/// Records the layers named by `@layer a, b, c;` statements in a prelude.
fn register_layer_statements(statements: &str, order: &mut Vec<Arc<str>>) {
    for statement in statements.split(';') {
        let statement = statement.trim();
        let Some(names) = statement.strip_prefix("@layer") else {
            continue;
        };
        for name in names.split(',') {
            register_layer(name.trim(), order);
        }
    }
}

fn register_layer(name: &str, order: &mut Vec<Arc<str>>) {
    if name.is_empty() {
        return;
    }
    let name: Arc<str> = Arc::from(name);
    if !order.contains(&name) {
        order.push(name);
    }
}

/// Rewrites a logical property to the physical one it stands for.
///
/// Only `writing-mode: horizontal-tb` is rendered here, so the inline axis is
/// horizontal and the block axis vertical, which makes every logical property
/// below a plain alias. Renaming at parse time rather than where declarations
/// are applied matters twice over: they take part in the cascade, and layout
/// looks some of them up by name. firefox.com sets `body { width: 700px }` near
/// the top of its sheet and overrides it further down with
/// `body { inline-size: 100% }` -- with the override unrecognised the whole
/// site rendered in a 700px column, headings wrapping every few characters.
///
/// The two-value forms (`margin-inline`, `padding-block`, ...) are not aliases
/// of any single property, so they are expanded in `apply_declaration` instead.
fn to_physical_property(property: String) -> String {
    let physical = match property.as_str() {
        "inline-size" => "width",
        "block-size" => "height",
        "min-inline-size" => "min-width",
        "max-inline-size" => "max-width",
        "min-block-size" => "min-height",
        "max-block-size" => "max-height",
        "margin-inline-start" => "margin-left",
        "margin-inline-end" => "margin-right",
        "margin-block-start" => "margin-top",
        "margin-block-end" => "margin-bottom",
        "padding-inline-start" => "padding-left",
        "padding-inline-end" => "padding-right",
        "padding-block-start" => "padding-top",
        "padding-block-end" => "padding-bottom",
        "inset-inline-start" => "left",
        "inset-inline-end" => "right",
        "inset-block-start" => "top",
        "inset-block-end" => "bottom",
        "border-inline-start" => "border-left",
        "border-inline-start-width" => "border-left-width",
        "border-inline-start-color" => "border-left-color",
        "border-inline-start-style" => "border-left-style",
        "border-inline-end" => "border-right",
        "border-inline-end-width" => "border-right-width",
        "border-inline-end-color" => "border-right-color",
        "border-inline-end-style" => "border-right-style",
        "border-block-start" => "border-top",
        "border-block-start-width" => "border-top-width",
        "border-block-start-color" => "border-top-color",
        "border-block-start-style" => "border-top-style",
        "border-block-end" => "border-bottom",
        "border-block-end-width" => "border-bottom-width",
        "border-block-end-color" => "border-bottom-color",
        "border-block-end-style" => "border-bottom-style",
        _ => return property,
    };
    physical.to_string()
}

/// Whether an `@supports` condition holds.
///
/// The answer defaults to yes, which leaves every block this renderer already
/// applied unconditionally exactly where it was. What it adds is a meaning for
/// `not`: pages wrap a whole legacy stylesheet in
/// `@supports not (<some modern feature>)`, and unwrapping that laid the old
/// rules back on top of the new ones. firefox.com re-includes its entire base
/// sheet inside `@supports not (all: revert-layer)` -- that put
/// `body { width: 700px }` back after the layered rules had already replaced it
/// with `inline-size: 100%`, and the whole site rendered in a 700px column with
/// every heading wrapping after a few characters.
///
/// Saying yes to `revert-layer` is a claim about cascade layers, which are
/// unwrapped here and left in source order rather than ordered by declaration.
/// That is an approximation, but a much closer one than a second copy of the
/// base sheet winning over everything built on top of it.
fn supports_condition(condition: &str) -> bool {
    let condition = condition.trim();
    if condition.is_empty() {
        return true;
    }

    if let Some(inner) = strip_wrapping_parens(condition) {
        return supports_condition(inner);
    }

    if let Some(rest) = condition.strip_prefix("not ") {
        return !supports_condition(rest);
    }

    // `and` binds no tighter than `or` in this grammar -- a condition may not
    // mix them without parentheses -- so either split works first.
    if let Some(parts) = split_supports_condition(condition, "and") {
        return parts.iter().all(|part| supports_condition(part));
    }
    if let Some(parts) = split_supports_condition(condition, "or") {
        return parts.iter().any(|part| supports_condition(part));
    }

    if let Some(rest) = condition.strip_prefix("selector(") {
        // `:has()` is the one selector worth answering no to: a page that asks
        // gets a layout built on a relationship this engine cannot match.
        return !rest.contains(":has(");
    }
    if condition.contains('(') && !condition.starts_with('(') {
        // `font-tech(...)`, `font-format(...)` and friends. Nothing here reads
        // them, so the honest answer is no.
        return false;
    }

    let Some((property, value)) = condition.split_once(':') else {
        return true;
    };
    supports_declaration(property.trim(), value.trim())
}

/// Features named in an `@supports` test that this renderer plainly lacks.
///
/// Everything not listed answers yes, so this only ever removes a block that
/// would have been applied before.
fn supports_declaration(property: &str, _value: &str) -> bool {
    !matches!(
        property,
        // Container queries: a block gated on these lays the page out against a
        // container size nothing here measures.
        "container-type" | "container-name" | "container"
        // Anchor positioning.
        | "anchor-name" | "position-anchor" | "position-area"
        // Effects with no painter behind them.
        | "backdrop-filter" | "-webkit-backdrop-filter"
    )
}

/// The inside of `(...)` when the whole string is one parenthesised group.
fn strip_wrapping_parens(condition: &str) -> Option<&str> {
    let inner = condition.strip_prefix('(')?.strip_suffix(')')?;
    let mut depth = 0_i32;
    for byte in inner.bytes() {
        match byte {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                // The opening paren closed before the end, so the string is a
                // sequence like `(a) and (b)` rather than one group.
                if depth < 0 {
                    return None;
                }
            }
            _ => {}
        }
    }
    // A leaf `(display: grid)` has to keep its parens off, but a bare
    // `display: grid` must not be split further either -- both are handled by
    // the caller re-entering with the inside.
    Some(inner)
}

/// Splits `a <keyword> b <keyword> c` at paren depth zero.
fn split_supports_condition<'a>(condition: &'a str, keyword: &str) -> Option<Vec<&'a str>> {
    let mut parts = Vec::new();
    let mut depth = 0_i32;
    let mut start = 0;
    let bytes = condition.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            _ if depth == 0 => {
                let rest = &condition[i..];
                let after = i + keyword.len();
                if rest.starts_with(keyword)
                    && (i == 0 || bytes[i - 1].is_ascii_whitespace())
                    && bytes.get(after).is_some_and(u8::is_ascii_whitespace)
                {
                    parts.push(condition[start..i].trim());
                    start = after;
                    i = after;
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }
    if parts.is_empty() {
        return None;
    }
    parts.push(condition[start..].trim());
    Some(parts)
}

/// Drops any statement at-rules from the front of a block's prelude.
///
/// Blocks are found by scanning for the next `{`, so a statement at-rule --
/// `@charset`, `@import`, or the `@layer a, b, c;` that names a layer order --
/// gets swallowed into the prelude of whatever block follows it. firefox.com
/// writes its layer order immediately before the legacy fallback:
/// `@layer base, theme, ...;@supports not (all: revert-layer){...}`. Read whole,
/// that prelude starts with `@layer`, so the `@supports` test never ran and an
/// entire second copy of the base stylesheet was applied over the real one.
fn split_statement_prelude(prelude: &str) -> (&str, &str) {
    let mut depth = 0_i32;
    let mut in_string: Option<char> = None;
    let mut escaped = false;
    let mut last_semicolon = None;
    for (i, ch) in prelude.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            q @ ('"' | '\'') if in_string.is_none() => in_string = Some(q),
            q if in_string == Some(q) => in_string = None,
            _ if in_string.is_some() => {}
            '(' => depth += 1,
            ')' => depth -= 1,
            ';' if depth == 0 => last_semicolon = Some(i),
            _ => {}
        }
    }
    match last_semicolon {
        Some(i) => (&prelude[..i], prelude[i + 1..].trim()),
        None => ("", prelude),
    }
}

pub fn parse_inline_declarations(input: &str) -> Vec<Declaration> {
    let stripped = strip_comments(input);
    split_at_top_level(&stripped, ';')
        .into_iter()
        .filter_map(|entry| {
            let (property, value) = entry.split_once(':')?;
            let property = to_physical_property(property.trim().to_ascii_lowercase());
            let (value, important) = split_important(value);
            if property.is_empty() || value.is_empty() {
                return None;
            }
            Some(Declaration {
                property,
                value,
                important,
            })
        })
        .collect()
}

thread_local! {
    /// Every distinct `ComputedStyle` currently reachable from a styled tree,
    /// held once. Layout is single-threaded, so a thread-local avoids threading
    /// an interner argument through the recursive builders.
    static STYLE_INTERNER: RefCell<HashSet<Arc<ComputedStyle>>> =
        RefCell::new(HashSet::new());
}

/// Return the shared handle for `style`, allocating only the first time a given
/// value is seen. `Arc<ComputedStyle>` borrows as `ComputedStyle`, so a repeat
/// lookup costs a hash and no allocation. `Arc` rather than `Rc` because the
/// finished tree is handed to the render worker thread.
fn intern_style(style: ComputedStyle) -> Arc<ComputedStyle> {
    STYLE_INTERNER.with(|cell| {
        let mut set = cell.borrow_mut();
        if let Some(existing) = set.get(&style) {
            return Arc::clone(existing);
        }
        let shared = Arc::new(style);
        set.insert(Arc::clone(&shared));
        shared
    })
}

/// Drop interned styles that no live tree references any more. Called when a
/// tree is rebuilt so the table tracks the current page rather than every style
/// ever computed in this process.
fn prune_style_interner() {
    STYLE_INTERNER.with(|cell| {
        cell.borrow_mut().retain(|shared| Arc::strong_count(shared) > 1);
    });
}

/// Number of distinct styles currently shared. Test-only introspection.
#[cfg(test)]
pub(crate) fn interned_style_count() -> usize {
    STYLE_INTERNER.with(|cell| cell.borrow().len())
}

/// Font size a document starts from, and the basis for `rem` until the root
/// element says otherwise.
pub const INITIAL_FONT_SIZE: u32 = 16;

thread_local! {
    /// Computed `font-size` of the root element -- what `rem` is relative to.
    static ROOT_FONT_SIZE: Cell<u32> = const { Cell::new(INITIAL_FONT_SIZE) };
}

fn root_font_size() -> u32 {
    ROOT_FONT_SIZE.with(|size| size.get())
}

/// Work out what `rem` means for this document, before anything is styled.
///
/// `rem` is relative to the *root element's* computed font size, not to a
/// constant. `html { font-size: 62.5% }` -- chosen so that `1.4rem` reads as
/// "14px" -- is one of the most widespread idioms in production CSS, and
/// resolving `rem` against a hardcoded 16px inflates every length on such a
/// page by 1.6x. On Yahoo! JAPAN that turned 12px navigation labels into 19px
/// ones, which is why the words in its tab bar were drawn on top of each other.
fn establish_root_font_size(
    document: &Node,
    stylesheet: &Stylesheet,
    viewport_width: u32,
    interactive: &InteractiveState,
) {
    ROOT_FONT_SIZE.with(|size| size.set(INITIAL_FONT_SIZE));
    let Some(root) = root_element(document) else {
        return;
    };
    // The root's own font size cannot itself depend on `rem` -- there is no
    // outer root -- so computing it against the initial 16px is well-defined.
    let style = compute_style(
        root,
        stylesheet,
        &stylesheet.rule_index,
        None,
        &[],
        0,
        1,
        &[],
        viewport_width,
        interactive,
    );
    ROOT_FONT_SIZE.with(|size| size.set(style.font_size_px));
}

/// The `<html>` element, however deeply the parser nested it.
fn root_element(node: &Node) -> Option<&Element> {
    match node {
        // Neither renders, and neither carries anything this walk wants.
        Node::Comment(_) | Node::Doctype(_) => Default::default(),
        Node::Element(element) if element.tag_name == "html" => Some(element),
        Node::Element(element) => element.children.iter().find_map(root_element),
        Node::Text(_) => None,
    }
}

pub fn build_styled_tree(
    document: &Node,
    stylesheet: &Stylesheet,
    viewport_width: u32,
    interactive: &InteractiveState,
) -> StyledNode {
    prune_style_interner();
    establish_root_font_size(document, stylesheet, viewport_width, interactive);
    let ancestors = Vec::new();
    let rule_index = &stylesheet.rule_index;
    build_node(
        document,
        stylesheet,
        &rule_index,
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

pub(crate) fn build_styled_tree_incremental(
    document: &Node,
    stylesheet: &Stylesheet,
    viewport_width: u32,
    interactive: &InteractiveState,
    old_styled: &StyledNode,
    old_node_order: &[u32],
    new_node_order: &[u32],
    dirty_roots: &HashSet<u32>,
) -> Option<StyledNode> {
    if new_node_order.is_empty() {
        return None;
    }
    if dirty_roots.contains(&new_node_order[0]) {
        return None;
    }
    establish_root_font_size(document, stylesheet, viewport_width, interactive);

    let mut old_map = HashMap::new();
    let mut old_iter = old_node_order.iter();
    collect_styled_node_map(old_styled, &mut old_iter, &mut old_map)?;
    if old_iter.next().is_some() {
        return None;
    }

    // Build a stable-id -> parent-stable-id map over the new tree, then derive
    // the "dirty spine": every dirty root plus all of its ancestors. A subtree
    // can be reused wholesale only when its root is NOT on the spine (i.e. no
    // dirty root lives anywhere inside it) and is not itself under a dirty root.
    // The stable ids are engine arena indices (sparse, since text/detached nodes
    // consume indices too) — never the browser pre-order data-tobira-node-id —
    // so the dirty comparisons must be done against these same arena ids.
    let mut parent_map: HashMap<u32, Option<u32>> = HashMap::new();
    let mut pm_iter = new_node_order.iter();
    build_parent_map(document, None, &mut pm_iter, &mut parent_map)?;
    if pm_iter.next().is_some() {
        return None;
    }
    let mut dirty_spine: HashSet<u32> = HashSet::new();
    for &root in dirty_roots {
        let mut cur = Some(root);
        while let Some(id) = cur {
            if !dirty_spine.insert(id) {
                break;
            }
            cur = parent_map.get(&id).copied().flatten();
        }
    }

    let mut new_iter = new_node_order.iter();
    let mut result = build_node_incremental(
        document,
        stylesheet,
        &stylesheet.rule_index,
        None,
        &[],
        0,
        0,
        &[],
        None,
        viewport_width,
        interactive,
        &mut new_iter,
        &old_map,
        dirty_roots,
        &dirty_spine,
        false,
    )?;
    if new_iter.next().is_some() {
        return None;
    }
    // Reused subtrees carry the previous tree's `data-tobira-node-id` values,
    // which shift whenever a structural change inserts or removes a node. Re-stamp
    // them in pre-order (matching browser::annotate_node_ids) so every element's
    // id reflects its new position — a full rebuild would assign exactly these.
    let mut counter = 0usize;
    restamp_tobira_node_ids(&mut result, &mut counter);
    Some(result)
}

/// Re-number `data-tobira-node-id` on every styled element in pre-order, 1-based,
/// reproducing `browser::annotate_node_ids` over the styled tree (pseudo-element
/// text nodes are skipped, exactly as they are absent from the source document).
fn restamp_tobira_node_ids(node: &mut StyledNode, counter: &mut usize) {
    if let StyledNode::Element(element) = node {
        *counter += 1;
        element
            .attributes
            .insert("data-tobira-node-id".to_string(), counter.to_string());
        for child in &mut element.children {
            restamp_tobira_node_ids(child, counter);
        }
    }
}

fn build_node(
    node: &Node,
    stylesheet: &Stylesheet,
    rule_index: &RuleIndex,
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
        // Neither renders. They reach here only because a caller mapped over
        // every child; an empty text node is how the styled tree holds nothing.
        Node::Comment(_) | Node::Doctype(_) => StyledNode::Text(StyledText {
            text: String::new(),
            style: intern_style(
                parent_style
                    .cloned()
                    .unwrap_or_else(|| ComputedStyle::for_element("body", None)),
            ),
        }),
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
                style: intern_style(style),
            })
        }
        Node::Element(element) => {
            let style = compute_style(
                element,
                stylesheet,
                rule_index,
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
                children: empty_siblings_rc(),
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
                        rule_index,
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
                children.insert(0, pseudo_node(before_text, pseudo_style));
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
                children.push(pseudo_node(after_text, pseudo_style));
            }

            StyledNode::Element(StyledElement {
                tag_name: element.tag_name.clone(),
                attributes: element.attributes.clone(),
                style: intern_style(style),
                children,
            })
        }
    }
}

fn collect_styled_node_map<'a>(
    node: &'a StyledNode,
    node_order: &mut std::slice::Iter<'_, u32>,
    map: &mut HashMap<u32, &'a StyledNode>,
) -> Option<()> {
    match node {
        StyledNode::Text(_) => Some(()),
        StyledNode::Element(element) => {
            let id = *node_order.next()?;
            map.insert(id, node);
            for child in &element.children {
                collect_styled_node_map(child, node_order, map)?;
            }
            Some(())
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn build_node_incremental(
    node: &Node,
    stylesheet: &Stylesheet,
    rule_index: &RuleIndex,
    parent_style: Option<&ComputedStyle>,
    ancestors: &[AncestorSlot],
    sibling_index: usize,
    sibling_count: usize,
    preceding_siblings: &[ElementIdentity],
    parent_all_sibling_ids: Option<Rc<[ElementIdentity]>>,
    viewport_width: u32,
    interactive: &InteractiveState,
    new_node_order: &mut std::slice::Iter<'_, u32>,
    old_map: &HashMap<u32, &StyledNode>,
    dirty_roots: &HashSet<u32>,
    dirty_spine: &HashSet<u32>,
    under_dirty: bool,
) -> Option<StyledNode> {
    match node {
        // Neither renders, and neither carries anything this walk wants.
        Node::Comment(_) | Node::Doctype(_) => Default::default(),
        Node::Text(text) => {
            let mut style = parent_style
                .cloned()
                .unwrap_or_else(|| ComputedStyle::for_element("body", None));
            if let Some(parent) = parent_style {
                let parent_is_block = !matches!(parent.display, Display::Inline);
                if parent.opacity < 255 && parent_is_block {
                    style.effective_opacity = 255;
                }
            }
            Some(StyledNode::Text(StyledText {
                text: text.clone(),
                style: intern_style(style),
            }))
        }
        Node::Element(element) => {
            let id = *new_node_order.next()?;
            // `under_dirty` (a dirty root is at or above this node) propagates
            // down; `dirty_spine` membership means a dirty root lives somewhere
            // inside this node's subtree. Either disqualifies wholesale reuse.
            let under_dirty = under_dirty || dirty_roots.contains(&id);
            let subtree_has_dirty = dirty_spine.contains(&id);
            let reuse = !under_dirty && !subtree_has_dirty && old_map.contains_key(&id);
            if reuse {
                // The whole subtree is reused unchanged, but its descendants'
                // stable ids must still be consumed from the iterator to keep it
                // aligned with the document walk.
                for child in &element.children {
                    skip_element_ids(child, new_node_order)?;
                }
                return old_map.get(&id).cloned().cloned();
            }

            let style = compute_style(
                element,
                stylesheet,
                rule_index,
                parent_style,
                ancestors,
                sibling_index,
                sibling_count,
                preceding_siblings,
                viewport_width,
                interactive,
            );
            let all_sibling_ids: Rc<[ElementIdentity]> = element
                .children
                .iter()
                .filter_map(|c| if let Node::Element(e) = c { Some(ElementIdentity::from(e)) } else { None })
                .collect::<Vec<_>>()
                .into();
            let child_element_count = all_sibling_ids.len();
            let current_slot = AncestorSlot {
                element: ElementIdentity::from(element),
                sibling_index,
                sibling_count,
                siblings: parent_all_sibling_ids.unwrap_or_else(|| Rc::from(preceding_siblings)),
                prec_count: sibling_index,
                children: empty_siblings_rc(),
            };
            let mut next_ancestors = ancestors.to_vec();
            next_ancestors.push(current_slot);

            let mut elem_sibling_idx = 0;
            let mut children = Vec::with_capacity(element.children.len());
            for child in &element.children {
                let (idx, count, prec_snap) = if matches!(child, Node::Element(_)) {
                    let idx = elem_sibling_idx;
                    elem_sibling_idx += 1;
                    (idx, child_element_count, &all_sibling_ids[..idx])
                } else {
                    (0, 0, &all_sibling_ids[..0])
                };
                children.push(build_node_incremental(
                    child,
                    stylesheet,
                    rule_index,
                    Some(&style),
                    &next_ancestors,
                    idx,
                    count,
                    prec_snap,
                    Some(all_sibling_ids.clone()),
                    viewport_width,
                    interactive,
                    new_node_order,
                    old_map,
                    dirty_roots,
                    dirty_spine,
                    under_dirty,
                )?);
            }

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
                children.insert(0, pseudo_node(before_text, pseudo_style));
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
                children.push(pseudo_node(after_text, pseudo_style));
            }

            Some(StyledNode::Element(StyledElement {
                tag_name: element.tag_name.clone(),
                attributes: element.attributes.clone(),
                style: intern_style(style),
                children,
            }))
        }
    }
}

/// Walk the document in the same Element-only pre-order as `node_order` and
/// record each element's stable id -> parent stable id. Returns None if the
/// document and `node_order` disagree on element count (the iterator runs dry).
fn build_parent_map(
    node: &Node,
    parent_id: Option<u32>,
    node_order: &mut std::slice::Iter<'_, u32>,
    out: &mut HashMap<u32, Option<u32>>,
) -> Option<()> {
    match node {
        // Neither renders, and neither carries anything this walk wants.
        Node::Comment(_) | Node::Doctype(_) => Default::default(),
        Node::Text(_) => Some(()),
        Node::Element(element) => {
            let id = *node_order.next()?;
            out.insert(id, parent_id);
            for child in &element.children {
                build_parent_map(child, Some(id), node_order, out)?;
            }
            Some(())
        }
    }
}

/// Advance `node_order` past every element id in `node`'s subtree (including
/// `node` itself). Used when a clean subtree is reused wholesale so the shared
/// iterator stays aligned with the document walk.
fn skip_element_ids(node: &Node, node_order: &mut std::slice::Iter<'_, u32>) -> Option<()> {
    match node {
        // Neither renders, and neither carries anything this walk wants.
        Node::Comment(_) | Node::Doctype(_) => Default::default(),
        Node::Text(_) => Some(()),
        Node::Element(element) => {
            node_order.next()?;
            for child in &element.children {
                skip_element_ids(child, node_order)?;
            }
            Some(())
        }
    }
}

/// Strip a matched pair of surrounding CSS string quotes (`"..."` or `'...'`).
/// Only removes quotes when the same quote character opens and closes the string.
/// Unbalanced quotes (e.g. `"foo'`) are left intact.
/// Parse a `content` value into the text it actually renders.
///
/// The value is a list -- quoted strings, `attr()`, `counter()`, `url()` -- and
/// it may end with `/ <string>`, which is *alternative text for speech* and is
/// never drawn. Stripping the outer quotes and keeping the rest wholesale drew
/// that alternative text as if it were content: Wikipedia writes its edit-link
/// brackets as `content: ']' / ''`, so every menu entry and every table-of-
/// contents row on the page ended with a stray `]' / '`.
///
/// Functions are skipped rather than guessed at -- there is no element in hand
/// to resolve `attr()` against, and no counter state here -- so a value made
/// only of them yields nothing, which is the same as having no content.
fn parse_content(value: &str) -> Option<String> {
    parse_content_for(value, None)
}

fn parse_content_for(value: &str, element: Option<&Element>) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let lower = value.to_ascii_lowercase();
    if lower == "none" || lower == "normal" {
        return None;
    }

    let mut text = String::new();
    let mut chars = value.chars().peekable();
    let mut produced = false;

    while let Some(character) = chars.next() {
        match character {
            // Everything from here on is the speech alternative.
            '/' => break,
            '"' | '\'' => {
                produced = true;
                let quote = character;
                while let Some(inner) = chars.next() {
                    match inner {
                        c if c == quote => break,
                        '\\' => text.push(unescape_css_char(&mut chars)),
                        c => text.push(c),
                    }
                }
            }
            c if c.is_whitespace() || c == ',' => {}
            _ => {
                // A keyword or a function: consume it, balancing parentheses.
                let mut token = String::from(character);
                let mut depth = 0usize;
                let mut current = character;
                loop {
                    match current {
                        '(' => depth += 1,
                        ')' => depth = depth.saturating_sub(1),
                        c if depth == 0 && (c.is_whitespace() || c == ',') => break,
                        c if depth == 0 && c == '/' => break,
                        _ => {}
                    }
                    match chars.next() {
                        Some(next) => {
                            current = next;
                            token.push(next);
                        }
                        None => break,
                    }
                }
                // `attr()` is the one function resolvable here, and only when
                // the originating element is at hand.
                if let Some(element) = element
                    && let Some(name) = token
                        .trim_end_matches(|c: char| c.is_whitespace() || c == ',' || c == '/')
                        .strip_prefix("attr(")
                        .and_then(|rest| rest.strip_suffix(')'))
                {
                    produced = true;
                    let name = name.trim().trim_matches(|c| c == '"' || c == '\'');
                    text.push_str(element.attribute(name).unwrap_or(""));
                }
                if current == '/' && depth == 0 {
                    break;
                }
            }
        }
    }

    if !produced && text.is_empty() {
        return None;
    }
    Some(text)
}

/// Resolve one CSS escape sequence, the backslash already consumed.
///
/// `\a0` is a non-breaking space and `\2019` a right quote; stylesheets write
/// separators and punctuation this way rather than embedding the characters.
fn unescape_css_char(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> char {
    let mut hex = String::new();
    while hex.len() < 6 {
        match chars.peek() {
            Some(c) if c.is_ascii_hexdigit() => {
                hex.push(*c);
                chars.next();
            }
            _ => break,
        }
    }
    if hex.is_empty() {
        // A backslash before anything else escapes that character itself.
        return chars.next().unwrap_or('\\');
    }
    // One optional whitespace terminates the escape and is not part of the text.
    if chars.peek().is_some_and(|c| c.is_whitespace()) {
        chars.next();
    }
    u32::from_str_radix(&hex, 16)
        .ok()
        .and_then(char::from_u32)
        .unwrap_or('\u{fffd}')
}

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
/// Turn a pseudo-element's content and style into a node.
///
/// A `::before` is normally just text, and a text node is all it needs. But a
/// pseudo-element is a box like any other, and pages lean on that: firefox.com
/// draws the fox curling under its hero entirely through
/// `.fl-home-intro::before` -- `content: ""`, a background image, an
/// aspect-ratio and an inset. As a text node it has nothing to paint and the
/// illustration simply was not there.
///
/// Only a pseudo carrying a picture becomes an element. Out-of-flow and
/// background-bearing boxes are exactly the decorative case, and leaving the
/// text-only ones alone keeps quotes, bullets and icon glyphs laid out the way
/// they already were.
fn pseudo_node(text: String, style: ComputedStyle) -> StyledNode {
    let is_decorative = style.background_image_url.is_some() || style.background_gradient.is_some();
    if !is_decorative {
        return StyledNode::Text(StyledText { text, style: intern_style(style) });
    }
    let children = if text.is_empty() {
        Vec::new()
    } else {
        vec![StyledNode::Text(StyledText { text, style: intern_style(style.clone()) })]
    };
    StyledNode::Element(StyledElement {
        // Not a real tag: nothing may match it as one, and it reads for what it
        // is in a dump.
        tag_name: "::pseudo".to_string(),
        attributes: BTreeMap::new(),
        style: intern_style(style),
        children,
    })
}

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
    // A pseudo-element inherits from its host, which is not the same as being a
    // copy of it: `display`, `position`, `background`, the box edges and the
    // rest do not inherit. Cloned outright, a `::before` on a flex container
    // came out `display: flex` itself, and firefox.com's background gradient --
    // a `::before` on a flex row -- took a paint path that never drew it.
    //
    // Built as an anonymous inline with the host as its parent, so exactly the
    // inheritable properties come through.
    let mut pseudo_style = ComputedStyle::for_element("span", Some(host_style));
    let mut content_text: Option<String> = None;
    // A pseudo-element resolves `var()` against the same set its host does:
    // the document's `:root` block, whatever a matching `@media` adds to it,
    // then anything declared on the host itself. Seeded from the host alone,
    // a `:root` variable was simply unknown -- and firefox.com gates the
    // gradient that washes its lower half in purple behind
    // `content: var(--content-dark-mode-only)`, so the whole thing was skipped.
    let mut pseudo_vars: BTreeMap<String, String> = (*stylesheet.root_vars).clone();
    for (condition, vars) in &stylesheet.media_root_vars {
        if condition.matches(viewport_width) {
            for (name, value) in vars {
                pseudo_vars.insert(name.clone(), value.clone());
            }
        }
    }
    if let Some(declared) = host_style.custom_properties.as_deref() {
        for (name, value) in declared {
            pseudo_vars.insert(name.clone(), value.clone());
        }
    }

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
                    &empty_siblings_rc(),
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
            // A pseudo-element sees the custom properties of the element it
            // hangs off, plus any it declares itself. Applied raw, every
            // `var()` in a `::before` rule was taken literally and dropped:
            // firefox.com sizes its hero fox with
            // `inline-size: var(--banner-kit-width, 580px)`, so the box came out
            // with no width at all and nothing was drawn.
            if let Some(name) = decl.property.strip_prefix("--") {
                let _ = name;
                pseudo_vars.insert(decl.property.clone(), decl.value.clone());
                continue;
            }
            let value = if decl.value.contains("var(") {
                let Some(substituted) = substitute_vars(&decl.value, &pseudo_vars) else {
                    // Guaranteed-invalid; leave the property as it stands
                    // rather than apply something the author never wrote.
                    continue;
                };
                std::borrow::Cow::Owned(substituted)
            } else {
                std::borrow::Cow::Borrowed(&decl.value)
            };
            if decl.property == "content" {
                content_text = parse_content_for(&value, Some(element));
            } else {
                // Use host_style.font_size_px so em/% units in pseudo-element rules
                // resolve against the originating element's font size (not a hardcoded 16px).
                let resolved = Declaration {
                    property: decl.property.clone(),
                    value: value.into_owned(),
                    important: decl.important,
                };
                apply_declaration(&mut pseudo_style, &resolved, host_style.font_size_px);
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
            if !cond.matches(viewport_width) { continue; }
        }
        let host_matches = rule.selectors.iter().any(|sel| {
            sel.pseudo_element.as_ref() == Some(&PseudoElement::Placeholder)
                && sel.matches(
                    &identity,
                    ancestors,
                    0,
                    1,
                    &[],
                    &empty_siblings_rc(),
                    &InteractiveState::default(),
                )
        });
        if !host_matches { continue; }
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
    rule_index: &RuleIndex,
    parent_style: Option<&ComputedStyle>,
    ancestors: &[AncestorSlot],
    sibling_index: usize,
    sibling_count: usize,
    preceding_siblings: &[ElementIdentity],
    viewport_width: u32,
    interactive: &InteractiveState,
) -> ComputedStyle {
    compute_style_with_rules(
        element,
        stylesheet,
        rule_index.candidates_for(&ElementIdentity::from(element)),
        parent_style,
        ancestors,
        sibling_index,
        sibling_count,
        preceding_siblings,
        viewport_width,
        interactive,
    )
}

fn compute_style_with_rules(
    element: &Element,
    stylesheet: &Stylesheet,
    candidate_rule_indices: Vec<usize>,
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
    // `:has()` is the only selector that looks downwards, and it is rare, so the
    // children are only gathered when some rule in the document asks for them.
    let child_identities: Rc<[ElementIdentity]> = if stylesheet.uses_has {
        element
            .children
            .iter()
            .filter_map(|child| match child {
                // Neither renders, and neither carries anything this walk wants.
                Node::Comment(_) | Node::Doctype(_) => Default::default(),
                Node::Element(child) => Some(ElementIdentity::from(child)),
                Node::Text(_) => None,
            })
            .collect::<Vec<_>>()
            .into()
    } else {
        empty_siblings_rc()
    };
    // O(1) ref bump — we avoid cloning the full BTreeMap unless this element has its own vars.
    let root_vars = Rc::clone(&stylesheet.root_vars);
    // What this element itself declares. Ancestors' declarations already ride
    // along on the style courtesy of `for_element`.
    let inherited_vars = style.custom_properties.clone();
    let mut element_vars: BTreeMap<String, String> = BTreeMap::new();

    // Root variables from a matching `@media` block. Every element sees these,
    // so they are a lookup fallback rather than part of this element's own set.
    let mut media_vars: BTreeMap<String, String> = BTreeMap::new();
    for (cond, vars) in &stylesheet.media_root_vars {
        if cond.matches(viewport_width) {
            // CSS cascade: last declaration wins, so use insert (not or_insert_with).
            // A later matching @media block should override an earlier one for the same var.
            for (k, v) in vars {
                media_vars.insert(k.clone(), v.clone());
            }
        }
    }
    // (important, layer rank, specificity, source order, declaration)
    let mut applicable: Vec<(bool, u32, usize, usize, Declaration)> = Vec::new();

    for rule_index in candidate_rule_indices {
        let rule = &stylesheet.rules[rule_index];
        // Skip rules where ALL selectors are pseudo-element rules — they are handled by collect_pseudo_content
        if rule.selectors.iter().all(|sel| sel.pseudo_element.is_some()) {
            continue;
        }
        // Check media condition
        if let Some(cond) = &rule.media {
            if !cond.matches(viewport_width) {
                continue;
            }
        }
        // A selector list is scored by the *most specific* selector in it that
        // matches, not the first one written. firefox.com writes
        // `.fl-home-intro .fl-intro, .fl-home-intro .fl-intro:first-child` and
        // relies on the `:first-child` half's extra weight to hold its hero
        // padding against a later rule; scored by the first half instead, the
        // two tied and the later rule won, putting 128px under the hero where
        // 64px belongs.
        let mut matched: Option<(usize, &Selector)> = None;
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
                &child_identities,
                interactive,
            ) {
                let score = selector.specificity();
                if matched.is_none_or(|(best, _)| score > best) {
                    matched = Some((score, selector));
                }
            }
        }
        {
            if let Some((specificity, _selector)) = matched {
                // First pass: collect CSS variables
                for decl in &rule.declarations {
                    if decl.property.starts_with("--") {
                        element_vars.insert(decl.property.clone(), decl.value.clone());
                    }
                }
                let rank = stylesheet.layer_rank(rule.layer.as_ref());
                applicable.extend(rule.declarations.iter().cloned().enumerate().map(
                    |(declaration_index, declaration)| {
                        (
                            declaration.important,
                            // Layers rank ahead of specificity, and `!important`
                            // turns the ordering upside down: an important
                            // declaration in an early layer beats a later one,
                            // and an unlayered important is the weakest of all.
                            if declaration.important {
                                u32::MAX - rank
                            } else {
                                rank
                            },
                            specificity,
                            rule_index * 100 + declaration_index,
                            declaration,
                        )
                    },
                ));
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
                .map(|(index, declaration)| {
                    // A style attribute is not in any layer and outranks every
                    // layered declaration of the same importance, so it takes
                    // the top rank whether or not it is important.
                    (
                        declaration.important,
                        u32::MAX,
                        1_000,
                        usize::MAX - 1_000 + index,
                        declaration,
                    )
                }),
        );
    }

    applicable.sort_by_key(|(important, layer, specificity, order, _)| {
        (*important, *layer, *specificity, *order)
    });

    // Publish this element's own declarations on top of what it inherited, so
    // its descendants see them too.
    if !element_vars.is_empty() {
        let merged = match &inherited_vars {
            Some(inherited) => {
                let mut merged = (**inherited).clone();
                merged.extend(element_vars.iter().map(|(k, v)| (k.clone(), v.clone())));
                merged
            }
            None => element_vars.clone(),
        };
        style.custom_properties = Some(Arc::new(merged));
    }

    // Resolution order for `var()`: this element and its ancestors, then the
    // `@media` root set, then the plain `:root` set. Only build the merged map
    // when something actually asks for a variable -- the fallbacks are large on
    // real pages and copying them per element would cost more than it saves.
    // Cloned (an Arc bump), not borrowed: `apply_declaration` needs `style` mutably.
    let declared_vars = style.custom_properties.clone();
    let needs_vars = applicable
        .iter()
        .any(|(_, _, _, _, declaration)| declaration.value.contains("var("));
    let merged_lookup: Option<BTreeMap<String, String>> = if !needs_vars {
        None
    } else {
        match declared_vars.as_deref() {
            Some(declared) if !media_vars.is_empty() || !root_vars.is_empty() => {
                let mut merged = declared.clone();
                for (k, v) in media_vars.iter().chain(root_vars.iter()) {
                    merged.entry(k.clone()).or_insert_with(|| v.clone());
                }
                Some(merged)
            }
            None if !media_vars.is_empty() => {
                let mut merged = media_vars.clone();
                for (k, v) in root_vars.iter() {
                    merged.entry(k.clone()).or_insert_with(|| v.clone());
                }
                Some(merged)
            }
            _ => None,
        }
    };
    let vars_ref: &BTreeMap<String, String> = match (&merged_lookup, declared_vars.as_deref()) {
        (Some(merged), _) => merged,
        (None, Some(declared)) => declared,
        (None, None) => &*root_vars,
    };

    // Font size first, whatever order the sheet wrote it in. Every other
    // property measures `em` against the element's own font size -- `5em` on an
    // element at 20px is 100px, not 5 times whatever the parent was set to --
    // so the size has to be settled before the rest are read.
    let mut applicable: Vec<_> = applicable.into_iter().collect();
    applicable.sort_by_key(|(_, _, _, _, declaration)| {
        u8::from(!matches!(declaration.property.as_str(), "font-size" | "font"))
    });

    for (_, _, _, _, mut declaration) in applicable {
        // skip CSS custom properties
        if declaration.property.starts_with("--") {
            continue;
        }
        // What a declaration finally became is the one thing worth seeing when a
        // page's colours or sizes come out wrong, and it is invisible from both
        // the stylesheet and the screen. Set TOBIRA_DEBUG_DECL to a property name
        // to follow it: the raw value, what var() substitution made of it, or
        // that it was dropped.
        let traced = debug_traced_property().is_some_and(|want| declaration.property == want);
        let raw_value = if traced {
            Some(declaration.value.clone())
        } else {
            None
        };

        // substitute var() references
        if declaration.value.contains("var(") {
            let Some(substituted) = substitute_vars(&declaration.value, vars_ref) else {
                if traced {
                    eprintln!(
                        "decl <{}> class={:?} {}: {:?} -> DROPPED (unresolvable var)",
                        element.tag_name,
                        element.attribute("class").map(|c| c.split_whitespace().collect::<Vec<_>>().join(".")),
                        declaration.property,
                        raw_value.unwrap_or_default(),
                    );
                }
                // Guaranteed-invalid: drop it rather than apply a value the
                // author never wrote.
                continue;
            };
            declaration.value = substituted;
        }
        if traced {
            eprintln!(
                "decl <{}> class={:?} {}: {:?} -> {:?}",
                element.tag_name,
                element.attribute("class").map(|c| c.split_whitespace().collect::<Vec<_>>().join(".")),
                declaration.property,
                raw_value.unwrap_or_default(),
                declaration.value,
            );
        }
        // After substitution, because the light and dark arms are usually
        // `var()` references themselves.
        if declaration.value.contains("light-dark(") {
            declaration.value = resolve_light_dark(&declaration.value);
        }
        // `font-size` itself is relative to the parent; everything else is
        // relative to what this element ended up with.
        let em_basis = if matches!(declaration.property.as_str(), "font-size" | "font") {
            parent_font_size
        } else {
            style.font_size_px
        };
        apply_declaration(&mut style, &declaration, em_basis);
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

    // Anti-FOUC guard: pages routinely hide <html>/<body> (via `opacity: 0` or
    // `visibility: hidden`, which we fold into opacity) and then reveal it with a
    // JS onload handler or a CSS fade-in animation. We run neither reliably, so
    // honoring opacity:0 on the root leaves the entire page invisible — which is
    // exactly the "blank page" failure on Google/YouTube. A root element is never
    // meant to be permanently transparent, so clamp it back to fully opaque.
    if style.opacity == 0
        && (element.tag_name.eq_ignore_ascii_case("body")
            || element.tag_name.eq_ignore_ascii_case("html"))
    {
        style.opacity = 255;
        style.effective_opacity = 255;
    }

    blockify(&mut style, parent_style);

    style
}

/// Some contexts force a box to be block-level whatever `display` asked for
/// (CSS Display 3, "blockification"). Two of them apply here: leaving the flow,
/// and being a flex or grid item.
///
/// An absolutely positioned or floated `<span>` is a block box, not an inline
/// one, and that matters beyond boxing: only the block path clips
/// `overflow: hidden`. The visually-hidden idiom
/// `position:absolute; width:1px; height:1px; overflow:hidden` is on almost
/// every real page to hide text from sight but not from screen readers — left
/// inline it was never clipped, so the text rendered in a 1px column, one
/// character per line.
///
/// The item half of the rule carries the image-replacement idiom. firefox.com
/// heads its pages with an `<a>` flex item that draws the logo as a background
/// and holds a real `<img>` for anyone not seeing it; `overflow: hidden` plus
/// `text-indent: -9999px` is what pushes that `<img>` out of sight. Both of
/// those only apply to a block container, so while the `<a>` stayed inline the
/// two logos painted one on top of the other.
fn blockify(style: &mut ComputedStyle, parent_style: Option<&ComputedStyle>) {
    let out_of_flow = matches!(style.position, Position::Absolute | Position::Fixed)
        || !matches!(style.float, FloatSide::None);
    let is_item = parent_style.is_some_and(|parent| {
        matches!(
            parent.display,
            Display::Flex | Display::InlineFlex | Display::Grid | Display::InlineGrid
        )
    });
    if !out_of_flow && !is_item {
        return;
    }
    style.display = match style.display {
        Display::Inline | Display::InlineBlock | Display::ListItem => Display::Block,
        Display::InlineFlex => Display::Flex,
        Display::InlineGrid => Display::Grid,
        // `none` stays hidden; everything else is already block-level.
        other => other,
    };
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn compute_style_naive(
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
    compute_style_with_rules(
        element,
        stylesheet,
        (0..stylesheet.rules.len()).collect(),
        parent_style,
        ancestors,
        sibling_index,
        sibling_count,
        preceding_siblings,
        viewport_width,
        interactive,
    )
}

/// Resolve `light-dark(<light>, <dark>)` to the value for the light scheme.
///
/// This engine renders light: `prefers-color-scheme: dark` never matches, so the
/// first argument is always the one that applies. MDN defines nearly every colour
/// token with it -- 49 of them on a docs page -- and leaving the function
/// unparsed left those declarations unreadable. `background-color` then went
/// unset, so the sticky header was transparent and the article scrolled visibly
/// through it.
fn resolve_light_dark(value: &str) -> String {
    let mut result = value.to_string();
    let mut guard = 0;
    while let Some(start) = result.find("light-dark(") {
        guard += 1;
        if guard > 10 {
            break;
        }
        let args_start = start + "light-dark(".len();
        let mut depth = 0usize;
        let mut end = None;
        for (offset, character) in result[args_start..].char_indices() {
            match character {
                '(' => depth += 1,
                ')' => {
                    if depth == 0 {
                        end = Some(args_start + offset);
                        break;
                    }
                    depth -= 1;
                }
                _ => {}
            }
        }
        let Some(end) = end else {
            break;
        };
        let light = split_at_top_level(&result[args_start..end], ',')
            .first()
            .map(|argument| argument.trim().to_string())
            .unwrap_or_default();
        result = format!("{}{}{}", &result[..start], light, &result[end + 1..]);
    }
    result
}

/// Substitute `var()` references.
///
/// Returns `None` when a reference names a custom property that does not exist
/// and gives no fallback. The spec calls the result a *guaranteed-invalid value*
/// and drops the whole declaration; substituting nothing instead quietly turns it
/// into a different, valid declaration.
///
/// That distinction decides which palette MDN renders in. Its colours go through
///
///   html[data-theme=light] { --csstools-color-scheme--light: initial }
///   --toggle: var(--csstools-color-scheme--light) var(--color-gray-05);
///   --color-background-page: var(--toggle, var(--color-white))
///
/// With no `data-theme` attribute set, the scheme variable is undefined, so
/// `--toggle` is guaranteed-invalid and the page falls back to white. Treating
/// the missing reference as empty made `--toggle` resolve to the dark grey
/// instead, and the whole site came out in its dark palette.
/// Does this resolved custom-property value contain a CSS-wide keyword?
///
/// `initial` (and its siblings) cannot appear part-way through a value, so a
/// custom property that resolves to one is guaranteed-invalid and any `var()`
/// naming it takes its fallback instead.
///
/// This is not a corner case: it is the whole mechanism behind the light/dark
/// toggle csstools generates, which is how MDN ships every colour.
///
///   html[data-theme=light] { --scheme-light: initial }
///   --toggle: var(--scheme-light) var(--dark);   /* -> "initial <dark>" */
///   --page:   var(--toggle, var(--light))        /* -> takes --light */
///
/// Without this the toggle resolved to the literal text `initial #18191b`,
/// which parses as no colour at all -- so MDN's sticky header had no background
/// and the article scrolled visibly through it.
fn holds_css_wide_keyword(value: &str) -> bool {
    value
        .split(|c: char| c.is_whitespace() || c == ',' || c == '(' || c == ')')
        .any(|token| {
            matches!(
                token.trim().to_ascii_lowercase().as_str(),
                "initial" | "inherit" | "unset" | "revert" | "revert-layer"
            )
        })
}

/// The property name `TOBIRA_DEBUG_DECL` asks to be traced, if any.
fn debug_traced_property() -> Option<&'static str> {
    static WANTED: OnceLock<Option<String>> = OnceLock::new();
    WANTED
        .get_or_init(|| std::env::var("TOBIRA_DEBUG_DECL").ok())
        .as_deref()
}

fn substitute_vars(value: &str, vars: &BTreeMap<String, String>) -> Option<String> {
    substitute_vars_at(value, vars, 0)
}

/// Resolve every `var()` in `value`, one reference at a time.
///
/// Each reference is resolved on its own so that an unresolvable one can fall
/// back instead of poisoning the whole value. A custom property whose *own*
/// value is guaranteed-invalid counts as absent, which is the rule the csstools
/// light/dark toggle is built on:
///
///   --toggle: var(--scheme) var(--dark);        /* --scheme is undefined */
///   --page:   var(--toggle, var(--light))       /* so this takes --light */
///
/// Resolving `--toggle` by pasting its raw text in and only then noticing the
/// missing `--scheme` would drop the `--page` declaration entirely; MDN would
/// lose its background colours instead of getting the light ones.
fn substitute_vars_at(
    value: &str,
    vars: &BTreeMap<String, String>,
    depth: usize,
) -> Option<String> {
    // Custom properties can reference each other; stop rather than spin.
    if depth > 16 {
        return None;
    }

    let mut result = String::with_capacity(value.len());
    let mut rest = value;

    while let Some(start) = rest.find("var(") {
        result.push_str(&rest[..start]);
        let inner_start = start + "var(".len();

        let mut nesting = 0usize;
        let mut close = None;
        for (offset, character) in rest[inner_start..].char_indices() {
            match character {
                '(' => nesting += 1,
                ')' => {
                    if nesting == 0 {
                        close = Some(inner_start + offset);
                        break;
                    }
                    nesting -= 1;
                }
                _ => {}
            }
        }
        let Some(close) = close else {
            // Unbalanced: keep what is left verbatim rather than guess.
            result.push_str(&rest[start..]);
            return Some(result);
        };

        let inner = &rest[inner_start..close];
        let (name, fallback) = match split_at_top_level(inner, ',').split_first() {
            Some((name, tail)) if !tail.is_empty() => {
                // The fallback is everything after the first comma, with the
                // separating space dropped.
                (name.trim().to_string(), Some(tail.join(",").trim().to_string()))
            }
            Some((name, _)) => (name.trim().to_string(), None),
            None => (String::new(), None),
        };

        let resolved = vars
            .get(&name)
            .and_then(|raw| substitute_vars_at(raw, vars, depth + 1))
            .filter(|text| !holds_css_wide_keyword(text));
        let replacement = match resolved {
            Some(replacement) => replacement,
            None => match fallback {
                Some(fallback) => substitute_vars_at(&fallback, vars, depth + 1)?,
                // Undefined, and nothing to fall back on: the value this sits in
                // is guaranteed-invalid and its declaration must be dropped.
                None => return None,
            },
        };
        result.push_str(&replacement);
        rest = &rest[close + 1..];
    }

    result.push_str(rest);
    Some(result)
}

fn split_important(value: &str) -> (String, bool) {
    let trimmed = value.trim();
    let lower = trimmed.to_ascii_lowercase();
    let mut idx = None;
    for (pos, ch) in trimmed.char_indices().rev() {
        if ch != '!' {
            continue;
        }
        let mut after = pos + ch.len_utf8();
        while after < trimmed.len() {
            let mut chars = trimmed[after..].chars();
            let Some(next) = chars.next() else {
                break;
            };
            if !next.is_whitespace() {
                break;
            }
            after += next.len_utf8();
        }
        if lower[after..].starts_with("important") {
            let after_keyword = after + "important".len();
            if trimmed[after_keyword..].trim().is_empty() {
                idx = Some(pos);
                break;
            }
        }
    }
    if let Some(idx) = idx {
        (trimmed[..idx].trim_end().to_string(), true)
    } else {
        (trimmed.to_string(), false)
    }
}

fn parse_filter_value(input: &str, style: &mut ComputedStyle) {
    let value = input.trim().to_ascii_lowercase();
    let mut rest = value.as_str();
    while !rest.is_empty() {
        rest = rest.trim_start();
        if rest.is_empty() { break; }

        if let Some(inner) = rest.strip_prefix("blur(") {
            if let Some(end) = inner.find(')') {
                let arg = &inner[..end];
                if let Some(px) = parse_length(arg.trim(), 16) {
                    style.filter_blur_px = px;
                }
                rest = &inner[end+1..];
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
                rest = &inner[end+1..];
                continue;
            }
        }
        if let Some(inner) = rest.strip_prefix("opacity(") {
            if let Some(end) = inner.find(')') {
                let arg = inner[..end].trim().trim_end_matches('%');
                let pct = arg.parse::<f32>().ok().unwrap_or(1.0);
                let factor = if pct <= 1.0 { pct } else { pct / 100.0 };
                style.filter_opacity = (factor.clamp(0.0, 1.0) * 255.0).round() as u8;
                rest = &inner[end+1..];
                continue;
            }
        }
        if let Some(inner) = rest.strip_prefix("grayscale(") {
            if let Some(end) = inner.find(')') {
                rest = &inner[end+1..];
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

/// Parse a CSS transform: value and accumulate into a ComputedStyle's transform fields.
/// Handles: none, translate(x,y), translateX(x), translateY(y),
///          scale(s) / scale(sx,sy), scaleX(sx), scaleY(sy),
///          rotate(Ndeg) / rotate(Nrad), skewX / skewY (ignored for now).
fn parse_transform_into(value: &str, style: &mut ComputedStyle) {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        style.transform_translate_x = 0;
        style.transform_translate_y = 0;
        style.transform_scale_x = 0;
        style.transform_scale_y = 0;
        style.transform_rotate_millideg = 0;
        return;
    }

    // Tokenise: split on ')' so each token is like "translateX(30px"
    for token in v.split(')') {
        let token = token.trim();
        if token.is_empty() { continue; }
        let (fname, args_str) = if let Some(p) = token.find('(') {
            (&token[..p], &token[p + 1..])
        } else {
            continue;
        };
        let fname = fname.trim().to_ascii_lowercase();
        // Parse comma- or space-separated arguments as f32
        let args: Vec<f32> = args_str
            .split(|c: char| c == ',' || c == ' ')
            .filter(|s| !s.is_empty())
            .filter_map(|s| parse_transform_length(s.trim()))
            .collect();

        match fname.as_str() {
            "translate" => {
                style.transform_translate_x += args.first().copied().unwrap_or(0.0).round() as i32;
                style.transform_translate_y += args.get(1).copied().unwrap_or(0.0).round() as i32;
            }
            "translatex" => {
                style.transform_translate_x += args.first().copied().unwrap_or(0.0).round() as i32;
            }
            "translatey" => {
                style.transform_translate_y += args.first().copied().unwrap_or(0.0).round() as i32;
            }
            "translate3d" => {
                style.transform_translate_x += args.first().copied().unwrap_or(0.0).round() as i32;
                style.transform_translate_y += args.get(1).copied().unwrap_or(0.0).round() as i32;
                // Z ignored
            }
            "scale" => {
                let sx = args.first().copied().unwrap_or(1.0);
                let sy = args.get(1).copied().unwrap_or(sx);
                // Accumulate by multiplying (convert millis → float → multiply → back)
                let prev_sx = if style.transform_scale_x == 0 { 1.0 } else { style.transform_scale_x as f32 / 1000.0 };
                let prev_sy = if style.transform_scale_y == 0 { 1.0 } else { style.transform_scale_y as f32 / 1000.0 };
                style.transform_scale_x = ((prev_sx * sx) * 1000.0).round() as u32;
                style.transform_scale_y = ((prev_sy * sy) * 1000.0).round() as u32;
            }
            "scalex" => {
                let sx = args.first().copied().unwrap_or(1.0);
                let prev = if style.transform_scale_x == 0 { 1.0 } else { style.transform_scale_x as f32 / 1000.0 };
                style.transform_scale_x = ((prev * sx) * 1000.0).round() as u32;
            }
            "scaley" => {
                let sy = args.first().copied().unwrap_or(1.0);
                let prev = if style.transform_scale_y == 0 { 1.0 } else { style.transform_scale_y as f32 / 1000.0 };
                style.transform_scale_y = ((prev * sy) * 1000.0).round() as u32;
            }
            "rotate" | "rotatez" => {
                style.transform_rotate_millideg += parse_transform_angle(args_str.trim());
            }
            // skew: ignore for now
            _ => {}
        }
    }
}

/// Parse a CSS length value to f32 pixels for transform arguments.
/// Handles: 42px, 3.5em (approximate as * 16px), 50% (returns 0 — % needs element size context).
fn parse_transform_length(s: &str) -> Option<f32> {
    if s.ends_with("px") {
        s[..s.len() - 2].trim().parse::<f32>().ok()
    } else if s.ends_with("rem") {
        s[..s.len() - 3].trim().parse::<f32>().ok().map(|v| v * 16.0)
    } else if s.ends_with("em") {
        s[..s.len() - 2].trim().parse::<f32>().ok().map(|v| v * 16.0)
    } else if s.ends_with('%') {
        // Can't resolve % without element size — return 0 (ignored)
        Some(0.0)
    } else {
        // Unitless (rare, typically for scale values like "1.5")
        s.parse::<f32>().ok()
    }
}

/// Parse a CSS angle string (from inside rotate(...)) to millidegrees.
/// Handles: 45deg, 3.14rad, 0.5turn, unitless (treated as deg).
fn parse_transform_angle(s: &str) -> i32 {
    if s.ends_with("deg") {
        s[..s.len() - 3].trim().parse::<f32>().ok()
            .map(|d| (d * 1000.0).round() as i32)
            .unwrap_or(0)
    } else if s.ends_with("grad") {
        s[..s.len() - 4].trim().parse::<f32>().ok()
            .map(|g| (g * 0.9 * 1000.0).round() as i32)
            .unwrap_or(0)
    } else if s.ends_with("rad") {
        s[..s.len() - 3].trim().parse::<f32>().ok()
            .map(|r| (r.to_degrees() * 1000.0).round() as i32)
            .unwrap_or(0)
    } else if s.ends_with("turn") {
        s[..s.len() - 4].trim().parse::<f32>().ok()
            .map(|t| (t * 360_000.0).round() as i32)
            .unwrap_or(0)
    } else {
        // unitless: treat as degrees
        s.parse::<f32>().ok()
            .map(|d| (d * 1000.0).round() as i32)
            .unwrap_or(0)
    }
}

/// Parse a `transform-origin` single component (e.g. "50%", "center", "left", "0px").
/// Returns permille (500 = 50% = center).
fn parse_transform_origin_pct(s: &str) -> u32 {
    match s.trim().to_ascii_lowercase().as_str() {
        "center" => 500,
        "left" | "top" => 0,
        "right" | "bottom" => 1000,
        other => {
            if other.ends_with('%') {
                other[..other.len() - 1].parse::<f32>().ok()
                    .map(|v| (v * 10.0).round() as u32)
                    .unwrap_or(500)
            } else if other.ends_with("px") {
                // pixel value — can't resolve without element size context, default to 0
                0
            } else {
                500 // fallback center
            }
        }
    }
}

/// Split a declaration value on whitespace, keeping bracketed groups whole.
///
/// `calc(a + b)` and `rgb(1, 2, 3)` contain spaces that are not separators.
fn split_value_components(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut depth = 0_usize;
    for character in value.chars() {
        match character {
            '(' | '[' => {
                depth += 1;
                current.push(character);
            }
            ')' | ']' => {
                depth = depth.saturating_sub(1);
                current.push(character);
            }
            c if c.is_whitespace() && depth == 0 => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

fn apply_declaration(style: &mut ComputedStyle, declaration: &Declaration, parent_font_size: u32) {
    let value = &declaration.value;
    match declaration.property.as_str() {
        // The two-value logical shorthands. `margin-inline: a b` sets the left
        // and right margins; given one value, both take it.
        "margin-inline" | "margin-block" | "padding-inline" | "padding-block"
        | "inset-inline" | "inset-block" => {
            // Split outside brackets: a plain `split_whitespace` tears
            // `calc(var(--kit-size) + var(--fl-section-v-padding)*2)` into four
            // meaningless words, and the whole declaration is then dropped.
            // firefox.com writes its pre-footer that way, so the 308px that
            // clears the floating download kit came out as 0.
            let parts = split_value_components(value);
            let Some(start_value) = parts.first() else {
                return;
            };
            let end_value = parts.get(1).unwrap_or(start_value);
            let (start, end) = match declaration.property.as_str() {
                "margin-inline" => ("margin-left", "margin-right"),
                "margin-block" => ("margin-top", "margin-bottom"),
                "padding-inline" => ("padding-left", "padding-right"),
                "padding-block" => ("padding-top", "padding-bottom"),
                "inset-inline" => ("left", "right"),
                _ => ("top", "bottom"),
            };
            for (property, value) in [(start, start_value), (end, end_value)] {
                apply_declaration(
                    style,
                    &Declaration {
                        property: property.to_string(),
                        value: value.clone(),
                        important: declaration.important,
                    },
                    parent_font_size,
                );
            }
        }
        "color" => {
            if let Some(color) = parse_color(value) {
                style.color = color;
            }
        }
        "background" => {
            let v = value.trim();
            let lowered = v.to_ascii_lowercase();
            if lowered.contains("linear-gradient(") || lowered.contains("radial-gradient(") {
                style.background_gradient = parse_linear_gradient(v);
            } else if v.to_ascii_lowercase().starts_with("url(") {
                style.background_image_url = extract_url(v);
            } else {
                style.background_color = parse_color(v);
            }
        }
        "background-color" => {
            // `currentColor` is whatever `color` is on this element, which is
            // how an icon drawn as a masked box takes the colour of the text
            // around it. Unread, the box had no colour and nothing was painted.
            style.background_color = if value.trim().eq_ignore_ascii_case("currentcolor") {
                Some(style.color)
            } else {
                parse_color(value)
            };
        }
        "background-image" => {
            let v = value.trim();
            let vl = v.to_ascii_lowercase();
            if vl == "none" {
                style.background_gradient = None;
                style.background_image_url = None;
            } else if vl.contains("linear-gradient(") || vl.contains("radial-gradient(") {
                style.background_gradient = parse_linear_gradient(v);
            } else if vl.starts_with("url(") {
                style.background_image_url = extract_url(v);
            }
        }
        // `mask` and its prefixed form carry the image among other components;
        // the url is the part that decides the shape.
        "mask-image" | "mask" | "-webkit-mask-image" | "-webkit-mask" => {
            let v = value.trim();
            if v.eq_ignore_ascii_case("none") {
                style.mask_image_url = None;
            } else if let Some(url) = find_url(v) {
                style.mask_image_url = Some(url);
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
                style.table_role = parse_table_role(value);
            }
        }
        "float" => {
            let v = value.trim().to_ascii_lowercase();
            style.float = match v.as_str() {
                "left" => FloatSide::Left,
                "right" => FloatSide::Right,
                "none" => FloatSide::None,
                _ => FloatSide::None,
            };
        }
        "clear" => {
            let v = value.trim().to_ascii_lowercase();
            style.clear = match v.as_str() {
                "left" => ClearSide::Left,
                "right" => ClearSide::Right,
                "both" => ClearSide::Both,
                "none" => ClearSide::None,
                _ => ClearSide::None,
            };
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
                style.max_width = parse_length_value(value, parent_font_size);
            }
        }
        "min-width" => {
            let v = value.trim().to_ascii_lowercase();
            if v == "auto" {
                style.min_width = None;
            } else {
                style.min_width = parse_length_value(value, parent_font_size);
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
        // `text-wrap` also carries `nowrap`, which `white-space` already
        // covers, and `stable`, which is about reflow while editing.
        "text-wrap" | "text-wrap-style" => {
            let v = value.trim().to_ascii_lowercase();
            style.text_wrap_balance = v == "balance";
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
                style.line_through = false;
            } else {
                if v.contains("underline") {
                    style.underline = true;
                }
                if v.contains("line-through") {
                    style.line_through = true;
                }
            }
        }
        "text-decoration-color" => {
            style.text_decoration_color = parse_color(value);
        }
        "text-transform" => {
            style.text_transform = parse_text_transform(value);
        }
        "text-indent" => {
            style.text_indent = parse_length_signed(value, parent_font_size).unwrap_or(0);
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
        "overflow-wrap" | "word-wrap" => {
            style.break_long_words = matches!(value.trim(), "break-word" | "anywhere");
        }
        "word-break" => {
            style.break_long_words = matches!(value.trim(), "break-all" | "break-word");
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
            if let Some(v) = parse_length_signed(value, parent_font_size) {
                style.margin.top = v;
            }
        }
        "margin-right" => {
            let v = value.trim().to_ascii_lowercase();
            if v == "auto" {
                style.margin_right_auto = true;
                style.margin.right = 0;
            } else if let Some(v) = parse_length_signed(value, parent_font_size) {
                style.margin_right_auto = false;
                style.margin.right = v;
            }
        }
        "margin-bottom" => {
            if let Some(v) = parse_length_signed(value, parent_font_size) {
                style.margin.bottom = v;
            }
        }
        "margin-left" => {
            let v = value.trim().to_ascii_lowercase();
            if v == "auto" {
                style.margin_left_auto = true;
                style.margin.left = 0;
            } else if let Some(v) = parse_length_signed(value, parent_font_size) {
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
            style.content = parse_content(value);
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
        // The four offsets in one, the same 1-to-4 value pattern `margin` uses.
        // Unhandled, firefox.com's background gradient had no position at all:
        // it is placed entirely by `inset: calc(70vh / -1.5) -30vw auto`.
        "inset" => {
            let parts = split_value_components(value);
            let (top, right, bottom, left) = match parts.len() {
                0 => return,
                1 => (0, 0, 0, 0),
                2 => (0, 1, 0, 1),
                3 => (0, 1, 2, 1),
                _ => (0, 1, 2, 3),
            };
            let pick = |index: usize| parts.get(index).map(String::as_str).unwrap_or(&parts[0]);
            for (property, raw) in [
                ("top", pick(top)),
                ("right", pick(right)),
                ("bottom", pick(bottom)),
                ("left", pick(left)),
            ] {
                apply_declaration(
                    style,
                    &Declaration {
                        property: property.to_string(),
                        value: raw.to_string(),
                        important: declaration.important,
                    },
                    parent_font_size,
                );
            }
        }
        "top" => { style.top = parse_offset(value, parent_font_size); }
        "right" => { style.right = parse_offset(value, parent_font_size); }
        "bottom" => { style.bottom = parse_offset(value, parent_font_size); }
        "left" => { style.left = parse_offset(value, parent_font_size); }
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
            match parts.as_slice() {
                ["none"] => {
                    style.flex_grow = 0;
                    style.flex_shrink = 0;
                    style.flex_basis = None;
                }
                ["auto"] => {
                    style.flex_grow = 100;
                    style.flex_shrink = 100;
                    style.flex_basis = None;
                }
                ["initial"] => {
                    style.flex_grow = 0;
                    style.flex_shrink = 100;
                    style.flex_basis = None;
                }
                _ => {
                    if let Some(first) = parts.first() {
                        if let Ok(grow) = first.parse::<f32>() {
                            style.flex_grow = (grow * 100.0).round() as u32;
                            // One number means `<grow> 1 0%`: the item starts
                            // from nothing and the whole width is shared out by
                            // the grow factors. Leaving the basis at `auto` let
                            // each item keep its content width first, so a row
                            // of three came out short of an even split.
                            if parts.len() == 1 {
                                style.flex_shrink = 100;
                                style.flex_basis = Some(LengthValue::Pixels(0));
                            }
                        } else {
                            // `flex: <basis>`, e.g. `flex: 200px`.
                            style.flex_grow = 100;
                            style.flex_shrink = 100;
                            style.flex_basis = parse_length_value(first, parent_font_size);
                        }
                    }
                    if parts.len() >= 2 {
                        if let Ok(shrink) = parts[1].parse::<f32>() {
                            style.flex_shrink = (shrink * 100.0).round() as u32;
                        } else {
                            style.flex_basis = parse_length_value(parts[1], parent_font_size);
                        }
                    }
                    if parts.len() >= 3 {
                        style.flex_basis = parse_length_value(parts[2], parent_font_size);
                    }
                }
            }
        }
        // `gap: <row> <column>`. This engine keeps one gap, so the row value
        // wins and a single value sets both -- which is how nearly every sheet
        // writes it.
        "gap" | "grid-gap" | "row-gap" | "grid-row-gap" => {
            let first = split_value_components(value)
                .into_iter()
                .next()
                .unwrap_or_default();
            if let Some(px) = parse_length(&first, parent_font_size) {
                style.gap = px;
            }
        }
        // A column gap on its own was dropped, so items written with only
        // `column-gap` sat flush against each other.
        "column-gap" | "grid-column-gap" => {
            if let Some(px) = parse_length(value, parent_font_size) {
                style.gap = px;
            }
        }
        // ── Grid properties ──────────────────────────────────────────────────
        "grid-template" => {
            apply_grid_template(style, value, parent_font_size);
        }
        "grid-template-columns" => {
            let (tracks, line_names) = parse_grid_track_list(value, parent_font_size);
            style.grid_template_columns = tracks;
            set_grid_line_names(style, line_names, false);
        }
        "grid-template-rows" => {
            let (tracks, line_names) = parse_grid_track_list(value, parent_font_size);
            style.grid_template_rows = tracks;
            set_grid_line_names(style, line_names, true);
        }
        "grid-auto-rows" => {
            style.grid_auto_rows = parse_grid_track_size(value.trim(), parent_font_size)
                .unwrap_or(GridTrackSize::Auto);
        }
        "grid-auto-columns" => {
            style.grid_auto_columns = parse_grid_track_size(value.trim(), parent_font_size)
                .unwrap_or(GridTrackSize::Auto);
        }
        "grid-template-areas" => {
            style.grid_template_areas = parse_grid_template_areas(value).map(Box::new);
        }
        "grid-area" => {
            apply_grid_area(style, value);
        }
        "grid-column" => {
            apply_grid_axis(style, value, false);
        }
        "grid-row" => {
            apply_grid_axis(style, value, true);
        }
        "grid-column-start" => {
            apply_grid_edge(style, value, false, GridEdge::Start);
        }
        "grid-column-end" => {
            apply_grid_edge(style, value, false, GridEdge::End);
        }
        "grid-row-start" => {
            apply_grid_edge(style, value, true, GridEdge::Start);
        }
        "grid-row-end" => {
            apply_grid_edge(style, value, true, GridEdge::End);
        }
        "grid" => {
            // The full `grid` shorthand also carries the implicit-track and
            // auto-flow settings; `grid-template` is handled above.
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
                    num.trim().parse::<f32>().ok().zip(den.trim().parse::<f32>().ok())
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
        "transform" => {
            parse_transform_into(value, style);
        }
        "transform-origin" => {
            let parts: Vec<&str> = value.split_whitespace().collect();
            style.transform_origin_x = parse_transform_origin_pct(parts.first().copied().unwrap_or("50%"));
            style.transform_origin_y = parse_transform_origin_pct(parts.get(1).copied().unwrap_or("50%"));
        }
        // No-op properties — parsed to prevent warnings, not yet implemented
        "scroll-behavior" | "overscroll-behavior" | "overscroll-behavior-x" | "overscroll-behavior-y"
        | "resize" | "writing-mode" | "text-orientation" | "direction" | "unicode-bidi"
        | "scroll-snap-type" | "scroll-snap-align" | "scroll-padding" | "scroll-padding-top"
        | "will-change" | "isolation" | "mix-blend-mode" | "backdrop-filter"
        | "-webkit-overflow-scrolling" | "touch-action" | "user-select" | "-webkit-user-select"
        | "appearance" | "-webkit-appearance" | "-moz-appearance"
        | "contain" | "content-visibility" => {
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
        other => record_unsupported_property(other),
    }
}

/// Tally of declarations that reached `apply_declaration` and fell through it.
///
/// Guessing which CSS the engine is missing does not scale: a real page's
/// stylesheet carries thousands of declarations, and only the ones that both
/// appear often *and* change layout are worth implementing. `TOBIRA_DEBUG_CSS=1`
/// makes the engine report exactly what it dropped, ranked by how often, so the
/// worklist comes from measurement rather than a hunch.
static UNSUPPORTED_PROPERTIES: Mutex<Option<BTreeMap<String, u32>>> = Mutex::new(None);

fn css_debug_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("TOBIRA_DEBUG_CSS").is_some())
}

fn record_unsupported_property(property: &str) {
    if !css_debug_enabled() || property.is_empty() {
        return;
    }
    let Ok(mut guard) = UNSUPPORTED_PROPERTIES.lock() else {
        return;
    };
    *guard
        .get_or_insert_with(BTreeMap::new)
        .entry(property.to_string())
        .or_insert(0) += 1;
}

/// Unsupported declarations seen so far, most frequent first.
pub fn unsupported_property_report() -> Vec<(String, u32)> {
    let Ok(guard) = UNSUPPORTED_PROPERTIES.lock() else {
        return Vec::new();
    };
    let Some(counts) = guard.as_ref() else {
        return Vec::new();
    };
    let mut ranked: Vec<(String, u32)> = counts
        .iter()
        .map(|(name, count)| (name.clone(), *count))
        .collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    ranked
}

// ─────────────────────────────────────────────────────────────────────────────
// default_display / default_margin
// ─────────────────────────────────────────────────────────────────────────────

fn default_display(tag_name: &str) -> Display {
    match tag_name {
        "document" | "html" | "body" | "main" | "section" | "article" | "div" | "header"
        | "footer" | "nav" | "aside" | "p" | "ul" | "ol" | "li" | "pre" | "blockquote" | "h1"
        | "h2" | "h3" | "h4" | "h5" | "h6" | "table" | "tbody" | "thead" | "tfoot" | "tr"
        | "td" | "th" | "center" | "frameset" | "hr"
        // The rest of the block-level elements the HTML rendering rules name.
        // Falling through to `inline` does not just misplace them: an inline
        // formatting context drops block-level children outright. firefox.com
        // wraps its front-page headline in `<hgroup>`, so the largest text on
        // the page -- an `<h1>` that is `display: block` -- was never laid out
        // at all, leaving the hero empty.
        | "hgroup" | "figure" | "figcaption" | "address" | "dl" | "dt" | "dd" | "fieldset"
        | "legend" | "form" | "details" | "summary" | "search" | "menu" | "dir" | "caption" => {
            if tag_name == "li" {
                Display::ListItem
            } else {
                Display::Block
            }
        }
        // `<template>` belongs here too: its contents are inert and are never
        // rendered, only cloned by script. Leaving it out let every custom
        // element on MDN paint the markup it keeps in a template -- the search
        // button was drawn twice, once from its template and once for real.
        // A `<select>` shows one option at a time, in a control it draws
        // itself; the rest are only there to be chosen from. Laying them out as
        // ordinary content spilled every one onto the page -- firefox.com's
        // footer carries a language picker with over a hundred entries, and it
        // alone made the footer 6120px tall against the 975px a browser gives
        // it.
        "script" | "style" | "title" | "head" | "meta" | "link" | "noscript" | "template"
        | "option" | "optgroup" => Display::None,
        _ => Display::Inline,
    }
}

fn default_margin(tag_name: &str) -> SignedEdgeSizes {
    match tag_name {
        // `1em` above and below, which collapses to one gap between two of
        // them. Writing only a bottom margin left no room above the first
        // paragraph of a section, and half the gap everywhere else.
        "p" => SignedEdgeSizes::vertical(16, 16),
        "ul" | "ol" | "menu" | "dir" => SignedEdgeSizes::vertical(16, 16),
        "dl" => SignedEdgeSizes::vertical(16, 16),
        // A list item has no margin of its own; the room around a list comes
        // from the list.
        "li" => SignedEdgeSizes::default(),
        "table" | "tr" => SignedEdgeSizes::default(),
        "td" | "th" => SignedEdgeSizes::default(),
        "hr" => SignedEdgeSizes::vertical(8, 8),
        "blockquote" => SignedEdgeSizes {
            top: 16,
            right: 40,
            bottom: 16,
            left: 40,
        },
        _ => SignedEdgeSizes::default(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Legacy HTML attributes
// ─────────────────────────────────────────────────────────────────────────────

fn apply_legacy_attributes(style: &mut ComputedStyle, element: &Element, parent_font_size: u32) {
    if element.tag_name == "table" {
        style.table_cellpadding = element
            .attribute("cellpadding")
            .and_then(|value| value.trim().parse::<u32>().ok());
    }

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

    if element.tag_name != "table"
        && let Some(text_align) = element.attribute("align").and_then(parse_text_align)
    {
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
        Some(Selector { parts, pseudo_element, specificity_override: None })
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
                // `:before` / `:after` with one colon are the legacy spelling of
                // the pseudo-elements, and minifiers emit it because it is a byte
                // shorter. Falling through to the pseudo-class path meant they
                // were dropped as "unknown", so `.x:after { width: 0 }` matched
                // `.x` itself -- collapsing the element to nothing.
                match pseudo_name.to_ascii_lowercase().as_str() {
                    "before" => {
                        selector.pseudo_element = Some(PseudoElement::Before);
                        continue;
                    }
                    "after" => {
                        selector.pseudo_element = Some(PseudoElement::After);
                        continue;
                    }
                    // Not modelled, but they must not style the host element
                    // either, so the selector matches nothing.
                    "first-line" | "first-letter" => {
                        selector.never_match = true;
                        continue;
                    }
                    _ => {}
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
                } else if !pseudo_class_is_ignorable(&pseudo_name) {
                    // A pseudo-class narrows what a selector matches. Dropping
                    // one that is not modelled *widens* it instead, which is the
                    // opposite of what it says. Wikipedia scopes its edit-link
                    // brackets with
                    //
                    //   .client-nojs a:has(+ a.mw-editsection-visualeditor…)::after
                    //
                    // and with `:has()` discarded that became "every link on the
                    // page", so a stray `]` was drawn after every menu entry and
                    // every row of the table of contents.
                    selector.never_match = true;
                }
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

/// Pseudo-classes that are safe to skip over rather than to fail on.
///
/// These either match nearly everything (`:is()` and `:where()` are grouping
/// constructs, and treating them as satisfied keeps the rest of the compound
/// selector doing its work) or describe a document-wide condition that holds
/// for ordinary rendering.
fn pseudo_class_is_ignorable(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "is" | "where"
            | "matches"
            | "any"
            | "any-link"
            // Every link this renders is an unvisited one -- there is no
            // history to consult -- so `:link` holds for all of them.
            // `:visited` is deliberately *not* here: nothing has been visited,
            // so a rule scoped to it should not apply, and dropping the
            // selector is how that gets said.
            | "link"
            | "scope"
            | "dir"
            | "lang"
            | "read-write"
            | "optional"
            | "defined"
            | "host"
            | "first-of-type"
    )
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
        "has" => {
            let arg = args.unwrap_or("").trim();
            // A sibling form asks about the element's siblings, not its
            // children, so answering it here would widen the rule rather than
            // narrow it. Leave it unmodelled -- Wikipedia scopes its
            // edit-link brackets with `a:has(+ a.mw-editsection-visualeditor)`,
            // and a wrong yes there drew a stray `]` after every link on the
            // page.
            if arg.starts_with('+') || arg.starts_with('~') {
                return None;
            }
            // `:has(> x)` and `:has(x)` are both answered against the children,
            // so the child combinator is simply consumed.
            let arg = arg.strip_prefix('>').unwrap_or(arg).trim();
            let selectors = split_at_top_level(arg, ',')
                .into_iter()
                .map(|part| parse_simple_selector(part.trim()))
                .collect::<Option<Vec<_>>>()?;
            if selectors.is_empty() {
                None
            } else {
                Some(PseudoClass::Has(selectors))
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
    fn key_bucket(&self) -> Option<RuleBucket<'_>> {
        let key = &self.parts.last()?.simple;
        if key.never_match {
            return None;
        }
        if let Some(id) = key.id.as_deref() {
            return Some(RuleBucket::Id(id));
        }
        if let Some(class) = key.classes.first().map(String::as_str) {
            return Some(RuleBucket::Class(class));
        }
        if let Some(tag) = key.tag_name.as_deref() {
            return Some(RuleBucket::Tag(tag));
        }
        Some(RuleBucket::Universal)
    }

    fn specificity(&self) -> usize {
        self.specificity_override
            .unwrap_or_else(|| self.parts.iter().map(|part| part.simple.specificity()).sum())
    }

    fn matches(
        &self,
        element: &ElementIdentity,
        ancestors: &[AncestorSlot],
        sibling_index: usize,
        sibling_count: usize,
        preceding_siblings: &[ElementIdentity],
        children: &Rc<[ElementIdentity]>,
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
            children: Rc::clone(children),
        };
        self.matches_part(last_index, &current, ancestors, preceding_siblings, interactive)
    }

    fn matches_part(
        &self,
        part_index: usize,
        current: &AncestorSlot,
        ancestors: &[AncestorSlot],
        current_preceding_siblings: &[ElementIdentity],
        interactive: &InteractiveState,
    ) -> bool {
        if !self.parts[part_index].simple.matches_slot(current, interactive) {
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
                        children: empty_siblings_rc(),
                    };
                    self.matches_part(
                        part_index - 1,
                        &sibling_slot,
                        ancestors,
                        &current_preceding_siblings[..sibling_index],
                        interactive,
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
                        children: empty_siblings_rc(),
                    };
                    self.matches_part(
                        part_index - 1,
                        &sibling_slot,
                        ancestors,
                        &current_preceding_siblings[..sibling_index],
                        interactive,
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
                PseudoClass::Not(selectors) => {
                    !selectors.iter().any(|selector| selector.matches_slot(slot, interactive))
                }
                PseudoClass::Has(selectors) => {
                    let count = slot.children.len();
                    (0..count).any(|index| {
                        let child = AncestorSlot {
                            element: slot.children[index].clone(),
                            sibling_index: index,
                            sibling_count: count,
                            siblings: slot.children.clone(),
                            prec_count: index,
                            children: empty_siblings_rc(),
                        };
                        selectors
                            .iter()
                            .any(|selector| selector.matches_slot(&child, interactive))
                    })
                }
                PseudoClass::Hover => {
                    slot.element.node_id.is_some()
                        && slot.element.node_id == interactive.hovered_node_id
                }
                PseudoClass::Focus => {
                    slot.element.node_id.is_some()
                        && slot.element.node_id == interactive.focused_node_id
                }
                PseudoClass::Active => {
                    slot.element.node_id
                        .is_some_and(|id| interactive.active_node_ids.contains(&id))
                }
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


/// The table part `display` names, if it names one.
fn parse_table_role(input: &str) -> TableRole {
    match input.trim().to_ascii_lowercase().as_str() {
        "table" | "inline-table" => TableRole::Table,
        "table-row-group" | "table-header-group" | "table-footer-group" => TableRole::RowGroup,
        "table-row" => TableRole::Row,
        "table-cell" => TableRole::Cell,
        _ => TableRole::None,
    }
}

fn parse_display(input: &str) -> Option<Display> {
    match input.trim().to_ascii_lowercase().as_str() {
        "block" | "flow-root" | "table" | "table-row" | "table-row-group"
        | "table-header-group" | "table-footer-group" => Some(Display::Block),
        "flex" => Some(Display::Flex),
        "inline-flex" => Some(Display::InlineFlex),
        "grid" => Some(Display::Grid),
        "inline-grid" => Some(Display::InlineGrid),
        "inline-block" => Some(Display::InlineBlock),
        "contents" => Some(Display::Contents),
        "inline" => Some(Display::Inline),
        // Block-level on the outside. Which column it lands in and how wide
        // it ends up are decided by the table layout, through `table_role`.
        "table-cell" => Some(Display::Block),
        "inline-table" => Some(Display::InlineBlock),
        "list-item" => Some(Display::ListItem),
        "none" => Some(Display::None),
        _ => None,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Grid parsing helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Parse a grid track list like "100px 1fr auto repeat(3, 200px)".
/// Parse a track list, returning the tracks and any names its `[bracket]`
/// groups attach to grid lines.
///
/// Line indices count lines rather than tracks, so a name sitting before the
/// first track is line 0 and a list of N tracks ends at line N.
fn parse_grid_track_list(
    input: &str,
    parent_font_size: u32,
) -> (Vec<GridTrackSize>, Vec<(Box<str>, usize)>) {
    fn push_token(token: &str, tracks: &mut Vec<GridTrackSize>, parent_font_size: u32) {
        let token = token.trim();
        if token.is_empty() {
            return;
        }
        if token.starts_with("repeat(") {
            tracks.extend(expand_grid_repeat(token, parent_font_size));
        } else if let Some(size) = parse_grid_track_size(token, parent_font_size) {
            tracks.push(size);
        }
    }

    let mut tracks: Vec<GridTrackSize> = Vec::new();
    let mut names: Vec<(Box<str>, usize)> = Vec::new();
    let mut buf = String::new();
    let mut depth = 0usize;
    let chars: Vec<char> = input.trim().chars().collect();
    let mut i = 0usize;

    while i < chars.len() {
        let ch = chars[i];
        match ch {
            '[' if depth == 0 => {
                push_token(&buf, &mut tracks, parent_font_size);
                buf.clear();
                let mut inner = String::new();
                i += 1;
                while i < chars.len() && chars[i] != ']' {
                    inner.push(chars[i]);
                    i += 1;
                }
                // One bracket group may name the same line several times.
                for name in inner.split_whitespace() {
                    names.push((name.to_string().into_boxed_str(), tracks.len()));
                }
            }
            '(' => {
                depth += 1;
                buf.push(ch);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                buf.push(ch);
            }
            ' ' | '\t' | '\n' | '\r' if depth == 0 => {
                push_token(&buf, &mut tracks, parent_font_size);
                buf.clear();
            }
            _ => buf.push(ch),
        }
        i += 1;
    }
    push_token(&buf, &mut tracks, parent_font_size);

    (tracks, names)
}

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
    // Names inside a `repeat()` are dropped: repeating a line name would need
    // per-repetition indices, and no page has needed it yet.
    let (track_sizes, _line_names) = parse_grid_track_list(track_str.trim(), parent_font_size);
    if track_sizes.is_empty() {
        return Vec::new();
    }
    track_sizes
        .into_iter()
        .cycle()
        .take(count)
        .collect()
}

fn parse_grid_track_size(token: &str, parent_font_size: u32) -> Option<GridTrackSize> {
    let t = token.trim().to_ascii_lowercase();

    // `minmax(min, max)`. We do not model a two-sided track, so take the max:
    // that is the size the track wants when there is room, and the resolver
    // already shrinks `fr` and `auto` tracks when there is not. So
    // `minmax(0, 1fr)` behaves like `1fr` and `minmax(0, 40rem)` like `40rem`.
    // Dropping the whole token instead -- which is what used to happen -- makes
    // the track vanish and shifts every item after it into the wrong column.
    if let Some(inner) = t
        .strip_prefix("minmax(")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        let mut depth = 0usize;
        let mut comma = None;
        for (index, c) in inner.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => depth = depth.saturating_sub(1),
                ',' if depth == 0 => {
                    comma = Some(index);
                    break;
                }
                _ => {}
            }
        }
        return match comma {
            Some(index) => parse_grid_track_size(&inner[index + 1..], parent_font_size)
                .or_else(|| parse_grid_track_size(&inner[..index], parent_font_size)),
            None => parse_grid_track_size(inner, parent_font_size),
        };
    }

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

/// Record the line names one axis' track list produced.
fn set_grid_line_names(style: &mut ComputedStyle, names: Vec<(Box<str>, usize)>, rows: bool) {
    if names.is_empty() {
        return;
    }
    let mut current = style
        .grid_line_names
        .take()
        .map(|boxed| *boxed)
        .unwrap_or_default();
    if rows {
        current.rows = names;
    } else {
        current.columns = names;
    }
    style.grid_line_names = Some(Box::new(current));
}

/// Everything outside the quoted strings, so the row sizes written between an
/// area template's rows can be read as an ordinary track list.
fn strip_quoted_strings(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut quote: Option<char> = None;
    for c in input.chars() {
        match quote {
            Some(open) => {
                if c == open {
                    quote = None;
                }
            }
            None => {
                if c == '"' || c == '\'' {
                    quote = Some(c);
                } else {
                    out.push(c);
                }
            }
        }
    }
    out
}

/// The `grid-template` shorthand: `<rows> / <columns>`, where the row side may
/// instead be an area template written as strings with optional row sizes
/// between them.
///
/// Wikipedia's article grid is defined entirely through this shorthand
/// (`grid-template: min-content 1fr min-content / 12.25rem minmax(0,1fr)`), so
/// without it the areas resolve but every column is the same width and the
/// article lands in the sidebar's half of the page.
fn apply_grid_template(style: &mut ComputedStyle, value: &str, parent_font_size: u32) {
    let value = value.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("none") {
        return;
    }

    // The rows/columns separator is the first `/` that is not inside a quoted
    // string, a function, or a line-name bracket.
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    let mut separator = None;
    for (index, c) in value.char_indices() {
        if let Some(open) = quote {
            if c == open {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => quote = Some(c),
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            '/' if depth == 0 => {
                separator = Some(index);
                break;
            }
            _ => {}
        }
    }

    let (rows_src, cols_src) = match separator {
        Some(index) => (&value[..index], Some(&value[index + 1..])),
        None => (value, None),
    };

    if rows_src.contains('"') || rows_src.contains('\'') {
        style.grid_template_areas = parse_grid_template_areas(rows_src).map(Box::new);
        let sizes = strip_quoted_strings(rows_src);
        let (tracks, names) = parse_grid_track_list(&sizes, parent_font_size);
        style.grid_template_rows = tracks;
        set_grid_line_names(style, names, true);
    } else {
        let (tracks, names) = parse_grid_track_list(rows_src, parent_font_size);
        style.grid_template_rows = tracks;
        set_grid_line_names(style, names, true);
    }

    if let Some(cols_src) = cols_src {
        let (tracks, names) = parse_grid_track_list(cols_src, parent_font_size);
        style.grid_template_columns = tracks;
        set_grid_line_names(style, names, false);
    }
}

/// Parse `grid-template-areas`.
///
/// Returns `None` for `none`, for anything unparseable, and -- per spec -- for
/// a template that is invalid, since an invalid declaration is dropped rather
/// than partially applied.
fn parse_grid_template_areas(value: &str) -> Option<GridTemplateAreas> {
    let value = value.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("none") {
        return None;
    }

    // Pull out the quoted strings; each one is a row. Anything outside quotes
    // is not part of this property's grammar.
    let mut rows: Vec<Vec<Option<String>>> = Vec::new();
    let mut rest = value;
    while let Some(open) = rest.find(['"', '\'']) {
        let quote = rest.as_bytes()[open] as char;
        let after = &rest[open + 1..];
        let close = after.find(quote)?;
        let row = &after[..close];
        rest = &after[close + 1..];

        // A null cell token is a *run* of one or more periods: `.`, `...` and
        // `.....` are each one empty cell, not one cell per period.
        let cells: Vec<Option<String>> = row
            .split_whitespace()
            .map(|token| {
                if token.chars().all(|c| c == '.') {
                    None
                } else {
                    Some(token.to_string())
                }
            })
            .collect();
        rows.push(cells);
    }

    if rows.is_empty() {
        return None;
    }

    // Ragged rows invalidate the whole declaration.
    let columns = rows[0].len();
    if columns == 0 || rows.iter().any(|row| row.len() != columns) {
        return None;
    }

    // Collect each name's bounding box, then require that the box is exactly
    // filled by that name -- an L-shaped or split area is invalid.
    let mut areas: Vec<(Box<str>, usize, usize, usize, usize)> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for row in &rows {
        for cell in row {
            if let Some(name) = cell {
                if !seen.iter().any(|s| s == name) {
                    seen.push(name.clone());
                }
            }
        }
    }

    for name in seen {
        let mut row_start = usize::MAX;
        let mut col_start = usize::MAX;
        let mut row_end = 0usize;
        let mut col_end = 0usize;
        let mut count = 0usize;
        for (r, row) in rows.iter().enumerate() {
            for (c, cell) in row.iter().enumerate() {
                if cell.as_deref() == Some(name.as_str()) {
                    row_start = row_start.min(r);
                    col_start = col_start.min(c);
                    row_end = row_end.max(r + 1);
                    col_end = col_end.max(c + 1);
                    count += 1;
                }
            }
        }
        if count != (row_end - row_start) * (col_end - col_start) {
            return None;
        }
        areas.push((
            name.into_boxed_str(),
            row_start,
            col_start,
            row_end,
            col_end,
        ));
    }

    Some(GridTemplateAreas {
        rows: rows.len(),
        columns,
        areas,
    })
}

/// Apply one `<grid-line>` pair to an axis, recording any names for layout.
fn apply_grid_line_pair(
    start: GridLineRef,
    end: Option<GridLineRef>,
    placement: &mut GridPlacement,
    start_name: &mut Option<Box<str>>,
    end_name: &mut Option<Box<str>>,
) {
    // An omitted end copies a named start. That is what makes
    // `grid-column: content` span content-start..content-end instead of
    // collapsing onto a single line.
    let end = match end {
        Some(end) => end,
        None => match &start {
            GridLineRef::Named(name) => GridLineRef::Named(name.clone()),
            _ => GridLineRef::Auto,
        },
    };

    match start {
        GridLineRef::Line(n) => placement.start = Some(n),
        GridLineRef::Named(name) => *start_name = Some(name),
        GridLineRef::Span(n) => placement.span = Some(n),
        GridLineRef::Auto => {}
    }
    match end {
        GridLineRef::Line(n) => {
            if let Some(start) = placement.start {
                placement.span = Some((n - start).max(1) as u32);
            } else {
                placement.start = Some(n);
            }
        }
        GridLineRef::Named(name) => *end_name = Some(name),
        GridLineRef::Span(n) => placement.span = Some(n),
        GridLineRef::Auto => {}
    }
}

/// Apply `grid-row` / `grid-column` to one axis.
fn apply_grid_axis(
    style: &mut ComputedStyle,
    value: &str,
    rows: bool,
) {
    let parts: Vec<&str> = value.split('/').collect();
    let start = parse_grid_line_ref(parts.first().copied().unwrap_or(""));
    let end = parts.get(1).map(|part| parse_grid_line_ref(part));

    let mut names = style
        .grid_placement_names
        .take()
        .map(|boxed| *boxed)
        .unwrap_or_default();
    if rows {
        apply_grid_line_pair(
            start,
            end,
            &mut style.grid_row,
            &mut names.row_start,
            &mut names.row_end,
        );
    } else {
        apply_grid_line_pair(
            start,
            end,
            &mut style.grid_column,
            &mut names.column_start,
            &mut names.column_end,
        );
    }
    if !names.is_empty() {
        style.grid_placement_names = Some(Box::new(names));
    }
}

/// Apply a single-edge longhand (`grid-row-start` and friends).
fn apply_grid_edge(style: &mut ComputedStyle, value: &str, rows: bool, edge: GridEdge) {
    let line = parse_grid_line_ref(value);
    let mut names = style
        .grid_placement_names
        .take()
        .map(|boxed| *boxed)
        .unwrap_or_default();
    let (placement, name_slot) = match (rows, edge) {
        (true, GridEdge::Start) => (&mut style.grid_row, &mut names.row_start),
        (true, GridEdge::End) => (&mut style.grid_row, &mut names.row_end),
        (false, GridEdge::Start) => (&mut style.grid_column, &mut names.column_start),
        (false, GridEdge::End) => (&mut style.grid_column, &mut names.column_end),
    };
    match line {
        GridLineRef::Named(name) => *name_slot = Some(name),
        GridLineRef::Span(n) => placement.span = Some(n),
        GridLineRef::Line(n) => match edge {
            GridEdge::Start => placement.start = Some(n),
            GridEdge::End => {
                if let Some(start) = placement.start {
                    placement.span = Some((n - start).max(1) as u32);
                } else {
                    placement.start = Some(n);
                }
            }
        },
        GridLineRef::Auto => {}
    }
    if !names.is_empty() {
        style.grid_placement_names = Some(Box::new(names));
    }
}

/// Apply the `grid-area` shorthand.
///
/// The positional order is row-start / column-start / row-end / column-end,
/// and a lone `<custom-ident>` sets all four longhands to that name, which
/// resolves through the area's implicit `-start` / `-end` lines to exactly the
/// named rectangle. We record the name and let layout do that lookup, since
/// the item cannot see its container's template from here.
fn apply_grid_area(style: &mut ComputedStyle, value: &str) {
    let refs: Vec<GridLineRef> = value
        .split('/')
        .map(|part| parse_grid_line_ref(part))
        .collect();

    // A lone name is also kept as an *area* name: if the container's template
    // defines an area by that name, layout uses its rectangle directly instead
    // of going through lines.
    if let [GridLineRef::Named(name)] = refs.as_slice() {
        style.grid_area_name = Some(name.clone());
    }

    let row_start = refs.first().cloned().unwrap_or(GridLineRef::Auto);
    // Order is row-start / column-start / row-end / column-end, and an omitted
    // column-start copies a named row-start into all four longhands.
    let col_start = match refs.get(1).cloned() {
        Some(value) => value,
        None => match &row_start {
            GridLineRef::Named(name) => GridLineRef::Named(name.clone()),
            _ => GridLineRef::Auto,
        },
    };

    let mut names = style
        .grid_placement_names
        .take()
        .map(|boxed| *boxed)
        .unwrap_or_default();
    apply_grid_line_pair(
        row_start,
        refs.get(2).cloned(),
        &mut style.grid_row,
        &mut names.row_start,
        &mut names.row_end,
    );
    apply_grid_line_pair(
        col_start,
        refs.get(3).cloned(),
        &mut style.grid_column,
        &mut names.column_start,
        &mut names.column_end,
    );
    if !names.is_empty() {
        style.grid_placement_names = Some(Box::new(names));
    }
}

/// Is this `grid-area` value a bare area name rather than a line number?
fn is_grid_area_ident(s: &str) -> bool {
    !s.is_empty()
        && !s.eq_ignore_ascii_case("auto")
        && !s.eq_ignore_ascii_case("none")
        && !s.starts_with("span")
        && s.parse::<i32>().is_err()
        && s.chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        && !s.starts_with(|c: char| c.is_ascii_digit())
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

/// The UA stylesheet sets `list-style-type` on the list container and lets it
/// inherit down to the items; everything else simply inherits.
fn default_list_style_type(tag_name: &str, parent: Option<&ComputedStyle>) -> ListStyleType {
    match tag_name {
        "ol" => ListStyleType::Decimal,
        "ul" | "menu" | "dir" => ListStyleType::Disc,
        _ => parent
            .map(|style| style.list_style_type)
            .unwrap_or(ListStyleType::Disc),
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

    // Inset shadows are not yet supported; return None so they are silently skipped
    // rather than being incorrectly drawn as outer shadows.
    if tokens.iter().any(|t| t.to_ascii_lowercase() == "inset") {
        return None;
    }

    let tokens: Vec<&str> = tokens.iter().map(|s| s.as_str()).collect();

    if tokens.len() < 2 {
        return None;
    }

    let mut offset_x: i32 = 0;
    let mut offset_y: i32 = 0;
    let mut blur: u32 = 0;
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
        match parse_color(token) {
            Some(color) => {
                style.border_color = color;
                style.border_color_transparent = false;
            }
            // A colour that paints nothing. Leaving the previous colour in place
            // would draw a line the author asked to be invisible.
            None => style.border_color_transparent = true,
        }
        {
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
    let mut set_width = |style: &mut ComputedStyle, px: u32| match side {
        "top" => style.border.top = px,
        "right" => style.border.right = px,
        "bottom" => style.border.bottom = px,
        "left" => style.border.left = px,
        _ => {}
    };
    for token in v.split_whitespace() {
        if token == "none" {
            set_width(style, 0);
            continue;
        }
        // A style keyword means this side is drawn, the same as in the all-sides
        // shorthand.
        if matches!(
            token,
            "solid" | "dashed" | "dotted" | "double" | "groove" | "ridge" | "inset" | "outset"
        ) {
            style.border_style_none = false;
            continue;
        }
        if let Some(px) = parse_length(token, parent_font_size) {
            set_width(style, px);
            continue;
        }
        // The colour was thrown away entirely: only the width was read, so
        // `border-top: 1px solid #c3c7cb` drew a *black* line. MDN's nav tabs are
        // separated by exactly that rule, so the bar came out ruled in black
        // instead of light grey.
        //
        // This engine keeps one border colour for all four sides, so a per-side
        // declaration sets that shared colour -- which is what a page means when
        // it only ever colours one side.
        match parse_color(token) {
            Some(color) => {
                style.border_color = color;
                style.border_color_transparent = false;
            }
            None => style.border_color_transparent = true,
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

fn parse_margin_shorthand(style: &mut ComputedStyle, input: &str, parent_font_size: u32) {
    // Reset auto flags
    style.margin_left_auto = false;
    style.margin_right_auto = false;

    let tokens: Vec<&str> = input.split_whitespace().collect();
    // Parse each token as length or auto (None means auto)
    let parsed: Vec<Option<i32>> = tokens.iter()
        .map(|t| {
            if t.to_ascii_lowercase() == "auto" {
                None // auto
            } else {
                parse_length_signed(t, parent_font_size)
            }
        })
        .collect();

    // Apply CSS box shorthand rules (1/2/3/4 values)
    // None means "auto" (0px, flag set separately)
    let resolve = |v: Option<i32>| v.unwrap_or(0);
    match parsed.as_slice() {
        [all] => {
            let v = resolve(*all);
            style.margin = SignedEdgeSizes::all(v);
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
            if left.is_none() { style.margin_left_auto = true; }
            if right.is_none() { style.margin_right_auto = true; }
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
                Some(r) => if is_max { r.max(v) } else { r.min(v) },
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
    if let Some(inner) = value.strip_prefix("clamp(").and_then(|s| s.strip_suffix(')')) {
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
        let root = root_font_size() as f32;
        return parse_float(number).map(|p| (p * root).round() as u32);
    }

    if let Some(number) = value.strip_suffix("em") {
        return parse_float(number).map(|p| (p * parent_font_size as f32).round() as u32);
    }

    // `ch` is the width of a "0" and `ex` the height of an "x", both of which
    // need the font. Lengths are resolved while styles are computed, before any
    // font is picked, so half the font size stands in for each -- close for the
    // proportional faces pages actually use. Unsupported, they were dropped
    // entirely: firefox.com holds its front-page blurb to `max-inline-size: 48ch`
    // and without it the line ran the full width of the column.
    if let Some(number) = value.strip_suffix("ch").or_else(|| value.strip_suffix("ex")) {
        return parse_float(number).map(|p| (p * parent_font_size as f32 / 2.0).round() as u32);
    }

    if let Some(number) = value.strip_suffix('%') {
        return parse_float(number).map(|p| ((p / 100.0) * parent_font_size as f32).round() as u32);
    }

    parse_float(&value).map(|p| p.round().max(0.0) as u32)
}

fn parse_length_signed(input: &str, parent_font_size: u32) -> Option<i32> {
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

fn parse_signed_length(input: &str, parent_font_size: u32) -> Option<i32> {
    parse_length_signed(input, parent_font_size)
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
    let mut depth = 0usize;
    while i < chars.len() {
        let ch = chars[i];
        match ch {
            '(' => {
                depth += 1;
                buf.push(ch);
                i += 1;
            }
            ')' => {
                depth = depth.saturating_sub(1);
                buf.push(ch);
                i += 1;
            }
            // Only split on operators at the top level. Without the depth test a
            // grouped sub-expression was torn apart -- `(15rem + 2rem) * 2` became
            // the tokens `(15rem` and `2rem)`, neither of which parses, so the
            // whole calc() was discarded. MDN writes its breakpoints this way.
            '+' | '*' | '/' if depth == 0 => {
                if !buf.trim().is_empty() {
                    values.push(resolve_calc_operand_f32(buf.trim(), parent_font_size)?);
                    buf.clear();
                }
                ops.push(ch);
                i += 1;
            }
            '-' if depth == 0 && !buf.trim().is_empty() => {
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
    // A well-formed calc has exactly one more operand than operators. Anything
    // else (a dangling operator, an empty operand from an unresolved value, etc.)
    // is invalid - bail instead of indexing past `values` and panicking.
    if ops.len() + 1 != values.len() {
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
    // A parenthesised group is its own calc() body.
    if let Some(inner) = t.strip_prefix('(').and_then(|s| s.strip_suffix(')')) {
        return parse_calc(inner, parent_font_size).map(|v| v as f32);
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
    // `rem` before `em`: the shorter suffix also matches a rem value, turning
    // `30rem` into `30r`, which then fails to parse and takes the whole calc()
    // down with it. Every `calc()` containing a rem was silently discarded.
    if let Some(n) = t.strip_suffix("rem") {
        return parse_float(n).map(|f| f * 16.0);
    }
    if let Some(n) = t.strip_suffix("em") {
        return parse_float(n).map(|f| f * parent_font_size as f32);
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

/// Resolve a `calc()` length against the containing block it applies to.
pub fn resolve_calc(percent_hundredths: i32, px: i32, basis: u32) -> u32 {
    ((basis as i64 * percent_hundredths as i64) / 10_000 + px as i64).max(0) as u32
}

/// Reduce a `calc()` body to a percentage plus a pixel offset.
///
/// Returns `None` for anything that does not fit that shape -- other units,
/// multiplication, nesting -- leaving those to the font-size-relative evaluator
/// that handles the general case.
fn parse_calc_length_value(expr: &str, parent_font_size: u32) -> Option<LengthValue> {
    // `var()` is substituted before we get here, so a leftover paren means a
    // nested function we do not model.
    if expr.contains('(') {
        return None;
    }

    let mut percent_hundredths = 0_f32;
    let mut px = 0_f32;
    let mut sign = 1_f32;
    let mut expect_operator = false;

    // calc() *requires* whitespace around `+` and `-` but merely *allows* it
    // around `*` and `/`. That asymmetry is what makes `20em * -1` unambiguous:
    // the `-` belongs to the number, not to the sum. Collapsing the optional
    // spaces first means each whitespace-delimited token is exactly one term.
    let normalized = collapse_spaces_around_muldiv(expr);
    for token in normalized.split_whitespace() {
        if expect_operator {
            sign = match token {
                "+" => 1.0,
                "-" => -1.0,
                _ => return None,
            };
            expect_operator = false;
            continue;
        }
        let (term_percent, term_px) = parse_calc_term(token, parent_font_size)?;
        percent_hundredths += sign * term_percent;
        px += sign * term_px;
        expect_operator = true;
    }

    if !expect_operator {
        return None;
    }

    let percent_hundredths = percent_hundredths.round() as i32;
    let px = px.round() as i32;
    if percent_hundredths == 0 && px >= 0 {
        return Some(LengthValue::Pixels(px as u32));
    }
    // A negative result has to survive. `top: calc(var(--offset) * -1)` is how
    // MDN parks its skip link above the viewport; clamping that to zero left the
    // link sitting on the page, and because its width came from another calc it
    // also had no room, so it rendered one character per line.
    Some(LengthValue::Calc {
        percent_hundredths,
        px,
    })
}

/// Remove the optional whitespace around `*` and `/` so each term survives a
/// `split_whitespace()` as one token. Spacing around `+` and `-` is left alone,
/// because calc() relies on it to tell a subtraction from a negative number.
fn collapse_spaces_around_muldiv(expr: &str) -> String {
    let mut out = String::with_capacity(expr.len());
    let mut chars = expr.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '*' || ch == '/' {
            while out.ends_with(' ') {
                out.pop();
            }
            out.push(ch);
            while chars.peek().is_some_and(|next| next.is_whitespace()) {
                chars.next();
            }
        } else if ch.is_whitespace() {
            out.push(' ');
        } else {
            out.push(ch);
        }
    }
    out
}

/// One multiplicative term of a `calc()` sum, as `(percent_hundredths, px)`.
///
/// A term is at most one length or percentage, scaled by any number of plain
/// numbers: `2px*2`, `35rem*-1/4`, `100%`.
fn parse_calc_term(token: &str, parent_font_size: u32) -> Option<(f32, f32)> {
    // Split into factors, keeping the operator that preceded each one.
    let mut factors: Vec<(char, &str)> = Vec::new();
    let mut operator = '*';
    let mut start = 0usize;
    for (index, ch) in token.char_indices() {
        if ch == '*' || ch == '/' {
            factors.push((operator, &token[start..index]));
            operator = ch;
            start = index + ch.len_utf8();
        }
    }
    factors.push((operator, &token[start..]));

    let mut scale = 1_f32;
    let mut length: Option<(f32, f32)> = None;

    for (operator, raw) in factors {
        match parse_calc_factor(raw, parent_font_size)? {
            CalcFactor::Number(n) => match operator {
                '*' => scale *= n,
                '/' if n != 0.0 => scale /= n,
                _ => return None,
            },
            CalcFactor::Length(term_percent, term_px) => {
                // A term holds at most one length, and dividing *by* a length
                // is not valid calc().
                if length.is_some() || operator == '/' {
                    return None;
                }
                length = Some((term_percent, term_px));
            }
        }
    }

    match length {
        Some((term_percent, term_px)) => Some((term_percent * scale, term_px * scale)),
        // A term with no length at all is a bare number, which only really makes
        // sense as zero; treat it as pixels the way this parser always has.
        None => Some((0.0, scale)),
    }
}

enum CalcFactor {
    Number(f32),
    /// `(percent_hundredths, px)`
    Length(f32, f32),
}

fn parse_calc_factor(token: &str, parent_font_size: u32) -> Option<CalcFactor> {
    let token = token.trim().to_ascii_lowercase();
    if token.is_empty() {
        return None;
    }
    if let Some(number) = token.strip_suffix('%') {
        return Some(CalcFactor::Length(parse_float(number)? * 100.0, 0.0));
    }
    if let Some(number) = token.strip_suffix("px") {
        return Some(CalcFactor::Length(0.0, parse_float(number)?));
    }
    // `rem` before `em`, or every rem would be read as an em.
    if let Some(number) = token.strip_suffix("rem") {
        return Some(CalcFactor::Length(
            0.0,
            parse_float(number)? * root_font_size() as f32,
        ));
    }
    if let Some(number) = token.strip_suffix("em") {
        return Some(CalcFactor::Length(
            0.0,
            parse_float(number)? * parent_font_size as f32,
        ));
    }
    parse_float(&token).map(CalcFactor::Number)
}

/// Parse a box offset (`top` / `right` / `bottom` / `left`).
///
/// Unlike a width these are routinely negative, and routinely percentages: the
/// oldest way to centre a fixed-width box is `left: 50%` with a negative margin
/// of half its width. Resolving that percentage here against the font size made
/// Yahoo! JAPAN's masthead logo `left: 7px` instead of `left: 495px`, so it sat
/// against the left edge of the page with its negative margin still applied.
fn parse_offset(input: &str, parent_font_size: u32) -> Option<LengthValue> {
    let value = input.trim().to_ascii_lowercase();
    if value == "auto" {
        return None;
    }
    if let Some(inner) = value.strip_prefix("calc(").and_then(|s| s.strip_suffix(')'))
        && let Some(length) = parse_calc_length_value(inner, parent_font_size)
    {
        return Some(length);
    }
    if let Some(number) = value.strip_suffix('%') {
        let percent = parse_float(number)?;
        return Some(LengthValue::Calc {
            percent_hundredths: (percent * 100.0).round() as i32,
            px: 0,
        });
    }
    let pixels = parse_length_signed(&value, parent_font_size)?;
    Some(if pixels >= 0 {
        LengthValue::Pixels(pixels as u32)
    } else {
        // `LengthValue::Pixels` cannot hold a negative length, and a negative
        // offset is ordinary here.
        LengthValue::Calc { percent_hundredths: 0, px: pixels }
    })
}

fn parse_length_value(input: &str, parent_font_size: u32) -> Option<LengthValue> {
    let value = input.trim().to_ascii_lowercase();
    match value.as_str() {
        "min-content" => return Some(LengthValue::MinContent),
        // The bare keyword, as against `fit-content(<length>)` below: as wide as
        // the contents want, within what the container offers. `u32::MAX` is the
        // "no stated cap" case of the same thing.
        "fit-content" => return Some(LengthValue::FitContent(u32::MAX)),
        "max-content" => return Some(LengthValue::MaxContent),
        "auto" => return None,
        _ => {}
    }
    if let Some(inner) = value.strip_prefix("calc(").and_then(|s| s.strip_suffix(')'))
        && let Some(length) = parse_calc_length_value(inner, parent_font_size)
    {
        return Some(length);
    }
    for (name, kind) in [("min(", 0_u8), ("max(", 1), ("clamp(", 2)] {
        let Some(inner) = value.strip_prefix(name).and_then(|s| s.strip_suffix(')')) else {
            continue;
        };
        let parts: Vec<(i32, i32)> = split_at_top_level(inner, ',')
            .iter()
            .filter_map(|part| linear_length_form(part.trim(), parent_font_size))
            .collect();
        // `min()` and `max()` take any number of arguments; only the first two
        // are kept, which is what real stylesheets write.
        return match (kind, parts.as_slice()) {
            (0, [a, b, ..]) => Some(LengthValue::Bounded {
                lower: None,
                value: *a,
                upper: Some(*b),
            }),
            (1, [a, b, ..]) => Some(LengthValue::Bounded {
                lower: Some(*b),
                value: *a,
                upper: None,
            }),
            (2, [low, mid, high]) => Some(LengthValue::Bounded {
                lower: Some(*low),
                value: *mid,
                upper: Some(*high),
            }),
            _ => None,
        };
    }
    if let Some(inner) = value.strip_prefix("fit-content(").and_then(|s| s.strip_suffix(')')) {
        if let Some(px) = parse_length(inner, parent_font_size) {
            return Some(LengthValue::FitContent(px));
        }
    }
    if let Some(number) = value.strip_suffix('%') {
        return parse_float(number).map(|p| LengthValue::Percent(p.round().max(0.0) as u32));
    }
    parse_length(&value, parent_font_size).map(LengthValue::Pixels)
}

/// One argument of `min()` / `max()` / `clamp()` as "a share of the containing
/// block, plus an offset" -- the same shape `calc()` reduces to.
fn linear_length_form(input: &str, parent_font_size: u32) -> Option<(i32, i32)> {
    let value = input.trim();
    if let Some(number) = value.strip_suffix('%') {
        let percent = parse_float(number)?;
        return Some(((percent * 100.0).round() as i32, 0));
    }
    if let Some(inner) = value
        .strip_prefix("calc(")
        .and_then(|rest| rest.strip_suffix(')'))
        && let Some(length) = parse_calc_length_value(inner, parent_font_size)
    {
        return match length {
            LengthValue::Pixels(px) => Some((0, px.min(i32::MAX as u32) as i32)),
            LengthValue::Percent(percent) => Some((percent.min(i32::MAX as u32) as i32 * 100, 0)),
            LengthValue::Calc { percent_hundredths, px } => Some((percent_hundredths, px)),
            _ => None,
        };
    }
    parse_length_signed(value, parent_font_size).map(|px| (0, px))
}

fn parse_float(input: &str) -> Option<f32> {
    input.trim().parse::<f32>().ok()
}

// ─────────────────────────────────────────────────────────────────────────────
// Comment stripping
// ─────────────────────────────────────────────────────────────────────────────

/// Remove `/* ... */` comments, leaving everything else byte-for-byte.
///
/// Scanning by byte is safe here because UTF-8 is self-synchronizing: `/` and
/// `*` are ASCII, and no continuation byte of a multi-byte character can equal
/// an ASCII one, so a match is always a real comment delimiter sitting on a
/// character boundary. Copying by byte is *not* safe, and this used to do it:
/// `bytes[index] as char` reads a byte as a Latin-1 code point, which explodes
/// every multi-byte character into one bogus character per byte. Because the
/// stripper runs twice over a stylesheet, the damage compounded -- Yahoo!
/// JAPAN's `content: "\u{30fb}"` separators reached the screen as `Ã£Â\u{83}Â»`,
/// and every non-ASCII font-family name and selector value was mangled the
/// same way. Copy whole slices instead.
fn strip_comments(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut result = String::with_capacity(input.len());
    let mut index = 0;
    let mut copied = 0;

    while index + 1 < bytes.len() {
        if bytes[index] == b'/' && bytes[index + 1] == b'*' {
            result.push_str(&input[copied..index]);
            index += 2;
            while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/') {
                index += 1;
            }
            index = (index + 2).min(bytes.len());
            copied = index;
            continue;
        }
        index += 1;
    }

    result.push_str(&input[copied..]);
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
/// The first `url(...)` anywhere in a value.
///
/// `extract_url` wants the whole value to be one, which the shorthands are not:
/// `mask: url(...) no-repeat center / 1em 1em` carries the image among the
/// other components.
fn find_url(value: &str) -> Option<String> {
    let start = value.find("url(")?;
    let rest = &value[start..];
    let end = rest.find(')')?;
    extract_url(&rest[..=end])
}

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
        [ox, oy] => Some(TextShadow { offset_x: *ox, offset_y: *oy, blur: 0, color }),
        [ox, oy, blur, ..] => Some(TextShadow {
            offset_x: *ox,
            offset_y: *oy,
            blur: (*blur).max(0) as u32,
            color,
        }),
        _ => None,
    }
}

/// Parse a `linear-gradient(...)` value.
fn parse_linear_gradient(value: &str) -> Option<LinearGradient> {
    // Find the linear-gradient(...) part
    let lower = value.to_ascii_lowercase();
    // `repeating-` forms are read as their plain counterparts: the first pass
    // through the stops is the part that shows over most of a box anyway.
    let (start, radial, prefix) = match (lower.find("linear-gradient("), lower.find("radial-gradient(")) {
        (Some(linear), Some(radial_at)) if radial_at < linear => {
            (radial_at, true, "radial-gradient(")
        }
        (Some(linear), _) => (linear, false, "linear-gradient("),
        (None, Some(radial_at)) => (radial_at, true, "radial-gradient("),
        (None, None) => return None,
    };
    let after = &value[start + prefix.len()..];
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

    if radial {
        // A radial gradient's first argument may describe the shape, the size
        // and the centre. None of that is modelled -- the stops always run from
        // the middle to the farthest corner -- so it is stepped over rather than
        // read as a colour.
        const SHAPE_WORDS: [&str; 7] = [
            "circle",
            "ellipse",
            "closest-side",
            "closest-corner",
            "farthest-side",
            "farthest-corner",
            " at ",
        ];
        if SHAPE_WORDS.iter().any(|word| first_arg.contains(word.trim()))
            && parse_color(&first_arg).is_none()
        {
            arg_iter.next();
        }
        angle_deg_x1000 = 0;
    } else if first_arg.starts_with("to ") {
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
    } else if first_arg.starts_with("to") || first_arg.ends_with("deg") || first_arg.ends_with("turn") || first_arg.ends_with("rad") || first_arg.ends_with("grad") {
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
        let color_str;
        let mut pos_str: Option<String> = None;

        // Attempt: first join parts that belong to a function (rgb/rgba/hsl)
        let joined = parts.join(" ");
        // Try the whole joined string as color first, or look for position at end
        // Position is a numeric token ending with % or px
        let last = parts.last().unwrap();
        let second_last = if parts.len() >= 2 { Some(&parts[parts.len() - 2]) } else { None };

        let last_is_position = last.ends_with('%') || (last.ends_with("px") && parse_length(last, 16).is_some());
        let second_last_is_position = second_last.map(|s| s.ends_with('%') || s.ends_with("px")).unwrap_or(false);

        if last_is_position && parts.len() >= 2 {
            pos_str = Some(last.clone());
            color_str = parts[..parts.len() - 1].join(" ");
        } else if second_last_is_position && parts.len() >= 3 {
            pos_str = Some(second_last.unwrap().clone());
            color_str = parts[..parts.len() - 2].join(" ");
        } else {
            color_str = joined;
        }

        if let Some(c) = parse_color(color_str.trim()) {
            let pos = pos_str.and_then(|p| {
                let p = p.trim();
                if p.ends_with('%') {
                    p[..p.len()-1].parse::<f64>().ok().map(|v| (v * 10.0).round() as u32)
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
    let stops: Vec<(u32, u32)> = raw_stops.into_iter().enumerate().map(|(i, (c, p))| {
        let pos = p.unwrap_or_else(|| {
            if count == 1 {
                0
            } else {
                (1000 * i / (count - 1)) as u32
            }
        });
        (c, pos)
    }).collect();

    Some(LinearGradient { angle_deg_x1000, stops, radial })
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::rc::Rc;

    use super::{
        AlignItems, Display, FlexDirection, FlexWrap, GridEdge, GridTrackSize, JustifyContent,
        LengthValue,
        Position, RuleIndex, StyledElement, StyledNode, TableRole, VerticalAlign, WhiteSpaceMode,
        build_styled_tree, compute_style, parse_calc, parse_color, parse_inline_declarations,
        parse_length, parse_stylesheet, split_at_top_level,
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

    fn compare_indexed_and_naive_styles(
        node: &Node,
        styled: &StyledNode,
        stylesheet: &super::Stylesheet,
        ancestors: &[super::AncestorSlot],
        sibling_index: usize,
        sibling_count: usize,
        preceding_siblings: &[super::ElementIdentity],
        viewport_width: u32,
        interactive: &super::InteractiveState,
        parent_style: Option<&super::ComputedStyle>,
        parent_all_sibling_ids: Option<Rc<[super::ElementIdentity]>>,
    ) {
        match (node, styled) {
            (Node::Text(_), StyledNode::Text(_)) => {}
            (Node::Element(element), StyledNode::Element(styled_element)) => {
                let naive = super::compute_style_naive(
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
                assert_eq!(
                    naive,
                    *styled_element.style,
                    "style mismatch for <{} id={:?}>",
                    element.tag_name,
                    element.attributes.get("id")
                );

                let all_sibling_ids: Rc<[super::ElementIdentity]> = element
                    .children
                    .iter()
                    .filter_map(|c| if let Node::Element(e) = c { Some(super::ElementIdentity::from(e)) } else { None })
                    .collect::<Vec<_>>()
                    .into();
                let current_slot = super::AncestorSlot {
                    element: super::ElementIdentity::from(element),
                    sibling_index,
                    sibling_count,
                    siblings: parent_all_sibling_ids.unwrap_or_else(|| Rc::from(preceding_siblings)),
                    prec_count: sibling_index,
                    children: super::empty_siblings_rc(),
                };
                let mut next_ancestors = ancestors.to_vec();
                next_ancestors.push(current_slot);

                let mut elem_sibling_idx = 0;
                for (child, styled_child) in element.children.iter().zip(&styled_element.children) {
                    let (idx, count, prec_snap) = if matches!(child, Node::Element(_)) {
                        let idx = elem_sibling_idx;
                        elem_sibling_idx += 1;
                        (idx, all_sibling_ids.len(), &all_sibling_ids[..idx])
                    } else {
                        (0, 0, &all_sibling_ids[..0])
                    };
                    compare_indexed_and_naive_styles(
                        child,
                        styled_child,
                        stylesheet,
                        &next_ancestors,
                        idx,
                        count,
                        prec_snap,
                        viewport_width,
                        interactive,
                        Some(&styled_element.style),
                        Some(all_sibling_ids.clone()),
                    );
                }
            }
            _ => panic!("node shape mismatch"),
        }
    }

    #[test]
    fn one_number_after_flex_means_start_from_nothing() {
        // `flex: 1` is `1 1 0%`: the item starts from nothing and the whole
        // width is shared out by the grow factors. Left at `auto`, each item
        // kept its content width first and a row of three came out uneven.
        let document = crate::html::parse_document(
            "<div style=\"display:flex\"><span id=a style=\"flex:1\">a</span><span id=b style=\"flex:none\">b</span><span id=c style=\"flex:200px\">c</span></div>",
        );
        let styled = build_styled_tree(
            &document,
            &parse_stylesheet(""),
            1280,
            &super::InteractiveState::default(),
        );
        let by_id = |id: &str| {
            fn walk(node: &StyledNode, id: &str) -> Option<StyledElement> {
                match node {
                    StyledNode::Element(element) => {
                        if element.attributes.get("id").map(String::as_str) == Some(id) {
                            return Some(element.clone());
                        }
                        element.children.iter().find_map(|child| walk(child, id))
                    }
                    StyledNode::Text(_) => None,
                }
            }
            walk(&styled, id).expect("the span should exist")
        };
        assert_eq!(by_id("a").style.flex_grow, 100);
        assert_eq!(by_id("a").style.flex_basis, Some(LengthValue::Pixels(0)));
        assert_eq!(by_id("b").style.flex_grow, 0);
        assert_eq!(by_id("c").style.flex_basis, Some(LengthValue::Pixels(200)));
    }

    #[test]
    fn a_superscript_is_smaller_and_lifted() {
        let document = crate::html::parse_document("<p>H<sub>2</sub>O and x<sup>2</sup></p>");
        let styled = build_styled_tree(
            &document,
            &parse_stylesheet("body{font-size:16px}"),
            1280,
            &super::InteractiveState::default(),
        );
        let sup = find_first_element(&styled, "sup").expect("the sup should exist");
        let sub = find_first_element(&styled, "sub").expect("the sub should exist");
        assert_eq!(sup.style.font_size_px, 13);
        assert!(sup.style.baseline_shift < 0, "a superscript is lifted");
        assert!(sub.style.baseline_shift > 0, "a subscript is dropped");
    }

    #[test]
    fn a_table_cell_is_a_box_of_its_own() {
        // Read as plain inline, `display: table-cell` took neither a width nor
        // a height, so a layout built out of them collapsed to a line of text.
        let document = crate::html::parse_document(
            "<div style=\"display:table-cell;width:100px;height:25px\">cell</div>",
        );
        let styled = build_styled_tree(
            &document,
            &parse_stylesheet(""),
            1280,
            &super::InteractiveState::default(),
        );
        let cell = find_first_element(&styled, "div").expect("the cell should exist");
        // Block-level on the outside; which column it lands in and how wide it
        // ends up are the table layout's business, reached through the role.
        assert_eq!(cell.style.display, Display::Block);
        assert_eq!(cell.style.table_role, TableRole::Cell);
    }

    #[test]
    fn a_list_leaves_room_for_its_own_markers() {
        // Chrome puts a list's content 40px in. Without the padding the bullets
        // sat in the page margin and the text started at the left edge.
        let document = crate::html::parse_document("<ul><li>item</li></ul><dl><dd>def</dd></dl>");
        let styled = build_styled_tree(
            &document,
            &parse_stylesheet(""),
            1280,
            &super::InteractiveState::default(),
        );
        let list = find_first_element(&styled, "ul").expect("the list should exist");
        assert_eq!(list.style.padding.left, 40);
        let definition = find_first_element(&styled, "dd").expect("the dd should exist");
        assert_eq!(definition.style.margin.left, 40);
    }

    #[test]
    fn em_measures_against_the_element_s_own_font_size() {
        // `5em` on an element set to 20px is 100px. Reading it against the
        // parent made every box sized in em wrong wherever the element changed
        // its own size -- which is exactly where authors write em.
        let document = crate::html::parse_document(
            "<div style=\"width:5em;height:2em;font-size:20px\">x</div>",
        );
        let styled = build_styled_tree(
            &document,
            &parse_stylesheet("body{font-size:16px}"),
            1280,
            &super::InteractiveState::default(),
        );
        let element = find_first_element(&styled, "div").expect("the div should exist");
        assert_eq!(element.style.font_size_px, 20);
        assert_eq!(element.style.width, Some(LengthValue::Pixels(100)));
        assert_eq!(element.style.height, Some(LengthValue::Pixels(40)));
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

        let styled = build_styled_tree(&document, &stylesheet, 1280, &super::InteractiveState::default());
        let paragraph = find_first_element(&styled, "p").expect("paragraph should exist");

        assert_eq!(paragraph.style.color, 0x00AA00);
        assert_eq!(paragraph.style.font_size_px, 24);
        assert_eq!(paragraph.style.margin.top, 6);
        assert_eq!(paragraph.style.white_space, WhiteSpaceMode::Pre);
    }

    #[test]
    fn important_overrides_higher_specificity_normal_rule() {
        let document = parse_document("<div class=\"a\" id=\"x\">Hello</div>");
        let stylesheet = parse_stylesheet("#x { color: red; } .a { color: green !important; }");

        let styled = build_styled_tree(&document, &stylesheet, 1280, &super::InteractiveState::default());
        let div = find_first_element(&styled, "div").expect("div should exist");

        assert_eq!(div.style.color, 0x008000);
        let parsed = parse_inline_declarations("color: green !important;");
        assert_eq!(parsed[0].value, "green");
        assert!(parsed[0].important);
    }

    #[test]
    fn inline_important_beats_author_important() {
        let document = parse_document(
            "<div id=\"x\" style=\"color: blue !important;\">Hello</div>",
        );
        let stylesheet = parse_stylesheet("#x { color: red !important; }");

        let styled = build_styled_tree(&document, &stylesheet, 1280, &super::InteractiveState::default());
        let div = find_first_element(&styled, "div").expect("div should exist");

        assert_eq!(div.style.color, 0x0000FF);
    }

    #[test]
    fn logical_shorthand_keeps_a_calc_value_whole() {
        // Two values, the first a `calc()` with spaces in it. Split on plain
        // whitespace the declaration falls apart and both paddings come out 0 --
        // which is what flattened firefox.com's pre-footer.
        let document = parse_document("<div>x</div>");
        let stylesheet = parse_stylesheet(
            ":root { --k: 180px; --v: 64px } div { padding-block: calc(var(--k) + var(--v) * 2) var(--v) }",
        );

        let styled = build_styled_tree(&document, &stylesheet, 1280, &super::InteractiveState::default());
        let div = find_first_element(&styled, "div").expect("div should exist");

        assert_eq!(div.style.padding.top, 308);
        assert_eq!(div.style.padding.bottom, 64);
    }

    #[test]
    fn a_link_pseudo_class_still_selects() {
        // firefox.com styles its whole footer through
        // `.fl-footer a:link:not(.fl-button)`. Dropped for the `:link`, every
        // footer link kept the default blue underline instead of the white the
        // page asks for.
        let document = parse_document("<div class=\"f\"><a href=\"/x\">go</a></div>");
        let stylesheet =
            parse_stylesheet(".f a:link:not(.b) { color: #ffffff; text-decoration: none }");

        let styled = build_styled_tree(&document, &stylesheet, 1280, &super::InteractiveState::default());
        let link = find_first_element(&styled, "a").expect("a should exist");

        assert_eq!(link.style.color, 0xFFFFFF);
        assert!(!link.style.underline);
    }

    #[test]
    fn a_visited_rule_does_not_apply_when_nothing_has_been_visited() {
        let document = parse_document("<a href=\"/x\">go</a>");
        let stylesheet = parse_stylesheet("a:visited { color: #ff0000 }");

        let styled = build_styled_tree(&document, &stylesheet, 1280, &super::InteractiveState::default());
        let link = find_first_element(&styled, "a").expect("a should exist");

        assert_ne!(link.style.color, 0xFF0000, "no history means nothing is visited");
    }

    #[test]
    fn inset_sets_all_four_offsets() {
        let document = parse_document("<div>x</div>");
        let stylesheet = parse_stylesheet("div { position: absolute; inset: 10px 20px 30px 40px }");

        let styled = build_styled_tree(&document, &stylesheet, 1280, &super::InteractiveState::default());
        let div = find_first_element(&styled, "div").expect("div should exist");

        assert_eq!(div.style.top, Some(LengthValue::Pixels(10)));
        assert_eq!(div.style.right, Some(LengthValue::Pixels(20)));
        assert_eq!(div.style.bottom, Some(LengthValue::Pixels(30)));
        assert_eq!(div.style.left, Some(LengthValue::Pixels(40)));
    }

    #[test]
    fn inset_repeats_its_values_the_way_margin_does() {
        let document = parse_document("<div>x</div>");
        let stylesheet = parse_stylesheet("div { position: absolute; inset: 5px 15px }");

        let styled = build_styled_tree(&document, &stylesheet, 1280, &super::InteractiveState::default());
        let div = find_first_element(&styled, "div").expect("div should exist");

        assert_eq!(div.style.top, Some(LengthValue::Pixels(5)));
        assert_eq!(div.style.bottom, Some(LengthValue::Pixels(5)));
        assert_eq!(div.style.left, Some(LengthValue::Pixels(15)));
        assert_eq!(div.style.right, Some(LengthValue::Pixels(15)));
    }

    #[test]
    fn a_pseudo_element_inherits_rather_than_copies() {
        // `display` does not inherit. Copied from the host, a `::before` on a
        // flex row became a flex container itself and took a paint path that
        // never drew its background -- which is how firefox.com lost the
        // gradient over the whole lower half of its front page.
        let document = parse_document("<div class=\"row\">x</div>");
        let stylesheet = parse_stylesheet(
            ".row { display: flex; color: #00ff00 }              .row::before { content: \"\"; background: linear-gradient(#ff0000, #ff0000) }",
        );

        let styled = build_styled_tree(&document, &stylesheet, 1280, &super::InteractiveState::default());
        let row = find_first_element(&styled, "div").expect("div should exist");
        let StyledNode::Element(pseudo) = &row.children[0] else {
            panic!("the ::before should have become a box");
        };

        assert_ne!(pseudo.style.display, Display::Flex, "display must not be inherited");
        // Colour does inherit, and the pseudo's own rules still apply.
        assert_eq!(pseudo.style.color, 0x00FF00);
        assert!(pseudo.style.background_gradient.is_some());
    }

    #[test]
    fn a_pseudo_element_reads_root_variables() {
        // The gradient over firefox.com's lower half is gated behind
        // `content: var(--content-dark-mode-only)`, declared on `:root`.
        // Looking only at the host's own properties left it unknown, and an
        // unresolvable `var()` drops the declaration -- so the pseudo-element
        // never came into being.
        let document = parse_document("<div class=\"a\">x</div>");
        let stylesheet = parse_stylesheet(
            ":root { --flag: \"\"; --art: url(/art.svg) }              .a::before { content: var(--flag); background: var(--art) }",
        );

        let styled = build_styled_tree(&document, &stylesheet, 1280, &super::InteractiveState::default());
        let host = find_first_element(&styled, "div").expect("div should exist");
        let StyledNode::Element(pseudo) = &host.children[0] else {
            panic!("the ::before should have become a box");
        };

        assert_eq!(pseudo.style.background_image_url.as_deref(), Some("/art.svg"));
    }

    #[test]
    fn a_pseudo_element_with_a_picture_becomes_a_box() {
        // firefox.com draws the fox under its hero entirely through
        // `.fl-home-intro::before`: empty content, a background image, and a
        // size written with custom properties. As a text node it painted
        // nothing, and with `var()` left unresolved it had no size either.
        let document = parse_document("<section class=\"hero\"><p>x</p></section>");
        let stylesheet = parse_stylesheet(
            ".hero { --kit-width: 940px }              .hero::before { content: \"\"; position: absolute;              inline-size: var(--kit-width, 580px);              background: url(/media/fox.svg) }",
        );

        let styled = build_styled_tree(&document, &stylesheet, 1280, &super::InteractiveState::default());
        let hero = find_first_element(&styled, "section").expect("section should exist");
        let StyledNode::Element(pseudo) = &hero.children[0] else {
            panic!("::before should be an element, not text");
        };

        assert_eq!(pseudo.style.background_image_url.as_deref(), Some("/media/fox.svg"));
        assert_eq!(pseudo.style.width, Some(LengthValue::Pixels(940)));
    }

    #[test]
    fn a_text_only_pseudo_element_stays_text() {
        let document = parse_document("<p>x</p>");
        let stylesheet = parse_stylesheet("p::before { content: \"> \" }");

        let styled = build_styled_tree(&document, &stylesheet, 1280, &super::InteractiveState::default());
        let paragraph = find_first_element(&styled, "p").expect("p should exist");

        assert!(
            matches!(&paragraph.children[0], StyledNode::Text(t) if t.text == "> "),
            "a pseudo-element with nothing to paint must not grow a box"
        );
    }

    #[test]
    fn selector_list_is_scored_by_its_most_specific_match() {
        // Both halves match; only the second carries the `:first-child` that
        // outweighs the later rule. firefox.com's hero padding rides on this.
        let document = parse_document("<section class=\"home\"><div class=\"intro\">x</div></section>");
        let stylesheet = parse_stylesheet(
            ".home .intro, .home .intro:first-child { color: green } .intro:first-child { color: red }",
        );

        let styled = build_styled_tree(&document, &stylesheet, 1280, &super::InteractiveState::default());
        let intro = find_first_element(&styled, "div").expect("intro should exist");

        assert_eq!(intro.style.color, 0x008000);
    }

    #[test]
    fn where_contributes_nothing_to_specificity() {
        let document = parse_document("<section class=\"home\"><div class=\"intro\">x</div></section>");
        // `:where(.home)` scores zero, leaving a bare `.intro` -- one class,
        // so it loses to the two-class rule written before it.
        let stylesheet =
            parse_stylesheet(".home .intro { color: green } :where(.home) .intro { color: red }");

        let styled = build_styled_tree(&document, &stylesheet, 1280, &super::InteractiveState::default());
        let intro = find_first_element(&styled, "div").expect("intro should exist");

        assert_eq!(intro.style.color, 0x008000);
    }

    #[test]
    fn higher_specificity_normal_rule_still_wins_without_important() {
        let document = parse_document("<div class=\"a\" id=\"x\">Hello</div>");
        let stylesheet = parse_stylesheet("#x { color: red; } .a { color: green; }");

        let styled = build_styled_tree(&document, &stylesheet, 1280, &super::InteractiveState::default());
        let div = find_first_element(&styled, "div").expect("div should exist");

        assert_eq!(div.style.color, 0xFF0000);
    }

    #[test]
    fn parses_important_with_spaces_and_case_insensitive_keyword() {
        let decls = parse_inline_declarations("color: rgb(1, 2, 3) ! IMPORTANT ;");
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].value, "rgb(1, 2, 3)");
        assert!(decls[0].important);
    }

    #[test]
    fn substitutes_var_with_nested_fallback_parentheses() {
        let mut vars = BTreeMap::new();
        vars.insert("--x".to_string(), "teal".to_string());
        assert_eq!(
            super::substitute_vars("var(--x, rgb(1,2,3))", &vars).as_deref(),
            Some("teal")
        );

        let empty = BTreeMap::new();
        assert_eq!(
            super::substitute_vars("var(--x, rgb(1,2,3))", &empty).as_deref(),
            Some("rgb(1,2,3)")
        );
        // Undefined with no fallback is a guaranteed-invalid value, not an empty
        // string: the declaration that used it has to be dropped.
        assert_eq!(super::substitute_vars("var(--y)", &empty), None);
        assert_eq!(
            super::substitute_vars("var(--y, blue)", &empty).as_deref(),
            Some("blue")
        );
    }

    #[test]
    fn supports_descendant_and_child_selectors() {
        let document = parse_document(
            "<section class=\"outer\"><div><p id=\"direct\">A</p></div><p id=\"nested\">B</p></section>",
        );
        let stylesheet =
            parse_stylesheet(".outer > p { color: red; } .outer div p { display: none; }");
        let styled = build_styled_tree(&document, &stylesheet, 1280, &super::InteractiveState::default());

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
        let styled = build_styled_tree(&document, &stylesheet, 1280, &super::InteractiveState::default());

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
        let styled = build_styled_tree(&document, &stylesheet, 1280, &super::InteractiveState::default());

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
        let styled = build_styled_tree(&document, &stylesheet, 1280, &super::InteractiveState::default());

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
        let styled = build_styled_tree(&document, &stylesheet, 1280, &super::InteractiveState::default());

        let target = find_element_by_id(&styled, "target").expect("target paragraph should exist");
        assert_eq!(target.style.color, 0xFF0000);
    }

    #[test]
    fn supports_general_sibling_then_child_combinator() {
        let document = parse_document(
            "<body><h1></h1><p></p><div><span id=\"target\"></span></div></body>",
        );
        let stylesheet = parse_stylesheet("h1 ~ div > span { color: #00ff00; }");
        let styled = build_styled_tree(&document, &stylesheet, 1280, &super::InteractiveState::default());

        let target = find_element_by_id(&styled, "target").expect("target span should exist");
        assert_eq!(target.style.color, 0x00FF00);
    }

    #[test]
    fn applies_legacy_html_attributes() {
        let document = parse_document(
            "<body bgcolor=\"#f0f0ff\"><h1 align=\"center\">Title</h1><font color=\"#ff0000\">red</font></body>",
        );
        let styled = build_styled_tree(&document, &super::Stylesheet::default(), 1280, &super::InteractiveState::default());

        let body = find_first_element(&styled, "body").expect("body should exist");
        let heading = find_first_element(&styled, "h1").expect("heading should exist");
        let font = find_first_element(&styled, "font").expect("font should exist");

        assert_eq!(body.style.background_color, Some(0xF0F0FF));
        assert_eq!(heading.style.text_align, super::TextAlign::Center);
        assert_eq!(font.style.color, 0xFF0000);
    }

    #[test]
    fn table_align_does_not_inherit_as_text_align() {
        let document = parse_document(
            "<table align=\"center\"><tr><td>text</td></tr></table><div align=\"center\">div</div><table><tr><td id=\"cell\" align=\"center\">cell</td></tr></table>",
        );
        let styled = build_styled_tree(&document, &super::Stylesheet::default(), 1280, &super::InteractiveState::default());
        let table = find_first_element(&styled, "table").expect("table should exist");
        let div = find_first_element(&styled, "div").expect("div should exist");
        let cell = find_element_by_id(&styled, "cell").expect("centered cell should exist");

        assert_eq!(table.style.text_align, super::TextAlign::Left);
        assert_eq!(div.style.text_align, super::TextAlign::Center);
        assert_eq!(cell.style.text_align, super::TextAlign::Center);
    }

    #[test]
    fn table_cells_default_to_middle_valign_but_attribute_can_override() {
        let document =
            parse_document("<table><tr><td>middle</td><th>head</th><td id=\"top\" valign=\"top\">top</td></tr></table>");
        let styled = build_styled_tree(&document, &super::Stylesheet::default(), 1280, &super::InteractiveState::default());
        let first_cell = find_first_element(&styled, "td").expect("td should exist");
        let header_cell = find_first_element(&styled, "th").expect("th should exist");
        let top_cell = find_element_by_id(&styled, "top").expect("top cell should exist");

        assert_eq!(first_cell.style.vertical_align, VerticalAlign::Middle);
        assert_eq!(header_cell.style.vertical_align, VerticalAlign::Middle);
        assert_eq!(top_cell.style.vertical_align, VerticalAlign::Top);
    }

    #[test]
    fn applies_css_and_legacy_width_height_and_valign() {
        let document = parse_document(
            "<table><tr><td width=\"120\" height=\"40\" valign=\"bottom\" style=\"width: 60%;\">Hello</td></tr></table>",
        );
        let stylesheet = parse_stylesheet("td { vertical-align: middle; }");
        let styled = build_styled_tree(&document, &stylesheet, 1280, &super::InteractiveState::default());
        let cell = find_first_element(&styled, "td").expect("cell should exist");

        assert_eq!(cell.style.width, Some(LengthValue::Percent(60)));
        assert_eq!(cell.style.height, Some(LengthValue::Pixels(40)));
        assert_eq!(cell.style.vertical_align, VerticalAlign::Middle);
    }

    #[test]
    fn indexed_style_matches_naive_style_for_complex_selector_mix() {
        let document = parse_document(
            "<div id=\"root\" class=\"shell a\" data-x=\"abc\" data-y=\"prefix-mid-suffix\" title=\"hello-world\">\
                <section id=\"sec\" class=\"panel a b\" data-x=\"abacus\" data-z=\"z1\">\
                    <h1 id=\"title\" class=\"head a\">Title</h1>\
                    <p id=\"p1\" class=\"a b\" data-x=\"abc\" data-y=\"prefix-mid-suffix\" data-flag>One</p>\
                    <p id=\"p2\" class=\"b\" data-x=\"zzz\" data-y=\"nope\" title=\"hello-world\">Two</p>\
                    <span id=\"s1\" class=\"a b c\" data-x=\"abc\" data-y=\"suffix\" data-k=\"v\">Three</span>\
                    <div id=\"wrap\" class=\"b\">\
                        <span id=\"s2\" class=\"a\" data-x=\"abc\" data-y=\"prefix\">Four</span>\
                        <span id=\"s3\" class=\"c\" data-x=\"xyz\">Five</span>\
                    </div>\
                </section>\
                <footer id=\"foot\" class=\"a\" data-x=\"abc\"><em id=\"em1\" class=\"b\">Six</em></footer>\
            </div>",
        );
        let stylesheet = parse_stylesheet(
            r#"
                * { margin: 1px; }
                [data-flag] { display: none; }
                [data-x] { color: #111111; }
                [data-x^=ab] { color: #222222; }
                [data-y$=suffix] { background-color: #333333; }
                [data-y*=mid] { font-size: 18px; }
                [title=hello-world] { white-space: pre; }
                #root { margin: 4px; }
                div.shell { margin-top: 7px; }
                section.panel > h1 { color: #ff0000; }
                section.panel p + p { color: #00ff00; }
                section.panel p ~ span { background-color: #0000ff; }
                section.panel div > span { font-size: 20px; }
                div section span.a.b { color: #aa00aa; }
                div section span.a.b:not(.c) { margin-left: 9px; }
                div section :nth-child(2) { margin-right: 11px; }
                div section :first-child { padding: 2px; }
                div section :last-child { padding: 3px; }
                div section span, p::before, span::after { border-width: 5px; }
                p::before { content: "x"; }
                span::after { content: attr(data-x); }
                em { color: #0f0f0f; }
            "#,
        );
        let optimized = build_styled_tree(&document, &stylesheet, 1280, &super::InteractiveState::default());
        compare_indexed_and_naive_styles(
            &document,
            &optimized,
            &stylesheet,
            &[],
            0,
            0,
            &[],
            1280,
            &super::InteractiveState::default(),
            None,
            None,
        );
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
        let styled = build_styled_tree(&document, &stylesheet, 1280, &super::InteractiveState::default());
        let a = find_first_element(&styled, "a").expect("a should exist");
        let span = find_first_element(&styled, "span").expect("span should exist");
        assert_eq!(a.style.color, 0xFF0000);
        assert_ne!(span.style.color, 0xFF0000);
    }

    #[test]
    fn attribute_equals_selector_matches() {
        let document = parse_document("<input type=\"text\"><input type=\"checkbox\">");
        let stylesheet = parse_stylesheet("[type=text] { color: #00ff00; }");
        let styled = build_styled_tree(&document, &stylesheet, 1280, &super::InteractiveState::default());
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
        let styled = build_styled_tree(&document, &stylesheet, 1280, &super::InteractiveState::default());
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
        let styled = build_styled_tree(&document, &stylesheet, 1280, &super::InteractiveState::default());
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
        let styled = build_styled_tree(&document, &stylesheet, 1280, &super::InteractiveState::default());
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
        let styled = build_styled_tree(&document, &stylesheet, 1280, &super::InteractiveState::default());
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
        let styled = build_styled_tree(&document, &stylesheet, 1280, &super::InteractiveState::default());
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
        let styled = build_styled_tree(&document, &stylesheet, 1280, &super::InteractiveState::default());
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
        let styled_wide = build_styled_tree(&document, &stylesheet, 1280, &super::InteractiveState::default());
        let p_wide = find_first_element(&styled_wide, "p").unwrap();
        assert_eq!(
            p_wide.style.color, 0x0000FF,
            "wide viewport: plain rule wins"
        );

        // Viewport 400 → max-width 600 rule SHOULD apply and wins (later in source)
        let styled_narrow = build_styled_tree(&document, &stylesheet, 400, &super::InteractiveState::default());
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
        let styled = build_styled_tree(&document, &stylesheet, 1280, &super::InteractiveState::default());
        let a = find_first_element(&styled, "p").unwrap();
        // Both rules inside @media screen should be parsed (screen always applies)
        assert_eq!(
            a.style.color, 0xFF0000,
            "first rule inside @media should apply"
        );
    }

    // ── calc() tests ──────────────────────────────────────────────────────────

    #[test]
    fn media_supports_and_conditions() {
        let document = parse_document("<p>Hello</p>");
        let stylesheet = parse_stylesheet(
            "p { color: #0000ff; } @media screen and (max-width: 768px) { p { color: #ff0000; } }",
        );

        let styled_700 = build_styled_tree(&document, &stylesheet, 700, &super::InteractiveState::default());
        assert_eq!(find_first_element(&styled_700, "p").unwrap().style.color, 0xFF0000);

        let styled_800 = build_styled_tree(&document, &stylesheet, 800, &super::InteractiveState::default());
        assert_eq!(find_first_element(&styled_800, "p").unwrap().style.color, 0x0000FF);
    }

    #[test]
    fn media_supports_and_ranges() {
        let document = parse_document("<p>Hello</p>");
        let stylesheet = parse_stylesheet(
            "@media (min-width: 768px) and (max-width: 1024px) { p { color: #ff0000; } }",
        );

        let styled_768 = build_styled_tree(&document, &stylesheet, 768, &super::InteractiveState::default());
        assert_eq!(find_first_element(&styled_768, "p").unwrap().style.color, 0xFF0000);

        let styled_900 = build_styled_tree(&document, &stylesheet, 900, &super::InteractiveState::default());
        assert_eq!(find_first_element(&styled_900, "p").unwrap().style.color, 0xFF0000);

        let styled_700 = build_styled_tree(&document, &stylesheet, 700, &super::InteractiveState::default());
        assert_ne!(find_first_element(&styled_700, "p").unwrap().style.color, 0xFF0000);

        let styled_1200 = build_styled_tree(&document, &stylesheet, 1200, &super::InteractiveState::default());
        assert_ne!(find_first_element(&styled_1200, "p").unwrap().style.color, 0xFF0000);
    }

    #[test]
    fn media_supports_comma_separated_or_conditions() {
        let document = parse_document("<p>Hello</p>");
        let stylesheet = parse_stylesheet(
            "@media (max-width: 480px), (min-width: 1200px) { p { color: #ff0000; } }",
        );

        let styled_400 = build_styled_tree(&document, &stylesheet, 400, &super::InteractiveState::default());
        assert_eq!(find_first_element(&styled_400, "p").unwrap().style.color, 0xFF0000);

        let styled_800 = build_styled_tree(&document, &stylesheet, 800, &super::InteractiveState::default());
        assert_ne!(find_first_element(&styled_800, "p").unwrap().style.color, 0xFF0000);

        let styled_1300 = build_styled_tree(&document, &stylesheet, 1300, &super::InteractiveState::default());
        assert_eq!(find_first_element(&styled_1300, "p").unwrap().style.color, 0xFF0000);
    }

    #[test]
    fn media_supports_not_conditions() {
        let document = parse_document("<p>Hello</p>");
        let stylesheet = parse_stylesheet(
            "@media not (max-width: 600px) { p { color: #ff0000; } }",
        );

        let styled_700 = build_styled_tree(&document, &stylesheet, 700, &super::InteractiveState::default());
        assert_eq!(find_first_element(&styled_700, "p").unwrap().style.color, 0xFF0000);

        let styled_500 = build_styled_tree(&document, &stylesheet, 500, &super::InteractiveState::default());
        assert_ne!(find_first_element(&styled_500, "p").unwrap().style.color, 0xFF0000);
    }

    #[test]
    fn media_single_condition_regression_still_works() {
        let document = parse_document("<p>Hello</p>");
        let stylesheet = parse_stylesheet("@media (max-width: 600px) { p { color: #ff0000; } }");

        let styled_500 = build_styled_tree(&document, &stylesheet, 500, &super::InteractiveState::default());
        assert_eq!(find_first_element(&styled_500, "p").unwrap().style.color, 0xFF0000);

        let styled_700 = build_styled_tree(&document, &stylesheet, 700, &super::InteractiveState::default());
        assert_ne!(find_first_element(&styled_700, "p").unwrap().style.color, 0xFF0000);
    }

    #[test]
    fn calc_addition_and_subtraction() {
        let document = parse_document("<p>text</p>");
        let stylesheet = parse_stylesheet("p { font-size: calc(10px + 6px); }");
        let styled = build_styled_tree(&document, &stylesheet, 1280, &super::InteractiveState::default());
        let p = find_first_element(&styled, "p").unwrap();
        assert_eq!(p.style.font_size_px, 16);
    }

    #[test]
    fn calc_multiplication_has_higher_precedence_than_addition() {
        // calc(2px + 3 * 4px) should be 2 + 12 = 14, NOT (2+3)*4 = 20
        let document = parse_document("<p>text</p>");
        let stylesheet = parse_stylesheet("p { font-size: calc(2px + 3 * 4px); }");
        let styled = build_styled_tree(&document, &stylesheet, 1280, &super::InteractiveState::default());
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
        let styled = build_styled_tree(&document, &stylesheet, 1280, &super::InteractiveState::default());
        let p = find_first_element(&styled, "p").unwrap();
        assert_eq!(p.style.font_size_px, 24);
    }

    /// A width like `calc(100% - var(--outline)*2)` reduces to a percentage and
    /// a pixel offset. Multiplication used to make the whole value unparseable,
    /// so the element fell back to `width: auto`.
    #[test]
    fn calc_length_value_handles_multiplication() {
        assert_eq!(
            super::parse_calc_length_value("100% - 2px*2", 16),
            Some(LengthValue::Calc {
                percent_hundredths: 10000,
                px: -4
            })
        );
    }

    /// `top: calc(var(--offset) * -1)` parks an element off-screen. Clamping the
    /// result to zero is what left MDN's skip link sitting on the page.
    #[test]
    fn calc_negative_offset_stays_negative() {
        let expected = Some(LengthValue::Calc {
            percent_hundredths: 0,
            px: -320,
        });
        // The stylesheet writes it without spaces; both forms are valid calc().
        assert_eq!(super::parse_calc_length_value("20em*-1", 16), expected);
        assert_eq!(super::parse_calc_length_value("20em * -1", 16), expected);
    }

    /// Division, and `rem` resolved against the root font size rather than the
    /// parent's. MDN's mandala uses `calc(var(--height)*-1/4)`.
    #[test]
    fn calc_length_value_handles_division_and_rem() {
        assert_eq!(
            super::parse_calc_length_value("2rem/4", 40),
            Some(LengthValue::Pixels(8))
        );
        assert_eq!(
            super::parse_calc_length_value("35rem*-1/4", 16),
            Some(LengthValue::Calc {
                percent_hundredths: 0,
                px: -140
            })
        );
    }

    /// Two lengths multiplied together is not a valid calc() term.
    #[test]
    fn calc_rejects_length_times_length() {
        assert_eq!(super::parse_calc_length_value("2px*3px", 16), None);
        assert_eq!(super::parse_calc_length_value("100%/2px", 16), None);
    }

    /// `min()`, `max()` and `clamp()` keep their percentages until the
    /// containing block is known, the same as `calc()` does.
    ///
    /// Unparsed they collapsed to almost nothing: firefox.com caps a banner's
    /// text column with `max-inline-size: min(600px, 100%)`, and at 16px wide it
    /// stacked a 64px heading one character to a line, running the section to
    /// twelve hundred pixels.
    #[test]
    fn min_max_and_clamp_resolve_against_the_containing_block() {
        let bounded = |input: &str, container: u32| match super::parse_length_value(input, 16) {
            Some(LengthValue::Bounded { lower, value, upper }) => {
                super::resolve_bounded(lower, value, upper, container)
            }
            other => panic!("{input} did not parse as a bounded length: {other:?}"),
        };
        assert_eq!(bounded("min(600px, 100%)", 1000), 600);
        assert_eq!(bounded("min(600px, 100%)", 400), 400);
        assert_eq!(bounded("max(200px, 30%)", 1000), 300);
        assert_eq!(bounded("max(200px, 30%)", 100), 200);
        assert_eq!(bounded("clamp(100px, 50%, 400px)", 1000), 400);
        assert_eq!(bounded("clamp(100px, 50%, 400px)", 400), 200);
        assert_eq!(bounded("clamp(100px, 50%, 400px)", 100), 100);
    }

    #[test]
    fn calc_vh_uses_800px_base() {
        // 50vh should resolve to 400px (50% of 800px viewport height)
        // This locks the vh base against parse_length's viewport-unit handling
        let result = parse_length("calc(50vh)", 16);
        assert_eq!(result, Some(400));
    }

    #[test]
    fn calc_invalid_trailing_operator_returns_none() {
        assert_eq!(parse_calc("2 *", 16), None);
        assert_eq!(parse_calc("100% *", 16), None);
    }

    #[test]
    fn calc_valid_expression_still_evaluates() {
        assert_eq!(parse_calc("2 * 3 + 1", 16), Some(7));
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
    fn rgba_half_transparent_encodes_alpha_in_high_byte() {
        let color = parse_color("rgba(0, 0, 0, 0.5)").expect("should return a color");
        let alpha = (color >> 24) & 0xFF;
        assert!((alpha as i32 - 128).abs() <= 1, "alpha should be ~128, got {alpha}");
        assert_eq!(color & 0x00FF_FFFF, 0x0000_0000);
    }

    #[test]
    fn rgba_hex_with_alpha_uses_high_byte() {
        assert_eq!(parse_color("#1234"), Some(0x4411_2233));
        assert_eq!(parse_color("#11223344"), Some(0x4411_2233));
    }

    #[test]
    fn rgba_hex_with_zero_alpha_returns_none() {
        assert_eq!(parse_color("#1230"), None);
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
    fn parse_stylesheet_ignores_closing_brace_inside_string() {
        let stylesheet = parse_stylesheet(r#".a::after { content: "}"; color: #ff0000; }"#);
        assert_eq!(stylesheet.rules.len(), 1);
        assert_eq!(stylesheet.rules[0].selectors.len(), 1);
        assert_eq!(stylesheet.rules[0].declarations.len(), 2);
        assert_eq!(stylesheet.rules[0].declarations[1].property, "color");
        assert_eq!(stylesheet.rules[0].declarations[1].value, "#ff0000");
    }

    #[test]
    fn parse_stylesheet_ignores_opening_brace_inside_string() {
        let stylesheet = parse_stylesheet(r#".b { content: "{"; color: #00ff00; } .c { color: #0000ff; }"#);
        assert_eq!(stylesheet.rules.len(), 2);
        assert_eq!(
            stylesheet.rules[0]
                .declarations
                .iter()
                .find(|decl| decl.property == "color")
                .unwrap()
                .value,
            "#00ff00"
        );
        assert_eq!(
            stylesheet.rules[1]
                .declarations
                .iter()
                .find(|decl| decl.property == "color")
                .unwrap()
                .value,
            "#0000ff"
        );
    }

    #[test]
    fn parse_stylesheet_still_handles_normal_rules() {
        let stylesheet = parse_stylesheet("p { color: #123456; }");
        assert_eq!(stylesheet.rules.len(), 1);
        assert_eq!(stylesheet.rules[0].declarations.len(), 1);
        assert_eq!(stylesheet.rules[0].declarations[0].property, "color");
        assert_eq!(stylesheet.rules[0].declarations[0].value, "#123456");
    }

    #[test]
    fn not_pseudo_class_selector_matches() {
        let document = parse_document("<p class=\"a\">A</p><p class=\"b\">B</p>");
        let stylesheet = parse_stylesheet("p:not(.a) { color: #ff0000; }");
        let styled = build_styled_tree(&document, &stylesheet, 1280, &super::InteractiveState::default());
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
        let styled = build_styled_tree(&document, &stylesheet, 1280, &super::InteractiveState::default());
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
        assert_eq!(p_el.style.color, 0xff0000, "p color should be #ff0000 from :root var");
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
    fn root_opacity_zero_is_clamped_but_others_honored() {
        fn opacity_of(tag: &str, css: &str) -> u8 {
            let ss = parse_stylesheet(css);
            let el = Element { namespace: Default::default(),
                tag_name: tag.into(),
                attributes: Default::default(),
                children: vec![],
            };
            let idx = RuleIndex::build(&ss.rules);
            compute_style(&el, &ss, &idx, None, &[], 0, 1, &[], 1280, &super::InteractiveState::default())
                .opacity
        }
        // Anti-FOUC: a transparent root would blank the whole page, so clamp it.
        assert_eq!(opacity_of("body", "body { opacity: 0; }"), 255);
        assert_eq!(opacity_of("html", "html { opacity: 0; }"), 255);
        assert_eq!(opacity_of("body", "body { visibility: hidden; }"), 255);
        // Non-root elements still honor opacity:0, and a partially transparent
        // root keeps its real value (only a fully-transparent root is clamped).
        assert_eq!(opacity_of("div", "div { opacity: 0; }"), 0);
        assert_eq!(opacity_of("body", "body { opacity: 0.5; }"), 128);
    }

    #[test]
    fn test_position_relative_parsed() {
        let ss = parse_stylesheet("div { position: relative; top: 10px; left: 20px; }");
        let el = Element { namespace: Default::default(), tag_name: "div".into(), attributes: Default::default(), children: vec![] };
        let rule_index = RuleIndex::build(&ss.rules);
        let style = compute_style(&el, &ss, &rule_index, None, &[], 0, 1, &[], 1280, &super::InteractiveState::default());
        assert_eq!(style.position, Position::Relative);
        assert_eq!(style.top, Some(LengthValue::Pixels(10)));
        assert_eq!(style.left, Some(LengthValue::Pixels(20)));
    }

    #[test]
    fn test_position_absolute_parsed() {
        let ss = parse_stylesheet("div { position: absolute; top: 0px; }");
        let el = Element { namespace: Default::default(), tag_name: "div".into(), attributes: Default::default(), children: vec![] };
        let rule_index = RuleIndex::build(&ss.rules);
        let style = compute_style(&el, &ss, &rule_index, None, &[], 0, 1, &[], 1280, &super::InteractiveState::default());
        assert_eq!(style.position, Position::Absolute);
    }

    #[test]
    fn test_flex_display_parsed() {
        let ss = parse_stylesheet("div { display: flex; flex-direction: column; gap: 8px; }");
        let el = Element { namespace: Default::default(), tag_name: "div".into(), attributes: Default::default(), children: vec![] };
        let rule_index = RuleIndex::build(&ss.rules);
        let style = compute_style(&el, &ss, &rule_index, None, &[], 0, 1, &[], 1280, &super::InteractiveState::default());
        assert_eq!(style.display, Display::Flex);
        assert_eq!(style.flex_direction, FlexDirection::Column);
        assert_eq!(style.gap, 8);
    }

    #[test]
    fn test_justify_content_parsed() {
        let ss = parse_stylesheet("div { display: flex; justify-content: space-between; align-items: center; }");
        let el = Element { namespace: Default::default(), tag_name: "div".into(), attributes: Default::default(), children: vec![] };
        let rule_index = RuleIndex::build(&ss.rules);
        let style = compute_style(&el, &ss, &rule_index, None, &[], 0, 1, &[], 1280, &super::InteractiveState::default());
        assert_eq!(style.justify_content, JustifyContent::SpaceBetween);
        assert_eq!(style.align_items, AlignItems::Center);
    }

    #[test]
    fn test_z_index_parsed() {
        let ss = parse_stylesheet("div { position: absolute; z-index: 10; }");
        let el = Element { namespace: Default::default(), tag_name: "div".into(), attributes: Default::default(), children: vec![] };
        let rule_index = RuleIndex::build(&ss.rules);
        let style = compute_style(&el, &ss, &rule_index, None, &[], 0, 1, &[], 1280, &super::InteractiveState::default());
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
        let styled_no_hover = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let a_no_hover = find_first_element(&styled_no_hover, "a").expect("<a> should exist");
        assert_ne!(a_no_hover.style.color, 0xFF0000, "color should not be red without hover");

        // With hover on node 42: link color should become red
        let interactive = super::InteractiveState {
            hovered_node_id: Some(42),
            ..Default::default()
        };
        let styled_hovered = build_styled_tree(&doc, &sheet, 1280, &interactive);
        let a_hovered = find_first_element(&styled_hovered, "a").expect("<a> should exist");
        assert_eq!(a_hovered.style.color, 0xFF0000, "color should be red when hovered");
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
        use super::{GridTrackSize};
        let html = r#"<div style="display:grid;grid-template-columns:100px 1fr 200px;"></div>"#;
        let doc = parse_document(html);
        let sheet = parse_stylesheet("");
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let div = find_first_element(&styled, "div").unwrap();
        assert_eq!(div.style.display, Display::Grid);
        assert_eq!(div.style.grid_template_columns.len(), 3);
        assert_eq!(div.style.grid_template_columns[0], GridTrackSize::Pixels(100));
        assert_eq!(div.style.grid_template_columns[1], GridTrackSize::Fr(1000));
        assert_eq!(div.style.grid_template_columns[2], GridTrackSize::Pixels(200));
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

    /// A holy-grail template becomes one rectangle per name, with the explicit
    /// grid sized from the strings.
    #[test]
    fn grid_template_areas_parsed_into_rectangles() {
        let html = r#"<div></div>"#;
        let doc = parse_document(html);
        let sheet = parse_stylesheet(
            r#"div { display: grid; grid-template-areas: "head head" "nav main" ". foot"; }"#,
        );
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let div = find_first_element(&styled, "div").unwrap();
        let areas = div.style.grid_template_areas.as_deref().expect("template");

        assert_eq!((areas.rows, areas.columns), (3, 2));
        // Half-open rectangles: head spans both columns of row 0.
        assert_eq!(areas.area("head"), Some((0, 0, 1, 2)));
        assert_eq!(areas.area("nav"), Some((1, 0, 2, 1)));
        assert_eq!(areas.area("main"), Some((1, 1, 2, 2)));
        // The null cell leaves foot in the second column only.
        assert_eq!(areas.area("foot"), Some((2, 1, 3, 2)));
        assert_eq!(areas.area("nope"), None);
    }

    /// A run of periods is *one* null cell, not one per period -- so these two
    /// rows have the same number of tokens and the template is valid.
    #[test]
    fn grid_template_areas_treats_a_period_run_as_one_null_cell() {
        let html = r#"<div></div>"#;
        let doc = parse_document(html);
        let sheet = parse_stylesheet(
            r#"div { display: grid; grid-template-areas: "a ..... b" "a . b"; }"#,
        );
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let div = find_first_element(&styled, "div").unwrap();
        let areas = div.style.grid_template_areas.as_deref().expect("template");

        assert_eq!(areas.columns, 3, "'.....' is one cell, not five");
        assert_eq!(areas.area("a"), Some((0, 0, 2, 1)));
        assert_eq!(areas.area("b"), Some((0, 2, 2, 3)));
    }

    /// Rows with differing token counts invalidate the whole declaration, and an
    /// invalid declaration is dropped rather than partly honoured.
    #[test]
    fn grid_template_areas_rejects_ragged_rows() {
        let html = r#"<div></div>"#;
        let doc = parse_document(html);
        let sheet =
            parse_stylesheet(r#"div { display: grid; grid-template-areas: "a b" "c d e"; }"#);
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let div = find_first_element(&styled, "div").unwrap();
        assert!(div.style.grid_template_areas.is_none());
    }

    /// An area whose cells do not form a filled rectangle is invalid. Here "a"
    /// is L-shaped: its bounding box holds four cells but only three are "a".
    #[test]
    fn grid_template_areas_rejects_a_non_rectangular_area() {
        let html = r#"<div></div>"#;
        let doc = parse_document(html);
        let sheet =
            parse_stylesheet(r#"div { display: grid; grid-template-areas: "a a" "a b"; }"#);
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let div = find_first_element(&styled, "div").unwrap();
        assert!(div.style.grid_template_areas.is_none());
    }

    /// A bare `grid-area: <ident>` is kept as a name for layout to resolve, and
    /// must not be mistaken for a line number.
    #[test]
    fn grid_area_records_a_bare_name() {
        let html = r#"<div style="grid-area: main-content;"></div>"#;
        let doc = parse_document(html);
        let sheet = parse_stylesheet("");
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let div = find_first_element(&styled, "div").unwrap();
        assert_eq!(div.style.grid_area_name.as_deref(), Some("main-content"));
        assert_eq!(div.style.grid_row.start, None);
    }

    /// The numeric form is row-start / column-start / row-end / column-end --
    /// row first, and both starts before both ends.
    #[test]
    fn grid_area_numeric_form_maps_row_first() {
        let html = r#"<div style="grid-area: 2 / 1 / 4 / 3;"></div>"#;
        let doc = parse_document(html);
        let sheet = parse_stylesheet("");
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let div = find_first_element(&styled, "div").unwrap();
        assert_eq!(div.style.grid_row.start, Some(2));
        assert_eq!(div.style.grid_row.span, Some(2));
        assert_eq!(div.style.grid_column.start, Some(1));
        assert_eq!(div.style.grid_column.span, Some(2));
        assert_eq!(div.style.grid_area_name, None);
    }

    /// `minmax(min, max)` used to fail to parse, which dropped the track
    /// entirely and shifted every later item one column to the left.
    #[test]
    fn minmax_track_resolves_to_its_maximum() {
        let html = r#"<div></div>"#;
        let doc = parse_document(html);
        let sheet = parse_stylesheet(
            r#"div { grid-template-columns: 200px minmax(0,1fr) minmax(0,40px); }"#,
        );
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let div = find_first_element(&styled, "div").unwrap();
        assert_eq!(
            div.style.grid_template_columns,
            vec![
                GridTrackSize::Pixels(200),
                GridTrackSize::Fr(1000),
                GridTrackSize::Pixels(40),
            ]
        );
    }

    /// `[name]` groups name the line at the current boundary, counted in lines
    /// rather than tracks.
    #[test]
    fn track_list_records_line_names() {
        let html = r#"<div></div>"#;
        let doc = parse_document(html);
        let sheet = parse_stylesheet(
            r#"div { grid-template-columns: [side-start] 100px [side-end content-start] 1fr [content-end]; }"#,
        );
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let div = find_first_element(&styled, "div").unwrap();
        let names = div.style.grid_line_names.as_deref().expect("line names");

        assert_eq!(names.column_line("side", GridEdge::Start), Some(0));
        // One bracket may name the same line twice.
        assert_eq!(names.column_line("side", GridEdge::End), Some(1));
        assert_eq!(names.column_line("content", GridEdge::Start), Some(1));
        assert_eq!(names.column_line("content", GridEdge::End), Some(2));
        assert_eq!(names.column_line("nope", GridEdge::Start), None);
    }

    /// A bare `grid-column: content` names both edges, so it spans
    /// content-start..content-end rather than collapsing to one line.
    #[test]
    fn bare_named_placement_fills_both_edges() {
        let html = r#"<div style="grid-column: content;"></div>"#;
        let doc = parse_document(html);
        let sheet = parse_stylesheet("");
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let div = find_first_element(&styled, "div").unwrap();
        let names = div.style.grid_placement_names.as_deref().expect("names");
        assert_eq!(names.column_start.as_deref(), Some("content"));
        assert_eq!(names.column_end.as_deref(), Some("content"));
    }

    /// `grid-template: <rows> / <columns>` feeds both axes at once.
    #[test]
    fn grid_template_shorthand_splits_rows_and_columns() {
        let html = r#"<div></div>"#;
        let doc = parse_document(html);
        let sheet = parse_stylesheet(
            r#"div { grid-template: min-content 1fr / 196px minmax(0,1fr); }"#,
        );
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let div = find_first_element(&styled, "div").unwrap();
        assert_eq!(
            div.style.grid_template_rows,
            vec![GridTrackSize::MinContent, GridTrackSize::Fr(1000)]
        );
        assert_eq!(
            div.style.grid_template_columns,
            vec![GridTrackSize::Pixels(196), GridTrackSize::Fr(1000)]
        );
    }

    /// The area form of the shorthand: strings build the template, and the
    /// sizes written between them size the rows.
    #[test]
    fn grid_template_shorthand_reads_areas_and_row_sizes() {
        let html = r#"<div></div>"#;
        let doc = parse_document(html);
        let sheet = parse_stylesheet(
            r#"div { grid-template: "head head" 40px "nav main" 1fr / 100px 1fr; }"#,
        );
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let div = find_first_element(&styled, "div").unwrap();

        let areas = div.style.grid_template_areas.as_deref().expect("areas");
        assert_eq!((areas.rows, areas.columns), (2, 2));
        assert_eq!(areas.area("head"), Some((0, 0, 1, 2)));
        assert_eq!(areas.area("main"), Some((1, 1, 2, 2)));
        assert_eq!(
            div.style.grid_template_rows,
            vec![GridTrackSize::Pixels(40), GridTrackSize::Fr(1000)]
        );
        assert_eq!(
            div.style.grid_template_columns,
            vec![GridTrackSize::Pixels(100), GridTrackSize::Fr(1000)]
        );
    }

    /// Custom properties inherit.
    ///
    /// MDN sets `--menu-button-padding` on `.menu` and reads it on a
    /// `.menu__tab-link` several levels below. Only the element's own
    /// declarations used to be consulted, so that padding silently vanished.
    #[test]
    fn custom_properties_inherit_to_descendants() {
        let html = r#"<div class="outer"><div><span class="inner">x</span></div></div>"#;
        let doc = parse_document(html);
        let sheet = parse_stylesheet(".outer { --pad: 12px } .inner { padding-left: var(--pad) }");
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let inner = find_first_element(&styled, "span").unwrap();
        assert_eq!(inner.style.padding.left, 12);
    }

    /// The nearest declaration wins: an ancestor shadows `:root`, and the
    /// element itself shadows the ancestor.
    #[test]
    fn a_nearer_custom_property_shadows_the_root_one() {
        let html = r#"<div class="outer"><span class="inner">x</span><b class="own">y</b></div>"#;
        let doc = parse_document(html);
        let sheet = parse_stylesheet(
            ":root { --pad: 4px } .outer { --pad: 20px } .inner { padding-left: var(--pad) }              .own { --pad: 33px; padding-left: var(--pad) }",
        );
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        assert_eq!(
            find_first_element(&styled, "span").unwrap().style.padding.left,
            20,
            "the ancestor's value should beat :root"
        );
        assert_eq!(
            find_first_element(&styled, "b").unwrap().style.padding.left,
            33,
            "the element's own value should beat the ancestor's"
        );
    }

    /// A variable an ancestor never declared still falls back to `:root`.
    #[test]
    fn root_custom_properties_still_reach_deep_elements() {
        let html = r#"<div class="outer"><span class="inner">x</span></div>"#;
        let doc = parse_document(html);
        let sheet = parse_stylesheet(
            ":root { --pad: 7px } .outer { --other: 1px } .inner { padding-left: var(--pad) }",
        );
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let inner = find_first_element(&styled, "span").unwrap();
        assert_eq!(inner.style.padding.left, 7);
    }

    /// `<template>` contents are inert: they are cloned by script, never drawn.
    ///
    /// It was missing from the not-rendered list, so every custom element on MDN
    /// painted the markup it keeps in a template alongside the real thing.
    #[test]
    fn template_contents_are_not_rendered() {
        let html = r#"<div><template><p class="ghost">hidden</p></template><p class="real">shown</p></div>"#;
        let doc = parse_document(html);
        let sheet = parse_stylesheet("");
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let template = find_first_element(&styled, "template").unwrap();
        assert_eq!(template.style.display, Display::None);
    }

    /// The modern media-query range syntax.
    ///
    /// It parsed to `Unknown`, and an unknown query matches, so a page using it
    /// applied its mobile and desktop rules at once. MDN's docs pages write every
    /// breakpoint this way, which is why the left sidebar stayed hidden.
    #[test]
    fn media_query_range_syntax_is_understood() {
        // Mobile-only rule must not apply on a wide viewport.
        let sheet = "p { color: #0000ff } @media (width <= 769px) { p { color: #ff0000 } }";
        let doc = parse_document("<p>x</p>");
        let wide = build_styled_tree(
            &doc,
            &parse_stylesheet(sheet),
            1280,
            &super::InteractiveState::default(),
        );
        assert_eq!(find_first_element(&wide, "p").unwrap().style.color, 0x0000FF);

        let narrow = build_styled_tree(
            &doc,
            &parse_stylesheet(sheet),
            400,
            &super::InteractiveState::default(),
        );
        assert_eq!(find_first_element(&narrow, "p").unwrap().style.color, 0xFF0000);
    }

    /// The value may be `calc()`, and the comparison may be written from either
    /// side. MDN uses `(width >= calc(1rem * 2 + 31rem))`.
    #[test]
    fn media_query_range_handles_calc_and_reversed_form() {
        let sheet = "p { color: #0000ff } @media (width >= calc(30rem + 10rem)) { p { color: #00ff00 } }";
        let doc = parse_document("<p>x</p>");
        // 40rem = 640px.
        let wide = build_styled_tree(&doc, &parse_stylesheet(sheet), 700, &super::InteractiveState::default());
        assert_eq!(find_first_element(&wide, "p").unwrap().style.color, 0x00FF00);
        let narrow = build_styled_tree(&doc, &parse_stylesheet(sheet), 600, &super::InteractiveState::default());
        assert_eq!(find_first_element(&narrow, "p").unwrap().style.color, 0x0000FF);

        let reversed = "p { color: #0000ff } @media (640px <= width) { p { color: #00ff00 } }";
        let wide = build_styled_tree(&doc, &parse_stylesheet(reversed), 700, &super::InteractiveState::default());
        assert_eq!(find_first_element(&wide, "p").unwrap().style.color, 0x00FF00);
        let narrow = build_styled_tree(&doc, &parse_stylesheet(reversed), 600, &super::InteractiveState::default());
        assert_eq!(find_first_element(&narrow, "p").unwrap().style.color, 0x0000FF);
    }

    /// `light-dark()` picks its first argument, because this engine renders in
    /// the light colour scheme. MDN builds nearly every colour token on it.
    #[test]
    fn light_dark_resolves_to_the_light_value() {
        let html = r#"<p>x</p>"#;
        let doc = parse_document(html);
        let sheet = parse_stylesheet(
            ":root { --white: #ffffff; --ink: #102030 }              p { color: light-dark(var(--ink), #ff0000); background-color: light-dark(var(--white), #000000) }",
        );
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let p = find_first_element(&styled, "p").unwrap();
        assert_eq!(p.style.color, 0x102030);
        assert_eq!(p.style.background_color, Some(0xFFFFFF));
    }

    /// The csstools light/dark toggle, which is how MDN ships every colour.
    ///
    /// The scheme variable is only defined under `html[data-theme=...]`. With no
    /// such attribute it stays undefined, so the toggle is guaranteed-invalid and
    /// the fallback -- the light colour -- wins. Substituting the missing
    /// reference as empty instead made the toggle resolve, and the whole page
    /// came out in the dark palette.
    #[test]
    fn an_unresolvable_var_falls_back_instead_of_resolving() {
        let doc = parse_document(r#"<div class="n">x</div>"#);
        let sheet = parse_stylesheet(
            ":root { --light: #ffffff; --dark: #18191b;                      --toggle: var(--scheme-is-dark) var(--dark);                      --page: var(--toggle, var(--light)) }              .n { background-color: var(--page) }",
        );
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let n = find_first_element(&styled, "div").unwrap();
        assert_eq!(
            n.style.background_color,
            Some(0xFFFFFF),
            "the toggle is invalid, so the light fallback applies"
        );
    }

    /// A declaration whose `var()` cannot resolve is dropped, leaving whatever
    /// the cascade had already put there.
    #[test]
    fn an_unresolvable_var_drops_only_its_own_declaration() {
        let doc = parse_document(r#"<div class="n">x</div>"#);
        let sheet = parse_stylesheet(".n { color: #00ff00 } .n { color: var(--nope) }");
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let n = find_first_element(&styled, "div").unwrap();
        assert_eq!(n.style.color, 0x00FF00);
    }

    /// A one-sided border shorthand carries a colour and a style, not just a
    /// width. Only the width was read, so every such border was drawn black.
    #[test]
    fn a_one_sided_border_shorthand_keeps_its_colour() {
        let doc = parse_document(r#"<div class="t">x</div>"#);
        let sheet = parse_stylesheet(".t { border-top: 1px solid #c3c7cb }");
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let t = find_first_element(&styled, "div").unwrap();
        assert_eq!(t.style.border.top, 1);
        assert_eq!(t.style.border_color, 0xC3C7CB);
        assert!(!t.style.border_style_none, "`solid` means it is drawn");
        assert_eq!(t.style.border.left, 0, "only the named side gets a width");
    }

    /// A transparent border colour paints nothing, but still takes its width.
    ///
    /// `border: 1px solid transparent` is how a page reserves the space a border
    /// will occupy later. Falling back to the default colour drew a solid black
    /// line instead: MDN rules its nav tabs exactly this way (`#0000`), so the
    /// bar came out boxed in black.
    #[test]
    fn a_transparent_border_is_not_painted() {
        let style_of = |css: &str| {
            let doc = parse_document(r#"<div>x</div>"#);
            let sheet = parse_stylesheet(css);
            let styled =
                build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
            find_first_element(&styled, "div").unwrap().style.clone()
        };

        for css in [
            "div { border: 1px solid transparent }",
            "div { border: 1px solid #0000 }",
            "div { border-top: 1px solid #0000 }",
        ] {
            let s = style_of(css);
            assert!(s.border_color_transparent, "{css} must not paint");
            assert_eq!(s.border.top, 1, "{css} still reserves the width");
        }

        let solid = style_of("div { border: 1px solid #c3c7cb }");
        assert!(!solid.border_color_transparent);
        assert_eq!(solid.border_color, 0xC3C7CB);
    }

    /// An unrecognised media feature does not match.
    ///
    /// postcss ships a breakpoint twice: the resolved `@media (max-width: 899px)`
    /// and the original `@media (--viewport-below-md)`. A browser ignores the
    /// second because it cannot read it. Treating unknown features as a match
    /// applied firefox.com's mobile rules at every width, one of which is
    /// `font-size: 0` on the header's download button.
    #[test]
    fn an_unknown_media_feature_does_not_match() {
        let colour_at = |css: &str, width: u32| {
            let doc = parse_document("<p>x</p>");
            let styled = build_styled_tree(
                &doc,
                &parse_stylesheet(css),
                width,
                &super::InteractiveState::default(),
            );
            find_first_element(&styled, "p").unwrap().style.color
        };

        let sheet = "p { color: #0000ff } @media (--viewport-below-md) { p { color: #ff0000 } }";
        assert_eq!(colour_at(sheet, 1280), 0x0000FF);
        assert_eq!(colour_at(sheet, 400), 0x0000FF, "and not at any other width");

        let nonsense = "p { color: #0000ff } @media (no-such-feature: 3) { p { color: #ff0000 } }";
        assert_eq!(colour_at(nonsense, 1280), 0x0000FF);
    }

    /// The features a desktop browser can actually answer are answered, rather
    /// than falling through to "unknown" and losing the rule.
    #[test]
    fn desktop_media_features_are_answered() {
        let colour_at = |css: &str| {
            let doc = parse_document("<p>x</p>");
            let styled = build_styled_tree(
                &doc,
                &parse_stylesheet(css),
                1280,
                &super::InteractiveState::default(),
            );
            find_first_element(&styled, "p").unwrap().style.color
        };

        // A mouse is present.
        assert_eq!(colour_at("p { color: #0000ff } @media (hover: hover) { p { color: #00ff00 } }"), 0x00FF00);
        assert_eq!(colour_at("p { color: #0000ff } @media (hover: none) { p { color: #00ff00 } }"), 0x0000FF);
        assert_eq!(colour_at("p { color: #0000ff } @media (pointer: fine) { p { color: #00ff00 } }"), 0x00FF00);

        // No accessibility preference is set, so the "reduce" branch is skipped.
        assert_eq!(
            colour_at("p { color: #0000ff } @media (prefers-reduced-motion: reduce) { p { color: #ff0000 } }"),
            0x0000FF
        );
        assert_eq!(
            colour_at("p { color: #0000ff } @media (forced-colors: active) { p { color: #ff0000 } }"),
            0x0000FF
        );
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

    fn color_of(css: &str) -> u32 {
        let doc = parse_document(r#"<div class="box"></div>"#);
        let sheet = parse_stylesheet(css);
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        find_first_element(&styled, "div").unwrap().style.color
    }

    /// A condition naming something this renderer has holds, so the block
    /// applies exactly as it did when every `@supports` was assumed true.
    #[test]
    fn a_supported_condition_still_applies() {
        assert_eq!(
            color_of("@supports (display: grid) { .box { color: #00ff00 } }"),
            0x00ff00
        );
    }

    /// `not` is the half that had to start meaning something. Pages wrap a whole
    /// legacy stylesheet in `@supports not (<modern feature>)`; applied anyway,
    /// those rules landed on top of the ones they were the fallback for.
    #[test]
    fn a_negated_condition_drops_its_block() {
        assert_eq!(
            color_of(".box{color:#00ff00}@supports not (all: revert-layer) { .box { color: #ff0000 } }"),
            0x00ff00
        );
    }

    /// Blocks are found by scanning for the next `{`, so the statement form of
    /// `@layer` runs straight into the prelude of whatever follows it.
    /// firefox.com writes exactly this, and read whole the prelude starts with
    /// `@layer` -- the `@supports` test never ran, and a second copy of the base
    /// stylesheet was applied over the real one.
    #[test]
    fn a_layer_statement_does_not_swallow_the_next_at_rule() {
        assert_eq!(
            color_of(
                ".box{color:#00ff00}@layer base, theme, defaults;                 @supports not (all: revert-layer) { .box { color: #ff0000 } }"
            ),
            0x00ff00
        );
    }

    fn color_with_linked_sheet(media: &str, viewport: u32) -> u32 {
        let doc = parse_document(r#"<div class="box"></div>"#);
        let mut sheet = parse_stylesheet(".box{color:#00ff00}");
        let mut linked = parse_stylesheet(".box{color:#ff0000}");
        linked.apply_media(super::parse_media_condition(media));
        sheet.extend(linked);
        let styled = build_styled_tree(&doc, &sheet, viewport, &super::InteractiveState::default());
        find_first_element(&styled, "div").unwrap().style.color
    }

    /// A sheet linked for a medium that does not apply must not be applied.
    /// firefox.com links its pre-layers base stylesheet as
    /// `media="all and (-ms-high-contrast: none)"`, a test only IE 10 and 11
    /// pass; applied anyway, its unlayered rules outranked the whole modern
    /// sheet and the page came out 700px wide with its navigation hidden.
    #[test]
    fn a_sheet_linked_for_another_medium_does_not_apply() {
        assert_eq!(
            color_with_linked_sheet("all and (-ms-high-contrast: none)", 1280),
            0x00ff00
        );
        assert_eq!(color_with_linked_sheet("print", 1280), 0x00ff00);
    }

    /// A width query on a link still follows the viewport rather than being
    /// answered once at load.
    #[test]
    fn a_sheet_linked_for_a_width_follows_the_viewport() {
        assert_eq!(color_with_linked_sheet("(max-width: 600px)", 1280), 0x00ff00);
        assert_eq!(color_with_linked_sheet("(max-width: 600px)", 400), 0xff0000);
    }

    fn color_of_html(css: &str, html: &str) -> u32 {
        let doc = parse_document(html);
        let sheet = parse_stylesheet(css);
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        find_first_element(&styled, "div").unwrap().style.color
    }

    /// An icon on a modern page is an empty box with a mask and
    /// `background-color: currentColor`, so one drawing takes the colour of the
    /// text around it. Neither half was read, so firefox.com's chevrons and
    /// globes came out as solid squares.
    #[test]
    fn a_masked_box_takes_its_shape_and_its_colour() {
        let doc = parse_document(r#"<div class="i"></div>"#);
        let sheet = parse_stylesheet(
            ".i{color:#00ff00;background-color:currentColor;             mask:url(\"/icon.svg\") no-repeat center /1em 1em}",
        );
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let div = find_first_element(&styled, "div").unwrap();
        assert_eq!(
            div.style.mask_image_url.as_deref(),
            Some("/icon.svg"),
            "the url is picked out of the shorthand"
        );
        assert_eq!(
            div.style.background_color,
            Some(0x00ff00),
            "currentColor is the element's own colour"
        );
    }

    /// `:has(> x)` asks about the element's children. firefox.com counts a card
    /// grid's children this way -- a grid with exactly four of them gets two
    /// columns until the window is wide enough for four -- and with the rule
    /// dropped the cards were laid out four across at every width.
    #[test]
    fn has_matches_on_the_children() {
        const CSS: &str = ".g{color:#0000ff}.g:has(>:nth-child(4):last-child){color:#00ff00}";
        assert_eq!(
            color_of_html(CSS, "<div class=\"g\"><i></i><i></i><i></i><i></i></div>"),
            0x00ff00,
            "exactly four children"
        );
        assert_eq!(
            color_of_html(CSS, "<div class=\"g\"><i></i><i></i><i></i></div>"),
            0x0000ff,
            "three children do not match"
        );
        assert_eq!(
            color_of_html(CSS, "<div class=\"g\"><i></i><i></i><i></i><i></i><i></i></div>"),
            0x0000ff,
            "five children do not match either"
        );
    }

    /// A sibling form asks about siblings, not children, so answering it from
    /// the children would widen the rule instead of narrowing it. Wikipedia
    /// scopes its edit-link brackets with `a:has(+ a.mw-editsection-…)`, and a
    /// wrong yes there drew a stray `]` after every link on the page.
    #[test]
    fn a_sibling_has_still_matches_nothing() {
        assert_eq!(
            color_of_html(
                ".g{color:#0000ff}.g:has(+ i){color:#00ff00}",
                "<div class=\"g\"><i></i></div>"
            ),
            0x0000ff
        );
    }

    /// An unlayered rule beats a layered one however late the layer appears.
    /// firefox.com's base sheet sets `body { width: 700px }` outside any layer;
    /// the `@layer defaults { body { inline-size: 100% } }` further down does not
    /// override it, and treating source order as the answer stretched the whole
    /// site to the window.
    #[test]
    fn an_unlayered_rule_beats_a_later_layer() {
        assert_eq!(
            color_of(".box{color:#00ff00}@layer defaults{.box{color:#ff0000}}"),
            0x00ff00
        );
    }

    /// Between layers it is the order they were declared in that decides, not
    /// where the rules sit.
    #[test]
    fn a_later_layer_beats_an_earlier_one() {
        assert_eq!(
            color_of("@layer base{.box{color:#ff0000}}@layer defaults{.box{color:#00ff00}}                      @layer base{.box{color:#ff0000}}"),
            0x00ff00,
            "base was declared first, so defaults wins even though a base block comes last"
        );
    }

    /// `@layer a, b, c;` names an order before any of those layers has a block,
    /// which is the only way a sheet can put a layer ahead of one written
    /// earlier.
    #[test]
    fn a_layer_statement_sets_the_order() {
        assert_eq!(
            color_of("@layer defaults, base;@layer base{.box{color:#00ff00}}                      @layer defaults{.box{color:#ff0000}}"),
            0x00ff00,
            "the statement puts base last, so it wins despite coming first"
        );
    }

    /// `!important` turns layer ordering upside down.
    #[test]
    fn important_reverses_the_layer_order() {
        assert_eq!(
            color_of("@layer base{.box{color:#00ff00!important}}@layer defaults{.box{color:#ff0000!important}}"),
            0x00ff00,
            "an important declaration in the earlier layer wins"
        );
        assert_eq!(
            color_of("@layer base{.box{color:#00ff00!important}}.box{color:#ff0000!important}"),
            0x00ff00,
            "and an unlayered important is the weakest of them"
        );
    }

    /// A nested layer belongs to its parent, so the parent's place in the order
    /// is what counts.
    #[test]
    fn a_nested_layer_ranks_under_its_parent() {
        assert_eq!(
            color_of("@layer base{@layer inner{.box{color:#ff0000}}}@layer defaults{.box{color:#00ff00}}"),
            0x00ff00
        );
    }

    /// `and` and `or` chains are read, not skipped over.
    #[test]
    fn compound_supports_conditions_are_evaluated() {
        assert_eq!(
            color_of("@supports (display: grid) and (display: flex) { .box { color: #00ff00 } }"),
            0x00ff00
        );
        assert_eq!(
            color_of(
                ".box{color:#00ff00}@supports (display: grid) and (container-type: inline-size) { .box { color: #ff0000 } }"
            ),
            0x00ff00
        );
        assert_eq!(
            color_of("@supports (container-type: inline-size) or (display: grid) { .box { color: #00ff00 } }"),
            0x00ff00
        );
    }

    /// Logical properties are aliases of physical ones and take part in the
    /// cascade as such: firefox.com sets `body { width: 700px }` and overrides it
    /// further down with `body { inline-size: 100% }`.
    #[test]
    fn a_logical_property_overrides_the_physical_one_it_aliases() {
        let width_of = |css: &str| {
            let doc = parse_document(r#"<div class="box"></div>"#);
            let sheet = parse_stylesheet(css);
            let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
            find_first_element(&styled, "div").unwrap().style.width
        };
        assert_eq!(
            width_of(".box{width:700px}.box{inline-size:100%}"),
            width_of(".box{width:100%}"),
            "the later logical declaration has to win"
        );
        assert_ne!(
            width_of(".box{width:700px}.box{inline-size:100%}"),
            width_of(".box{width:700px}"),
            "control: the two widths are distinguishable"
        );
    }

    #[test]
    fn placeholder_pseudo_element_parsed() {
        // ::placeholder rules should be parsed without errors
        let css = r#"input::placeholder { color: #999999; }"#;
        let sheet = parse_stylesheet(css);
        // Should have one rule with Placeholder pseudo-element
        assert!(!sheet.rules.is_empty());
        let rule = &sheet.rules[0];
        assert!(rule.selectors.iter().any(|s| s.pseudo_element == Some(super::PseudoElement::Placeholder)));
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
    fn transform_translate_parsed() {
        let html = r#"<div style="transform: translateX(30px) translateY(-10px);"></div>"#;
        let doc = parse_document(html);
        let sheet = parse_stylesheet("");
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let div = find_first_element(&styled, "div").unwrap();
        assert_eq!(div.style.transform_translate_x, 30);
        assert_eq!(div.style.transform_translate_y, -10);
    }

    #[test]
    fn transform_scale_parsed() {
        let html = r#"<div style="transform: scale(1.5);"></div>"#;
        let doc = parse_document(html);
        let sheet = parse_stylesheet("");
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let div = find_first_element(&styled, "div").unwrap();
        assert_eq!(div.style.transform_scale_x, 1500);
        assert_eq!(div.style.transform_scale_y, 1500);
    }

    #[test]
    fn transform_rotate_parsed() {
        let html = r#"<div style="transform: rotate(45deg);"></div>"#;
        let doc = parse_document(html);
        let sheet = parse_stylesheet("");
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let div = find_first_element(&styled, "div").unwrap();
        assert_eq!(div.style.transform_rotate_millideg, 45000);
    }

    #[test]
    fn margin_auto_sets_auto_flags() {
        // 5em at 16px = 80px; "auto" for horizontal → both auto flags set
        let html = r#"<div style="margin: 5em auto;"></div>"#;
        let doc = parse_document(html);
        let sheet = parse_stylesheet("");
        let styled = build_styled_tree(&doc, &sheet, 1280, &super::InteractiveState::default());
        let div = find_first_element(&styled, "div").unwrap();
        assert_eq!(div.style.margin.top, 80, "5em at 16px base = 80px");
        assert_eq!(div.style.margin.bottom, 80, "5em at 16px base = 80px");
        assert_eq!(div.style.margin.left, 0, "auto resolves to 0 in parsed value");
        assert_eq!(div.style.margin.right, 0, "auto resolves to 0 in parsed value");
        assert!(div.style.margin_left_auto, "margin-left should be auto");
        assert!(div.style.margin_right_auto, "margin-right should be auto");
    }
}

#[cfg(test)]
mod style_sharing {
    use super::*;
    use std::collections::HashSet;

    fn walk(node: &StyledNode, nodes: &mut usize, uniq: &mut HashSet<ComputedStyle>) {
        *nodes += 1;
        match node {
            StyledNode::Element(e) => {
                uniq.insert((*e.style).clone());
                for c in &e.children {
                    walk(c, nodes, uniq);
                }
            }
            StyledNode::Text(t) => {
                uniq.insert((*t.style).clone());
            }
        }
    }

    fn build(html: &str) -> StyledNode {
        let doc = crate::html::parse_document(html);
        let sheet = parse_stylesheet("");
        build_styled_tree(&doc, &sheet, 1280, &InteractiveState::default())
    }

    /// A `ComputedStyle` is 520 bytes and pages reuse a handful of them across
    /// many nodes, so the styled tree holds shared handles rather than a copy
    /// per node. Guard the node structs against re-inlining anything large.
    #[test]
    fn styled_nodes_hold_a_handle_not_a_copy() {
        use std::mem::size_of;
        assert!(
            size_of::<StyledNode>() <= 96,
            "StyledNode grew to {} bytes; it should hold Arc<ComputedStyle>, not a copy",
            size_of::<StyledNode>()
        );
        assert!(size_of::<Arc<ComputedStyle>>() == 8);
    }

    /// Repeated markup must collapse to one style allocation, and the interner
    /// must end up holding exactly the styles the tree actually references --
    /// no leftovers from previous builds, nothing missing.
    #[test]
    fn repeated_markup_shares_one_style_allocation() {
        let rows: String = (0..200)
            .map(|i| format!("<li class=\"row\">item {i}</li>"))
            .collect();
        let styled = build(&format!("<html><body><ul>{rows}</ul></body></html>"));

        let mut nodes = 0usize;
        let mut uniq = HashSet::new();
        walk(&styled, &mut nodes, &mut uniq);

        assert!(nodes >= 400, "expected the 200 <li> plus their text, got {nodes}");
        assert!(
            nodes / uniq.len().max(1) >= 10,
            "200 identical rows should share styles heavily: {nodes} nodes but {} distinct styles",
            uniq.len()
        );
        assert_eq!(
            uniq.len(),
            super::interned_style_count(),
            "the interner should hold exactly the styles this tree references"
        );
    }

    /// Two nodes with the same computed style must be the *same* allocation,
    /// which is what turns the sharing into a memory win rather than just a
    /// pointer indirection.
    #[test]
    fn identical_styles_are_one_allocation() {
        let styled = build("<html><body><p>one</p><p>two</p></body></html>");
        let mut paragraphs = Vec::new();
        fn collect<'a>(node: &'a StyledNode, out: &mut Vec<&'a StyledElement>) {
            if let StyledNode::Element(e) = node {
                if e.tag_name == "p" {
                    out.push(e);
                }
                for c in &e.children {
                    collect(c, out);
                }
            }
        }
        collect(&styled, &mut paragraphs);

        assert_eq!(paragraphs.len(), 2);
        assert!(
            Arc::ptr_eq(&paragraphs[0].style, &paragraphs[1].style),
            "two <p> with identical computed styles should share one allocation"
        );
    }
}
