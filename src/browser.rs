use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Instant;

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
use crate::layout::ElementHitbox;
use crate::render::render_document;
use crate::text::decode_text_response;
use crate::url::Url;

const MAX_FRAME_DEPTH: usize = 3;
const MAX_SCRIPT_NAVIGATION_DEPTH: usize = 3;

fn load_trace_enabled() -> bool {
    std::env::var_os("TOBIRA_TRACE_LOAD").is_some()
}

fn load_trace(message: impl AsRef<str>) {
    if load_trace_enabled() {
        eprintln!("[tobira-load] {}", message.as_ref());
    }
}

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
}

impl BrowserPage {
    pub fn status_text(&self) -> String {
        format!("{} {}", self.status_code, self.reason_phrase)
            .trim()
            .to_string()
    }

    pub fn apply_script_snapshot(&mut self, snapshot: ProcessedScriptHtml) {
        let html_unchanged = snapshot.html == self.html_source;
        let has_navigation_target = snapshot.navigation_target.is_some();
        let has_soft_navigation_target = snapshot.soft_navigation_target.is_some();
        if html_unchanged && !has_navigation_target && !has_soft_navigation_target {
            if let Some(title) = snapshot.title_override.clone() {
                self.title = title;
            }
            self.scroll_y = snapshot.scroll_y;
            return;
        }

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
    }

    pub(crate) fn layout_revision(&self) -> u64 {
        self.layout_revision
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

    /// True when this page has at least one element with a non-empty `animation` list.
    /// Used by the GUI to decide whether to schedule a per-frame redraw.
    pub fn has_active_animations(&self) -> bool {
        fn walk(node: &crate::css::StyledNode) -> bool {
            match node {
                crate::css::StyledNode::Element(el) => {
                    if !el.style.animations.is_empty() {
                        return true;
                    }
                    el.children.iter().any(walk)
                }
                crate::css::StyledNode::Text(_) => false,
            }
        }
        walk(&self.styled_document)
    }

    /// Apply animation interpolation in-place using `now_ms` as the current time and
    /// `start_ms` as a global animation start anchor. Per-element start-time tracking
    /// is not yet implemented; this is the MVP that animates everything against the
    /// same anchor (good enough for infinite loaders / passive UI animations).
    pub fn apply_animations(&mut self, now_ms: u64, start_ms: u64) {
        crate::css::apply_animations_to_tree(
            &mut self.styled_document,
            &self.main_stylesheet.keyframes,
            now_ms,
            start_ms,
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
            let changed = session.set_viewport_size(width, height);
            if changed {
                self.refresh_from_script_session();
            }
            return changed;
        }
        false
    }

    pub fn set_scroll_position(&mut self, y: u32) -> bool {
        if let Some(session) = &self.javascript_session {
            let changed = session.set_scroll_position(y);
            if changed {
                self.refresh_from_script_session();
            }
            return changed;
        }
        false
    }

    pub fn set_layout_hitboxes(&mut self, hitboxes: Vec<ElementHitbox>) -> bool {
        let Some(session) = self.javascript_session.as_ref() else {
            return false;
        };
        let changed = session.set_layout_hitboxes(hitboxes);
        if changed {
            self.refresh_from_script_session();
        }
        changed
    }

    pub(crate) fn javascript_session(&self) -> Option<JavaScriptSession> {
        self.javascript_session.as_ref().cloned()
    }

    pub(crate) fn has_pending_fetches(&self) -> bool {
        self.javascript_session
            .as_ref()
            .is_some_and(JavaScriptSession::has_pending_fetches)
    }

    pub(crate) fn fetch_result_queue(
        &self,
    ) -> Option<Arc<Mutex<VecDeque<crate::js::CompletedFetch>>>> {
        self.javascript_session
            .as_ref()
            .map(JavaScriptSession::fetch_result_queue)
    }

    pub(crate) fn refresh_from_script_session(&mut self) -> bool {
        let Some(session) = self.javascript_session.as_ref().cloned() else {
            return false;
        };
        let before_layout_revision = self.layout_revision;
        let before_html = self.html_source.clone();
        let before_title = self.title.clone();
        let before_scroll = self.scroll_y;
        let Some(snapshot) = session.snapshot() else {
            return false;
        };
        self.apply_script_snapshot(snapshot);
        before_layout_revision != self.layout_revision
            || before_html != self.html_source
            || before_title != self.title
            || before_scroll != self.scroll_y
    }

    pub fn dispatch_window_resize(&mut self) -> Option<DomEventDispatchResult> {
        if !self.has_global_event_listener("resize") {
            return None;
        }
        let result = self
            .javascript_session
            .as_ref()
            .and_then(|session| session.dispatch_global_event("resize", false, false))?;
        self.apply_script_snapshot(result.snapshot.clone());
        Some(result)
    }

    pub fn dispatch_scroll_event(&mut self) -> Option<DomEventDispatchResult> {
        if !self.has_global_event_listener("scroll") {
            return None;
        }
        let result = self
            .javascript_session
            .as_ref()
            .and_then(|session| session.dispatch_global_event("scroll", false, false))?;
        self.apply_script_snapshot(result.snapshot.clone());
        Some(result)
    }

    pub fn has_global_event_listener(&mut self, event_type: &str) -> bool {
        self.javascript_session
            .as_mut()
            .map(|session| session.has_global_event_listener(event_type))
            .unwrap_or(false)
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
pub(crate) struct LoadedDocumentSource {
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
    let source = load_page_source(url)?;
    Ok(build_page_from_source(source, include_rendered_output))
}

pub(crate) fn load_page_source(url: &Url) -> Result<LoadedDocumentSource> {
    load_document_source(url, 0)
}

pub(crate) fn build_page_from_source(
    source: LoadedDocumentSource,
    include_rendered_output: bool,
) -> BrowserPage {
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
    if let Some(soft_target) = source
        .processed_html
        .soft_navigation_target
        .as_deref()
        .and_then(|target| Url::parse(target).ok())
    {
        page.url = soft_target;
    }
    page
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
    let load_started = Instant::now();
    load_trace(format!(
        "load start url={} frame_depth={} script_nav_depth={}",
        url, frame_depth, script_navigation_depth
    ));
    let response = fetch(url)?;
    let content_type = response.header("content-type").map(str::to_string);
    load_trace(format!(
        "fetch done url={} final_url={} status={} bytes={} elapsed={:?}",
        url,
        response.final_url,
        response.status_code,
        response.body.len(),
        load_started.elapsed()
    ));
    let text = decode_text_response(&response.body, response.header("content-type"));
    load_trace(format!(
        "decode done final_url={} chars={} elapsed={:?}",
        response.final_url,
        text.len(),
        load_started.elapsed()
    ));
    let script_started = Instant::now();
    let (scripted, javascript_session) = start_document_script_session(&text, &response.final_url);
    load_trace(format!(
        "script session done final_url={} logs={} elapsed={:?}",
        response.final_url,
        scripted.console_logs.len(),
        script_started.elapsed()
    ));
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
    annotate_resource_urls(&mut parsed_document, &response.final_url);
    let document = if frame_depth < MAX_FRAME_DEPTH {
        expand_frames(&parsed_document, &response.final_url, frame_depth + 1)?
            .unwrap_or(parsed_document)
    } else {
        parsed_document
    };
    load_trace(format!(
        "load complete final_url={} elapsed={:?}",
        response.final_url,
        load_started.elapsed()
    ));
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

fn annotate_node_ids(document: &mut Node) {
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

        if !should_expand_frame(base_url, &frame_url) {
            load_trace(format!(
                "skip cross-origin frame base={} frame={} src={}",
                base_url, frame_url, frame.src
            ));
            continue;
        }

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

fn should_expand_frame(base_url: &Url, frame_url: &Url) -> bool {
    base_url.shares_origin(frame_url)
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
        if !should_expand_frame(base_url, &frame_url) {
            load_trace(format!(
                "skip cross-origin frame base={} frame={} src={}",
                base_url, frame_url, src
            ));
            continue;
        }
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

fn build_site_specific_document(_document: &Node, _html: &str, _url: &Url) -> Option<Node> {
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
    use super::{
        BrowserPage, collect_frame_specs, collect_stylesheet, document_has_meaningful_body,
        document_title, extract_body_children, rebuild_page_from_html, should_expand_frame,
        should_follow_script_navigation, synthetic_document,
    };
    use crate::css::StyledNode;
    use crate::html::{Node, parse_document};
    use crate::js::start_document_script_session;
    use crate::url::Url;

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
                    box_shadows: vec![],
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
                    grid_auto_flow: crate::css::GridAutoFlow::Row,
                    scroll_behavior_smooth: false,
                    clip_path: None,
                    writing_mode: crate::css::WritingMode::HorizontalTb,
                    direction: crate::css::Direction::Ltr,
                    grid_template_areas: Vec::new(),
                    grid_column: crate::css::GridPlacement::default(),
                    grid_row: crate::css::GridPlacement::default(),
                    grid_area_name: None,
                    filter_blur_px: 0,
                    filter_brightness: 10000,
                    filter_opacity: 255,
                    transform_translate_x: 0,
                    transform_translate_y: 0,
                    transform_scale_x: 0,
                    transform_scale_y: 0,
                    transform_rotate_millideg: 0,
                    transform_origin_x: 500,
                    transform_origin_y: 500,
                    line_through: false,
                    text_overflow_ellipsis: false,
                    text_shadow: None,
                    background_gradient: None,
                    background_image_url: None,
                    background_size: crate::css::BackgroundSize::Auto,
                    background_repeat: crate::css::BackgroundRepeat::Repeat,
                    background_position_x: 50,
                    background_position_y: 50,
                    word_break: crate::css::WordBreak::Normal,
                    overflow_wrap_break_word: false,
                    counter_reset: vec![],
                    counter_increment: vec![],
                    outline_offset: 0,
                    outline_visible: false,
                    animations: vec![],
                    transitions: vec![],
                },
            }),
            raw_document: Node::Text(String::new()),
            main_stylesheet: crate::css::Stylesheet::default(),
            images: crate::image::ImageStore::default(),
            rendered: Some("   ".to_string()),
            javascript_session: None,
            layout_revision: 0,
            scroll_y: 0,
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
    fn expands_only_same_origin_frames() {
        let base = Url::parse("https://www.youtube.com/").unwrap();
        let same_origin = Url::parse("https://www.youtube.com/embed/demo").unwrap();
        let cross_origin = Url::parse("https://accounts.google.com/signin").unwrap();

        assert!(should_expand_frame(&base, &same_origin));
        assert!(!should_expand_frame(&base, &cross_origin));
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
                    line_through: false,
                    white_space: crate::css::WhiteSpaceMode::Normal,
                    text_overflow_ellipsis: false,
                    text_shadow: None,
                    background_gradient: None,
                    background_image_url: None,
                    background_size: crate::css::BackgroundSize::default(),
                    background_repeat: crate::css::BackgroundRepeat::default(),
                    background_position_x: 0,
                    background_position_y: 0,
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
                    box_shadows: vec![],
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
                    grid_auto_flow: crate::css::GridAutoFlow::Row,
                    scroll_behavior_smooth: false,
                    clip_path: None,
                    writing_mode: crate::css::WritingMode::HorizontalTb,
                    direction: crate::css::Direction::Ltr,
                    grid_template_areas: Vec::new(),
                    grid_column: crate::css::GridPlacement::default(),
                    grid_row: crate::css::GridPlacement::default(),
                    grid_area_name: None,
                    filter_blur_px: 0,
                    filter_brightness: 10000,
                    filter_opacity: 255,
                    transform_translate_x: 0,
                    transform_translate_y: 0,
                    transform_scale_x: 0,
                    transform_scale_y: 0,
                    transform_rotate_millideg: 0,
                    transform_origin_x: 500,
                    transform_origin_y: 500,
                    word_break: crate::css::WordBreak::Normal,
                    overflow_wrap_break_word: false,
                    counter_reset: vec![],
                    counter_increment: vec![],
                    outline_offset: 0,
                    outline_visible: false,
                    animations: vec![],
                    transitions: vec![],
                },
            }),
            raw_document: Node::Text(String::new()),
            main_stylesheet: crate::css::Stylesheet::default(),
            images: crate::image::ImageStore::default(),
            rendered: None,
            javascript_session: None,
            layout_revision: 0,
            scroll_y: 0,
        };
        let snapshot = crate::js::ProcessedScriptHtml {
            html: "<html><body>Updated</body></html>".to_string(),
            title_override: Some("Updated".to_string()),
            console_logs: Vec::new(),
            navigation_target: None,
            soft_navigation_target: None,
            scroll_y: 0,
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
    fn scroll_events_do_not_rebuild_unchanged_documents() {
        let html = "<html><body><script>window.foo = 1;</script><div style=\"height: 2000px\"></div></body></html>";
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

        let initial_revision = page.layout_revision();
        let initial_html = page.html_source.clone();
        assert!(page.set_scroll_position(120));

        assert!(page.dispatch_scroll_event().is_none());

        assert_eq!(page.layout_revision(), initial_revision);
        assert_eq!(page.html_source, initial_html);
        assert_eq!(page.scroll_y(), 120);
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
                box_shadows: vec![],
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
                grid_auto_flow: crate::css::GridAutoFlow::Row,
                scroll_behavior_smooth: false,
                clip_path: None,
                writing_mode: crate::css::WritingMode::HorizontalTb,
                direction: crate::css::Direction::Ltr,
                grid_template_areas: Vec::new(),
                grid_column: crate::css::GridPlacement::default(),
                grid_row: crate::css::GridPlacement::default(),
                grid_area_name: None,
                filter_blur_px: 0,
                filter_brightness: 10000,
                filter_opacity: 255,
                transform_translate_x: 0,
                transform_translate_y: 0,
                transform_scale_x: 0,
                transform_scale_y: 0,
                transform_rotate_millideg: 0,
                transform_origin_x: 500,
                transform_origin_y: 500,
                line_through: false,
                text_overflow_ellipsis: false,
                text_shadow: None,
                background_gradient: None,
                background_image_url: None,
                background_size: crate::css::BackgroundSize::Auto,
                background_repeat: crate::css::BackgroundRepeat::Repeat,
                background_position_x: 50,
                background_position_y: 50,
                word_break: crate::css::WordBreak::Normal,
                overflow_wrap_break_word: false,
                counter_reset: vec![],
                counter_increment: vec![],
                outline_offset: 0,
                outline_visible: false,
                animations: vec![],
                transitions: vec![],
            },
        })
    }
}
