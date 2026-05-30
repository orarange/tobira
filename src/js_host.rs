// Phase 6 - New engine host implementation
// Provides a Host trait implementation bridging tobira-engine to browser-codex.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use std::any::Any;

use tobira_engine::engine::{
    Compiler, DomEventRequest, DomEventResult, DomMutation, DomMutationResult, DomRead,
    DomReadResult, DomRect, EventTarget, FetchBody, FetchMode, FetchRequest, FrameId,
    HistoryAction, HistoryOutcome, Heap, Host, HostError, HostEvent, HostResult, HostTimeSnapshot,
    LocationSnapshot, NavigationAction, NavigationOutcome, NetworkRequestId, NodeId, NodeKind,
    ObserverOp, ObserverResult, Parser, ScrollMetrics, SiblingDirection, StorageAreaKind, StorageOp,
    StorageResult, TickResult, TimerId, TimerKind, TimerRequest, Vm, WindowId, WindowMetrics,
};

use crate::html::{parse_document, Node};
use crate::js::ProcessedScriptHtml;
use crate::url::Url;

// ---------------------------------------------------------------------------
// DOM arena
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
struct DomArena {
    nodes: Vec<DomArenaNode>,
    document_id: usize,
    html_id: Option<usize>,
    head_id: Option<usize>,
    body_id: Option<usize>,
}

#[derive(Debug, Clone)]
struct DomArenaNode {
    parent: Option<usize>,
    children: Vec<usize>,
    kind: DomArenaNodeKind,
}

#[derive(Debug, Clone)]
enum DomArenaNodeKind {
    Document,
    Element {
        tag_name: String,
        attributes: BTreeMap<String, String>,
    },
    Text(String),
    Fragment,
}

impl DomArena {
    fn from_html(html: &str) -> Self {
        let mut arena = Self::default();
        let doc_id = arena.alloc(None, DomArenaNodeKind::Document);
        arena.document_id = doc_id;

        let parsed = parse_document(html);
        if let Node::Element(root) = &parsed {
            let child_ids: Vec<_> = root
                .children
                .iter()
                .map(|child| arena.push_node(Some(doc_id), child))
                .collect();
            arena.nodes[doc_id].children = child_ids;
        }

        arena.html_id = arena.find_child_tag(doc_id, "html");
        if let Some(html_id) = arena.html_id {
            arena.head_id = arena.find_child_tag(html_id, "head");
            arena.body_id = arena.find_child_tag(html_id, "body");
        }
        arena
    }

    fn alloc(&mut self, parent: Option<usize>, kind: DomArenaNodeKind) -> usize {
        let id = self.nodes.len();
        self.nodes.push(DomArenaNode {
            parent,
            children: Vec::new(),
            kind,
        });
        id
    }

    fn push_node(&mut self, parent: Option<usize>, node: &Node) -> usize {
        match node {
            Node::Text(text) => self.alloc(parent, DomArenaNodeKind::Text(text.clone())),
            Node::Element(element) => {
                let id = self.alloc(
                    parent,
                    DomArenaNodeKind::Element {
                        tag_name: element.tag_name.clone(),
                        attributes: element.attributes.clone(),
                    },
                );
                let children: Vec<_> = element
                    .children
                    .iter()
                    .map(|child| self.push_node(Some(id), child))
                    .collect();
                self.nodes[id].children = children;
                id
            }
        }
    }

    fn find_child_tag(&self, parent_id: usize, tag: &str) -> Option<usize> {
        let children = self.nodes.get(parent_id)?.children.clone();
        for child_id in children {
            if let Some(DomArenaNodeKind::Element { tag_name, .. }) =
                self.nodes.get(child_id).map(|n| &n.kind)
            {
                if tag_name.eq_ignore_ascii_case(tag) {
                    return Some(child_id);
                }
            }
        }
        None
    }

    fn create_element(&mut self, tag_name: &str) -> usize {
        self.alloc(
            None,
            DomArenaNodeKind::Element {
                tag_name: tag_name.to_ascii_lowercase(),
                attributes: BTreeMap::new(),
            },
        )
    }

    fn create_text_node(&mut self, text: &str) -> usize {
        self.alloc(None, DomArenaNodeKind::Text(text.to_string()))
    }

    fn create_fragment(&mut self) -> usize {
        self.alloc(None, DomArenaNodeKind::Fragment)
    }

    fn get_node_kind(&self, id: usize) -> NodeKind {
        match self.nodes.get(id).map(|n| &n.kind) {
            Some(DomArenaNodeKind::Document) => NodeKind::Document,
            Some(DomArenaNodeKind::Element { .. }) => NodeKind::Element,
            Some(DomArenaNodeKind::Text(_)) => NodeKind::Text,
            Some(DomArenaNodeKind::Fragment) => NodeKind::DocumentFragment,
            None => NodeKind::Document,
        }
    }

    fn get_text_content(&self, id: usize) -> String {
        let Some(node) = self.nodes.get(id) else {
            return String::new();
        };
        match &node.kind {
            DomArenaNodeKind::Text(text) => text.clone(),
            _ => node
                .children
                .iter()
                .map(|child_id| self.get_text_content(*child_id))
                .collect::<Vec<_>>()
                .join(""),
        }
    }

    fn set_text_content(&mut self, id: usize, text: &str) {
        let old_children = self.nodes.get(id).map(|n| n.children.clone()).unwrap_or_default();
        for child_id in old_children {
            if let Some(n) = self.nodes.get_mut(child_id) {
                n.parent = None;
            }
        }
        match self.nodes.get_mut(id).map(|n| &mut n.kind) {
            Some(DomArenaNodeKind::Text(t)) => {
                *t = text.to_string();
                if let Some(n) = self.nodes.get_mut(id) {
                    n.children.clear();
                }
            }
            Some(_) => {
                if let Some(n) = self.nodes.get_mut(id) {
                    n.children.clear();
                }
                if !text.is_empty() {
                    let text_id = self.alloc(Some(id), DomArenaNodeKind::Text(text.to_string()));
                    self.nodes[id].children.push(text_id);
                }
            }
            None => {}
        }
    }

    fn get_inner_html(&self, id: usize) -> String {
        let Some(node) = self.nodes.get(id) else {
            return String::new();
        };
        let mut out = String::new();
        for child_id in &node.children {
            self.serialize_node(*child_id, &mut out);
        }
        out
    }

    fn set_inner_html(&mut self, id: usize, html: &str) {
        let old_children = self.nodes.get(id).map(|n| n.children.clone()).unwrap_or_default();
        for child_id in old_children {
            if let Some(n) = self.nodes.get_mut(child_id) {
                n.parent = None;
            }
        }
        if let Some(n) = self.nodes.get_mut(id) {
            n.children.clear();
        }

        let wrapped = format!("<_inner>{html}</_inner>");
        let parsed = parse_document(&wrapped);
        if let Node::Element(doc) = &parsed {
            if let Some(inner) = doc.children.iter().find(|n| {
                matches!(n, Node::Element(e) if e.tag_name == "_inner")
            }) {
                if let Node::Element(inner_elem) = inner {
                    let new_children: Vec<_> = inner_elem
                        .children
                        .iter()
                        .map(|child| self.push_node(Some(id), child))
                        .collect();
                    self.nodes[id].children = new_children;
                }
            }
        }
    }

    fn append_children(&mut self, parent_id: usize, child_ids: Vec<usize>) {
        for child_id in &child_ids {
            let child_id = *child_id;
            if let Some(old_parent_id) =
                self.nodes.get(child_id).and_then(|n| n.parent)
            {
                self.nodes[old_parent_id]
                    .children
                    .retain(|id| *id != child_id);
            }
            if let Some(n) = self.nodes.get_mut(child_id) {
                n.parent = Some(parent_id);
            }
        }
        if let Some(parent_node) = self.nodes.get_mut(parent_id) {
            parent_node.children.extend(child_ids);
        }
    }

    fn prepend_children(&mut self, parent_id: usize, child_ids: Vec<usize>) {
        for child_id in child_ids.iter().copied() {
            if let Some(old_parent_id) = self.nodes.get(child_id).and_then(|n| n.parent) {
                self.nodes[old_parent_id]
                    .children
                    .retain(|id| *id != child_id);
            }
            if let Some(n) = self.nodes.get_mut(child_id) {
                n.parent = Some(parent_id);
            }
        }
        if let Some(parent_node) = self.nodes.get_mut(parent_id) {
            let mut new_children = child_ids;
            new_children.extend(parent_node.children.iter().copied());
            parent_node.children = new_children;
        }
    }

    fn insert_before(&mut self, parent_id: usize, child_id: usize, reference_id: Option<usize>) {
        if let Some(old_parent_id) = self.nodes.get(child_id).and_then(|n| n.parent) {
            self.nodes[old_parent_id]
                .children
                .retain(|id| *id != child_id);
        }
        if let Some(n) = self.nodes.get_mut(child_id) {
            n.parent = Some(parent_id);
        }
        let parent_children = &mut self.nodes[parent_id].children;
        if let Some(ref_id) = reference_id {
            if let Some(pos) = parent_children.iter().position(|id| *id == ref_id) {
                parent_children.insert(pos, child_id);
                return;
            }
        }
        parent_children.push(child_id);
    }

    fn replace_child(&mut self, parent_id: usize, new_child_id: usize, old_child_id: usize) {
        // Detach new_child from its current parent
        if let Some(old_parent_id) = self.nodes.get(new_child_id).and_then(|n| n.parent) {
            self.nodes[old_parent_id]
                .children
                .retain(|id| *id != new_child_id);
        }
        if let Some(n) = self.nodes.get_mut(new_child_id) {
            n.parent = Some(parent_id);
        }
        // Detach old_child
        if let Some(n) = self.nodes.get_mut(old_child_id) {
            n.parent = None;
        }
        let children = &mut self.nodes[parent_id].children;
        if let Some(pos) = children.iter().position(|id| *id == old_child_id) {
            children[pos] = new_child_id;
        }
    }

    fn remove_node(&mut self, node_id: usize) {
        if let Some(parent_id) = self.nodes.get(node_id).and_then(|n| n.parent) {
            self.nodes[parent_id]
                .children
                .retain(|id| *id != node_id);
        }
        if let Some(n) = self.nodes.get_mut(node_id) {
            n.parent = None;
        }
    }

    fn query_selector_all(&self, root_id: usize, selector: &str) -> Vec<usize> {
        let mut results = Vec::new();
        let children = self
            .nodes
            .get(root_id)
            .map(|n| n.children.clone())
            .unwrap_or_default();
        for child_id in children {
            self.collect_matches(child_id, selector, &mut results);
        }
        results
    }

    fn query_selector(&self, root_id: usize, selector: &str) -> Option<usize> {
        self.query_selector_all(root_id, selector).into_iter().next()
    }

    fn collect_matches(&self, node_id: usize, selector: &str, results: &mut Vec<usize>) {
        if self.matches_selector(node_id, selector) {
            results.push(node_id);
        }
        let children = self
            .nodes
            .get(node_id)
            .map(|n| n.children.clone())
            .unwrap_or_default();
        for child_id in children {
            self.collect_matches(child_id, selector, results);
        }
    }

    fn matches_selector(&self, node_id: usize, selector: &str) -> bool {
        let Some(DomArenaNodeKind::Element { tag_name, attributes }) =
            self.nodes.get(node_id).map(|n| &n.kind)
        else {
            return false;
        };

        let selector = selector.trim();

        if selector.contains(',') {
            return selector
                .split(',')
                .any(|s| self.matches_selector(node_id, s.trim()));
        }

        // Descendant combinator: only handle the rightmost part
        if let Some(last_simple) = selector.rsplit(' ').next() {
            if last_simple != selector {
                return self.matches_selector(node_id, last_simple);
            }
        }

        // :first-child / :last-child / :nth-child stubs — return false for now
        if selector.contains(':') {
            let base = selector.split(':').next().unwrap_or("");
            if base.is_empty() {
                return false;
            }
            return self.matches_selector(node_id, base);
        }

        // [attr=value]
        if let Some(inner) = selector.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            if let Some(eq_pos) = inner.find('=') {
                let attr_name = inner[..eq_pos].trim_matches('"').trim_matches('\'');
                let attr_value = inner[eq_pos + 1..].trim_matches('"').trim_matches('\'');
                return attributes
                    .get(attr_name)
                    .map(|v| v.as_str())
                    == Some(attr_value);
            } else {
                return attributes.contains_key(inner);
            }
        }

        // tag.class#id combinations
        let mut remaining = selector;
        let mut tag_match = true;
        let mut class_match = true;
        let mut id_match = true;

        // Extract tag
        let tag_end = remaining
            .find(|c| c == '.' || c == '#' || c == '[')
            .unwrap_or(remaining.len());
        let tag_part = &remaining[..tag_end];
        remaining = &remaining[tag_end..];
        if !tag_part.is_empty() {
            tag_match = tag_name.eq_ignore_ascii_case(tag_part);
        }

        // Extract class / id parts
        while !remaining.is_empty() {
            if let Some(stripped) = remaining.strip_prefix('#') {
                let end = stripped
                    .find(|c| c == '.' || c == '#' || c == '[')
                    .unwrap_or(stripped.len());
                let id_val = &stripped[..end];
                id_match = attributes.get("id").map(|s| s.as_str()) == Some(id_val);
                remaining = &stripped[end..];
            } else if let Some(stripped) = remaining.strip_prefix('.') {
                let end = stripped
                    .find(|c| c == '.' || c == '#' || c == '[')
                    .unwrap_or(stripped.len());
                let class_val = &stripped[..end];
                class_match = attributes
                    .get("class")
                    .map(|classes| classes.split_whitespace().any(|c| c == class_val))
                    .unwrap_or(false);
                remaining = &stripped[end..];
            } else {
                break;
            }
        }

        tag_match && class_match && id_match
    }

    fn get_attribute_names(&self, id: usize) -> Vec<String> {
        match self.nodes.get(id).map(|n| &n.kind) {
            Some(DomArenaNodeKind::Element { attributes, .. }) => {
                attributes.keys().cloned().collect()
            }
            _ => Vec::new(),
        }
    }

    fn clone_node(&mut self, id: usize, deep: bool) -> usize {
        let Some(node) = self.nodes.get(id).cloned() else {
            return id;
        };
        let new_id = self.alloc(None, node.kind.clone());
        if deep {
            let children = node.children.clone();
            let new_children: Vec<_> = children
                .iter()
                .map(|child_id| self.clone_node(*child_id, true))
                .collect();
            for child_id in &new_children {
                if let Some(n) = self.nodes.get_mut(*child_id) {
                    n.parent = Some(new_id);
                }
            }
            self.nodes[new_id].children = new_children;
        }
        new_id
    }

    fn get_node_name(&self, id: usize) -> String {
        match self.nodes.get(id).map(|n| &n.kind) {
            Some(DomArenaNodeKind::Document) => "#document".to_string(),
            Some(DomArenaNodeKind::Element { tag_name, .. }) => tag_name.to_uppercase(),
            Some(DomArenaNodeKind::Text(_)) => "#text".to_string(),
            Some(DomArenaNodeKind::Fragment) => "#document-fragment".to_string(),
            None => String::new(),
        }
    }

    fn serialize_to_html(&self) -> String {
        let mut out = String::new();
        let root_children = self
            .nodes
            .get(self.document_id)
            .map(|n| n.children.clone())
            .unwrap_or_default();
        for child_id in root_children {
            self.serialize_node(child_id, &mut out);
        }
        out
    }

    fn serialize_node(&self, id: usize, out: &mut String) {
        let Some(node) = self.nodes.get(id) else {
            return;
        };
        match &node.kind {
            DomArenaNodeKind::Document | DomArenaNodeKind::Fragment => {
                for child_id in &node.children {
                    self.serialize_node(*child_id, out);
                }
            }
            DomArenaNodeKind::Text(text) => {
                out.push_str(text);
            }
            DomArenaNodeKind::Element { tag_name, attributes } => {
                out.push('<');
                out.push_str(tag_name);
                for (k, v) in attributes {
                    out.push(' ');
                    out.push_str(k);
                    out.push_str("=\"");
                    out.push_str(&v.replace('"', "&quot;"));
                    out.push('"');
                }
                out.push('>');
                if !is_void_element(tag_name) {
                    for child_id in &node.children {
                        self.serialize_node(*child_id, out);
                    }
                    out.push_str("</");
                    out.push_str(tag_name);
                    out.push('>');
                }
            }
        }
    }

    fn extract_scripts(&self) -> Vec<(usize, ScriptEntry)> {
        let mut scripts = Vec::new();
        self.collect_scripts(self.document_id, &mut scripts);
        scripts
    }

    fn collect_scripts(&self, node_id: usize, scripts: &mut Vec<(usize, ScriptEntry)>) {
        let Some(node) = self.nodes.get(node_id) else {
            return;
        };
        if let DomArenaNodeKind::Element { tag_name, attributes } = &node.kind {
            if tag_name.eq_ignore_ascii_case("script") {
                let src = attributes.get("src").cloned();
                let type_attr = attributes.get("type").map(|s| s.as_str());
                let is_module = type_attr == Some("module");
                let is_js = !matches!(
                    type_attr,
                    Some(t) if !t.is_empty()
                        && !t.eq_ignore_ascii_case("text/javascript")
                        && !t.eq_ignore_ascii_case("application/javascript")
                        && !t.eq_ignore_ascii_case("module")
                );
                if is_js {
                    let text_content = self.get_text_content(node_id);
                    if !text_content.trim().is_empty() || src.is_some() {
                        scripts.push((
                            node_id,
                            ScriptEntry {
                                src,
                                inline_source: text_content,
                                is_module,
                            },
                        ));
                    }
                }
                return; // Don't recurse into script children
            }
        }
        let children = node.children.clone();
        for child_id in children {
            self.collect_scripts(child_id, scripts);
        }
    }
}

fn is_void_element(tag: &str) -> bool {
    matches!(
        tag.to_ascii_lowercase().as_str(),
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

// ---------------------------------------------------------------------------
// Script extraction
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct ScriptEntry {
    src: Option<String>,
    inline_source: String,
    is_module: bool,
}

// ---------------------------------------------------------------------------
// Host context
// ---------------------------------------------------------------------------

struct TobirahHost {
    dom: DomArena,
    document_url: Url,
    viewport_width: u32,
    viewport_height: u32,
    scroll_y: f64,
    console_logs: Vec<String>,
    navigation_target: Option<String>,
    soft_navigation_target: Option<String>,
    next_timer_id: u64,
    next_raf_id: u64,
}

impl TobirahHost {
    fn new(dom: DomArena, document_url: Url) -> Self {
        Self {
            dom,
            document_url,
            viewport_width: 1280,
            viewport_height: 720,
            scroll_y: 0.0,
            console_logs: Vec::new(),
            navigation_target: None,
            soft_navigation_target: None,
            next_timer_id: 1,
            next_raf_id: 1,
        }
    }

    fn node_id(raw: usize) -> NodeId {
        NodeId(raw as u32)
    }

    fn raw_id(node: NodeId) -> usize {
        node.0 as usize
    }
}

impl Host for TobirahHost {
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn window(&self) -> WindowId {
        WindowId(0)
    }

    fn window_metrics(&self, _window: WindowId) -> HostResult<WindowMetrics> {
        Ok(WindowMetrics {
            inner_width: self.viewport_width as f64,
            inner_height: self.viewport_height as f64,
            scroll_x: 0.0,
            scroll_y: self.scroll_y,
            device_pixel_ratio: 1.0,
        })
    }

    fn location(&self, _window: WindowId) -> HostResult<LocationSnapshot> {
        let url = &self.document_url;
        let scheme = url.scheme.clone();
        let hostname = url.host.clone();
        let port = url.port;
        let is_default_port =
            (scheme == "http" && port == 80) || (scheme == "https" && port == 443);
        let port_str = if is_default_port {
            String::new()
        } else {
            port.to_string()
        };
        let host_with_port = if is_default_port {
            hostname.clone()
        } else {
            format!("{hostname}:{port}")
        };

        let (path_no_frag, fragment) = match url.path.split_once('#') {
            Some((p, f)) => (p, Some(f.to_string())),
            None => (url.path.as_str(), None),
        };
        let (pathname, query) = match path_no_frag.split_once('?') {
            Some((p, q)) => (p.to_string(), Some(q.to_string())),
            None => (path_no_frag.to_string(), None),
        };
        let search = query.map(|q| format!("?{q}")).unwrap_or_default();
        let hash = fragment.map(|f| format!("#{f}")).unwrap_or_default();

        Ok(LocationSnapshot {
            href: url.to_string(),
            origin: url.origin(),
            protocol: format!("{scheme}:"),
            host: host_with_port,
            hostname,
            port: port_str,
            pathname,
            search,
            hash,
        })
    }

    fn navigate(&mut self, action: NavigationAction) -> HostResult<NavigationOutcome> {
        match action {
            NavigationAction::Navigate { url, replace: _, .. } => {
                self.navigation_target = Some(url);
            }
            NavigationAction::SetHash { hash, .. } => {
                self.soft_navigation_target = Some(hash);
            }
        }
        Ok(NavigationOutcome {
            committed: true,
            same_document: false,
        })
    }

    fn history(&mut self, _action: HistoryAction) -> HostResult<HistoryOutcome> {
        Ok(HistoryOutcome {
            href: self.document_url.to_string(),
            state: None,
            length: 1,
            restored_scroll_y: None,
        })
    }

    fn read_dom(&self, read: DomRead) -> HostResult<DomReadResult> {
        match read {
            DomRead::DocumentRoot { .. } => {
                Ok(DomReadResult::Node(Self::node_id(self.dom.document_id)))
            }
            DomRead::DocumentHead { .. } => Ok(match self.dom.head_id {
                Some(id) => DomReadResult::Node(Self::node_id(id)),
                None => DomReadResult::None,
            }),
            DomRead::DocumentBody { .. } => Ok(match self.dom.body_id {
                Some(id) => DomReadResult::Node(Self::node_id(id)),
                None => DomReadResult::None,
            }),
            DomRead::ActiveElement { .. } => Ok(DomReadResult::None),
            DomRead::QuerySelector { root, selectors } => {
                let root_id = Self::raw_id(root);
                Ok(match self.dom.query_selector(root_id, &selectors) {
                    Some(id) => DomReadResult::Node(Self::node_id(id)),
                    None => DomReadResult::None,
                })
            }
            DomRead::QuerySelectorAll { root, selectors } => {
                let root_id = Self::raw_id(root);
                let ids = self.dom.query_selector_all(root_id, &selectors);
                Ok(DomReadResult::Nodes(ids.into_iter().map(Self::node_id).collect()))
            }
            DomRead::Matches { node, selectors } => {
                let node_id = Self::raw_id(node);
                Ok(DomReadResult::Bool(
                    self.dom.matches_selector(node_id, &selectors),
                ))
            }
            DomRead::Closest { node, selectors } => {
                let mut current = Self::raw_id(node);
                loop {
                    if self.dom.matches_selector(current, &selectors) {
                        return Ok(DomReadResult::Node(Self::node_id(current)));
                    }
                    match self.dom.nodes.get(current).and_then(|n| n.parent) {
                        Some(parent_id) => current = parent_id,
                        None => return Ok(DomReadResult::None),
                    }
                }
            }
            DomRead::Contains { ancestor, descendant } => {
                let ancestor_id = Self::raw_id(ancestor);
                let descendant_id = Self::raw_id(descendant);
                Ok(DomReadResult::Bool(
                    self.dom_is_descendant(descendant_id, ancestor_id),
                ))
            }
            DomRead::Parent { node } => {
                let node_id = Self::raw_id(node);
                Ok(match self.dom.nodes.get(node_id).and_then(|n| n.parent) {
                    Some(parent_id) => DomReadResult::Node(Self::node_id(parent_id)),
                    None => DomReadResult::None,
                })
            }
            DomRead::Children { node, elements_only } => {
                let node_id = Self::raw_id(node);
                let children = self
                    .dom
                    .nodes
                    .get(node_id)
                    .map(|n| n.children.clone())
                    .unwrap_or_default();
                let filtered: Vec<_> = children
                    .into_iter()
                    .filter(|&child_id| {
                        if elements_only {
                            matches!(
                                self.dom.nodes.get(child_id).map(|n| &n.kind),
                                Some(DomArenaNodeKind::Element { .. })
                            )
                        } else {
                            true
                        }
                    })
                    .map(Self::node_id)
                    .collect();
                Ok(DomReadResult::Nodes(filtered))
            }
            DomRead::Sibling { node, direction, elements_only } => {
                let node_id = Self::raw_id(node);
                let parent_id = match self.dom.nodes.get(node_id).and_then(|n| n.parent) {
                    Some(p) => p,
                    None => return Ok(DomReadResult::None),
                };
                let siblings = self
                    .dom
                    .nodes
                    .get(parent_id)
                    .map(|n| n.children.clone())
                    .unwrap_or_default();
                let pos = siblings.iter().position(|&id| id == node_id);
                let Some(pos) = pos else {
                    return Ok(DomReadResult::None);
                };
                let candidates: Box<dyn Iterator<Item = usize>> = match direction {
                    SiblingDirection::Next => Box::new(siblings[pos + 1..].iter().copied()),
                    SiblingDirection::Previous => {
                        Box::new(siblings[..pos].iter().rev().copied())
                    }
                };
                for candidate_id in candidates {
                    let is_element = matches!(
                        self.dom.nodes.get(candidate_id).map(|n| &n.kind),
                        Some(DomArenaNodeKind::Element { .. })
                    );
                    if !elements_only || is_element {
                        return Ok(DomReadResult::Node(Self::node_id(candidate_id)));
                    }
                }
                Ok(DomReadResult::None)
            }
            DomRead::NodeKind { node } => {
                let node_id = Self::raw_id(node);
                Ok(DomReadResult::Kind(self.dom.get_node_kind(node_id)))
            }
            DomRead::NodeName { node } => {
                let node_id = Self::raw_id(node);
                Ok(DomReadResult::String(self.dom.get_node_name(node_id)))
            }
            DomRead::NodeValue { node } => {
                let node_id = Self::raw_id(node);
                Ok(match self.dom.nodes.get(node_id).map(|n| &n.kind) {
                    Some(DomArenaNodeKind::Text(text)) => {
                        DomReadResult::String(text.clone())
                    }
                    _ => DomReadResult::None,
                })
            }
            DomRead::TextContent { node } => {
                let node_id = Self::raw_id(node);
                Ok(DomReadResult::String(self.dom.get_text_content(node_id)))
            }
            DomRead::InnerHtml { node } => {
                let node_id = Self::raw_id(node);
                Ok(DomReadResult::String(self.dom.get_inner_html(node_id)))
            }
            DomRead::Attribute { node, name } => {
                let node_id = Self::raw_id(node);
                Ok(
                    match self.dom.nodes.get(node_id).map(|n| &n.kind) {
                        Some(DomArenaNodeKind::Element { attributes, .. }) => {
                            match attributes.get(&name) {
                                Some(val) => DomReadResult::String(val.clone()),
                                None => DomReadResult::None,
                            }
                        }
                        _ => DomReadResult::None,
                    },
                )
            }
            DomRead::AttributeNames { node } => {
                let node_id = Self::raw_id(node);
                Ok(DomReadResult::StringList(
                    self.dom.get_attribute_names(node_id),
                ))
            }
            DomRead::ShadowRoot { .. }
            | DomRead::AssignedNodes { .. } => Ok(DomReadResult::None),
            DomRead::BoundingClientRect { .. } => Ok(DomReadResult::Rect(DomRect {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 0.0,
            })),
            DomRead::ScrollMetrics { .. } => Ok(DomReadResult::ScrollMetrics(ScrollMetrics {
                scroll_left: 0.0,
                scroll_top: 0.0,
                scroll_width: 0.0,
                scroll_height: 0.0,
                client_width: self.viewport_width as f64,
                client_height: self.viewport_height as f64,
            })),
        }
    }

    fn mutate_dom(&mut self, mutation: DomMutation) -> HostResult<DomMutationResult> {
        match mutation {
            DomMutation::CreateElement { local_name, .. } => {
                let id = self.dom.create_element(&local_name);
                Ok(DomMutationResult::Node(Self::node_id(id)))
            }
            DomMutation::CreateTextNode { data, .. } => {
                let id = self.dom.create_text_node(&data);
                Ok(DomMutationResult::Node(Self::node_id(id)))
            }
            DomMutation::CreateDocumentFragment { .. } => {
                let id = self.dom.create_fragment();
                Ok(DomMutationResult::Node(Self::node_id(id)))
            }
            DomMutation::CloneNode { node, deep } => {
                let node_id = Self::raw_id(node);
                let new_id = self.dom.clone_node(node_id, deep);
                Ok(DomMutationResult::Node(Self::node_id(new_id)))
            }
            DomMutation::SetTextContent { node, value } => {
                let node_id = Self::raw_id(node);
                self.dom.set_text_content(node_id, &value);
                Ok(DomMutationResult::None)
            }
            DomMutation::SetInnerHtml { node, html } => {
                let node_id = Self::raw_id(node);
                self.dom.set_inner_html(node_id, &html);
                Ok(DomMutationResult::None)
            }
            DomMutation::WriteHtml { html, .. } => {
                // document.write: append to body or document
                let target_id = self.dom.body_id.unwrap_or(self.dom.document_id);
                self.dom.set_inner_html(
                    target_id,
                    &format!("{}{html}", self.dom.get_inner_html(target_id)),
                );
                Ok(DomMutationResult::None)
            }
            DomMutation::SetAttribute { node, name, value } => {
                let node_id = Self::raw_id(node);
                if let Some(DomArenaNodeKind::Element { attributes, .. }) =
                    self.dom.nodes.get_mut(node_id).map(|n| &mut n.kind)
                {
                    attributes.insert(name, value);
                }
                Ok(DomMutationResult::None)
            }
            DomMutation::RemoveAttribute { node, name } => {
                let node_id = Self::raw_id(node);
                if let Some(DomArenaNodeKind::Element { attributes, .. }) =
                    self.dom.nodes.get_mut(node_id).map(|n| &mut n.kind)
                {
                    attributes.remove(&name);
                }
                Ok(DomMutationResult::None)
            }
            DomMutation::ToggleAttribute { node, name, force } => {
                let node_id = Self::raw_id(node);
                if let Some(DomArenaNodeKind::Element { attributes, .. }) =
                    self.dom.nodes.get_mut(node_id).map(|n| &mut n.kind)
                {
                    let present = attributes.contains_key(&name);
                    let should_add = force.unwrap_or(!present);
                    if should_add {
                        attributes.insert(name, String::new());
                    } else {
                        attributes.remove(&name);
                    }
                    return Ok(DomMutationResult::Bool(should_add));
                }
                Ok(DomMutationResult::Bool(false))
            }
            DomMutation::Append { parent, children } => {
                let parent_id = Self::raw_id(parent);
                let child_ids: Vec<_> = children.into_iter().map(Self::raw_id).collect();
                self.dom.append_children(parent_id, child_ids);
                Ok(DomMutationResult::None)
            }
            DomMutation::Prepend { parent, children } => {
                let parent_id = Self::raw_id(parent);
                let child_ids: Vec<_> = children.into_iter().map(Self::raw_id).collect();
                self.dom.prepend_children(parent_id, child_ids);
                Ok(DomMutationResult::None)
            }
            DomMutation::InsertBefore { parent, child, reference } => {
                let parent_id = Self::raw_id(parent);
                let child_id = Self::raw_id(child);
                let reference_id = reference.map(Self::raw_id);
                self.dom.insert_before(parent_id, child_id, reference_id);
                Ok(DomMutationResult::None)
            }
            DomMutation::ReplaceChild { parent, new_child, old_child } => {
                let parent_id = Self::raw_id(parent);
                let new_child_id = Self::raw_id(new_child);
                let old_child_id = Self::raw_id(old_child);
                self.dom.replace_child(parent_id, new_child_id, old_child_id);
                Ok(DomMutationResult::None)
            }
            DomMutation::SetScrollOffset { node: _, x: _, y } => {
                self.scroll_y = y;
                Ok(DomMutationResult::None)
            }
            DomMutation::SetWindowScroll { y, .. } => {
                self.scroll_y = y;
                Ok(DomMutationResult::None)
            }
            DomMutation::Remove { node } => {
                let node_id = Self::raw_id(node);
                self.dom.remove_node(node_id);
                Ok(DomMutationResult::None)
            }
            DomMutation::AttachShadow { .. } => Ok(DomMutationResult::None),
        }
    }

    fn dispatch_dom_event(&mut self, _request: DomEventRequest) -> HostResult<DomEventResult> {
        Ok(DomEventResult {
            default_prevented: false,
        })
    }

    fn console(&mut self, message: tobira_engine::engine::ConsoleMessage) -> HostResult<()> {
        let text = message.parts.join(" ");
        eprintln!("[JS console] {text}");
        self.console_logs.push(text);
        Ok(())
    }

    fn schedule_timer(&mut self, _request: TimerRequest) -> HostResult<TimerId> {
        let id = self.next_timer_id;
        self.next_timer_id += 1;
        Ok(TimerId(id))
    }

    fn cancel_timer(&mut self, _timer_id: TimerId) -> HostResult<bool> {
        Ok(true)
    }

    fn request_animation_frame(&mut self, _window: WindowId) -> HostResult<FrameId> {
        let id = self.next_raf_id;
        self.next_raf_id += 1;
        Ok(FrameId(id))
    }

    fn cancel_animation_frame(&mut self, _frame_id: FrameId) -> HostResult<bool> {
        Ok(true)
    }

    fn fetch(&mut self, _request: FetchRequest) -> HostResult<NetworkRequestId> {
        Err(HostError::Unsupported)
    }

    fn abort_fetch(&mut self, _request_id: NetworkRequestId) -> HostResult<bool> {
        Ok(false)
    }

    fn storage(&mut self, operation: StorageOp) -> HostResult<StorageResult> {
        use crate::site_state::{
            storage_clear, storage_get_item, storage_key, storage_length, storage_remove_item,
            storage_set_item, StorageKind,
        };

        let (kind, scope_url) = match &operation {
            StorageOp::Get { kind, scope, .. }
            | StorageOp::Set { kind, scope, .. }
            | StorageOp::Remove { kind, scope, .. }
            | StorageOp::Clear { kind, scope }
            | StorageOp::Keys { kind, scope }
            | StorageOp::Len { kind, scope } => {
                let sk = match kind {
                    StorageAreaKind::Local => StorageKind::Local,
                    StorageAreaKind::Session => StorageKind::Session,
                    StorageAreaKind::Cookie => StorageKind::Session,
                };
                (sk, self.document_url.clone())
            }
        };

        match operation {
            StorageOp::Get { key, .. } => Ok(StorageResult::Value(storage_get_item(
                kind,
                &scope_url,
                &key,
            ))),
            StorageOp::Set { key, value, .. } => {
                storage_set_item(kind, &scope_url, key, value);
                Ok(StorageResult::None)
            }
            StorageOp::Remove { key, .. } => {
                storage_remove_item(kind, &scope_url, &key);
                Ok(StorageResult::None)
            }
            StorageOp::Clear { .. } => {
                storage_clear(kind, &scope_url);
                Ok(StorageResult::None)
            }
            StorageOp::Keys { .. } => {
                let len = storage_length(kind, &scope_url);
                let keys: Vec<String> = (0..len)
                    .filter_map(|i| storage_key(kind, &scope_url, i))
                    .collect();
                Ok(StorageResult::Keys(keys))
            }
            StorageOp::Len { .. } => {
                Ok(StorageResult::Len(storage_length(kind, &scope_url)))
            }
        }
    }

    fn observer(&mut self, _operation: ObserverOp) -> HostResult<ObserverResult> {
        Err(HostError::Unsupported)
    }

    fn now(&self) -> HostTimeSnapshot {
        let unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        HostTimeSnapshot {
            monotonic_ms: unix_ms,
            unix_ms,
        }
    }

    fn wait_for_host_events(&mut self, _max_wait_ms: Option<u64>) -> HostResult<Vec<HostEvent>> {
        Ok(Vec::new())
    }
}

impl TobirahHost {
    fn dom_is_descendant(&self, node_id: usize, ancestor_id: usize) -> bool {
        let mut current = node_id;
        loop {
            if current == ancestor_id {
                return true;
            }
            match self.dom.nodes.get(current).and_then(|n| n.parent) {
                Some(parent_id) => current = parent_id,
                None => return false,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Runs document scripts using the new tobira-engine VM.
/// Returns a `ProcessedScriptHtml` compatible with the existing browser pipeline.
/// This is a synchronous single-pass implementation (no timers/async in Phase 6 Step 1).
pub fn run_with_new_engine(html: &str, base_url: &Url) -> ProcessedScriptHtml {
    let mut dom = DomArena::from_html(html);
    let scripts = dom.extract_scripts();

    if scripts.is_empty() {
        return ProcessedScriptHtml {
            html: html.to_string(),
            title_override: None,
            console_logs: Vec::new(),
            navigation_target: None,
            soft_navigation_target: None,
            scroll_y: 0,
        };
    }

    let host = TobirahHost::new(dom, base_url.clone());
    let mut vm = Vm::with_host(Heap::new(), Box::new(host));

    for (_node_id, script) in &scripts {
        if script.src.is_some() {
            // External scripts not yet supported in Phase 6 Step 1
            let now_ms = vm.host_mut().now().monotonic_ms;
            vm.host_mut()
                .as_any_mut()
                .downcast_mut::<TobirahHost>()
                .unwrap()
                .console_logs
                .push(format!("JS: skipping external script (src not supported yet)"));
            let _ = now_ms;
            continue;
        }
        let source = {
            let h = vm.host_mut().as_any_mut().downcast_mut::<TobirahHost>().unwrap();
            script.inline_source.clone()
        };
        if source.trim().is_empty() {
            continue;
        }

        let program = match Parser::new(&source).parse() {
            Ok(p) => p,
            Err(e) => {
                vm.host_mut()
                    .as_any_mut()
                    .downcast_mut::<TobirahHost>()
                    .unwrap()
                    .console_logs
                    .push(format!("JS parse error: {e:?}"));
                continue;
            }
        };

        let chunk = match Compiler::new(&program).compile() {
            Ok(c) => c,
            Err(e) => {
                vm.host_mut()
                    .as_any_mut()
                    .downcast_mut::<TobirahHost>()
                    .unwrap()
                    .console_logs
                    .push(format!("JS compile error: {e:?}"));
                continue;
            }
        };

        if let Err(e) = vm.execute(&chunk) {
            vm.host_mut()
                .as_any_mut()
                .downcast_mut::<TobirahHost>()
                .unwrap()
                .console_logs
                .push(format!("JS runtime error: {e:?}"));
        }

        // Drive microtask queue / due timers after each script
        let now_ms = vm.host_mut().now().monotonic_ms;
        vm.event_loop_tick(now_ms, false);
    }

    let host = vm
        .host_mut()
        .as_any_mut()
        .downcast_mut::<TobirahHost>()
        .unwrap();

    let title_override = find_title(&host.dom);
    let html_out = host.dom.serialize_to_html();

    ProcessedScriptHtml {
        html: html_out,
        title_override,
        console_logs: host.console_logs.clone(),
        navigation_target: host.navigation_target.clone(),
        soft_navigation_target: host.soft_navigation_target.clone(),
        scroll_y: host.scroll_y as u32,
    }
}

fn find_title(dom: &DomArena) -> Option<String> {
    let head_id = dom.head_id?;
    let title_id = dom.find_child_tag(head_id, "title")?;
    let text = dom.get_text_content(title_id);
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}
