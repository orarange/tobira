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
use std::collections::BTreeMap;

use tobira_engine::engine::{
    Compiler, ConsoleMessage, DomEventRequest, DomEventResult, DomMutation, DomMutationResult,
    DomRead, DomReadResult, DomRect, FetchRequest, FrameId, Heap, HistoryAction, HistoryOutcome,
    Host, HostError, HostEvent, HostResult, HostTimeSnapshot, LocationSnapshot, NavigationAction,
    NavigationOutcome, NetworkRequestId, NodeId, NodeKind, ObserverOp, ObserverResult, Parser,
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

/// A `Host` implementation backed by an arena DOM built from parsed HTML.
pub struct BrowserHost {
    nodes: Vec<DomNode>,
    document: usize,
    html: usize,
    head: usize,
    body: usize,
    console: Vec<String>,
    location: LocationSnapshot,
    navigation: Option<String>,
}

impl BrowserHost {
    /// Build a host with an arena DOM parsed from `html`, located at `url`.
    pub fn from_html(html: &str, url: &str) -> Self {
        let root = parse_document(html);
        let mut host = Self {
            nodes: Vec::new(),
            document: 0,
            html: 0,
            head: 0,
            body: 0,
            console: Vec::new(),
            location: location_from_url(url),
            navigation: None,
        };
        host.document = host.push(DomNode::document());
        // The parsed root is an "document" element; graft its children under our
        // document node, then make sure html/head/body exist.
        if let Node::Element(element) = &root {
            for child in &element.children {
                let child_idx = host.build_from_node(child);
                host.attach(host.document, child_idx);
            }
        }
        host.ensure_skeleton();
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

    /// Make sure <html>, <head>, <body> exist and record their indices.
    fn ensure_skeleton(&mut self) {
        self.html = self
            .find_descendant_tag(self.document, "html")
            .unwrap_or_else(|| {
                let idx = self.push(DomNode::element("html"));
                self.attach(self.document, idx);
                idx
            });
        self.head = self.find_descendant_tag(self.html, "head").unwrap_or_else(|| {
            let idx = self.push(DomNode::element("head"));
            // head goes first under html
            self.nodes[idx].parent = Some(self.html);
            self.nodes[self.html].children.insert(0, idx);
            idx
        });
        self.body = self.find_descendant_tag(self.html, "body").unwrap_or_else(|| {
            let idx = self.push(DomNode::element("body"));
            self.attach(self.html, idx);
            idx
        });
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
                out.push_str(&self.inner_html(idx));
                out.push_str(&format!("</{tag}>"));
                out
            }
            DomNodeKind::Document | DomNodeKind::Fragment => self.inner_html(idx),
        }
    }

    /// Serialize the whole document back to HTML (for the page snapshot).
    pub fn serialize_document(&self) -> String {
        self.serialize_node(self.document)
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
            // The path may carry a query string; split it off for `search`.
            let (pathname, search) = match u.path.split_once('?') {
                Some((path, query)) => (path.to_string(), format!("?{query}")),
                None => (u.path.clone(), String::new()),
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
                hash: String::new(),
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
            inner_width: 1280.0,
            inner_height: 720.0,
            scroll_x: 0.0,
            scroll_y: 0.0,
            device_pixel_ratio: 1.0,
        })
    }

    fn location(&self, _window: WindowId) -> HostResult<LocationSnapshot> {
        Ok(self.location.clone())
    }

    fn navigate(&mut self, action: NavigationAction) -> HostResult<NavigationOutcome> {
        if let NavigationAction::Navigate { url, .. } = action {
            self.navigation = Some(url);
            Ok(NavigationOutcome {
                committed: true,
                same_document: false,
            })
        } else {
            Ok(NavigationOutcome {
                committed: false,
                same_document: true,
            })
        }
    }

    fn history(&mut self, _action: HistoryAction) -> HostResult<HistoryOutcome> {
        Ok(HistoryOutcome {
            href: self.location.href.clone(),
            state: None,
            length: 1,
            restored_scroll_y: None,
        })
    }

    fn read_dom(&self, read: DomRead) -> HostResult<DomReadResult> {
        let node_exists = |idx: usize| idx < self.nodes.len();
        match read {
            DomRead::DocumentRoot { .. } => Ok(DomReadResult::Node(NodeId(self.document as u32))),
            DomRead::DocumentHead { .. } => Ok(DomReadResult::Node(NodeId(self.head as u32))),
            DomRead::DocumentBody { .. } => Ok(DomReadResult::Node(NodeId(self.body as u32))),
            DomRead::ActiveElement { .. } => Ok(DomReadResult::None),
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
                    DomNodeKind::Fragment => NodeKind::DocumentFragment,
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
            DomRead::ShadowRoot { .. } | DomRead::AssignedNodes { .. } => Ok(DomReadResult::None),
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
                let old: Vec<usize> = self.nodes[idx].children.clone();
                for child in old {
                    self.nodes[child].parent = None;
                }
                self.nodes[idx].children.clear();
                if !value.is_empty() {
                    let text = self.push(DomNode::text(&value));
                    self.attach(idx, text);
                }
                Ok(DomMutationResult::None)
            }
            DomMutation::SetInnerHtml { node, html } => {
                let idx = node.0 as usize;
                if !exists(&self.nodes, idx) {
                    return Err(HostError::InvalidHandle);
                }
                self.parse_and_set_inner_html(idx, &html);
                Ok(DomMutationResult::None)
            }
            DomMutation::WriteHtml { html, .. } => {
                // document.write appends parsed content to <body>.
                let body = self.body;
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
                self.nodes[idx].attrs.insert(name, value);
                Ok(DomMutationResult::None)
            }
            DomMutation::RemoveAttribute { node, name } => {
                let idx = node.0 as usize;
                if !exists(&self.nodes, idx) {
                    return Err(HostError::InvalidHandle);
                }
                self.nodes[idx].attrs.remove(&name);
                Ok(DomMutationResult::None)
            }
            DomMutation::ToggleAttribute { node, name, force } => {
                let idx = node.0 as usize;
                if !exists(&self.nodes, idx) {
                    return Err(HostError::InvalidHandle);
                }
                let present = self.nodes[idx].attrs.contains_key(&name);
                let add = force.unwrap_or(!present);
                if add {
                    self.nodes[idx].attrs.entry(name).or_default();
                } else {
                    self.nodes[idx].attrs.remove(&name);
                }
                Ok(DomMutationResult::Bool(add))
            }
            DomMutation::Append { parent, children } => {
                let parent_idx = parent.0 as usize;
                if !exists(&self.nodes, parent_idx) {
                    return Err(HostError::InvalidHandle);
                }
                for child in children {
                    let child_idx = child.0 as usize;
                    if !exists(&self.nodes, child_idx) {
                        continue;
                    }
                    self.detach(child_idx);
                    self.attach(parent_idx, child_idx);
                }
                Ok(DomMutationResult::None)
            }
            DomMutation::Prepend { parent, children } => {
                let parent_idx = parent.0 as usize;
                if !exists(&self.nodes, parent_idx) {
                    return Err(HostError::InvalidHandle);
                }
                let mut pos = 0;
                for child in children {
                    let child_idx = child.0 as usize;
                    if !exists(&self.nodes, child_idx) {
                        continue;
                    }
                    self.detach(child_idx);
                    self.nodes[child_idx].parent = Some(parent_idx);
                    self.nodes[parent_idx].children.insert(pos, child_idx);
                    pos += 1;
                }
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
                self.detach(child_idx);
                self.nodes[child_idx].parent = Some(parent_idx);
                match reference {
                    Some(reference) => {
                        let pos = self.nodes[parent_idx]
                            .children
                            .iter()
                            .position(|&c| c == reference.0 as usize)
                            .unwrap_or(self.nodes[parent_idx].children.len());
                        self.nodes[parent_idx].children.insert(pos, child_idx);
                    }
                    None => self.nodes[parent_idx].children.push(child_idx),
                }
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
                if !exists(&self.nodes, parent_idx) {
                    return Err(HostError::InvalidHandle);
                }
                if let Some(pos) = self.nodes[parent_idx]
                    .children
                    .iter()
                    .position(|&c| c == old_idx)
                {
                    self.detach(new_idx);
                    self.nodes[new_idx].parent = Some(parent_idx);
                    self.nodes[old_idx].parent = None;
                    self.nodes[parent_idx].children[pos] = new_idx;
                }
                Ok(DomMutationResult::None)
            }
            DomMutation::Remove { node } => {
                let idx = node.0 as usize;
                if !exists(&self.nodes, idx) {
                    return Err(HostError::InvalidHandle);
                }
                self.detach(idx);
                Ok(DomMutationResult::None)
            }
            DomMutation::SetScrollOffset { .. } | DomMutation::SetWindowScroll { .. } => {
                Ok(DomMutationResult::None)
            }
            DomMutation::AttachShadow { .. } => Err(HostError::Unsupported),
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
    fn abort_fetch(&mut self, _id: NetworkRequestId) -> HostResult<bool> {
        Ok(false)
    }
    fn storage(&mut self, _op: StorageOp) -> HostResult<StorageResult> {
        Ok(StorageResult::None)
    }
    fn observer(&mut self, _op: ObserverOp) -> HostResult<ObserverResult> {
        Err(HostError::Unsupported)
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
    pub error: Option<String>,
}

/// Parse `html`, run its inline `<script>`s on the self-built engine against a
/// `BrowserHost` DOM, and return the resulting HTML snapshot + console output.
///
/// This is the engine-backed counterpart of the boa path in `js.rs`, used behind
/// the `TOBIRA_ENGINE` flag while parity is built up.
pub fn run_document_scripts(html: &str, url: &str) -> EngineRunResult {
    let host = BrowserHost::from_html(html, url);
    let scripts = host.inline_scripts();
    let mut vm = Vm::with_host(Heap::new(), Box::new(host));

    let mut error = None;
    for script in &scripts {
        match Parser::new(script).parse() {
            Ok(program) => match Compiler::new(&program).compile() {
                Ok(chunk) => {
                    if let Err(e) = vm.execute(&chunk) {
                        error = Some(format!("{e:?}"));
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

    let host = vm
        .host_mut()
        .as_any_mut()
        .downcast_mut::<BrowserHost>()
        .expect("host is a BrowserHost");
    EngineRunResult {
        html: host.serialize_document(),
        console_logs: host.take_console(),
        title: host.title(),
        navigation_target: host.navigation_target(),
        error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
