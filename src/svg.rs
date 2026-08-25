//! A rasterizer for the subset of SVG that icons are drawn with.
//!
//! Icons on the modern web are overwhelmingly inline SVG: of the 59 images
//! Yahoo! JAPAN's front page draws, 56 are SVG data URLs. With no decoder for
//! them the page rendered its navigation, weather and service shortcuts as
//! blank boxes. Full SVG is an enormous specification, but the part icons
//! actually use is small: a `viewBox`, a handful of shapes, flat fills, and the
//! occasional stroke or group transform. That is what this covers.
//!
//! What it deliberately does not do: gradients, patterns, filters, clipping and
//! masking, text, `<use>` references, and CSS inside `<style>`. Anything it
//! cannot understand is skipped rather than guessed at, so a partly-supported
//! icon still draws the parts that are plain shapes.

use crate::image::DecodedImage;

/// Longest edge of the raster we produce.
///
/// The `viewBox` is a coordinate system, not a pixel size — icons routinely
/// declare `0 0 24 24` and are displayed at 38px. Rasterizing at the declared
/// size would leave them visibly blocky, so small drawings are scaled up to a
/// sensible working resolution and the layout scales from there.
const TARGET_EDGE: f32 = 96.0;
/// Ceiling on the raster, so a drawing that declares a huge `viewBox` cannot
/// turn one icon into a multi-megabyte buffer.
const MAX_EDGE: f32 = 512.0;
/// Sub-scanlines per pixel row. Four is enough to keep icon edges smooth
/// without making the fill noticeably slower.
const SUBSAMPLES: usize = 4;
/// Guard against a pathological path: icons are tens of segments, not millions.
const MAX_SEGMENTS: usize = 200_000;

// ─────────────────────────────────────────────────────────────────────────────
// Public entry point
// ─────────────────────────────────────────────────────────────────────────────

/// Rasterize an SVG document, or `None` if it has no usable drawing in it.
pub fn rasterize(source: &str) -> Option<DecodedImage> {
    let document = Document::parse(source)?;
    let (width, height) = document.raster_size();
    if width == 0 || height == 0 {
        return None;
    }

    let mut canvas = Canvas::new(width, height);
    let root = document.root_transform(width, height);
    let mut painted = false;
    for shape in document.shapes(root) {
        painted |= canvas.fill(&shape);
    }
    if !painted {
        return None;
    }

    Some(DecodedImage {
        width,
        height,
        rgba: canvas.rgba,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Geometry
// ─────────────────────────────────────────────────────────────────────────────

/// A 2D affine transform, laid out as SVG writes `matrix(a b c d e f)`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Transform {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    e: f32,
    f: f32,
}

impl Transform {
    const IDENTITY: Self = Self { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };

    fn translate(tx: f32, ty: f32) -> Self {
        Self { e: tx, f: ty, ..Self::IDENTITY }
    }

    fn scale(sx: f32, sy: f32) -> Self {
        Self { a: sx, d: sy, ..Self::IDENTITY }
    }

    fn rotate(degrees: f32) -> Self {
        let (sin, cos) = degrees.to_radians().sin_cos();
        Self { a: cos, b: sin, c: -sin, d: cos, e: 0.0, f: 0.0 }
    }

    /// `self` applied after `inner` — the order SVG nests transforms in.
    fn then(self, inner: Transform) -> Transform {
        Transform {
            a: self.a * inner.a + self.c * inner.b,
            b: self.b * inner.a + self.d * inner.b,
            c: self.a * inner.c + self.c * inner.d,
            d: self.b * inner.c + self.d * inner.d,
            e: self.a * inner.e + self.c * inner.f + self.e,
            f: self.b * inner.e + self.d * inner.f + self.f,
        }
    }

    fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (self.a * x + self.c * y + self.e, self.b * x + self.d * y + self.f)
    }

    /// Roughly how much this transform magnifies lengths — used to pick how
    /// finely curves are flattened and how wide a stroke ends up.
    fn scale_factor(&self) -> f32 {
        let x = (self.a * self.a + self.b * self.b).sqrt();
        let y = (self.c * self.c + self.d * self.d).sqrt();
        ((x + y) / 2.0).max(0.001)
    }
}

/// One closed or open run of points, already in device space.
type Contour = Vec<(f32, f32)>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FillRule {
    NonZero,
    EvenOdd,
}

/// A filled area ready to rasterize.
#[derive(Debug, Clone)]
struct Shape {
    contours: Vec<Contour>,
    color: [u8; 4],
    rule: FillRule,
}

// ─────────────────────────────────────────────────────────────────────────────
// Document
// ─────────────────────────────────────────────────────────────────────────────

struct Document {
    /// `(min_x, min_y, width, height)` of the `viewBox`, or the declared size.
    view_box: (f32, f32, f32, f32),
    elements: Vec<Element>,
}

struct Element {
    name: String,
    attributes: Vec<(String, String)>,
    /// Depth in the tree, so inherited state can be tracked with a stack.
    depth: usize,
}

impl Document {
    fn parse(source: &str) -> Option<Document> {
        let elements = parse_elements(source);
        let root = elements.iter().find(|element| element.name == "svg")?;

        let view_box = root
            .attribute("viewbox")
            .and_then(|value| {
                let numbers = parse_numbers(value);
                match numbers[..] {
                    [x, y, w, h] if w > 0.0 && h > 0.0 => Some((x, y, w, h)),
                    _ => None,
                }
            })
            .or_else(|| {
                let width = root.attribute("width").and_then(parse_dimension)?;
                let height = root.attribute("height").and_then(parse_dimension)?;
                (width > 0.0 && height > 0.0).then_some((0.0, 0.0, width, height))
            })?;

        Some(Document { view_box, elements })
    }

    fn raster_size(&self) -> (u32, u32) {
        let (_, _, w, h) = self.view_box;
        let longest = w.max(h);
        let scale = (TARGET_EDGE / longest).clamp(1.0, MAX_EDGE / longest);
        (
            (w * scale).round().max(1.0) as u32,
            (h * scale).round().max(1.0) as u32,
        )
    }

    /// Maps the `viewBox` onto the raster.
    fn root_transform(&self, width: u32, height: u32) -> Transform {
        let (min_x, min_y, w, h) = self.view_box;
        Transform::scale(width as f32 / w, height as f32 / h)
            .then(Transform::translate(-min_x, -min_y))
    }

    /// Walk the tree, resolving inherited paint and transforms into flat shapes.
    fn shapes(&self, root: Transform) -> Vec<Shape> {
        // One entry per open ancestor: its depth and the state it contributes.
        let mut stack: Vec<(usize, State)> = vec![(0, State::root(root))];
        let mut shapes = Vec::new();
        let mut segments = 0usize;

        for element in &self.elements {
            while stack.len() > 1 && stack.last().is_some_and(|(d, _)| *d >= element.depth) {
                stack.pop();
            }
            let inherited = stack.last().map(|(_, s)| s.clone()).unwrap_or(State::root(root));
            let state = inherited.inherit(element);

            if element.name == "g" || element.name == "svg" || element.name == "a" {
                stack.push((element.depth, state));
                continue;
            }

            let Some(contours) = element_contours(element, &state) else {
                continue;
            };
            segments += contours.iter().map(Vec::len).sum::<usize>();
            if segments > MAX_SEGMENTS {
                break;
            }

            if let Some(color) = state.fill {
                shapes.push(Shape { contours: contours.clone(), color, rule: state.fill_rule });
            }
            if let Some(color) = state.stroke {
                let width = (state.stroke_width * state.transform.scale_factor()).max(0.6);
                let outlined = contours
                    .iter()
                    .flat_map(|contour| stroke_outline(contour, width))
                    .collect::<Vec<_>>();
                if !outlined.is_empty() {
                    // Each segment's quad is filled on its own, so overlapping
                    // ones must not cancel out: non-zero winding keeps them.
                    shapes.push(Shape { contours: outlined, color, rule: FillRule::NonZero });
                }
            }
        }

        shapes
    }
}

impl Element {
    fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Inherited state
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct State {
    transform: Transform,
    fill: Option<[u8; 4]>,
    stroke: Option<[u8; 4]>,
    stroke_width: f32,
    fill_rule: FillRule,
    opacity: f32,
}

impl State {
    fn root(transform: Transform) -> State {
        State {
            transform,
            // The initial fill really is opaque black, which is what an icon
            // that names no colour at all expects.
            fill: Some([0, 0, 0, 255]),
            stroke: None,
            stroke_width: 1.0,
            fill_rule: FillRule::NonZero,
            opacity: 1.0,
        }
    }

    fn inherit(&self, element: &Element) -> State {
        let mut state = self.clone();

        // A `style` attribute wins over presentation attributes, so read the
        // attributes first and let the declarations overwrite them.
        let mut apply = |property: &str, value: &str, state: &mut State| match property {
            "fill" => state.fill = parse_paint(value, state.fill),
            "stroke" => state.stroke = parse_paint(value, state.stroke),
            "stroke-width" => {
                if let Some(width) = parse_dimension(value) {
                    state.stroke_width = width;
                }
            }
            "fill-rule" | "clip-rule" => {
                state.fill_rule = if value.trim() == "evenodd" {
                    FillRule::EvenOdd
                } else {
                    FillRule::NonZero
                };
            }
            "opacity" | "fill-opacity" => {
                if let Ok(value) = value.trim().parse::<f32>() {
                    state.opacity *= value.clamp(0.0, 1.0);
                }
            }
            _ => {}
        };

        for (name, value) in &element.attributes {
            apply(name, value, &mut state);
        }
        if let Some(style) = element.attribute("style") {
            for declaration in style.split(';') {
                if let Some((property, value)) = declaration.split_once(':') {
                    apply(property.trim(), value.trim(), &mut state);
                }
            }
        }
        if let Some(transform) = element.attribute("transform") {
            state.transform = state.transform.then(parse_transform(transform));
        }

        // `opacity` multiplies down the tree; fold it into the paint so the
        // rasterizer only ever deals with a single alpha.
        for paint in [&mut state.fill, &mut state.stroke] {
            if let Some(color) = paint {
                color[3] = (color[3] as f32 * state.opacity).round().clamp(0.0, 255.0) as u8;
            }
        }
        state
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Shapes
// ─────────────────────────────────────────────────────────────────────────────

fn element_contours(element: &Element, state: &State) -> Option<Vec<Contour>> {
    let transform = state.transform;
    let number = |name: &str| element.attribute(name).and_then(parse_dimension).unwrap_or(0.0);

    match element.name.as_str() {
        "path" => {
            let data = element.attribute("d")?;
            let contours = parse_path(data, transform);
            (!contours.is_empty()).then_some(contours)
        }
        "rect" => {
            let (x, y) = (number("x"), number("y"));
            let (w, h) = (number("width"), number("height"));
            if w <= 0.0 || h <= 0.0 {
                return None;
            }
            // Rounded corners are approximated by their bounding rectangle:
            // at icon sizes the difference is a pixel or two.
            Some(vec![vec![
                transform.apply(x, y),
                transform.apply(x + w, y),
                transform.apply(x + w, y + h),
                transform.apply(x, y + h),
            ]])
        }
        "circle" => {
            let r = number("r");
            (r > 0.0).then(|| vec![ellipse(number("cx"), number("cy"), r, r, transform)])
        }
        "ellipse" => {
            let (rx, ry) = (number("rx"), number("ry"));
            (rx > 0.0 && ry > 0.0)
                .then(|| vec![ellipse(number("cx"), number("cy"), rx, ry, transform)])
        }
        "line" => Some(vec![vec![
            transform.apply(number("x1"), number("y1")),
            transform.apply(number("x2"), number("y2")),
        ]]),
        "polygon" | "polyline" => {
            let points = parse_numbers(element.attribute("points")?);
            let contour: Contour = points
                .chunks_exact(2)
                .map(|pair| transform.apply(pair[0], pair[1]))
                .collect();
            (contour.len() >= 2).then_some(vec![contour])
        }
        _ => None,
    }
}

fn ellipse(cx: f32, cy: f32, rx: f32, ry: f32, transform: Transform) -> Contour {
    let radius = rx.max(ry) * transform.scale_factor();
    let steps = ((radius * 2.0) as usize).clamp(12, 180);
    (0..steps)
        .map(|step| {
            let angle = step as f32 / steps as f32 * std::f32::consts::TAU;
            transform.apply(cx + rx * angle.cos(), cy + ry * angle.sin())
        })
        .collect()
}

/// Turn an open polyline into filled quads, one per segment, plus a square at
/// each joint so corners do not show a notch.
fn stroke_outline(contour: &Contour, width: f32) -> Vec<Contour> {
    let half = width / 2.0;
    let mut quads = Vec::new();
    for pair in contour.windows(2) {
        let (x1, y1) = pair[0];
        let (x2, y2) = pair[1];
        let (dx, dy) = (x2 - x1, y2 - y1);
        let length = (dx * dx + dy * dy).sqrt();
        if length < 0.0001 {
            continue;
        }
        let (nx, ny) = (-dy / length * half, dx / length * half);
        quads.push(vec![
            (x1 + nx, y1 + ny),
            (x2 + nx, y2 + ny),
            (x2 - nx, y2 - ny),
            (x1 - nx, y1 - ny),
        ]);
    }
    if quads.len() > 1 {
        for &(x, y) in &contour[1..contour.len() - 1] {
            quads.push(vec![
                (x - half, y - half),
                (x + half, y - half),
                (x + half, y + half),
                (x - half, y + half),
            ]);
        }
    }
    quads
}

// ─────────────────────────────────────────────────────────────────────────────
// Rasterizer
// ─────────────────────────────────────────────────────────────────────────────

struct Canvas {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl Canvas {
    fn new(width: u32, height: u32) -> Canvas {
        Canvas {
            width,
            height,
            rgba: vec![0; (width as usize) * (height as usize) * 4],
        }
    }

    /// Scanline fill with vertical supersampling, returning whether it painted.
    fn fill(&mut self, shape: &Shape) -> bool {
        if shape.color[3] == 0 {
            return false;
        }
        let edges: Vec<((f32, f32), (f32, f32))> = shape
            .contours
            .iter()
            .filter(|contour| contour.len() >= 3)
            .flat_map(|contour| {
                let closed = contour.iter().copied().chain(std::iter::once(contour[0]));
                contour.iter().copied().zip(closed.skip(1)).collect::<Vec<_>>()
            })
            .filter(|((_, y1), (_, y2))| y1 != y2)
            .collect();
        if edges.is_empty() {
            return false;
        }

        let (min_y, max_y) = edges.iter().fold((f32::MAX, f32::MIN), |(lo, hi), (a, b)| {
            (lo.min(a.1).min(b.1), hi.max(a.1).max(b.1))
        });
        let first_row = min_y.floor().max(0.0) as u32;
        let last_row = (max_y.ceil().min(self.height as f32) as u32).min(self.height);

        let mut coverage = vec![0.0_f32; self.width as usize];
        let mut crossings: Vec<(f32, i32)> = Vec::new();
        let mut painted = false;

        for row in first_row..last_row {
            coverage.iter_mut().for_each(|value| *value = 0.0);
            for sub in 0..SUBSAMPLES {
                let y = row as f32 + (sub as f32 + 0.5) / SUBSAMPLES as f32;
                crossings.clear();
                for ((x1, y1), (x2, y2)) in &edges {
                    let (top, bottom) = if y1 < y2 { (y1, y2) } else { (y2, y1) };
                    if y < *top || y >= *bottom {
                        continue;
                    }
                    let t = (y - y1) / (y2 - y1);
                    crossings.push((x1 + (x2 - x1) * t, if y2 > y1 { 1 } else { -1 }));
                }
                if crossings.len() < 2 {
                    continue;
                }
                crossings.sort_by(|a, b| a.0.total_cmp(&b.0));

                let mut winding = 0;
                for pair in 0..crossings.len() - 1 {
                    winding += crossings[pair].1;
                    let inside = match shape.rule {
                        FillRule::NonZero => winding != 0,
                        FillRule::EvenOdd => (pair as i32 + 1) % 2 == 1,
                    };
                    if inside {
                        self.accumulate(
                            &mut coverage,
                            crossings[pair].0,
                            crossings[pair + 1].0,
                        );
                    }
                }
            }

            for (column, value) in coverage.iter().enumerate() {
                let alpha = (value / SUBSAMPLES as f32).clamp(0.0, 1.0);
                if alpha > 0.002 {
                    self.blend(column as u32, row, shape.color, alpha);
                    painted = true;
                }
            }
        }
        painted
    }

    /// Add one span's horizontal coverage, with fractional ends so edges are
    /// antialiased rather than stair-stepped.
    fn accumulate(&self, coverage: &mut [f32], from: f32, to: f32) {
        let left = from.max(0.0);
        let right = to.min(self.width as f32);
        if right <= left {
            return;
        }
        let first = left.floor() as usize;
        let last = (right.ceil() as usize).min(coverage.len());
        for (column, slot) in coverage.iter_mut().enumerate().take(last).skip(first) {
            let start = (column as f32).max(left);
            let end = ((column + 1) as f32).min(right);
            if end > start {
                *slot += end - start;
            }
        }
    }

    fn blend(&mut self, x: u32, y: u32, color: [u8; 4], coverage: f32) {
        if x >= self.width || y >= self.height {
            return;
        }
        let alpha = color[3] as f32 / 255.0 * coverage;
        if alpha <= 0.0 {
            return;
        }
        let index = ((y as usize) * (self.width as usize) + x as usize) * 4;
        let existing = self.rgba[index + 3] as f32 / 255.0;
        let out_alpha = alpha + existing * (1.0 - alpha);
        if out_alpha <= 0.0 {
            return;
        }
        for channel in 0..3 {
            let src = color[channel] as f32;
            let dst = self.rgba[index + channel] as f32;
            let value = (src * alpha + dst * existing * (1.0 - alpha)) / out_alpha;
            self.rgba[index + channel] = value.round().clamp(0.0, 255.0) as u8;
        }
        self.rgba[index + 3] = (out_alpha * 255.0).round().clamp(0.0, 255.0) as u8;
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Parsing
// ─────────────────────────────────────────────────────────────────────────────

/// Flatten the document into elements tagged with their depth.
///
/// A real XML parser is not needed here: the shapes are leaves and the only
/// structure that matters is nesting, so tracking depth as tags open and close
/// is enough.
fn parse_elements(source: &str) -> Vec<Element> {
    let bytes = source.as_bytes();
    let mut elements = Vec::new();
    let mut depth = 0usize;
    let mut index = 0usize;

    while index < bytes.len() {
        let Some(open) = source[index..].find('<').map(|offset| index + offset) else {
            break;
        };
        let rest = &source[open..];

        if rest.starts_with("<!--") {
            index = rest.find("-->").map_or(bytes.len(), |end| open + end + 3);
            continue;
        }
        if rest.starts_with("<?") || rest.starts_with("<!") {
            index = rest.find('>').map_or(bytes.len(), |end| open + end + 1);
            continue;
        }
        let Some(close) = find_tag_end(rest) else {
            break;
        };
        let inner = &rest[1..close];
        index = open + close + 1;

        if let Some(name) = inner.strip_prefix('/') {
            let _ = name;
            depth = depth.saturating_sub(1);
            continue;
        }

        let self_closing = inner.ends_with('/');
        let inner = inner.trim_end_matches('/');
        let name_end = inner
            .find(|c: char| c.is_whitespace())
            .unwrap_or(inner.len());
        let name = inner[..name_end].trim().to_ascii_lowercase();
        if name.is_empty() {
            continue;
        }
        // Namespace prefixes appear on documents copied out of editors.
        let name = name.rsplit(':').next().unwrap_or(&name).to_string();

        elements.push(Element {
            name,
            attributes: parse_attributes(&inner[name_end..]),
            depth,
        });
        if !self_closing {
            depth += 1;
        }
    }

    elements
}

/// Find the `>` that ends a tag, ignoring any inside quoted attribute values.
fn find_tag_end(tag: &str) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (offset, character) in tag.char_indices() {
        match (quote, character) {
            (Some(open), c) if c == open => quote = None,
            (Some(_), _) => {}
            (None, '"') | (None, '\'') => quote = Some(character),
            (None, '>') => return Some(offset),
            _ => {}
        }
    }
    None
}

fn parse_attributes(input: &str) -> Vec<(String, String)> {
    let mut attributes = Vec::new();
    let bytes = input.as_bytes();
    let mut index = 0;

    while index < bytes.len() {
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        let start = index;
        while index < bytes.len() && bytes[index] != b'=' && !bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index == start {
            break;
        }
        let name = input[start..index].to_ascii_lowercase();
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() || bytes[index] != b'=' {
            attributes.push((name, String::new()));
            continue;
        }
        index += 1;
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        let value = if index < bytes.len() && (bytes[index] == b'"' || bytes[index] == b'\'') {
            let quote = bytes[index];
            index += 1;
            let start = index;
            while index < bytes.len() && bytes[index] != quote {
                index += 1;
            }
            let value = &input[start..index.min(input.len())];
            index += 1;
            value
        } else {
            let start = index;
            while index < bytes.len() && !bytes[index].is_ascii_whitespace() {
                index += 1;
            }
            &input[start..index]
        };
        attributes.push((name, unescape_entities(value)));
    }

    attributes
}

fn unescape_entities(value: &str) -> String {
    if !value.contains('&') {
        return value.to_string();
    }
    value
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// Every number in a string, however they are separated. SVG allows commas,
/// whitespace, and nothing at all before a sign or a decimal point.
fn parse_numbers(input: &str) -> Vec<f32> {
    let mut numbers = Vec::new();
    let bytes = input.as_bytes();
    let mut index = 0;

    while index < bytes.len() {
        let byte = bytes[index];
        if !(byte.is_ascii_digit() || byte == b'-' || byte == b'+' || byte == b'.') {
            index += 1;
            continue;
        }
        let start = index;
        if bytes[index] == b'-' || bytes[index] == b'+' {
            index += 1;
        }
        let mut seen_dot = false;
        while index < bytes.len() {
            match bytes[index] {
                b'0'..=b'9' => index += 1,
                b'.' if !seen_dot => {
                    seen_dot = true;
                    index += 1;
                }
                b'e' | b'E'
                    if index + 1 < bytes.len()
                        && (bytes[index + 1].is_ascii_digit()
                            || bytes[index + 1] == b'-'
                            || bytes[index + 1] == b'+') =>
                {
                    index += 2;
                    seen_dot = true;
                }
                _ => break,
            }
        }
        if let Ok(value) = input[start..index].parse::<f32>() {
            numbers.push(value);
        } else if index == start {
            index += 1;
        }
    }

    numbers
}

/// A length with an optional unit. Only absolute units make sense here.
fn parse_dimension(value: &str) -> Option<f32> {
    let trimmed = value.trim();
    let digits: String = trimmed
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-' || *c == '+')
        .collect();
    let number = digits.parse::<f32>().ok()?;
    let unit = trimmed[digits.len()..].trim().to_ascii_lowercase();
    let scale = match unit.as_str() {
        "" | "px" => 1.0,
        "pt" => 96.0 / 72.0,
        "pc" => 16.0,
        "in" => 96.0,
        "cm" => 96.0 / 2.54,
        "mm" => 96.0 / 25.4,
        // A percentage has no basis here, and `em` has no font: treat the
        // drawing as unsized rather than inventing a number.
        _ => return None,
    };
    Some(number * scale)
}

fn parse_paint(value: &str, inherited: Option<[u8; 4]>) -> Option<[u8; 4]> {
    let value = value.trim();
    match value.to_ascii_lowercase().as_str() {
        "none" | "transparent" => None,
        // Without a host element to ask, the sensible reading of
        // `currentColor` is whatever colour was already in force.
        "currentcolor" | "inherit" => inherited,
        _ => parse_color(value).or(inherited),
    }
}

fn parse_color(value: &str) -> Option<[u8; 4]> {
    let value = value.trim();
    if let Some(hex) = value.strip_prefix('#') {
        let digits: Vec<u8> = hex
            .chars()
            .filter_map(|c| c.to_digit(16).map(|d| d as u8))
            .collect();
        return match digits.len() {
            3 => Some([digits[0] * 17, digits[1] * 17, digits[2] * 17, 255]),
            4 => Some([digits[0] * 17, digits[1] * 17, digits[2] * 17, digits[3] * 17]),
            6 => Some([
                digits[0] * 16 + digits[1],
                digits[2] * 16 + digits[3],
                digits[4] * 16 + digits[5],
                255,
            ]),
            8 => Some([
                digits[0] * 16 + digits[1],
                digits[2] * 16 + digits[3],
                digits[4] * 16 + digits[5],
                digits[6] * 16 + digits[7],
            ]),
            _ => None,
        };
    }
    let lower = value.to_ascii_lowercase();
    if lower.starts_with("rgb") {
        let parts = parse_numbers(&lower);
        return match parts[..] {
            [r, g, b] => Some([r as u8, g as u8, b as u8, 255]),
            [r, g, b, a] => Some([r as u8, g as u8, b as u8, (a * 255.0) as u8]),
            _ => None,
        };
    }
    named_color(&lower)
}

fn named_color(name: &str) -> Option<[u8; 4]> {
    let rgb = match name {
        "black" => [0, 0, 0],
        "white" => [255, 255, 255],
        "red" => [255, 0, 0],
        "green" => [0, 128, 0],
        "lime" => [0, 255, 0],
        "blue" => [0, 0, 255],
        "yellow" => [255, 255, 0],
        "cyan" | "aqua" => [0, 255, 255],
        "magenta" | "fuchsia" => [255, 0, 255],
        "gray" | "grey" => [128, 128, 128],
        "silver" => [192, 192, 192],
        "maroon" => [128, 0, 0],
        "olive" => [128, 128, 0],
        "navy" => [0, 0, 128],
        "purple" => [128, 0, 128],
        "teal" => [0, 128, 128],
        "orange" => [255, 165, 0],
        _ => return None,
    };
    Some([rgb[0], rgb[1], rgb[2], 255])
}

fn parse_transform(input: &str) -> Transform {
    let mut result = Transform::IDENTITY;
    let mut rest = input;

    while let Some(open) = rest.find('(') {
        let name = rest[..open].trim().rsplit(|c: char| !c.is_alphabetic()).next().unwrap_or("");
        let Some(close) = rest[open..].find(')') else {
            break;
        };
        let arguments = parse_numbers(&rest[open + 1..open + close]);
        rest = &rest[open + close + 1..];

        let step = match (name, arguments.as_slice()) {
            ("translate", [tx]) => Transform::translate(*tx, 0.0),
            ("translate", [tx, ty, ..]) => Transform::translate(*tx, *ty),
            ("scale", [s]) => Transform::scale(*s, *s),
            ("scale", [sx, sy, ..]) => Transform::scale(*sx, *sy),
            ("rotate", [angle]) => Transform::rotate(*angle),
            ("rotate", [angle, cx, cy, ..]) => Transform::translate(*cx, *cy)
                .then(Transform::rotate(*angle))
                .then(Transform::translate(-cx, -cy)),
            ("matrix", [a, b, c, d, e, f, ..]) => {
                Transform { a: *a, b: *b, c: *c, d: *d, e: *e, f: *f }
            }
            ("skewx", [angle]) => Transform { c: angle.to_radians().tan(), ..Transform::IDENTITY },
            ("skewy", [angle]) => Transform { b: angle.to_radians().tan(), ..Transform::IDENTITY },
            _ => continue,
        };
        result = result.then(step);
    }

    result
}

// ─────────────────────────────────────────────────────────────────────────────
// Path data
// ─────────────────────────────────────────────────────────────────────────────

/// Turn a `d` attribute into device-space contours.
fn parse_path(data: &str, transform: Transform) -> Vec<Contour> {
    let mut builder = PathBuilder::new(transform);
    let mut command = ' ';
    let mut numbers: Vec<f32> = Vec::new();
    let mut index = 0;
    let bytes = data.as_bytes();

    while index < bytes.len() {
        let byte = bytes[index];
        if byte.is_ascii_alphabetic() {
            builder.run(command, &numbers);
            numbers.clear();
            command = byte as char;
            index += 1;
            continue;
        }
        if byte.is_ascii_digit() || byte == b'-' || byte == b'+' || byte == b'.' {
            let start = index;
            index += 1;
            while index < bytes.len() {
                let next = bytes[index];
                if next.is_ascii_digit() {
                    index += 1;
                } else if next == b'.' && !data[start..index].contains('.') {
                    index += 1;
                } else if (next == b'e' || next == b'E')
                    && index + 1 < bytes.len()
                    && (bytes[index + 1].is_ascii_digit()
                        || bytes[index + 1] == b'-'
                        || bytes[index + 1] == b'+')
                {
                    index += 2;
                } else {
                    break;
                }
            }
            if let Ok(value) = data[start..index].parse::<f32>() {
                numbers.push(value);
            }
            continue;
        }
        index += 1;
    }
    builder.run(command, &numbers);
    builder.finish()
}

struct PathBuilder {
    transform: Transform,
    contours: Vec<Contour>,
    current: Contour,
    /// Position in user space, before the transform.
    cursor: (f32, f32),
    start: (f32, f32),
    /// Last curve control point, for the smooth `S` / `T` forms.
    last_control: Option<(f32, f32)>,
    steps: usize,
}

impl PathBuilder {
    fn new(transform: Transform) -> PathBuilder {
        // How finely to flatten curves: enough steps that a segment is under a
        // pixel at the size this drawing will be rasterized to.
        let steps = (8.0 * transform.scale_factor()).clamp(8.0, 48.0) as usize;
        PathBuilder {
            transform,
            contours: Vec::new(),
            current: Vec::new(),
            cursor: (0.0, 0.0),
            start: (0.0, 0.0),
            last_control: None,
            steps,
        }
    }

    fn push(&mut self, point: (f32, f32)) {
        self.current.push(self.transform.apply(point.0, point.1));
        self.cursor = point;
    }

    fn close_current(&mut self) {
        if self.current.len() >= 2 {
            let contour = std::mem::take(&mut self.current);
            self.contours.push(contour);
        } else {
            self.current.clear();
        }
    }

    fn finish(mut self) -> Vec<Contour> {
        self.close_current();
        self.contours
    }

    fn run(&mut self, command: char, numbers: &[f32]) {
        let relative = command.is_ascii_lowercase();
        let arity = match command.to_ascii_uppercase() {
            'M' | 'L' | 'T' => 2,
            'H' | 'V' => 1,
            'C' => 6,
            'S' | 'Q' => 4,
            'A' => 7,
            'Z' => 0,
            _ => return,
        };

        if arity == 0 {
            if !self.current.is_empty() {
                let start = self.start;
                self.push(start);
                self.close_current();
                self.cursor = start;
            }
            return;
        }
        if numbers.len() < arity {
            return;
        }

        // Repeating the arguments repeats the command, except that a repeated
        // `moveto` means `lineto` — the rule that keeps `M 0 0 1 1 2 2` from
        // drawing three disconnected points.
        let mut command = command;
        for (repeat, chunk) in numbers.chunks_exact(arity).enumerate() {
            if repeat == 1 && command.to_ascii_uppercase() == 'M' {
                command = if relative { 'l' } else { 'L' };
            }
            self.step(command, chunk, relative);
        }
    }

    fn step(&mut self, command: char, values: &[f32], relative: bool) {
        let (cx, cy) = self.cursor;
        let point = |x: f32, y: f32| if relative { (cx + x, cy + y) } else { (x, y) };

        match command.to_ascii_uppercase() {
            'M' => {
                self.close_current();
                let target = point(values[0], values[1]);
                self.start = target;
                self.push(target);
                self.last_control = None;
            }
            'L' => {
                let target = point(values[0], values[1]);
                self.push(target);
                self.last_control = None;
            }
            'H' => {
                let x = if relative { cx + values[0] } else { values[0] };
                self.push((x, cy));
                self.last_control = None;
            }
            'V' => {
                let y = if relative { cy + values[0] } else { values[0] };
                self.push((cx, y));
                self.last_control = None;
            }
            'C' => {
                let c1 = point(values[0], values[1]);
                let c2 = point(values[2], values[3]);
                let end = point(values[4], values[5]);
                self.cubic(c1, c2, end);
            }
            'S' => {
                let c1 = self.reflected_control();
                let c2 = point(values[0], values[1]);
                let end = point(values[2], values[3]);
                self.cubic(c1, c2, end);
            }
            'Q' => {
                let control = point(values[0], values[1]);
                let end = point(values[2], values[3]);
                self.quadratic(control, end);
            }
            'T' => {
                let control = self.reflected_control();
                let end = point(values[0], values[1]);
                self.quadratic(control, end);
            }
            'A' => {
                let end = point(values[5], values[6]);
                self.arc(
                    values[0].abs(),
                    values[1].abs(),
                    values[2],
                    values[3] != 0.0,
                    values[4] != 0.0,
                    end,
                );
            }
            _ => {}
        }
    }

    fn reflected_control(&self) -> (f32, f32) {
        let (cx, cy) = self.cursor;
        match self.last_control {
            Some((px, py)) => (2.0 * cx - px, 2.0 * cy - py),
            None => (cx, cy),
        }
    }

    fn cubic(&mut self, c1: (f32, f32), c2: (f32, f32), end: (f32, f32)) {
        let (x0, y0) = self.cursor;
        for step in 1..=self.steps {
            let t = step as f32 / self.steps as f32;
            let u = 1.0 - t;
            let x = u * u * u * x0 + 3.0 * u * u * t * c1.0 + 3.0 * u * t * t * c2.0 + t * t * t * end.0;
            let y = u * u * u * y0 + 3.0 * u * u * t * c1.1 + 3.0 * u * t * t * c2.1 + t * t * t * end.1;
            self.current.push(self.transform.apply(x, y));
        }
        self.cursor = end;
        self.last_control = Some(c2);
    }

    fn quadratic(&mut self, control: (f32, f32), end: (f32, f32)) {
        let (x0, y0) = self.cursor;
        for step in 1..=self.steps {
            let t = step as f32 / self.steps as f32;
            let u = 1.0 - t;
            let x = u * u * x0 + 2.0 * u * t * control.0 + t * t * end.0;
            let y = u * u * y0 + 2.0 * u * t * control.1 + t * t * end.1;
            self.current.push(self.transform.apply(x, y));
        }
        self.cursor = end;
        self.last_control = Some(control);
    }

    /// Elliptical arc, via the endpoint-to-centre conversion in SVG 1.1 F.6.5.
    fn arc(&mut self, rx: f32, ry: f32, rotation: f32, large: bool, sweep: bool, end: (f32, f32)) {
        let (x0, y0) = self.cursor;
        if rx <= 0.0 || ry <= 0.0 || (x0 - end.0).abs() < 1e-6 && (y0 - end.1).abs() < 1e-6 {
            self.push(end);
            self.last_control = None;
            return;
        }

        let (sin, cos) = rotation.to_radians().sin_cos();
        let dx = (x0 - end.0) / 2.0;
        let dy = (y0 - end.1) / 2.0;
        let x1 = cos * dx + sin * dy;
        let y1 = -sin * dx + cos * dy;

        // Radii too small to reach the endpoint are scaled up, as the spec asks.
        let lambda = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
        let (rx, ry) = if lambda > 1.0 {
            (rx * lambda.sqrt(), ry * lambda.sqrt())
        } else {
            (rx, ry)
        };

        let numerator = (rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1).max(0.0);
        let denominator = rx * rx * y1 * y1 + ry * ry * x1 * x1;
        let coefficient = if denominator <= 0.0 {
            0.0
        } else {
            (numerator / denominator).sqrt() * if large == sweep { -1.0 } else { 1.0 }
        };
        let cx1 = coefficient * rx * y1 / ry;
        let cy1 = -coefficient * ry * x1 / rx;
        let cx = cos * cx1 - sin * cy1 + (x0 + end.0) / 2.0;
        let cy = sin * cx1 + cos * cy1 + (y0 + end.1) / 2.0;

        let angle = |ux: f32, uy: f32, vx: f32, vy: f32| {
            let dot = ux * vx + uy * vy;
            let len = ((ux * ux + uy * uy) * (vx * vx + vy * vy)).sqrt();
            let mut value = if len <= 0.0 { 0.0 } else { (dot / len).clamp(-1.0, 1.0).acos() };
            if ux * vy - uy * vx < 0.0 {
                value = -value;
            }
            value
        };
        let start_angle = angle(1.0, 0.0, (x1 - cx1) / rx, (y1 - cy1) / ry);
        let mut sweep_angle = angle(
            (x1 - cx1) / rx,
            (y1 - cy1) / ry,
            (-x1 - cx1) / rx,
            (-y1 - cy1) / ry,
        );
        if !sweep && sweep_angle > 0.0 {
            sweep_angle -= std::f32::consts::TAU;
        } else if sweep && sweep_angle < 0.0 {
            sweep_angle += std::f32::consts::TAU;
        }

        let steps = self.steps.max(8);
        for step in 1..=steps {
            let theta = start_angle + sweep_angle * (step as f32 / steps as f32);
            let (sin_t, cos_t) = theta.sin_cos();
            let x = cx + rx * cos_t * cos - ry * sin_t * sin;
            let y = cy + rx * cos_t * sin + ry * sin_t * cos;
            self.current.push(self.transform.apply(x, y));
        }
        self.cursor = end;
        self.last_control = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(image: &DecodedImage, x: u32, y: u32) -> [u8; 4] {
        let index = ((y as usize) * (image.width as usize) + x as usize) * 4;
        [
            image.rgba[index],
            image.rgba[index + 1],
            image.rgba[index + 2],
            image.rgba[index + 3],
        ]
    }

    #[test]
    fn a_filled_rectangle_covers_its_area() {
        let image = rasterize(
            "<svg viewBox='0 0 10 10'><rect x='0' y='0' width='10' height='10' fill='#ff0000'/></svg>",
        )
        .expect("rasterized");
        let middle = pixel(&image, image.width / 2, image.height / 2);
        assert_eq!(middle, [255, 0, 0, 255]);
    }

    #[test]
    fn a_path_is_filled_and_its_outside_is_left_clear() {
        let image = rasterize(
            "<svg viewBox='0 0 10 10'><path d='M0 0 L10 0 L10 10 Z' fill='#000'/></svg>",
        )
        .expect("rasterized");
        // The triangle covers the top-right half, so the bottom-left corner is
        // untouched and the top-right one is solid.
        assert_eq!(pixel(&image, 1, image.height - 2)[3], 0);
        assert_eq!(pixel(&image, image.width - 2, 1)[3], 255);
    }

    #[test]
    fn fill_none_draws_nothing() {
        assert!(
            rasterize("<svg viewBox='0 0 10 10'><rect width='10' height='10' fill='none'/></svg>")
                .is_none()
        );
    }

    #[test]
    fn a_small_view_box_is_scaled_up_to_a_usable_raster() {
        let image =
            rasterize("<svg viewBox='0 0 24 24'><rect width='24' height='24'/></svg>").unwrap();
        assert!(image.width >= 96, "got {}x{}", image.width, image.height);
    }

    #[test]
    fn a_group_transform_moves_its_children() {
        let image = rasterize(
            "<svg viewBox='0 0 10 10'><g transform='translate(5 0)'>\
             <rect width='5' height='10' fill='#000'/></g></svg>",
        )
        .expect("rasterized");
        // The rectangle was declared on the left and moved to the right half.
        assert_eq!(pixel(&image, 1, image.height / 2)[3], 0);
        assert_eq!(pixel(&image, image.width - 2, image.height / 2)[3], 255);
    }

    #[test]
    fn the_even_odd_rule_punches_a_hole() {
        // Two nested squares wound the same way: non-zero would fill both, and
        // even-odd leaves the inner one clear.
        let svg = "<svg viewBox='0 0 100 100'><path fill-rule='evenodd' fill='#000' \
                   d='M0 0 H100 V100 H0 Z M25 25 H75 V75 H25 Z'/></svg>";
        let image = rasterize(svg).expect("rasterized");
        let middle = image.width / 2;
        assert_eq!(pixel(&image, middle, image.height / 2)[3], 0, "hole");
        assert_eq!(pixel(&image, 2, image.height / 2)[3], 255, "border");
    }

    #[test]
    fn a_stroke_without_a_fill_still_draws() {
        let image = rasterize(
            "<svg viewBox='0 0 10 10'><path d='M0 5 L10 5' fill='none' stroke='#000' \
             stroke-width='2'/></svg>",
        )
        .expect("rasterized");
        assert!(pixel(&image, image.width / 2, image.height / 2)[3] > 0);
    }

    #[test]
    fn a_document_with_no_svg_root_is_rejected() {
        assert!(rasterize("<html><body>not a drawing</body></html>").is_none());
    }
}
