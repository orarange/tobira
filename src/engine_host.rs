//! Bridge between the self-built JS engine (`tobira_engine::engine`) and the
//! browser's DOM/host services.
//!
//! `BrowserHost` implements the engine's `Host` trait over an arena-backed DOM
//! built from the parsed HTML tree, so the engine's `document`/`window` bindings
//! operate on a real page. This is the foundation of the "boa removal" work
//! (see `ENGINE_INTEGRATION_PLAN.md`): it lets the engine run a document's
//! inline scripts and produce an updated HTML snapshot, behind a flag, without
//! disturbing the existing boa runtime in `js.rs`.
//!
//! The implementation is adapted from the engine's `TestDom` reference host.
#![allow(dead_code)]

use std::any::Any;
use std::collections::{BTreeMap, HashMap};

use tobira_engine::engine::{
    AdjacentPosition, Compiler, ConsoleMessage, DomEventInit, DomEventRequest, DomEventResult,
    DomMutation, DomMutationResult, DomStructuralChange,
    DomRead, DomReadResult, DomRect, FetchBody, FetchRequest, FetchResponse, FrameId, Heap, HistoryAction,
    HistoryOutcome,
    Host, HostData, HostError, HostEvent, HostResult, HostTimeSnapshot, LocationSnapshot,
    NavigationAction,
    NavigationOutcome, NetworkRequestId, NodeId, NodeKind, ObserverId, ObserverKind,
    ObserverOptions, ObserverOp, ObserverRecord, ObserverResult, Parser,
    ScrollMetrics, SiblingDirection, StorageOp, StorageResult, TimerId, TimerRequest, Vm, WindowId,
    WindowMetrics,
};

use crate::html::{Node, parse_document};
use crate::url::Url;

#[derive(Debug, Clone)]
enum DomNodeKind {
    Document,
    Element(String), // lowercased tag name
    Text(String),
    Fragment,
    /// A shadow root attached to `host` (arena index). `open` is its mode.
    ShadowRoot { host: usize, open: bool },
}

#[derive(Debug, Clone)]
struct DomNode {
    kind: DomNodeKind,
    parent: Option<usize>,
    children: Vec<usize>,
    attrs: BTreeMap<String, String>,
}

impl DomNode {
    fn document() -> Self {
        Self {
            kind: DomNodeKind::Document,
            parent: None,
            children: Vec::new(),
            attrs: BTreeMap::new(),
        }
    }
    fn element(tag: &str) -> Self {
        Self {
            kind: DomNodeKind::Element(tag.to_lowercase()),
            parent: None,
            children: Vec::new(),
            attrs: BTreeMap::new(),
        }
    }
    fn text(data: &str) -> Self {
        Self {
            kind: DomNodeKind::Text(data.to_string()),
            parent: None,
            children: Vec::new(),
            attrs: BTreeMap::new(),
        }
    }
    fn fragment() -> Self {
        Self {
            kind: DomNodeKind::Fragment,
            parent: None,
            children: Vec::new(),
            attrs: BTreeMap::new(),
        }
    }
    fn is_element(&self) -> bool {
        matches!(self.kind, DomNodeKind::Element(_))
    }
    fn tag_name(&self) -> Option<&str> {
        match &self.kind {
            DomNodeKind::Element(tag) => Some(tag.as_str()),
            _ => None,
        }
    }
}

/// Void (self-closing) HTML elements that have no closing tag / children.
const VOID_ELEMENTS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

/// Raw-text elements whose text content must be serialized verbatim (HTML
/// entities are NOT escaped inside them); escaping would corrupt JS (`=>`
/// becoming `=&gt;`) and CSS (`a > b` becoming `a &gt; b`).
const RAW_TEXT_ELEMENTS: &[&str] = &["script", "style"];

/// A `Host` implementation backed by an arena DOM built from parsed HTML.
/// A live observer registration (Mutation or Intersection). `targets` holds each
/// observed node (arena index) with its options; `pending` holds records
/// accumulated since the last delivery / `takeRecords`.
struct ObserverEntry {
    kind: ObserverKind,
    targets: Vec<(usize, ObserverOptions)>,
    pending: Vec<ObserverRecord>,
    /// For IntersectionObserver: last delivered `isIntersecting` per target arena
    /// index, so a record is only queued when the state changes (and once on
    /// first observe).
    intersecting: HashMap<usize, bool>,
}

/// One session-history entry: its URL, the `history.state` associated with it,
/// and the scroll position to restore when navigating (back/forward) to it.
struct HistoryEntry {
    url: String,
    state: HostData,
    scroll_y: f64,
}

pub struct BrowserHost {
    nodes: Vec<DomNode>,
    document: usize,
    /// The focused node (`document.activeElement`), set by focus/blur events.
    active_element: Option<usize>,
    console: Vec<String>,
    location: LocationSnapshot,
    /// A full (cross-document) navigation requested this turn — the browser
    /// reloads. Set by `location.href`/`assign`/`reload` and `Navigate`.
    navigation: Option<String>,
    /// A same-document navigation requested this turn (hash change, pushState,
    /// replaceState, back/forward) — the browser updates the URL bar + history
    /// without reloading. The most recent one wins.
    soft_navigation: Option<String>,
    /// Session history stack and the index of the current entry. Always holds at
    /// least the initial entry.
    history_stack: Vec<HistoryEntry>,
    history_index: usize,
    scroll_x: f64,
    scroll_y: f64,
    inner_width: f64,
    inner_height: f64,
    /// Shadow root arena index keyed by host element index.
    shadow_root_by_host: HashMap<usize, usize>,
    /// Per-slot snapshot of assigned light nodes, so `slotchange` only fires on
    /// a real change. Keyed by slot arena index.
    slot_snapshots: HashMap<usize, Vec<usize>>,
    /// Observer registrations, indexed by `ObserverId.0`. `None` slots are
    /// disconnected observers (kept so ids stay stable).
    observers: Vec<Option<ObserverEntry>>,
    /// Last-known element geometry from the browser's layout, keyed by the
    /// `data-tobira-node-id` (pre-order id) the browser assigns. Document
    /// coordinates; `getBoundingClientRect` subtracts the scroll offset to get
    /// viewport coordinates. Stale after a DOM mutation until the next layout
    /// feed (same as a real browser between reflows).
    geometry: HashMap<usize, DomRect>,
    structural_changes: Vec<DomStructuralChange>,
}

impl BrowserHost {
    /// Build a host with an arena DOM parsed from `html`, located at `url`.
    pub fn from_html(html: &str, url: &str) -> Self {
        let root = parse_document(html);
        let mut host = Self {
            nodes: Vec::new(),
            document: 0,
            active_element: None,
            console: Vec::new(),
            location: location_from_url(url),
            navigation: None,
            soft_navigation: None,
            history_stack: vec![HistoryEntry {
                url: location_from_url(url).href,
                state: HostData::Null,
                scroll_y: 0.0,
            }],
            history_index: 0,
            scroll_x: 0.0,
            scroll_y: 0.0,
            inner_width: 1280.0,
            inner_height: 720.0,
            shadow_root_by_host: HashMap::new(),
            slot_snapshots: HashMap::new(),
            observers: Vec::new(),
            geometry: HashMap::new(),
            structural_changes: Vec::new(),
        };
        host.document = host.push(DomNode::document());
        // The parsed root is an "document" element; graft its children under
        // our document node. html/head/body are NOT synthesized — like the boa
        // backend, `document.body` etc. are dynamic lookups that return null
        // until the page (or its scripts) provide the element.
        if let Node::Element(element) = &root {
            for child in &element.children {
                let child_idx = host.build_from_node(child);
                host.attach(host.document, child_idx);
            }
        }
        host
    }

    fn push(&mut self, node: DomNode) -> usize {
        let idx = self.nodes.len();
        self.nodes.push(node);
        idx
    }

    fn attach(&mut self, parent: usize, child: usize) {
        self.nodes[child].parent = Some(parent);
        self.nodes[parent].children.push(child);
    }

    /// Recursively build arena nodes from an `html::Node`, returning the new idx.
    fn build_from_node(&mut self, node: &Node) -> usize {
        match node {
            Node::Text(text) => self.push(DomNode::text(text)),
            Node::Element(element) => {
                let idx = self.push(DomNode::element(&element.tag_name));
                for (key, value) in &element.attributes {
                    self.nodes[idx].attrs.insert(key.clone(), value.clone());
                }
                let child_indices: Vec<usize> = element
                    .children
                    .iter()
                    .map(|child| self.build_from_node(child))
                    .collect();
                for child in child_indices {
                    self.attach(idx, child);
                }
                idx
            }
        }
    }

    /// The current `<html>` element (`document.documentElement`), found live so
    /// script-built documents resolve correctly.
    fn html_idx(&self) -> Option<usize> {
        self.find_descendant_tag(self.document, "html")
    }

    /// The current `<head>` element, found live (pre-order, like boa).
    fn head_idx(&self) -> Option<usize> {
        self.find_descendant_tag(self.document, "head")
    }

    /// The current `<body>` element, found live (pre-order, like boa).
    fn body_idx(&self) -> Option<usize> {
        self.find_descendant_tag(self.document, "body")
    }

    fn find_descendant_tag(&self, root: usize, tag: &str) -> Option<usize> {
        for &child in &self.nodes[root].children {
            if self.nodes[child].tag_name() == Some(tag) {
                return Some(child);
            }
            if let Some(found) = self.find_descendant_tag(child, tag) {
                return Some(found);
            }
        }
        None
    }

    fn detach(&mut self, child: usize) {
        if let Some(parent) = self.nodes[child].parent {
            self.nodes[parent].children.retain(|&c| c != child);
            self.nodes[child].parent = None;
        }
    }

    // ── MutationObserver recording ──────────────────────────────────────────

    /// True if `node` is `ancestor` or a descendant of it (walks parent links).
    fn is_self_or_ancestor(&self, ancestor: usize, node: usize) -> bool {
        let mut cur = Some(node);
        while let Some(idx) = cur {
            if idx == ancestor {
                return true;
            }
            cur = self.nodes.get(idx).and_then(|n| n.parent);
        }
        false
    }

    /// Record an attribute mutation against every observer watching `target_idx`
    /// (directly or, with `subtree`, as an ancestor) with `attributes` enabled.
    fn record_attribute_mutation(
        &mut self,
        target_idx: usize,
        name: &str,
        old_value: Option<String>,
    ) {
        let node_id = NodeId(target_idx as u32);
        if let Some(value) = self.nodes[target_idx].attrs.get(name).cloned() {
            self.structural_changes.push(DomStructuralChange::SetAttribute {
                node: node_id,
                name: name.to_string(),
                value,
            });
        } else {
            self.structural_changes.push(DomStructuralChange::RemoveAttribute {
                node: node_id,
                name: name.to_string(),
            });
        }
        if self.observers.is_empty() {
            return;
        }
        let mut hits: Vec<(usize, bool)> = Vec::new();
        for (oid, slot) in self.observers.iter().enumerate() {
            let Some(entry) = slot else { continue };
            if entry.kind != ObserverKind::Mutation {
                continue;
            }
            let mut matched = false;
            let mut want_old = false;
            for (obs_target, opts) in &entry.targets {
                if !opts.attributes {
                    continue;
                }
                let in_scope = *obs_target == target_idx
                    || (opts.subtree && self.is_self_or_ancestor(*obs_target, target_idx));
                if in_scope {
                    matched = true;
                    want_old |= opts.attribute_old_value;
                }
            }
            if matched {
                hits.push((oid, want_old));
            }
        }
        for (oid, want_old) in hits {
            let payload = HostData::Object(vec![
                ("type".to_string(), HostData::String("attributes".to_string())),
                (
                    "attributeName".to_string(),
                    HostData::String(name.to_string()),
                ),
                (
                    "oldValue".to_string(),
                    match (want_old, &old_value) {
                        (true, Some(v)) => HostData::String(v.clone()),
                        _ => HostData::Null,
                    },
                ),
            ]);
            if let Some(Some(entry)) = self.observers.get_mut(oid) {
                entry.pending.push(ObserverRecord {
                    target: NodeId(target_idx as u32),
                    kind: ObserverKind::Mutation,
                    payload,
                });
            }
        }
    }

    /// Record a characterData mutation against every observer watching
    /// `target_idx` (directly or, with `subtree`, as an ancestor).
    fn record_characterdata_mutation(&mut self, target_idx: usize, old_value: &str) {
        if let DomNodeKind::Text(text) = &self.nodes[target_idx].kind {
            self.structural_changes.push(DomStructuralChange::SetText {
                node: NodeId(target_idx as u32),
                value: text.clone(),
            });
        }
        if self.observers.is_empty() {
            return;
        }
        let mut hits: Vec<(usize, bool)> = Vec::new();
        for (oid, slot) in self.observers.iter().enumerate() {
            let Some(entry) = slot else { continue };
            if entry.kind != ObserverKind::Mutation {
                continue;
            }
            let mut matched = false;
            let mut want_old = false;
            for (obs_target, opts) in &entry.targets {
                if !opts.character_data {
                    continue;
                }
                let in_scope = *obs_target == target_idx
                    || (opts.subtree && self.is_self_or_ancestor(*obs_target, target_idx));
                if in_scope {
                    matched = true;
                    want_old |= opts.character_data_old_value;
                }
            }
            if matched {
                hits.push((oid, want_old));
            }
        }
        for (oid, want_old) in hits {
            let payload = HostData::Object(vec![
                (
                    "type".to_string(),
                    HostData::String("characterData".to_string()),
                ),
                (
                    "oldValue".to_string(),
                    if want_old {
                        HostData::String(old_value.to_string())
                    } else {
                        HostData::Null
                    },
                ),
            ]);
            if let Some(Some(entry)) = self.observers.get_mut(oid) {
                entry.pending.push(ObserverRecord {
                    target: NodeId(target_idx as u32),
                    kind: ObserverKind::Mutation,
                    payload,
                });
            }
        }
    }

    /// Record a childList mutation (added/removed nodes) against every observer
    /// watching `parent_idx` (directly or, with `subtree`, as an ancestor).
    fn record_childlist_mutation(&mut self, parent_idx: usize, added: &[usize], removed: &[usize]) {
        if added.is_empty() && removed.is_empty() {
            return;
        }
        self.structural_changes.push(DomStructuralChange::ChildList {
            parent: NodeId(parent_idx as u32),
            added: added.iter().map(|i| NodeId(*i as u32)).collect(),
            removed: removed.iter().map(|i| NodeId(*i as u32)).collect(),
        });
        if self.observers.is_empty() {
            return;
        }
        let mut hits: Vec<usize> = Vec::new();
        for (oid, slot) in self.observers.iter().enumerate() {
            let Some(entry) = slot else { continue };
            if entry.kind != ObserverKind::Mutation {
                continue;
            }
            let matched = entry.targets.iter().any(|(obs_target, opts)| {
                opts.child_list
                    && (*obs_target == parent_idx
                        || (opts.subtree && self.is_self_or_ancestor(*obs_target, parent_idx)))
            });
            if matched {
                hits.push(oid);
            }
        }
        if hits.is_empty() {
            return;
        }
        let added_nodes: Vec<HostData> = added
            .iter()
            .map(|i| HostData::Node(NodeId(*i as u32)))
            .collect();
        let removed_nodes: Vec<HostData> = removed
            .iter()
            .map(|i| HostData::Node(NodeId(*i as u32)))
            .collect();
        for oid in hits {
            let payload = HostData::Object(vec![
                ("type".to_string(), HostData::String("childList".to_string())),
                (
                    "addedNodes".to_string(),
                    HostData::Array(added_nodes.clone()),
                ),
                (
                    "removedNodes".to_string(),
                    HostData::Array(removed_nodes.clone()),
                ),
            ]);
            if let Some(Some(entry)) = self.observers.get_mut(oid) {
                entry.pending.push(ObserverRecord {
                    target: NodeId(parent_idx as u32),
                    kind: ObserverKind::Mutation,
                    payload,
                });
            }
        }
    }

    fn collect_text(&self, idx: usize) -> String {
        match &self.nodes[idx].kind {
            DomNodeKind::Text(text) => text.clone(),
            _ => {
                let mut out = String::new();
                for &child in &self.nodes[idx].children {
                    out.push_str(&self.collect_text(child));
                }
                out
            }
        }
    }

    fn inner_html(&self, idx: usize) -> String {
        let mut out = String::new();
        for &child in &self.nodes[idx].children {
            out.push_str(&self.serialize_node(child));
        }
        out
    }

    fn serialize_node(&self, idx: usize) -> String {
        match &self.nodes[idx].kind {
            DomNodeKind::Text(text) => escape_text(text),
            DomNodeKind::Element(tag) => {
                let mut out = format!("<{tag}");
                for (key, value) in &self.nodes[idx].attrs {
                    out.push_str(&format!(" {key}=\"{}\"", escape_attr(value)));
                }
                out.push('>');
                if VOID_ELEMENTS.contains(&tag.as_str()) {
                    return out;
                }
                if tag == "script" {
                    // Scripts already ran; the snapshot must not re-expose their
                    // source (matches the boa snapshot behavior).
                } else if RAW_TEXT_ELEMENTS.contains(&tag.as_str()) {
                    // <style> content is raw text — emit verbatim.
                    out.push_str(&self.collect_text(idx));
                } else {
                    out.push_str(&self.inner_html(idx));
                }
                out.push_str(&format!("</{tag}>"));
                out
            }
            // Shadow content is not part of the light-DOM snapshot.
            DomNodeKind::ShadowRoot { .. } => String::new(),
            DomNodeKind::Document | DomNodeKind::Fragment => self.inner_html(idx),
        }
    }

    /// Current vertical scroll offset (`window.scrollY`).
    pub fn scroll_y(&self) -> u32 {
        self.scroll_y.max(0.0) as u32
    }

    pub fn take_structural_changes(&mut self) -> Vec<DomStructuralChange> {
        std::mem::take(&mut self.structural_changes)
    }

    /// Update the window scroll offset (driven by the browser's scroll input).
    pub fn set_scroll(&mut self, x: f64, y: f64) {
        self.scroll_x = x.max(0.0);
        self.scroll_y = y.max(0.0);
    }

    /// Update the viewport size (`window.innerWidth` / `innerHeight`).
    pub fn set_viewport(&mut self, width: f64, height: f64) {
        self.inner_width = width.max(0.0);
        self.inner_height = height.max(0.0);
    }

    /// Serialize the whole document back to HTML (for the page snapshot).
    pub fn serialize_document(&self) -> String {
        self.serialize_node(self.document)
    }

    /// Map a browser `data-tobira-node-id` to this arena's node index.
    ///
    /// `browser.rs::annotate_node_ids` numbers the parsed document with a
    /// 1-based pre-order walk over `Node::Element`s, where the parsed root (the
    /// synthetic `document` element) is id 1 and its element descendants follow.
    /// Our arena's `Document` node plays the role of that root, so we count the
    /// `Document` node and every `Element` in the same pre-order. This is the
    /// bridge that lets the browser dispatch an event by `target_node_id` and
    /// have it land on the right engine node.
    pub fn handle_for_node_id(&self, target_id: usize) -> Option<usize> {
        if target_id == 0 {
            return None;
        }
        let mut counter = 0usize;
        self.find_by_tobira_id(self.document, target_id, &mut counter)
    }

    /// Inverse of `handle_for_node_id`: the `data-tobira-node-id` (1-based
    /// pre-order position over Document + Elements) for an arena node index.
    fn tobira_id_for_handle(&self, target_idx: usize) -> Option<usize> {
        let mut counter = 0usize;
        self.find_tobira_id(self.document, target_idx, &mut counter)
    }

    fn find_tobira_id(&self, idx: usize, target_idx: usize, counter: &mut usize) -> Option<usize> {
        if matches!(
            self.nodes[idx].kind,
            DomNodeKind::Document | DomNodeKind::Element(_)
        ) {
            *counter += 1;
            if idx == target_idx {
                return Some(*counter);
            }
        }
        for i in 0..self.nodes[idx].children.len() {
            let child = self.nodes[idx].children[i];
            if let Some(found) = self.find_tobira_id(child, target_idx, counter) {
                return Some(found);
            }
        }
        None
    }

    /// Feed element geometry from the browser's most recent layout. `rects` is
    /// `(data-tobira-node-id, x, y, width, height)` in document coordinates.
    pub fn set_geometry(&mut self, rects: &[(usize, f32, f32, f32, f32)]) {
        self.geometry.clear();
        for &(id, x, y, w, h) in rects {
            self.geometry.insert(
                id,
                DomRect {
                    x: x as f64,
                    y: y as f64,
                    width: w as f64,
                    height: h as f64,
                },
            );
        }
        self.compute_intersections();
    }

    /// Recompute IntersectionObserver state against the current viewport and
    /// queue a record for every observed target whose `isIntersecting` changed
    /// (or is being reported for the first time). Called whenever geometry is
    /// fed (layout/scroll). The VM drains the queued records and fires callbacks.
    fn compute_intersections(&mut self) {
        if self.observers.is_empty() {
            return;
        }
        let vp_top = self.scroll_y;
        let vp_bottom = self.scroll_y + self.inner_height;
        // (observer index, target idx, is_intersecting, ratio, rect)
        let mut updates: Vec<(usize, usize, bool, f64, DomRect)> = Vec::new();
        for (oid, slot) in self.observers.iter().enumerate() {
            let Some(entry) = slot else { continue };
            if entry.kind != ObserverKind::Intersection {
                continue;
            }
            for (target_idx, _opts) in &entry.targets {
                let Some(tid) = self.tobira_id_for_handle(*target_idx) else {
                    continue;
                };
                let Some(rect) = self.geometry.get(&tid) else {
                    continue;
                };
                let top = rect.y;
                let bottom = rect.y + rect.height;
                let visible = (bottom.min(vp_bottom) - top.max(vp_top)).max(0.0);
                let ratio = if rect.height > 0.0 {
                    (visible / rect.height).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let is_intersecting = ratio > 0.0;
                if entry.intersecting.get(target_idx).copied() != Some(is_intersecting) {
                    updates.push((oid, *target_idx, is_intersecting, ratio, rect.clone()));
                }
            }
        }
        let vp_top_v = self.scroll_y;
        let vp_h = self.inner_height;
        let vp_w = self.inner_width;
        for (oid, target_idx, is_intersecting, ratio, rect) in updates {
            if let Some(Some(entry)) = self.observers.get_mut(oid) {
                entry.intersecting.insert(target_idx, is_intersecting);
                let viewport_rect = |x: f64, y: f64, w: f64, h: f64| {
                    HostData::Object(vec![
                        ("x".to_string(), HostData::Number(x)),
                        ("y".to_string(), HostData::Number(y)),
                        ("width".to_string(), HostData::Number(w)),
                        ("height".to_string(), HostData::Number(h)),
                        ("top".to_string(), HostData::Number(y)),
                        ("left".to_string(), HostData::Number(x)),
                        ("right".to_string(), HostData::Number(x + w)),
                        ("bottom".to_string(), HostData::Number(y + h)),
                    ])
                };
                let bcr = viewport_rect(rect.x, rect.y - vp_top_v, rect.width, rect.height);
                let root_bounds = viewport_rect(0.0, 0.0, vp_w, vp_h);
                let payload = HostData::Object(vec![
                    ("isIntersecting".to_string(), HostData::Bool(is_intersecting)),
                    ("intersectionRatio".to_string(), HostData::Number(ratio)),
                    ("boundingClientRect".to_string(), bcr.clone()),
                    ("intersectionRect".to_string(), bcr),
                    ("rootBounds".to_string(), root_bounds),
                    ("time".to_string(), HostData::Number(0.0)),
                ]);
                entry.pending.push(ObserverRecord {
                    target: NodeId(target_idx as u32),
                    kind: ObserverKind::Intersection,
                    payload,
                });
            }
        }
    }

    /// `getBoundingClientRect` for an arena node: document-coordinate geometry
    /// from the last layout, shifted into viewport coordinates by the current
    /// scroll offset. Returns a zero rect when geometry is unknown.
    fn bounding_client_rect(&self, arena_idx: usize) -> DomRect {
        if let Some(id) = self.tobira_id_for_handle(arena_idx) {
            if let Some(rect) = self.geometry.get(&id) {
                return DomRect {
                    x: rect.x - self.scroll_x,
                    y: rect.y - self.scroll_y,
                    width: rect.width,
                    height: rect.height,
                };
            }
        }
        DomRect {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
        }
    }

    fn find_by_tobira_id(
        &self,
        idx: usize,
        target: usize,
        counter: &mut usize,
    ) -> Option<usize> {
        if matches!(
            self.nodes[idx].kind,
            DomNodeKind::Document | DomNodeKind::Element(_)
        ) {
            *counter += 1;
            if *counter == target {
                return Some(idx);
            }
        }
        for i in 0..self.nodes[idx].children.len() {
            let child = self.nodes[idx].children[i];
            if let Some(found) = self.find_by_tobira_id(child, target, counter) {
                return Some(found);
            }
        }
        None
    }

    /// Drain captured console output.
    pub fn take_console(&mut self) -> Vec<String> {
        std::mem::take(&mut self.console)
    }

    /// The document's `<title>` text, if any.
    pub fn title(&self) -> Option<String> {
        self.find_descendant_tag(self.document, "title")
            .map(|idx| self.collect_text(idx))
    }

    pub fn navigation_target(&self) -> Option<String> {
        self.navigation.clone()
    }

    /// A same-document navigation (hash / pushState / popstate) requested this
    /// turn, if any. The browser updates the URL + history without reloading.
    pub fn soft_navigation_target(&self) -> Option<String> {
        self.soft_navigation.clone()
    }

    /// Resolve a (possibly relative) URL against the current document URL.
    fn resolve_href(&self, url: &str) -> String {
        Url::parse(&self.location.href)
            .and_then(|base| base.resolve(url))
            .map(|u| u.to_string())
            .or_else(|_| Url::parse(url).map(|u| u.to_string()))
            .unwrap_or_else(|_| url.to_string())
    }

    fn strip_hash<'a>(&self, href: &'a str) -> &'a str {
        href.split('#').next().unwrap_or(href)
    }

    /// Apply a same-document URL change: update `location`, record the soft
    /// navigation target. Does NOT touch the history stack (callers do).
    fn commit_same_document(&mut self, href: &str) {
        self.location = location_from_url(href);
        self.soft_navigation = Some(href.to_string());
    }

    fn save_current_scroll(&mut self) {
        let y = self.scroll_y;
        if let Some(entry) = self.history_stack.get_mut(self.history_index) {
            entry.scroll_y = y;
        }
    }

    /// Move the history cursor by `delta`, clamped to the stack. When the cursor
    /// actually moves, restores the target entry's URL + scroll and reports a
    /// soft navigation; a zero/clamped move is a pure query (no side effects).
    fn history_go(&mut self, delta: i32) -> HistoryOutcome {
        let target = (self.history_index as i64 + delta as i64)
            .clamp(0, self.history_stack.len() as i64 - 1) as usize;
        if target == self.history_index {
            return self.current_history_outcome(None);
        }
        self.save_current_scroll();
        self.history_index = target;
        let (url, scroll) = {
            let entry = &self.history_stack[target];
            (entry.url.clone(), entry.scroll_y)
        };
        self.commit_same_document(&url);
        self.scroll_y = scroll;
        self.current_history_outcome(Some(scroll))
    }

    fn current_history_outcome(&self, restored_scroll_y: Option<f64>) -> HistoryOutcome {
        let entry = &self.history_stack[self.history_index];
        HistoryOutcome {
            href: entry.url.clone(),
            state: Some(entry.state.clone()),
            length: self.history_stack.len(),
            restored_scroll_y,
        }
    }

    /// Collect inline `<script>` source (those without a `src` attribute).
    pub fn inline_scripts(&self) -> Vec<String> {
        let mut scripts = Vec::new();
        self.collect_inline_scripts(self.document, &mut scripts);
        scripts
    }

    fn collect_inline_scripts(&self, root: usize, out: &mut Vec<String>) {
        for &child in &self.nodes[root].children {
            if self.nodes[child].tag_name() == Some("script")
                && !self.nodes[child].attrs.contains_key("src")
            {
                let text = self.collect_text(child);
                if !text.trim().is_empty() {
                    out.push(text);
                }
            }
            self.collect_inline_scripts(child, out);
        }
    }

    /// Collect every `<script>` in document order, distinguishing inline source
    /// from external (`src`) references. External scripts carry the raw `src`
    /// attribute, to be resolved against the document URL and fetched by the
    /// caller. Preserves source order so dependencies (e.g. React before the app)
    /// execute correctly.
    pub fn ordered_scripts(&self) -> Vec<ScriptSource> {
        let mut scripts = Vec::new();
        self.collect_ordered_scripts(self.document, &mut scripts);
        scripts
    }

    fn collect_ordered_scripts(&self, root: usize, out: &mut Vec<ScriptSource>) {
        for &child in &self.nodes[root].children {
            if self.nodes[child].tag_name() == Some("script") {
                match self.nodes[child].attrs.get("src") {
                    Some(src) if !src.trim().is_empty() => {
                        out.push(ScriptSource::External(src.trim().to_string()));
                    }
                    _ => {
                        let text = self.collect_text(child);
                        if !text.trim().is_empty() {
                            out.push(ScriptSource::Inline(text));
                        }
                    }
                }
            }
            self.collect_ordered_scripts(child, out);
        }
    }

    /// The document's base URL (used to resolve relative `src`/`href`).
    pub fn base_href(&self) -> String {
        self.location.href.clone()
    }

    /// Match a (possibly grouped) selector against a node.
    ///
    /// Supports: selector lists (`a, b`), compound selectors (`p.note#x[a=b]`),
    /// and the descendant (` `) and child (`>`) combinators, matched
    /// right-to-left like a real engine. Sibling combinators (`+`, `~`) and
    /// pseudo-classes are not yet handled (noted for Stage 3 parity work).
    fn node_matches_selector(&self, idx: usize, selector: &str) -> bool {
        selector
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .any(|complex| self.matches_complex(idx, complex))
    }

    /// Match a complex selector (compounds joined by descendant/child
    /// combinators) against `idx`, working right-to-left up the ancestor chain.
    fn matches_complex(&self, idx: usize, complex: &str) -> bool {
        let (compounds, combinators) = Self::split_complex(complex);
        if compounds.is_empty() {
            return false;
        }
        // The rightmost compound must match the candidate node itself.
        if !self.matches_compound(idx, compounds.last().unwrap()) {
            return false;
        }
        // Walk the remaining compounds right-to-left against ancestors.
        let mut current = idx;
        for i in (0..compounds.len() - 1).rev() {
            let combinator = combinators[i]; // between compounds[i] and [i+1]
            let target = &compounds[i];
            match combinator {
                '>' => {
                    let Some(parent) = self.nodes[current].parent else {
                        return false;
                    };
                    if !self.matches_compound(parent, target) {
                        return false;
                    }
                    current = parent;
                }
                _ => {
                    // Descendant: find any ancestor matching the target.
                    let mut anc = self.nodes[current].parent;
                    let mut matched = None;
                    while let Some(a) = anc {
                        if self.matches_compound(a, target) {
                            matched = Some(a);
                            break;
                        }
                        anc = self.nodes[a].parent;
                    }
                    match matched {
                        Some(a) => current = a,
                        None => return false,
                    }
                }
            }
        }
        true
    }

    /// Split a complex selector into compound selectors and the combinators
    /// between them. `combinators[i]` sits between `compounds[i]` and `[i+1]`
    /// (' ' for descendant, '>' for child).
    fn split_complex(complex: &str) -> (Vec<String>, Vec<char>) {
        let mut compounds = Vec::new();
        let mut combinators = Vec::new();
        let mut current = String::new();
        let mut pending_combinator: Option<char> = None;
        let mut chars = complex.trim().chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '>' => {
                    if !current.is_empty() {
                        compounds.push(std::mem::take(&mut current));
                    }
                    pending_combinator = Some('>');
                }
                c if c.is_whitespace() => {
                    if !current.is_empty() {
                        compounds.push(std::mem::take(&mut current));
                        // Default to descendant unless a '>' follows.
                        pending_combinator = Some(' ');
                    }
                }
                _ => {
                    if let Some(comb) = pending_combinator.take() {
                        combinators.push(comb);
                    }
                    current.push(c);
                }
            }
        }
        if !current.is_empty() {
            if let Some(comb) = pending_combinator.take() {
                combinators.push(comb);
            }
            compounds.push(current);
        }
        (compounds, combinators)
    }

    /// Match a compound selector (no combinators, e.g. `p.note#x[a=b]`).
    fn matches_compound(&self, idx: usize, compound: &str) -> bool {
        let node = &self.nodes[idx];
        if !node.is_element() {
            return false;
        }
        Self::tokenize_compound(compound)
            .iter()
            .all(|token| self.matches_simple(node, token))
    }

    /// Split a compound selector into its simple-selector tokens, e.g.
    /// `p.note#x[a=b]` -> ["p", ".note", "#x", "[a=b]"].
    fn tokenize_compound(compound: &str) -> Vec<String> {
        let mut tokens = Vec::new();
        let mut current = String::new();
        let mut chars = compound.chars().peekable();
        while let Some(&c) = chars.peek() {
            match c {
                '.' | '#' => {
                    if !current.is_empty() {
                        tokens.push(std::mem::take(&mut current));
                    }
                    current.push(c);
                    chars.next();
                }
                '[' => {
                    if !current.is_empty() {
                        tokens.push(std::mem::take(&mut current));
                    }
                    for c2 in chars.by_ref() {
                        current.push(c2);
                        if c2 == ']' {
                            break;
                        }
                    }
                    tokens.push(std::mem::take(&mut current));
                }
                _ => {
                    current.push(c);
                    chars.next();
                }
            }
        }
        if !current.is_empty() {
            tokens.push(current);
        }
        tokens
    }

    /// Match a single simple selector token against an element node.
    fn matches_simple(&self, node: &DomNode, token: &str) -> bool {
        let tag = node.tag_name().unwrap_or("");
        let id_attr = node.attrs.get("id").map(String::as_str).unwrap_or("");
        let class_attr = node.attrs.get("class").map(String::as_str).unwrap_or("");
        if token == "*" {
            return true;
        }
        if let Some(id) = token.strip_prefix('#') {
            return id_attr == id;
        }
        if let Some(class) = token.strip_prefix('.') {
            return class_attr.split_whitespace().any(|c| c == class);
        }
        if let Some(rest) = token.strip_prefix('[') {
            if let Some(inner) = rest.strip_suffix(']') {
                if let Some((name, value)) = inner.split_once('=') {
                    let value = value.trim_matches(['"', '\'']);
                    return node.attrs.get(name.trim()).map(String::as_str) == Some(value);
                }
                return node.attrs.contains_key(inner.trim());
            }
            return false;
        }
        tag == token.to_lowercase()
    }

    fn query_selector(&self, root: usize, selector: &str) -> Option<usize> {
        for &child in &self.nodes[root].children {
            if self.node_matches_selector(child, selector) {
                return Some(child);
            }
            if let Some(found) = self.query_selector(child, selector) {
                return Some(found);
            }
        }
        None
    }

    fn query_selector_all(&self, root: usize, selector: &str, out: &mut Vec<usize>) {
        for &child in &self.nodes[root].children {
            if self.node_matches_selector(child, selector) {
                out.push(child);
            }
            self.query_selector_all(child, selector, out);
        }
    }

    // ── Shadow DOM helpers (ported from the boa backend) ────────────────────

    fn is_shadow_root(&self, idx: usize) -> bool {
        matches!(
            self.nodes.get(idx).map(|n| &n.kind),
            Some(DomNodeKind::ShadowRoot { .. })
        )
    }

    /// The host element of a shadow-root node.
    fn shadow_root_host(&self, idx: usize) -> Option<usize> {
        match self.nodes.get(idx).map(|n| &n.kind) {
            Some(DomNodeKind::ShadowRoot { host, .. }) => Some(*host),
            _ => None,
        }
    }

    /// The nearest ancestor shadow root above `idx` (excluding `idx`).
    fn enclosing_shadow_root(&self, idx: usize) -> Option<usize> {
        let mut current = self.nodes.get(idx).and_then(|n| n.parent);
        while let Some(parent) = current {
            if self.is_shadow_root(parent) {
                return Some(parent);
            }
            current = self.nodes.get(parent).and_then(|n| n.parent);
        }
        None
    }

    /// The host element of the shadow tree containing `idx` (or, if `idx` is a
    /// shadow root, its own host).
    fn shadow_root_host_for_node(&self, idx: usize) -> Option<usize> {
        if self.is_shadow_root(idx) {
            return self.shadow_root_host(idx);
        }
        self.enclosing_shadow_root(idx)
            .and_then(|sr| self.shadow_root_host(sr))
    }

    /// Like `parent`, but a shadow root's "parent" is its host element.
    fn shadow_including_parent(&self, idx: usize) -> Option<usize> {
        if let Some(parent) = self.nodes.get(idx).and_then(|n| n.parent) {
            return Some(parent);
        }
        self.shadow_root_host(idx)
    }

    /// The root of `idx`'s tree. With `composed`, crosses shadow boundaries.
    fn root_node_id(&self, idx: usize, composed: bool) -> Option<usize> {
        let mut current = idx;
        loop {
            let parent = if composed {
                self.shadow_including_parent(current)
            } else {
                self.nodes.get(current).and_then(|n| n.parent)
            };
            match parent {
                Some(p) => current = p,
                None => return Some(current),
            }
        }
    }

    fn attr_of(&self, idx: usize, name: &str) -> String {
        self.nodes
            .get(idx)
            .and_then(|n| n.attrs.get(name).cloned())
            .unwrap_or_default()
    }

    /// Light-DOM nodes assigned to `slot_idx` (by matching slot name).
    fn slot_assigned_nodes(&self, slot_idx: usize) -> Vec<usize> {
        let Some(host_idx) = self.shadow_root_host_for_node(slot_idx) else {
            return Vec::new();
        };
        let slot_name = self.attr_of(slot_idx, "name");
        self.nodes
            .get(host_idx)
            .map(|host| {
                host.children
                    .iter()
                    .copied()
                    .filter(|&child| {
                        let child_slot = self.attr_of(child, "slot");
                        if slot_name.is_empty() {
                            child_slot.is_empty()
                        } else {
                            child_slot == slot_name
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn slot_assigned_nodes_flattened(&self, slot_idx: usize) -> Vec<usize> {
        fn collect(host: &BrowserHost, slot_idx: usize, visited: &mut Vec<usize>, out: &mut Vec<usize>) {
            if visited.contains(&slot_idx) {
                return;
            }
            visited.push(slot_idx);
            for node in host.slot_assigned_nodes(slot_idx) {
                if host.nodes[node].tag_name() == Some("slot") {
                    let before = out.len();
                    collect(host, node, visited, out);
                    if out.len() > before {
                        continue;
                    }
                }
                out.push(node);
            }
            visited.pop();
        }
        let mut visited = Vec::new();
        let mut out = Vec::new();
        collect(self, slot_idx, &mut visited, &mut out);
        out
    }

    /// All `<slot>` elements in a shadow tree (pre-order).
    fn slot_nodes_in_shadow_root(&self, shadow_idx: usize) -> Vec<usize> {
        let mut out = Vec::new();
        self.collect_slots(shadow_idx, &mut out);
        out
    }

    fn collect_slots(&self, idx: usize, out: &mut Vec<usize>) {
        for &child in &self.nodes[idx].children {
            if self.nodes[child].tag_name() == Some("slot") {
                out.push(child);
            }
            self.collect_slots(child, out);
        }
    }

    /// The `<slot>` a light-DOM `idx` is assigned to (its host's shadow tree).
    fn assigned_slot_for_node(&self, idx: usize) -> Option<usize> {
        let parent = self.nodes.get(idx).and_then(|n| n.parent)?;
        let shadow_idx = self.shadow_root_by_host.get(&parent).copied()?;
        let slot_name = self.attr_of(idx, "slot");
        self.slot_nodes_in_shadow_root(shadow_idx).into_iter().find(|&slot| {
            let name_on_slot = self.attr_of(slot, "name");
            if name_on_slot.is_empty() {
                slot_name.is_empty()
            } else {
                name_on_slot == slot_name
            }
        })
    }

    /// Event propagation path for `idx` in target → root order.
    fn event_path(&self, idx: usize, composed: bool) -> Vec<usize> {
        let mut path = vec![idx];
        let mut current = idx;
        loop {
            let parent = if composed {
                self.shadow_including_parent(current)
            } else {
                self.nodes.get(current).and_then(|n| n.parent)
            };
            match parent {
                Some(p) => {
                    path.push(p);
                    current = p;
                    if path.len() > 4096 {
                        break;
                    }
                }
                None => break,
            }
        }
        path
    }

    /// Retarget `target` relative to `current`'s tree root (shadow retargeting).
    fn retarget_event_target(&self, target: usize, current: usize) -> usize {
        let current_root = self.root_node_id(current, false).unwrap_or(current);
        let mut candidate = target;
        loop {
            let candidate_root = self.root_node_id(candidate, false).unwrap_or(candidate);
            if candidate_root == current_root {
                return candidate;
            }
            match self.shadow_root_host_for_node(candidate) {
                Some(next) if next != candidate => candidate = next,
                _ => return candidate,
            }
        }
    }

    /// DocumentFragment insertion semantics: inserting a fragment moves its
    /// children and leaves the fragment empty. Returns the nodes to insert
    /// (the node itself when it isn't a fragment), detached from any parent.
    fn flatten_fragment(&mut self, child_idx: usize) -> Vec<usize> {
        if matches!(self.nodes[child_idx].kind, DomNodeKind::Fragment) {
            let kids: Vec<usize> = self.nodes[child_idx].children.drain(..).collect();
            for &kid in &kids {
                self.nodes[kid].parent = None;
            }
            kids
        } else {
            vec![child_idx]
        }
    }

    /// Parse an HTML fragment and build arena nodes for its top-level children
    /// (returned detached — the caller decides where they go).
    fn build_fragment_children(&mut self, html: &str) -> Vec<usize> {
        let fragment = parse_document(html);
        match &fragment {
            Node::Element(element) => element
                .children
                .iter()
                .map(|child| self.build_from_node(child))
                .collect(),
            Node::Text(_) => vec![self.build_from_node(&fragment)],
        }
    }

    fn parse_and_set_inner_html(&mut self, parent: usize, html: &str) {
        let old_children: Vec<usize> = self.nodes[parent].children.clone();
        for child in old_children {
            self.nodes[child].parent = None;
        }
        self.nodes[parent].children.clear();

        // Parse the fragment via the real HTML parser, then graft its children.
        let fragment = parse_document(html);
        if let Node::Element(element) = &fragment {
            let child_indices: Vec<usize> = element
                .children
                .iter()
                .map(|child| self.build_from_node(child))
                .collect();
            for child in child_indices {
                self.attach(parent, child);
            }
        }
    }
}

fn escape_text(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn escape_attr(value: &str) -> String {
    value.replace('&', "&amp;").replace('"', "&quot;")
}

fn location_from_url(url: &str) -> LocationSnapshot {
    match Url::parse(url) {
        Ok(u) => {
            let protocol = format!("{}:", u.scheme);
            let default_port = match u.scheme.as_str() {
                "http" => 80,
                "https" => 443,
                _ => 0,
            };
            let port = if u.port == 0 || u.port == default_port {
                String::new()
            } else {
                u.port.to_string()
            };
            let host = if port.is_empty() {
                u.host.clone()
            } else {
                format!("{}:{}", u.host, port)
            };
            // The fragment (`#…`) is taken from the raw URL — the path parser may
            // fold it into `u.path`. Strip it before splitting path / query.
            let hash = match url.split_once('#') {
                Some((_, frag)) => format!("#{frag}"),
                None => String::new(),
            };
            let path_no_hash = u.path.split('#').next().unwrap_or(&u.path);
            let (pathname, search) = match path_no_hash.split_once('?') {
                Some((path, query)) => (path.to_string(), format!("?{query}")),
                None => (path_no_hash.to_string(), String::new()),
            };
            LocationSnapshot {
                href: url.to_string(),
                origin: format!("{protocol}//{host}"),
                protocol,
                host,
                hostname: u.host.clone(),
                port,
                pathname,
                search,
                hash,
            }
        }
        Err(_) => LocationSnapshot {
            href: url.to_string(),
            origin: String::new(),
            protocol: String::new(),
            host: String::new(),
            hostname: String::new(),
            port: String::new(),
            pathname: url.to_string(),
            search: String::new(),
            hash: String::new(),
        },
    }
}

impl Host for BrowserHost {
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn window(&self) -> WindowId {
        WindowId(0)
    }

    fn window_metrics(&self, _window: WindowId) -> HostResult<WindowMetrics> {
        Ok(WindowMetrics {
            inner_width: self.inner_width,
            inner_height: self.inner_height,
            scroll_x: self.scroll_x,
            scroll_y: self.scroll_y,
            device_pixel_ratio: 1.0,
        })
    }

    fn location(&self, _window: WindowId) -> HostResult<LocationSnapshot> {
        Ok(self.location.clone())
    }

    fn navigate(&mut self, action: NavigationAction) -> HostResult<NavigationOutcome> {
        match action {
            NavigationAction::Navigate { url, .. } => {
                // A full (cross-document) navigation: resolve against the current
                // document URL and let the browser reload.
                let resolved = self.resolve_href(&url);
                self.navigation = Some(resolved);
                Ok(NavigationOutcome {
                    committed: true,
                    same_document: false,
                })
            }
            NavigationAction::SetHash { hash, .. } => {
                // Same-document hash change: update the location + current history
                // entry and request a soft navigation (no reload).
                let hash = if hash.is_empty() || hash.starts_with('#') {
                    hash
                } else {
                    format!("#{hash}")
                };
                let base = self.strip_hash(&self.location.href);
                let new_href = format!("{base}{hash}");
                self.commit_same_document(&new_href);
                if let Some(entry) = self.history_stack.get_mut(self.history_index) {
                    entry.url = new_href.clone();
                }
                Ok(NavigationOutcome {
                    committed: true,
                    same_document: true,
                })
            }
        }
    }

    fn history(&mut self, action: HistoryAction) -> HostResult<HistoryOutcome> {
        match action {
            HistoryAction::PushState { url, state, .. } => {
                let href = url
                    .filter(|u| !u.is_empty())
                    .map(|u| self.resolve_href(&u))
                    .unwrap_or_else(|| self.location.href.clone());
                // Save the outgoing entry's scroll, drop any forward entries,
                // then push the new entry and make it current.
                self.save_current_scroll();
                self.history_stack.truncate(self.history_index + 1);
                self.history_stack.push(HistoryEntry {
                    url: href.clone(),
                    state,
                    scroll_y: 0.0,
                });
                self.history_index = self.history_stack.len() - 1;
                self.commit_same_document(&href);
                Ok(self.current_history_outcome(None))
            }
            HistoryAction::ReplaceState { url, state, .. } => {
                let href = url
                    .filter(|u| !u.is_empty())
                    .map(|u| self.resolve_href(&u))
                    .unwrap_or_else(|| self.location.href.clone());
                if let Some(entry) = self.history_stack.get_mut(self.history_index) {
                    entry.url = href.clone();
                    entry.state = state;
                }
                self.commit_same_document(&href);
                Ok(self.current_history_outcome(None))
            }
            HistoryAction::Back { .. } => Ok(self.history_go(-1)),
            HistoryAction::Forward { .. } => Ok(self.history_go(1)),
            HistoryAction::Go { delta, .. } => Ok(self.history_go(delta)),
        }
    }

    fn read_dom(&self, read: DomRead) -> HostResult<DomReadResult> {
        let node_exists = |idx: usize| idx < self.nodes.len();
        match read {
            DomRead::DocumentRoot { .. } => Ok(match self.html_idx() {
                Some(idx) => DomReadResult::Node(NodeId(idx as u32)),
                None => DomReadResult::None,
            }),
            DomRead::DocumentHead { .. } => Ok(match self.head_idx() {
                Some(idx) => DomReadResult::Node(NodeId(idx as u32)),
                None => DomReadResult::None,
            }),
            DomRead::DocumentBody { .. } => Ok(match self.body_idx() {
                Some(idx) => DomReadResult::Node(NodeId(idx as u32)),
                None => DomReadResult::None,
            }),
            DomRead::ActiveElement { .. } => {
                // Falls back body → html → document, mirroring boa.
                let idx = self
                    .active_element
                    .filter(|&idx| node_exists(idx))
                    .or_else(|| self.body_idx())
                    .or_else(|| self.html_idx())
                    .unwrap_or(self.document);
                Ok(DomReadResult::Node(NodeId(idx as u32)))
            }
            DomRead::QuerySelector { root, selectors } => {
                match self.query_selector(root.0 as usize, &selectors) {
                    Some(idx) => Ok(DomReadResult::Node(NodeId(idx as u32))),
                    None => Ok(DomReadResult::None),
                }
            }
            DomRead::QuerySelectorAll { root, selectors } => {
                let mut found = Vec::new();
                self.query_selector_all(root.0 as usize, &selectors, &mut found);
                Ok(DomReadResult::Nodes(
                    found.into_iter().map(|i| NodeId(i as u32)).collect(),
                ))
            }
            DomRead::Matches { node, selectors } => {
                if !node_exists(node.0 as usize) {
                    return Err(HostError::InvalidHandle);
                }
                Ok(DomReadResult::Bool(
                    self.node_matches_selector(node.0 as usize, &selectors),
                ))
            }
            DomRead::Closest { node, selectors } => {
                let mut cur = node.0 as usize;
                loop {
                    if node_exists(cur) && self.node_matches_selector(cur, &selectors) {
                        return Ok(DomReadResult::Node(NodeId(cur as u32)));
                    }
                    match self.nodes.get(cur).and_then(|n| n.parent) {
                        Some(parent) => cur = parent,
                        None => return Ok(DomReadResult::None),
                    }
                }
            }
            DomRead::Contains { ancestor, descendant } => {
                let mut cur = descendant.0 as usize;
                loop {
                    if cur == ancestor.0 as usize {
                        return Ok(DomReadResult::Bool(true));
                    }
                    match self.nodes.get(cur).and_then(|n| n.parent) {
                        Some(parent) => cur = parent,
                        None => return Ok(DomReadResult::Bool(false)),
                    }
                }
            }
            DomRead::Parent { node } => {
                if !node_exists(node.0 as usize) {
                    return Err(HostError::InvalidHandle);
                }
                Ok(match self.nodes[node.0 as usize].parent {
                    Some(parent) => DomReadResult::Node(NodeId(parent as u32)),
                    None => DomReadResult::None,
                })
            }
            DomRead::Children { node, elements_only } => {
                if !node_exists(node.0 as usize) {
                    return Err(HostError::InvalidHandle);
                }
                let ids = self.nodes[node.0 as usize]
                    .children
                    .iter()
                    .filter(|&&c| !elements_only || self.nodes[c].is_element())
                    .map(|&c| NodeId(c as u32))
                    .collect();
                Ok(DomReadResult::Nodes(ids))
            }
            DomRead::Sibling {
                node,
                direction,
                elements_only,
            } => {
                let idx = node.0 as usize;
                if !node_exists(idx) {
                    return Err(HostError::InvalidHandle);
                }
                if let Some(parent) = self.nodes[idx].parent {
                    let siblings = &self.nodes[parent].children;
                    if let Some(pos) = siblings.iter().position(|&c| c == idx) {
                        let candidates: Box<dyn Iterator<Item = usize>> = match direction {
                            SiblingDirection::Next => Box::new(siblings[pos + 1..].iter().copied()),
                            SiblingDirection::Previous => {
                                Box::new(siblings[..pos].iter().copied().rev())
                            }
                        };
                        for sib in candidates {
                            if !elements_only || self.nodes[sib].is_element() {
                                return Ok(DomReadResult::Node(NodeId(sib as u32)));
                            }
                        }
                    }
                }
                Ok(DomReadResult::None)
            }
            DomRead::NodeKind { node } => {
                if !node_exists(node.0 as usize) {
                    return Err(HostError::InvalidHandle);
                }
                let kind = match &self.nodes[node.0 as usize].kind {
                    DomNodeKind::Document => NodeKind::Document,
                    DomNodeKind::Element(_) => NodeKind::Element,
                    DomNodeKind::Text(_) => NodeKind::Text,
                    DomNodeKind::Fragment | DomNodeKind::ShadowRoot { .. } => {
                        NodeKind::DocumentFragment
                    }
                };
                Ok(DomReadResult::Kind(kind))
            }
            DomRead::NodeName { node } => {
                if !node_exists(node.0 as usize) {
                    return Err(HostError::InvalidHandle);
                }
                let name = match &self.nodes[node.0 as usize].kind {
                    DomNodeKind::Document => "#document".to_string(),
                    DomNodeKind::Element(tag) => tag.to_uppercase(),
                    DomNodeKind::Text(_) => "#text".to_string(),
                    DomNodeKind::Fragment => "#document-fragment".to_string(),
                    DomNodeKind::ShadowRoot { .. } => "#document-fragment".to_string(),
                };
                Ok(DomReadResult::String(name))
            }
            DomRead::NodeValue { node } => {
                if !node_exists(node.0 as usize) {
                    return Err(HostError::InvalidHandle);
                }
                match &self.nodes[node.0 as usize].kind {
                    DomNodeKind::Text(text) => Ok(DomReadResult::String(text.clone())),
                    _ => Ok(DomReadResult::None),
                }
            }
            DomRead::TextContent { node } => {
                if !node_exists(node.0 as usize) {
                    return Err(HostError::InvalidHandle);
                }
                Ok(DomReadResult::String(self.collect_text(node.0 as usize)))
            }
            DomRead::InnerHtml { node } => {
                if !node_exists(node.0 as usize) {
                    return Err(HostError::InvalidHandle);
                }
                Ok(DomReadResult::String(self.inner_html(node.0 as usize)))
            }
            DomRead::OuterHtml { node } => {
                if !node_exists(node.0 as usize) {
                    return Err(HostError::InvalidHandle);
                }
                Ok(DomReadResult::String(self.serialize_node(node.0 as usize)))
            }
            DomRead::Attribute { node, name } => {
                if !node_exists(node.0 as usize) {
                    return Err(HostError::InvalidHandle);
                }
                match self.nodes[node.0 as usize].attrs.get(&name) {
                    Some(value) => Ok(DomReadResult::String(value.clone())),
                    None => Ok(DomReadResult::None),
                }
            }
            DomRead::AttributeNames { node } => {
                if !node_exists(node.0 as usize) {
                    return Err(HostError::InvalidHandle);
                }
                Ok(DomReadResult::StringList(
                    self.nodes[node.0 as usize].attrs.keys().cloned().collect(),
                ))
            }
            DomRead::ShadowRoot { host } => Ok(match self.shadow_root_by_host.get(&(host.0 as usize)) {
                Some(&idx) => DomReadResult::Node(NodeId(idx as u32)),
                None => DomReadResult::None,
            }),
            DomRead::ShadowRootHost { node } => {
                match self.nodes.get(node.0 as usize).map(|n| &n.kind) {
                    Some(DomNodeKind::ShadowRoot { host, .. }) => {
                        Ok(DomReadResult::Node(NodeId(*host as u32)))
                    }
                    _ => Ok(DomReadResult::None),
                }
            }
            DomRead::ShadowRootMode { node } => {
                match self.nodes.get(node.0 as usize).map(|n| &n.kind) {
                    Some(DomNodeKind::ShadowRoot { open, .. }) => Ok(DomReadResult::String(
                        if *open { "open" } else { "closed" }.to_string(),
                    )),
                    _ => Ok(DomReadResult::String(String::new())),
                }
            }
            DomRead::RootNode { node, composed } => {
                match self.root_node_id(node.0 as usize, composed) {
                    Some(idx) => Ok(DomReadResult::Node(NodeId(idx as u32))),
                    None => Ok(DomReadResult::None),
                }
            }
            DomRead::AssignedSlot { node } => {
                match self.assigned_slot_for_node(node.0 as usize) {
                    Some(idx) => Ok(DomReadResult::Node(NodeId(idx as u32))),
                    None => Ok(DomReadResult::None),
                }
            }
            DomRead::EventPath { node, composed } => {
                let path = self.event_path(node.0 as usize, composed);
                Ok(DomReadResult::Nodes(
                    path.into_iter().map(|i| NodeId(i as u32)).collect(),
                ))
            }
            DomRead::RetargetTarget { target, current } => {
                let idx = self.retarget_event_target(target.0 as usize, current.0 as usize);
                Ok(DomReadResult::Node(NodeId(idx as u32)))
            }
            DomRead::AssignedNodes { slot, flatten } => {
                let nodes = if flatten {
                    self.slot_assigned_nodes_flattened(slot.0 as usize)
                } else {
                    self.slot_assigned_nodes(slot.0 as usize)
                };
                Ok(DomReadResult::Nodes(
                    nodes.into_iter().map(|i| NodeId(i as u32)).collect(),
                ))
            }
            DomRead::BoundingClientRect { node } => {
                Ok(DomReadResult::Rect(self.bounding_client_rect(node.0 as usize)))
            }
            DomRead::ScrollMetrics { .. } => Ok(DomReadResult::ScrollMetrics(ScrollMetrics {
                scroll_left: 0.0,
                scroll_top: 0.0,
                scroll_width: 0.0,
                scroll_height: 0.0,
                client_width: 0.0,
                client_height: 0.0,
            })),
        }
    }

    fn mutate_dom(&mut self, mutation: DomMutation) -> HostResult<DomMutationResult> {
        let exists = |nodes: &Vec<DomNode>, idx: usize| idx < nodes.len();
        match mutation {
            DomMutation::CreateElement { local_name, .. } => {
                let idx = self.push(DomNode::element(&local_name));
                Ok(DomMutationResult::Node(NodeId(idx as u32)))
            }
            DomMutation::CreateTextNode { data, .. } => {
                let idx = self.push(DomNode::text(&data));
                Ok(DomMutationResult::Node(NodeId(idx as u32)))
            }
            DomMutation::CreateDocumentFragment { .. } => {
                let idx = self.push(DomNode::fragment());
                Ok(DomMutationResult::Node(NodeId(idx as u32)))
            }
            DomMutation::CloneNode { node, deep } => {
                let idx = node.0 as usize;
                if !exists(&self.nodes, idx) {
                    return Err(HostError::InvalidHandle);
                }
                let new_idx = self.clone_subtree(idx, deep);
                self.nodes[new_idx].parent = None;
                Ok(DomMutationResult::Node(NodeId(new_idx as u32)))
            }
            DomMutation::SetTextContent { node, value } => {
                let idx = node.0 as usize;
                if !exists(&self.nodes, idx) {
                    return Err(HostError::InvalidHandle);
                }
                // On a Text/Comment node, `textContent`/`nodeValue`/`data` set the
                // node's OWN character data — it has no children. (React's
                // setTextContent fast-path does `firstChild.nodeValue = text` on the
                // text node, so getting this wrong leaves the UI stuck on its
                // initial text after every state update.)
                if let DomNodeKind::Text(text) = &mut self.nodes[idx].kind {
                    let old = std::mem::replace(text, value);
                    self.record_characterdata_mutation(idx, &old);
                    return Ok(DomMutationResult::None);
                }
                let removed: Vec<usize> = self.nodes[idx].children.clone();
                for &child in &removed {
                    self.nodes[child].parent = None;
                }
                self.nodes[idx].children.clear();
                let mut added: Vec<usize> = Vec::new();
                if !value.is_empty() {
                    let text = self.push(DomNode::text(&value));
                    self.attach(idx, text);
                    added.push(text);
                }
                self.record_childlist_mutation(idx, &added, &removed);
                Ok(DomMutationResult::None)
            }
            DomMutation::SetInnerHtml { node, html } => {
                let idx = node.0 as usize;
                if !exists(&self.nodes, idx) {
                    return Err(HostError::InvalidHandle);
                }
                let removed: Vec<usize> = self.nodes[idx].children.clone();
                self.parse_and_set_inner_html(idx, &html);
                let added: Vec<usize> = self.nodes[idx].children.clone();
                self.record_childlist_mutation(idx, &added, &removed);
                Ok(DomMutationResult::None)
            }
            DomMutation::SetOuterHtml { node, html } => {
                let idx = node.0 as usize;
                if !exists(&self.nodes, idx) {
                    return Err(HostError::InvalidHandle);
                }
                let Some(parent) = self.nodes[idx].parent else {
                    return Err(HostError::InvalidHandle);
                };
                let pos = self.nodes[parent]
                    .children
                    .iter()
                    .position(|&c| c == idx)
                    .unwrap_or(0);
                let added = self.build_fragment_children(&html);
                self.detach(idx);
                for (offset, &child) in added.iter().enumerate() {
                    self.nodes[child].parent = Some(parent);
                    self.nodes[parent].children.insert(pos + offset, child);
                }
                self.record_childlist_mutation(parent, &added, &[idx]);
                Ok(DomMutationResult::None)
            }
            DomMutation::InsertAdjacentHtml { node, position, html } => {
                let idx = node.0 as usize;
                if !exists(&self.nodes, idx) {
                    return Err(HostError::InvalidHandle);
                }
                let added = self.build_fragment_children(&html);
                let (parent, insert_at) = match position {
                    AdjacentPosition::BeforeBegin | AdjacentPosition::AfterEnd => {
                        let Some(parent) = self.nodes[idx].parent else {
                            return Err(HostError::InvalidHandle);
                        };
                        let pos = self.nodes[parent]
                            .children
                            .iter()
                            .position(|&c| c == idx)
                            .unwrap_or(0);
                        let at = if matches!(position, AdjacentPosition::BeforeBegin) {
                            pos
                        } else {
                            pos + 1
                        };
                        (parent, at)
                    }
                    AdjacentPosition::AfterBegin => (idx, 0),
                    AdjacentPosition::BeforeEnd => (idx, self.nodes[idx].children.len()),
                };
                for (offset, &child) in added.iter().enumerate() {
                    self.nodes[child].parent = Some(parent);
                    self.nodes[parent].children.insert(insert_at + offset, child);
                }
                self.record_childlist_mutation(parent, &added, &[]);
                Ok(DomMutationResult::None)
            }
            DomMutation::WriteHtml { html, .. } => {
                // document.write appends parsed content to <body> (falling back
                // to <html>/document when the page has no body element).
                let body = self
                    .body_idx()
                    .or_else(|| self.html_idx())
                    .unwrap_or(self.document);
                let fragment = parse_document(&html);
                if let Node::Element(element) = &fragment {
                    let child_indices: Vec<usize> = element
                        .children
                        .iter()
                        .map(|child| self.build_from_node(child))
                        .collect();
                    for child in child_indices {
                        self.attach(body, child);
                    }
                }
                Ok(DomMutationResult::None)
            }
            DomMutation::SetAttribute { node, name, value } => {
                let idx = node.0 as usize;
                if !exists(&self.nodes, idx) {
                    return Err(HostError::InvalidHandle);
                }
                let old = self.nodes[idx].attrs.get(&name).cloned();
                self.nodes[idx].attrs.insert(name.clone(), value);
                self.record_attribute_mutation(idx, &name, old);
                Ok(DomMutationResult::None)
            }
            DomMutation::RemoveAttribute { node, name } => {
                let idx = node.0 as usize;
                if !exists(&self.nodes, idx) {
                    return Err(HostError::InvalidHandle);
                }
                let old = self.nodes[idx].attrs.remove(&name);
                self.record_attribute_mutation(idx, &name, old);
                Ok(DomMutationResult::None)
            }
            DomMutation::ToggleAttribute { node, name, force } => {
                let idx = node.0 as usize;
                if !exists(&self.nodes, idx) {
                    return Err(HostError::InvalidHandle);
                }
                let present = self.nodes[idx].attrs.contains_key(&name);
                let old = self.nodes[idx].attrs.get(&name).cloned();
                let add = force.unwrap_or(!present);
                if add {
                    self.nodes[idx].attrs.entry(name.clone()).or_default();
                } else {
                    self.nodes[idx].attrs.remove(&name);
                }
                self.record_attribute_mutation(idx, &name, old);
                Ok(DomMutationResult::Bool(add))
            }
            DomMutation::Append { parent, children } => {
                let parent_idx = parent.0 as usize;
                if !exists(&self.nodes, parent_idx) {
                    return Err(HostError::InvalidHandle);
                }
                let mut added: Vec<usize> = Vec::new();
                for child in children {
                    let child_idx = child.0 as usize;
                    if !exists(&self.nodes, child_idx) {
                        continue;
                    }
                    for idx in self.flatten_fragment(child_idx) {
                        self.detach(idx);
                        self.attach(parent_idx, idx);
                        added.push(idx);
                    }
                }
                self.record_childlist_mutation(parent_idx, &added, &[]);
                Ok(DomMutationResult::None)
            }
            DomMutation::Prepend { parent, children } => {
                let parent_idx = parent.0 as usize;
                if !exists(&self.nodes, parent_idx) {
                    return Err(HostError::InvalidHandle);
                }
                let mut pos = 0;
                let mut added: Vec<usize> = Vec::new();
                for child in children {
                    let child_idx = child.0 as usize;
                    if !exists(&self.nodes, child_idx) {
                        continue;
                    }
                    for idx in self.flatten_fragment(child_idx) {
                        self.detach(idx);
                        self.nodes[idx].parent = Some(parent_idx);
                        self.nodes[parent_idx].children.insert(pos, idx);
                        pos += 1;
                        added.push(idx);
                    }
                }
                self.record_childlist_mutation(parent_idx, &added, &[]);
                Ok(DomMutationResult::None)
            }
            DomMutation::InsertBefore {
                parent,
                child,
                reference,
            } => {
                let parent_idx = parent.0 as usize;
                let child_idx = child.0 as usize;
                if !exists(&self.nodes, parent_idx) || !exists(&self.nodes, child_idx) {
                    return Err(HostError::InvalidHandle);
                }
                let inserted = self.flatten_fragment(child_idx);
                for &idx in &inserted {
                    self.detach(idx);
                    self.nodes[idx].parent = Some(parent_idx);
                    // Recompute against the reference each round: inserting
                    // before it keeps the fragment children in order.
                    let pos = match reference {
                        Some(reference) => self.nodes[parent_idx]
                            .children
                            .iter()
                            .position(|&c| c == reference.0 as usize)
                            .unwrap_or(self.nodes[parent_idx].children.len()),
                        None => self.nodes[parent_idx].children.len(),
                    };
                    self.nodes[parent_idx].children.insert(pos, idx);
                }
                self.record_childlist_mutation(parent_idx, &inserted, &[]);
                Ok(DomMutationResult::None)
            }
            DomMutation::ReplaceChild {
                parent,
                new_child,
                old_child,
            } => {
                let parent_idx = parent.0 as usize;
                let new_idx = new_child.0 as usize;
                let old_idx = old_child.0 as usize;
                if !exists(&self.nodes, parent_idx)
                    || !exists(&self.nodes, new_idx)
                    || !exists(&self.nodes, old_idx)
                {
                    return Err(HostError::InvalidHandle);
                }
                // Detach `new` from its current location FIRST — it may already be
                // a child of this same parent, in which case removing it shifts the
                // indices, so `pos` must be computed against the post-detach list
                // (otherwise children[pos] can index out of bounds and panic).
                self.detach(new_idx);
                if let Some(pos) = self.nodes[parent_idx]
                    .children
                    .iter()
                    .position(|&c| c == old_idx)
                {
                    self.nodes[new_idx].parent = Some(parent_idx);
                    self.nodes[old_idx].parent = None;
                    self.nodes[parent_idx].children[pos] = new_idx;
                    self.record_childlist_mutation(parent_idx, &[new_idx], &[old_idx]);
                }
                Ok(DomMutationResult::None)
            }
            DomMutation::Remove { node } => {
                let idx = node.0 as usize;
                if !exists(&self.nodes, idx) {
                    return Err(HostError::InvalidHandle);
                }
                // Capture the parent before detaching so the childList record
                // (removedNodes) is attributed to the old parent.
                let old_parent = self.nodes[idx].parent;
                self.detach(idx);
                if let Some(parent_idx) = old_parent {
                    self.record_childlist_mutation(parent_idx, &[], &[idx]);
                }
                Ok(DomMutationResult::None)
            }
            DomMutation::SplitText { node, offset } => {
                let idx = node.0 as usize;
                if !exists(&self.nodes, idx) {
                    return Err(HostError::InvalidHandle);
                }
                let DomNodeKind::Text(text) = &self.nodes[idx].kind else {
                    return Err(HostError::InvalidHandle);
                };
                let old = text.clone();
                let split_at = old
                    .char_indices()
                    .nth(offset)
                    .map(|(byte, _)| byte)
                    .unwrap_or(old.len());
                let head = old[..split_at].to_string();
                let tail = old[split_at..].to_string();
                if let DomNodeKind::Text(text) = &mut self.nodes[idx].kind {
                    *text = head;
                }
                let tail_idx = self.push(DomNode::text(&tail));
                if let Some(parent) = self.nodes[idx].parent {
                    let pos = self.nodes[parent]
                        .children
                        .iter()
                        .position(|&c| c == idx)
                        .map(|p| p + 1)
                        .unwrap_or(self.nodes[parent].children.len());
                    self.nodes[tail_idx].parent = Some(parent);
                    self.nodes[parent].children.insert(pos, tail_idx);
                }
                self.record_characterdata_mutation(idx, &old);
                Ok(DomMutationResult::Node(NodeId(tail_idx as u32)))
            }
            DomMutation::NoteFocusChange { node, focused, .. } => {
                let idx = node.0 as usize;
                if focused {
                    self.active_element = Some(idx);
                } else if self.active_element == Some(idx) {
                    self.active_element = None;
                }
                Ok(DomMutationResult::None)
            }
            DomMutation::SetWindowScroll { x, y, .. } => {
                // `window.scrollTo/scrollBy` — record the offset so `window.scrollY`
                // reflects it (and history scroll-restore can save/restore it).
                self.scroll_x = x.max(0.0);
                self.scroll_y = y.max(0.0);
                Ok(DomMutationResult::None)
            }
            DomMutation::SetScrollOffset { .. } => Ok(DomMutationResult::None),
            DomMutation::AttachShadow { host, open } => {
                let host_idx = host.0 as usize;
                if !exists(&self.nodes, host_idx) || !self.nodes[host_idx].is_element() {
                    return Err(HostError::InvalidHandle);
                }
                if self.shadow_root_by_host.contains_key(&host_idx) {
                    // Already attached — return the existing root.
                    return Ok(DomMutationResult::Node(NodeId(
                        self.shadow_root_by_host[&host_idx] as u32,
                    )));
                }
                let shadow_idx = self.push(DomNode {
                    kind: DomNodeKind::ShadowRoot { host: host_idx, open },
                    parent: None,
                    children: Vec::new(),
                    attrs: BTreeMap::new(),
                });
                self.shadow_root_by_host.insert(host_idx, shadow_idx);
                Ok(DomMutationResult::Node(NodeId(shadow_idx as u32)))
            }
            DomMutation::TakeSlotchangeSlots { .. } => {
                let mut changed = Vec::new();
                let shadow_ids: Vec<usize> = self.shadow_root_by_host.values().copied().collect();
                for shadow_idx in shadow_ids {
                    for slot in self.slot_nodes_in_shadow_root(shadow_idx) {
                        let assigned = self.slot_assigned_nodes(slot);
                        let changed_now = match self.slot_snapshots.get(&slot) {
                            Some(prev) => prev != &assigned,
                            None => !assigned.is_empty(),
                        };
                        if changed_now {
                            self.slot_snapshots.insert(slot, assigned);
                            changed.push(NodeId(slot as u32));
                        }
                    }
                }
                Ok(DomMutationResult::Nodes(changed))
            }
        }
    }

    fn dispatch_dom_event(&mut self, _request: DomEventRequest) -> HostResult<DomEventResult> {
        Ok(DomEventResult {
            default_prevented: false,
        })
    }

    fn console(&mut self, message: ConsoleMessage) -> HostResult<()> {
        self.console.push(message.parts.join(" "));
        Ok(())
    }

    fn schedule_timer(&mut self, _request: TimerRequest) -> HostResult<TimerId> {
        Ok(TimerId(0))
    }
    fn cancel_timer(&mut self, _id: TimerId) -> HostResult<bool> {
        Ok(false)
    }
    fn request_animation_frame(&mut self, _window: WindowId) -> HostResult<FrameId> {
        Ok(FrameId(0))
    }
    fn cancel_animation_frame(&mut self, _id: FrameId) -> HostResult<bool> {
        Ok(false)
    }
    fn fetch(&mut self, _request: FetchRequest) -> HostResult<NetworkRequestId> {
        Err(HostError::Unsupported)
    }
    fn fetch_sync(&mut self, request: FetchRequest) -> HostResult<FetchResponse> {
        // Browser policy (boa parity): the browser only issues body-less
        // requests, so a request carrying a body is rejected as a network
        // error (XHR/fetch then fire onerror/reject).
        if !matches!(request.body, FetchBody::Empty) {
            return Err(HostError::Network);
        }
        // Resolve the (possibly relative) URL against the document URL, then
        // perform the request with the browser's own HTTP client.
        let resolved = Url::parse(&self.location.href)
            .and_then(|base| base.resolve(&request.url))
            .or_else(|_| Url::parse(&request.url))
            .map_err(|_| HostError::Network)?;
        let response = crate::http::fetch(&resolved).map_err(|_| HostError::Network)?;
        let headers = response
            .headers
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();
        Ok(FetchResponse {
            final_url: response.final_url.to_string(),
            status: response.status_code,
            status_text: response.reason_phrase.clone(),
            headers,
            body: response.body,
        })
    }
    fn abort_fetch(&mut self, _id: NetworkRequestId) -> HostResult<bool> {
        Ok(false)
    }
    fn storage(&mut self, _op: StorageOp) -> HostResult<StorageResult> {
        Ok(StorageResult::None)
    }
    fn observer(&mut self, op: ObserverOp) -> HostResult<ObserverResult> {
        match op {
            ObserverOp::Create { kind } => {
                let id = self.observers.len() as u64;
                self.observers.push(Some(ObserverEntry {
                    kind,
                    targets: Vec::new(),
                    pending: Vec::new(),
                    intersecting: HashMap::new(),
                }));
                Ok(ObserverResult::Created(ObserverId(id)))
            }
            ObserverOp::Observe {
                observer,
                target,
                options,
            } => {
                let target_idx = target.0 as usize;
                if target_idx >= self.nodes.len() {
                    return Err(HostError::InvalidHandle);
                }
                if let Some(Some(entry)) = self.observers.get_mut(observer.0 as usize) {
                    // Per spec, re-observing the same target replaces its options.
                    entry.targets.retain(|(idx, _)| *idx != target_idx);
                    entry.targets.push((target_idx, options));
                    Ok(ObserverResult::None)
                } else {
                    Err(HostError::InvalidHandle)
                }
            }
            ObserverOp::Unobserve { observer, target } => {
                let target_idx = target.0 as usize;
                if let Some(Some(entry)) = self.observers.get_mut(observer.0 as usize) {
                    entry.targets.retain(|(idx, _)| *idx != target_idx);
                    entry.intersecting.remove(&target_idx);
                }
                Ok(ObserverResult::None)
            }
            ObserverOp::Disconnect { observer } => {
                if let Some(slot) = self.observers.get_mut(observer.0 as usize) {
                    *slot = None;
                }
                Ok(ObserverResult::None)
            }
            ObserverOp::TakeRecords { observer } => {
                if let Some(Some(entry)) = self.observers.get_mut(observer.0 as usize) {
                    Ok(ObserverResult::Records(std::mem::take(&mut entry.pending)))
                } else {
                    Ok(ObserverResult::Records(Vec::new()))
                }
            }
        }
    }
    fn now(&self) -> HostTimeSnapshot {
        let unix_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        HostTimeSnapshot {
            monotonic_ms: unix_ms,
            unix_ms,
        }
    }
    fn wait_for_host_events(&mut self, _max_wait_ms: Option<u64>) -> HostResult<Vec<HostEvent>> {
        Ok(Vec::new())
    }
}

impl BrowserHost {
    /// Deep/shallow clone a subtree, returning the new root index.
    fn clone_subtree(&mut self, idx: usize, deep: bool) -> usize {
        let mut clone = self.nodes[idx].clone();
        clone.parent = None;
        clone.children.clear();
        let new_idx = self.push(clone);
        if deep {
            let children = self.nodes[idx].children.clone();
            for child in children {
                let cloned_child = self.clone_subtree(child, true);
                self.attach(new_idx, cloned_child);
            }
        }
        new_idx
    }
}

/// Result of running a document's inline scripts on the self-built engine.
#[derive(Debug, Clone, Default)]
pub struct EngineRunResult {
    pub html: String,
    pub console_logs: Vec<String>,
    pub title: Option<String>,
    pub navigation_target: Option<String>,
    pub soft_navigation_target: Option<String>,
    pub error: Option<String>,
    pub scroll_y: u32,
    pub default_prevented: bool,
    /// Whether the engine still has pending event-loop work (timers / RAF /
    /// queued tasks) after this operation — the host should keep pumping.
    pub has_pending_work: bool,
    pub structural_changes: Vec<DomStructuralChange>,
}

/// Parse `html`, run its inline `<script>`s on the self-built engine against a
/// `BrowserHost` DOM, and return the resulting HTML snapshot + console output.
///
/// This is the engine-backed counterpart of the boa path in `js.rs`, used behind
/// the `TOBIRA_ENGINE` flag while parity is built up.
/// A `<script>` collected from the document: either inline source text or an
/// external reference whose `src` must be resolved + fetched before execution.
#[derive(Debug, Clone)]
pub enum ScriptSource {
    Inline(String),
    External(String),
}

pub fn run_document_scripts(html: &str, url: &str) -> EngineRunResult {
    EngineSession::start(html, url).1
}

/// A persistent engine-backed runtime over a `BrowserHost`. It keeps the `Vm`
/// alive after the initial document scripts run, so the browser can dispatch
/// DOM events (clicks, input, …) and request fresh snapshots over time. This is
/// what backs the engine path's interactive `JavaScriptSession`.
pub struct EngineSession {
    vm: Vm,
}

/// JS polyfills injected before page scripts. See `EngineSession::start`.
const RUNTIME_PRELUDE: &str = r#"
(function(){
  var g = globalThis;
  if (typeof g.MessageChannel === 'undefined') {
    function MessagePort(){ this._onmessage = null; this._other = null; this._listeners = null; }
    MessagePort.prototype.postMessage = function(data){
      var other = this._other;
      setTimeout(function(){
        if (!other) return;
        var ev = { data: data };
        if (typeof other._onmessage === 'function') other._onmessage(ev);
        if (other._listeners) {
          for (var i = 0; i < other._listeners.length; i++) other._listeners[i](ev);
        }
      }, 0);
    };
    Object.defineProperty(MessagePort.prototype, 'onmessage', {
      configurable: true,
      get: function(){ return this._onmessage; },
      set: function(v){ this._onmessage = v; }
    });
    MessagePort.prototype.addEventListener = function(t, fn){
      if (t === 'message') { if (!this._listeners) this._listeners = []; this._listeners.push(fn); }
    };
    MessagePort.prototype.removeEventListener = function(t, fn){
      if (t === 'message' && this._listeners) {
        this._listeners = this._listeners.filter(function(l){ return l !== fn; });
      }
    };
    MessagePort.prototype.start = function(){};
    MessagePort.prototype.close = function(){};
    function MessageChannel(){
      this.port1 = new MessagePort();
      this.port2 = new MessagePort();
      this.port1._other = this.port2;
      this.port2._other = this.port1;
    }
    g.MessageChannel = MessageChannel;
    g.MessagePort = MessagePort;
  }
})();
"#;

impl EngineSession {
    /// Build the runtime, run the document's inline scripts, settle async
    /// deferred work, and return the runtime plus the initial snapshot.
    pub fn start(html: &str, url: &str) -> (Self, EngineRunResult) {
        let host = BrowserHost::from_html(html, url);
        // Collect scripts in document order (inline + external `src`) and the base
        // URL to resolve relative `src` against, before the host moves into the Vm.
        let scripts = host.ordered_scripts();
        let base_href = host.base_href();
        let mut vm = Vm::with_host(Heap::new(), Box::new(host));

        let mut error = None;
        // Runtime prelude: a small JS polyfill for MessageChannel, which the engine
        // doesn't implement natively but React's scheduler uses to flush deferred
        // work (e.g. passive effects / useEffect on update). Built on setTimeout,
        // which the event loop already pumps. Guarded so a future native impl wins.
        if let Ok(program) = Parser::new(RUNTIME_PRELUDE).parse() {
            if let Ok(chunk) = Compiler::new(&program).compile() {
                let _ = vm.execute(&chunk);
            }
        }
        'scripts: for script in &scripts {
            // Resolve the source: inline text is used directly; an external `src`
            // is resolved against the document URL and fetched over HTTP (just like
            // a real browser loading `<script src>`). A fetch failure aborts the
            // remaining scripts, mirroring a hard load error.
            let source = match script {
                ScriptSource::Inline(text) => text.clone(),
                ScriptSource::External(src) => {
                    let resolved = Url::parse(&base_href)
                        .and_then(|base| base.resolve(src))
                        .or_else(|_| Url::parse(src));
                    match resolved {
                        Ok(url) => match crate::http::fetch(&url) {
                            Ok(response) => String::from_utf8_lossy(&response.body).into_owned(),
                            Err(e) => {
                                error = Some(format!("failed to fetch script {src}: {e}"));
                                break 'scripts;
                            }
                        },
                        Err(e) => {
                            error = Some(format!("invalid script url {src}: {e:?}"));
                            break 'scripts;
                        }
                    }
                }
            };
            match Parser::new(&source).parse() {
                Ok(program) => match Compiler::new(&program).compile() {
                    Ok(chunk) => {
                        if let Err(e) = vm.execute(&chunk) {
                            error = Some(format!("{e}"));
                            break;
                        }
                    }
                    Err(e) => {
                        error = Some(format!("compile: {e:?}"));
                        break;
                    }
                },
                Err(e) => {
                    error = Some(format!("parse: {e:?}"));
                    break;
                }
            }
        }

        // The document is parsed and its scripts ran: fire the initial load
        // events on the document/window (handle 0), like boa's
        // dispatch_initial_load_events.
        for event_type in ["readystatechange", "DOMContentLoaded", "load"] {
            let _ = vm.fire_dom_event(0, event_type);
        }

        // Settle deferred work (Promise microtasks + a 1ms timer window) so
        // async initial rendering reflects in the snapshot. The 1ms window
        // runs `setTimeout(fn, 1)` "next turn" callbacks like boa does, while
        // timers those callbacks schedule (and longer delays) stay pending.
        vm.run_due_jobs_at(1, 10_000);

        let mut session = Self { vm };
        let snapshot = session.snapshot_with_error(error);
        (session, snapshot)
    }

    fn host(&mut self) -> &mut BrowserHost {
        self.vm
            .host_mut()
            .as_any_mut()
            .downcast_mut::<BrowserHost>()
            .expect("host is a BrowserHost")
    }

    fn snapshot_with_error(&mut self, error: Option<String>) -> EngineRunResult {
        let pending = self.vm.has_pending_event_loop_work();
        let host = self.host();
        EngineRunResult {
            html: host.serialize_document(),
            console_logs: host.take_console(),
            title: host.title(),
            navigation_target: host.navigation_target(),
            soft_navigation_target: host.soft_navigation_target(),
            error,
            scroll_y: host.scroll_y(),
            default_prevented: false,
            has_pending_work: pending,
            structural_changes: host.take_structural_changes(),
        }
    }

    /// Advance time to `now_ms` and run any due timers + a single
    /// `requestAnimationFrame` pass. Returns whether anything actually ran, so
    /// the caller can skip serializing a fresh snapshot when the frame was a
    /// no-op (a page with a pending interval but nothing due this frame). Drives
    /// `setInterval`, `setTimeout(fn, delay)`, and animation loops over time.
    pub fn pump(&mut self, now_ms: u64) -> bool {
        self.vm.pump_event_loop(now_ms, 10_000)
    }

    /// Whether the engine still has pending event-loop work (timers / RAF /
    /// queued tasks). Lets the host decide whether to keep pumping.
    pub fn has_pending_work(&self) -> bool {
        self.vm.has_pending_event_loop_work()
    }

    /// Earliest pending timer due time (ms), if any — lets the host schedule a
    /// wakeup rather than busy-polling.
    pub fn next_timer_due_ms(&self) -> Option<u64> {
        self.vm.next_timer_due_ms()
    }

    /// Record the window scroll offset (from the browser's scroll handling) so
    /// `window.scrollY` reflects it and snapshots preserve the position instead
    /// of resetting it to 0.
    pub fn set_scroll_position(&mut self, y: u32) {
        self.host().set_scroll(0.0, y as f64);
    }

    /// Record the viewport size (`window.innerWidth` / `innerHeight`).
    pub fn set_viewport_size(&mut self, width: u32, height: u32) {
        self.host().set_viewport(width as f64, height as f64);
    }

    /// Feed element geometry from the browser's latest layout so
    /// `getBoundingClientRect` / `offsetWidth` etc. return real values.
    /// `rects` is `(data-tobira-node-id, x, y, width, height)` in document coords.
    /// Also recomputes IntersectionObserver state and delivers any changes.
    pub fn set_geometry(&mut self, rects: &[(usize, f32, f32, f32, f32)]) {
        self.host().set_geometry(rects);
        // IntersectionObserver records queued by the geometry update are flushed
        // to their callbacks here (the host can't call JS itself).
        self.vm.deliver_observer_records();
    }

    /// Current DOM snapshot. Console output captured since the last snapshot is
    /// drained into the result.
    pub fn snapshot(&mut self) -> EngineRunResult {
        self.snapshot_with_error(None)
    }

    /// Dispatch a DOM event to the node identified by the browser's
    /// `data-tobira-node-id` (`target_node_id`), settle deferred work, and
    /// return a fresh snapshot. Unknown node ids are a no-op (still snapshots).
    pub fn dispatch_event(
        &mut self,
        node_id: usize,
        event_type: &str,
        init: &DomEventInit,
    ) -> EngineRunResult {
        let handle = self.host().handle_for_node_id(node_id).map(|h| h as u32);
        let mut default_prevented = false;
        if let Some(handle) = handle {
            default_prevented = self
                .vm
                .fire_dom_event_with(handle, event_type, init)
                .unwrap_or(false);
            self.vm.run_due_jobs(10_000);
        }
        let mut snapshot = self.snapshot();
        snapshot.default_prevented = default_prevented;
        snapshot
    }

    /// Dispatch a global (window/document) event — engine node handle 0.
    /// Returns `None` when nothing listens for `event_type`, so the caller can
    /// skip applying a (no-op) snapshot — important for high-frequency events
    /// like `scroll`, which would otherwise force a relayout on every tick.
    pub fn dispatch_global_event(&mut self, event_type: &str) -> Option<EngineRunResult> {
        if !self.vm.has_event_listener(0, event_type) {
            return None;
        }
        let _ = self.vm.fire_dom_event(0, event_type);
        self.vm.run_due_jobs(10_000);
        Some(self.snapshot())
    }

    /// Set an attribute on the node identified by the browser's node id.
    pub fn set_attribute(&mut self, node_id: usize, name: &str, value: &str) {
        let host = self.host();
        if let Some(mut handle) = host.handle_for_node_id(node_id) {
            // Browser id 1 is the synthetic document root; attributes can't
            // serialize on the Document node, so they land on the root
            // element instead (matches the boa backend).
            if matches!(host.nodes[handle].kind, DomNodeKind::Document) {
                if let Some(&root) = host.nodes[handle]
                    .children
                    .iter()
                    .find(|&&c| matches!(host.nodes[c].kind, DomNodeKind::Element(_)))
                {
                    handle = root;
                }
            }
            let _ = host.mutate_dom(DomMutation::SetAttribute {
                node: NodeId(handle as u32),
                name: name.to_string(),
                value: value.to_string(),
            });
        }
    }

    /// Test helper: compile and run an extra script against the live session,
    /// settle deferred work, and return a fresh snapshot. Lets tests drive the
    /// page the way later user interaction would (e.g. `el.click()`).
    #[cfg(test)]
    pub fn eval_for_test(&mut self, src: &str) -> EngineRunResult {
        let error = match Parser::new(src).parse() {
            Ok(program) => match Compiler::new(&program).compile() {
                Ok(chunk) => self.vm.execute(&chunk).err().map(|e| format!("{e}")),
                Err(e) => Some(format!("compile: {e:?}")),
            },
            Err(e) => Some(format!("parse: {e:?}")),
        };
        self.vm.run_due_jobs(10_000);
        self.snapshot_with_error(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_structural_changes(html: &str, script: &str) -> Vec<DomStructuralChange> {
        let (mut session, initial) = EngineSession::start(html, "http://localhost/");
        assert!(initial.error.is_none(), "initial error: {:?}", initial.error);
        let result = session.eval_for_test(script);
        assert!(result.error.is_none(), "script error: {:?}", result.error);
        result.structural_changes
    }

    /// DOM-heavy probe: runs framework-grade DOM snippets through a real
    /// `BrowserHost` (the engine's production host) and reports which fail. Each
    /// snippet self-verifies with `assert(...)`, so a wrong result or a missing
    /// API surfaces as a captured engine error. Diagnostic — always passes.
    /// Run with: cargo test --bin tobira dom_heavy_probe -- --nocapture
    #[test]
    fn dom_heavy_probe_report() {
        let probes: Vec<(&str, &str)> = vec![
            ("createElement+append+textContent", r#"
                const d = document.createElement('div'); d.textContent = 'hi';
                document.body.appendChild(d);
                assert(document.body.lastChild.textContent === 'hi');
            "#),
            ("setAttribute/getAttribute/has/remove", r#"
                const e = document.createElement('a');
                e.setAttribute('href', '/x'); assert(e.getAttribute('href') === '/x');
                assert(e.hasAttribute('href')); e.removeAttribute('href');
                assert(!e.hasAttribute('href'));
            "#),
            ("classList", r#"
                const e = document.createElement('div');
                e.classList.add('a'); e.classList.add('b');
                assert(e.classList.contains('a') && e.classList.contains('b'));
                e.classList.toggle('a'); assert(!e.classList.contains('a'));
                e.classList.remove('b'); assert(e.className.trim() === '');
            "#),
            ("className/id properties", r#"
                const e = document.createElement('div');
                e.className = 'x y'; e.id = 'foo';
                assert(e.className === 'x y' && e.id === 'foo');
                assert(e.getAttribute('class') === 'x y');
            "#),
            ("querySelector/All", r#"
                document.body.innerHTML = '<ul><li class="a">1</li><li class="a">2</li></ul>';
                assert(document.querySelector('li.a').textContent === '1');
                assert(document.querySelectorAll('li.a').length === 2);
            "#),
            ("getElementById/ByClass/ByTag", r#"
                document.body.innerHTML = '<div id="m" class="c">x</div><div class="c">y</div>';
                assert(document.getElementById('m').textContent === 'x');
                assert(document.getElementsByClassName('c').length === 2);
                assert(document.getElementsByTagName('div').length === 2);
            "#),
            ("insertBefore/removeChild/replaceChild", r#"
                const p = document.createElement('div');
                const a = document.createElement('span'); a.textContent='a';
                const b = document.createElement('span'); b.textContent='b';
                p.appendChild(a); p.insertBefore(b, a);
                assert(p.firstChild.textContent === 'b');
                const c = document.createElement('i'); c.textContent='c';
                p.replaceChild(c, a); assert(p.lastChild.textContent === 'c');
                p.removeChild(c); assert(p.childNodes.length === 1);
            "#),
            ("traversal: children/parent/sibling", r#"
                const p = document.createElement('div');
                const a = document.createElement('span'); const b = document.createElement('b');
                p.appendChild(a); p.appendChild(b);
                assert(p.children.length === 2);
                assert(a.parentNode === p && a.nextSibling === b);
                assert(b.previousSibling === a);
                assert(a.parentElement === p);
            "#),
            ("nodeType/tagName/nodeName", r#"
                const e = document.createElement('div');
                assert(e.nodeType === 1);
                assert(e.tagName === 'DIV' || e.tagName === 'div');
                const t = document.createTextNode('x'); assert(t.nodeType === 3);
            "#),
            ("style get/set", r#"
                const e = document.createElement('div');
                e.style.color = 'red'; e.style.width = '10px';
                assert(e.style.color === 'red');
                assert(e.getAttribute('style').indexOf('color') >= 0);
            "#),
            ("cloneNode", r#"
                const e = document.createElement('div'); e.textContent = 'hi'; e.setAttribute('data-x','1');
                const c = e.cloneNode(true);
                assert(c.getAttribute('data-x') === '1');
            "#),
            ("dataset", r#"
                const e = document.createElement('div');
                e.setAttribute('data-user-id', '42');
                assert(e.dataset.userId === '42');
            "#),
            ("createTextNode/nodeValue", r#"
                const t = document.createTextNode('hello');
                assert(t.nodeValue === 'hello' || t.textContent === 'hello');
            "#),
            ("matches/closest", r#"
                document.body.innerHTML = '<div class="outer"><p id="t">x</p></div>';
                const t = document.getElementById('t');
                assert(t.matches('p'));
                assert(t.closest('.outer') !== null);
            "#),
            ("append/prepend multiple", r#"
                const p = document.createElement('div');
                const a = document.createElement('a'); const b = document.createElement('b');
                p.append(a, b); assert(p.children.length === 2);
                const c = document.createElement('i'); p.prepend(c);
                assert(p.firstChild === c);
            "#),
            ("addEventListener+dispatchEvent", r#"
                const e = document.createElement('button');
                let hits = 0; e.addEventListener('click', () => { hits++; });
                e.dispatchEvent(new Event('click'));
                assert(hits === 1);
            "#),
            ("innerHTML set+read", r#"
                const e = document.createElement('div');
                e.innerHTML = '<span>x</span>';
                assert(e.children.length === 1 && e.firstChild.tagName.toLowerCase() === 'span');
            "#),
            ("remove()", r#"
                const p = document.createElement('div');
                const c = document.createElement('span'); p.appendChild(c);
                c.remove(); assert(p.childNodes.length === 0);
            "#),
            ("contains", r#"
                const p = document.createElement('div');
                const c = document.createElement('span'); p.appendChild(c);
                assert(p.contains(c)); assert(!c.contains(p));
            "#),
            ("createElementNS (SVG)", r#"
                const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
                assert(svg !== null && svg !== undefined);
            "#),
            ("documentElement/head/body", r#"
                assert(document.documentElement !== null);
                assert(document.head !== null && document.body !== null);
            "#),
            ("input value property", r#"
                const i = document.createElement('input');
                i.value = 'typed'; assert(i.value === 'typed');
            "#),
            ("hasChildNodes/firstElementChild", r#"
                const p = document.createElement('div');
                assert(!p.hasChildNodes());
                p.appendChild(document.createTextNode(' '));
                p.appendChild(document.createElement('b'));
                assert(p.hasChildNodes());
                assert(p.firstElementChild.tagName.toLowerCase() === 'b');
            "#),
            ("node expando property", r#"
                const a = document.createElement('div');
                a.__myKey = 42; a._data = { n: 1 };
                document.body.appendChild(a);
                assert(a.__myKey === 42);
                assert(document.body.lastChild.__myKey === 42);
                assert(document.body.lastChild._data.n === 1);
            "#),
            ("event bubbling + target", r#"
                document.body.innerHTML = '<div id="p"><button id="c">x</button></div>';
                const p = document.getElementById('p');
                const c = document.getElementById('c');
                let log = [];
                p.addEventListener('click', (e) => { log.push('p:' + e.target.id + ':' + e.currentTarget.id); });
                c.addEventListener('click', () => { log.push('c'); });
                c.dispatchEvent(new Event('click', { bubbles: true }));
                assert(log.join(',') === 'c,p:c:p', 'got ' + log.join(','));
            "#),
            ("event preventDefault", r#"
                const b = document.createElement('button');
                b.addEventListener('click', (e) => { e.preventDefault(); });
                const ev = new Event('click', { cancelable: true });
                const notCancelled = b.dispatchEvent(ev);
                assert(ev.defaultPrevented === true);
                assert(notCancelled === false);
            "#),
            ("removeEventListener", r#"
                const b = document.createElement('button');
                let n = 0; const handler = () => { n++; };
                b.addEventListener('click', handler);
                b.dispatchEvent(new Event('click'));
                b.removeEventListener('click', handler);
                b.dispatchEvent(new Event('click'));
                assert(n === 1, 'n=' + n);
            "#),
            ("stopPropagation", r#"
                document.body.innerHTML = '<div id="p2"><span id="c2">x</span></div>';
                let hits = 0;
                document.getElementById('p2').addEventListener('click', () => { hits++; });
                document.getElementById('c2').addEventListener('click', (e) => { e.stopPropagation(); });
                document.getElementById('c2').dispatchEvent(new Event('click', { bubbles: true }));
                assert(hits === 0, 'hits=' + hits);
            "#),
            ("mini-react render to real DOM", r#"
                function h(tag, props, ...kids){
                    const el = document.createElement(tag);
                    for (const k in props||{}) {
                        if (k === 'className') el.className = props[k];
                        else el.setAttribute(k, props[k]);
                    }
                    for (const kid of kids.flat()) {
                        el.appendChild(typeof kid === 'string' ? document.createTextNode(kid) : kid);
                    }
                    return el;
                }
                const app = h('div', {className:'app'}, h('h1', null, 'Title'), h('p', {id:'c'}, 'count: ', '3'));
                document.body.appendChild(app);
                const root = document.querySelector('.app');
                assert(root !== null);
                assert(root.querySelector('h1').textContent === 'Title');
                assert(root.querySelector('#c').textContent === 'count: 3');
            "#),
        ];

        let mut failures: Vec<(&str, String)> = Vec::new();
        for (name, snippet) in &probes {
            let snippet = snippet.to_string();
            // Catch panics (a missing/edge-case DOM op crashing the host) so the
            // probe reports them as failures instead of aborting the whole run.
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let html = format!("<html><body><script>{snippet}</script></body></html>");
                run_document_scripts(&html, "http://localhost/").error
            }));
            match outcome {
                Ok(Some(err)) => failures.push((name, err)),
                Ok(None) => {}
                Err(_) => failures.push((name, "PANIC".to_string())),
            }
        }
        let total = probes.len();
        let passed = total - failures.len();
        println!("\n=== dom-heavy probe: {passed}/{total} passed ===");
        for (name, err) in &failures {
            println!("  [DOM] {name}: {err}");
        }
        println!();
    }

    #[test]
    fn console_log_captured() {
        let result = run_document_scripts(
            "<html><body><script>console.log('hello', 1 + 2)</script></body></html>",
            "http://localhost/",
        );
        assert!(result.error.is_none(), "error: {:?}", result.error);
        assert_eq!(result.console_logs, vec!["hello 3".to_string()]);
    }

    #[test]
    fn structural_changes_record_without_observers() {
        let changes = run_structural_changes(
            "<html><body><div id=\"box\"></div><input id=\"field\" value=\"a\"><p id=\"p\">x</p><div id=\"wrap\">y</div><script>/* noop */</script></body></html>",
            r#"
                const box = document.getElementById('box');
                const field = document.getElementById('field');
                const p = document.getElementById('p');
                const wrap = document.getElementById('wrap');
                box.setAttribute('data-x', '1');
                field.removeAttribute('value');
                const child = document.createElement('div');
                box.appendChild(child);
                child.remove();
                p.textContent = 'hello';
                const text = wrap.firstChild;
                text.data = 'z';
            "#,
        );

        assert_eq!(changes.len(), 6);
        assert!(matches!(
            &changes[0],
            DomStructuralChange::SetAttribute { name, value, .. }
                if name == "data-x" && value == "1"
        ));
        assert!(matches!(
            &changes[1],
            DomStructuralChange::RemoveAttribute { name, .. } if name == "value"
        ));
        assert!(matches!(
            &changes[2],
            DomStructuralChange::ChildList { added, removed, .. }
                if added.len() == 1 && removed.is_empty()
        ));
        assert!(matches!(
            &changes[3],
            DomStructuralChange::ChildList { added, removed, .. }
                if added.is_empty() && removed.len() == 1
        ));
        assert!(matches!(
            &changes[4],
            DomStructuralChange::ChildList { added, removed, .. }
                if added.len() == 1 && removed.len() == 1
        ));
        assert!(matches!(
            &changes[5],
            DomStructuralChange::SetText { value, .. } if value == "z"
        ));
    }

    #[test]
    fn settimeout_zero_settles_before_snapshot() {
        let result = run_document_scripts(
            r#"<html><body><div id="x">a</div><script>
                setTimeout(() => { document.getElementById('x').textContent = 'b'; }, 0);
            </script></body></html>"#,
            "http://localhost/",
        );
        assert!(result.error.is_none(), "error: {:?}", result.error);
        assert!(result.html.contains(">b</div>"), "html: {}", result.html);
    }

    #[test]
    fn promise_then_settles_before_snapshot() {
        let result = run_document_scripts(
            r#"<html><body><div id="x">a</div><script>
                Promise.resolve().then(() => {
                    document.getElementById('x').textContent = 'c';
                });
            </script></body></html>"#,
            "http://localhost/",
        );
        assert!(result.error.is_none(), "error: {:?}", result.error);
        assert!(result.html.contains(">c</div>"), "html: {}", result.html);
    }

    #[test]
    fn nested_zero_delay_timers_cascade() {
        let result = run_document_scripts(
            r#"<html><body><div id="x">0</div><script>
                setTimeout(() => {
                    setTimeout(() => {
                        document.getElementById('x').textContent = 'done';
                    }, 0);
                }, 0);
            </script></body></html>"#,
            "http://localhost/",
        );
        assert!(result.error.is_none(), "error: {:?}", result.error);
        assert!(result.html.contains(">done</div>"), "html: {}", result.html);
    }

    #[test]
    fn delayed_timer_stays_pending_in_initial_snapshot() {
        let result = run_document_scripts(
            r#"<html><body><div id="x">initial</div><script>
                setTimeout(() => {
                    document.getElementById('x').textContent = 'late';
                }, 5000);
            </script></body></html>"#,
            "http://localhost/",
        );
        assert!(result.error.is_none(), "error: {:?}", result.error);
        assert!(
            result.html.contains(">initial</div>"),
            "html: {}",
            result.html
        );
        assert!(!result.html.contains(">late</div>"));
    }

    #[test]
    fn add_event_listener_and_dispatch_event() {
        let result = run_document_scripts(
            r#"<html><body><button id="btn">go</button><script>
                const btn = document.getElementById('btn');
                btn.addEventListener('click', (e) => {
                    console.log('fired:' + e.type);
                });
                const ret = btn.dispatchEvent(new Event('click'));
                console.log('returned:' + ret);
            </script></body></html>"#,
            "http://localhost/",
        );
        assert!(result.error.is_none(), "error: {:?}", result.error);
        assert_eq!(
            result.console_logs,
            vec!["fired:click".to_string(), "returned:true".to_string()]
        );
    }

    #[test]
    fn dispatch_event_drives_dom_mutation() {
        let result = run_document_scripts(
            r#"<html><body>
                <button id="btn">go</button>
                <div id="out">idle</div>
                <script>
                    const btn = document.getElementById('btn');
                    const out = document.getElementById('out');
                    btn.addEventListener('click', () => { out.textContent = 'clicked'; });
                    btn.dispatchEvent(new Event('click'));
                </script>
            </body></html>"#,
            "http://localhost/",
        );
        assert!(result.error.is_none(), "error: {:?}", result.error);
        assert!(
            result.html.contains(">clicked</div>"),
            "html: {}",
            result.html
        );
    }

    #[test]
    fn dispatch_event_returns_false_when_default_prevented() {
        let result = run_document_scripts(
            r#"<html><body><button id="btn">go</button><script>
                const btn = document.getElementById('btn');
                btn.addEventListener('click', (e) => { e.preventDefault(); });
                const ret = btn.dispatchEvent(new Event('click', { cancelable: true }));
                console.log('ret:' + ret);
            </script></body></html>"#,
            "http://localhost/",
        );
        assert!(result.error.is_none(), "error: {:?}", result.error);
        assert_eq!(result.console_logs, vec!["ret:false".to_string()]);
    }

    #[test]
    fn stop_immediate_propagation_halts_remaining_listeners() {
        let result = run_document_scripts(
            r#"<html><body><button id="btn">go</button><script>
                const btn = document.getElementById('btn');
                btn.addEventListener('click', (e) => {
                    console.log('first');
                    e.stopImmediatePropagation();
                });
                btn.addEventListener('click', () => { console.log('second'); });
                btn.dispatchEvent(new Event('click'));
            </script></body></html>"#,
            "http://localhost/",
        );
        assert!(result.error.is_none(), "error: {:?}", result.error);
        assert_eq!(result.console_logs, vec!["first".to_string()]);
    }

    #[test]
    fn custom_event_detail_reaches_listener() {
        let result = run_document_scripts(
            r#"<html><body><div id="x"></div><script>
                const x = document.getElementById('x');
                x.addEventListener('ping', (e) => {
                    console.log('detail=' + e.detail.value);
                });
                x.dispatchEvent(new CustomEvent('ping', { detail: { value: 42 } }));
            </script></body></html>"#,
            "http://localhost/",
        );
        assert!(result.error.is_none(), "error: {:?}", result.error);
        assert_eq!(result.console_logs, vec!["detail=42".to_string()]);
    }

    /// Assert that `BrowserHost::handle_for_node_id` reproduces the browser's
    /// real `annotate_node_ids` numbering over the actual snapshot pipeline
    /// (serialize -> parse -> annotate). Every annotated element's
    /// `data-tobira-node-id` must map back to an arena node with the same tag
    /// (and id attribute, when present).
    fn assert_node_id_alignment(html: &str, min_checks: usize) {
        use crate::browser::annotate_node_ids;
        use crate::html::{Node, parse_document};

        let host = BrowserHost::from_html(html, "http://localhost/");
        let serialized = host.serialize_document();
        let mut tree = parse_document(&serialized);
        annotate_node_ids(&mut tree);

        fn check(node: &Node, host: &BrowserHost, checked: &mut usize) {
            if let Node::Element(el) = node {
                if let Some(id_str) = el.attributes.get("data-tobira-node-id") {
                    let id: usize = id_str.parse().expect("numeric tobira id");
                    let handle = host
                        .handle_for_node_id(id)
                        .unwrap_or_else(|| panic!("no handle for tobira id {id}"));
                    let arena = &host.nodes[handle];
                    if el.tag_name == "document" {
                        assert!(
                            matches!(arena.kind, DomNodeKind::Document),
                            "tobira id {id} should map to the Document node"
                        );
                    } else {
                        assert_eq!(
                            arena.tag_name(),
                            Some(el.tag_name.to_lowercase().as_str()),
                            "tobira id {id}: tag mismatch"
                        );
                        if let Some(want) = el.attributes.get("id") {
                            assert_eq!(
                                arena.attrs.get("id"),
                                Some(want),
                                "tobira id {id}: id-attribute mismatch"
                            );
                        }
                    }
                    *checked += 1;
                }
                for child in &el.children {
                    check(child, host, checked);
                }
            }
        }
        let mut checked = 0;
        check(&tree, &host, &mut checked);
        assert!(
            checked >= min_checks,
            "expected at least {min_checks} elements checked, got {checked}"
        );
    }

    #[test]
    fn node_id_mapping_full_document() {
        assert_node_id_alignment(
            r#"<html><head><title>T</title></head><body>
                <div id="a"><p id="b">x</p><span id="c">y</span></div>
                <button id="d">go</button>
            </body></html>"#,
            6,
        );
    }

    #[test]
    fn node_id_mapping_with_synthesized_skeleton() {
        // No explicit <html>/<head>/<body>; BrowserHost synthesizes them and
        // the alignment must still hold.
        assert_node_id_alignment(
            r#"<div id="root"><span id="inner">hi</span><a id="link">go</a></div>"#,
            4,
        );
    }

    #[test]
    fn node_id_mapping_deeply_nested() {
        assert_node_id_alignment(
            r#"<html><body><ul id="list">
                <li id="one"><a id="la">1</a></li>
                <li id="two"><a id="lb">2</a></li>
                <li id="three"><a id="lc">3</a></li>
            </ul></body></html>"#,
            8,
        );
    }

    /// Find the `data-tobira-node-id` of the first element with `attr == value`
    /// in an annotated tree (reproduces how the browser identifies a clicked
    /// node).
    fn find_node_id_by_attr(node: &crate::html::Node, attr: &str, value: &str) -> Option<usize> {
        if let crate::html::Node::Element(el) = node {
            if el.attributes.get(attr).map(String::as_str) == Some(value) {
                return el
                    .attributes
                    .get("data-tobira-node-id")
                    .and_then(|id| id.parse().ok());
            }
            for child in &el.children {
                if let Some(found) = find_node_id_by_attr(child, attr, value) {
                    return Some(found);
                }
            }
        }
        None
    }

    #[test]
    fn engine_session_dispatches_click_by_node_id() {
        use crate::browser::annotate_node_ids;
        use crate::html::parse_document;

        let html = r#"<html><body>
            <button id="btn">go</button>
            <div id="out">idle</div>
            <script>
                document.getElementById('btn').addEventListener('click', () => {
                    document.getElementById('out').textContent = 'clicked';
                });
            </script>
        </body></html>"#;
        let (mut session, initial) = EngineSession::start(html, "http://localhost/");
        assert!(initial.error.is_none(), "error: {:?}", initial.error);
        assert!(initial.html.contains(">idle</div>"));

        // Identify the button the way the browser does (annotate the snapshot).
        let mut tree = parse_document(&initial.html);
        annotate_node_ids(&mut tree);
        let btn_id = find_node_id_by_attr(&tree, "id", "btn").expect("button node id");

        // A click dispatched by that node id must reach the listener.
        let result = session.dispatch_event(btn_id, "click", &DomEventInit::default());
        assert!(
            result.html.contains(">clicked</div>"),
            "expected click to mutate the DOM, html: {}",
            result.html
        );
    }

    #[test]
    fn engine_session_snapshot_reflects_later_event() {
        use crate::browser::annotate_node_ids;
        use crate::html::parse_document;

        let html = r#"<html><body>
            <button id="btn">go</button>
            <ul id="log"></ul>
            <script>
                document.getElementById('btn').addEventListener('click', () => {
                    const li = document.createElement('li');
                    li.textContent = 'hit';
                    document.getElementById('log').appendChild(li);
                });
            </script>
        </body></html>"#;
        let (mut session, initial) = EngineSession::start(html, "http://localhost/");
        let mut tree = parse_document(&initial.html);
        annotate_node_ids(&mut tree);
        let btn_id = find_node_id_by_attr(&tree, "id", "btn").expect("button node id");

        // Two clicks should append two <li>; a plain snapshot reflects state.
        session.dispatch_event(btn_id, "click", &DomEventInit::default());
        let result = session.dispatch_event(btn_id, "click", &DomEventInit::default());
        assert_eq!(
            result.html.matches("<li>hit</li>").count(),
            2,
            "html: {}",
            result.html
        );
        let snap = session.snapshot();
        assert_eq!(snap.html.matches("<li>hit</li>").count(), 2);
    }

    #[test]
    fn intersection_observer_fires_on_scroll_into_view() {
        use crate::browser::annotate_node_ids;
        use crate::html::parse_document;

        let html = r#"<html><body>
            <div id="target">x</div>
            <div id="out">init</div>
            <script>
                const log = [];
                const io = new IntersectionObserver((entries) => {
                    for (const e of entries) {
                        log.push(e.isIntersecting ? 'in' : 'out');
                    }
                    document.getElementById('out').textContent = log.join(',');
                });
                io.observe(document.getElementById('target'));
            </script>
        </body></html>"#;
        let (mut session, initial) = EngineSession::start(html, "http://localhost/");
        assert!(initial.error.is_none(), "error: {:?}", initial.error);
        let mut tree = parse_document(&initial.html);
        annotate_node_ids(&mut tree);
        let target_id = find_node_id_by_attr(&tree, "id", "target").expect("target id");

        // Target far below the 720px viewport → first feed reports "out".
        session.set_geometry(&[(target_id, 0.0, 2000.0, 100.0, 50.0)]);
        let snap = session.snapshot();
        assert!(
            snap.html.contains(">out</div>"),
            "initial out-of-view not reported, html: {}",
            snap.html
        );

        // Scroll so the target enters the viewport → reports "in".
        session.set_scroll_position(1900);
        session.set_geometry(&[(target_id, 0.0, 2000.0, 100.0, 50.0)]);
        let snap = session.snapshot();
        assert!(
            snap.html.contains(">out,in</div>"),
            "scroll-into-view did not fire intersecting, html: {}",
            snap.html
        );

        // Scrolling back out reports "out" again (state-change only).
        session.set_scroll_position(0);
        session.set_geometry(&[(target_id, 0.0, 2000.0, 100.0, 50.0)]);
        let snap = session.snapshot();
        assert!(
            snap.html.contains(">out,in,out</div>"),
            "scroll-out did not fire, html: {}",
            snap.html
        );
    }

    #[test]
    fn get_bounding_client_rect_uses_fed_geometry() {
        use crate::browser::annotate_node_ids;
        use crate::html::parse_document;

        let html = r#"<html><body>
            <button id="btn">go</button>
            <div id="out">none</div>
            <script>
                document.getElementById('btn').addEventListener('click', () => {
                    const r = document.getElementById('btn').getBoundingClientRect();
                    document.getElementById('out').textContent =
                        r.x + ',' + r.y + ',' + r.width + ',' + r.height + ',' + r.right + ',' + r.bottom;
                });
            </script>
        </body></html>"#;
        let (mut session, initial) = EngineSession::start(html, "http://localhost/");
        assert!(initial.error.is_none(), "error: {:?}", initial.error);
        let mut tree = parse_document(&initial.html);
        annotate_node_ids(&mut tree);
        let btn_id = find_node_id_by_attr(&tree, "id", "btn").expect("button id");

        // Feed geometry the browser would compute from layout (document coords).
        session.set_geometry(&[(btn_id, 10.0, 20.0, 100.0, 40.0)]);

        let result = session.dispatch_event(btn_id, "click", &DomEventInit::default());
        assert!(
            result.html.contains(">10,20,100,40,110,60</div>"),
            "getBoundingClientRect did not reflect fed geometry, html: {}",
            result.html
        );
    }

    #[test]
    fn get_bounding_client_rect_subtracts_scroll() {
        use crate::browser::annotate_node_ids;
        use crate::html::parse_document;

        let html = r#"<html><body>
            <button id="btn">go</button>
            <div id="out">none</div>
            <script>
                document.getElementById('btn').addEventListener('click', () => {
                    const r = document.getElementById('btn').getBoundingClientRect();
                    document.getElementById('out').textContent = r.y + '';
                });
            </script>
        </body></html>"#;
        let (mut session, initial) = EngineSession::start(html, "http://localhost/");
        let mut tree = parse_document(&initial.html);
        annotate_node_ids(&mut tree);
        let btn_id = find_node_id_by_attr(&tree, "id", "btn").expect("button id");

        session.set_geometry(&[(btn_id, 0.0, 500.0, 100.0, 40.0)]);
        session.set_scroll_position(300); // viewport y = 500 - 300 = 200
        let result = session.dispatch_event(btn_id, "click", &DomEventInit::default());
        assert!(
            result.html.contains(">200</div>"),
            "getBoundingClientRect should subtract scroll, html: {}",
            result.html
        );
    }

    #[test]
    fn react_like_app_full_interactive_loop() {
        // End-to-end: a small "React-like" framework — element factory, component
        // functions, state, event delegation by bubbling to a root listener, and
        // re-render on click — driven through the real EngineSession dispatch path
        // the browser uses. Proves the engine can run framework-style apps.
        use crate::browser::annotate_node_ids;
        use crate::html::parse_document;

        let html = r#"<html><body>
            <div id="root"></div>
            <div id="status">none</div>
            <script>
                function h(tag, props, ...children) {
                    const el = document.createElement(tag);
                    const p = props || {};
                    for (const k of Object.keys(p)) {
                        if (k === 'onClick') el.addEventListener('click', p[k]);
                        else if (k === 'className') el.className = p[k];
                        else el.setAttribute(k, p[k]);
                    }
                    for (const child of children.flat()) {
                        el.appendChild(typeof child === 'object' ? child : document.createTextNode(String(child)));
                    }
                    return el;
                }
                const state = { count: 0 };
                const root = document.getElementById('root');
                function render() {
                    root.innerHTML = '';
                    root.appendChild(
                        h('button', { id: 'inc', onClick: () => { state.count++; render(); } },
                          'count: ', state.count)
                    );
                }
                // Event delegation: one listener on the persistent root reacts to
                // clicks bubbling up from the (re-rendered) child button, and writes
                // to a status node OUTSIDE root so the effect is observable.
                root.addEventListener('click', (e) => {
                    if (e.target && e.target.tagName && e.target.tagName.toLowerCase() === 'button') {
                        document.getElementById('status').textContent =
                            'target=' + e.target.id + ' current=' + (e.currentTarget && e.currentTarget.id);
                    }
                });
                render();
            </script>
        </body></html>"#;

        let (mut session, initial) = EngineSession::start(html, "http://localhost/");
        assert!(initial.error.is_none(), "init error: {:?}", initial.error);
        assert!(
            initial.html.contains("count: 0"),
            "initial render missing, html: {}",
            initial.html
        );

        // Click the increment button (found the way the browser locates it).
        let find_inc = |html: &str| {
            let mut tree = parse_document(html);
            annotate_node_ids(&mut tree);
            find_node_id_by_attr(&tree, "id", "inc").expect("inc button id")
        };

        let click = DomEventInit {
            bubbles: true,
            cancelable: true,
            ..Default::default()
        };
        let inc_id = find_inc(&initial.html);
        let after1 = session.dispatch_event(inc_id, "click", &click);
        assert!(after1.error.is_none(), "click1 error: {:?}", after1.error);
        // The button's own handler incremented + re-rendered.
        assert!(
            after1.html.contains("count: 1"),
            "count did not increment, html: {}",
            after1.html
        );
        // The click bubbled to root's delegated listener: target = the button,
        // currentTarget = root.
        assert!(
            after1.html.contains("target=inc current=root"),
            "delegated bubble handler did not run, html: {}",
            after1.html
        );

        // A second click on the freshly rendered button keeps working.
        let inc_id2 = find_inc(&after1.html);
        let after2 = session.dispatch_event(inc_id2, "click", &click);
        assert!(
            after2.html.contains("count: 2"),
            "second click failed, html: {}",
            after2.html
        );
    }

    #[test]
    fn mutation_observer_reports_childlist() {
        // A childList MutationObserver delivers added nodes; the callback runs at
        // the microtask checkpoint (before the initial snapshot).
        let html = r#"<html><body>
            <div id="target"></div>
            <div id="out">none</div>
            <script>
                const obs = new MutationObserver((records) => {
                    const r = records[0];
                    document.getElementById('out').textContent =
                        'n=' + records.length + ' type=' + r.type +
                        ' added=' + r.addedNodes.length;
                });
                obs.observe(document.getElementById('target'), { childList: true });
                const el = document.createElement('span');
                document.getElementById('target').appendChild(el);
            </script>
        </body></html>"#;
        let (_session, initial) = EngineSession::start(html, "http://localhost/");
        assert!(initial.error.is_none(), "error: {:?}", initial.error);
        assert!(
            initial.html.contains(">n=1 type=childList added=1</div>"),
            "observer callback did not deliver childList record, html: {}",
            initial.html
        );
    }

    #[test]
    fn mutation_observer_reports_attributes_with_old_value() {
        let html = r#"<html><body>
            <div id="target"></div>
            <div id="out">none</div>
            <script>
                const obs = new MutationObserver((records) => {
                    const r = records[records.length - 1];
                    document.getElementById('out').textContent =
                        r.type + ':' + r.attributeName + ':' + r.oldValue;
                });
                const t = document.getElementById('target');
                obs.observe(t, { attributes: true, attributeOldValue: true });
                t.setAttribute('data-x', 'one');
                t.setAttribute('data-x', 'two');
            </script>
        </body></html>"#;
        let (_session, initial) = EngineSession::start(html, "http://localhost/");
        assert!(initial.error.is_none(), "error: {:?}", initial.error);
        // Last record: data-x changed from 'one' to 'two'.
        assert!(
            initial.html.contains(">attributes:data-x:one</div>"),
            "observer did not report attribute oldValue, html: {}",
            initial.html
        );
    }

    #[test]
    fn mutation_observer_disconnect_stops_delivery() {
        use crate::browser::annotate_node_ids;
        use crate::html::parse_document;

        let html = r#"<html><body>
            <button id="btn">go</button>
            <div id="target"></div>
            <div id="out">start</div>
            <script>
                let hits = 0;
                const obs = new MutationObserver((records) => {
                    hits += records.length;
                    document.getElementById('out').textContent = 'hits=' + hits;
                });
                obs.observe(document.getElementById('target'), { childList: true });
                document.getElementById('btn').addEventListener('click', () => {
                    obs.disconnect();
                    document.getElementById('target').appendChild(document.createElement('i'));
                    document.getElementById('out').textContent = 'after-disconnect';
                });
                // First mutation (observed) fires the callback at the checkpoint.
                document.getElementById('target').appendChild(document.createElement('b'));
            </script>
        </body></html>"#;
        let (mut session, initial) = EngineSession::start(html, "http://localhost/");
        assert!(initial.error.is_none(), "error: {:?}", initial.error);
        assert!(
            initial.html.contains(">hits=1</div>"),
            "first observed mutation should deliver, html: {}",
            initial.html
        );

        // After disconnect, the mutation in the click handler must NOT fire it.
        let mut tree = parse_document(&initial.html);
        annotate_node_ids(&mut tree);
        let btn_id = find_node_id_by_attr(&tree, "id", "btn").expect("button id");
        let after = session.dispatch_event(btn_id, "click", &DomEventInit::default());
        assert!(
            after.html.contains(">after-disconnect</div>"),
            "click handler should have run, html: {}",
            after.html
        );
        assert!(
            !after.html.contains(">hits=2</div>"),
            "disconnected observer must not deliver further records, html: {}",
            after.html
        );
    }

    #[test]
    fn demo_page_runs_on_the_engine() {
        use crate::browser::annotate_node_ids;
        use crate::html::parse_document;

        // The interactive verification demo must actually work on the engine
        // (so the user's GUI check only has to confirm the render path).
        let html = include_str!("../demo/engine-demo.html");
        let (mut session, initial) = EngineSession::start(html, "http://localhost:8000/");
        assert!(initial.error.is_none(), "demo error: {:?}", initial.error);

        // Initial script ran (banner flipped) and async settled before snapshot.
        assert!(
            initial.html.contains("初期スクリプト実行 OK"),
            "engine-status banner not updated"
        );
        assert!(
            initial.html.contains("Promise.then"),
            "async settling did not run before snapshot"
        );

        // A click on the counter increments it.
        let mut tree = parse_document(&initial.html);
        annotate_node_ids(&mut tree);
        let inc_id = find_node_id_by_attr(&tree, "id", "inc").expect("counter button id");
        let after = session.dispatch_event(inc_id, "click", &DomEventInit::default());
        assert!(
            after.html.contains(r#"id="count" class="count">1<"#)
                || after.html.contains(">1</span>"),
            "counter did not increment, html: {}",
            after.html
        );
    }

    #[test]
    fn engine_session_preserves_scroll_position() {
        // Regression: the engine snapshot must report the tracked scroll offset,
        // not 0, or the browser resets scroll to the top on every event.
        let html = r#"<html><body><div>tall page</div></body></html>"#;
        let (mut session, initial) = EngineSession::start(html, "http://localhost/");
        assert_eq!(initial.scroll_y, 0);
        session.set_scroll_position(500);
        assert_eq!(session.snapshot().scroll_y, 500);
        // No `resize` listener → the global dispatch is a no-op (None), so the
        // browser won't apply a snapshot or relayout for it.
        assert!(session.dispatch_global_event("resize").is_none());
        // Scroll is still preserved.
        assert_eq!(session.snapshot().scroll_y, 500);
    }

    #[test]
    fn engine_session_skips_global_event_without_listener() {
        // Regression for the scroll/white-screen bug: a global event with no
        // listener returns None so the browser skips the relayout that was
        // breaking scrolling and clicks.
        let html = r#"<html><body><p>no listeners here</p></body></html>"#;
        let (mut session, _) = EngineSession::start(html, "http://localhost/");
        assert!(session.dispatch_global_event("scroll").is_none());
        assert!(session.dispatch_global_event("resize").is_none());
    }

    #[test]
    fn engine_session_exposes_scroll_to_window_scrolly() {
        let html = r#"<html><body><p id="out">start</p>
            <script>
                window.addEventListener('scroll', () => {
                    document.getElementById('out').textContent = 'y=' + window.scrollY;
                });
            </script></body></html>"#;
        let (mut session, _) = EngineSession::start(html, "http://localhost/");
        session.set_scroll_position(250);
        // There IS a scroll listener, so the dispatch fires and snapshots.
        let result = session
            .dispatch_global_event("scroll")
            .expect("scroll listener should fire");
        assert_eq!(result.scroll_y, 250);
        assert!(
            result.html.contains(">y=250</p>"),
            "window.scrollY not reflected, html: {}",
            result.html
        );
    }

    #[test]
    fn engine_session_keyboard_event_carries_key_and_modifiers() {
        use crate::browser::annotate_node_ids;
        use crate::html::parse_document;

        let html = r#"<html><body><input id="f">
            <p id="out">none</p>
            <script>
                document.getElementById('f').addEventListener('keydown', (e) => {
                    document.getElementById('out').textContent =
                        'key=' + e.key + ' shift=' + e.shiftKey;
                });
            </script></body></html>"#;
        let (mut session, initial) = EngineSession::start(html, "http://localhost/");
        let mut tree = parse_document(&initial.html);
        annotate_node_ids(&mut tree);
        let id = find_node_id_by_attr(&tree, "id", "f").expect("input node id");

        let init = DomEventInit {
            key: Some("a".to_string()),
            shift_key: true,
            cancelable: true,
            bubbles: true,
            ..DomEventInit::default()
        };
        let result = session.dispatch_event(id, "keydown", &init);
        assert!(
            result.html.contains(">key=a shift=true</p>"),
            "event detail not delivered, html: {}",
            result.html
        );
    }

    #[test]
    fn engine_session_input_value_reflects_set_attribute() {
        use crate::browser::annotate_node_ids;
        use crate::html::parse_document;

        // The browser syncs the typed value via set_attribute("value", ...)
        // BEFORE firing the input event; the listener must then see it.
        let html = r#"<html><body><input id="f">
            <p id="out">empty</p>
            <script>
                const f = document.getElementById('f');
                f.addEventListener('input', () => {
                    document.getElementById('out').textContent = 'val=' + f.value;
                });
            </script></body></html>"#;
        let (mut session, initial) = EngineSession::start(html, "http://localhost/");
        let mut tree = parse_document(&initial.html);
        annotate_node_ids(&mut tree);
        let id = find_node_id_by_attr(&tree, "id", "f").expect("input node id");

        session.set_attribute(id, "value", "hello");
        let init = DomEventInit {
            bubbles: true,
            input_type: Some("insertText".to_string()),
            data: Some("o".to_string()),
            ..DomEventInit::default()
        };
        let result = session.dispatch_event(id, "input", &init);
        assert!(
            result.html.contains(">val=hello</p>"),
            "input.value not reflected, html: {}",
            result.html
        );
    }

    /// `for await...of` over a sync iterable awaits each element. Covers the
    /// common shapes (promise sequences, plain values, empty, break/continue,
    /// an await in the body, and a rejected element propagating as a throw).
    #[test]
    fn supports_for_await_of_over_sync_iterables() {
        let cases: &[(&str, &str, &str)] = &[
            ("promises", "async function f(){let s=0; for await (const x of [Promise.resolve(1),Promise.resolve(2)]) s+=x; return s}", "3"),
            ("plain", "async function f(){let o=''; for await (const x of [1,2,3]) o+=x; return o}", "123"),
            ("empty", "async function f(){let n=0; for await (const x of []) n++; return n}", "0"),
            ("break", "async function f(){let s=0; for await (const x of [1,2,3,4]){ if(x===3) break; s+=x; } return s}", "3"),
            ("continue", "async function f(){let s=0; for await (const x of [1,2,3,4]){ if(x%2===0) continue; s+=x; } return s}", "4"),
            ("await-in-body", "async function f(){let s=0; for await (const x of [10,20]){ const y=await Promise.resolve(x); s+=y; } return s}", "30"),
            ("reject", "async function f(){try{ for await (const x of [Promise.reject('boom')]) {} }catch(e){return 'caught:'+e} return 'no'}", "caught:boom"),
        ];
        for (name, body, expected) in cases {
            let html = format!(
                "<html><body><script>{body}\nf().then(r=>{{ document.title = String(r); }});</script></body></html>"
            );
            let result = run_document_scripts(&html, "http://localhost/");
            assert!(result.error.is_none(), "[{name}] error: {:?}", result.error);
            assert_eq!(
                result.title.as_deref(),
                Some(*expected),
                "[{name}] wrong result"
            );
        }
    }

    #[test]
    fn engine_session_keydown_preventdefault_is_reported() {
        use crate::browser::annotate_node_ids;
        use crate::html::parse_document;

        let html = r#"<html><body><input id="f"><script>
            document.getElementById('f').addEventListener('keydown', (e) => {
                e.preventDefault();
            });
        </script></body></html>"#;
        let (mut session, initial) = EngineSession::start(html, "http://localhost/");
        let mut tree = parse_document(&initial.html);
        annotate_node_ids(&mut tree);
        let id = find_node_id_by_attr(&tree, "id", "f").expect("input node id");

        let init = DomEventInit {
            key: Some("a".to_string()),
            cancelable: true,
            bubbles: true,
            ..DomEventInit::default()
        };
        let result = session.dispatch_event(id, "keydown", &init);
        assert!(
            result.default_prevented,
            "preventDefault() on keydown should be reported back"
        );
    }

    #[test]
    fn engine_session_unknown_node_id_is_safe() {
        let html = r#"<html><body><div id="x">ok</div></body></html>"#;
        let (mut session, _) = EngineSession::start(html, "http://localhost/");
        let result = session.dispatch_event(99999, "click", &DomEventInit::default());
        assert!(result.error.is_none());
        assert!(result.html.contains(">ok</div>"));
    }

    #[test]
    fn dom_query_and_text_mutation() {
        let result = run_document_scripts(
            r#"<html><body>
                <div id="app">old</div>
                <script>
                    const el = document.getElementById('app');
                    el.textContent = 'new';
                    console.log(el.textContent);
                </script>
            </body></html>"#,
            "http://localhost/",
        );
        assert!(result.error.is_none(), "error: {:?}", result.error);
        assert_eq!(result.console_logs, vec!["new".to_string()]);
        assert!(
            result.html.contains(">new</div>"),
            "serialized html should reflect the mutation: {}",
            result.html
        );
        assert!(!result.html.contains(">old</div>"));
    }

    #[test]
    fn create_and_append_element() {
        let result = run_document_scripts(
            r#"<html><body><ul id="list"></ul><script>
                const ul = document.getElementById('list');
                const li = document.createElement('li');
                li.textContent = 'item';
                li.setAttribute('class', 'row');
                ul.appendChild(li);
            </script></body></html>"#,
            "http://localhost/",
        );
        assert!(result.error.is_none(), "error: {:?}", result.error);
        assert!(
            result.html.contains(r#"<li class="row">item</li>"#),
            "html: {}",
            result.html
        );
    }

    #[test]
    fn query_selector_class_and_attr() {
        let result = run_document_scripts(
            r#"<html><body>
                <p class="note">a</p><p class="note">b</p>
                <script>
                    console.log(document.querySelectorAll('.note').length);
                    console.log(document.querySelector('p.note') !== null);
                </script>
            </body></html>"#,
            "http://localhost/",
        );
        assert!(result.error.is_none(), "error: {:?}", result.error);
        assert_eq!(result.console_logs, vec!["2".to_string(), "true".to_string()]);
    }

    #[test]
    fn title_and_location() {
        let result = run_document_scripts(
            r#"<html><head><title>Hi</title></head><body><script>
                console.log(location.protocol);
                console.log(location.hostname);
            </script></body></html>"#,
            "https://example.com/path",
        );
        assert!(result.error.is_none(), "error: {:?}", result.error);
        assert_eq!(result.title, Some("Hi".to_string()));
        assert_eq!(
            result.console_logs,
            vec!["https:".to_string(), "example.com".to_string()]
        );
    }

    #[test]
    fn engine_runs_real_language_features() {
        // Confirms the page scripts run on the self-built engine (closures,
        // array methods, template literals, etc.), not just trivial DOM ops.
        let result = run_document_scripts(
            r#"<html><body><script>
                const nums = [1, 2, 3, 4];
                const sum = nums.filter(n => n % 2 === 0).reduce((a, b) => a + b, 0);
                console.log(`sum=${sum}`);
            </script></body></html>"#,
            "http://localhost/",
        );
        assert!(result.error.is_none(), "error: {:?}", result.error);
        assert_eq!(result.console_logs, vec!["sum=6".to_string()]);
    }

    // Empirical "does real React run on our engine" probe. Loads the React 18 UMD
    // production bundles from tests/fixtures/react and renders a component against
    // the real DOM host. Ignored by default (still failing while we close gaps);
    // run with: cargo test --bin tobira -- --ignored react_umd --nocapture
    /// Real React 18 interactivity: a useState counter with an onClick handler.
    /// Clicking the button (via `el.click()`, which now dispatches a bubbling
    /// click that React's delegated root listener catches) must update state and
    /// re-render the DOM. This is the end-to-end "React works as a framework"
    /// proof, not just a static render.
    #[test]
    fn react_umd_usestate_onclick_rerenders() {
        let react = std::fs::read_to_string("tests/fixtures/react/react.production.min.js")
            .expect("react fixture present");
        let react_dom =
            std::fs::read_to_string("tests/fixtures/react/react-dom.production.min.js")
                .expect("react-dom fixture present");

        let html = format!(
            "<html><body><div id=\"root\"></div>\
             <script>{react}</script>\
             <script>{react_dom}</script>\
             <script>\
               var e = React.createElement;\
               function Counter() {{\
                 var s = React.useState(0); var n = s[0]; var setN = s[1];\
                 return e('button', {{ id: 'btn', onClick: function(){{ setN(n + 1); }} }},\
                          'count: ' + n);\
               }}\
               try {{\
                 ReactDOM.createRoot(document.getElementById('root')).render(e(Counter));\
                 console.log('RENDER_CALLED');\
               }} catch (err) {{ console.log('THREW: ' + (err && err.message ? err.message : err)); }}\
             </script></body></html>"
        );

        let (mut session, initial) = EngineSession::start(&html, "http://localhost/");
        assert!(initial.error.is_none(), "engine error: {:?}", initial.error);
        let pump = |session: &mut EngineSession| {
            let mut now = 0u64;
            for _ in 0..300 {
                if !session.has_pending_work() {
                    break;
                }
                now += 16;
                session.pump(now);
            }
        };
        pump(&mut session);
        let after_mount = session.snapshot();
        assert!(
            after_mount.html.contains("count: 0"),
            "initial render missing 'count: 0'; html={}",
            after_mount.html
        );

        // Click the button: dispatch through the engine (bubbles to React's root
        // delegated listener), then flush the scheduled re-render.
        let click = session.eval_for_test("document.getElementById('btn').click();");
        assert!(click.error.is_none(), "click error: {:?}", click.error);
        pump(&mut session);
        let after_click = session.snapshot();
        assert!(
            after_click.html.contains("count: 1"),
            "state did not update after click; html={}",
            after_click.html
        );
    }

    /// Real React 18 with useEffect (re-run on dep change), keyed list rendering
    /// that grows on click, and conditional rendering toggled on click. Exercises
    /// the scheduler-driven passive-effect flush (relies on the MessageChannel
    /// polyfill) end-to-end against the production bundle.
    #[test]
    fn react_umd_effects_lists_conditional() {
        let react = std::fs::read_to_string("tests/fixtures/react/react.production.min.js")
            .expect("react fixture present");
        let react_dom =
            std::fs::read_to_string("tests/fixtures/react/react-dom.production.min.js")
                .expect("react-dom fixture present");
        let app = r#"
            var e = React.createElement;
            var useState = React.useState, useEffect = React.useEffect;
            function App() {
              var s = useState(['a','b']); var items = s[0]; var setItems = s[1];
              var t = useState(false); var shown = t[0]; var setShown = t[1];
              useEffect(function(){ console.log('EFFECT len=' + items.length); }, [items]);
              return e('div', null,
                e('button', { id:'add', onClick:function(){ setItems(items.concat('x')); } }, 'add'),
                e('button', { id:'toggle', onClick:function(){ setShown(!shown); } }, 'toggle'),
                shown ? e('p', { id:'panel' }, 'PANEL') : null,
                e('ul', { id:'list' }, items.map(function(it, i){ return e('li', { key:i }, it); }))
              );
            }
            ReactDOM.createRoot(document.getElementById('root')).render(e(App));
        "#;
        let html = format!(
            "<html><body><div id=\"root\"></div><script>{react}</script><script>{react_dom}</script><script>{app}</script></body></html>"
        );
        let (mut session, initial) = EngineSession::start(&html, "http://localhost/");
        assert!(initial.error.is_none(), "engine error: {:?}", initial.error);
        let pump = |s: &mut EngineSession| {
            let mut now = 0u64;
            for _ in 0..200 { if !s.has_pending_work() { break; } now += 16; s.pump(now); }
        };
        pump(&mut session);
        let mount = session.snapshot();
        assert!(
            mount.html.contains("<li>a</li><li>b</li>"),
            "initial list wrong: {}",
            mount.html
        );

        // Add an item; the effect must re-run (dep changed) and the list must grow.
        let add = session.eval_for_test("document.getElementById('add').click();");
        assert!(add.error.is_none(), "add error: {:?}", add.error);
        pump(&mut session);
        assert!(
            add.console_logs.iter().any(|l| l == "EFFECT len=3"),
            "useEffect did not re-run on update; logs: {:?}",
            add.console_logs
        );
        let after_add = session.snapshot();
        assert!(
            after_add.html.contains("<li>a</li><li>b</li><li>x</li>"),
            "list did not grow after click: {}",
            after_add.html
        );

        // Toggle conditional rendering: the panel appears.
        let toggle = session.eval_for_test("document.getElementById('toggle').click();");
        assert!(toggle.error.is_none(), "toggle error: {:?}", toggle.error);
        pump(&mut session);
        assert!(
            session.snapshot().html.contains("id=\"panel\""),
            "conditional panel did not render after toggle"
        );
    }

    /// React 18 renders a controlled `<input>` (value bound to state) plus a
    /// sibling. This used to crash the whole render: React's input value-tracker
    /// reads `node.constructor.prototype` and calls `node.hasOwnProperty(...)`,
    /// which host DOM nodes didn't support ("attempted to call a non-function
    /// value"). Now inputs mount with their bound value.
    #[test]
    fn react_umd_controlled_input_mounts() {
        let react = std::fs::read_to_string("tests/fixtures/react/react.production.min.js")
            .expect("react fixture present");
        let react_dom =
            std::fs::read_to_string("tests/fixtures/react/react-dom.production.min.js")
                .expect("react-dom fixture present");
        let app = r#"
            var e = React.createElement;
            function Form() {
              var s = React.useState('hi'); var val = s[0];
              return e('div', null,
                e('input', { id:'in', value: val, readOnly: true }),
                e('span', { id:'echo' }, 'echo:' + val)
              );
            }
            ReactDOM.createRoot(document.getElementById('root')).render(e(Form));
        "#;
        let html = format!(
            "<html><body><div id=\"root\"></div><script>{react}</script><script>{react_dom}</script><script>{app}</script></body></html>"
        );
        let (mut session, initial) = EngineSession::start(&html, "http://localhost/");
        assert!(initial.error.is_none(), "engine error: {:?}", initial.error);
        let mut now = 0u64;
        for _ in 0..200 { if !session.has_pending_work() { break; } now += 16; session.pump(now); }
        let snap = session.snapshot();
        assert!(
            snap.html.contains("id=\"in\""),
            "controlled <input> did not mount: {}",
            snap.html
        );
        assert!(
            snap.html.contains("echo:hi"),
            "state-bound sibling did not render: {}",
            snap.html
        );
    }

    /// Real React 18 controlled input: typing fires onChange, which updates state
    /// and re-renders. Exercises React's native-input event path end-to-end (the
    /// `'oninput' in document` feature-detect must read true, else React falls
    /// back to an IE polyfill and onChange never fires).
    #[test]
    fn react_umd_controlled_input_onchange() {
        let react = std::fs::read_to_string("tests/fixtures/react/react.production.min.js")
            .expect("react fixture present");
        let react_dom =
            std::fs::read_to_string("tests/fixtures/react/react-dom.production.min.js")
                .expect("react-dom fixture present");
        let app = r#"
            var e = React.createElement;
            function Form() {
              var s = React.useState(''); var val = s[0]; var setVal = s[1];
              return e('div', null,
                e('input', { id:'in', value: val,
                    onChange: function(ev){ setVal(ev.target.value); } }),
                e('span', { id:'echo' }, 'echo:' + val)
              );
            }
            ReactDOM.createRoot(document.getElementById('root')).render(e(Form));
        "#;
        let html = format!(
            "<html><body><div id=\"root\"></div><script>{react}</script><script>{react_dom}</script><script>{app}</script></body></html>"
        );
        let (mut session, initial) = EngineSession::start(&html, "http://localhost/");
        assert!(initial.error.is_none(), "engine error: {:?}", initial.error);
        let pump = |s: &mut EngineSession| { let mut now=0u64; for _ in 0..200 { if !s.has_pending_work(){break;} now+=16; s.pump(now); } };
        pump(&mut session);
        assert!(session.snapshot().html.contains("echo:"), "form did not mount");

        // Type into the input: set value + fire a bubbling 'input' event, the way a
        // real keystroke does. React's onChange should update the bound state.
        let typed = session.eval_for_test(
            "var el=document.getElementById('in'); el.value='hello'; \
             el.dispatchEvent(new Event('input', { bubbles:true }));",
        );
        assert!(typed.error.is_none(), "type error: {:?}", typed.error);
        pump(&mut session);
        assert!(
            session.snapshot().html.contains("echo:hello"),
            "controlled onChange did not update state; html={}",
            session.snapshot().html
        );
    }

    #[test]
    #[ignore]
    fn react_umd_form_diag() {
        let react = std::fs::read_to_string("tests/fixtures/react/react.production.min.js")
            .expect("react fixture present");
        let react_dom =
            std::fs::read_to_string("tests/fixtures/react/react-dom.production.min.js")
                .expect("react-dom fixture present");
        let app = r#"
            var e = React.createElement;
            var useState = React.useState;
            function Form() {
              var s = useState(''); var val = s[0]; var setVal = s[1];
              return e('div', null,
                e('input', { id:'in', value: val,
                    onClick: function(){ console.log('INPUT_CLICK'); },
                    onInput: function(ev){ console.log('ONINPUT ' + ev.target.value); },
                    onChange: function(ev){ console.log('CHANGE ' + ev.target.value); setVal(ev.target.value); } }),
                e('span', { id:'echo' }, 'echo:' + val)
              );
            }
            ReactDOM.createRoot(document.getElementById('root')).render(e(Form));
            console.log('RENDER_OK');
        "#;
        let html = format!(
            "<html><body><div id=\"root\"></div><script>{react}</script><script>{react_dom}</script><script>{app}</script></body></html>"
        );
        let (mut session, initial) = EngineSession::start(&html, "http://localhost/");
        println!("INIT_ERR: {:?}", initial.error);
        let pump = |s: &mut EngineSession| { let mut now=0u64; for _ in 0..200 { if !s.has_pending_work(){break;} now+=16; s.pump(now); } };
        pump(&mut session);
        let show = |s: &mut EngineSession, label: &str| {
            let snap = s.snapshot();
            if let Some(i) = snap.html.find("id=\"echo\"") { let end=(i+60).min(snap.html.len()); println!("{label} ECHO: {}", &snap.html[i..end]); }
            if let Some(i) = snap.html.find("id=\"in\"") { let end=(i+80).min(snap.html.len()); println!("{label} INPUT: {}", &snap.html[i..end]); }
        };
        {
            let snap = session.snapshot();
            println!("INIT_LOGS: {:?}", initial.console_logs);
            if let Some(i) = snap.html.find("id=\"root\"") { let end=(i+200).min(snap.html.len()); println!("ROOT: {}", &snap.html[i..end]); }
        }
        show(&mut session, "MOUNT");
        // Simulate typing: set value + dispatch a bubbling input event (React maps
        // onChange to the 'input' event).
        let r = session.eval_for_test(
            "var el=document.getElementById('in'); \
             console.log('el.type=' + el.type + ' el.nodeName=' + el.nodeName); \
             var keys = Object.keys(el).filter(function(k){ return k.indexOf('__react')===0; }); \
             console.log('react_keys=' + keys.join(',')); \
             console.log('tracker=' + (el._valueTracker !== undefined && el._valueTracker !== null)); \
             console.log('hasOwn_value=' + el.hasOwnProperty('value')); \
             console.log('ctor_proto=' + (typeof el.constructor + '/' + (el.constructor && typeof el.constructor.prototype))); \
             el.value='hello'; \
             el.dispatchEvent(new Event('input', { bubbles:true }));",
        );
        println!("TYPE_ERR: {:?}", r.error);
        println!("TYPE_LOGS: {:?}", r.console_logs);
        pump(&mut session);
        show(&mut session, "AFTER_TYPE");
    }

    #[test]
    #[ignore]
    fn react_umd_complex_diag() {
        let react = std::fs::read_to_string("tests/fixtures/react/react.production.min.js")
            .expect("react fixture present");
        let react_dom =
            std::fs::read_to_string("tests/fixtures/react/react-dom.production.min.js")
                .expect("react-dom fixture present");
        // Exercise useEffect, list rendering with keys, conditional rendering, and
        // a multi-item state update driven by a click.
        let app = r#"
            var e = React.createElement;
            var useState = React.useState, useEffect = React.useEffect;
            function App() {
              var s = useState(['a','b']); var items = s[0]; var setItems = s[1];
              var t = useState(false); var shown = t[0]; var setShown = t[1];
              useEffect(function(){ console.log('EFFECT items=' + items.length); }, [items]);
              useEffect(function(){ console.log('EFFECT_EVERY items=' + items.length); });
              return e('div', null,
                e('button', { id:'add', onClick:function(){ setItems(items.concat('x')); } }, 'add'),
                e('button', { id:'toggle', onClick:function(){ setShown(!shown); } }, 'toggle'),
                shown ? e('p', { id:'panel' }, 'PANEL') : null,
                e('ul', { id:'list' }, items.map(function(it, i){ return e('li', { key: i }, it); }))
              );
            }
            ReactDOM.createRoot(document.getElementById('root')).render(e(App));
        "#;
        let html = format!(
            "<html><body><div id=\"root\"></div><script>{react}</script><script>{react_dom}</script><script>{app}</script></body></html>"
        );
        let (mut session, initial) = EngineSession::start(&html, "http://localhost/");
        println!("INIT_ERR: {:?}", initial.error);
        println!("INIT_LOGS: {:?}", initial.console_logs);
        let mc = session.eval_for_test("console.log('MC=' + typeof MessageChannel + ' perf=' + typeof performance);");
        println!("MC_LOGS: {:?}", mc.console_logs);
        let pump = |s: &mut EngineSession| {
            let mut now = 0u64;
            for _ in 0..200 { if !s.has_pending_work() { break; } now += 16; s.pump(now); }
        };
        pump(&mut session);
        let m = session.snapshot();
        println!("AFTER_MOUNT_LOGS: {:?}", m.console_logs);
        let show_list = |s: &mut EngineSession, label: &str| {
            let snap = s.snapshot();
            if let Some(i) = snap.html.find("id=\"list\"") {
                let end = (i + 120).min(snap.html.len());
                println!("{label} LIST: {}", &snap.html[i..end]);
            }
            println!("{label} HAS_PANEL: {}", snap.html.contains("id=\"panel\""));
        };
        show_list(&mut session, "MOUNT");
        let r1 = session.eval_for_test("document.getElementById('add').click();");
        println!("ADD_ERR: {:?}", r1.error);
        println!("ADD_IMMEDIATE_LOGS: {:?}", r1.console_logs);
        println!("PENDING_AFTER_ADD: {}", session.has_pending_work());
        pump(&mut session);
        println!("ADD_LOGS: {:?}", session.snapshot().console_logs);
        show_list(&mut session, "AFTER_ADD");
        let r2 = session.eval_for_test("document.getElementById('toggle').click();");
        println!("TOGGLE_ERR: {:?}", r2.error);
        pump(&mut session);
        show_list(&mut session, "AFTER_TOGGLE");
    }

    #[test]
    #[ignore]
    fn react_umd_click_diag() {
        let react = std::fs::read_to_string("tests/fixtures/react/react.production.min.js")
            .expect("react fixture present");
        let react_dom =
            std::fs::read_to_string("tests/fixtures/react/react-dom.production.min.js")
                .expect("react-dom fixture present");
        let html = format!(
            "<html><body><div id=\"root\"></div>\
             <script>{react}</script>\
             <script>{react_dom}</script>\
             <script>\
               var e = React.createElement;\
               function Counter() {{\
                 var s = React.useState(0); var n = s[0]; var setN = s[1];\
                 console.log('RENDER n=' + n);\
                 return e('button', {{ id: 'btn', onClick: function(){{ console.log('HANDLER_FIRED'); setN(n+1); }} }}, 'count: ' + n);\
               }}\
               ReactDOM.createRoot(document.getElementById('root')).render(e(Counter));\
               console.log('btn_exists=' + (document.getElementById('btn') !== null));\
             </script></body></html>"
        );
        let (mut session, initial) = EngineSession::start(&html, "http://localhost/");
        println!("INIT_ERR: {:?}", initial.error);
        println!("INIT_LOGS: {:?}", initial.console_logs);
        let mut now = 0u64;
        for _ in 0..50 { if !session.has_pending_work() { break; } now += 16; session.pump(now); }
        // manual listener probe + click
        let r1 = session.eval_for_test(
            "var b=document.getElementById('btn'); \
             console.log('have_btn=' + (b!==null)); \
             b.addEventListener('click', function(){ console.log('OWN_LISTENER'); }); \
             b.click();",
        );
        println!("CLICK_ERR: {:?}", r1.error);
        println!("CLICK_LOGS: {:?}", r1.console_logs);
        println!("PENDING_AFTER_CLICK: {}", session.has_pending_work());
        let mut now = 0u64;
        for _ in 0..50 { if !session.has_pending_work() { break; } now += 16; session.pump(now); }
        let snap = session.snapshot();
        println!("AFTER_LOGS: {:?}", snap.console_logs);
        if let Some(i) = snap.html.find("id=\"root\"") {
            let end = (i + 90).min(snap.html.len());
            println!("ROOT: {}", &snap.html[i..end]);
        }
    }

    #[test]
    #[ignore]
    fn react_umd_mount_diag() {
        let react = std::fs::read_to_string("tests/fixtures/react/react.production.min.js")
            .expect("react fixture present");
        let react_dom =
            std::fs::read_to_string("tests/fixtures/react/react-dom.production.min.js")
                .expect("react-dom fixture present");
        // Try BOTH APIs to isolate concurrent-scheduler issues from DOM mount issues:
        // legacy ReactDOM.render (synchronous) vs createRoot().render (concurrent).
        let html = format!(
            "<html><body><div id=\"legacy\"></div><div id=\"concurrent\"></div>\
             <script>{react}</script>\
             <script>{react_dom}</script>\
             <script>\
               try {{\
                 ReactDOM.render(React.createElement('h1',{{id:'L'}},'Legacy'), document.getElementById('legacy'));\
                 console.log('LEGACY_CALLED');\
               }} catch(e) {{ console.log('LEGACY_THREW: ' + (e&&e.message?e.message:e)); }}\
               try {{\
                 var r = ReactDOM.createRoot(document.getElementById('concurrent'));\
                 r.render(React.createElement('h1',{{id:'C'}},'Concurrent'));\
                 console.log('CONCURRENT_CALLED');\
               }} catch(e) {{ console.log('CONCURRENT_THREW: ' + (e&&e.message?e.message:e)); }}\
             </script></body></html>"
        );
        let (mut session, initial) = EngineSession::start(&html, "http://localhost/");
        println!("INIT_ERROR: {:?}", initial.error);
        println!("INIT_LOGS: {:?}", initial.console_logs);
        println!("INIT_PENDING: {}", session.has_pending_work());
        let mut now = 0u64;
        for i in 0..300 {
            if !session.has_pending_work() {
                println!("quiescent after {i} pumps");
                break;
            }
            now += 16;
            session.pump(now);
        }
        let snap = session.snapshot();
        println!("FINAL_ERROR: {:?}", snap.error);
        println!("FINAL_LOGS: {:?}", snap.console_logs);
        println!("LEGACY_MOUNTED: {}", snap.html.contains("id=\"L\"") || snap.html.contains("Legacy</h1>"));
        println!("CONCURRENT_MOUNTED: {}", snap.html.contains("id=\"C\"") || snap.html.contains("Concurrent</h1>"));
        // Dump the two containers' regions.
        for marker in ["id=\"legacy\"", "id=\"concurrent\""] {
            if let Some(i) = snap.html.find(marker) {
                let end = (i + 120).min(snap.html.len());
                println!("REGION[{marker}]: {}", &snap.html[i..end]);
            }
        }
    }

    #[test]
    #[ignore]
    fn react_umd_global_diag() {
        let react = std::fs::read_to_string("tests/fixtures/react/react.production.min.js")
            .expect("react fixture present");
        let html = format!(
            "<html><body><script>\
               console.log('this=' + (typeof this));\
               console.log('self=' + (typeof self));\
               console.log('window=' + (typeof window));\
               console.log('globalThis=' + (typeof globalThis));\
             </script>\
             <script>{react}</script>\
             <script>\
               console.log('React=' + (typeof React));\
               console.log('window.React=' + (typeof window.React));\
               console.log('globalThis.React=' + (typeof globalThis.React));\
             </script></body></html>"
        );
        let result = run_document_scripts(&html, "http://localhost/");
        println!("ERROR: {:?}", result.error);
        println!("LOGS: {:?}", result.console_logs);
    }

    #[test]
    #[ignore]
    fn react_umd_dev_renders_into_dom() {
        // Dev builds are large (~1MB) and gitignored; download with:
        //   curl -sSL -o tests/fixtures/react/react.development.js \
        //     https://unpkg.com/react@18.3.1/umd/react.development.js
        //   curl -sSL -o tests/fixtures/react/react-dom.development.js \
        //     https://unpkg.com/react-dom@18.3.1/umd/react-dom.development.js
        let (react, react_dom) = match (
            std::fs::read_to_string("tests/fixtures/react/react.development.js"),
            std::fs::read_to_string("tests/fixtures/react/react-dom.development.js"),
        ) {
            (Ok(a), Ok(b)) => (a, b),
            _ => {
                println!("SKIP: react dev fixtures not present (gitignored)");
                return;
            }
        };
        let html = format!(
            "<html><body><div id=\"root\"></div>\
             <script>{react}</script>\
             <script>{react_dom}</script>\
             <script>\
               try {{\
                 var root = ReactDOM.createRoot(document.getElementById('root'));\
                 root.render(React.createElement('h1', {{ id: 'title' }}, 'Hello, Tobira'));\
                 console.log('RENDER_CALLED');\
               }} catch (e) {{ console.log('RENDER_THREW: ' + (e && e.message ? e.message : e)); }}\
             </script></body></html>"
        );
        let (mut session, initial) = EngineSession::start(&html, "http://localhost/");
        println!("INIT_ERROR: {:?}", initial.error);
        println!("INIT_LOGS: {:?}", initial.console_logs);
        let mut now = 0u64;
        for _ in 0..200 {
            if !session.has_pending_work() {
                break;
            }
            now += 16;
            session.pump(now);
        }
        let snap = session.snapshot();
        println!("ERROR: {:?}", snap.error);
        println!("LOGS: {:?}", snap.console_logs);
        println!("RENDERED_TITLE_TAG: {}", snap.html.contains("id=\"title\""));
        // Surface the region around #root so we can see what actually mounted.
        if let Some(i) = snap.html.find("id=\"root\"") {
            let start = i.saturating_sub(8);
            let end = (i + 160).min(snap.html.len());
            println!("ROOT_REGION: {}", &snap.html[start..end]);
        }
    }

    /// End-to-end proof that the REAL React 18 production bundle runs on our
    /// from-scratch engine and renders into our DOM. Uses the committed UMD
    /// fixtures (tests/fixtures/react/*.production.min.js). Not ignored — this is
    /// a real regression gate now that React works.
    #[test]
    fn react_umd_renders_into_dom() {
        let react = std::fs::read_to_string("tests/fixtures/react/react.production.min.js")
            .expect("react fixture present");
        let react_dom =
            std::fs::read_to_string("tests/fixtures/react/react-dom.production.min.js")
                .expect("react-dom fixture present");

        let html = format!(
            "<html><body><div id=\"root\"></div>\
             <script>{react}</script>\
             <script>{react_dom}</script>\
             <script>\
               try {{\
                 var root = ReactDOM.createRoot(document.getElementById('root'));\
                 root.render(\
                   React.createElement('h1', {{ id: 'title', className: 'greeting' }},\
                     'Hello, Tobira'));\
                 console.log('RENDER_CALLED');\
               }} catch (e) {{ console.log('RENDER_THREW: ' + (e && e.message ? e.message : e)); }}\
             </script></body></html>"
        );

        let (mut session, initial) = EngineSession::start(&html, "http://localhost/");
        assert!(initial.error.is_none(), "engine error: {:?}", initial.error);
        assert!(
            initial.console_logs.iter().any(|l| l == "RENDER_CALLED"),
            "render() did not complete cleanly; logs: {:?}",
            initial.console_logs
        );
        // Flush any scheduled work (React 18 may finish the mount asynchronously).
        let mut now = 0u64;
        for _ in 0..300 {
            if !session.has_pending_work() {
                break;
            }
            now += 16;
            session.pump(now);
        }
        let snap = session.snapshot();
        assert!(snap.error.is_none(), "engine error after pump: {:?}", snap.error);
        // The rendered <h1> (with React-applied id/class) must be inside #root.
        assert!(
            snap.html.contains("id=\"title\"") && snap.html.contains("Hello, Tobira"),
            "React did not render the element into the DOM. html={}",
            snap.html
        );
    }

    /// History API + location parity on the engine path: mirrors the boa-path
    /// tests in js.rs (navigation_target / soft_navigation_target / title) but
    /// drives the self-built engine via `run_document_scripts`. These are the
    /// Stage 4 cutover blockers; closing them lets the default flip to the engine.
    #[test]
    fn engine_history_and_location() {
        let run = |script: &str| {
            run_document_scripts(
                &format!("<html><body><script>{script}</script></body></html>"),
                "https://example.com/start",
            )
        };

        // location.href = full navigation (reload), resolved against the doc URL.
        let r = run("location.href = '/next?from=test';");
        assert_eq!(r.navigation_target.as_deref(), Some("https://example.com/next?from=test"), "err={:?}", r.error);

        // location.hash = soft navigation + hashchange.
        let r = run("window.addEventListener('hashchange', function(){ document.title = location.href + '|' + location.hash; }); location.hash = '#frag';");
        assert_eq!(r.soft_navigation_target.as_deref(), Some("https://example.com/start#frag"));
        assert!(r.navigation_target.is_none(), "hash change must not full-navigate");
        assert_eq!(r.title.as_deref(), Some("https://example.com/start#frag|#frag"));

        // history.pushState: soft nav, location updates, no reload.
        let r = run("history.pushState({ page: 1 }, '', '/next?from=test#frag'); document.title = location.href + '|' + location.hash;");
        assert_eq!(r.soft_navigation_target.as_deref(), Some("https://example.com/next?from=test#frag"));
        assert!(r.navigation_target.is_none());
        assert_eq!(r.title.as_deref(), Some("https://example.com/next?from=test#frag|#frag"));

        // pushState x2 + back: popstate fires, history.state restored.
        let r = run("window.addEventListener('popstate', function(){ document.title = location.href + '|' + String(history.state.page); }); history.pushState({page:1},'','/one'); history.pushState({page:2},'','/two'); history.back();");
        assert_eq!(r.soft_navigation_target.as_deref(), Some("https://example.com/one"));
        assert_eq!(r.title.as_deref(), Some("https://example.com/one|1"));

        // back then forward: lands on /two, history.length == 3.
        let r = run("history.pushState({},'','/one'); history.pushState({},'','/two'); history.back(); history.forward(); document.title = location.href + '|' + location.hash + '|' + String(history.length);");
        assert_eq!(r.soft_navigation_target.as_deref(), Some("https://example.com/two"));
        assert!(r.navigation_target.is_none());
        assert_eq!(r.title.as_deref(), Some("https://example.com/two||3"));

        // scroll restore across back/forward.
        let r = run("window.scrollTo(0,120); history.pushState({},'','/one'); window.scrollTo(0,240); history.pushState({},'','/two'); history.back(); var a = location.href + '|' + String(window.scrollY); history.back(); var b = location.href + '|' + String(window.scrollY); history.forward(); var c = location.href + '|' + String(window.scrollY); document.title = a + '||' + b + '||' + c;");
        assert_eq!(
            r.title.as_deref(),
            Some("https://example.com/one|240||https://example.com/start|120||https://example.com/one|240")
        );
    }

    /// End-to-end smoke test of the shipped demo (`demo/react-demo.html`). Reads
    /// the ACTUAL demo file, inlines its external `<script src>` bundles exactly as
    /// the engine would fetch them over HTTP, mounts it, and drives the three
    /// interactive widgets (counter / controlled input / keyed todo list) through
    /// real DOM events. Guards the demo against rot: if any feature the demo relies
    /// on regresses, this fails. Run with:
    ///   cargo test --bin tobira -- react_demo_file --nocapture
    #[test]
    fn react_demo_file_renders_and_is_interactive() {
        let dir = std::path::Path::new("demo");
        let raw = std::fs::read_to_string(dir.join("react-demo.html"))
            .expect("demo/react-demo.html present");
        // Inline external bundles in place of their <script src="./x"> tags, just
        // like the browser would after fetching them.
        let inline = |html: String, src: &str, file: &str| -> String {
            let bundle = std::fs::read_to_string(dir.join(file))
                .unwrap_or_else(|_| panic!("demo bundle {file} present"));
            let tag = format!("<script src=\"{src}\"></script>");
            assert!(html.contains(&tag), "demo missing expected tag: {tag}");
            html.replace(&tag, &format!("<script>{bundle}</script>"))
        };
        let html = inline(raw, "./react.production.min.js", "react.production.min.js");
        let html = inline(html, "./react-dom.production.min.js", "react-dom.production.min.js");

        let (mut session, initial) = EngineSession::start(&html, "http://localhost:8000/");
        assert!(initial.error.is_none(), "engine error: {:?}", initial.error);
        let pump = |s: &mut EngineSession| {
            let mut now = 0u64;
            for _ in 0..400 {
                if !s.has_pending_work() {
                    break;
                }
                now += 16;
                s.pump(now);
            }
        };
        pump(&mut session);

        // ① Mount: all three sections present, counter at 0, both seed todos shown.
        let mount = session.snapshot();
        assert!(mount.error.is_none(), "error after mount: {:?}", mount.error);
        println!("=== MOUNTED DOM (excerpt) ===\n{}", excerpt_root(&mount.html));
        assert!(mount.html.contains("カウンター"), "counter section missing: {}", mount.html);
        assert!(mount.html.contains("エンジンを書く"), "seed todo 1 missing");
        assert!(mount.html.contains("React を動かす"), "seed todo 2 missing");

        // ② Counter: the first +/− buttons live in the counter section. Click '＋'
        // twice via the first button whose text is '＋'.
        let inc = session.eval_for_test(
            "var bs=document.getElementsByTagName('button'); \
             for (var i=0;i<bs.length;i++){ if(bs[i].textContent==='＋'){ bs[i].click(); bs[i].click(); break; } }",
        );
        assert!(inc.error.is_none(), "increment error: {:?}", inc.error);
        pump(&mut session);
        let after_inc = session.snapshot();
        assert!(
            after_inc.html.contains(">2<") || after_inc.html.contains("class=\"count\">2"),
            "counter did not reach 2; html={}",
            excerpt_root(&after_inc.html)
        );
        println!("counter after two clicks: OK (=2)");

        // ③ Controlled input: type into the echo input (first input on the page),
        // React's onChange must update the echo line.
        let typed = session.eval_for_test(
            "var el=document.getElementsByTagName('input')[0]; \
             el.value='tobira'; el.dispatchEvent(new Event('input', { bubbles:true }));",
        );
        assert!(typed.error.is_none(), "type error: {:?}", typed.error);
        pump(&mut session);
        let after_type = session.snapshot();
        assert!(
            after_type.html.contains("echo: tobira"),
            "controlled input echo did not update; html={}",
            excerpt_root(&after_type.html)
        );
        println!("controlled input echo: OK (echo: tobira)");

        // ④ Todo list: type into the todo input (second input), click '追加'.
        let add = session.eval_for_test(
            "var ins=document.getElementsByTagName('input'); \
             ins[1].value='デモを作る'; ins[1].dispatchEvent(new Event('input', { bubbles:true })); \
             var bs=document.getElementsByTagName('button'); \
             for (var i=0;i<bs.length;i++){ if(bs[i].textContent==='追加'){ bs[i].click(); break; } }",
        );
        assert!(add.error.is_none(), "add-todo error: {:?}", add.error);
        pump(&mut session);
        let after_add = session.snapshot();
        assert!(
            after_add.html.contains("デモを作る"),
            "new todo not rendered; html={}",
            excerpt_root(&after_add.html)
        );
        println!("todo add: OK (デモを作る appended)");

        // ⑤ Delete the first todo via its '削除' button.
        let del = session.eval_for_test(
            "var bs=document.getElementsByTagName('button'); \
             for (var i=0;i<bs.length;i++){ if(bs[i].textContent==='削除'){ bs[i].click(); break; } }",
        );
        assert!(del.error.is_none(), "delete-todo error: {:?}", del.error);
        pump(&mut session);
        let after_del = session.snapshot();
        // Check the rendered #root subtree, not the full document: the demo's own
        // inline <script> source contains the seed-todo string literal, so a
        // whole-document `contains` would always see it.
        let del_root = excerpt_root(&after_del.html);
        assert!(
            !del_root.contains("エンジンを書く"),
            "first todo was not removed; root={del_root}"
        );
        println!("todo delete: OK (エンジンを書く removed)");
        println!("=== FINAL DOM (excerpt) ===\n{}", excerpt_root(&after_del.html));
    }

    /// Verifies the REAL external-`<script src>` load path end-to-end: fetches the
    /// demo page over HTTP and lets the engine resolve + fetch the React bundles
    /// itself (exactly the GUI/`TOBIRA_ENGINE` path), rather than pre-inlining them.
    /// Needs a static server serving `demo/`:
    ///   (cd demo && python -m http.server 8000)
    ///   cargo test --bin tobira react_demo_external_src -- --ignored --nocapture
    #[test]
    #[ignore]
    fn react_demo_external_src_over_http() {
        let url = crate::url::Url::parse("http://localhost:8000/react-demo.html")
            .expect("valid url");
        let page = crate::http::fetch(&url).expect("demo server running on :8000");
        let html = String::from_utf8_lossy(&page.body).into_owned();
        // The page must reference the bundles externally — no inlining here.
        assert!(
            html.contains("<script src=\"./react.production.min.js\"></script>"),
            "demo should load React via external src"
        );

        let (mut session, initial) =
            EngineSession::start(&html, "http://localhost:8000/react-demo.html");
        assert!(initial.error.is_none(), "engine error: {:?}", initial.error);
        let mut now = 0u64;
        for _ in 0..400 {
            if !session.has_pending_work() {
                break;
            }
            now += 16;
            session.pump(now);
        }
        let mount = session.snapshot();
        let root = excerpt_root(&mount.html);
        println!("=== EXTERNAL-SRC MOUNT (excerpt) ===\n{root}");
        // If the engine fetched + ran the external React bundles, the component
        // tree renders into #root.
        assert!(
            root.contains("カウンター") && root.contains("エンジンを書く"),
            "React did not render via external src; root={root}"
        );
        println!("external <script src> React load: OK");
    }

    /// Reproduces the REAL GUI click path: instead of `el.click()` (the
    /// script-driven builtin), it dispatches through `EngineSession::dispatch_event`
    /// using a `data-tobira-node-id` computed exactly like the browser does
    /// (`annotate_node_ids` over the serialized snapshot) — the same id the GUI's
    /// hit-test passes. Isolates whether React onClick fires via the host event
    /// path + node-id mapping (the GUI), not just the script path.
    /// Run with: cargo test --bin tobira react_gui_click_path -- --nocapture
    #[test]
    fn react_gui_click_path_increments_counter() {
        use crate::browser::annotate_node_ids;
        use crate::html::{parse_document, Node};

        let react = std::fs::read_to_string("tests/fixtures/react/react.production.min.js")
            .expect("react fixture present");
        let react_dom = std::fs::read_to_string("tests/fixtures/react/react-dom.production.min.js")
            .expect("react-dom fixture present");
        let app = r#"
            var e = React.createElement;
            function Counter() {
              var s = React.useState(0); var n = s[0]; var setN = s[1];
              return e('button', { onClick: function(){ setN(n + 1); } }, 'count: ' + n);
            }
            ReactDOM.createRoot(document.getElementById('root')).render(e(Counter));
        "#;
        let html = format!(
            "<html><body><div id=\"root\"></div><script>{react}</script><script>{react_dom}</script><script>{app}</script></body></html>"
        );
        let (mut session, initial) = EngineSession::start(&html, "http://localhost/");
        assert!(initial.error.is_none(), "engine error: {:?}", initial.error);
        let pump = |s: &mut EngineSession| {
            let mut now = 0u64;
            for _ in 0..400 {
                if !s.has_pending_work() {
                    break;
                }
                now += 16;
                s.pump(now);
            }
        };
        pump(&mut session);
        let mount = session.snapshot();
        assert!(mount.html.contains("count: 0"), "mount: {}", excerpt_root(&mount.html));

        // Replicate the browser: parse the snapshot + annotate node ids, then find
        // the button's id by its rendered text — exactly what hit-test would carry.
        let mut tree = parse_document(&mount.html);
        annotate_node_ids(&mut tree);
        fn find_button_id(node: &Node, want_text: &str) -> Option<usize> {
            if let Node::Element(el) = node {
                if el.tag_name.eq_ignore_ascii_case("button") {
                    let text = collect_node_text(node);
                    if text.contains(want_text) {
                        return el
                            .attributes
                            .get("data-tobira-node-id")
                            .and_then(|s| s.parse().ok());
                    }
                }
                for c in &el.children {
                    if let Some(id) = find_button_id(c, want_text) {
                        return Some(id);
                    }
                }
            }
            None
        }
        fn collect_node_text(node: &Node) -> String {
            match node {
                Node::Text(t) => t.clone(),
                Node::Element(el) => el.children.iter().map(collect_node_text).collect(),
            }
        }
        let btn_id = find_button_id(&tree, "count:").expect("counter button has a node id");
        println!("counter button data-tobira-node-id = {btn_id}");
        // Cross-check: the engine maps that id back to a <button>.
        let mapped_tag = session
            .host()
            .handle_for_node_id(btn_id)
            .map(|h| session.host().nodes[h].tag_name().map(|s| s.to_string()));
        println!("handle_for_node_id({btn_id}) -> tag {mapped_tag:?}");

        // Dispatch the click the way the GUI does.
        let init = DomEventInit {
            bubbles: true,
            cancelable: true,
            button: Some(0),
            buttons: Some(1),
            ..Default::default()
        };
        let after = session.dispatch_event(btn_id, "click", &init);
        pump(&mut session);
        let snap = session.snapshot();
        println!("after GUI-path click: {}", excerpt_root(&snap.html));
        assert!(
            snap.html.contains("count: 1"),
            "GUI-path click did NOT fire React onClick; default_prevented={}; html={}",
            after.default_prevented,
            excerpt_root(&snap.html)
        );
        println!("GUI-path click: OK (count: 1)");
    }

    /// Pull the `#root` subtree out of a serialized document for readable test
    /// output (the React bundles are huge and inline in the serialized <head>).
    #[cfg(test)]
    fn excerpt_root(html: &str) -> String {
        match html.find("id=\"root\"") {
            Some(i) => {
                let start = html[..i].rfind('<').unwrap_or(i);
                let end = (start + 1200).min(html.len());
                html[start..end].to_string()
            }
            None => html.chars().take(400).collect(),
        }
    }
}
