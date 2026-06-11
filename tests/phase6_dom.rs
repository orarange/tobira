use std::any::Any;
use std::collections::BTreeMap;

use tobira_engine::engine::{
    Compiler, DomEventRequest, DomEventResult, DomMutation, DomMutationResult, DomRead,
    DomReadResult, FetchRequest, FetchResponse, FrameId, HistoryAction, HistoryOutcome, Host,
    HostError,
    HostEvent, HostResult, HostTimeSnapshot, LocationSnapshot, NavigationAction,
    NavigationOutcome, NetworkRequestId, NodeId, NodeKind, ObserverOp, ObserverResult, Parser,
    StorageOp, StorageResult, TimerId, TimerRequest, Vm, Heap, WindowId, WindowMetrics,
    ConsoleMessage, SiblingDirection,
};

// ---------------------------------------------------------------------------
// TestDom — minimal in-memory DOM arena
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum TestNodeKind {
    Document,
    Element(String),     // tag name
    Text(String),        // text data
    Fragment,
}

#[derive(Debug, Clone)]
struct TestNode {
    kind: TestNodeKind,
    parent: Option<usize>,
    children: Vec<usize>,
    attrs: BTreeMap<String, String>,
}

impl TestNode {
    fn new_document() -> Self {
        Self { kind: TestNodeKind::Document, parent: None, children: vec![], attrs: BTreeMap::new() }
    }
    fn new_element(tag: &str) -> Self {
        Self { kind: TestNodeKind::Element(tag.to_lowercase()), parent: None, children: vec![], attrs: BTreeMap::new() }
    }
    fn new_text(data: &str) -> Self {
        Self { kind: TestNodeKind::Text(data.to_string()), parent: None, children: vec![], attrs: BTreeMap::new() }
    }
    fn new_fragment() -> Self {
        Self { kind: TestNodeKind::Fragment, parent: None, children: vec![], attrs: BTreeMap::new() }
    }
    fn is_element(&self) -> bool {
        matches!(self.kind, TestNodeKind::Element(_))
    }
    fn tag_name(&self) -> Option<&str> {
        if let TestNodeKind::Element(tag) = &self.kind { Some(tag.as_str()) } else { None }
    }
}

struct TestDom {
    nodes: Vec<TestNode>,
}

impl TestDom {
    fn new() -> Self {
        // node 0 = document root
        // node 1 = <html>
        // node 2 = <head>
        // node 3 = <body>
        let mut dom = Self { nodes: Vec::new() };
        dom.nodes.push(TestNode::new_document());          // 0 document
        dom.nodes.push(TestNode::new_element("html"));     // 1
        dom.nodes.push(TestNode::new_element("head"));     // 2
        dom.nodes.push(TestNode::new_element("body"));     // 3

        // wire 0 → 1
        dom.nodes[0].children.push(1);
        dom.nodes[1].parent = Some(0);
        // wire 1 → 2, 3
        dom.nodes[1].children.push(2);
        dom.nodes[2].parent = Some(1);
        dom.nodes[1].children.push(3);
        dom.nodes[3].parent = Some(1);

        dom
    }

    /// Add a new node, return its index.
    fn push(&mut self, node: TestNode) -> usize {
        let idx = self.nodes.len();
        self.nodes.push(node);
        idx
    }

    /// Detach node from its current parent.
    fn detach(&mut self, child: usize) {
        if let Some(parent_idx) = self.nodes[child].parent {
            self.nodes[parent_idx].children.retain(|&c| c != child);
            self.nodes[child].parent = None;
        }
    }

    /// Recursively collect text content.
    fn collect_text(&self, idx: usize) -> String {
        let node = &self.nodes[idx];
        match &node.kind {
            TestNodeKind::Text(s) => s.clone(),
            _ => {
                let mut out = String::new();
                for &child in &node.children {
                    out.push_str(&self.collect_text(child));
                }
                out
            }
        }
    }

    /// Very simple innerHTML serialiser (enough for the tests we write).
    fn inner_html(&self, idx: usize) -> String {
        let mut out = String::new();
        for &child in &self.nodes[idx].children {
            out.push_str(&self.serialize_node(child));
        }
        out
    }

    fn serialize_node(&self, idx: usize) -> String {
        let node = &self.nodes[idx];
        match &node.kind {
            TestNodeKind::Text(s) => s.clone(),
            TestNodeKind::Element(tag) => {
                let mut s = format!("<{}", tag);
                for (k, v) in &node.attrs {
                    s.push_str(&format!(" {}=\"{}\"", k, v));
                }
                s.push('>');
                s.push_str(&self.inner_html(idx));
                s.push_str(&format!("</{}>", tag));
                s
            }
            TestNodeKind::Document => self.inner_html(idx),
            TestNodeKind::Fragment => self.inner_html(idx),
        }
    }

    /// Very simple innerHTML parser — handles `<tag>text</tag>` one level deep.
    fn parse_and_set_inner_html(&mut self, parent: usize, html: &str) {
        // Remove existing children.
        let old_children: Vec<usize> = self.nodes[parent].children.clone();
        for child in old_children {
            self.nodes[child].parent = None;
        }
        self.nodes[parent].children.clear();

        // Minimal parser: scan for tags.
        let mut rest = html;
        while !rest.is_empty() {
            if let Some(stripped) = rest.strip_prefix('<') {
                if let Some(close_bracket) = stripped.find('>') {
                    let tag_content = &stripped[..close_bracket];
                    let tag_name = tag_content.split_whitespace().next().unwrap_or("").to_lowercase();
                    let after_open = &stripped[close_bracket + 1..];
                    // find closing tag
                    let close_tag = format!("</{}>", tag_name);
                    if let Some(close_pos) = after_open.find(&close_tag) {
                        let inner_text = &after_open[..close_pos];
                        let el_idx = self.push(TestNode::new_element(&tag_name));
                        if !inner_text.is_empty() {
                            let text_idx = self.push(TestNode::new_text(inner_text));
                            self.nodes[text_idx].parent = Some(el_idx);
                            self.nodes[el_idx].children.push(text_idx);
                        }
                        self.nodes[el_idx].parent = Some(parent);
                        self.nodes[parent].children.push(el_idx);
                        rest = &after_open[close_pos + close_tag.len()..];
                    } else {
                        // self-closing or unclosed — just create element, skip
                        let el_idx = self.push(TestNode::new_element(&tag_name));
                        self.nodes[el_idx].parent = Some(parent);
                        self.nodes[parent].children.push(el_idx);
                        rest = after_open;
                    }
                } else {
                    break;
                }
            } else {
                // text node until next '<'
                let end = rest.find('<').unwrap_or(rest.len());
                let text = &rest[..end];
                if !text.is_empty() {
                    let text_idx = self.push(TestNode::new_text(text));
                    self.nodes[text_idx].parent = Some(parent);
                    self.nodes[parent].children.push(text_idx);
                }
                rest = &rest[end..];
            }
        }
    }

    /// Match selector against node (supports: tag, #id, .class, *)
    fn node_matches_selector(&self, idx: usize, sel: &str) -> bool {
        let node = &self.nodes[idx];
        if !node.is_element() { return false; }
        let tag = node.tag_name().unwrap_or("");
        let id_attr = node.attrs.get("id").map(|s| s.as_str()).unwrap_or("");
        let class_attr = node.attrs.get("class").map(|s| s.as_str()).unwrap_or("");

        let sel = sel.trim();
        if sel == "*" { return true; }
        if let Some(id) = sel.strip_prefix('#') {
            return id_attr == id;
        }
        if let Some(cls) = sel.strip_prefix('.') {
            return class_attr.split_whitespace().any(|c| c == cls);
        }
        // tag name selector
        tag == sel.to_lowercase()
    }

    /// Depth-first search for the first node matching the selector.
    fn query_selector(&self, root: usize, sel: &str) -> Option<usize> {
        for &child in &self.nodes[root].children {
            if self.node_matches_selector(child, sel) {
                return Some(child);
            }
            if let Some(found) = self.query_selector(child, sel) {
                return Some(found);
            }
        }
        None
    }

    /// Depth-first search for all matching nodes.
    fn query_selector_all(&self, root: usize, sel: &str) -> Vec<usize> {
        let mut results = Vec::new();
        self.collect_matching(root, sel, &mut results);
        results
    }

    fn collect_matching(&self, root: usize, sel: &str, out: &mut Vec<usize>) {
        for &child in &self.nodes[root].children {
            if self.node_matches_selector(child, sel) {
                out.push(child);
            }
            self.collect_matching(child, sel, out);
        }
    }

    /// Return the body node index (node 3 in our layout).
    fn body_idx(&self) -> usize {
        // node 3 is always <body>
        3
    }
}

// ---------------------------------------------------------------------------
// Host impl for TestDom
// ---------------------------------------------------------------------------

impl Host for TestDom {
    fn as_any_mut(&mut self) -> &mut dyn Any { self }

    fn window(&self) -> WindowId { WindowId(0) }

    fn window_metrics(&self, _w: WindowId) -> HostResult<WindowMetrics> {
        Ok(WindowMetrics { inner_width: 800.0, inner_height: 600.0, scroll_x: 0.0, scroll_y: 0.0, device_pixel_ratio: 1.0 })
    }

    fn location(&self, _w: WindowId) -> HostResult<LocationSnapshot> {
        Ok(LocationSnapshot {
            href: "http://localhost/".into(), origin: "http://localhost".into(),
            protocol: "http:".into(), host: "localhost".into(), hostname: "localhost".into(),
            port: "".into(), pathname: "/".into(), search: "".into(), hash: "".into(),
        })
    }

    fn navigate(&mut self, _a: NavigationAction) -> HostResult<NavigationOutcome> {
        Ok(NavigationOutcome { committed: false, same_document: false })
    }

    fn history(&mut self, _a: HistoryAction) -> HostResult<HistoryOutcome> {
        Ok(HistoryOutcome { href: String::new(), state: None, length: 0, restored_scroll_y: None })
    }

    fn read_dom(&self, read: DomRead) -> HostResult<DomReadResult> {
        match read {
            DomRead::DocumentRoot { .. } => Ok(DomReadResult::Node(NodeId(0))),
            DomRead::DocumentHead { .. } => Ok(DomReadResult::Node(NodeId(2))),
            DomRead::DocumentBody { .. } => Ok(DomReadResult::Node(NodeId(self.body_idx() as u32))),
            DomRead::ActiveElement { .. } => Ok(DomReadResult::None),

            DomRead::QuerySelector { root, selectors } => {
                let root_idx = root.0 as usize;
                match self.query_selector(root_idx, &selectors) {
                    Some(idx) => Ok(DomReadResult::Node(NodeId(idx as u32))),
                    None => Ok(DomReadResult::None),
                }
            }
            DomRead::QuerySelectorAll { root, selectors } => {
                let root_idx = root.0 as usize;
                let ids: Vec<NodeId> = self.query_selector_all(root_idx, &selectors)
                    .iter().map(|&i| NodeId(i as u32)).collect();
                Ok(DomReadResult::Nodes(ids))
            }
            DomRead::Matches { node, selectors } => {
                let idx = node.0 as usize;
                if idx >= self.nodes.len() { return Err(HostError::InvalidHandle); }
                Ok(DomReadResult::Bool(self.node_matches_selector(idx, &selectors)))
            }
            DomRead::Contains { ancestor, descendant } => {
                // check if descendant is in ancestor's subtree
                let mut cur = descendant.0 as usize;
                loop {
                    if cur == ancestor.0 as usize { return Ok(DomReadResult::Bool(true)); }
                    match self.nodes.get(cur).and_then(|n| n.parent) {
                        Some(p) => cur = p,
                        None => return Ok(DomReadResult::Bool(false)),
                    }
                }
            }
            DomRead::Parent { node } => {
                let idx = node.0 as usize;
                if idx >= self.nodes.len() { return Err(HostError::InvalidHandle); }
                Ok(match self.nodes[idx].parent {
                    Some(p) => DomReadResult::Node(NodeId(p as u32)),
                    None => DomReadResult::None,
                })
            }
            DomRead::Children { node, elements_only } => {
                let idx = node.0 as usize;
                if idx >= self.nodes.len() { return Err(HostError::InvalidHandle); }
                let ids: Vec<NodeId> = self.nodes[idx].children.iter()
                    .filter(|&&c| {
                        if elements_only { self.nodes[c].is_element() } else { true }
                    })
                    .map(|&c| NodeId(c as u32))
                    .collect();
                Ok(DomReadResult::Nodes(ids))
            }
            DomRead::Sibling { node, direction, elements_only } => {
                let idx = node.0 as usize;
                if idx >= self.nodes.len() { return Err(HostError::InvalidHandle); }
                if let Some(parent_idx) = self.nodes[idx].parent {
                    let siblings = &self.nodes[parent_idx].children;
                    let pos = siblings.iter().position(|&c| c == idx);
                    if let Some(pos) = pos {
                        let candidates: Box<dyn Iterator<Item = usize>> = match direction {
                            SiblingDirection::Next => Box::new(siblings[pos + 1..].iter().cloned()),
                            SiblingDirection::Previous => Box::new(siblings[..pos].iter().cloned().rev()),
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
                let idx = node.0 as usize;
                if idx >= self.nodes.len() { return Err(HostError::InvalidHandle); }
                let kind = match &self.nodes[idx].kind {
                    TestNodeKind::Document => NodeKind::Document,
                    TestNodeKind::Element(_) => NodeKind::Element,
                    TestNodeKind::Text(_) => NodeKind::Text,
                    TestNodeKind::Fragment => NodeKind::DocumentFragment,
                };
                Ok(DomReadResult::Kind(kind))
            }
            DomRead::NodeName { node } => {
                let idx = node.0 as usize;
                if idx >= self.nodes.len() { return Err(HostError::InvalidHandle); }
                let name = match &self.nodes[idx].kind {
                    TestNodeKind::Document => "#document".to_string(),
                    TestNodeKind::Element(tag) => tag.to_uppercase(),
                    TestNodeKind::Text(_) => "#text".to_string(),
                    TestNodeKind::Fragment => "#document-fragment".to_string(),
                };
                Ok(DomReadResult::String(name))
            }
            DomRead::NodeValue { node } => {
                let idx = node.0 as usize;
                if idx >= self.nodes.len() { return Err(HostError::InvalidHandle); }
                if let TestNodeKind::Text(s) = &self.nodes[idx].kind {
                    Ok(DomReadResult::String(s.clone()))
                } else {
                    Ok(DomReadResult::None)
                }
            }
            DomRead::TextContent { node } => {
                let idx = node.0 as usize;
                if idx >= self.nodes.len() { return Err(HostError::InvalidHandle); }
                Ok(DomReadResult::String(self.collect_text(idx)))
            }
            DomRead::InnerHtml { node } => {
                let idx = node.0 as usize;
                if idx >= self.nodes.len() { return Err(HostError::InvalidHandle); }
                Ok(DomReadResult::String(self.inner_html(idx)))
            }
            DomRead::OuterHtml { node } => {
                let idx = node.0 as usize;
                if idx >= self.nodes.len() { return Err(HostError::InvalidHandle); }
                Ok(DomReadResult::String(self.serialize_node(idx)))
            }
            DomRead::Attribute { node, name } => {
                let idx = node.0 as usize;
                if idx >= self.nodes.len() { return Err(HostError::InvalidHandle); }
                match self.nodes[idx].attrs.get(&name) {
                    Some(v) => Ok(DomReadResult::String(v.clone())),
                    None => Ok(DomReadResult::None),
                }
            }
            DomRead::AttributeNames { node } => {
                let idx = node.0 as usize;
                if idx >= self.nodes.len() { return Err(HostError::InvalidHandle); }
                let names: Vec<String> = self.nodes[idx].attrs.keys().cloned().collect();
                Ok(DomReadResult::StringList(names))
            }
            DomRead::Closest { node, selectors } => {
                let mut cur = node.0 as usize;
                loop {
                    if self.node_matches_selector(cur, &selectors) {
                        return Ok(DomReadResult::Node(NodeId(cur as u32)));
                    }
                    match self.nodes.get(cur).and_then(|n| n.parent) {
                        Some(p) => cur = p,
                        None => return Ok(DomReadResult::None),
                    }
                }
            }
            DomRead::ShadowRoot { .. } | DomRead::AssignedNodes { .. } => Ok(DomReadResult::None),
            DomRead::BoundingClientRect { .. } => {
                Ok(DomReadResult::Rect(tobira_engine::engine::DomRect { x: 0.0, y: 0.0, width: 100.0, height: 20.0 }))
            }
            DomRead::ScrollMetrics { .. } => {
                Ok(DomReadResult::ScrollMetrics(tobira_engine::engine::ScrollMetrics {
                    scroll_left: 0.0, scroll_top: 0.0,
                    scroll_width: 0.0, scroll_height: 0.0,
                    client_width: 0.0, client_height: 0.0,
                }))
            }
        }
    }

    fn mutate_dom(&mut self, mutation: DomMutation) -> HostResult<DomMutationResult> {
        match mutation {
            DomMutation::CreateElement { local_name, .. } => {
                let idx = self.push(TestNode::new_element(&local_name));
                Ok(DomMutationResult::Node(NodeId(idx as u32)))
            }
            DomMutation::CreateTextNode { data, .. } => {
                let idx = self.push(TestNode::new_text(&data));
                Ok(DomMutationResult::Node(NodeId(idx as u32)))
            }
            DomMutation::CreateDocumentFragment { .. } => {
                let idx = self.push(TestNode::new_fragment());
                Ok(DomMutationResult::Node(NodeId(idx as u32)))
            }
            DomMutation::CloneNode { node, deep } => {
                let idx = node.0 as usize;
                if idx >= self.nodes.len() { return Err(HostError::InvalidHandle); }
                let cloned = self.nodes[idx].clone();
                let new_idx = self.nodes.len();
                self.nodes.push(cloned);
                self.nodes[new_idx].parent = None;
                if !deep {
                    self.nodes[new_idx].children.clear();
                }
                Ok(DomMutationResult::Node(NodeId(new_idx as u32)))
            }
            DomMutation::SetTextContent { node, value } => {
                let idx = node.0 as usize;
                if idx >= self.nodes.len() { return Err(HostError::InvalidHandle); }
                // Remove existing children
                let old_children: Vec<usize> = self.nodes[idx].children.clone();
                for child in old_children { self.nodes[child].parent = None; }
                self.nodes[idx].children.clear();
                if !value.is_empty() {
                    let text_idx = self.push(TestNode::new_text(&value));
                    self.nodes[text_idx].parent = Some(idx);
                    self.nodes[idx].children.push(text_idx);
                }
                Ok(DomMutationResult::None)
            }
            DomMutation::SetInnerHtml { node, html } => {
                let idx = node.0 as usize;
                if idx >= self.nodes.len() { return Err(HostError::InvalidHandle); }
                self.parse_and_set_inner_html(idx, &html);
                Ok(DomMutationResult::None)
            }
            DomMutation::WriteHtml { .. } => Ok(DomMutationResult::None),
            // The TestDom doesn't exercise outerHTML/insertAdjacentHTML/focus —
            // the BrowserHost (engine_host.rs) covers them with the real DOM.
            DomMutation::SetOuterHtml { .. }
            | DomMutation::InsertAdjacentHtml { .. }
            | DomMutation::SplitText { .. }
            | DomMutation::NoteFocusChange { .. } => {
                Ok(DomMutationResult::None)
            }
            DomMutation::SetAttribute { node, name, value } => {
                let idx = node.0 as usize;
                if idx >= self.nodes.len() { return Err(HostError::InvalidHandle); }
                self.nodes[idx].attrs.insert(name, value);
                Ok(DomMutationResult::None)
            }
            DomMutation::RemoveAttribute { node, name } => {
                let idx = node.0 as usize;
                if idx >= self.nodes.len() { return Err(HostError::InvalidHandle); }
                self.nodes[idx].attrs.remove(&name);
                Ok(DomMutationResult::None)
            }
            DomMutation::ToggleAttribute { node, name, force } => {
                let idx = node.0 as usize;
                if idx >= self.nodes.len() { return Err(HostError::InvalidHandle); }
                let exists = self.nodes[idx].attrs.contains_key(&name);
                let should_add = force.unwrap_or(!exists);
                if should_add {
                    self.nodes[idx].attrs.insert(name, String::new());
                } else {
                    self.nodes[idx].attrs.remove(&name);
                }
                Ok(DomMutationResult::Bool(should_add))
            }
            DomMutation::Append { parent, children } => {
                let parent_idx = parent.0 as usize;
                if parent_idx >= self.nodes.len() { return Err(HostError::InvalidHandle); }
                for child in children {
                    let child_idx = child.0 as usize;
                    if child_idx >= self.nodes.len() { continue; }
                    self.detach(child_idx);
                    self.nodes[child_idx].parent = Some(parent_idx);
                    self.nodes[parent_idx].children.push(child_idx);
                }
                Ok(DomMutationResult::None)
            }
            DomMutation::Prepend { parent, children } => {
                let parent_idx = parent.0 as usize;
                if parent_idx >= self.nodes.len() { return Err(HostError::InvalidHandle); }
                let mut insert_pos = 0;
                for child in children {
                    let child_idx = child.0 as usize;
                    if child_idx >= self.nodes.len() { continue; }
                    self.detach(child_idx);
                    self.nodes[child_idx].parent = Some(parent_idx);
                    self.nodes[parent_idx].children.insert(insert_pos, child_idx);
                    insert_pos += 1;
                }
                Ok(DomMutationResult::None)
            }
            DomMutation::InsertBefore { parent, child, reference } => {
                let parent_idx = parent.0 as usize;
                let child_idx = child.0 as usize;
                if parent_idx >= self.nodes.len() || child_idx >= self.nodes.len() {
                    return Err(HostError::InvalidHandle);
                }
                self.detach(child_idx);
                self.nodes[child_idx].parent = Some(parent_idx);
                if let Some(ref_node) = reference {
                    let ref_idx = ref_node.0 as usize;
                    let pos = self.nodes[parent_idx].children.iter().position(|&c| c == ref_idx)
                        .unwrap_or(self.nodes[parent_idx].children.len());
                    self.nodes[parent_idx].children.insert(pos, child_idx);
                } else {
                    self.nodes[parent_idx].children.push(child_idx);
                }
                Ok(DomMutationResult::None)
            }
            DomMutation::ReplaceChild { parent, new_child, old_child } => {
                let parent_idx = parent.0 as usize;
                let new_idx = new_child.0 as usize;
                let old_idx = old_child.0 as usize;
                if parent_idx >= self.nodes.len() { return Err(HostError::InvalidHandle); }
                let pos = self.nodes[parent_idx].children.iter().position(|&c| c == old_idx);
                if let Some(pos) = pos {
                    self.detach(new_idx);
                    self.nodes[new_idx].parent = Some(parent_idx);
                    self.nodes[old_idx].parent = None;
                    self.nodes[parent_idx].children[pos] = new_idx;
                }
                Ok(DomMutationResult::None)
            }
            DomMutation::Remove { node } => {
                let idx = node.0 as usize;
                if idx >= self.nodes.len() { return Err(HostError::InvalidHandle); }
                self.detach(idx);
                Ok(DomMutationResult::None)
            }
            DomMutation::SetScrollOffset { .. } | DomMutation::SetWindowScroll { .. } => {
                Ok(DomMutationResult::None)
            }
            DomMutation::AttachShadow { .. } => Err(HostError::Unsupported),
        }
    }

    fn dispatch_dom_event(&mut self, _r: DomEventRequest) -> HostResult<DomEventResult> {
        Ok(DomEventResult { default_prevented: false })
    }

    fn console(&mut self, message: ConsoleMessage) -> HostResult<()> {
        eprintln!("[console] {}", message.parts.join(" "));
        Ok(())
    }

    fn schedule_timer(&mut self, _r: TimerRequest) -> HostResult<TimerId> { Ok(TimerId(0)) }
    fn cancel_timer(&mut self, _id: TimerId) -> HostResult<bool> { Ok(false) }
    fn request_animation_frame(&mut self, _w: WindowId) -> HostResult<FrameId> { Ok(FrameId(0)) }
    fn cancel_animation_frame(&mut self, _id: FrameId) -> HostResult<bool> { Ok(false) }
    fn fetch(&mut self, _r: FetchRequest) -> HostResult<NetworkRequestId> { Err(HostError::Unsupported) }
    fn fetch_sync(&mut self, request: FetchRequest) -> HostResult<FetchResponse> {
        // Canned response for tests: echoes the URL + a small JSON body.
        Ok(FetchResponse {
            final_url: request.url.clone(),
            status: 200,
            status_text: "OK".to_string(),
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            body: br#"{"message":"hello","n":42}"#.to_vec(),
        })
    }
    fn abort_fetch(&mut self, _id: NetworkRequestId) -> HostResult<bool> { Ok(false) }
    fn storage(&mut self, _op: StorageOp) -> HostResult<StorageResult> { Ok(StorageResult::None) }
    fn observer(&mut self, _op: ObserverOp) -> HostResult<ObserverResult> { Err(HostError::Unsupported) }
    fn now(&self) -> HostTimeSnapshot { HostTimeSnapshot { monotonic_ms: 0, unix_ms: 0 } }
    fn wait_for_host_events(&mut self, _ms: Option<u64>) -> HostResult<Vec<HostEvent>> { Ok(Vec::new()) }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_vm() -> Vm {
    Vm::with_host(Heap::new(), Box::new(TestDom::new()))
}

fn run(vm: &mut Vm, source: &str) {
    let program = Parser::new(source).parse().expect("parse");
    let chunk = Compiler::new(&program).compile().expect("compile");
    vm.execute(&chunk).expect("execute");
}

fn dom(vm: &mut Vm) -> &mut TestDom {
    vm.host_mut().as_any_mut().downcast_mut::<TestDom>().unwrap()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn fetch_resolves_with_response_text_and_json() {
    let mut vm = make_vm();
    // The fetch chains settle through the microtask queue, which `execute`
    // drains; results are stashed on globals and asserted synchronously after.
    run(
        &mut vm,
        r#"
        var __status = 0; var __ok = false; var __ctype = '';
        var __text = ''; var __msg = ''; var __n = 0;
        fetch('http://example.test/data').then(function (r) {
            __status = r.status;
            __ok = r.ok;
            __ctype = r.headers.get('Content-Type');
            return r.text();
        }).then(function (t) { __text = t; });
        fetch('http://example.test/data').then(function (r) {
            return r.json();
        }).then(function (d) { __msg = d.message; __n = d.n; });
    "#,
    );
    run(
        &mut vm,
        r#"
        assert(__status === 200);
        assert(__ok === true);
        assert(__ctype === 'application/json');
        assert(__text === '{"message":"hello","n":42}');
        assert(__msg === 'hello');
        assert(__n === 42);
    "#,
    );
}

#[test]
fn fetch_rejects_when_host_has_no_network() {
    // NoopHost uses the default fetch_sync (Unsupported) -> the promise rejects.
    let mut vm = Vm::new(Heap::new());
    run(
        &mut vm,
        r#"
        var __rejected = false; var __isTypeError = false;
        fetch('http://example.test/').then(
            function () {},
            function (e) { __rejected = true; __isTypeError = (e instanceof TypeError); }
        );
    "#,
    );
    run(
        &mut vm,
        r#"
        assert(__rejected === true);
        assert(__isTypeError === true);
    "#,
    );
}

#[test]
fn fetch_with_await() {
    let mut vm = make_vm();
    run(
        &mut vm,
        r#"
        var __awaited = '';
        (async function () {
            const r = await fetch('http://example.test/data');
            const d = await r.json();
            __awaited = d.message + '!' + d.n;
        })();
    "#,
    );
    run(&mut vm, r#"assert(__awaited === 'hello!42');"#);
}

/// 1. document.createElement returns a non-null value.
#[test]
fn document_create_element() {
    let mut vm = make_vm();
    run(&mut vm, r#"
        const div = document.createElement('div');
        assert(div !== null);
        assert(div !== undefined);
    "#);
}

/// 2. Setting innerHTML on an element parses HTML and the text is accessible.
#[test]
fn set_inner_html() {
    let mut vm = make_vm();
    run(&mut vm, r#"
        const div = document.createElement('div');
        div.innerHTML = '<p>hello</p>';
    "#);
    // verify in the TestDom that the div has a child <p> with text "hello"
    let d = dom(&mut vm);
    // The div is the most recently created element; find it via query in the arena.
    // It won't be attached to the tree, but we can scan directly.
    let div_idx = d.nodes.iter().rposition(|n| matches!(&n.kind, TestNodeKind::Element(t) if t == "div")).unwrap();
    let text = d.collect_text(div_idx);
    assert_eq!(text, "hello", "text content should be 'hello'");
}

/// 3. Setting innerHTML and reading it back round-trips.
#[test]
fn get_inner_html() {
    let mut vm = make_vm();
    run(&mut vm, r#"
        const div = document.createElement('div');
        div.innerHTML = '<span>world</span>';
        assert(div.innerHTML.includes('world'));
    "#);
}

/// 4. appendChild attaches a child to document.body.
#[test]
fn append_child() {
    let mut vm = make_vm();
    run(&mut vm, r#"
        const div = document.createElement('div');
        document.body.appendChild(div);
        const children = document.body.children;
        assert(children.length > 0);
    "#);
    let d = dom(&mut vm);
    let body_idx = d.body_idx();
    assert!(!d.nodes[body_idx].children.is_empty(), "body should have children");
}

/// 5. querySelector with '#id' returns the matching element.
#[test]
fn query_selector() {
    let mut vm = make_vm();
    run(&mut vm, r#"
        const el = document.createElement('div');
        el.setAttribute('id', 'myid');
        document.body.appendChild(el);
        const found = document.querySelector('#myid');
        assert(found !== null);
    "#);
}

/// 6. classList.add and classList.remove modify className.
#[test]
fn classlist_add_remove() {
    let mut vm = make_vm();
    run(&mut vm, r#"
        const el = document.createElement('div');
        el.classList.add('foo');
        assert(el.className.includes('foo'));
        el.classList.remove('foo');
        assert(!el.className.includes('foo'));
    "#);
}

/// 7. classList.toggle and classList.contains work correctly.
#[test]
fn classlist_contains_toggle() {
    let mut vm = make_vm();
    run(&mut vm, r#"
        const el = document.createElement('span');
        assert(!el.classList.contains('active'));
        const added = el.classList.toggle('active');
        assert(added === true);
        assert(el.classList.contains('active'));
        const removed = el.classList.toggle('active');
        assert(removed === false);
        assert(!el.classList.contains('active'));
    "#);
}

/// 8. setAttribute / getAttribute round-trips a value.
#[test]
fn set_attribute_get_attribute() {
    let mut vm = make_vm();
    run(&mut vm, r#"
        const el = document.createElement('input');
        el.setAttribute('type', 'text');
        el.setAttribute('placeholder', 'enter value');
        assert(el.getAttribute('type') === 'text');
        assert(el.getAttribute('placeholder') === 'enter value');
        assert(el.getAttribute('missing') === null);
    "#);
}

/// 9. createTextNode — text node textContent round-trip.
#[test]
fn create_text_node() {
    let mut vm = make_vm();
    run(&mut vm, r#"
        const tn = document.createTextNode('hello text');
        assert(tn.textContent === 'hello text');
    "#);
    // also verify in the TestDom arena
    let d = dom(&mut vm);
    let text_idx = d.nodes.iter().rposition(|n| matches!(&n.kind, TestNodeKind::Text(s) if s == "hello text")).unwrap();
    assert!(matches!(&d.nodes[text_idx].kind, TestNodeKind::Text(s) if s == "hello text"));
}

/// 10. element.remove() detaches the element from its parent.
#[test]
fn element_remove() {
    let mut vm = make_vm();
    run(&mut vm, r#"
        const el = document.createElement('div');
        document.body.appendChild(el);
        // Verify it was appended
        assert(document.body.children.length > 0);
        el.remove();
        // After removal the body children should not include el anymore.
        // We check that querying yields different count or null.
    "#);
    let d = dom(&mut vm);
    let body_idx = d.body_idx();
    // The div was appended then removed; body children should be empty.
    let has_divs = d.nodes[body_idx].children.iter().any(|&c| {
        matches!(&d.nodes[c].kind, TestNodeKind::Element(t) if t == "div")
    });
    assert!(!has_divs, "div should have been removed from body");
}

// ---------------------------------------------------------------------------
// XMLHttpRequest (synchronous under the hood via Host::fetch_sync)
// ---------------------------------------------------------------------------

#[test]
fn xhr_basic_get_fires_onload() {
    let mut vm = make_vm();
    run(&mut vm, r#"
        let out = '';
        const xhr = new XMLHttpRequest();
        xhr.open('GET', '/data.json');
        xhr.onload = function () {
            out = xhr.status + '|' + xhr.readyState + '|' + xhr.responseText;
        };
        xhr.send();
    "#);
    run(&mut vm, r#"assert(out === '200|4|{"message":"hello","n":42}', 'got: ' + out);"#);
}

#[test]
fn xhr_onreadystatechange_at_done() {
    let mut vm = make_vm();
    run(&mut vm, r#"
        let done = false;
        const xhr = new XMLHttpRequest();
        xhr.open('GET', '/x');
        xhr.onreadystatechange = function () {
            if (xhr.readyState === 4 && xhr.status === 200) done = true;
        };
        xhr.send();
    "#);
    run(&mut vm, "assert(done === true);");
}

#[test]
fn xhr_response_type_json() {
    let mut vm = make_vm();
    run(&mut vm, r#"
        let n = 0; let msg = '';
        const xhr = new XMLHttpRequest();
        xhr.responseType = 'json';
        xhr.open('GET', '/x');
        xhr.onload = function () { n = xhr.response.n; msg = xhr.response.message; };
        xhr.send();
    "#);
    run(&mut vm, "assert(n === 42 && msg === 'hello');");
}

#[test]
fn xhr_get_response_header() {
    let mut vm = make_vm();
    run(&mut vm, r#"
        let ct = '';
        const xhr = new XMLHttpRequest();
        xhr.open('GET', '/x');
        xhr.onload = function () { ct = xhr.getResponseHeader('Content-Type'); };
        xhr.send();
    "#);
    run(&mut vm, "assert(ct === 'application/json', 'got: ' + ct);");
}

#[test]
fn xhr_post_with_request_header() {
    let mut vm = make_vm();
    // The canned host echoes a fixed body; here we just verify the POST path
    // runs end to end (open/setRequestHeader/send/onload) without error.
    run(&mut vm, r#"
        let ok = false;
        const xhr = new XMLHttpRequest();
        xhr.open('POST', '/submit');
        xhr.setRequestHeader('Content-Type', 'application/json');
        xhr.onload = function () { ok = xhr.status === 200; };
        xhr.send(JSON.stringify({ a: 1 }));
    "#);
    run(&mut vm, "assert(ok === true);");
}
