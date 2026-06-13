use std::collections::BTreeMap;

use serde_json::Value;

use crate::css::{InteractiveState, StyledNode, Stylesheet, build_styled_tree, parse_stylesheet};
use crate::error::Result;
use crate::html::{Element, Node, parse_document};
use crate::http::fetch;
use crate::image::{ImageStore, decode_image};
use crate::js::{
    DomEventDispatchResult, DomEventRequest, JavaScriptSession, ProcessedScriptHtml,
    start_document_script_session,
};
use crate::render::render_document;
use crate::text::decode_text_response;
use crate::url::Url;

const MAX_FRAME_DEPTH: usize = 3;
const MAX_SCRIPT_NAVIGATION_DEPTH: usize = 3;

#[derive(Debug, Clone)]
pub struct BrowserPage {
    pub url: Url,
    pub status_code: u16,
    pub reason_phrase: String,
    pub content_type: Option<String>,
    pub title: String,
    pub html_source: String,
    pub styled_document: StyledNode,
    pub raw_document: Node,
    pub main_stylesheet: Stylesheet,
    pub images: ImageStore,
    pub rendered: Option<String>,
    pub javascript_session: Option<JavaScriptSession>,
    layout_revision: u64,
    scroll_y: u32,
    /// Whether the JS engine still has pending event-loop work (timers / RAF).
    /// Drives the GUI's animation pump: while true, the event loop keeps
    /// ticking the session so `setInterval` / `setTimeout` / RAF fire over time.
    engine_pending: bool,
}

// BrowserPage is moved between threads only through owned message passing:
// the load worker constructs it, then hands ownership to the UI thread, which
// owns the page afterwards. We avoid sharing the same instance concurrently.
unsafe impl Send for BrowserPage {}

impl BrowserPage {
    pub fn status_text(&self) -> String {
        format!("{} {}", self.status_code, self.reason_phrase)
            .trim()
            .to_string()
    }

    pub fn apply_script_snapshot(&mut self, snapshot: ProcessedScriptHtml) {
        // Fast path: the scripts didn't change the DOM (e.g. a scroll/resize
        // event whose listeners — if any — mutated nothing). Skip the expensive
        // full re-parse/re-layout rebuild and just update the scroll offset.
        // Without this, rapid scroll events each trigger a full page rebuild,
        // which on the engine path starves the main thread and intermittently
        // drops clicks.
        if snapshot.html == self.html_source
            && snapshot.navigation_target.is_none()
            && snapshot.soft_navigation_target.is_none()
        {
            self.scroll_y = snapshot.scroll_y;
            self.engine_pending = snapshot.has_pending_work;
            return;
        }
        let pending = snapshot.has_pending_work;
        let include_rendered_output = self.rendered.is_some();
        let javascript_session = self.javascript_session.take();
        let layout_revision = self
            .layout_revision
            .checked_add(1)
            .expect("layout revision overflowed");
        let url = snapshot
            .soft_navigation_target
            .as_deref()
            .and_then(|target| Url::parse(target).ok())
            .unwrap_or_else(|| self.url.clone());
        let rebuilt = rebuild_page_from_html(
            &url,
            self.status_code,
            self.reason_phrase.clone(),
            self.content_type.clone(),
            &snapshot.html,
            snapshot.title_override.clone(),
            include_rendered_output,
            layout_revision,
            javascript_session,
        );
        *self = rebuilt;
        self.scroll_y = snapshot.scroll_y;
        self.engine_pending = pending;
    }

    pub(crate) fn layout_revision(&self) -> u64 {
        self.layout_revision
    }

    /// Whether the JS engine still has pending event-loop work (timers / RAF).
    /// The GUI checks this to decide whether to keep pumping the session.
    pub(crate) fn engine_pending(&self) -> bool {
        self.engine_pending
    }

    /// Advance JS engine time to `now_ms`, run any due timers / `requestAnimation
    /// Frame` callbacks, and apply the resulting snapshot. Returns whether the
    /// page changed (so the caller can request a render). `engine_pending()` is
    /// updated as a side effect so the caller knows whether to keep ticking.
    pub fn tick(&mut self, now_ms: u64) -> bool {
        let Some((maybe_snapshot, has_more)) = self
            .javascript_session
            .as_ref()
            .and_then(|session| session.tick(now_ms))
        else {
            // No session (or worker gone): nothing left to pump.
            self.engine_pending = false;
            return false;
        };
        // The frame was a no-op (pending timer not yet due): no snapshot to
        // apply, just refresh whether more work remains.
        let Some(snapshot) = maybe_snapshot else {
            self.engine_pending = has_more;
            return false;
        };
        let changed = snapshot.html != self.html_source
            || snapshot.navigation_target.is_some()
            || snapshot.soft_navigation_target.is_some();
        // `apply_script_snapshot` updates `engine_pending` from the snapshot.
        self.apply_script_snapshot(snapshot);
        changed
    }

    pub(crate) fn scroll_y(&self) -> u32 {
        self.scroll_y
    }

    pub fn dispatch_dom_event(
        &mut self,
        request: DomEventRequest,
    ) -> Option<DomEventDispatchResult> {
        let result = self
            .javascript_session
            .as_ref()
            .and_then(|session| session.dispatch_event(request));
        if let Some(result) = result.clone() {
            self.apply_script_snapshot(result.snapshot.clone());
        }
        result
    }

    pub fn relayout(&mut self, viewport_width: u32, interactive: &InteractiveState) {
        self.styled_document = build_styled_tree(
            &self.raw_document,
            &self.main_stylesheet,
            viewport_width,
            interactive,
        );
    }

    pub fn set_dom_attribute(&mut self, node_id: Option<usize>, name: &str, value: &str) {
        let Some(node_id) = node_id else {
            return;
        };
        let Some(session) = self.javascript_session.as_ref().cloned() else {
            return;
        };
        if session.set_attribute(node_id, name, value)
            && let Some(snapshot) = session.snapshot()
        {
            self.apply_script_snapshot(snapshot);
        }
    }

    pub fn set_viewport_size(&mut self, width: u32, height: u32) -> bool {
        if let Some(session) = &self.javascript_session {
            return session.set_viewport_size(width, height);
        }
        false
    }

    /// Feed element geometry from the latest layout into the JS session so
    /// `getBoundingClientRect` / `offsetWidth` etc. return real values.
    /// `rects` is `(data-tobira-node-id, x, y, width, height)` (document coords).
    pub fn set_geometry(&self, rects: Vec<(usize, f32, f32, f32, f32)>) -> bool {
        if let Some(session) = &self.javascript_session {
            return session.set_geometry(rects);
        }
        false
    }

    pub fn set_scroll_position(&mut self, y: u32) -> bool {
        // Track the offset directly so `scroll_y()` is correct even when the
        // scroll event doesn't round-trip through a snapshot (e.g. the engine
        // path skips dispatch when nothing listens for `scroll`).
        self.scroll_y = y;
        if let Some(session) = &self.javascript_session {
            return session.set_scroll_position(y);
        }
        false
    }

    pub fn dispatch_window_resize(&mut self) -> Option<DomEventDispatchResult> {
        let result = self
            .javascript_session
            .as_ref()
            .and_then(|session| session.dispatch_global_event("resize", false, false))?;
        self.apply_script_snapshot(result.snapshot.clone());
        Some(result)
    }

    pub fn dispatch_scroll_event(&mut self) -> Option<DomEventDispatchResult> {
        let result = self
            .javascript_session
            .as_ref()
            .and_then(|session| session.dispatch_global_event("scroll", false, false))?;
        self.apply_script_snapshot(result.snapshot.clone());
        Some(result)
    }

    pub fn body_text(&self) -> &str {
        match &self.rendered {
            Some(rendered) => {
                let trimmed = rendered.trim();
                if trimmed.is_empty() {
                    "[empty document]"
                } else {
                    trimmed
                }
            }
            None => "[render unavailable]",
        }
    }

    pub fn to_cli_output(&self) -> String {
        let mut output = String::new();
        output.push_str(&format!("URL: {}\n", self.url));
        output.push_str(&format!("Status: {}\n", self.status_text()));
        if let Some(content_type) = &self.content_type {
            output.push_str(&format!("Content-Type: {content_type}\n"));
        }
        output.push('\n');
        output.push_str(self.body_text());
        output.push('\n');
        output
    }
}

#[derive(Debug, Clone)]
struct LoadedDocumentSource {
    final_url: Url,
    status_code: u16,
    reason_phrase: String,
    content_type: Option<String>,
    document: Node,
    processed_html: ProcessedScriptHtml,
    javascript_session: Option<JavaScriptSession>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FrameSpec {
    src: String,
    title: Option<String>,
}

pub fn load_page(url: &Url) -> Result<BrowserPage> {
    load_page_with_options(url, false)
}

pub fn load_page_for_cli(url: &Url) -> Result<BrowserPage> {
    load_page_with_options(url, true)
}

fn load_page_with_options(url: &Url, include_rendered_output: bool) -> Result<BrowserPage> {
    let source = load_document_source(url, 0)?;
    let mut page = rebuild_page_from_document(
        &source.final_url,
        source.status_code,
        source.reason_phrase,
        source.content_type,
        source.document,
        source.processed_html.html,
        source.processed_html.title_override,
        include_rendered_output,
        0,
        source.javascript_session,
    );
    page.scroll_y = source.processed_html.scroll_y;
    page.engine_pending = source.processed_html.has_pending_work;
    if let Some(soft_target) = source
        .processed_html
        .soft_navigation_target
        .as_deref()
        .and_then(|target| Url::parse(target).ok())
    {
        page.url = soft_target;
    }
    Ok(page)
}

fn rebuild_page_from_html(
    url: &Url,
    status_code: u16,
    reason_phrase: String,
    content_type: Option<String>,
    html: &str,
    title_override: Option<String>,
    include_rendered_output: bool,
    layout_revision: u64,
    javascript_session: Option<JavaScriptSession>,
) -> BrowserPage {
    let mut parsed_document = parse_document(html);
    if let Some(rewritten) = build_site_specific_document(&parsed_document, html, url) {
        parsed_document = rewritten;
    } else if is_google_host(url) && !document_has_meaningful_body(&parsed_document) {
        parsed_document = build_google_document_from_html(html, url);
    } else if is_youtube_host(url)
        && !is_youtube_watch_url(url)
        && !document_has_meaningful_body(&parsed_document)
    {
        parsed_document = build_youtube_generic_document_from_html(html, url);
    }
    annotate_resource_urls(&mut parsed_document, url);
    let document = expand_frames(&parsed_document, url, 1)
        .ok()
        .flatten()
        .unwrap_or(parsed_document);
    rebuild_page_from_document(
        url,
        status_code,
        reason_phrase,
        content_type,
        document,
        html.to_string(),
        title_override,
        include_rendered_output,
        layout_revision,
        javascript_session,
    )
}

fn rebuild_page_from_document(
    url: &Url,
    status_code: u16,
    reason_phrase: String,
    content_type: Option<String>,
    mut document: Node,
    html_source: String,
    title_override: Option<String>,
    include_rendered_output: bool,
    layout_revision: u64,
    javascript_session: Option<JavaScriptSession>,
) -> BrowserPage {
    annotate_node_ids(&mut document);
    let original_title = title_override.or_else(|| document_title(&document));
    let title = original_title
        .or_else(|| document_title(&document))
        .or_else(|| first_heading(&document))
        .unwrap_or_else(|| "Tobira".to_string());

    let stylesheet = collect_stylesheet(&document, url);
    let mut images = collect_image_resources(&document);
    let rendered = include_rendered_output.then(|| render_document(&document));
    let styled_document =
        build_styled_tree(&document, &stylesheet, 1280, &InteractiveState::default());
    collect_styled_background_images(&styled_document, url, &mut images);

    BrowserPage {
        url: url.clone(),
        status_code,
        reason_phrase,
        content_type,
        title,
        html_source,
        styled_document,
        raw_document: document,
        main_stylesheet: stylesheet,
        images,
        rendered,
        javascript_session,
        layout_revision,
        scroll_y: 0,
        engine_pending: false,
    }
}

fn load_document_source(url: &Url, frame_depth: usize) -> Result<LoadedDocumentSource> {
    load_document_source_with_script_navigation(url, frame_depth, 0)
}

fn load_document_source_with_script_navigation(
    url: &Url,
    frame_depth: usize,
    script_navigation_depth: usize,
) -> Result<LoadedDocumentSource> {
    let response = fetch(url)?;
    let content_type = response.header("content-type").map(str::to_string);
    let text = decode_text_response(&response.body, response.header("content-type"));
    let (scripted, javascript_session) = start_document_script_session(&text, &response.final_url);
    if let Some(target) = scripted.navigation_target.as_deref()
        && target != response.final_url.to_string()
        && script_navigation_depth < MAX_SCRIPT_NAVIGATION_DEPTH
        && let Ok(next_url) = Url::parse(target)
    {
        if should_follow_script_navigation(&response.final_url, &next_url) {
            return load_document_source_with_script_navigation(
                &next_url,
                frame_depth,
                script_navigation_depth + 1,
            );
        }
    }
    let mut parsed_document = parse_document(&scripted.html);
    if let Some(rewritten) =
        build_site_specific_document(&parsed_document, &text, &response.final_url)
    {
        parsed_document = rewritten;
    } else if is_google_host(&response.final_url) && !document_has_meaningful_body(&parsed_document)
    {
        parsed_document = build_google_document_from_html(&text, &response.final_url);
    } else if is_youtube_host(&response.final_url)
        && !is_youtube_watch_url(&response.final_url)
        && !document_has_meaningful_body(&parsed_document)
    {
        parsed_document = build_youtube_generic_document_from_html(&text, &response.final_url);
    }
    annotate_resource_urls(&mut parsed_document, &response.final_url);
    let document = if frame_depth < MAX_FRAME_DEPTH {
        expand_frames(&parsed_document, &response.final_url, frame_depth + 1)?
            .unwrap_or(parsed_document)
    } else {
        parsed_document
    };
    Ok(LoadedDocumentSource {
        final_url: response.final_url,
        status_code: response.status_code,
        reason_phrase: response.reason_phrase,
        content_type,
        document,
        processed_html: scripted,
        javascript_session,
    })
}

pub(crate) fn annotate_node_ids(document: &mut Node) {
    fn walk(node: &mut Node, next_id: &mut usize) {
        if let Node::Element(element) = node {
            element
                .attributes
                .insert("data-tobira-node-id".to_string(), next_id.to_string());
            *next_id = next_id
                .checked_add(1)
                .expect("document node id counter overflowed");
            for child in &mut element.children {
                walk(child, next_id);
            }
        }
    }

    let mut next_id = 1;
    walk(document, &mut next_id);
}

fn expand_frames(document: &Node, base_url: &Url, frame_depth: usize) -> Result<Option<Node>> {
    if let Some(frameset) = first_frameset(document)
        && let Some(layout) = expand_frameset(document, frameset, base_url, frame_depth)?
    {
        return Ok(Some(layout));
    }

    let mut frames = collect_frame_specs(document);
    if frames.is_empty() {
        return Ok(None);
    }

    frames.sort_by_key(frame_display_priority);

    let mut body_children = Vec::new();
    let multiple_frames = frames.len() > 1;

    for frame in frames {
        let Ok(frame_url) = base_url.resolve(&frame.src) else {
            continue;
        };

        let Ok(frame_document) = load_document_source(&frame_url, frame_depth) else {
            continue;
        };

        if multiple_frames && let Some(section_title) = frame_section_title(&frame, &frame_document)
        {
            body_children.push(section_heading_node(&section_title));
        }

        body_children.push(frame_document.document);

        if multiple_frames {
            body_children.push(hr_node());
        }
    }

    if body_children.is_empty() {
        return Ok(None);
    }

    if multiple_frames
        && matches!(
            body_children.last(),
            Some(Node::Element(element)) if element.tag_name == "hr"
        )
    {
        body_children.pop();
    }

    let title = document_title(document).unwrap_or_else(|| "Tobira".to_string());
    Ok(Some(synthetic_document(&title, body_children)))
}

fn first_frameset(node: &Node) -> Option<&Element> {
    match node {
        Node::Text(_) => None,
        Node::Element(element) => {
            if element.tag_name == "frameset" {
                return Some(element);
            }

            for child in &element.children {
                if let Some(found) = first_frameset(child) {
                    return Some(found);
                }
            }

            None
        }
    }
}

fn expand_frameset(
    document: &Node,
    frameset: &Element,
    base_url: &Url,
    frame_depth: usize,
) -> Result<Option<Node>> {
    let frames = frameset
        .children
        .iter()
        .filter_map(|child| match child {
            Node::Element(element) if element.tag_name == "frame" => Some(element),
            _ => None,
        })
        .collect::<Vec<_>>();
    if frames.is_empty() {
        return Ok(None);
    }

    let title = document_title(document).unwrap_or_else(|| "Tobira".to_string());
    let cols = frameset
        .attribute("cols")
        .map(|value| parse_frame_tracks(value, frames.len()));
    let rows = frameset
        .attribute("rows")
        .map(|value| parse_frame_tracks(value, frames.len()));

    // Collect (children, body_style_attrs) tuples so bgcolor/background from each
    // frame's <body> element can be transferred to the synthetic wrapper <td>.
    let mut frame_nodes: Vec<(Vec<Node>, BTreeMap<String, String>)> = Vec::new();
    for frame in frames {
        let Some(src) = frame.attribute("src") else {
            continue;
        };
        let Ok(frame_url) = base_url.resolve(src) else {
            continue;
        };
        let Ok(frame_document) = load_document_source(&frame_url, frame_depth) else {
            continue;
        };
        let children = extract_body_children(&frame_document.document);
        let body_attrs = extract_body_style_attrs(&frame_document.document);
        frame_nodes.push((children, body_attrs));
    }

    if frame_nodes.is_empty() {
        return Ok(None);
    }

    if let Some(cols) = cols
        && cols.len() == frame_nodes.len()
    {
        return Ok(Some(synthetic_frameset_columns_document(
            &title,
            &cols,
            frame_nodes,
        )));
    }

    if let Some(rows) = rows
        && rows.len() == frame_nodes.len()
    {
        return Ok(Some(synthetic_frameset_rows_document(
            &title,
            &rows,
            frame_nodes,
        )));
    }

    let mut body_children = Vec::new();
    for (children, _) in frame_nodes {
        body_children.extend(children);
        body_children.push(hr_node());
    }
    if matches!(
        body_children.last(),
        Some(Node::Element(element)) if element.tag_name == "hr"
    ) {
        body_children.pop();
    }

    Ok(Some(synthetic_document(&title, body_children)))
}

fn collect_frame_specs(document: &Node) -> Vec<FrameSpec> {
    let mut frames = Vec::new();
    collect_frame_specs_into(document, &mut frames);
    frames
}

fn collect_frame_specs_into(node: &Node, output: &mut Vec<FrameSpec>) {
    match node {
        Node::Text(_) => {}
        Node::Element(element) => {
            if matches!(element.tag_name.as_str(), "frame" | "iframe")
                && let Some(src) = element.attribute("src")
                && !src.trim().is_empty()
            {
                let title = element
                    .attribute("title")
                    .or_else(|| element.attribute("name"))
                    .map(str::to_string);
                output.push(FrameSpec {
                    src: src.to_string(),
                    title,
                });
            }

            for child in &element.children {
                collect_frame_specs_into(child, output);
            }
        }
    }
}

fn section_heading_node(title: &str) -> Node {
    Node::Element(Element {
        tag_name: "h2".to_string(),
        attributes: BTreeMap::new(),
        children: vec![Node::Text(title.to_string())],
    })
}

fn frame_section_title(frame: &FrameSpec, frame_document: &LoadedDocumentSource) -> Option<String> {
    let frame_title = document_title(&frame_document.document)
        .or_else(|| first_heading(&frame_document.document))
        .unwrap_or_default();

    if let Some(first_heading) = first_heading(&frame_document.document)
        && first_heading == frame_title
    {
        return None;
    }

    if !frame_title.trim().is_empty() && frame_title != "Tobira" {
        return Some(frame_title);
    }

    frame
        .title
        .clone()
        .or_else(|| Some(frame.src.clone()))
        .filter(|title| !title.trim().is_empty())
}

fn frame_display_priority(frame: &FrameSpec) -> u8 {
    let hint = format!(
        "{} {}",
        frame.title.as_deref().unwrap_or_default(),
        frame.src
    )
    .to_ascii_lowercase();

    if hint.contains("menu") || hint.contains("left") || hint.contains("nav") {
        1
    } else {
        0
    }
}

fn hr_node() -> Node {
    Node::Element(Element {
        tag_name: "hr".to_string(),
        attributes: BTreeMap::new(),
        children: Vec::new(),
    })
}

fn synthetic_document(title: &str, body_children: Vec<Node>) -> Node {
    Node::Element(Element {
        tag_name: "document".to_string(),
        attributes: BTreeMap::new(),
        children: vec![Node::Element(Element {
            tag_name: "html".to_string(),
            attributes: BTreeMap::new(),
            children: vec![
                Node::Element(Element {
                    tag_name: "head".to_string(),
                    attributes: BTreeMap::new(),
                    children: vec![Node::Element(Element {
                        tag_name: "title".to_string(),
                        attributes: BTreeMap::new(),
                        children: vec![Node::Text(title.to_string())],
                    })],
                }),
                Node::Element(Element {
                    tag_name: "body".to_string(),
                    attributes: BTreeMap::new(),
                    children: body_children,
                }),
            ],
        })],
    })
}

fn synthetic_frameset_columns_document(
    title: &str,
    tracks: &[FrameTrack],
    frame_nodes: Vec<(Vec<Node>, BTreeMap<String, String>)>,
) -> Node {
    let cells = frame_nodes
        .into_iter()
        .zip(tracks.iter())
        .map(|((children, body_attrs), track)| {
            let mut attrs = table_cell_attributes(track);
            // Carry bgcolor / text / data-scratch-background from the frame's <body>
            // so the synthetic <td> inherits the frame's background color.
            for key in &["bgcolor", "text", "data-scratch-background"] {
                if let Some(val) = body_attrs.get(*key) {
                    attrs.insert(key.to_string(), val.clone());
                }
            }
            Node::Element(Element {
                tag_name: "td".to_string(),
                attributes: attrs,
                children,
            })
        })
        .collect();

    synthetic_document(
        title,
        vec![Node::Element(Element {
            tag_name: "table".to_string(),
            attributes: full_width_table_attributes(),
            children: vec![Node::Element(Element {
                tag_name: "tr".to_string(),
                attributes: BTreeMap::new(),
                children: cells,
            })],
        })],
    )
}

fn synthetic_frameset_rows_document(
    title: &str,
    tracks: &[FrameTrack],
    frame_nodes: Vec<(Vec<Node>, BTreeMap<String, String>)>,
) -> Node {
    let rows = frame_nodes
        .into_iter()
        .zip(tracks.iter())
        .map(|((children, body_attrs), track)| {
            let mut td_attrs = BTreeMap::new();
            for key in &["bgcolor", "text", "data-scratch-background"] {
                if let Some(val) = body_attrs.get(*key) {
                    td_attrs.insert(key.to_string(), val.clone());
                }
            }
            Node::Element(Element {
                tag_name: "tr".to_string(),
                attributes: row_attributes(track),
                children: vec![Node::Element(Element {
                    tag_name: "td".to_string(),
                    attributes: td_attrs,
                    children,
                })],
            })
        })
        .collect();

    synthetic_document(
        title,
        vec![Node::Element(Element {
            tag_name: "table".to_string(),
            attributes: full_width_table_attributes(),
            children: rows,
        })],
    )
}

fn full_width_table_attributes() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("width".to_string(), "100%".to_string()),
        ("border".to_string(), "0".to_string()),
        ("cellpadding".to_string(), "0".to_string()),
        ("cellspacing".to_string(), "0".to_string()),
    ])
}

fn table_cell_attributes(track: &FrameTrack) -> BTreeMap<String, String> {
    let mut attributes = BTreeMap::from([("valign".to_string(), "top".to_string())]);
    match track {
        FrameTrack::Percent(value) => {
            attributes.insert("width".to_string(), format!("{value}%"));
        }
        FrameTrack::Pixels(value) if *value > 0 => {
            attributes.insert("width".to_string(), value.to_string());
        }
        FrameTrack::Flex(_) | FrameTrack::Pixels(_) => {}
    }
    attributes
}

fn row_attributes(track: &FrameTrack) -> BTreeMap<String, String> {
    match track {
        FrameTrack::Percent(value) => BTreeMap::from([("height".to_string(), format!("{value}%"))]),
        FrameTrack::Pixels(value) if *value > 0 => {
            BTreeMap::from([("height".to_string(), value.to_string())])
        }
        FrameTrack::Flex(_) | FrameTrack::Pixels(_) => BTreeMap::new(),
    }
}

fn extract_body_children(node: &Node) -> Vec<Node> {
    match node {
        Node::Text(_) => Vec::new(),
        Node::Element(element) => {
            if element.tag_name == "body" {
                return element.children.clone();
            }

            for child in &element.children {
                let extracted = extract_body_children(child);
                if !extracted.is_empty() {
                    return extracted;
                }
            }

            Vec::new()
        }
    }
}

/// Extract style-relevant attributes from the `<body>` element so they can be
/// transferred to the synthetic `<td>` wrapper when merging frameset frames.
/// Preserves `bgcolor`, `text` (body text color), and `data-scratch-background`.
fn extract_body_style_attrs(node: &Node) -> BTreeMap<String, String> {
    match node {
        Node::Text(_) => BTreeMap::new(),
        Node::Element(element) => {
            if element.tag_name == "body" {
                let mut attrs = BTreeMap::new();
                for key in &["bgcolor", "text", "data-scratch-background"] {
                    if let Some(val) = element.attributes.get(*key) {
                        attrs.insert(key.to_string(), val.clone());
                    }
                }
                return attrs;
            }
            for child in &element.children {
                let attrs = extract_body_style_attrs(child);
                if !attrs.is_empty() {
                    return attrs;
                }
            }
            BTreeMap::new()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrameTrack {
    Percent(u32),
    Pixels(u32),
    Flex(u32),
}

fn parse_frame_tracks(input: &str, count: usize) -> Vec<FrameTrack> {
    let tokens = input
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if tokens.is_empty() {
        return vec![FrameTrack::Flex(1); count];
    }

    let all_numeric = tokens
        .iter()
        .all(|value| value.chars().all(|character| character.is_ascii_digit()));
    let numeric_sum = tokens
        .iter()
        .filter_map(|value| value.parse::<u32>().ok())
        .sum::<u32>();

    tokens
        .into_iter()
        .map(|token| {
            if let Some(percent) = token.strip_suffix('%')
                && let Ok(value) = percent.parse::<u32>()
            {
                return FrameTrack::Percent(value);
            }

            if let Some(flex) = token.strip_suffix('*') {
                let weight = flex.parse::<u32>().unwrap_or(1).max(1);
                return FrameTrack::Flex(weight);
            }

            if all_numeric && numeric_sum == 100 {
                return FrameTrack::Percent(token.parse::<u32>().unwrap_or(0));
            }

            FrameTrack::Pixels(token.parse::<u32>().unwrap_or(0))
        })
        .collect()
}

fn annotate_resource_urls(document: &mut Node, base_url: &Url) {
    match document {
        Node::Text(_) => {}
        Node::Element(element) => {
            if element.tag_name == "img"
                && let Some(src) = element.attribute("src")
                && let Ok(url) = base_url.resolve(src)
            {
                element
                    .attributes
                    .insert("data-scratch-src".to_string(), url.to_string());
            }

            if element.tag_name == "body"
                && let Some(background) = element.attribute("background")
                && let Ok(url) = base_url.resolve(background)
            {
                element
                    .attributes
                    .insert("data-scratch-background".to_string(), url.to_string());
            }

            if element.tag_name == "a"
                && let Some(href) = element.attribute("href")
                && !href.starts_with('#')
                && let Ok(url) = base_url.resolve(href)
            {
                element
                    .attributes
                    .insert("href".to_string(), url.to_string());
            }

            for child in &mut element.children {
                annotate_resource_urls(child, base_url);
            }
        }
    }
}

fn collect_image_resources(document: &Node) -> ImageStore {
    let mut sources = Vec::new();
    collect_image_sources_into(document, &mut sources);

    let mut images = ImageStore::default();
    for source in sources {
        let Ok(url) = Url::parse(&source) else {
            continue;
        };
        let Ok(response) = fetch(&url) else {
            continue;
        };
        let Ok(image) = decode_image(&response.body) else {
            continue;
        };
        images.insert(source, image);
    }

    images
}

fn collect_styled_background_images(styled: &StyledNode, base_url: &Url, images: &mut ImageStore) {
    match styled {
        StyledNode::Text(_) => {}
        StyledNode::Element(element) => {
            if let Some(ref url_str) = element.style.background_image_url {
                if images.get(url_str).is_none() {
                    let resolved =
                        if url_str.starts_with("http://") || url_str.starts_with("https://") {
                            Url::parse(url_str).ok()
                        } else {
                            base_url.resolve(url_str).ok()
                        };
                    if let Some(resolved_url) = resolved {
                        if let Ok(response) = fetch(&resolved_url) {
                            if let Ok(image) = decode_image(&response.body) {
                                images.insert(url_str.clone(), image);
                            }
                        }
                    }
                }
            }
            for child in &element.children {
                collect_styled_background_images(child, base_url, images);
            }
        }
    }
}

fn collect_image_sources_into(node: &Node, output: &mut Vec<String>) {
    match node {
        Node::Text(_) => {}
        Node::Element(element) => {
            if element.tag_name == "img"
                && let Some(src) = element.attribute("data-scratch-src")
                && !output.iter().any(|known| known == src)
            {
                output.push(src.to_string());
            }

            for child in &element.children {
                collect_image_sources_into(child, output);
            }
        }
    }
}

fn collect_stylesheet(document: &Node, base_url: &Url) -> Stylesheet {
    let mut stylesheet = Stylesheet::default();

    for style_text in collect_style_blocks(document) {
        stylesheet.extend(parse_stylesheet(&style_text));
    }

    for href in collect_stylesheet_links(document) {
        let Ok(url) = base_url.resolve(&href) else {
            continue;
        };
        let Ok(response) = fetch(&url) else {
            continue;
        };
        let css_text = decode_text_response(&response.body, response.header("content-type"));
        stylesheet.extend(parse_stylesheet(&css_text));
    }

    stylesheet
}

fn collect_style_blocks(document: &Node) -> Vec<String> {
    let mut blocks = Vec::new();
    collect_style_blocks_into(document, &mut blocks);
    blocks
}

fn collect_style_blocks_into(node: &Node, output: &mut Vec<String>) {
    match node {
        Node::Text(_) => {}
        Node::Element(element) => {
            if element.tag_name == "style" {
                let content = collect_raw_text(node);
                if !content.trim().is_empty() {
                    output.push(content);
                }
                return;
            }

            for child in &element.children {
                collect_style_blocks_into(child, output);
            }
        }
    }
}

fn collect_stylesheet_links(document: &Node) -> Vec<String> {
    let mut links = Vec::new();
    collect_stylesheet_links_into(document, &mut links);
    links
}

fn collect_stylesheet_links_into(node: &Node, output: &mut Vec<String>) {
    match node {
        Node::Text(_) => {}
        Node::Element(element) => {
            if element.tag_name == "link"
                && element
                    .attribute("rel")
                    .map(|value| {
                        value
                            .split_whitespace()
                            .any(|token| token.eq_ignore_ascii_case("stylesheet"))
                    })
                    .unwrap_or(false)
            {
                if let Some(href) = element.attribute("href") {
                    output.push(href.to_string());
                }
            }

            for child in &element.children {
                collect_stylesheet_links_into(child, output);
            }
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct YouTubeRelatedVideo {
    title: String,
    channel: Option<String>,
    views: Option<String>,
    published: Option<String>,
    duration: Option<String>,
    url: Option<String>,
    thumbnail_url: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct YouTubeCommentPreview {
    author: String,
    body: String,
    likes: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct YouTubeWatchData {
    title: String,
    channel: Option<String>,
    subscribers: Option<String>,
    description: Option<String>,
    view_count: Option<String>,
    duration: Option<String>,
    published: Option<String>,
    thumbnail_url: Option<String>,
    canonical_url: Option<String>,
    embed_url: Option<String>,
    comment_count: Option<String>,
    related_videos: Vec<YouTubeRelatedVideo>,
    featured_comments: Vec<YouTubeCommentPreview>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct YouTubeHomeData {
    title: String,
    section_title: String,
    search_placeholder: Option<String>,
    settings_label: Option<String>,
    login_label: Option<String>,
    nudge_title: Option<String>,
    nudge_subtitle: Option<String>,
    category_chips: Vec<String>,
    featured_videos: Vec<YouTubeRelatedVideo>,
    quick_links: Vec<String>,
}

fn build_site_specific_document(document: &Node, html: &str, url: &Url) -> Option<Node> {
    if is_youtube_watch_url(url) {
        let data = extract_youtube_watch_data(document, html, url)?;
        return Some(build_youtube_watch_document(&data));
    }

    None
}

fn is_youtube_host(url: &Url) -> bool {
    let host = url.host.to_ascii_lowercase();
    host == "youtube.com" || host.ends_with(".youtube.com")
}

fn is_google_host(url: &Url) -> bool {
    let host = url.host.to_ascii_lowercase();
    host == "google.com" || host.ends_with(".google.com")
}

fn is_youtube_watch_url(url: &Url) -> bool {
    is_youtube_host(url) && url.path.starts_with("/watch")
}

fn should_follow_script_navigation(current_url: &Url, target_url: &Url) -> bool {
    current_url.shares_origin(target_url)
}

fn document_has_meaningful_body(document: &Node) -> bool {
    extract_body_children(document)
        .iter()
        .any(node_has_meaningful_content)
}

fn node_has_meaningful_content(node: &Node) -> bool {
    match node {
        Node::Text(text) => !text.trim().is_empty(),
        Node::Element(element) => {
            if matches!(
                element.tag_name.as_str(),
                "script" | "style" | "meta" | "link" | "noscript" | "head" | "title"
            ) {
                return false;
            }

            if matches!(
                element.tag_name.as_str(),
                "img"
                    | "input"
                    | "button"
                    | "textarea"
                    | "select"
                    | "option"
                    | "video"
                    | "audio"
                    | "canvas"
                    | "svg"
                    | "iframe"
            ) {
                return true;
            }

            element.children.iter().any(node_has_meaningful_content)
        }
    }
}

fn extract_youtube_watch_data_from_html(html: &str, url: &Url) -> Option<YouTubeWatchData> {
    let player_response = extract_assigned_json_object(
        html,
        &[
            "var ytInitialPlayerResponse =",
            "window['ytInitialPlayerResponse'] =",
            "window[\"ytInitialPlayerResponse\"] =",
        ],
    )
    .and_then(|json| serde_json::from_str::<Value>(&json).ok());
    let initial_data = extract_assigned_json_object(
        html,
        &[
            "var ytInitialData =",
            "window['ytInitialData'] =",
            "window[\"ytInitialData\"] =",
        ],
    )
    .and_then(|json| serde_json::from_str::<Value>(&json).ok());
    let ld_json = extract_first_ld_json_video_object(html);

    let title = player_response
        .as_ref()
        .and_then(|value| value.pointer("/videoDetails/title"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| extract_meta_content_from_html(html, "name", "title"))
        .or_else(|| extract_meta_content_from_html(html, "property", "og:title"))
        .or_else(|| extract_html_tag_text(html, "title"))?;

    let description = player_response
        .as_ref()
        .and_then(|value| value.pointer("/videoDetails/shortDescription"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| extract_meta_content_from_html(html, "name", "description"))
        .or_else(|| extract_meta_content_from_html(html, "property", "og:description"));
    let channel = player_response
        .as_ref()
        .and_then(|value| value.pointer("/videoDetails/author"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            initial_data
                .as_ref()
                .and_then(extract_owner_renderer)
                .and_then(|owner| json_text(owner.get("title")?))
        })
        .or_else(|| {
            ld_json
                .as_ref()
                .and_then(|value| value.pointer("/author/name"))
                .and_then(Value::as_str)
                .map(str::to_string)
        });
    let subscribers = initial_data
        .as_ref()
        .and_then(extract_owner_renderer)
        .and_then(|owner| json_text(owner.get("subscriberCountText")?));
    let view_count = player_response
        .as_ref()
        .and_then(|value| value.pointer("/videoDetails/viewCount"))
        .and_then(Value::as_str)
        .map(format_numeric_count)
        .or_else(|| {
            ld_json
                .as_ref()
                .and_then(|value| value.pointer("/interactionStatistic/1/userInteractionCount"))
                .and_then(Value::as_str)
                .map(format_numeric_count)
        });
    let duration = player_response
        .as_ref()
        .and_then(|value| value.pointer("/videoDetails/lengthSeconds"))
        .and_then(Value::as_str)
        .and_then(format_duration_seconds)
        .or_else(|| {
            ld_json
                .as_ref()
                .and_then(|value| value.get("duration"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .and_then(parse_iso8601_duration)
        });
    let published = player_response
        .as_ref()
        .and_then(|value| {
            value
                .pointer("/microformat/playerMicroformatRenderer/publishDate")
                .and_then(Value::as_str)
        })
        .map(str::to_string)
        .or_else(|| {
            ld_json
                .as_ref()
                .and_then(|value| value.get("uploadDate"))
                .and_then(Value::as_str)
                .map(str::to_string)
        });
    let thumbnail_url = player_response
        .as_ref()
        .and_then(extract_thumbnail_url)
        .or_else(|| extract_link_href_from_html(html, "rel", "image_src"))
        .or_else(|| extract_meta_content_from_html(html, "property", "og:image"));
    let canonical_url = extract_link_href_from_html(html, "rel", "canonical")
        .or_else(|| extract_meta_content_from_html(html, "property", "og:url"))
        .or_else(|| Some(url.to_string()));
    let embed_url = player_response
        .as_ref()
        .and_then(|value| {
            value
                .pointer("/microformat/playerMicroformatRenderer/embed/iframeUrl")
                .and_then(Value::as_str)
        })
        .map(str::to_string)
        .or_else(|| extract_meta_content_from_html(html, "property", "og:video:url"))
        .or_else(|| {
            player_response
                .as_ref()
                .and_then(|value| value.pointer("/videoDetails/videoId"))
                .and_then(Value::as_str)
                .map(|video_id| format!("https://www.youtube.com/embed/{video_id}"))
        });
    let comment_count = initial_data
        .as_ref()
        .and_then(extract_comments_panel_header)
        .and_then(|header| json_text(header.get("contextualInfo")?));
    let related_videos = initial_data
        .as_ref()
        .map(extract_related_videos)
        .unwrap_or_default();
    let featured_comments = ld_json
        .as_ref()
        .map(extract_featured_comments_from_ld_json)
        .unwrap_or_default();

    Some(YouTubeWatchData {
        title,
        channel,
        subscribers,
        description,
        view_count,
        duration,
        published,
        thumbnail_url,
        canonical_url,
        embed_url,
        comment_count,
        related_videos,
        featured_comments,
    })
}

fn extract_youtube_watch_data(document: &Node, html: &str, url: &Url) -> Option<YouTubeWatchData> {
    let html_data = extract_youtube_watch_data_from_html(html, url)?;
    let player_response = extract_assigned_json_object(
        html,
        &[
            "var ytInitialPlayerResponse =",
            "window['ytInitialPlayerResponse'] =",
            "window[\"ytInitialPlayerResponse\"] =",
        ],
    )
    .and_then(|json| serde_json::from_str::<Value>(&json).ok());

    let title = player_response
        .as_ref()
        .and_then(|value| value.pointer("/videoDetails/title"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| find_meta_content(document, "name", "title"))
        .or_else(|| find_meta_content(document, "property", "og:title"))
        .or_else(|| document_title(document))
        .unwrap_or(html_data.title);

    let description = player_response
        .as_ref()
        .and_then(|value| value.pointer("/videoDetails/shortDescription"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| find_meta_content(document, "name", "description"))
        .or_else(|| find_meta_content(document, "property", "og:description"))
        .or(html_data.description);
    let channel = player_response
        .as_ref()
        .and_then(|value| value.pointer("/videoDetails/author"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| find_link_content(document, "itemprop", "name"))
        .or(html_data.channel);
    let view_count = player_response
        .as_ref()
        .and_then(|value| value.pointer("/videoDetails/viewCount"))
        .and_then(Value::as_str)
        .map(format_numeric_count)
        .or_else(|| find_interaction_count(document, "WatchAction"))
        .or(html_data.view_count);
    let duration = player_response
        .as_ref()
        .and_then(|value| value.pointer("/videoDetails/lengthSeconds"))
        .and_then(Value::as_str)
        .and_then(format_duration_seconds)
        .or_else(|| {
            find_meta_content(document, "itemprop", "duration").and_then(parse_iso8601_duration)
        })
        .or(html_data.duration);
    let published = player_response
        .as_ref()
        .and_then(|value| {
            value
                .pointer("/microformat/playerMicroformatRenderer/publishDate")
                .and_then(Value::as_str)
        })
        .map(str::to_string)
        .or_else(|| find_meta_content(document, "itemprop", "datePublished"))
        .or(html_data.published);
    let thumbnail_url = player_response
        .as_ref()
        .and_then(extract_thumbnail_url)
        .or_else(|| find_link_href(document, "rel", "image_src"))
        .or_else(|| find_meta_content(document, "property", "og:image"))
        .or(html_data.thumbnail_url);
    let embed_url = player_response
        .as_ref()
        .and_then(|value| {
            value
                .pointer("/microformat/playerMicroformatRenderer/embed/iframeUrl")
                .and_then(Value::as_str)
        })
        .map(str::to_string)
        .or_else(|| find_meta_content(document, "property", "og:video:url"))
        .or_else(|| {
            player_response
                .as_ref()
                .and_then(|value| value.pointer("/videoDetails/videoId"))
                .and_then(Value::as_str)
                .map(|video_id| format!("https://www.youtube.com/embed/{video_id}"))
        });
    let canonical_url = find_link_href(document, "rel", "canonical")
        .or_else(|| find_meta_content(document, "property", "og:url"))
        .or(html_data.canonical_url)
        .or_else(|| Some(url.to_string()));

    Some(YouTubeWatchData {
        title,
        channel,
        subscribers: html_data.subscribers,
        description,
        view_count,
        duration,
        published,
        thumbnail_url,
        canonical_url,
        embed_url,
        comment_count: html_data.comment_count,
        related_videos: html_data.related_videos,
        featured_comments: html_data.featured_comments,
    })
}

fn build_youtube_watch_document(data: &YouTubeWatchData) -> Node {
    let mut left_column = vec![simple_text_element("h1", &data.title)];
    if let Some(thumbnail_url) = &data.thumbnail_url {
        left_column.push(Node::Element(Element {
            tag_name: "img".to_string(),
            attributes: BTreeMap::from([
                ("src".to_string(), thumbnail_url.clone()),
                ("data-scratch-src".to_string(), thumbnail_url.clone()),
                ("width".to_string(), "720".to_string()),
                ("alt".to_string(), data.title.clone()),
            ]),
            children: Vec::new(),
        }));
    }

    if let Some(channel) = data.channel.as_deref() {
        left_column.push(simple_text_element("h2", channel));
    }
    push_detail(&mut left_column, "Subscribers", data.subscribers.as_deref());
    push_detail(&mut left_column, "Views", data.view_count.as_deref());
    push_detail(&mut left_column, "Published", data.published.as_deref());
    push_detail(&mut left_column, "Length", data.duration.as_deref());
    push_detail(&mut left_column, "Comments", data.comment_count.as_deref());
    push_detail(&mut left_column, "Watch URL", data.canonical_url.as_deref());
    push_detail(&mut left_column, "Embed URL", data.embed_url.as_deref());
    left_column.push(hr_node());
    left_column.push(simple_text_element("h2", "Description"));

    let description = data
        .description
        .as_deref()
        .unwrap_or("No description was embedded in the page.");
    let mut pushed_description = false;
    for paragraph in description
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(12)
    {
        left_column.push(simple_text_element("p", paragraph));
        pushed_description = true;
    }
    if !pushed_description {
        left_column.push(simple_text_element("p", description.trim()));
    }
    if !data.featured_comments.is_empty() {
        left_column.push(hr_node());
        left_column.push(simple_text_element("h2", "Featured Comments"));
        for comment in data.featured_comments.iter().take(3) {
            left_column.push(simple_text_element("h3", &comment.author));
            if let Some(likes) = comment.likes.as_deref() {
                left_column.push(simple_text_element("p", &format!("Likes: {likes}")));
            }
            left_column.push(simple_text_element("p", &comment.body));
        }
    }

    let mut right_column = vec![simple_text_element("h2", "Up Next")];
    if data.related_videos.is_empty() {
        right_column.push(simple_text_element(
            "p",
            "Related videos were not embedded in this response.",
        ));
    } else {
        for related in data.related_videos.iter().take(8) {
            right_column.push(build_related_video_node(related));
        }
    }

    synthetic_document(
        &data.title,
        vec![Node::Element(Element {
            tag_name: "table".to_string(),
            attributes: BTreeMap::from([
                ("width".to_string(), "100%".to_string()),
                ("border".to_string(), "0".to_string()),
                ("cellpadding".to_string(), "0".to_string()),
                ("cellspacing".to_string(), "16".to_string()),
            ]),
            children: vec![Node::Element(Element {
                tag_name: "tr".to_string(),
                attributes: BTreeMap::new(),
                children: vec![
                    Node::Element(Element {
                        tag_name: "td".to_string(),
                        attributes: BTreeMap::from([
                            ("width".to_string(), "70%".to_string()),
                            ("valign".to_string(), "top".to_string()),
                        ]),
                        children: left_column,
                    }),
                    Node::Element(Element {
                        tag_name: "td".to_string(),
                        attributes: BTreeMap::from([
                            ("width".to_string(), "30%".to_string()),
                            ("valign".to_string(), "top".to_string()),
                        ]),
                        children: right_column,
                    }),
                ],
            })],
        })],
    )
}

fn build_youtube_generic_document_from_html(html: &str, url: &Url) -> Node {
    if let Some(data) = extract_youtube_home_data_from_html(html, url) {
        return build_youtube_home_document(&data, url);
    }

    let title = extract_html_tag_text(html, "title").unwrap_or_else(|| "YouTube".to_string());
    let description = extract_meta_content_from_html(html, "name", "description")
        .or_else(|| extract_meta_content_from_html(html, "property", "og:description"))
        .unwrap_or_else(|| {
            "This YouTube page relies on a large app shell. Tobira currently renders specific watch URLs more accurately than the full home feed.".to_string()
        });

    let mut body_children = vec![
        simple_text_element("h1", &title),
        simple_text_element("p", &description),
        simple_text_element(
            "p",
            "Tip: open a specific https://www.youtube.com/watch?v=... URL for a richer video summary.",
        ),
    ];

    let watch_links = extract_html_attribute_values(html, "href=\"/watch?v=", '"', 10)
        .into_iter()
        .map(|path| format!("https://www.youtube.com{path}"))
        .collect::<Vec<_>>();
    if !watch_links.is_empty() {
        body_children.push(hr_node());
        body_children.push(simple_text_element("h2", "Watch Links"));
        body_children.push(Node::Element(Element {
            tag_name: "ul".to_string(),
            attributes: BTreeMap::new(),
            children: watch_links
                .into_iter()
                .map(|href| {
                    Node::Element(Element {
                        tag_name: "li".to_string(),
                        attributes: BTreeMap::new(),
                        children: vec![Node::Text(href)],
                    })
                })
                .collect(),
        }));
    }

    body_children.push(hr_node());
    body_children.push(simple_text_element("p", &format!("URL: {}", url)));

    synthetic_document(&title, body_children)
}

fn extract_youtube_home_data_from_html(html: &str, _url: &Url) -> Option<YouTubeHomeData> {
    let initial_data = extract_assigned_json_object(
        html,
        &[
            "var ytInitialData =",
            "window['ytInitialData'] =",
            "window[\"ytInitialData\"] =",
        ],
    )
    .and_then(|json| serde_json::from_str::<Value>(&json).ok())?;

    let title = extract_html_tag_text(html, "title")
        .or_else(|| extract_meta_content_from_html(html, "property", "og:title"))
        .unwrap_or_else(|| "YouTube".to_string());
    let section_title = initial_data
        .pointer("/header/feedTabbedHeaderRenderer/title")
        .and_then(json_text)
        .unwrap_or_else(|| title.clone());
    let search_placeholder = initial_data
        .pointer("/topbar/desktopTopbarRenderer/searchbox/fusionSearchboxRenderer/placeholderText")
        .and_then(json_text);
    let topbar_buttons = initial_data
        .pointer("/topbar/desktopTopbarRenderer/topbarButtons")
        .and_then(Value::as_array);
    let settings_label = topbar_buttons.and_then(|buttons| {
        buttons.iter().find_map(|button| {
            button.get("topbarMenuButtonRenderer").and_then(|renderer| {
                renderer
                    .get("tooltip")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| {
                        renderer
                            .pointer("/accessibility/accessibilityData/label")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
            })
        })
    });
    let login_label = topbar_buttons.and_then(|buttons| {
        buttons.iter().find_map(|button| {
            button
                .get("buttonRenderer")
                .and_then(|renderer| renderer.get("text"))
                .and_then(json_text)
        })
    });
    let feed_nudge = find_first_object_by_key(&initial_data, "feedNudgeRenderer");
    let nudge_title = feed_nudge
        .and_then(|renderer| renderer.get("title"))
        .and_then(json_text);
    let nudge_subtitle = feed_nudge
        .and_then(|renderer| renderer.get("subtitle"))
        .and_then(json_text);
    let category_chips = find_first_object_by_key(&initial_data, "feedFilterChipBarRenderer")
        .and_then(|renderer| renderer.get("contents"))
        .and_then(Value::as_array)
        .map(|chips| {
            chips
                .iter()
                .filter_map(|chip| chip.get("chipCloudChipRenderer"))
                .filter_map(|chip| chip.get("text"))
                .filter_map(json_text)
                .take(8)
                .collect()
        })
        .unwrap_or_default();
    let featured_videos = extract_youtube_home_feed_videos(&initial_data);
    let quick_links = extract_html_attribute_values(html, "href=\"/watch?v=", '"', 10)
        .into_iter()
        .map(|path| format!("https://www.youtube.com{path}"))
        .collect();

    Some(YouTubeHomeData {
        title,
        section_title,
        search_placeholder,
        settings_label,
        login_label,
        nudge_title,
        nudge_subtitle,
        category_chips,
        featured_videos,
        quick_links,
    })
}

fn extract_youtube_home_feed_videos(initial_data: &Value) -> Vec<YouTubeRelatedVideo> {
    let contents = initial_data
        .pointer("/contents/twoColumnBrowseResultsRenderer/tabs")
        .and_then(Value::as_array)
        .and_then(|tabs| {
            tabs.iter().find_map(|tab| {
                tab.get("tabRenderer")
                    .filter(|renderer| {
                        renderer
                            .get("selected")
                            .and_then(Value::as_bool)
                            .unwrap_or(false)
                    })
                    .and_then(|renderer| renderer.pointer("/content/richGridRenderer/contents"))
                    .and_then(Value::as_array)
            })
        });
    let Some(contents) = contents else {
        return Vec::new();
    };

    let mut videos = Vec::new();
    for item in contents {
        if let Some(video) = item
            .pointer("/richItemRenderer/content/videoRenderer")
            .and_then(parse_home_feed_video)
        {
            videos.push(video);
        } else {
            for renderer in collect_objects_by_key(item, "videoRenderer", 12 - videos.len()) {
                if let Some(video) = parse_home_feed_video(renderer) {
                    videos.push(video);
                    if videos.len() >= 12 {
                        break;
                    }
                }
            }
        }
        if videos.len() >= 12 {
            break;
        }
    }

    videos
}

fn parse_home_feed_video(value: &Value) -> Option<YouTubeRelatedVideo> {
    let title = value.get("title").and_then(json_text)?;
    let channel = value
        .get("ownerText")
        .and_then(json_text)
        .or_else(|| value.get("shortBylineText").and_then(json_text));
    let views = value
        .get("viewCountText")
        .and_then(json_text)
        .or_else(|| value.get("shortViewCountText").and_then(json_text));
    let published = value.get("publishedTimeText").and_then(json_text);
    let duration = value
        .pointer("/lengthText/simpleText")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| value.get("lengthText").and_then(json_text));
    let thumbnail_url = value
        .pointer("/thumbnail/thumbnails")
        .and_then(Value::as_array)
        .and_then(|items| items.last())
        .and_then(|item| item.get("url"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let url = value
        .pointer("/navigationEndpoint/commandMetadata/webCommandMetadata/url")
        .and_then(Value::as_str)
        .map(|path| format!("https://www.youtube.com{path}"))
        .or_else(|| {
            value
                .pointer("/navigationEndpoint/watchEndpoint/videoId")
                .and_then(Value::as_str)
                .map(|video_id| format!("https://www.youtube.com/watch?v={video_id}"))
        });

    Some(YouTubeRelatedVideo {
        title,
        channel,
        views,
        published,
        duration,
        url,
        thumbnail_url,
    })
}

fn build_youtube_home_document(data: &YouTubeHomeData, url: &Url) -> Node {
    let sidebar_labels =
        default_youtube_sidebar_labels(data.search_placeholder.as_deref(), &data.section_title);

    let mut topbar_actions = Vec::new();
    if let Some(settings) = data.settings_label.as_deref() {
        topbar_actions.push(simple_text_element("p", settings));
    }
    if let Some(login) = data.login_label.as_deref() {
        topbar_actions.push(simple_text_element("p", login));
    }

    let topbar = Node::Element(Element {
        tag_name: "table".to_string(),
        attributes: BTreeMap::from([
            ("width".to_string(), "100%".to_string()),
            ("border".to_string(), "0".to_string()),
            ("cellpadding".to_string(), "10".to_string()),
            ("cellspacing".to_string(), "0".to_string()),
            ("bgcolor".to_string(), "#ffffff".to_string()),
        ]),
        children: vec![Node::Element(Element {
            tag_name: "tr".to_string(),
            attributes: BTreeMap::new(),
            children: vec![
                Node::Element(Element {
                    tag_name: "td".to_string(),
                    attributes: BTreeMap::from([
                        ("width".to_string(), "18%".to_string()),
                        ("valign".to_string(), "middle".to_string()),
                    ]),
                    children: vec![simple_text_element("h2", "YouTube")],
                }),
                Node::Element(Element {
                    tag_name: "td".to_string(),
                    attributes: BTreeMap::from([
                        ("width".to_string(), "60%".to_string()),
                        ("valign".to_string(), "middle".to_string()),
                    ]),
                    children: vec![Node::Element(Element {
                        tag_name: "table".to_string(),
                        attributes: BTreeMap::from([
                            ("width".to_string(), "100%".to_string()),
                            ("border".to_string(), "0".to_string()),
                            ("cellpadding".to_string(), "8".to_string()),
                            ("cellspacing".to_string(), "0".to_string()),
                            ("bgcolor".to_string(), "#f1f1f1".to_string()),
                        ]),
                        children: vec![Node::Element(Element {
                            tag_name: "tr".to_string(),
                            attributes: BTreeMap::new(),
                            children: vec![Node::Element(Element {
                                tag_name: "td".to_string(),
                                attributes: BTreeMap::new(),
                                children: vec![simple_text_element(
                                    "p",
                                    data.search_placeholder
                                        .as_deref()
                                        .unwrap_or("Search YouTube"),
                                )],
                            })],
                        })],
                    })],
                }),
                Node::Element(Element {
                    tag_name: "td".to_string(),
                    attributes: BTreeMap::from([
                        ("width".to_string(), "22%".to_string()),
                        ("valign".to_string(), "middle".to_string()),
                        ("align".to_string(), "right".to_string()),
                    ]),
                    children: topbar_actions,
                }),
            ],
        })],
    });

    let sidebar_children = sidebar_labels
        .into_iter()
        .map(|(label, href)| link_element(href, label))
        .collect::<Vec<_>>();

    let mut main_children = vec![simple_text_element("h1", &data.section_title)];
    if !data.category_chips.is_empty() {
        main_children.push(build_youtube_chip_bar(&data.category_chips));
    }

    if !data.featured_videos.is_empty() {
        main_children.push(build_youtube_feed_grid(&data.featured_videos));
    } else if data.nudge_title.is_some() || data.nudge_subtitle.is_some() {
        main_children.push(build_youtube_nudge_card(data));
    }

    if !data.quick_links.is_empty() {
        main_children.push(hr_node());
        main_children.push(simple_text_element("h2", "Quick Links"));
        for href in data.quick_links.iter().take(6) {
            main_children.push(link_element(href, href));
        }
    }

    main_children.push(hr_node());
    main_children.push(simple_text_element("p", &format!("URL: {url}")));

    synthetic_document(
        &data.title,
        vec![
            topbar,
            hr_node(),
            Node::Element(Element {
                tag_name: "table".to_string(),
                attributes: BTreeMap::from([
                    ("width".to_string(), "100%".to_string()),
                    ("border".to_string(), "0".to_string()),
                    ("cellpadding".to_string(), "0".to_string()),
                    ("cellspacing".to_string(), "20".to_string()),
                ]),
                children: vec![Node::Element(Element {
                    tag_name: "tr".to_string(),
                    attributes: BTreeMap::new(),
                    children: vec![
                        Node::Element(Element {
                            tag_name: "td".to_string(),
                            attributes: BTreeMap::from([
                                ("width".to_string(), "18%".to_string()),
                                ("valign".to_string(), "top".to_string()),
                                ("bgcolor".to_string(), "#f8f8f8".to_string()),
                            ]),
                            children: sidebar_children,
                        }),
                        Node::Element(Element {
                            tag_name: "td".to_string(),
                            attributes: BTreeMap::from([
                                ("width".to_string(), "82%".to_string()),
                                ("valign".to_string(), "top".to_string()),
                            ]),
                            children: main_children,
                        }),
                    ],
                })],
            }),
        ],
    )
}

fn build_youtube_feed_grid(videos: &[YouTubeRelatedVideo]) -> Node {
    let mut rows = Vec::new();
    for chunk in videos.chunks(3) {
        let mut cells = Vec::new();
        for video in chunk {
            cells.push(Node::Element(Element {
                tag_name: "td".to_string(),
                attributes: BTreeMap::from([
                    ("width".to_string(), "33%".to_string()),
                    ("valign".to_string(), "top".to_string()),
                ]),
                children: vec![build_youtube_feed_card(video)],
            }));
        }
        rows.push(Node::Element(Element {
            tag_name: "tr".to_string(),
            attributes: BTreeMap::new(),
            children: cells,
        }));
    }

    Node::Element(Element {
        tag_name: "table".to_string(),
        attributes: BTreeMap::from([
            ("width".to_string(), "100%".to_string()),
            ("border".to_string(), "0".to_string()),
            ("cellpadding".to_string(), "8".to_string()),
            ("cellspacing".to_string(), "8".to_string()),
        ]),
        children: rows,
    })
}

fn build_youtube_feed_card(video: &YouTubeRelatedVideo) -> Node {
    let mut children = Vec::new();
    if let Some(thumbnail_url) = &video.thumbnail_url {
        children.push(Node::Element(Element {
            tag_name: "img".to_string(),
            attributes: BTreeMap::from([
                ("src".to_string(), thumbnail_url.clone()),
                ("data-scratch-src".to_string(), thumbnail_url.clone()),
                ("width".to_string(), "320".to_string()),
                ("alt".to_string(), video.title.clone()),
            ]),
            children: Vec::new(),
        }));
    }
    children.push(simple_text_element("h3", &video.title));
    push_detail(&mut children, "Channel", video.channel.as_deref());
    push_detail(&mut children, "Views", video.views.as_deref());
    push_detail(&mut children, "Published", video.published.as_deref());
    push_detail(&mut children, "Length", video.duration.as_deref());

    let card = Node::Element(Element {
        tag_name: "table".to_string(),
        attributes: BTreeMap::from([
            ("width".to_string(), "100%".to_string()),
            ("border".to_string(), "0".to_string()),
            ("cellpadding".to_string(), "6".to_string()),
            ("cellspacing".to_string(), "0".to_string()),
            ("bgcolor".to_string(), "#ffffff".to_string()),
        ]),
        children: vec![Node::Element(Element {
            tag_name: "tr".to_string(),
            attributes: BTreeMap::new(),
            children: vec![Node::Element(Element {
                tag_name: "td".to_string(),
                attributes: BTreeMap::from([("valign".to_string(), "top".to_string())]),
                children,
            })],
        })],
    });

    if let Some(url) = &video.url {
        Node::Element(Element {
            tag_name: "a".to_string(),
            attributes: BTreeMap::from([
                ("href".to_string(), url.clone()),
                ("style".to_string(), "display: block;".to_string()),
            ]),
            children: vec![card, hr_node()],
        })
    } else {
        Node::Element(Element {
            tag_name: "div".to_string(),
            attributes: BTreeMap::new(),
            children: vec![card, hr_node()],
        })
    }
}

fn build_youtube_nudge_card(data: &YouTubeHomeData) -> Node {
    let mut content = Vec::new();
    if let Some(title) = data.nudge_title.as_deref() {
        content.push(simple_text_element("h2", title));
    }
    if let Some(subtitle) = data.nudge_subtitle.as_deref() {
        content.push(simple_text_element("p", subtitle));
    }
    content.push(link_element(
        "https://www.youtube.com/feed/trending",
        "→ トレンド動画を見る (View Trending)",
    ));

    Node::Element(Element {
        tag_name: "table".to_string(),
        attributes: BTreeMap::from([
            ("width".to_string(), "100%".to_string()),
            ("border".to_string(), "0".to_string()),
            ("cellpadding".to_string(), "0".to_string()),
            ("cellspacing".to_string(), "0".to_string()),
        ]),
        children: vec![Node::Element(Element {
            tag_name: "tr".to_string(),
            attributes: BTreeMap::new(),
            children: vec![
                Node::Element(Element {
                    tag_name: "td".to_string(),
                    attributes: BTreeMap::from([("width".to_string(), "20%".to_string())]),
                    children: Vec::new(),
                }),
                Node::Element(Element {
                    tag_name: "td".to_string(),
                    attributes: BTreeMap::from([
                        ("width".to_string(), "60%".to_string()),
                        ("valign".to_string(), "top".to_string()),
                        ("bgcolor".to_string(), "#f9f9f9".to_string()),
                    ]),
                    children: content,
                }),
                Node::Element(Element {
                    tag_name: "td".to_string(),
                    attributes: BTreeMap::from([("width".to_string(), "20%".to_string())]),
                    children: Vec::new(),
                }),
            ],
        })],
    })
}

fn build_youtube_chip_bar(chips: &[String]) -> Node {
    let children = chips
        .iter()
        .take(6)
        .map(|chip| {
            Node::Element(Element {
                tag_name: "td".to_string(),
                attributes: BTreeMap::from([
                    ("bgcolor".to_string(), "#f1f1f1".to_string()),
                    ("align".to_string(), "center".to_string()),
                    ("valign".to_string(), "middle".to_string()),
                ]),
                children: vec![simple_text_element("p", chip)],
            })
        })
        .collect();

    Node::Element(Element {
        tag_name: "table".to_string(),
        attributes: BTreeMap::from([
            ("width".to_string(), "100%".to_string()),
            ("border".to_string(), "0".to_string()),
            ("cellpadding".to_string(), "6".to_string()),
            ("cellspacing".to_string(), "8".to_string()),
        ]),
        children: vec![Node::Element(Element {
            tag_name: "tr".to_string(),
            attributes: BTreeMap::new(),
            children,
        })],
    })
}

/*
fn default_youtube_sidebar_labels<'a>(
    search_placeholder: Option<&'a str>,
    section_title: &'a str,
) -> Vec<&'a str> {
    if looks_like_japanese(search_placeholder.unwrap_or(section_title)) {
        vec![
            "ホーム",
            "ショート",
            "登録チャンネル",
            "履歴",
            "再生リスト",
            "後で見る",
            "高く評価した動画",
        ]
    } else {
        vec![
            "Home",
            "Shorts",
            "Subscriptions",
            "History",
            "Playlists",
            "Watch later",
            "Liked videos",
        ]
    }
}

*/

fn default_youtube_sidebar_labels<'a>(
    search_placeholder: Option<&'a str>,
    section_title: &'a str,
) -> Vec<(&'a str, &'a str)> {
    if looks_like_japanese(search_placeholder.unwrap_or(section_title)) {
        vec![
            ("ホーム", "https://www.youtube.com/"),
            ("ショート", "https://www.youtube.com/shorts"),
            (
                "登録チャンネル",
                "https://www.youtube.com/feed/subscriptions",
            ),
            ("トレンド", "https://www.youtube.com/feed/trending"),
            ("履歴", "https://www.youtube.com/feed/history"),
            ("後で見る", "https://www.youtube.com/playlist?list=WL"),
            (
                "高く評価した動画",
                "https://www.youtube.com/playlist?list=LL",
            ),
        ]
    } else {
        vec![
            ("Home", "https://www.youtube.com/"),
            ("Shorts", "https://www.youtube.com/shorts"),
            (
                "Subscriptions",
                "https://www.youtube.com/feed/subscriptions",
            ),
            ("Trending", "https://www.youtube.com/feed/trending"),
            ("History", "https://www.youtube.com/feed/history"),
            ("Watch later", "https://www.youtube.com/playlist?list=WL"),
            ("Liked videos", "https://www.youtube.com/playlist?list=LL"),
        ]
    }
}

fn build_google_document_from_html(html: &str, url: &Url) -> Node {
    let title = extract_html_tag_text(html, "title").unwrap_or_else(|| "Google".to_string());
    let description = extract_meta_content_from_html(html, "name", "description")
        .or_else(|| extract_meta_content_from_html(html, "property", "og:description"))
        .unwrap_or_else(|| {
            "This Google page uses a large interactive shell. Tobira keeps it lightweight instead of trying to execute the full app.".to_string()
        });

    synthetic_document(
        &title,
        vec![
            simple_text_element("h1", &title),
            simple_text_element("p", &description),
            simple_text_element(
                "p",
                "Tip: direct content URLs usually render better than large search or portal shells.",
            ),
            hr_node(),
            simple_text_element("p", &format!("URL: {}", url)),
        ],
    )
}

fn build_related_video_node(video: &YouTubeRelatedVideo) -> Node {
    let mut detail_children = Vec::new();
    detail_children.push(simple_text_element("h3", &video.title));
    push_detail(&mut detail_children, "Channel", video.channel.as_deref());
    push_detail(&mut detail_children, "Views", video.views.as_deref());
    push_detail(
        &mut detail_children,
        "Published",
        video.published.as_deref(),
    );
    push_detail(&mut detail_children, "Length", video.duration.as_deref());

    let mut row_children = Vec::new();
    if let Some(thumbnail_url) = &video.thumbnail_url {
        row_children.push(Node::Element(Element {
            tag_name: "td".to_string(),
            attributes: BTreeMap::from([
                ("width".to_string(), "180".to_string()),
                ("valign".to_string(), "top".to_string()),
            ]),
            children: vec![Node::Element(Element {
                tag_name: "img".to_string(),
                attributes: BTreeMap::from([
                    ("src".to_string(), thumbnail_url.clone()),
                    ("data-scratch-src".to_string(), thumbnail_url.clone()),
                    ("width".to_string(), "168".to_string()),
                    ("alt".to_string(), video.title.clone()),
                ]),
                children: Vec::new(),
            })],
        }));
    }
    row_children.push(Node::Element(Element {
        tag_name: "td".to_string(),
        attributes: BTreeMap::from([("valign".to_string(), "top".to_string())]),
        children: detail_children,
    }));

    let card = Node::Element(Element {
        tag_name: "table".to_string(),
        attributes: BTreeMap::from([
            ("width".to_string(), "100%".to_string()),
            ("border".to_string(), "0".to_string()),
            ("cellpadding".to_string(), "0".to_string()),
            ("cellspacing".to_string(), "8".to_string()),
        ]),
        children: vec![Node::Element(Element {
            tag_name: "tr".to_string(),
            attributes: BTreeMap::new(),
            children: row_children,
        })],
    });

    if let Some(url) = &video.url {
        Node::Element(Element {
            tag_name: "a".to_string(),
            attributes: BTreeMap::from([
                ("href".to_string(), url.clone()),
                ("style".to_string(), "display: block;".to_string()),
            ]),
            children: vec![card, hr_node()],
        })
    } else {
        Node::Element(Element {
            tag_name: "div".to_string(),
            attributes: BTreeMap::new(),
            children: vec![card, hr_node()],
        })
    }
}

fn simple_text_element(tag_name: &str, text: &str) -> Node {
    Node::Element(Element {
        tag_name: tag_name.to_string(),
        attributes: BTreeMap::new(),
        children: vec![Node::Text(text.to_string())],
    })
}

fn link_element(href: &str, text: &str) -> Node {
    Node::Element(Element {
        tag_name: "p".to_string(),
        attributes: BTreeMap::new(),
        children: vec![Node::Element(Element {
            tag_name: "a".to_string(),
            attributes: BTreeMap::from([("href".to_string(), href.to_string())]),
            children: vec![Node::Text(text.to_string())],
        })],
    })
}

fn push_detail(output: &mut Vec<Node>, label: &str, value: Option<&str>) {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    output.push(simple_text_element("p", &format!("{label}: {value}")));
}

fn extract_assigned_json_object(html: &str, markers: &[&str]) -> Option<String> {
    for marker in markers {
        let Some(marker_index) = html.find(marker) else {
            continue;
        };
        let after_marker = marker_index + marker.len();
        let Some(open_offset) = html[after_marker..].find('{') else {
            continue;
        };
        let open_index = after_marker + open_offset;
        if let Some(close_index) = find_matching_json_brace(html, open_index) {
            return Some(html[open_index..=close_index].to_string());
        }
    }

    None
}

fn extract_html_tag_text(html: &str, tag_name: &str) -> Option<String> {
    let open_tag = format!("<{tag_name}>");
    let close_tag = format!("</{tag_name}>");
    let start = html.find(&open_tag)?;
    let content_start = start + open_tag.len();
    let end_offset = html[content_start..].find(&close_tag)?;
    let text = &html[content_start..content_start + end_offset];
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn extract_meta_content_from_html(html: &str, attribute: &str, expected: &str) -> Option<String> {
    let mut search_start = 0;
    let attribute_marker = format!("{attribute}=\"{expected}\"");
    while let Some(meta_offset) = html[search_start..].find("<meta") {
        let tag_start = search_start + meta_offset;
        let tag_end = tag_start + html[tag_start..].find('>')?;
        let tag = &html[tag_start..=tag_end];
        if tag.contains(&attribute_marker)
            && let Some(content_offset) = tag.find("content=\"")
        {
            let value_start = content_offset + "content=\"".len();
            let value_end = tag[value_start..].find('"')?;
            let content = &tag[value_start..value_start + value_end];
            if !content.trim().is_empty() {
                return Some(content.to_string());
            }
        }
        search_start = tag_end + 1;
    }

    None
}

fn extract_link_href_from_html(html: &str, attribute: &str, expected: &str) -> Option<String> {
    let mut search_start = 0;
    let attribute_marker = format!("{attribute}=\"{expected}\"");
    while let Some(link_offset) = html[search_start..].find("<link") {
        let tag_start = search_start + link_offset;
        let tag_end = tag_start + html[tag_start..].find('>')?;
        let tag = &html[tag_start..=tag_end];
        if tag.contains(&attribute_marker)
            && let Some(href_offset) = tag.find("href=\"")
        {
            let value_start = href_offset + "href=\"".len();
            let value_end = tag[value_start..].find('"')?;
            let href = &tag[value_start..value_start + value_end];
            if !href.trim().is_empty() {
                return Some(href.to_string());
            }
        }
        search_start = tag_end + 1;
    }

    None
}

fn extract_html_attribute_values(
    html: &str,
    marker: &str,
    closing_quote: char,
    limit: usize,
) -> Vec<String> {
    let mut values = Vec::new();
    let mut search_start = 0;

    while values.len() < limit {
        let Some(found) = html[search_start..].find(marker) else {
            break;
        };
        let value_start = search_start + found + "href=\"".len();
        let Some(end_offset) = html[value_start..].find(closing_quote) else {
            break;
        };
        let value = &html[value_start..value_start + end_offset];
        if !values.iter().any(|known| known == value) {
            values.push(value.to_string());
        }
        search_start = value_start + end_offset + 1;
    }

    values
}

fn find_matching_json_brace(input: &str, open_index: usize) -> Option<usize> {
    let bytes = input.as_bytes();
    let mut depth = 0_u32;
    let mut in_string = false;
    let mut escaped = false;

    for (index, byte) in bytes.iter().copied().enumerate().skip(open_index) {
        if in_string {
            if escaped {
                escaped = false;
                continue;
            }
            match byte {
                b'\\' => escaped = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }

        match byte {
            b'"' => in_string = true,
            b'{' => depth = depth.saturating_add(1),
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }

    None
}

fn extract_thumbnail_url(value: &Value) -> Option<String> {
    value
        .pointer("/videoDetails/thumbnail/thumbnails")
        .and_then(Value::as_array)
        .and_then(|items| items.iter().rev().find_map(|item| item.get("url")))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn extract_owner_renderer(initial_data: &Value) -> Option<&Value> {
    initial_data
        .pointer("/contents/twoColumnWatchNextResults/results/results/contents")
        .and_then(Value::as_array)
        .and_then(|items| {
            items.iter().find_map(|item| {
                item.get("videoSecondaryInfoRenderer")
                    .and_then(|renderer| renderer.pointer("/owner/videoOwnerRenderer"))
            })
        })
}

fn extract_comments_panel_header(initial_data: &Value) -> Option<&Value> {
    initial_data
        .get("engagementPanels")
        .and_then(Value::as_array)
        .and_then(|panels| {
            panels.iter().find_map(|panel| {
                let section = panel.get("engagementPanelSectionListRenderer")?;
                let identifier = section.get("panelIdentifier")?.as_str()?;
                (identifier == "engagement-panel-comments-section")
                    .then(|| section.get("header"))
                    .flatten()
                    .and_then(|header| header.get("engagementPanelTitleHeaderRenderer"))
            })
        })
}

fn extract_related_videos(initial_data: &Value) -> Vec<YouTubeRelatedVideo> {
    let contents = initial_data
        .pointer("/contents/twoColumnWatchNextResults/secondaryResults/secondaryResults/results")
        .and_then(Value::as_array)
        .and_then(|results| {
            results.iter().find_map(|entry| {
                entry
                    .get("itemSectionRenderer")
                    .and_then(|renderer| renderer.get("contents"))
                    .and_then(Value::as_array)
            })
        });

    let Some(contents) = contents else {
        return Vec::new();
    };

    contents
        .iter()
        .filter_map(|entry| {
            entry
                .get("lockupViewModel")
                .and_then(parse_related_lockup_video)
                .or_else(|| {
                    entry
                        .get("compactVideoRenderer")
                        .and_then(parse_related_compact_video)
                })
        })
        .collect()
}

fn parse_related_lockup_video(value: &Value) -> Option<YouTubeRelatedVideo> {
    let title = value.pointer("/metadata/lockupMetadataViewModel/title/content")?;
    let title = title.as_str()?.to_string();
    let rows = value
        .pointer("/metadata/lockupMetadataViewModel/metadata/contentMetadataViewModel/metadataRows")
        .and_then(Value::as_array);
    let channel = rows
        .and_then(|items| items.first())
        .and_then(|row| row.pointer("/metadataParts/0/text/content"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let views = rows
        .and_then(|items| items.get(1))
        .and_then(|row| row.pointer("/metadataParts/0/text/content"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let published = rows
        .and_then(|items| items.get(1))
        .and_then(|row| row.pointer("/metadataParts/1/text/content"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let duration = value
        .pointer("/contentImage/thumbnailViewModel/overlays")
        .and_then(Value::as_array)
        .and_then(|overlays| {
            overlays.iter().find_map(|overlay| {
                overlay
                    .pointer(
                        "/thumbnailBottomOverlayViewModel/badges/0/thumbnailBadgeViewModel/text",
                    )
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
        });
    let thumbnail_url = value
        .pointer("/contentImage/thumbnailViewModel/image/sources")
        .and_then(Value::as_array)
        .and_then(|sources| sources.last())
        .and_then(|source| source.get("url"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let url = value
        .pointer("/rendererContext/commandContext/onTap/innertubeCommand/commandMetadata/webCommandMetadata/url")
        .and_then(Value::as_str)
        .map(|path| format!("https://www.youtube.com{path}"));

    Some(YouTubeRelatedVideo {
        title,
        channel,
        views,
        published,
        duration,
        url,
        thumbnail_url,
    })
}

fn parse_related_compact_video(value: &Value) -> Option<YouTubeRelatedVideo> {
    let title = json_text(value.get("title")?)?;
    let channel = json_text(value.get("shortBylineText")?);
    let views = json_text(value.get("viewCountText")?);
    let published = json_text(value.get("publishedTimeText")?);
    let duration = value
        .pointer("/lengthText/simpleText")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| json_text(value.get("lengthText")?));
    let thumbnail_url = value
        .pointer("/thumbnail/thumbnails")
        .and_then(Value::as_array)
        .and_then(|sources| sources.last())
        .and_then(|source| source.get("url"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let url = value
        .pointer("/navigationEndpoint/commandMetadata/webCommandMetadata/url")
        .and_then(Value::as_str)
        .map(|path| format!("https://www.youtube.com{path}"));

    Some(YouTubeRelatedVideo {
        title,
        channel,
        views,
        published,
        duration,
        url,
        thumbnail_url,
    })
}

fn extract_first_ld_json_video_object(html: &str) -> Option<Value> {
    let mut search_start = 0;
    while let Some(script_offset) = html[search_start..].find("<script") {
        let tag_start = search_start + script_offset;
        let tag_end = tag_start + html[tag_start..].find('>')?;
        let open_tag = &html[tag_start..=tag_end];
        if open_tag.contains("application/ld+json") {
            let close_offset = html[tag_end + 1..].find("</script>")?;
            let json_text = &html[tag_end + 1..tag_end + 1 + close_offset];
            if let Ok(value) = serde_json::from_str::<Value>(json_text)
                && value
                    .get("@type")
                    .and_then(Value::as_str)
                    .map(|kind| kind == "VideoObject")
                    .unwrap_or(false)
            {
                return Some(value);
            }
        }
        search_start = tag_end + 1;
    }

    None
}

fn extract_featured_comments_from_ld_json(value: &Value) -> Vec<YouTubeCommentPreview> {
    let comments = value.get("comment");
    let Some(items) = comments else {
        return Vec::new();
    };

    let array = match items {
        Value::Array(array) => array,
        single => {
            return extract_featured_comments_from_ld_json(&Value::Array(vec![single.clone()]));
        }
    };

    array
        .iter()
        .filter_map(|item| {
            let author = item
                .pointer("/author/name")
                .and_then(Value::as_str)
                .or_else(|| {
                    item.pointer("/author/alternateName")
                        .and_then(Value::as_str)
                })?;
            let body = item.get("text").and_then(Value::as_str)?;
            Some(YouTubeCommentPreview {
                author: author.to_string(),
                body: body.to_string(),
                likes: item.get("upvoteCount").and_then(|value| {
                    value
                        .as_i64()
                        .map(|number| number.to_string())
                        .or_else(|| value.as_str().map(str::to_string))
                }),
            })
        })
        .collect()
}

fn find_first_object_by_key<'a>(root: &'a Value, key: &str) -> Option<&'a Value> {
    let mut stack = vec![root];
    while let Some(value) = stack.pop() {
        match value {
            Value::Object(map) => {
                if let Some(found) = map.get(key) {
                    return Some(found);
                }
                for child in map.values() {
                    stack.push(child);
                }
            }
            Value::Array(items) => {
                for child in items.iter().rev() {
                    stack.push(child);
                }
            }
            _ => {}
        }
    }

    None
}

fn collect_objects_by_key<'a>(root: &'a Value, key: &str, limit: usize) -> Vec<&'a Value> {
    if limit == 0 {
        return Vec::new();
    }

    let mut results = Vec::new();
    let mut stack = vec![root];
    while let Some(value) = stack.pop() {
        if results.len() >= limit {
            break;
        }
        match value {
            Value::Object(map) => {
                if let Some(found) = map.get(key) {
                    results.push(found);
                    if results.len() >= limit {
                        break;
                    }
                }
                for child in map.values() {
                    stack.push(child);
                }
            }
            Value::Array(items) => {
                for child in items.iter().rev() {
                    stack.push(child);
                }
            }
            _ => {}
        }
    }

    results
}

fn json_text(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return Some(text.to_string());
    }
    if let Some(text) = value.get("simpleText").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    if let Some(text) = value.get("content").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    value
        .get("runs")
        .and_then(Value::as_array)
        .map(|runs| {
            runs.iter()
                .filter_map(|run| run.get("text").and_then(Value::as_str))
                .collect::<String>()
        })
        .filter(|text| !text.trim().is_empty())
}

fn find_meta_content(node: &Node, attribute: &str, expected: &str) -> Option<String> {
    match node {
        Node::Text(_) => None,
        Node::Element(element) => {
            if element.tag_name == "meta"
                && element
                    .attribute(attribute)
                    .map(|value| value.eq_ignore_ascii_case(expected))
                    .unwrap_or(false)
                && let Some(content) = element.attribute("content")
                && !content.trim().is_empty()
            {
                return Some(content.to_string());
            }

            for child in &element.children {
                if let Some(found) = find_meta_content(child, attribute, expected) {
                    return Some(found);
                }
            }

            None
        }
    }
}

fn find_link_href(node: &Node, attribute: &str, expected: &str) -> Option<String> {
    match node {
        Node::Text(_) => None,
        Node::Element(element) => {
            if element.tag_name == "link"
                && element
                    .attribute(attribute)
                    .map(|value| value.eq_ignore_ascii_case(expected))
                    .unwrap_or(false)
                && let Some(href) = element.attribute("href")
                && !href.trim().is_empty()
            {
                return Some(href.to_string());
            }

            for child in &element.children {
                if let Some(found) = find_link_href(child, attribute, expected) {
                    return Some(found);
                }
            }

            None
        }
    }
}

fn find_link_content(node: &Node, attribute: &str, expected: &str) -> Option<String> {
    match node {
        Node::Text(_) => None,
        Node::Element(element) => {
            if element.tag_name == "link"
                && element
                    .attribute(attribute)
                    .map(|value| value.eq_ignore_ascii_case(expected))
                    .unwrap_or(false)
                && let Some(content) = element.attribute("content")
                && !content.trim().is_empty()
            {
                return Some(content.to_string());
            }

            for child in &element.children {
                if let Some(found) = find_link_content(child, attribute, expected) {
                    return Some(found);
                }
            }

            None
        }
    }
}

fn find_interaction_count(node: &Node, expected_type_fragment: &str) -> Option<String> {
    match node {
        Node::Text(_) => None,
        Node::Element(element) => {
            if element.tag_name == "div"
                && element
                    .attribute("itemprop")
                    .map(|value| value.eq_ignore_ascii_case("interactionStatistic"))
                    .unwrap_or(false)
            {
                let mut interaction_type = None;
                let mut user_count = None;
                for child in &element.children {
                    if let Node::Element(meta) = child
                        && meta.tag_name == "meta"
                    {
                        match meta.attribute("itemprop") {
                            Some("interactionType") => {
                                interaction_type = meta.attribute("content");
                            }
                            Some("userInteractionCount") => {
                                user_count = meta.attribute("content");
                            }
                            _ => {}
                        }
                    }
                }

                if interaction_type
                    .map(|value| value.contains(expected_type_fragment))
                    .unwrap_or(false)
                    && let Some(count) = user_count
                {
                    return Some(format_numeric_count(count));
                }
            }

            for child in &element.children {
                if let Some(found) = find_interaction_count(child, expected_type_fragment) {
                    return Some(found);
                }
            }

            None
        }
    }
}

fn format_numeric_count(raw: &str) -> String {
    let digits: String = raw
        .chars()
        .filter(|character| character.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        return raw.to_string();
    }

    let mut grouped = String::new();
    for (index, character) in digits.chars().rev().enumerate() {
        if index != 0 && index % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(character);
    }

    grouped.chars().rev().collect()
}

fn format_duration_seconds(raw: &str) -> Option<String> {
    let total_seconds = raw.parse::<u64>().ok()?;
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;

    Some(if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    })
}

fn looks_like_japanese(text: &str) -> bool {
    text.chars().any(|character| {
        ('\u{3040}'..='\u{30ff}').contains(&character)
            || ('\u{4e00}'..='\u{9faf}').contains(&character)
    })
}

fn parse_iso8601_duration(raw: String) -> Option<String> {
    if !raw.starts_with("PT") {
        return None;
    }

    let mut remaining = raw.trim_start_matches("PT");
    let mut hours = 0_u64;
    let mut minutes = 0_u64;
    let mut seconds = 0_u64;

    if let Some((value, rest)) = remaining.split_once('H') {
        hours = value.parse().ok()?;
        remaining = rest;
    }
    if let Some((value, rest)) = remaining.split_once('M') {
        minutes = value.parse().ok()?;
        remaining = rest;
    }
    if let Some((value, _)) = remaining.split_once('S') {
        seconds = value.parse().ok()?;
    }

    Some(if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    })
}

fn document_title(node: &Node) -> Option<String> {
    first_text_by_tag(node, "title")
}

fn first_heading(node: &Node) -> Option<String> {
    match node {
        Node::Text(_) => None,
        Node::Element(element) => {
            if matches!(
                element.tag_name.as_str(),
                "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
            ) {
                let text = collect_text(node);
                if !text.trim().is_empty() {
                    return Some(text.trim().to_string());
                }
            }

            for child in &element.children {
                if let Some(found) = first_heading(child) {
                    return Some(found);
                }
            }

            None
        }
    }
}

fn first_text_by_tag(node: &Node, tag_name: &str) -> Option<String> {
    match node {
        Node::Text(_) => None,
        Node::Element(element) => {
            if element.tag_name == tag_name {
                let text = collect_text(node);
                if !text.trim().is_empty() {
                    return Some(text.trim().to_string());
                }
            }

            for child in &element.children {
                if let Some(found) = first_text_by_tag(child, tag_name) {
                    return Some(found);
                }
            }

            None
        }
    }
}

fn collect_text(node: &Node) -> String {
    let mut text = String::new();
    collect_text_into(node, &mut text);
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn collect_text_into(node: &Node, output: &mut String) {
    match node {
        Node::Text(text) => {
            output.push_str(text);
            output.push(' ');
        }
        Node::Element(element) => {
            if matches!(element.tag_name.as_str(), "script" | "style" | "noscript") {
                return;
            }

            for child in &element.children {
                collect_text_into(child, output);
            }
        }
    }
}

fn collect_raw_text(node: &Node) -> String {
    let mut text = String::new();
    collect_raw_text_into(node, &mut text);
    text
}

fn collect_raw_text_into(node: &Node, output: &mut String) {
    match node {
        Node::Text(text) => output.push_str(text),
        Node::Element(element) => {
            for child in &element.children {
                collect_raw_text_into(child, output);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;
    use std::fmt::Write as _;
    use std::time::{Duration, Instant};

    use super::{
        BrowserPage, build_site_specific_document, build_youtube_generic_document_from_html,
        collect_frame_specs, collect_stylesheet, document_has_meaningful_body, document_title,
        extract_body_children, rebuild_page_from_html, should_follow_script_navigation,
        synthetic_document, annotate_node_ids,
    };
    use crate::css::{InteractiveState, StyledNode, build_styled_tree};
    use crate::html::{Node, parse_document};
    use crate::js::start_document_script_session;
    use crate::url::Url;

    const HEAVY_CLASS_POOL: &[&str] = &[
        "page",
        "shell",
        "masthead",
        "brand",
        "nav",
        "nav-item",
        "nav-link",
        "content",
        "feed",
        "story",
        "story-card",
        "card",
        "card-title",
        "card-body",
        "media",
        "thumb",
        "avatar",
        "meta",
        "tag",
        "badge",
        "btn",
        "btn-primary",
        "btn-ghost",
        "kicker",
        "summary",
        "excerpt",
        "list",
        "list-row",
        "list-item",
        "toolbar",
        "toolbar-item",
        "panel",
        "panel-header",
        "panel-body",
        "footer",
        "footer-link",
        "timestamp",
        "byline",
        "hero",
        "hero-copy",
        "figure",
        "caption",
        "topic",
    ];

    fn pick<'a>(items: &'a [&'a str], idx: usize) -> &'a str {
        items[idx % items.len()]
    }

    fn append_classes(out: &mut String, idx: usize) {
        let c1 = pick(HEAVY_CLASS_POOL, idx * 7 + 3);
        let c2 = pick(HEAVY_CLASS_POOL, idx * 11 + 5);
        let c3 = pick(HEAVY_CLASS_POOL, idx * 13 + 17);
        out.push_str(" class=\"");
        out.push_str(c1);
        if c2 != c1 {
            out.push(' ');
            out.push_str(c2);
        }
        if idx % 3 == 0 && c3 != c1 && c3 != c2 {
            out.push(' ');
            out.push_str(c3);
        }
        out.push('"');
    }

    fn make_heavy_html(n: usize) -> String {
        let block_count = ((n + 19) / 20).max(1);
        let mut html = String::with_capacity(block_count.saturating_mul(1600));
        html.push_str("<html><head><title>Heavy DOM</title><style>");
        html.push_str(
            r###"
            html, body { margin: 0; padding: 0; }
            body { font-family: sans-serif; background: #fafafa; }
            .page { color: #1c1c1c; }
            .shell { display: block; }
            .masthead { border-bottom: 1px solid #ddd; }
            .brand { font-weight: 700; }
            .nav { display: flex; gap: 12px; }
            .nav .nav-item { display: inline-flex; }
            .nav-item + .nav-item { margin-left: 6px; }
            .nav-link { text-decoration: none; color: #2156d1; }
            .content { padding: 16px; }
            .feed { display: grid; gap: 16px; }
            .story { contain: content; }
            .story:nth-child(odd) { background: #fff; }
            .story:nth-child(even) { background: #f4f7fb; }
            .story-card { border: 1px solid #e2e5ea; border-radius: 10px; overflow: hidden; }
            .story-card > .card-title { margin: 0; }
            .card { display: block; }
            .card > .card-body { padding: 12px 14px; }
            .card .meta { font-size: 12px; color: #667; }
            .card .badge { display: inline-block; padding: 1px 6px; border-radius: 999px; }
            .badge[data-state="active"] { background: #dff4e7; }
            .badge[data-state="idle"] { background: #f0f0f0; }
            .btn { display: inline-flex; align-items: center; }
            .btn.btn-primary { background: #2156d1; color: #fff; }
            .btn.btn-ghost { background: transparent; color: #2156d1; }
            .btn + .btn { margin-left: 8px; }
            .toolbar { display: flex; justify-content: space-between; }
            .toolbar-item:first-child { margin-left: 0; }
            .toolbar-item:last-child { margin-right: 0; }
            .panel > .panel-header { font-size: 13px; text-transform: uppercase; }
            .panel > .panel-body { padding: 10px 12px; }
            .list { list-style: none; padding: 0; margin: 0; }
            .list-row { display: flex; align-items: center; }
            .list-row + .list-row { border-top: 1px solid #eceef3; }
            .list-row:first-child { border-top: 0; }
            .list-row:last-child { border-bottom: 0; }
            .list-item { line-height: 1.45; }
            .list-item:not(.muted) { color: #1f2430; }
            .hero { padding: 20px 16px; }
            .hero-copy > p { max-width: 62ch; }
            .figure img { display: block; width: 100%; }
            .figure figcaption { font-size: 12px; color: #667; }
            .avatar { width: 32px; height: 32px; border-radius: 50%; }
            .thumb { aspect-ratio: 16 / 9; object-fit: cover; }
            .excerpt p { margin: 0 0 8px; }
            .excerpt p:first-child { font-weight: 500; }
            .excerpt p:last-child { opacity: 0.88; }
            .timestamp[data-kind="primary"] { color: #506070; }
            .timestamp[data-kind="secondary"] { color: #708090; }
            .byline { font-size: 12px; letter-spacing: 0.1px; }
            .tag { display: inline-block; }
            .tag + .tag { margin-left: 6px; }
            .footer { border-top: 1px solid #e7e7e7; padding-top: 8px; }
            .footer-link[href^="/"] { color: #2457cf; }
            .footer-link[aria-hidden="true"] { color: #9aa; }
            .kicker { text-transform: uppercase; }
            .summary { line-height: 1.5; }
            .summary:not(.muted) { color: #28323f; }
            .caption { font-size: 12px; }
            .topic { font-style: italic; }
            .panel[data-state="active"] { box-shadow: 0 0 0 1px rgba(33, 86, 209, 0.2); }
            .panel[data-state="idle"] { box-shadow: 0 0 0 1px rgba(0, 0, 0, 0.04); }
            section > article > .card { margin-bottom: 18px; }
            article > .story-card + .story-card { margin-top: 12px; }
            header .brand + .timestamp { margin-left: 10px; }
            nav .nav-item:first-child .nav-link { font-weight: 600; }
            nav .nav-item:last-child .nav-link { opacity: 0.8; }
            .feed > .story:nth-child(3n) .badge { letter-spacing: 0.03em; }
            .feed > .story:nth-child(4n) .card-title { text-decoration: underline; }
            .feed > .story:nth-child(5n) .summary { max-width: 54ch; }
            .story[data-kind="primary"] .card-title { color: #111827; }
            .story[data-kind="secondary"] .card-title { color: #26364d; }
            .story[data-state="active"] .card-body { background: #fbfdff; }
            .story[data-state="idle"] .card-body { background: #ffffff; }
            [data-state="active"] .btn-primary:not(:disabled) { cursor: pointer; }
            [data-state="idle"] .btn-ghost { opacity: 0.9; }
            .card[id^="card-1"] { outline: 0; }
            .card[id^="card-2"] { border-left: 3px solid #e6efff; }
            .card[id^="card-3"] { border-left: 3px solid #eaf5ea; }
            .card[id^="card-4"] { border-left: 3px solid #fdebd6; }
            .card[id^="card-5"] { border-left: 3px solid #f7e4ff; }
            .card[id^="card-6"] { border-left: 3px solid #e5f4f4; }
            .card[id^="card-7"] { border-left: 3px solid #f1f1f1; }
            .card[id^="card-8"] { border-left: 3px solid #fff1d9; }
            .card[id^="card-9"] { border-left: 3px solid #dde8ff; }
            .card[data-kind="primary"] .meta { font-weight: 600; }
            .card[data-kind="secondary"] .meta { font-weight: 400; }
            .list-item[data-state="active"] { background: #f8fbff; }
            .list-item[data-state="idle"] { background: transparent; }
            .list-row > .avatar + .list-item { margin-left: 10px; }
            .panel-header > .kicker + .summary { margin-top: 4px; }
            .hero-copy > h2 + p { margin-top: 8px; }
            .hero-copy > p + .btn { margin-top: 12px; }
            .nav-link[aria-current="page"] { text-decoration: underline; }
            .nav-link[href^="/"] { padding: 2px 0; }
            .card .tag[data-state="active"] { color: #175d32; }
            .card .tag[data-state="idle"] { color: #5b6575; }
            .summary + .footer { margin-top: 10px; }
            .figure + .excerpt { margin-top: 8px; }
            .card-title:first-child { letter-spacing: 0.01em; }
            .card-title:last-child { letter-spacing: 0.02em; }
        "###,
        );
        html.push_str("</style></head><body>");
        html.push_str("<div class=\"page shell\"><header class=\"masthead\" data-kind=\"primary\" data-state=\"active\"><div class=\"brand\" id=\"brand-root\">tobira</div><nav class=\"nav\" aria-label=\"Primary\"><a class=\"nav-item nav-link\" href=\"/home\" aria-current=\"page\">Home</a><a class=\"nav-item nav-link\" href=\"/topics\">Topics</a><a class=\"nav-item nav-link\" href=\"#\">More</a></nav><div class=\"timestamp\" data-kind=\"secondary\" data-state=\"idle\">updated now</div></header><main class=\"content\"><section class=\"hero\" data-kind=\"primary\" data-state=\"active\"><div class=\"hero-copy\"><h2 class=\"kicker\">bench</h2><p class=\"summary\">Deterministic multi-shape DOM for selector indexing and style invalidation.</p><button class=\"btn btn-primary toolbar-item\" data-state=\"active\" aria-pressed=\"false\">Run</button><button class=\"btn btn-ghost toolbar-item\" data-state=\"idle\">Later</button></div></section><section class=\"feed\">");

        for k in 0..block_count {
            let kind = if k % 4 == 0 { "primary" } else { "secondary" };
            let state = if k % 5 == 0 { "active" } else { "idle" };
            let article_tag = pick(&["article", "div"], k);
            let heading_tag = pick(&["h2", "h3"], k);
            let list_tag = pick(&["ul", "ol"], k + 1);
            let lead_tag = pick(&["p", "span"], k + 2);
            let mut class_attr = String::new();

            write!(&mut html, "<{} ", article_tag).unwrap();
            append_classes(&mut class_attr, k * 3 + 1);
            html.push_str(&class_attr);
            class_attr.clear();
            if k % 7 == 0 {
                write!(&mut html, " id=\"card-{}\"", k).unwrap();
            }
            write!(
                &mut html,
                " data-kind=\"{}\" data-state=\"{}\" data-seq=\"{}\">",
                kind, state, k
            )
            .unwrap();

            write!(&mut html, "<section ").unwrap();
            append_classes(&mut class_attr, k * 3 + 2);
            html.push_str(&class_attr);
            class_attr.clear();
            if k % 11 == 0 {
                write!(&mut html, " aria-label=\"story {}\"", k).unwrap();
            }
            html.push('>');

            write!(&mut html, "<div ").unwrap();
            append_classes(&mut class_attr, k * 3 + 3);
            html.push_str(&class_attr);
            class_attr.clear();
            write!(
                &mut html,
                " data-kind=\"{}\" data-state=\"{}\">",
                kind, state
            )
            .unwrap();

            write!(&mut html, "<{} class=\"card-title\" ", heading_tag).unwrap();
            if k % 6 == 0 {
                write!(&mut html, "id=\"card-{}-title\" ", k).unwrap();
            }
            html.push_str("data-kind=\"primary\">");
            write!(&mut html, "Story {}", k).unwrap();
            write!(&mut html, "</{}>", heading_tag).unwrap();

            html.push_str("<div class=\"card-body excerpt\" data-kind=\"secondary\" data-state=\"");
            html.push_str(state);
            html.push_str("\">");

            write!(&mut html, "<{} class=\"summary\" data-kind=\"{}\">", lead_tag, kind).unwrap();
            write!(&mut html, "{}", "A varied subtree with list, metadata, links, and controls.").unwrap();
            write!(&mut html, "</{}>", lead_tag).unwrap();

            write!(&mut html, "<{} class=\"list\" data-kind=\"secondary\">", list_tag).unwrap();
            for j in 0..3 {
                let item_state = if (k + j) % 4 == 0 { "active" } else { "idle" };
                let item_kind = if j == 0 { "primary" } else { "secondary" };
                write!(&mut html, "<li ").unwrap();
                if j == 1 && k % 4 == 0 {
                    write!(&mut html, "id=\"card-{}-item-{}\" ", k, j).unwrap();
                }
                html.push_str("class=\"list-row list-item\"");
                if j == 0 {
                    html.push_str(" data-kind=\"primary\"");
                } else {
                    html.push_str(" data-kind=\"secondary\"");
                }
                write!(
                    &mut html,
                    " data-state=\"{}\" data-index=\"{}\">",
                    item_state, j
                )
                .unwrap();
                write!(&mut html, "<span class=\"avatar\" aria-hidden=\"true\">{}</span>", if j == 0 { "◎" } else { "◌" }).unwrap();
                write!(
                    &mut html,
                    "<span class=\"meta\" data-kind=\"{}\" data-state=\"{}\">meta {}/{} </span>",
                    item_kind, item_state, k, j
                )
                .unwrap();
                write!(
                    &mut html,
                    "<a class=\"tag\" href=\"/topic/{}\" aria-label=\"tag {}\">tag</a>",
                    (k + j) % 9,
                    j
                )
                .unwrap();
                if j == 2 {
                    html.push_str("<button class=\"btn btn-ghost\" data-state=\"idle\">Open</button>");
                }
                html.push_str("</li>");
            }
            write!(&mut html, "</{}>", list_tag).unwrap();

            html.push_str("<footer class=\"footer\" data-kind=\"secondary\" data-state=\"idle\">");
            write!(
                &mut html,
                "<a class=\"footer-link\" href=\"/story/{}\">read more</a>",
                k
            )
            .unwrap();
            html.push_str("<a class=\"footer-link\" href=\"#\" aria-hidden=\"true\">share</a>");
            html.push_str("</footer>");

            html.push_str("</div></section></");
            html.push_str(article_tag);
            html.push('>');
        }

        html.push_str("</section><aside class=\"panel\" data-kind=\"secondary\" data-state=\"idle\"><div class=\"panel-header\"><span class=\"kicker\">topics</span><p class=\"summary muted\">More varied selectors, same deterministic tree.</p></div><div class=\"panel-body\"><figure class=\"figure\"><img class=\"thumb\" src=\"/img/thumb.png\" alt=\"thumb\" /><figcaption class=\"caption\">caption text</figcaption></figure></div></aside></main><footer class=\"footer\" data-kind=\"secondary\" data-state=\"idle\"><a class=\"footer-link\" href=\"/about\">about</a></footer></div>");

        html.push_str("</body></html>");
        html
    }

    fn median_ms(samples: &[Duration]) -> f64 {
        let mut values: Vec<f64> = samples
            .iter()
            .map(|duration| duration.as_secs_f64() * 1000.0)
            .collect();
        values.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mid = values.len() / 2;
        if values.len() % 2 == 1 {
            values[mid]
        } else {
            (values[mid - 1] + values[mid]) / 2.0
        }
    }

    fn measure_n<T, F>(runs: usize, mut f: F) -> Vec<Duration>
    where
        F: FnMut() -> T,
    {
        let mut samples = Vec::with_capacity(runs.saturating_sub(1));
        for i in 0..runs {
            let start = Instant::now();
            let value = f();
            black_box(value);
            let elapsed = start.elapsed();
            if i > 0 {
                samples.push(elapsed);
            }
        }
        samples
    }

    fn benchmark_heavy_dom_case(n: usize) -> (f64, f64, f64, f64, f64, f64) {
        let url = Url::parse("https://example.com/heavy-dom").unwrap();
        let html = make_heavy_html(n);
        let runs = 6;

        let build_samples = measure_n(runs, || {
            let (processed, session) = start_document_script_session(&html, &url);
            rebuild_page_from_html(
                &url,
                200,
                "OK".to_string(),
                Some("text/html".to_string()),
                &processed.html,
                processed.title_override.clone(),
                true,
                0,
                session,
            )
        });

        let parse_samples = measure_n(runs, || parse_document(&html));

        let mut style_document = parse_document(&html);
        annotate_node_ids(&mut style_document);
        let style_stylesheet = collect_stylesheet(&style_document, &url);
        let style_samples = measure_n(runs, || {
            build_styled_tree(
                &style_document,
                &style_stylesheet,
                1280,
                &InteractiveState::default(),
            )
        });

        let serialize_samples = measure_n(runs, || {
            let (_processed, mut session) = start_document_script_session(&html, &url);
            session.as_mut().map(|s| s.snapshot())
        });

        let apply_samples = measure_n(runs, || {
            let (processed, session) = start_document_script_session(&html, &url);
            let mut page = rebuild_page_from_html(
                &url,
                200,
                "OK".to_string(),
                Some("text/html".to_string()),
                &processed.html,
                processed.title_override.clone(),
                true,
                0,
                session,
            );
            page.set_dom_attribute(Some(1), "data-bench", "1");
            page
        });

        let cycle_samples = measure_n(runs, || {
            let (processed, session) = start_document_script_session(&html, &url);
            let mut page = rebuild_page_from_html(
                &url,
                200,
                "OK".to_string(),
                Some("text/html".to_string()),
                &processed.html,
                processed.title_override.clone(),
                true,
                0,
                session,
            );
            if let Some(session) = page.javascript_session.as_ref().cloned()
                && session.set_attribute(1, "data-bench", "1")
                && let Some(snapshot) = session.snapshot()
            {
                page.apply_script_snapshot(snapshot);
            }
            page
        });

        (
            median_ms(&build_samples),
            median_ms(&parse_samples),
            median_ms(&style_samples),
            median_ms(&serialize_samples),
            median_ms(&apply_samples),
            median_ms(&cycle_samples),
        )
    }

    #[test]
    #[ignore]
    fn heavy_dom_benchmark() {
        let cases = [1000, 5000, 10000, 20000];
        let mut rows = Vec::new();

        for &n in &cases {
            rows.push((n, benchmark_heavy_dom_case(n)));
        }

        println!("=== tobira heavy-DOM benchmark (release) ===");
        println!("N        build(ms)  parse(ms)  style(ms)  serialize(ms)  apply(ms)  cycle(ms)");
        println!("        build = initial parse + styled tree rebuild");
        println!("        parse = parse_document() only");
        println!("        style = build_styled_tree() only");
        println!("        serialize = session.snapshot() / serialize_document");
        println!("        apply = one DOM attribute change + rebuild");
        println!("        cycle = attribute change -> snapshot -> apply");
        for (n, (build, parse, style, serialize, apply, cycle)) in rows {
            println!(
                "{:<8} {:<10.2} {:<10.2} {:<10.2} {:<14.2} {:<10.2} {:<10.2}",
                n, build, parse, style, serialize, apply, cycle
            );
        }
        println!("querySelectorAll benchmark: not measured");
    }

    #[test]
    fn falls_back_when_document_is_empty() {
        let page = BrowserPage {
            url: Url::parse("http://example.com").unwrap(),
            status_code: 200,
            reason_phrase: "OK".to_string(),
            content_type: Some("text/html".to_string()),
            title: "Example".to_string(),
            html_source: String::new(),
            styled_document: StyledNode::Text(crate::css::StyledText {
                text: String::new(),
                style: crate::css::ComputedStyle {
                    display: crate::css::Display::Inline,
                    color: crate::css::DEFAULT_TEXT_COLOR,
                    background_color: None,
                    margin: crate::css::EdgeSizes::default(),
                    margin_left_auto: false,
                    margin_right_auto: false,
                    padding: crate::css::EdgeSizes::default(),
                    width: None,
                    height: None,
                    font_size_px: 16,
                    font_family: crate::css::FontFamilyKind::Sans,
                    text_align: crate::css::TextAlign::Left,
                    vertical_align: crate::css::VerticalAlign::Top,
                    font_weight: false,
                    underline: false,
                    white_space: crate::css::WhiteSpaceMode::Normal,
                    border: crate::css::EdgeSizes::default(),
                    border_color: 0,
                    border_style_none: false,
                    border_radius: 0,
                    outline_width: 0,
                    outline_color: None,
                    line_height: 0,
                    opacity: 255,
                    font_style_italic: false,
                    text_transform: crate::css::TextTransform::None,
                    text_indent: 0,
                    letter_spacing: 0,
                    max_width: None,
                    min_width: 0,
                    max_height: None,
                    min_height: 0,
                    box_sizing: crate::css::BoxSizing::ContentBox,
                    overflow: crate::css::Overflow::Visible,
                    list_style_type: crate::css::ListStyleType::Disc,
                    cursor_pointer: false,
                    cursor_kind: crate::css::CursorKind::Auto,
                    pointer_events_none: false,
                    text_decoration_color: None,
                    box_shadow: None,
                    content: None,
                    position: crate::css::Position::Static,
                    z_index: None,
                    top: None,
                    right: None,
                    bottom: None,
                    left: None,
                    flex_direction: crate::css::FlexDirection::Row,
                    flex_wrap: crate::css::FlexWrap::NoWrap,
                    align_items: crate::css::AlignItems::Stretch,
                    justify_content: crate::css::JustifyContent::FlexStart,
                    align_self: crate::css::AlignSelf::Auto,
                    align_content: crate::css::AlignContent::Stretch,
                    flex_grow: 0,
                    flex_shrink: 100,
                    flex_basis: None,
                    gap: 0,
                    order: 0,
                    effective_opacity: 255,
                    aspect_ratio: None,
                    object_fit: crate::css::ObjectFit::Fill,
                    object_position_x: 50,
                    object_position_y: 50,
                    grid_template_columns: Vec::new(),
                    grid_template_rows: Vec::new(),
                    grid_auto_rows: crate::css::GridTrackSize::Auto,
                    grid_auto_columns: crate::css::GridTrackSize::Auto,
                    grid_column: crate::css::GridPlacement::default(),
                    grid_row: crate::css::GridPlacement::default(),
                    filter_blur_px: 0,
                    filter_brightness: 10000,
                    filter_opacity: 255,
                    line_through: false,
                    text_overflow_ellipsis: false,
                    text_shadow: None,
                    background_gradient: None,
                    background_image_url: None,
                    background_size: crate::css::BackgroundSize::Auto,
                    background_repeat: crate::css::BackgroundRepeat::Repeat,
                    background_position_x: 50,
                    background_position_y: 50,
                    transform_translate_x: 0,
                    transform_translate_y: 0,
                    transform_scale_x: 0,
                    transform_scale_y: 0,
                    transform_rotate_millideg: 0,
                    transform_origin_x: 500,
                    transform_origin_y: 500,
                },
            }),
            raw_document: Node::Text(String::new()),
            main_stylesheet: crate::css::Stylesheet::default(),
            images: crate::image::ImageStore::default(),
            rendered: Some("   ".to_string()),
            javascript_session: None,
            layout_revision: 0,
            scroll_y: 0,
            engine_pending: false,
        };

        assert_eq!(page.body_text(), "[empty document]");
    }

    #[test]
    fn formats_cli_output() {
        let page = BrowserPage {
            url: Url::parse("http://example.com").unwrap(),
            status_code: 200,
            reason_phrase: "OK".to_string(),
            content_type: Some("text/html".to_string()),
            title: "Hello".to_string(),
            html_source: String::new(),
            styled_document: parse_styled_text("Hello"),
            raw_document: Node::Text(String::new()),
            main_stylesheet: crate::css::Stylesheet::default(),
            images: crate::image::ImageStore::default(),
            rendered: Some("# Hello".to_string()),
            javascript_session: None,
            layout_revision: 0,
            scroll_y: 0,
            engine_pending: false,
        };

        let output = page.to_cli_output();

        assert!(output.contains("URL: http://example.com/"));
        assert!(output.contains("Status: 200 OK"));
        assert!(output.contains("# Hello"));
    }

    #[test]
    fn extracts_embedded_stylesheet_and_title() {
        let document = parse_document(
            "<html><head><title>Demo</title><style>p { color: #ff0000; }</style></head><body><p>Hello</p></body></html>",
        );
        let stylesheet = collect_stylesheet(&document, &Url::parse("http://example.com").unwrap());

        assert_eq!(stylesheet.rules.len(), 1);
        assert_eq!(document_title(&document), Some("Demo".to_string()));
    }

    #[test]
    fn collects_frame_sources() {
        let document = parse_document(
            "<frameset cols=\"18,82\"><frame src=\"menu.htm\" name=\"left\"><frame src=\"top.htm\" name=\"right\"></frameset>",
        );

        let frames = collect_frame_specs(&document);

        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].src, "menu.htm");
        assert_eq!(frames[1].title.as_deref(), Some("right"));
    }

    #[test]
    fn script_snapshots_bump_layout_revision() {
        let mut page = BrowserPage {
            url: Url::parse("http://example.com").unwrap(),
            status_code: 200,
            reason_phrase: "OK".to_string(),
            content_type: Some("text/html".to_string()),
            title: "Hello".to_string(),
            html_source: "<html><body>Hello</body></html>".to_string(),
            styled_document: StyledNode::Text(crate::css::StyledText {
                text: "Hello".to_string(),
                style: crate::css::ComputedStyle {
                    display: crate::css::Display::Inline,
                    color: crate::css::DEFAULT_TEXT_COLOR,
                    background_color: None,
                    margin: crate::css::EdgeSizes::default(),
                    margin_left_auto: false,
                    margin_right_auto: false,
                    padding: crate::css::EdgeSizes::default(),
                    width: None,
                    height: None,
                    font_size_px: 16,
                    font_family: crate::css::FontFamilyKind::Sans,
                    text_align: crate::css::TextAlign::Left,
                    vertical_align: crate::css::VerticalAlign::Top,
                    font_weight: false,
                    underline: false,
                    white_space: crate::css::WhiteSpaceMode::Normal,
                    border: crate::css::EdgeSizes::default(),
                    border_color: 0,
                    border_style_none: false,
                    border_radius: 0,
                    outline_width: 0,
                    outline_color: None,
                    line_height: 0,
                    opacity: 255,
                    effective_opacity: 255,
                    font_style_italic: false,
                    text_transform: crate::css::TextTransform::None,
                    text_indent: 0,
                    letter_spacing: 0,
                    max_width: None,
                    min_width: 0,
                    max_height: None,
                    min_height: 0,
                    box_sizing: crate::css::BoxSizing::ContentBox,
                    overflow: crate::css::Overflow::Visible,
                    list_style_type: crate::css::ListStyleType::Disc,
                    cursor_pointer: false,
                    cursor_kind: crate::css::CursorKind::Auto,
                    pointer_events_none: false,
                    text_decoration_color: None,
                    box_shadow: None,
                    content: None,
                    position: crate::css::Position::Static,
                    z_index: None,
                    top: None,
                    right: None,
                    bottom: None,
                    left: None,
                    flex_direction: crate::css::FlexDirection::Row,
                    flex_wrap: crate::css::FlexWrap::NoWrap,
                    align_items: crate::css::AlignItems::Stretch,
                    justify_content: crate::css::JustifyContent::FlexStart,
                    align_self: crate::css::AlignSelf::Auto,
                    align_content: crate::css::AlignContent::Stretch,
                    flex_grow: 0,
                    flex_shrink: 100,
                    flex_basis: None,
                    gap: 0,
                    order: 0,
                    aspect_ratio: None,
                    object_fit: crate::css::ObjectFit::Fill,
                    object_position_x: 50,
                    object_position_y: 50,
                    grid_template_columns: Vec::new(),
                    grid_template_rows: Vec::new(),
                    grid_auto_rows: crate::css::GridTrackSize::Auto,
                    grid_auto_columns: crate::css::GridTrackSize::Auto,
                    grid_column: crate::css::GridPlacement::default(),
                    grid_row: crate::css::GridPlacement::default(),
                    filter_blur_px: 0,
                    filter_brightness: 10000,
                    filter_opacity: 255,
                    line_through: false,
                    text_overflow_ellipsis: false,
                    text_shadow: None,
                    background_gradient: None,
                    background_image_url: None,
                    background_size: crate::css::BackgroundSize::Auto,
                    background_repeat: crate::css::BackgroundRepeat::Repeat,
                    background_position_x: 50,
                    background_position_y: 50,
                    transform_translate_x: 0,
                    transform_translate_y: 0,
                    transform_scale_x: 0,
                    transform_scale_y: 0,
                    transform_rotate_millideg: 0,
                    transform_origin_x: 500,
                    transform_origin_y: 500,
                },
            }),
            raw_document: Node::Text(String::new()),
            main_stylesheet: crate::css::Stylesheet::default(),
            images: crate::image::ImageStore::default(),
            rendered: None,
            javascript_session: None,
            layout_revision: 0,
            scroll_y: 0,
            engine_pending: false,
        };
        let snapshot = crate::js::ProcessedScriptHtml {
            html: "<html><body>Updated</body></html>".to_string(),
            title_override: Some("Updated".to_string()),
            console_logs: Vec::new(),
            navigation_target: None,
            soft_navigation_target: None,
            scroll_y: 0,
            has_pending_work: false,
            structural_changes: Vec::new(),
        };

        page.apply_script_snapshot(snapshot);

        assert_eq!(page.layout_revision(), 1);
    }

    #[test]
    fn set_dom_attribute_rebuilds_live_page_snapshot() {
        let html = "<html><body><input id=\"name\" value=\"a\"></body></html>";
        let url = Url::parse("https://example.com").unwrap();
        let (processed, session) = start_document_script_session(html, &url);
        let mut page = rebuild_page_from_html(
            &url,
            200,
            "OK".to_string(),
            Some("text/html".to_string()),
            &processed.html,
            processed.title_override.clone(),
            true,
            0,
            session,
        );

        let initial_html = page.html_source.clone();
        page.set_dom_attribute(Some(1), "data-test", "updated");

        assert_eq!(page.layout_revision(), 1);
        assert_ne!(page.html_source, initial_html);
        assert!(page.html_source.contains("data-test=\"updated\""));
    }

    #[test]
    fn apply_script_snapshot_skips_rebuild_when_html_unchanged() {
        // A scroll/resize event produces a snapshot with the same HTML; that
        // must NOT trigger a full rebuild (which would starve the main thread on
        // rapid scroll), only update the scroll offset.
        let html = "<html><body><p>hi</p></body></html>";
        let url = Url::parse("https://example.com").unwrap();
        let (processed, session) = start_document_script_session(html, &url);
        let mut page = rebuild_page_from_html(
            &url,
            200,
            "OK".to_string(),
            Some("text/html".to_string()),
            &processed.html,
            processed.title_override.clone(),
            true,
            0,
            session,
        );
        let baseline_revision = page.layout_revision();

        let snapshot = crate::js::ProcessedScriptHtml {
            html: page.html_source.clone(),
            title_override: None,
            console_logs: Vec::new(),
            navigation_target: None,
            soft_navigation_target: None,
            scroll_y: 320,
            has_pending_work: false,
            structural_changes: Vec::new(),
        };
        page.apply_script_snapshot(snapshot);

        assert_eq!(
            page.layout_revision(),
            baseline_revision,
            "unchanged HTML must not rebuild the page"
        );
        assert_eq!(page.scroll_y(), 320);
    }

    #[test]
    fn synthetic_document_preserves_title_and_body_content() {
        let document = synthetic_document("Demo", vec![Node::Text("hello".to_string())]);
        let rendered = crate::render::render_document(&document);

        assert!(rendered.contains("# Demo"));
        assert!(rendered.contains("hello"));
    }

    #[test]
    fn extracts_only_body_children_from_document() {
        let document = parse_document(
            "<html><head><title>Demo</title></head><body><p>Hello</p></body></html>",
        );

        let children = extract_body_children(&document);

        assert_eq!(children.len(), 1);
        let Node::Element(paragraph) = &children[0] else {
            panic!("body child should be an element");
        };
        assert_eq!(paragraph.tag_name, "p");
    }

    #[test]
    fn rewrites_youtube_watch_pages_into_visible_summary() {
        let html = r#"
            <html>
              <head>
                <title>Video - YouTube</title>
                <meta name="description" content="fallback description">
                <meta property="og:image" content="https://i.ytimg.com/vi/demo/hqdefault.jpg">
                <link rel="canonical" href="https://www.youtube.com/watch?v=demo123">
              </head>
              <body>
                <script>
                  var ytInitialPlayerResponse = {
                    "videoDetails": {
                      "title": "Demo Video",
                      "author": "Demo Channel",
                      "viewCount": "1234567",
                      "lengthSeconds": "214",
                      "shortDescription": "Line one\nLine two",
                      "thumbnail": {
                        "thumbnails": [
                          {"url": "https://i.ytimg.com/vi/demo/default.jpg"},
                          {"url": "https://i.ytimg.com/vi/demo/maxresdefault.jpg"}
                        ]
                      },
                      "videoId": "demo123"
                    },
                    "microformat": {
                      "playerMicroformatRenderer": {
                        "publishDate": "2026-05-13",
                        "embed": {
                          "iframeUrl": "https://www.youtube.com/embed/demo123"
                        }
                      }
                    }
                  };
                </script>
              </body>
            </html>
        "#;
        let document = parse_document(html);

        let rewritten = build_site_specific_document(
            &document,
            html,
            &Url::parse("https://www.youtube.com/watch?v=demo123").unwrap(),
        )
        .expect("youtube watch pages should be rewritten");
        let rendered = crate::render::render_document(&rewritten);

        assert!(rendered.contains("Demo Video"));
        assert!(rendered.contains("Demo Channel"));
        assert!(rendered.contains("1,234,567"));
        assert!(rendered.contains("3:34"));
        assert!(rendered.contains("2026-05-13"));
    }

    #[test]
    fn does_not_rewrite_generic_youtube_pages_through_site_specific_path() {
        let html = "<html><body><div id=\"app\">Real shell</div></body></html>";
        let document = parse_document(html);

        let rewritten = build_site_specific_document(
            &document,
            html,
            &Url::parse("https://www.youtube.com/").unwrap(),
        );

        assert!(rewritten.is_none());
    }

    #[test]
    fn meaningful_body_detection_ignores_script_only_shells() {
        let document = parse_document(
            "<html><head><title>Demo</title></head><body><script>boot()</script></body></html>",
        );

        assert!(!document_has_meaningful_body(&document));
    }

    #[test]
    fn meaningful_body_detection_accepts_interactive_shell_markup() {
        let document = parse_document(
            "<html><body><div id=\"app\"></div><input type=\"text\" value=\"search\"></body></html>",
        );

        assert!(document_has_meaningful_body(&document));
    }

    #[test]
    fn only_follows_same_origin_script_navigation() {
        let current = Url::parse("https://www.youtube.com/watch?v=demo").unwrap();
        let same_origin = Url::parse("https://www.youtube.com/results?search_query=rust").unwrap();
        let cross_origin = Url::parse("https://accounts.google.com/signin").unwrap();

        assert!(should_follow_script_navigation(&current, &same_origin));
        assert!(!should_follow_script_navigation(&current, &cross_origin));
    }

    #[test]
    fn rewrites_youtube_home_pages_into_shell_ui() {
        let html = r#"
            <html>
              <head>
                <title>YouTube</title>
              </head>
              <body>
                <script>
                  var ytInitialData = {
                    "contents": {
                      "twoColumnBrowseResultsRenderer": {
                        "tabs": [
                          {
                            "tabRenderer": {
                              "selected": true,
                              "content": {
                                "richGridRenderer": {
                                  "contents": [
                                    {
                                      "richItemRenderer": {
                                        "content": {
                                          "videoRenderer": {
                                            "title": {"runs": [{"text": "Demo Home Video"}]},
                                            "ownerText": {"runs": [{"text": "Demo Creator"}]},
                                            "viewCountText": {"simpleText": "1,234 views"},
                                            "publishedTimeText": {"simpleText": "2 days ago"},
                                            "lengthText": {"simpleText": "12:34"},
                                            "thumbnail": {
                                              "thumbnails": [
                                                {"url": "https://i.ytimg.com/vi/demo/hqdefault.jpg"}
                                              ]
                                            },
                                            "navigationEndpoint": {
                                              "commandMetadata": {
                                                "webCommandMetadata": {"url": "/watch?v=demo123"}
                                              }
                                            }
                                          }
                                        }
                                      }
                                    },
                                    {
                                      "richSectionRenderer": {
                                        "content": {
                                          "feedNudgeRenderer": {
                                            "title": {"runs": [{"text": "Start by searching"}]},
                                            "subtitle": {"runs": [{"text": "Watch a few videos to build recommendations."}]}
                                          }
                                        }
                                      }
                                    }
                                  ]
                                }
                              }
                            }
                          }
                        ]
                      }
                    },
                    "header": {
                      "feedTabbedHeaderRenderer": {
                        "title": {"runs": [{"text": "Home"}]}
                      }
                    },
                    "topbar": {
                      "desktopTopbarRenderer": {
                        "searchbox": {
                          "fusionSearchboxRenderer": {
                            "placeholderText": {"runs": [{"text": "Search"}]}
                          }
                        },
                        "topbarButtons": [
                          {
                            "topbarMenuButtonRenderer": {
                              "tooltip": "Settings"
                            }
                          },
                          {
                            "buttonRenderer": {
                              "text": {"runs": [{"text": "Sign in"}]}
                            }
                          }
                        ]
                      }
                    }
                  };
                </script>
              </body>
            </html>
        "#;

        let document = build_youtube_generic_document_from_html(
            html,
            &Url::parse("https://www.youtube.com/").unwrap(),
        );
        let rendered = crate::render::render_document(&document);

        assert!(rendered.contains("YouTube"));
        assert!(rendered.contains("Home"));
        assert!(rendered.contains("Search"));
        assert!(rendered.contains("Settings"));
        assert!(rendered.contains("Sign in"));
        assert!(rendered.contains("Demo Home Video"));
        assert!(rendered.contains("Demo Creator"));
    }

    fn parse_styled_text(text: &str) -> StyledNode {
        crate::css::StyledNode::Text(crate::css::StyledText {
            text: text.to_string(),
            style: crate::css::ComputedStyle {
                display: crate::css::Display::Inline,
                color: crate::css::DEFAULT_TEXT_COLOR,
                background_color: None,
                margin: crate::css::EdgeSizes::default(),
                margin_left_auto: false,
                margin_right_auto: false,
                padding: crate::css::EdgeSizes::default(),
                width: None,
                height: None,
                font_size_px: 16,
                font_family: crate::css::FontFamilyKind::Sans,
                text_align: crate::css::TextAlign::Left,
                vertical_align: crate::css::VerticalAlign::Top,
                font_weight: false,
                underline: false,
                white_space: crate::css::WhiteSpaceMode::Normal,
                border: crate::css::EdgeSizes::default(),
                border_color: 0,
                border_style_none: false,
                border_radius: 0,
                outline_width: 0,
                outline_color: None,
                line_height: 0,
                opacity: 255,
                font_style_italic: false,
                text_transform: crate::css::TextTransform::None,
                text_indent: 0,
                letter_spacing: 0,
                max_width: None,
                min_width: 0,
                max_height: None,
                min_height: 0,
                box_sizing: crate::css::BoxSizing::ContentBox,
                overflow: crate::css::Overflow::Visible,
                list_style_type: crate::css::ListStyleType::Disc,
                cursor_pointer: false,
                cursor_kind: crate::css::CursorKind::Auto,
                pointer_events_none: false,
                text_decoration_color: None,
                box_shadow: None,
                content: None,
                position: crate::css::Position::Static,
                z_index: None,
                top: None,
                right: None,
                bottom: None,
                left: None,
                flex_direction: crate::css::FlexDirection::Row,
                flex_wrap: crate::css::FlexWrap::NoWrap,
                align_items: crate::css::AlignItems::Stretch,
                justify_content: crate::css::JustifyContent::FlexStart,
                align_self: crate::css::AlignSelf::Auto,
                align_content: crate::css::AlignContent::Stretch,
                flex_grow: 0,
                flex_shrink: 100,
                flex_basis: None,
                gap: 0,
                order: 0,
                effective_opacity: 255,
                aspect_ratio: None,
                object_fit: crate::css::ObjectFit::Fill,
                object_position_x: 50,
                object_position_y: 50,
                grid_template_columns: Vec::new(),
                grid_template_rows: Vec::new(),
                grid_auto_rows: crate::css::GridTrackSize::Auto,
                grid_auto_columns: crate::css::GridTrackSize::Auto,
                grid_column: crate::css::GridPlacement::default(),
                grid_row: crate::css::GridPlacement::default(),
                filter_blur_px: 0,
                filter_brightness: 10000,
                filter_opacity: 255,
                line_through: false,
                text_overflow_ellipsis: false,
                text_shadow: None,
                background_gradient: None,
                background_image_url: None,
                background_size: crate::css::BackgroundSize::Auto,
                background_repeat: crate::css::BackgroundRepeat::Repeat,
                background_position_x: 50,
                background_position_y: 50,
                transform_translate_x: 0,
                transform_translate_y: 0,
                transform_scale_x: 0,
                transform_scale_y: 0,
                transform_rotate_millideg: 0,
                transform_origin_x: 500,
                transform_origin_y: 500,
            },
        })
    }
}
