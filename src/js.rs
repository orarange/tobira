use std::cell::RefCell;
use std::collections::BTreeMap;
use std::mem;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use boa_engine::object::{ObjectInitializer, builtins::{JsFunction, JsPromise}};
use boa_engine::property::Attribute;
use boa_engine::{
    Context, Finalize, JsData, JsNativeError, JsResult, JsValue, NativeFunction, Source, Trace,
    js_string,
};

use crate::html::{Node, parse_document};
use crate::http::fetch;
use crate::text::decode_text_response;
use crate::url::Url;

const MAX_SCRIPT_SOURCE_BYTES: usize = 2 * 1024 * 1024;
const MAX_TOTAL_SCRIPT_BYTES: usize = 16 * 1024 * 1024;
const MAX_SCRIPT_ITERATIONS: usize = 1024;
const JS_THREAD_STACK_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProcessedScriptHtml {
    pub html: String,
    pub title_override: Option<String>,
    pub console_logs: Vec<String>,
    pub navigation_target: Option<String>,
}

#[derive(Debug, Default)]
struct JavaScriptState {
    current_title: String,
    title_dirty: bool,
    write_buffer: String,
    console_logs: Vec<String>,
    location_href: String,
    current_script: Option<usize>,
    dom: DomState,
}

#[derive(Debug, Trace, Finalize, JsData)]
struct JavaScriptHostData {
    #[unsafe_ignore_trace]
    state: RefCell<JavaScriptState>,
}

#[derive(Debug, Clone, Default)]
struct DomState {
    nodes: Vec<DomNode>,
    document_id: usize,
    html_id: Option<usize>,
    head_id: Option<usize>,
    body_id: Option<usize>,
}

#[derive(Debug, Clone)]
struct DomNode {
    parent: Option<usize>,
    children: Vec<usize>,
    kind: DomNodeKind,
}

#[derive(Debug, Clone)]
enum DomNodeKind {
    Element(DomElementData),
    Text(String),
}

#[derive(Debug, Clone, Default)]
struct DomElementData {
    tag_name: String,
    attributes: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Trace, Finalize, JsData)]
struct DomNodeHandle {
    #[unsafe_ignore_trace]
    node_id: usize,
}

#[derive(Debug, Clone, Trace, Finalize, JsData)]
struct DomNodeListHandle {
    #[unsafe_ignore_trace]
    node_ids: Vec<usize>,
}

#[derive(Debug, Clone, Trace, Finalize, JsData)]
struct DomClassListHandle {
    #[unsafe_ignore_trace]
    node_id: usize,
}

#[derive(Debug, Clone, Trace, Finalize, JsData)]
struct FetchResponseHandle {
    #[unsafe_ignore_trace]
    response: crate::http::HttpResponse,
}

#[derive(Debug, Clone, Trace, Finalize, JsData)]
struct ResponseHeadersHandle {
    #[unsafe_ignore_trace]
    headers: BTreeMap<String, String>,
}

#[derive(Debug, Trace, Finalize, JsData)]
struct XmlHttpRequestHandle {
    #[unsafe_ignore_trace]
    state: RefCell<XmlHttpRequestState>,
}

#[derive(Debug, Default)]
struct XmlHttpRequestState {
    method: String,
    url: Option<String>,
    request_headers: BTreeMap<String, String>,
    ready_state: u8,
    status: u16,
    status_text: String,
    response_text: String,
    response_url: String,
}

pub fn process_document_scripts(html: &str, base_url: &Url) -> ProcessedScriptHtml {
    let html_owned = html.to_string();
    let base_url_owned = base_url.clone();
    let worker = thread::Builder::new()
        .name("tobira-js".to_string())
        .stack_size(JS_THREAD_STACK_BYTES)
        .spawn({
            let html = html_owned.clone();
            let base_url = base_url_owned.clone();
            move || process_document_scripts_impl(&html, &base_url)
        });

    match worker {
        Ok(handle) => match handle.join() {
            Ok(processed) => processed,
            Err(_) => ProcessedScriptHtml {
                html: html_owned,
                title_override: None,
                console_logs: vec!["js error: runtime worker panicked".to_string()],
                navigation_target: None,
            },
        },
        Err(_) => process_document_scripts_impl(html, base_url),
    }
}

fn process_document_scripts_impl(html: &str, base_url: &Url) -> ProcessedScriptHtml {
    let mut runtime = JavaScriptRuntime::new(base_url, html);
    let mut iterations = 0;

    while let Some((script_id, attributes, inline_source)) = runtime.next_script() {
        iterations += 1;
        if iterations > MAX_SCRIPT_ITERATIONS {
            runtime.push_log("js skip: script iteration limit reached".to_string());
            break;
        }
        if let Some(source) = load_script_source(&inline_source, &attributes, base_url) {
            runtime.set_current_script(script_id);
            runtime.execute(&source);
            runtime.flush_document_writes(script_id);
            runtime.clear_current_script();
        }
        runtime.remove_script_node(script_id);
    }

    let html = runtime.serialize_html();
    let title_override = runtime.title_override();

    ProcessedScriptHtml {
        html,
        title_override,
        console_logs: runtime.take_logs(),
        navigation_target: runtime.navigation_target(base_url),
    }
}

struct JavaScriptRuntime {
    context: Context,
    executed_bytes: usize,
    host: String,
}

impl JavaScriptRuntime {
    fn new(base_url: &Url, html: &str) -> Self {
        let mut context = Context::default();
        let dom = DomState::from_html(html);
        let initial_title = dom.title_text().unwrap_or_default();
        context.insert_data(JavaScriptHostData {
            state: RefCell::new(JavaScriptState {
                current_title: initial_title,
                title_dirty: false,
                write_buffer: String::new(),
                console_logs: Vec::new(),
                location_href: base_url.to_string(),
                current_script: None,
                dom,
            }),
        });

        install_browser_globals(&mut context);

        Self {
            context,
            executed_bytes: 0,
            host: base_url.host.to_ascii_lowercase(),
        }
    }

    fn execute(&mut self, source: &str) {
        if source.trim().is_empty() {
            return;
        }

        if !is_supported_script_source(source, &self.host) {
            self.push_log(format!(
                "js skip: script policy rejected source ({} bytes)",
                source.len()
            ));
            return;
        }

        if self.executed_bytes.saturating_add(source.len()) > MAX_TOTAL_SCRIPT_BYTES {
            self.push_log(format!(
                "js skip: script budget exceeded at {} bytes",
                self.executed_bytes.saturating_add(source.len())
            ));
            return;
        }

        self.executed_bytes = self.executed_bytes.saturating_add(source.len());

        match self.context.eval(Source::from_bytes(source)) {
            Ok(_) => {
                if let Err(error) = self.context.run_jobs() {
                    self.push_log(format!("js job error: {error}"));
                }
            }
            Err(error) => self.push_log(format!("js error: {error}")),
        }
    }

    fn take_written_html(&self) -> String {
        let Some(host) = self.context.get_data::<JavaScriptHostData>() else {
            return String::new();
        };

        mem::take(&mut host.state.borrow_mut().write_buffer)
    }

    fn title_override(&self) -> Option<String> {
        let host = self.context.get_data::<JavaScriptHostData>()?;
        let state = host.state.borrow();
        state
            .dom
            .title_text()
            .or_else(|| state.title_dirty.then(|| state.current_title.clone()))
    }

    fn take_logs(&self) -> Vec<String> {
        let Some(host) = self.context.get_data::<JavaScriptHostData>() else {
            return Vec::new();
        };

        mem::take(&mut host.state.borrow_mut().console_logs)
    }

    fn push_log(&self, message: String) {
        if let Some(host) = self.context.get_data::<JavaScriptHostData>() {
            host.state.borrow_mut().console_logs.push(message);
        }
    }

    fn navigation_target(&self, base_url: &Url) -> Option<String> {
        let host = self.context.get_data::<JavaScriptHostData>()?;
        let href = host.state.borrow().location_href.clone();
        (href != base_url.to_string()).then_some(href)
    }

    fn next_script(&self) -> Option<(usize, BTreeMap<String, String>, String)> {
        let host = self.context.get_data::<JavaScriptHostData>()?;
        let state = host.state.borrow();
        let script_id = state
            .dom
            .descendant_nodes(state.dom.document_id, true)
            .into_iter()
            .find(|id| state.dom.should_execute_script_node(*id))?;
        Some((
            script_id,
            state.dom.node_attributes(script_id).cloned().unwrap_or_default(),
            state.dom.script_inline_source(script_id).unwrap_or_default(),
        ))
    }

    fn set_current_script(&self, script_id: usize) {
        if let Some(host) = self.context.get_data::<JavaScriptHostData>() {
            host.state.borrow_mut().current_script = Some(script_id);
        }
    }

    fn clear_current_script(&self) {
        if let Some(host) = self.context.get_data::<JavaScriptHostData>() {
            host.state.borrow_mut().current_script = None;
        }
    }

    fn flush_document_writes(&self, script_id: usize) {
        let written = self.take_written_html();
        if written.trim().is_empty() {
            return;
        }
        if let Some(host) = self.context.get_data::<JavaScriptHostData>() {
            host.state
                .borrow_mut()
                .dom
                .insert_fragment_before(script_id, &written);
        }
    }

    fn remove_script_node(&self, script_id: usize) {
        if let Some(host) = self.context.get_data::<JavaScriptHostData>() {
            host.state.borrow_mut().dom.detach_node(script_id);
        }
    }

    fn serialize_html(&self) -> String {
        self.context
            .get_data::<JavaScriptHostData>()
            .map(|host| host.state.borrow().dom.serialize_document())
            .unwrap_or_default()
    }
}

impl DomState {
    fn from_html(html: &str) -> Self {
        let mut dom = Self::default();
        let document = parse_document(html);
        let document_id = dom.push_node(None, &document);
        dom.document_id = document_id;
        dom.html_id = dom.find_first_tag(document_id, "html");
        dom.head_id = dom.find_first_tag(document_id, "head");
        dom.body_id = dom.find_first_tag(document_id, "body");
        dom
    }

    fn push_node(&mut self, parent: Option<usize>, node: &Node) -> usize {
        let node_id = self.nodes.len();
        let kind = match node {
            Node::Text(text) => DomNodeKind::Text(text.clone()),
            Node::Element(element) => DomNodeKind::Element(DomElementData {
                tag_name: element.tag_name.clone(),
                attributes: element.attributes.clone(),
            }),
        };
        self.nodes.push(DomNode {
            parent,
            children: Vec::new(),
            kind,
        });

        if let Node::Element(element) = node {
            let mut child_ids = Vec::new();
            for child in &element.children {
                let child_id = self.push_node(Some(node_id), child);
                child_ids.push(child_id);
            }
            self.nodes[node_id].children = child_ids;
        }

        node_id
    }

    fn create_element(&mut self, tag_name: &str) -> usize {
        let node_id = self.nodes.len();
        self.nodes.push(DomNode {
            parent: None,
            children: Vec::new(),
            kind: DomNodeKind::Element(DomElementData {
                tag_name: tag_name.to_ascii_lowercase(),
                attributes: BTreeMap::new(),
            }),
        });
        node_id
    }

    fn create_text_node(&mut self, text: &str) -> usize {
        let node_id = self.nodes.len();
        self.nodes.push(DomNode {
            parent: None,
            children: Vec::new(),
            kind: DomNodeKind::Text(text.to_string()),
        });
        node_id
    }

    fn node(&self, node_id: usize) -> Option<&DomNode> {
        self.nodes.get(node_id)
    }

    fn node_mut(&mut self, node_id: usize) -> Option<&mut DomNode> {
        self.nodes.get_mut(node_id)
    }

    fn element(&self, node_id: usize) -> Option<&DomElementData> {
        match &self.node(node_id)?.kind {
            DomNodeKind::Element(element) => Some(element),
            DomNodeKind::Text(_) => None,
        }
    }

    fn element_mut(&mut self, node_id: usize) -> Option<&mut DomElementData> {
        match &mut self.node_mut(node_id)?.kind {
            DomNodeKind::Element(element) => Some(element),
            DomNodeKind::Text(_) => None,
        }
    }

    fn find_first_tag(&self, start_id: usize, tag_name: &str) -> Option<usize> {
        let mut stack = vec![start_id];
        while let Some(node_id) = stack.pop() {
            if self
                .element(node_id)
                .map(|element| element.tag_name == tag_name)
                .unwrap_or(false)
            {
                return Some(node_id);
            }
            if let Some(node) = self.node(node_id) {
                for child_id in node.children.iter().rev() {
                    stack.push(*child_id);
                }
            }
        }
        None
    }

    fn title_text(&self) -> Option<String> {
        let title_id = self.find_first_tag(self.document_id, "title")?;
        let text = self.text_content(title_id);
        (!text.trim().is_empty()).then_some(text)
    }

    fn set_title_text(&mut self, title: &str) {
        let title_id = if let Some(existing) = self.find_first_tag(self.document_id, "title") {
            existing
        } else {
            let head_id = self.ensure_head_node();
            let title_id = self.create_element("title");
            self.append_child(head_id, title_id);
            title_id
        };
        self.set_text_content(title_id, title);
    }

    fn ensure_head_node(&mut self) -> usize {
        if let Some(head_id) = self.head_id {
            return head_id;
        }
        let parent_id = self.html_id.unwrap_or(self.document_id);
        let head_id = self.create_element("head");
        self.append_child(parent_id, head_id);
        self.head_id = Some(head_id);
        head_id
    }

    fn script_inline_source(&self, node_id: usize) -> Option<String> {
        let element = self.element(node_id)?;
        (element.tag_name == "script").then(|| self.raw_text(node_id))
    }

    fn node_attributes(&self, node_id: usize) -> Option<&BTreeMap<String, String>> {
        Some(&self.element(node_id)?.attributes)
    }

    fn should_execute_script_node(&self, node_id: usize) -> bool {
        self.element(node_id)
            .filter(|element| element.tag_name == "script")
            .and_then(|_| self.node_attributes(node_id))
            .map(should_execute_script)
            .unwrap_or(false)
    }

    fn descendant_nodes(&self, scope_id: usize, include_scope: bool) -> Vec<usize> {
        let mut output = Vec::new();
        let mut stack = vec![scope_id];
        while let Some(node_id) = stack.pop() {
            if include_scope || node_id != scope_id {
                output.push(node_id);
            }
            if let Some(node) = self.node(node_id) {
                for child_id in node.children.iter().rev() {
                    stack.push(*child_id);
                }
            }
        }
        output
    }

    fn detach_node(&mut self, node_id: usize) {
        let parent_id = self.node(node_id).and_then(|node| node.parent);
        if let Some(parent_id) = parent_id
            && let Some(parent) = self.node_mut(parent_id)
        {
            parent.children.retain(|child_id| *child_id != node_id);
        }
        if let Some(node) = self.node_mut(node_id) {
            node.parent = None;
        }
    }

    fn append_child(&mut self, parent_id: usize, child_id: usize) {
        self.detach_node(child_id);
        if let Some(parent) = self.node_mut(parent_id) {
            parent.children.push(child_id);
        }
        if let Some(child) = self.node_mut(child_id) {
            child.parent = Some(parent_id);
        }
    }

    fn insert_before(&mut self, parent_id: usize, child_id: usize, before_id: Option<usize>) {
        self.detach_node(child_id);
        let insert_index = before_id
            .and_then(|before_id| {
                self.node(parent_id)
                    .and_then(|parent| parent.children.iter().position(|id| *id == before_id))
            })
            .unwrap_or_else(|| self.node(parent_id).map(|parent| parent.children.len()).unwrap_or(0));
        if let Some(parent) = self.node_mut(parent_id) {
            parent.children.insert(insert_index, child_id);
        }
        if let Some(child) = self.node_mut(child_id) {
            child.parent = Some(parent_id);
        }
    }

    fn insert_fragment_before(&mut self, target_id: usize, html: &str) {
        let parent_id = self.node(target_id).and_then(|node| node.parent);
        let Some(parent_id) = parent_id else {
            return;
        };
        let fragment = parse_document(html);
        let fragment_root_id = self.push_node(None, &fragment);
        let fragment_children = self
            .node(fragment_root_id)
            .map(|node| node.children.clone())
            .unwrap_or_default();
        for child_id in fragment_children {
            self.insert_before(parent_id, child_id, Some(target_id));
        }
    }

    fn replace_children_with_fragment(&mut self, node_id: usize, html: &str) {
        let fragment = parse_document(html);
        let fragment_root_id = self.push_node(None, &fragment);
        let fragment_children = self
            .node(fragment_root_id)
            .map(|node| node.children.clone())
            .unwrap_or_default();
        self.replace_children(node_id, fragment_children);
    }

    fn replace_children(&mut self, node_id: usize, children: Vec<usize>) {
        let previous = self
            .node(node_id)
            .map(|node| node.children.clone())
            .unwrap_or_default();
        for child_id in previous {
            if let Some(child) = self.node_mut(child_id) {
                child.parent = None;
            }
        }
        if let Some(node) = self.node_mut(node_id) {
            node.children.clear();
        }
        for child_id in children {
            self.append_child(node_id, child_id);
        }
    }

    fn text_content(&self, node_id: usize) -> String {
        let Some(node) = self.node(node_id) else {
            return String::new();
        };
        match &node.kind {
            DomNodeKind::Text(text) => text.clone(),
            DomNodeKind::Element(_) => node
                .children
                .iter()
                .map(|child_id| self.text_content(*child_id))
                .collect::<String>(),
        }
    }

    fn set_text_content(&mut self, node_id: usize, text: &str) {
        let text_id = self.create_text_node(text);
        self.replace_children(node_id, vec![text_id]);
    }

    fn inner_html(&self, node_id: usize) -> String {
        let Some(node) = self.node(node_id) else {
            return String::new();
        };
        node.children
            .iter()
            .map(|child_id| self.serialize_node(*child_id))
            .collect()
    }

    fn raw_text(&self, node_id: usize) -> String {
        let Some(node) = self.node(node_id) else {
            return String::new();
        };
        node.children
            .iter()
            .map(|child_id| match self.node(*child_id).map(|child| &child.kind) {
                Some(DomNodeKind::Text(text)) => text.clone(),
                _ => self.serialize_node(*child_id),
            })
            .collect()
    }

    fn serialize_document(&self) -> String {
        self.node(self.document_id)
            .map(|node| {
                node.children
                    .iter()
                    .map(|child_id| self.serialize_node(*child_id))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn serialize_node(&self, node_id: usize) -> String {
        let Some(node) = self.node(node_id) else {
            return String::new();
        };
        match &node.kind {
            DomNodeKind::Text(text) => escape_html_text(text),
            DomNodeKind::Element(element) => {
                let mut html = String::new();
                html.push('<');
                html.push_str(&element.tag_name);
                for (name, value) in &element.attributes {
                    html.push(' ');
                    html.push_str(name);
                    if !value.is_empty() {
                        html.push_str("=\"");
                        html.push_str(&escape_html_attribute(value));
                        html.push('"');
                    }
                }
                if is_void_element(&element.tag_name) {
                    html.push('>');
                    return html;
                }
                html.push('>');
                if is_raw_text_element(&element.tag_name) {
                    html.push_str(&self.raw_text(node_id));
                } else {
                    for child_id in &node.children {
                        html.push_str(&self.serialize_node(*child_id));
                    }
                }
                html.push_str("</");
                html.push_str(&element.tag_name);
                html.push('>');
                html
            }
        }
    }

    fn get_attribute(&self, node_id: usize, name: &str) -> Option<String> {
        self.element(node_id)
            .and_then(|element| element.attributes.get(name))
            .cloned()
    }

    fn set_attribute(&mut self, node_id: usize, name: &str, value: &str) {
        if let Some(element) = self.element_mut(node_id) {
            element
                .attributes
                .insert(name.to_ascii_lowercase(), value.to_string());
        }
    }

    fn remove_attribute(&mut self, node_id: usize, name: &str) {
        if let Some(element) = self.element_mut(node_id) {
            element.attributes.remove(&name.to_ascii_lowercase());
        }
    }

    fn has_class(&self, node_id: usize, class_name: &str) -> bool {
        self.get_attribute(node_id, "class")
            .map(|classes| {
                classes
                    .split_ascii_whitespace()
                    .any(|existing| existing == class_name)
            })
            .unwrap_or(false)
    }

    fn add_class(&mut self, node_id: usize, class_name: &str) {
        let mut classes = self
            .get_attribute(node_id, "class")
            .unwrap_or_default()
            .split_ascii_whitespace()
            .map(str::to_string)
            .collect::<Vec<_>>();
        if !classes.iter().any(|existing| existing == class_name) {
            classes.push(class_name.to_string());
            self.set_attribute(node_id, "class", &classes.join(" "));
        }
    }

    fn remove_class(&mut self, node_id: usize, class_name: &str) {
        let current = self.get_attribute(node_id, "class").unwrap_or_default();
        let classes = current
            .split_ascii_whitespace()
            .filter(|existing| *existing != class_name)
            .collect::<Vec<_>>();
        if classes.is_empty() {
            self.remove_attribute(node_id, "class");
        } else {
            self.set_attribute(node_id, "class", &classes.join(" "));
        }
    }

    fn toggle_class(&mut self, node_id: usize, class_name: &str) -> bool {
        if self.has_class(node_id, class_name) {
            self.remove_class(node_id, class_name);
            false
        } else {
            self.add_class(node_id, class_name);
            true
        }
    }

    fn query_selector(&self, scope_id: usize, selector: &str, include_scope: bool) -> Option<usize> {
        let selector = ParsedSelector::parse(selector)?;
        self.descendant_nodes(scope_id, include_scope)
            .into_iter()
            .find(|node_id| self.matches_selector_in_scope(*node_id, scope_id, &selector))
    }

    fn query_selector_all(
        &self,
        scope_id: usize,
        selector: &str,
        include_scope: bool,
    ) -> Vec<usize> {
        let Some(selector) = ParsedSelector::parse(selector) else {
            return Vec::new();
        };
        self.descendant_nodes(scope_id, include_scope)
            .into_iter()
            .filter(|node_id| self.matches_selector_in_scope(*node_id, scope_id, &selector))
            .collect()
    }

    fn get_element_by_id(&self, scope_id: usize, target_id: &str) -> Option<usize> {
        self.descendant_nodes(scope_id, true).into_iter().find(|node_id| {
            self.get_attribute(*node_id, "id")
                .map(|value| value == target_id)
                .unwrap_or(false)
        })
    }

    fn matches_selector_in_scope(
        &self,
        node_id: usize,
        scope_id: usize,
        selector: &ParsedSelector,
    ) -> bool {
        self.match_selector_part(node_id, scope_id, selector.parts.len() - 1, selector)
    }

    fn match_selector_part(
        &self,
        node_id: usize,
        scope_id: usize,
        part_index: usize,
        selector: &ParsedSelector,
    ) -> bool {
        if !self.matches_simple_selector(node_id, &selector.parts[part_index].simple) {
            return false;
        }
        if part_index == 0 {
            return true;
        }

        match selector.parts[part_index].combinator_to_previous {
            Combinator::Child => {
                let Some(parent_id) = self.node(node_id).and_then(|node| node.parent) else {
                    return false;
                };
                if !self.is_within_scope(parent_id, scope_id) {
                    return false;
                }
                self.match_selector_part(parent_id, scope_id, part_index - 1, selector)
            }
            Combinator::Descendant => {
                let mut cursor = self.node(node_id).and_then(|node| node.parent);
                while let Some(ancestor_id) = cursor {
                    if !self.is_within_scope(ancestor_id, scope_id) {
                        break;
                    }
                    if self.match_selector_part(ancestor_id, scope_id, part_index - 1, selector) {
                        return true;
                    }
                    cursor = self.node(ancestor_id).and_then(|node| node.parent);
                }
                false
            }
        }
    }

    fn is_within_scope(&self, node_id: usize, scope_id: usize) -> bool {
        if node_id == scope_id {
            return true;
        }
        let mut cursor = self.node(node_id).and_then(|node| node.parent);
        while let Some(parent_id) = cursor {
            if parent_id == scope_id {
                return true;
            }
            cursor = self.node(parent_id).and_then(|node| node.parent);
        }
        false
    }

    fn matches_simple_selector(&self, node_id: usize, selector: &SimpleSelector) -> bool {
        let Some(element) = self.element(node_id) else {
            return false;
        };

        if let Some(tag_name) = selector.tag_name.as_deref()
            && element.tag_name != tag_name
        {
            return false;
        }
        if let Some(id) = selector.id.as_deref()
            && self.get_attribute(node_id, "id").as_deref() != Some(id)
        {
            return false;
        }
        for class_name in &selector.classes {
            if !self.has_class(node_id, class_name) {
                return false;
            }
        }
        for attribute in &selector.attributes {
            match &attribute.value {
                Some(value) => {
                    if self.get_attribute(node_id, &attribute.name).as_deref() != Some(value.as_str())
                    {
                        return false;
                    }
                }
                None => {
                    if self.get_attribute(node_id, &attribute.name).is_none() {
                        return false;
                    }
                }
            }
        }
        true
    }
}

#[derive(Debug, Clone)]
struct ParsedSelector {
    parts: Vec<SelectorPart>,
}

#[derive(Debug, Clone)]
struct SelectorPart {
    simple: SimpleSelector,
    combinator_to_previous: Combinator,
}

#[derive(Debug, Clone)]
struct SimpleSelector {
    tag_name: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
    attributes: Vec<AttributeSelector>,
}

#[derive(Debug, Clone)]
struct AttributeSelector {
    name: String,
    value: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Combinator {
    Descendant,
    Child,
}

impl ParsedSelector {
    fn parse(input: &str) -> Option<Self> {
        let selector = input.trim();
        if selector.is_empty() {
            return None;
        }
        let selector = selector.split(',').next()?.trim();
        let mut parts = Vec::new();
        let mut token = String::new();
        let mut combinator = Combinator::Descendant;
        let mut in_attribute = false;
        let mut quote = None;

        for character in selector.chars() {
            match character {
                '"' | '\'' if in_attribute => {
                    if quote == Some(character) {
                        quote = None;
                    } else if quote.is_none() {
                        quote = Some(character);
                    }
                    token.push(character);
                }
                '[' => {
                    in_attribute = true;
                    token.push(character);
                }
                ']' => {
                    in_attribute = false;
                    token.push(character);
                }
                '>' if !in_attribute && quote.is_none() => {
                    if !token.trim().is_empty() {
                        parts.push(SelectorPart {
                            simple: SimpleSelector::parse(token.trim())?,
                            combinator_to_previous: combinator,
                        });
                        token.clear();
                    }
                    combinator = Combinator::Child;
                }
                character if character.is_whitespace() && !in_attribute && quote.is_none() => {
                    if !token.trim().is_empty() {
                        parts.push(SelectorPart {
                            simple: SimpleSelector::parse(token.trim())?,
                            combinator_to_previous: combinator,
                        });
                        token.clear();
                    }
                    combinator = Combinator::Descendant;
                }
                _ => token.push(character),
            }
        }

        if !token.trim().is_empty() {
            parts.push(SelectorPart {
                simple: SimpleSelector::parse(token.trim())?,
                combinator_to_previous: combinator,
            });
        }

        (!parts.is_empty()).then_some(Self { parts })
    }
}

impl SimpleSelector {
    fn parse(input: &str) -> Option<Self> {
        let token = input.trim();
        if token.is_empty() {
            return None;
        }

        let mut selector = Self {
            tag_name: None,
            id: None,
            classes: Vec::new(),
            attributes: Vec::new(),
        };
        let bytes = token.as_bytes();
        let mut index = 0;

        if !matches!(bytes.first(), Some(b'#' | b'.' | b'[' | b':')) {
            let start = index;
            while index < bytes.len()
                && !matches!(bytes[index], b'#' | b'.' | b'[' | b':')
            {
                index += 1;
            }
            let tag_name = token[start..index].trim();
            if !tag_name.is_empty() && tag_name != "*" {
                selector.tag_name = Some(tag_name.to_ascii_lowercase());
            }
        }

        while index < bytes.len() {
            match bytes[index] {
                b'#' => {
                    index += 1;
                    let start = index;
                    while index < bytes.len()
                        && is_selector_name_char(bytes[index])
                    {
                        index += 1;
                    }
                    selector.id = Some(token[start..index].to_string());
                }
                b'.' => {
                    index += 1;
                    let start = index;
                    while index < bytes.len()
                        && is_selector_name_char(bytes[index])
                    {
                        index += 1;
                    }
                    selector.classes.push(token[start..index].to_string());
                }
                b'[' => {
                    index += 1;
                    let start = index;
                    while index < bytes.len() && bytes[index] != b']' {
                        index += 1;
                    }
                    let content = token[start..index].trim();
                    selector.attributes.push(parse_attribute_selector(content)?);
                    if index < bytes.len() {
                        index += 1;
                    }
                }
                b':' => break,
                _ => index += 1,
            }
        }

        Some(selector)
    }
}

fn parse_attribute_selector(input: &str) -> Option<AttributeSelector> {
    let (name, value) = input.split_once('=').map_or((input, None), |(name, value)| {
        let value = value
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .to_string();
        (name, Some(value))
    });
    let name = name.trim().to_ascii_lowercase();
    (!name.is_empty()).then_some(AttributeSelector { name, value })
}

fn is_selector_name_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':')
}

fn install_browser_globals(context: &mut Context) {
    let title_getter =
        NativeFunction::from_fn_ptr(js_document_get_title).to_js_function(context.realm());
    let title_setter =
        NativeFunction::from_fn_ptr(js_document_set_title).to_js_function(context.realm());
    let href_getter =
        NativeFunction::from_fn_ptr(js_location_get_href).to_js_function(context.realm());
    let href_setter =
        NativeFunction::from_fn_ptr(js_location_set_href).to_js_function(context.realm());
    let search_getter =
        NativeFunction::from_fn_ptr(js_location_get_search).to_js_function(context.realm());
    let hash_getter =
        NativeFunction::from_fn_ptr(js_location_get_hash).to_js_function(context.realm());
    let pathname_getter =
        NativeFunction::from_fn_ptr(js_location_get_pathname).to_js_function(context.realm());
    let origin_getter =
        NativeFunction::from_fn_ptr(js_location_get_origin).to_js_function(context.realm());
    let host_getter =
        NativeFunction::from_fn_ptr(js_location_get_host).to_js_function(context.realm());
    let hostname_getter =
        NativeFunction::from_fn_ptr(js_location_get_hostname).to_js_function(context.realm());
    let protocol_getter =
        NativeFunction::from_fn_ptr(js_location_get_protocol).to_js_function(context.realm());

    let location = ObjectInitializer::new(context)
        .accessor(
            js_string!("href"),
            Some(href_getter),
            Some(href_setter),
            Attribute::all(),
        )
        .accessor(
            js_string!("search"),
            Some(search_getter),
            None,
            Attribute::all(),
        )
        .accessor(
            js_string!("hash"),
            Some(hash_getter),
            None,
            Attribute::all(),
        )
        .accessor(
            js_string!("pathname"),
            Some(pathname_getter),
            None,
            Attribute::all(),
        )
        .accessor(
            js_string!("origin"),
            Some(origin_getter),
            None,
            Attribute::all(),
        )
        .accessor(
            js_string!("host"),
            Some(host_getter),
            None,
            Attribute::all(),
        )
        .accessor(
            js_string!("hostname"),
            Some(hostname_getter),
            None,
            Attribute::all(),
        )
        .accessor(
            js_string!("protocol"),
            Some(protocol_getter),
            None,
            Attribute::all(),
        )
        .function(
            NativeFunction::from_fn_ptr(js_location_assign),
            js_string!("assign"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_location_replace),
            js_string!("replace"),
            1,
        )
        .build();
    let document_id = context
        .get_data::<JavaScriptHostData>()
        .map(|host| host.state.borrow().dom.document_id)
        .unwrap_or(0);
    let body_id = context
        .get_data::<JavaScriptHostData>()
        .and_then(|host| host.state.borrow().dom.body_id)
        .unwrap_or(document_id);
    let head_id = context
        .get_data::<JavaScriptHostData>()
        .and_then(|host| host.state.borrow().dom.head_id)
        .unwrap_or(document_id);
    let html_id = context
        .get_data::<JavaScriptHostData>()
        .and_then(|host| host.state.borrow().dom.html_id)
        .unwrap_or(document_id);
    let body_object = build_dom_node_object(context, body_id);
    let head_object = build_dom_node_object(context, head_id);
    let html_object = build_dom_node_object(context, html_id);
    let node_list_stub = build_simple_node_list_stub(context);
    let document_fonts = ObjectInitializer::new(context)
        .function(NativeFunction::from_fn_ptr(js_noop), js_string!("load"), 2)
        .build();

    let global_object = context.global_object();
    let document = ObjectInitializer::with_native_data(DomNodeHandle { node_id: document_id }, context)
        .function(
            NativeFunction::from_fn_ptr(js_document_write),
            js_string!("write"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_document_writeln),
            js_string!("writeln"),
            1,
        )
        .accessor(
            js_string!("title"),
            Some(title_getter),
            Some(title_setter),
            Attribute::all(),
        )
        .function(
            NativeFunction::from_fn_ptr(js_dom_query_selector),
            js_string!("querySelector"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_dom_query_selector_all),
            js_string!("querySelectorAll"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_document_get_element_by_id),
            js_string!("getElementById"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_document_create_element),
            js_string!("createElement"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_document_create_text_node),
            js_string!("createTextNode"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_add_event_listener),
            js_string!("addEventListener"),
            2,
        )
        .function(
            NativeFunction::from_fn_ptr(js_noop),
            js_string!("removeEventListener"),
            2,
        )
        .property(js_string!("location"), location.clone(), Attribute::all())
        .property(js_string!("body"), body_object, Attribute::all())
        .property(js_string!("head"), head_object, Attribute::all())
        .property(js_string!("documentElement"), html_object, Attribute::all())
        .property(js_string!("fonts"), document_fonts, Attribute::all())
        .property(js_string!("cookie"), js_string!(""), Attribute::all())
        .property(js_string!("readyState"), js_string!("complete"), Attribute::all())
        .property(js_string!("compatMode"), js_string!("CSS1Compat"), Attribute::all())
        .property(js_string!("hidden"), false, Attribute::all())
        .property(js_string!("visibilityState"), js_string!("visible"), Attribute::all())
        .property(js_string!("defaultView"), global_object.clone(), Attribute::all())
        .build();

    let console = ObjectInitializer::new(context)
        .function(
            NativeFunction::from_fn_ptr(js_console_log),
            js_string!("log"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_console_log),
            js_string!("info"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_console_log),
            js_string!("warn"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_console_log),
            js_string!("error"),
            1,
        )
        .build();

    context
        .register_global_property(js_string!("document"), document, Attribute::all())
        .expect("document should be installable");
    context
        .register_global_property(js_string!("location"), location, Attribute::all())
        .expect("location should be installable");
    context
        .register_global_property(js_string!("console"), console, Attribute::all())
        .expect("console should be installable");
    let navigator = ObjectInitializer::new(context)
        .property(
            js_string!("userAgent"),
            js_string!(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36"
            ),
            Attribute::all(),
        )
        .property(js_string!("language"), js_string!("ja-JP"), Attribute::all())
        .property(js_string!("languages"), node_list_stub.clone(), Attribute::all())
        .property(js_string!("platform"), js_string!("Win32"), Attribute::all())
        .property(js_string!("vendor"), js_string!("Google Inc."), Attribute::all())
        .build();
    let performance_timing = ObjectInitializer::new(context)
        .property(js_string!("navigationStart"), 0, Attribute::all())
        .build();
    let performance = ObjectInitializer::new(context)
        .function(
            NativeFunction::from_fn_ptr(js_performance_now),
            js_string!("now"),
            0,
        )
        .property(js_string!("timing"), performance_timing, Attribute::all())
        .build();
    let history = ObjectInitializer::new(context)
        .function(
            NativeFunction::from_fn_ptr(js_noop),
            js_string!("pushState"),
            3,
        )
        .function(
            NativeFunction::from_fn_ptr(js_noop),
            js_string!("replaceState"),
            3,
        )
        .function(NativeFunction::from_fn_ptr(js_noop), js_string!("back"), 0)
        .function(
            NativeFunction::from_fn_ptr(js_noop),
            js_string!("forward"),
            0,
        )
        .build();
    let storage = build_storage_stub(context);
    let ytcfg = ObjectInitializer::new(context)
        .function(
            NativeFunction::from_fn_ptr(js_ytcfg_data),
            js_string!("d"),
            0,
        )
        .function(
            NativeFunction::from_fn_ptr(js_ytcfg_get),
            js_string!("get"),
            2,
        )
        .function(NativeFunction::from_fn_ptr(js_noop), js_string!("set"), 2)
        .build();
    let ytcsi = ObjectInitializer::new(context)
        .function(
            NativeFunction::from_fn_ptr(js_document_create_stub_object),
            js_string!("gt"),
            1,
        )
        .function(NativeFunction::from_fn_ptr(js_noop), js_string!("tick"), 3)
        .function(NativeFunction::from_fn_ptr(js_noop), js_string!("info"), 3)
        .function(
            NativeFunction::from_fn_ptr(js_noop),
            js_string!("infoGel"),
            2,
        )
        .function(
            NativeFunction::from_fn_ptr(js_noop),
            js_string!("setStart"),
            2,
        )
        .build();
    context
        .register_global_property(js_string!("navigator"), navigator, Attribute::all())
        .expect("navigator should be installable");
    context
        .register_global_property(js_string!("performance"), performance, Attribute::all())
        .expect("performance should be installable");
    context
        .register_global_property(js_string!("history"), history, Attribute::all())
        .expect("history should be installable");
    context
        .register_global_property(
            js_string!("localStorage"),
            storage.clone(),
            Attribute::all(),
        )
        .expect("localStorage should be installable");
    context
        .register_global_property(js_string!("sessionStorage"), storage, Attribute::all())
        .expect("sessionStorage should be installable");
    context
        .register_global_property(js_string!("ytcfg"), ytcfg, Attribute::all())
        .expect("ytcfg should be installable");
    context
        .register_global_property(js_string!("ytcsi"), ytcsi, Attribute::all())
        .expect("ytcsi should be installable");
    context
        .register_global_property(
            js_string!("window"),
            global_object.clone(),
            Attribute::all(),
        )
        .expect("window should be installable");
    context
        .register_global_property(js_string!("self"), global_object, Attribute::all())
        .expect("self should be installable");
    context
        .register_global_builtin_callable(
            js_string!("setTimeout"),
            2,
            NativeFunction::from_fn_ptr(js_set_timeout),
        )
        .expect("setTimeout should be installable");
    context
        .register_global_builtin_callable(
            js_string!("setInterval"),
            2,
            NativeFunction::from_fn_ptr(js_set_timeout),
        )
        .expect("setInterval should be installable");
    context
        .register_global_builtin_callable(
            js_string!("clearTimeout"),
            1,
            NativeFunction::from_fn_ptr(js_clear_timeout),
        )
        .expect("clearTimeout should be installable");
    context
        .register_global_builtin_callable(
            js_string!("clearInterval"),
            1,
            NativeFunction::from_fn_ptr(js_clear_timeout),
        )
        .expect("clearInterval should be installable");
    context
        .register_global_builtin_callable(
            js_string!("requestAnimationFrame"),
            1,
            NativeFunction::from_fn_ptr(js_request_animation_frame),
        )
        .expect("requestAnimationFrame should be installable");
    context
        .register_global_builtin_callable(
            js_string!("cancelAnimationFrame"),
            1,
            NativeFunction::from_fn_ptr(js_clear_timeout),
        )
        .expect("cancelAnimationFrame should be installable");
    context
        .register_global_builtin_callable(
            js_string!("queueMicrotask"),
            1,
            NativeFunction::from_fn_ptr(js_queue_microtask),
        )
        .expect("queueMicrotask should be installable");
    context
        .register_global_builtin_callable(
            js_string!("matchMedia"),
            1,
            NativeFunction::from_fn_ptr(js_match_media),
        )
        .expect("matchMedia should be installable");
    context
        .register_global_builtin_callable(
            js_string!("addEventListener"),
            2,
            NativeFunction::from_fn_ptr(js_add_event_listener),
        )
        .expect("addEventListener should be installable");
    context
        .register_global_builtin_callable(
            js_string!("removeEventListener"),
            2,
            NativeFunction::from_fn_ptr(js_noop),
        )
        .expect("removeEventListener should be installable");
    context
        .register_global_builtin_callable(
            js_string!("alert"),
            1,
            NativeFunction::from_fn_ptr(js_alert),
        )
        .expect("alert should be installable");
    context
        .register_global_builtin_callable(
            js_string!("confirm"),
            1,
            NativeFunction::from_fn_ptr(js_confirm),
        )
        .expect("confirm should be installable");
    context
        .register_global_builtin_callable(
            js_string!("prompt"),
            2,
            NativeFunction::from_fn_ptr(js_prompt),
        )
        .expect("prompt should be installable");
    context
        .register_global_builtin_callable(
            js_string!("fetch"),
            2,
            NativeFunction::from_fn_ptr(js_fetch),
        )
        .expect("fetch should be installable");
    context
        .register_global_builtin_callable(
            js_string!("__tobiraCreateXMLHttpRequest"),
            0,
            NativeFunction::from_fn_ptr(js_create_xml_http_request),
        )
        .expect("XMLHttpRequest factory should be installable");
    context
        .register_global_property(js_string!("innerWidth"), 1280, Attribute::all())
        .expect("innerWidth should be installable");
    context
        // Note: changed from 720 in a previous version to align with the CSS vh unit
        // (1vh = 8px at 800px base in css.rs parse_length). Scripts that rely on
        // window.innerHeight == 720 may behave differently.
        .register_global_property(js_string!("innerHeight"), 800, Attribute::all()) // must match vh base (800px) in css.rs parse_length
        .expect("innerHeight should be installable");

    let crypto_subtle = ObjectInitializer::new(context)
        .function(
            NativeFunction::from_fn_ptr(js_noop),
            js_string!("digest"),
            2,
        )
        .function(
            NativeFunction::from_fn_ptr(js_noop),
            js_string!("encrypt"),
            3,
        )
        .function(
            NativeFunction::from_fn_ptr(js_noop),
            js_string!("decrypt"),
            3,
        )
        .build();
    let crypto = ObjectInitializer::new(context)
        .function(
            NativeFunction::from_fn_ptr(js_crypto_get_random_values),
            js_string!("getRandomValues"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_crypto_random_uuid),
            js_string!("randomUUID"),
            0,
        )
        .property(js_string!("subtle"), crypto_subtle, Attribute::all())
        .build();
    context
        .register_global_property(js_string!("crypto"), crypto, Attribute::all())
        .expect("crypto should be installable");

    let url_search_params = ObjectInitializer::new(context)
        .function(
            NativeFunction::from_fn_ptr(js_return_null),
            js_string!("get"),
            1,
        )
        .function(NativeFunction::from_fn_ptr(js_noop), js_string!("set"), 2)
        .function(
            NativeFunction::from_fn_ptr(js_noop),
            js_string!("append"),
            2,
        )
        .function(
            NativeFunction::from_fn_ptr(js_noop),
            js_string!("delete"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_return_false),
            js_string!("has"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_noop),
            js_string!("forEach"),
            1,
        )
        .property(js_string!("toString"), js_string!(""), Attribute::all())
        .build();
    context
        .register_global_property(
            js_string!("URLSearchParams"),
            url_search_params,
            Attribute::all(),
        )
        .expect("URLSearchParams should be installable");

    let _ = context.eval(Source::from_bytes(
        "globalThis.XMLHttpRequest = function XMLHttpRequest(){ return __tobiraCreateXMLHttpRequest(); };",
    ));
}

fn build_simple_node_list_stub(context: &mut Context) -> boa_engine::object::JsObject {
    ObjectInitializer::new(context)
        .function(
            NativeFunction::from_fn_ptr(js_noop),
            js_string!("forEach"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_return_undefined),
            js_string!("item"),
            1,
        )
        .property(js_string!("length"), 0, Attribute::all())
        .build()
}

fn build_dom_node_object(context: &mut Context, node_id: usize) -> boa_engine::object::JsObject {
    let get_class_list =
        NativeFunction::from_fn_ptr(js_dom_get_class_list).to_js_function(context.realm());
    let get_children =
        NativeFunction::from_fn_ptr(js_dom_get_children).to_js_function(context.realm());
    let get_child_nodes =
        NativeFunction::from_fn_ptr(js_dom_get_child_nodes).to_js_function(context.realm());
    let get_text_content =
        NativeFunction::from_fn_ptr(js_dom_get_text_content).to_js_function(context.realm());
    let set_text_content =
        NativeFunction::from_fn_ptr(js_dom_set_text_content).to_js_function(context.realm());
    let get_inner_html =
        NativeFunction::from_fn_ptr(js_dom_get_inner_html).to_js_function(context.realm());
    let set_inner_html =
        NativeFunction::from_fn_ptr(js_dom_set_inner_html).to_js_function(context.realm());
    let get_id = NativeFunction::from_fn_ptr(js_dom_get_id).to_js_function(context.realm());
    let set_id = NativeFunction::from_fn_ptr(js_dom_set_id).to_js_function(context.realm());
    let get_class_name =
        NativeFunction::from_fn_ptr(js_dom_get_class_name).to_js_function(context.realm());
    let set_class_name =
        NativeFunction::from_fn_ptr(js_dom_set_class_name).to_js_function(context.realm());
    let get_value = NativeFunction::from_fn_ptr(js_dom_get_value).to_js_function(context.realm());
    let set_value = NativeFunction::from_fn_ptr(js_dom_set_value).to_js_function(context.realm());
    let get_src = NativeFunction::from_fn_ptr(js_dom_get_src).to_js_function(context.realm());
    let set_src = NativeFunction::from_fn_ptr(js_dom_set_src).to_js_function(context.realm());
    let get_href = NativeFunction::from_fn_ptr(js_dom_get_href).to_js_function(context.realm());
    let set_href = NativeFunction::from_fn_ptr(js_dom_set_href).to_js_function(context.realm());
    let get_rel = NativeFunction::from_fn_ptr(js_dom_get_rel).to_js_function(context.realm());
    let set_rel = NativeFunction::from_fn_ptr(js_dom_set_rel).to_js_function(context.realm());
    let get_type = NativeFunction::from_fn_ptr(js_dom_get_type).to_js_function(context.realm());
    let set_type = NativeFunction::from_fn_ptr(js_dom_set_type).to_js_function(context.realm());
    let get_name = NativeFunction::from_fn_ptr(js_dom_get_name).to_js_function(context.realm());
    let set_name = NativeFunction::from_fn_ptr(js_dom_set_name).to_js_function(context.realm());
    let get_content =
        NativeFunction::from_fn_ptr(js_dom_get_content).to_js_function(context.realm());
    let set_content =
        NativeFunction::from_fn_ptr(js_dom_set_content).to_js_function(context.realm());
    let get_parent_element =
        NativeFunction::from_fn_ptr(js_dom_get_parent_element).to_js_function(context.realm());
    let get_owner_document =
        NativeFunction::from_fn_ptr(js_dom_get_owner_document).to_js_function(context.realm());
    let get_tag_name =
        NativeFunction::from_fn_ptr(js_dom_get_tag_name).to_js_function(context.realm());
    let get_parent_node =
        NativeFunction::from_fn_ptr(js_dom_get_parent_node).to_js_function(context.realm());
    let style = ObjectInitializer::new(context).build();
    ObjectInitializer::with_native_data(DomNodeHandle { node_id }, context)
        .function(
            NativeFunction::from_fn_ptr(js_dom_query_selector),
            js_string!("querySelector"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_dom_query_selector_all),
            js_string!("querySelectorAll"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_dom_append_child),
            js_string!("appendChild"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_dom_insert_before),
            js_string!("insertBefore"),
            2,
        )
        .function(
            NativeFunction::from_fn_ptr(js_dom_get_attribute),
            js_string!("getAttribute"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_dom_set_attribute),
            js_string!("setAttribute"),
            2,
        )
        .function(
            NativeFunction::from_fn_ptr(js_dom_remove_attribute),
            js_string!("removeAttribute"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_add_event_listener),
            js_string!("addEventListener"),
            2,
        )
        .function(
            NativeFunction::from_fn_ptr(js_noop),
            js_string!("removeEventListener"),
            2,
        )
        .function(NativeFunction::from_fn_ptr(js_noop), js_string!("focus"), 0)
        .function(NativeFunction::from_fn_ptr(js_noop), js_string!("blur"), 0)
        .function(
            NativeFunction::from_fn_ptr(js_dom_remove),
            js_string!("remove"),
            0,
        )
        .function(
            NativeFunction::from_fn_ptr(js_return_dom_rect_stub),
            js_string!("getBoundingClientRect"),
            0,
        )
        .function(
            NativeFunction::from_fn_ptr(js_get_video_aspect_ratio),
            js_string!("getVideoAspectRatio"),
            0,
        )
        .accessor(
            js_string!("classList"),
            Some(get_class_list),
            None,
            Attribute::all(),
        )
        .accessor(
            js_string!("children"),
            Some(get_children),
            None,
            Attribute::all(),
        )
        .accessor(
            js_string!("childNodes"),
            Some(get_child_nodes),
            None,
            Attribute::all(),
        )
        .accessor(
            js_string!("textContent"),
            Some(get_text_content),
            Some(set_text_content),
            Attribute::all(),
        )
        .accessor(
            js_string!("innerHTML"),
            Some(get_inner_html),
            Some(set_inner_html),
            Attribute::all(),
        )
        .accessor(
            js_string!("id"),
            Some(get_id),
            Some(set_id),
            Attribute::all(),
        )
        .accessor(
            js_string!("className"),
            Some(get_class_name),
            Some(set_class_name),
            Attribute::all(),
        )
        .accessor(
            js_string!("value"),
            Some(get_value),
            Some(set_value),
            Attribute::all(),
        )
        .accessor(
            js_string!("src"),
            Some(get_src),
            Some(set_src),
            Attribute::all(),
        )
        .accessor(
            js_string!("href"),
            Some(get_href),
            Some(set_href),
            Attribute::all(),
        )
        .accessor(
            js_string!("rel"),
            Some(get_rel),
            Some(set_rel),
            Attribute::all(),
        )
        .accessor(
            js_string!("type"),
            Some(get_type),
            Some(set_type),
            Attribute::all(),
        )
        .accessor(
            js_string!("name"),
            Some(get_name),
            Some(set_name),
            Attribute::all(),
        )
        .accessor(
            js_string!("content"),
            Some(get_content),
            Some(set_content),
            Attribute::all(),
        )
        .accessor(
            js_string!("tagName"),
            Some(get_tag_name.clone()),
            None,
            Attribute::all(),
        )
        .accessor(
            js_string!("nodeName"),
            Some(get_tag_name),
            None,
            Attribute::all(),
        )
        .accessor(
            js_string!("parentNode"),
            Some(get_parent_node),
            None,
            Attribute::all(),
        )
        .accessor(
            js_string!("parentElement"),
            Some(get_parent_element),
            None,
            Attribute::all(),
        )
        .accessor(
            js_string!("ownerDocument"),
            Some(get_owner_document),
            None,
            Attribute::all(),
        )
        .property(js_string!("style"), style, Attribute::all())
        .property(js_string!("checked"), false, Attribute::all())
        .property(js_string!("hidden"), false, Attribute::all())
        .property(js_string!("clientWidth"), 1280, Attribute::all())
        .property(js_string!("clientHeight"), 720, Attribute::all())
        .property(js_string!("scrollWidth"), 1280, Attribute::all())
        .property(js_string!("scrollHeight"), 720, Attribute::all())
        .build()
}

fn build_dom_node_list_object(
    context: &mut Context,
    node_ids: Vec<usize>,
) -> boa_engine::object::JsObject {
    let object = ObjectInitializer::with_native_data(DomNodeListHandle { node_ids: node_ids.clone() }, context)
        .function(
            NativeFunction::from_fn_ptr(js_dom_node_list_for_each),
            js_string!("forEach"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_dom_node_list_item),
            js_string!("item"),
            1,
        )
        .property(js_string!("length"), node_ids.len() as i32, Attribute::all())
        .build();

    for (index, node_id) in node_ids.into_iter().enumerate() {
        let _ = object.create_data_property_or_throw(
            js_string!(index.to_string()),
            JsValue::from(build_dom_node_object(context, node_id)),
            context,
        );
    }

    object
}

fn build_dom_class_list_object(
    context: &mut Context,
    node_id: usize,
) -> boa_engine::object::JsObject {
    ObjectInitializer::with_native_data(DomClassListHandle { node_id }, context)
        .function(
            NativeFunction::from_fn_ptr(js_dom_class_list_add),
            js_string!("add"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_dom_class_list_remove),
            js_string!("remove"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_dom_class_list_contains),
            js_string!("contains"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_dom_class_list_toggle),
            js_string!("toggle"),
            1,
        )
        .build()
}

fn build_storage_stub(context: &mut Context) -> boa_engine::object::JsObject {
    ObjectInitializer::new(context)
        .function(
            NativeFunction::from_fn_ptr(js_return_null),
            js_string!("getItem"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_noop),
            js_string!("setItem"),
            2,
        )
        .function(
            NativeFunction::from_fn_ptr(js_noop),
            js_string!("removeItem"),
            1,
        )
        .function(NativeFunction::from_fn_ptr(js_noop), js_string!("clear"), 0)
        .property(js_string!("length"), 0, Attribute::all())
        .build()
}

fn build_fetch_response_object(
    context: &mut Context,
    response: crate::http::HttpResponse,
) -> boa_engine::object::JsObject {
    let ok = (200..=299).contains(&response.status_code);
    let status = response.status_code as i32;
    let status_text = response.reason_phrase.clone();
    let url = response.final_url.to_string();
    let headers = build_response_headers_object(context, &response.headers);

    ObjectInitializer::with_native_data(FetchResponseHandle { response }, context)
        .function(
            NativeFunction::from_fn_ptr(js_fetch_response_text),
            js_string!("text"),
            0,
        )
        .function(
            NativeFunction::from_fn_ptr(js_fetch_response_json),
            js_string!("json"),
            0,
        )
        .function(
            NativeFunction::from_fn_ptr(js_fetch_response_clone),
            js_string!("clone"),
            0,
        )
        .property(js_string!("ok"), ok, Attribute::all())
        .property(js_string!("status"), status, Attribute::all())
        .property(js_string!("statusText"), js_string!(status_text), Attribute::all())
        .property(js_string!("url"), js_string!(url), Attribute::all())
        .property(js_string!("headers"), headers, Attribute::all())
        .build()
}

fn build_response_headers_object(
    context: &mut Context,
    headers: &std::collections::HashMap<String, String>,
) -> boa_engine::object::JsObject {
    let headers = headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect::<BTreeMap<_, _>>();
    ObjectInitializer::with_native_data(ResponseHeadersHandle { headers }, context)
        .function(
            NativeFunction::from_fn_ptr(js_response_headers_get),
            js_string!("get"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_response_headers_has),
            js_string!("has"),
            1,
        )
        .build()
}

fn build_xml_http_request_object(context: &mut Context) -> boa_engine::object::JsObject {
    let get_ready_state =
        NativeFunction::from_fn_ptr(js_xhr_get_ready_state).to_js_function(context.realm());
    let get_status = NativeFunction::from_fn_ptr(js_xhr_get_status).to_js_function(context.realm());
    let get_status_text =
        NativeFunction::from_fn_ptr(js_xhr_get_status_text).to_js_function(context.realm());
    let get_response_text =
        NativeFunction::from_fn_ptr(js_xhr_get_response_text).to_js_function(context.realm());
    let get_response =
        NativeFunction::from_fn_ptr(js_xhr_get_response).to_js_function(context.realm());
    let get_response_url =
        NativeFunction::from_fn_ptr(js_xhr_get_response_url).to_js_function(context.realm());

    ObjectInitializer::with_native_data(
        XmlHttpRequestHandle {
            state: RefCell::new(XmlHttpRequestState::default()),
        },
        context,
    )
    .function(NativeFunction::from_fn_ptr(js_xhr_open), js_string!("open"), 3)
    .function(
        NativeFunction::from_fn_ptr(js_xhr_set_request_header),
        js_string!("setRequestHeader"),
        2,
    )
    .function(NativeFunction::from_fn_ptr(js_xhr_send), js_string!("send"), 1)
    .function(NativeFunction::from_fn_ptr(js_xhr_abort), js_string!("abort"), 0)
    .function(
        NativeFunction::from_fn_ptr(js_xhr_get_response_header),
        js_string!("getResponseHeader"),
        1,
    )
    .accessor(
        js_string!("readyState"),
        Some(get_ready_state),
        None,
        Attribute::all(),
    )
    .accessor(js_string!("status"), Some(get_status), None, Attribute::all())
    .accessor(
        js_string!("statusText"),
        Some(get_status_text),
        None,
        Attribute::all(),
    )
    .accessor(
        js_string!("responseText"),
        Some(get_response_text.clone()),
        None,
        Attribute::all(),
    )
    .accessor(
        js_string!("response"),
        Some(get_response),
        None,
        Attribute::all(),
    )
    .accessor(
        js_string!("responseURL"),
        Some(get_response_url),
        None,
        Attribute::all(),
    )
    .property(js_string!("responseType"), js_string!(""), Attribute::all())
    .property(js_string!("withCredentials"), false, Attribute::all())
    .property(js_string!("onreadystatechange"), JsValue::undefined(), Attribute::all())
    .property(js_string!("onload"), JsValue::undefined(), Attribute::all())
    .property(js_string!("onerror"), JsValue::undefined(), Attribute::all())
    .build()
}

fn build_match_media_stub(context: &mut Context, media: String) -> boa_engine::object::JsObject {
    ObjectInitializer::new(context)
        .function(
            NativeFunction::from_fn_ptr(js_add_event_listener),
            js_string!("addEventListener"),
            2,
        )
        .function(
            NativeFunction::from_fn_ptr(js_noop),
            js_string!("removeEventListener"),
            2,
        )
        .function(
            NativeFunction::from_fn_ptr(js_noop),
            js_string!("addListener"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_noop),
            js_string!("removeListener"),
            1,
        )
        .property(js_string!("matches"), false, Attribute::all())
        .property(js_string!("media"), js_string!(media), Attribute::all())
        .build()
}

fn build_dom_rect_stub(context: &mut Context) -> boa_engine::object::JsObject {
    ObjectInitializer::new(context)
        .property(js_string!("x"), 0, Attribute::all())
        .property(js_string!("y"), 0, Attribute::all())
        .property(js_string!("top"), 0, Attribute::all())
        .property(js_string!("left"), 0, Attribute::all())
        .property(js_string!("right"), 1280, Attribute::all())
        .property(js_string!("bottom"), 720, Attribute::all())
        .property(js_string!("width"), 1280, Attribute::all())
        .property(js_string!("height"), 720, Attribute::all())
        .build()
}

fn load_script_source(
    inline_script: &str,
    attributes: &BTreeMap<String, String>,
    base_url: &Url,
) -> Option<String> {
    if let Some(src) = attributes.get("src") {
        let url = base_url.resolve(src).ok()?;
        let response = fetch(&url).ok()?;
        return Some(decode_text_response(
            &response.body,
            response.header("content-type"),
        ));
    }

    Some(inline_script.to_string())
}

fn should_execute_script(attributes: &BTreeMap<String, String>) -> bool {
    let language = attributes
        .get("language")
        .map(|value| value.trim().to_ascii_lowercase());
    if let Some(language) = language {
        let language = language.trim_start_matches("text/");
        if !matches!(language, "" | "javascript" | "jscript" | "ecmascript") {
            return false;
        }
    }

    let script_type = attributes
        .get("type")
        .map(|value| value.trim().to_ascii_lowercase());
    match script_type.as_deref() {
        None
        | Some("")
        | Some("text/javascript")
        | Some("application/javascript")
        | Some("application/ecmascript")
        | Some("text/ecmascript") => true,
        Some("module") | Some("text/module") => false,
        Some(other) if other.ends_with("javascript") || other.ends_with("ecmascript") => true,
        Some(_) => false,
    }
}

fn is_supported_script_source(source: &str, _host: &str) -> bool {
    source.len() <= MAX_SCRIPT_SOURCE_BYTES
}

fn escape_html_text(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_html_attribute(input: &str) -> String {
    escape_html_text(input).replace('"', "&quot;")
}

fn is_void_element(name: &str) -> bool {
    matches!(
        name,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "frame"
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

fn is_raw_text_element(name: &str) -> bool {
    matches!(name, "script" | "style" | "title" | "textarea")
}

fn js_document_write(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    append_written_html(args, "", context)?;
    Ok(JsValue::undefined())
}

fn js_document_writeln(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    append_written_html(args, "\n", context)?;
    Ok(JsValue::undefined())
}

fn append_written_html(args: &[JsValue], suffix: &str, context: &mut Context) -> JsResult<()> {
    let mut written = String::new();
    for value in args {
        written.push_str(&js_value_to_string(value, context)?);
    }
    written.push_str(suffix);

    if let Some(host) = context.get_data::<JavaScriptHostData>() {
        host.state.borrow_mut().write_buffer.push_str(&written);
    }

    Ok(())
}

fn js_document_get_title(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let title = context
        .get_data::<JavaScriptHostData>()
        .map(|host| {
            let state = host.state.borrow();
            state
                .dom
                .title_text()
                .unwrap_or_else(|| state.current_title.clone())
        })
        .unwrap_or_default();
    Ok(JsValue::from(js_string!(title)))
}

fn js_document_set_title(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let title = js_value_to_string(args.first().unwrap_or(&JsValue::undefined()), context)?;
    if let Some(host) = context.get_data::<JavaScriptHostData>() {
        let mut state = host.state.borrow_mut();
        state.current_title = title;
        state.title_dirty = true;
        let current_title = state.current_title.clone();
        state.dom.set_title_text(&current_title);
    }
    Ok(JsValue::undefined())
}

fn js_location_get_href(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let href = context
        .get_data::<JavaScriptHostData>()
        .map(|host| host.state.borrow().location_href.clone())
        .unwrap_or_default();
    Ok(JsValue::from(js_string!(href)))
}

fn js_location_set_href(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let href = js_value_to_string(args.first().unwrap_or(&JsValue::undefined()), context)?;
    set_location_href(&href, context);
    Ok(JsValue::undefined())
}

fn js_location_get_search(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let search = current_location_url(context)
        .map(|url| {
            url.path
                .split_once('?')
                .map(|(_, query)| format!("?{query}"))
                .unwrap_or_default()
        })
        .unwrap_or_default();
    Ok(JsValue::from(js_string!(search)))
}

fn js_location_get_hash(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::from(js_string!("")))
}

fn js_location_get_pathname(
    _: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let pathname = current_location_url(context)
        .map(|url| {
            url.path
                .split(['?', '#'])
                .next()
                .unwrap_or(url.path.as_str())
                .to_string()
        })
        .unwrap_or_else(|| "/".to_string());
    Ok(JsValue::from(js_string!(pathname)))
}

fn js_location_get_origin(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let origin = current_location_url(context)
        .map(|url| format!("{}://{}", url.scheme, url.host_header()))
        .unwrap_or_default();
    Ok(JsValue::from(js_string!(origin)))
}

fn js_location_get_host(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let host = current_location_url(context)
        .map(|url| url.host_header())
        .unwrap_or_default();
    Ok(JsValue::from(js_string!(host)))
}

fn js_location_get_hostname(
    _: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let hostname = current_location_url(context)
        .map(|url| url.host)
        .unwrap_or_default();
    Ok(JsValue::from(js_string!(hostname)))
}

fn js_location_get_protocol(
    _: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let protocol = current_location_url(context)
        .map(|url| format!("{}:", url.scheme))
        .unwrap_or_default();
    Ok(JsValue::from(js_string!(protocol)))
}

fn js_location_assign(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let href = js_value_to_string(args.first().unwrap_or(&JsValue::undefined()), context)?;
    set_location_href(&href, context);
    Ok(JsValue::undefined())
}

fn js_location_replace(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let href = js_value_to_string(args.first().unwrap_or(&JsValue::undefined()), context)?;
    set_location_href(&href, context);
    Ok(JsValue::undefined())
}

fn set_location_href(href: &str, context: &mut Context) {
    let resolved = current_location_url(context)
        .and_then(|url| url.resolve(href).ok())
        .map(|url| url.to_string())
        .or_else(|| Url::parse(href).ok().map(|url| url.to_string()))
        .unwrap_or_else(|| href.to_string());
    if let Some(host) = context.get_data::<JavaScriptHostData>() {
        host.state.borrow_mut().location_href = resolved;
    }
}

fn current_location_url(context: &mut Context) -> Option<Url> {
    let href = context
        .get_data::<JavaScriptHostData>()
        .map(|host| host.state.borrow().location_href.clone())?;
    Url::parse(&href).ok()
}

fn resolve_requested_url(request: &JsValue, context: &mut Context) -> JsResult<Url> {
    let request_url = if let Some(object) = request.as_object() {
        match object.get(js_string!("url"), context) {
            Ok(value) if !value.is_undefined() && !value.is_null() => js_value_to_string(&value, context)?,
            _ => js_value_to_string(request, context)?,
        }
    } else {
        js_value_to_string(request, context)?
    };

    if let Ok(url) = Url::parse(&request_url) {
        return Ok(url);
    }

    if let Some(base) = current_location_url(context)
        && let Ok(url) = base.resolve(&request_url)
    {
        return Ok(url);
    }

    Err(JsNativeError::error()
        .with_message(format!("unsupported fetch url: {request_url}"))
        .into())
}

fn js_fetch(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let request = args.first().cloned().unwrap_or_else(JsValue::undefined);
    let url = resolve_requested_url(&request, context);
    let promise = match url {
        Ok(url) => JsPromise::from_result(
            fetch(&url)
                .map(|response| JsValue::from(build_fetch_response_object(context, response)))
                .map_err(|error| JsNativeError::error().with_message(error.to_string())),
            context,
        ),
        Err(error) => JsPromise::reject(error, context),
    };
    Ok(JsValue::from(promise))
}

fn js_fetch_response_text(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let promise = if let Some(object) = this.as_object()
        && let Some(handle) = object.downcast_ref::<FetchResponseHandle>()
    {
        let text = decode_text_response(
            &handle.response.body,
            handle.response.header("content-type"),
        );
        JsPromise::resolve(js_string!(text), context)
    } else {
        JsPromise::reject(
            JsNativeError::typ().with_message("Response.text called on non-response object"),
            context,
        )
    };
    Ok(JsValue::from(promise))
}

fn js_fetch_response_json(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let promise = if let Some(object) = this.as_object()
        && let Some(handle) = object.downcast_ref::<FetchResponseHandle>()
    {
        let text = decode_text_response(
            &handle.response.body,
            handle.response.header("content-type"),
        );
        let value = serde_json::from_str::<serde_json::Value>(&text)
            .map_err(|error| JsNativeError::syntax().with_message(error.to_string()))
            .and_then(|json| JsValue::from_json(&json, context).map_err(|error| {
                JsNativeError::error().with_message(error.to_string())
            }));
        JsPromise::from_result(value, context)
    } else {
        JsPromise::reject(
            JsNativeError::typ().with_message("Response.json called on non-response object"),
            context,
        )
    };
    Ok(JsValue::from(promise))
}

fn js_fetch_response_clone(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    if let Some(object) = this.as_object()
        && let Some(handle) = object.downcast_ref::<FetchResponseHandle>()
    {
        return Ok(JsValue::from(build_fetch_response_object(
            context,
            handle.response.clone(),
        )));
    }

    Ok(JsValue::undefined())
}

fn js_response_headers_get(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let name = js_value_to_string(args.first().unwrap_or(&JsValue::undefined()), context)?
        .to_ascii_lowercase();
    let value = if let Some(object) = this.as_object() {
        object
            .downcast_ref::<ResponseHeadersHandle>()
            .and_then(|handle| handle.headers.get(&name).cloned())
    } else {
        None
    };
    Ok(value
        .map(|value| JsValue::from(js_string!(value)))
        .unwrap_or_else(JsValue::null))
}

fn js_response_headers_has(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let name = js_value_to_string(args.first().unwrap_or(&JsValue::undefined()), context)?
        .to_ascii_lowercase();
    let has = if let Some(object) = this.as_object() {
        object
            .downcast_ref::<ResponseHeadersHandle>()
            .map(|handle| handle.headers.contains_key(&name))
            .unwrap_or(false)
    } else {
        false
    };
    Ok(JsValue::new(has))
}

fn js_create_xml_http_request(
    _: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    Ok(JsValue::from(build_xml_http_request_object(context)))
}

fn js_xhr_open(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let method = js_value_to_string(args.first().unwrap_or(&JsValue::undefined()), context)?
        .to_ascii_uppercase();
    let url = js_value_to_string(args.get(1).unwrap_or(&JsValue::undefined()), context)?;
    if let Some(object) = this.as_object()
        && let Some(handle) = object.downcast_ref::<XmlHttpRequestHandle>()
    {
        let mut state = handle.state.borrow_mut();
        state.method = method;
        state.url = Some(url);
        state.ready_state = 1;
        state.status = 0;
        state.status_text.clear();
        state.response_text.clear();
        state.response_url.clear();
        state.request_headers.clear();
    }
    Ok(JsValue::undefined())
}

fn js_xhr_set_request_header(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let name = js_value_to_string(args.first().unwrap_or(&JsValue::undefined()), context)?
        .to_ascii_lowercase();
    let value = js_value_to_string(args.get(1).unwrap_or(&JsValue::undefined()), context)?;
    if let Some(object) = this.as_object()
        && let Some(handle) = object.downcast_ref::<XmlHttpRequestHandle>()
    {
        handle.state.borrow_mut().request_headers.insert(name, value);
    }
    Ok(JsValue::undefined())
}

fn js_xhr_send(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(object) = this.as_object() else {
        return Ok(JsValue::undefined());
    };
    let Some(handle) = object.downcast_ref::<XmlHttpRequestHandle>() else {
        return Ok(JsValue::undefined());
    };

    let (method, target_url) = {
        let state = handle.state.borrow();
        (
            state.method.clone(),
            state.url.clone().unwrap_or_else(|| "/".to_string()),
        )
    };

    let result = if method.is_empty() || method == "GET" {
        resolve_requested_url(&JsValue::from(js_string!(target_url)), context)
            .and_then(|url| fetch(&url).map_err(|error| {
                JsNativeError::error().with_message(error.to_string()).into()
            }))
    } else {
        Err(JsNativeError::error()
            .with_message(format!("unsupported XMLHttpRequest method: {method}"))
            .into())
    };

    match result {
        Ok(response) => {
            let text = decode_text_response(&response.body, response.header("content-type"));
            {
                let mut state = handle.state.borrow_mut();
                state.ready_state = 4;
                state.status = response.status_code;
                state.status_text = response.reason_phrase.clone();
                state.response_text = text;
                state.response_url = response.final_url.to_string();
            }
            trigger_xhr_handler(&object, "onreadystatechange", context)?;
            trigger_xhr_handler(&object, "onload", context)?;
        }
        Err(error) => {
            {
                let mut state = handle.state.borrow_mut();
                state.ready_state = 4;
                state.status = 0;
                state.status_text = error.to_string();
                state.response_text.clear();
                state.response_url.clear();
            }
            trigger_xhr_handler(&object, "onreadystatechange", context)?;
            trigger_xhr_handler(&object, "onerror", context)?;
        }
    }

    Ok(JsValue::undefined())
}

fn js_xhr_abort(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    if let Some(object) = this.as_object()
        && let Some(handle) = object.downcast_ref::<XmlHttpRequestHandle>()
    {
        let mut state = handle.state.borrow_mut();
        state.ready_state = 0;
        state.status = 0;
        state.status_text.clear();
        state.response_text.clear();
        state.response_url.clear();
    }
    Ok(JsValue::undefined())
}

fn js_xhr_get_response_header(
    _: &JsValue,
    _: &[JsValue],
    _: &mut Context,
) -> JsResult<JsValue> {
    Ok(JsValue::null())
}

fn js_xhr_get_ready_state(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::new(
        xhr_state_value(this, |state| state.ready_state).unwrap_or(0),
    ))
}

fn js_xhr_get_status(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::new(
        xhr_state_value(this, |state| state.status).unwrap_or(0),
    ))
}

fn js_xhr_get_status_text(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::from(js_string!(
        xhr_state_value(this, |state| state.status_text.clone())
            .unwrap_or_default()
    )))
}

fn js_xhr_get_response_text(
    this: &JsValue,
    _: &[JsValue],
    _: &mut Context,
) -> JsResult<JsValue> {
    Ok(JsValue::from(js_string!(
        xhr_state_value(this, |state| state.response_text.clone())
            .unwrap_or_default()
    )))
}

fn js_xhr_get_response(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::from(js_string!(
        xhr_state_value(this, |state| state.response_text.clone())
            .unwrap_or_default()
    )))
}

fn js_xhr_get_response_url(
    this: &JsValue,
    _: &[JsValue],
    _: &mut Context,
) -> JsResult<JsValue> {
    Ok(JsValue::from(js_string!(
        xhr_state_value(this, |state| state.response_url.clone())
            .unwrap_or_default()
    )))
}

fn xhr_state_value<T>(this: &JsValue, map: impl FnOnce(&XmlHttpRequestState) -> T) -> Option<T> {
    let object = this.as_object()?;
    let handle = object.downcast_ref::<XmlHttpRequestHandle>()?;
    Some(map(&handle.state.borrow()))
}

fn trigger_xhr_handler(
    object: &boa_engine::object::JsObject,
    property: &str,
    context: &mut Context,
) -> JsResult<()> {
    let callback = object.get(js_string!(property), context)?;
    call_js_callback(&callback, &[], context)?;
    Ok(())
}

fn js_console_log(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let mut parts = Vec::new();
    for value in args {
        parts.push(js_value_to_string(value, context)?);
    }

    if let Some(host) = context.get_data::<JavaScriptHostData>() {
        host.state.borrow_mut().console_logs.push(parts.join(" "));
    }

    Ok(JsValue::undefined())
}

fn js_request_animation_frame(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    if let Some(callback) = args.first() {
        call_js_callback(callback, &[JsValue::new(performance_now_ms())], context)?;
    }
    Ok(JsValue::new(1))
}

fn js_queue_microtask(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    if let Some(callback) = args.first() {
        call_js_callback(callback, &[], context)?;
    }
    Ok(JsValue::undefined())
}

fn js_set_timeout(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    if let Some(callback) = args.first() {
        if callback.as_object().is_some() {
            call_js_callback(callback, &[], context)?;
        } else if callback.is_string() {
            let script = js_value_to_string(callback, context)?;
            let _ = context.eval(Source::from_bytes(script.as_str()));
        }
    }

    Ok(JsValue::new(0))
}

fn js_clear_timeout(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::undefined())
}

fn js_add_event_listener(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let event_name = args
        .first()
        .map(|value| js_value_to_string(value, context))
        .transpose()?
        .unwrap_or_default()
        .to_ascii_lowercase();
    if matches!(
        event_name.as_str(),
        "load" | "domcontentloaded" | "readystatechange" | "script-load-dpj"
    ) && let Some(callback) = args.get(1)
    {
        call_js_callback(callback, &[], context)?;
    }

    Ok(JsValue::undefined())
}

fn js_alert(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    js_console_log(&JsValue::undefined(), args, context)?;
    Ok(JsValue::undefined())
}

fn js_confirm(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    js_console_log(&JsValue::undefined(), args, context)?;
    Ok(JsValue::new(false))
}

fn js_prompt(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    js_console_log(&JsValue::undefined(), args, context)?;
    Ok(JsValue::null())
}

fn js_match_media(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let media = js_value_to_string(args.first().unwrap_or(&JsValue::undefined()), context)?;
    Ok(JsValue::from(build_match_media_stub(context, media)))
}

fn this_node_id(this: &JsValue) -> Option<usize> {
    this.as_object()?
        .downcast_ref::<DomNodeHandle>()
        .map(|handle| handle.node_id)
}

fn node_id_argument(arg: Option<&JsValue>) -> Option<usize> {
    arg.and_then(JsValue::as_object)
        .and_then(|object| object.downcast_ref::<DomNodeHandle>().map(|handle| handle.node_id))
}

fn js_dom_query_selector(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let selector = js_value_to_string(args.first().unwrap_or(&JsValue::undefined()), context)?;
    let Some(scope_id) = this_node_id(this) else {
        return Ok(JsValue::null());
    };
    let found = context
        .get_data::<JavaScriptHostData>()
        .and_then(|host| {
            let state = host.state.borrow();
            let include_scope = scope_id == state.dom.document_id;
            state.dom.query_selector(scope_id, &selector, include_scope)
        });
    Ok(found
        .map(|node_id| JsValue::from(build_dom_node_object(context, node_id)))
        .unwrap_or_else(JsValue::null))
}

fn js_dom_query_selector_all(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let selector = js_value_to_string(args.first().unwrap_or(&JsValue::undefined()), context)?;
    let Some(scope_id) = this_node_id(this) else {
        return Ok(JsValue::from(build_dom_node_list_object(context, Vec::new())));
    };
    let node_ids = context
        .get_data::<JavaScriptHostData>()
        .map(|host| {
            let state = host.state.borrow();
            let include_scope = scope_id == state.dom.document_id;
            state.dom.query_selector_all(scope_id, &selector, include_scope)
        })
        .unwrap_or_default();
    Ok(JsValue::from(build_dom_node_list_object(context, node_ids)))
}

fn js_document_get_element_by_id(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let target_id = js_value_to_string(args.first().unwrap_or(&JsValue::undefined()), context)?;
    let Some(scope_id) = this_node_id(this) else {
        return Ok(JsValue::null());
    };
    let found = context
        .get_data::<JavaScriptHostData>()
        .and_then(|host| host.state.borrow().dom.get_element_by_id(scope_id, &target_id));
    Ok(found
        .map(|node_id| JsValue::from(build_dom_node_object(context, node_id)))
        .unwrap_or_else(JsValue::null))
}

fn js_document_create_element(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let tag_name = js_value_to_string(args.first().unwrap_or(&JsValue::undefined()), context)?;
    let node_id = context
        .get_data::<JavaScriptHostData>()
        .map(|host| host.state.borrow_mut().dom.create_element(&tag_name))
        .unwrap_or(0);
    Ok(JsValue::from(build_dom_node_object(context, node_id)))
}

fn js_document_create_text_node(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let text = js_value_to_string(args.first().unwrap_or(&JsValue::undefined()), context)?;
    let node_id = context
        .get_data::<JavaScriptHostData>()
        .map(|host| host.state.borrow_mut().dom.create_text_node(&text))
        .unwrap_or(0);
    Ok(JsValue::from(build_dom_node_object(context, node_id)))
}

fn js_document_create_stub_object(
    _: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = context
        .get_data::<JavaScriptHostData>()
        .map(|host| host.state.borrow().dom.document_id)
        .unwrap_or(0);
    Ok(JsValue::from(build_dom_node_object(context, node_id)))
}

fn js_dom_append_child(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(parent_id) = this_node_id(this) else {
        return Ok(JsValue::undefined());
    };
    let Some(child_id) = node_id_argument(args.first()) else {
        return Ok(JsValue::undefined());
    };
    if let Some(host) = context.get_data::<JavaScriptHostData>() {
        host.state.borrow_mut().dom.append_child(parent_id, child_id);
    }
    Ok(args.first().cloned().unwrap_or_else(JsValue::undefined))
}

fn js_dom_insert_before(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(parent_id) = this_node_id(this) else {
        return Ok(JsValue::undefined());
    };
    let Some(child_id) = node_id_argument(args.first()) else {
        return Ok(JsValue::undefined());
    };
    let before_id = node_id_argument(args.get(1));
    if let Some(host) = context.get_data::<JavaScriptHostData>() {
        host.state
            .borrow_mut()
            .dom
            .insert_before(parent_id, child_id, before_id);
    }
    Ok(args.first().cloned().unwrap_or_else(JsValue::undefined))
}

fn js_dom_get_attribute(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let name = js_value_to_string(args.first().unwrap_or(&JsValue::undefined()), context)?;
    let Some(node_id) = this_node_id(this) else {
        return Ok(JsValue::null());
    };
    let value = context
        .get_data::<JavaScriptHostData>()
        .and_then(|host| host.state.borrow().dom.get_attribute(node_id, &name));
    Ok(value
        .map(|value| JsValue::from(js_string!(value)))
        .unwrap_or_else(JsValue::null))
}

fn js_dom_get_property_attribute(
    this: &JsValue,
    name: &str,
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(node_id) = this_node_id(this) else {
        return Ok(JsValue::from(js_string!("")));
    };
    let value = context
        .get_data::<JavaScriptHostData>()
        .and_then(|host| host.state.borrow().dom.get_attribute(node_id, name))
        .unwrap_or_default();
    Ok(JsValue::from(js_string!(value)))
}

fn js_dom_set_attribute(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let name = js_value_to_string(args.first().unwrap_or(&JsValue::undefined()), context)?;
    let value = js_value_to_string(args.get(1).unwrap_or(&JsValue::undefined()), context)?;
    if let Some(node_id) = this_node_id(this)
        && let Some(host) = context.get_data::<JavaScriptHostData>()
    {
        host.state.borrow_mut().dom.set_attribute(node_id, &name, &value);
    }
    Ok(JsValue::undefined())
}

fn js_dom_set_property_attribute(
    this: &JsValue,
    args: &[JsValue],
    name: &str,
    context: &mut Context,
) -> JsResult<JsValue> {
    let value = js_value_to_string(args.first().unwrap_or(&JsValue::undefined()), context)?;
    if let Some(node_id) = this_node_id(this)
        && let Some(host) = context.get_data::<JavaScriptHostData>()
    {
        host.state.borrow_mut().dom.set_attribute(node_id, name, &value);
    }
    Ok(JsValue::undefined())
}

fn js_dom_remove_attribute(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let name = js_value_to_string(args.first().unwrap_or(&JsValue::undefined()), context)?;
    if let Some(node_id) = this_node_id(this)
        && let Some(host) = context.get_data::<JavaScriptHostData>()
    {
        host.state.borrow_mut().dom.remove_attribute(node_id, &name);
    }
    Ok(JsValue::undefined())
}

fn js_dom_remove(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    if let Some(node_id) = this_node_id(this)
        && let Some(host) = context.get_data::<JavaScriptHostData>()
    {
        host.state.borrow_mut().dom.detach_node(node_id);
    }
    Ok(JsValue::undefined())
}

fn js_dom_get_class_list(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node_id = this_node_id(this).unwrap_or(0);
    Ok(JsValue::from(build_dom_class_list_object(context, node_id)))
}

fn js_dom_get_children(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node_id = this_node_id(this).unwrap_or(0);
    let node_ids = context
        .get_data::<JavaScriptHostData>()
        .map(|host| {
            let state = host.state.borrow();
            state
                .dom
                .node(node_id)
                .map(|node| {
                    node.children
                        .iter()
                        .copied()
                        .filter(|child_id| state.dom.element(*child_id).is_some())
                        .collect()
                })
                .unwrap_or_default()
        })
        .unwrap_or_default();
    Ok(JsValue::from(build_dom_node_list_object(context, node_ids)))
}

fn js_dom_get_child_nodes(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node_id = this_node_id(this).unwrap_or(0);
    let node_ids = context
        .get_data::<JavaScriptHostData>()
        .and_then(|host| host.state.borrow().dom.node(node_id).map(|node| node.children.clone()))
        .unwrap_or_default();
    Ok(JsValue::from(build_dom_node_list_object(context, node_ids)))
}

fn js_dom_get_text_content(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node_id = this_node_id(this).unwrap_or(0);
    let text = context
        .get_data::<JavaScriptHostData>()
        .map(|host| host.state.borrow().dom.text_content(node_id))
        .unwrap_or_default();
    Ok(JsValue::from(js_string!(text)))
}

fn js_dom_set_text_content(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this).unwrap_or(0);
    let text = js_value_to_string(args.first().unwrap_or(&JsValue::undefined()), context)?;
    if let Some(host) = context.get_data::<JavaScriptHostData>() {
        host.state.borrow_mut().dom.set_text_content(node_id, &text);
    }
    Ok(JsValue::undefined())
}

fn js_dom_get_inner_html(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node_id = this_node_id(this).unwrap_or(0);
    let html = context
        .get_data::<JavaScriptHostData>()
        .map(|host| host.state.borrow().dom.inner_html(node_id))
        .unwrap_or_default();
    Ok(JsValue::from(js_string!(html)))
}

fn js_dom_set_inner_html(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this).unwrap_or(0);
    let html = js_value_to_string(args.first().unwrap_or(&JsValue::undefined()), context)?;
    if let Some(host) = context.get_data::<JavaScriptHostData>() {
        host.state
            .borrow_mut()
            .dom
            .replace_children_with_fragment(node_id, &html);
    }
    Ok(JsValue::undefined())
}

fn js_dom_get_id(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    js_dom_get_attribute(
        this,
        &[JsValue::from(js_string!("id"))],
        context,
    )
}

fn js_dom_set_id(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let value = args.first().cloned().unwrap_or_else(JsValue::undefined);
    js_dom_set_attribute(
        this,
        &[JsValue::from(js_string!("id")), value],
        context,
    )
}

fn js_dom_get_class_name(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    js_dom_get_attribute(
        this,
        &[JsValue::from(js_string!("class"))],
        context,
    )
}

fn js_dom_set_class_name(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let value = args.first().cloned().unwrap_or_else(JsValue::undefined);
    js_dom_set_attribute(
        this,
        &[JsValue::from(js_string!("class")), value],
        context,
    )
}

fn js_dom_get_value(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    js_dom_get_property_attribute(this, "value", context)
}

fn js_dom_set_value(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    js_dom_set_property_attribute(this, args, "value", context)
}

fn js_dom_get_src(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    js_dom_get_property_attribute(this, "src", context)
}

fn js_dom_set_src(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    js_dom_set_property_attribute(this, args, "src", context)
}

fn js_dom_get_href(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    js_dom_get_property_attribute(this, "href", context)
}

fn js_dom_set_href(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    js_dom_set_property_attribute(this, args, "href", context)
}

fn js_dom_get_rel(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    js_dom_get_property_attribute(this, "rel", context)
}

fn js_dom_set_rel(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    js_dom_set_property_attribute(this, args, "rel", context)
}

fn js_dom_get_type(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    js_dom_get_property_attribute(this, "type", context)
}

fn js_dom_set_type(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    js_dom_set_property_attribute(this, args, "type", context)
}

fn js_dom_get_name(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    js_dom_get_property_attribute(this, "name", context)
}

fn js_dom_set_name(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    js_dom_set_property_attribute(this, args, "name", context)
}

fn js_dom_get_content(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    js_dom_get_property_attribute(this, "content", context)
}

fn js_dom_set_content(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    js_dom_set_property_attribute(this, args, "content", context)
}

fn js_dom_get_tag_name(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node_id = this_node_id(this).unwrap_or(0);
    let tag_name = context
        .get_data::<JavaScriptHostData>()
        .and_then(|host| host.state.borrow().dom.element(node_id).map(|element| element.tag_name.to_ascii_uppercase()))
        .unwrap_or_default();
    Ok(JsValue::from(js_string!(tag_name)))
}

fn js_dom_get_parent_node(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node_id = this_node_id(this).unwrap_or(0);
    let parent_id = context
        .get_data::<JavaScriptHostData>()
        .and_then(|host| host.state.borrow().dom.node(node_id).and_then(|node| node.parent));
    Ok(parent_id
        .map(|parent_id| JsValue::from(build_dom_node_object(context, parent_id)))
        .unwrap_or_else(JsValue::null))
}

fn js_dom_get_parent_element(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this).unwrap_or(0);
    let parent_id = context
        .get_data::<JavaScriptHostData>()
        .and_then(|host| {
            let state = host.state.borrow();
            state
                .dom
                .node(node_id)
                .and_then(|node| node.parent)
                .filter(|parent_id| state.dom.element(*parent_id).is_some())
        });
    Ok(parent_id
        .map(|parent_id| JsValue::from(build_dom_node_object(context, parent_id)))
        .unwrap_or_else(JsValue::null))
}

fn js_dom_get_owner_document(
    _: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let document_id = context
        .get_data::<JavaScriptHostData>()
        .map(|host| host.state.borrow().dom.document_id)
        .unwrap_or(0);
    Ok(JsValue::from(build_dom_node_object(context, document_id)))
}

fn js_dom_class_list_add(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let class_name = js_value_to_string(args.first().unwrap_or(&JsValue::undefined()), context)?;
    if let Some(object) = this.as_object()
        && let Some(handle) = object.downcast_ref::<DomClassListHandle>()
        && let Some(host) = context.get_data::<JavaScriptHostData>()
    {
        host.state.borrow_mut().dom.add_class(handle.node_id, &class_name);
    }
    Ok(JsValue::undefined())
}

fn js_dom_class_list_remove(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let class_name = js_value_to_string(args.first().unwrap_or(&JsValue::undefined()), context)?;
    if let Some(object) = this.as_object()
        && let Some(handle) = object.downcast_ref::<DomClassListHandle>()
        && let Some(host) = context.get_data::<JavaScriptHostData>()
    {
        host.state.borrow_mut().dom.remove_class(handle.node_id, &class_name);
    }
    Ok(JsValue::undefined())
}

fn js_dom_class_list_contains(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let class_name = js_value_to_string(args.first().unwrap_or(&JsValue::undefined()), context)?;
    let contains = this
        .as_object()
        .and_then(|object| object.downcast_ref::<DomClassListHandle>().map(|handle| handle.node_id))
        .and_then(|node_id| {
            context
                .get_data::<JavaScriptHostData>()
                .map(|host| host.state.borrow().dom.has_class(node_id, &class_name))
        })
        .unwrap_or(false);
    Ok(JsValue::new(contains))
}

fn js_dom_class_list_toggle(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let class_name = js_value_to_string(args.first().unwrap_or(&JsValue::undefined()), context)?;
    let toggled = if let Some(object) = this.as_object()
        && let Some(handle) = object.downcast_ref::<DomClassListHandle>()
        && let Some(host) = context.get_data::<JavaScriptHostData>()
    {
        host.state
            .borrow_mut()
            .dom
            .toggle_class(handle.node_id, &class_name)
    } else {
        false
    };
    Ok(JsValue::new(toggled))
}

fn js_dom_node_list_for_each(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(callback) = args.first() else {
        return Ok(JsValue::undefined());
    };
    let Some(object) = this.as_object() else {
        return Ok(JsValue::undefined());
    };
    let Some(handle) = object.downcast_ref::<DomNodeListHandle>() else {
        return Ok(JsValue::undefined());
    };
    for (index, node_id) in handle.node_ids.iter().copied().enumerate() {
        let node_value = JsValue::from(build_dom_node_object(context, node_id));
        let index_value = JsValue::new(index as i32);
        let list_value = JsValue::from(build_dom_node_list_object(context, handle.node_ids.clone()));
        let args = [node_value, index_value, list_value];
        call_js_callback(callback, &args, context)?;
    }
    Ok(JsValue::undefined())
}

fn js_dom_node_list_item(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let index = args
        .first()
        .and_then(JsValue::as_number)
        .map(|value| value as usize)
        .unwrap_or(0);
    let Some(object) = this.as_object() else {
        return Ok(JsValue::undefined());
    };
    let Some(handle) = object.downcast_ref::<DomNodeListHandle>() else {
        return Ok(JsValue::undefined());
    };
    Ok(handle
        .node_ids
        .get(index)
        .copied()
        .map(|node_id| JsValue::from(build_dom_node_object(context, node_id)))
        .unwrap_or_else(JsValue::undefined))
}

fn js_performance_now(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::new(performance_now_ms()))
}

fn js_return_dom_rect_stub(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::from(build_dom_rect_stub(context)))
}

fn js_get_video_aspect_ratio(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::new(16.0 / 9.0))
}

fn js_return_false(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::new(false))
}

fn js_return_null(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::null())
}

fn js_return_undefined(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::undefined())
}

fn js_noop(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::undefined())
}

fn js_crypto_get_random_values(
    _: &JsValue,
    args: &[JsValue],
    _: &mut Context,
) -> JsResult<JsValue> {
    Ok(args.first().cloned().unwrap_or_else(JsValue::undefined))
}

fn js_crypto_random_uuid(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::from(js_string!(
        "00000000-0000-4000-8000-000000000000"
    )))
}

fn js_ytcfg_data(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node_id = context
        .get_data::<JavaScriptHostData>()
        .map(|host| host.state.borrow().dom.document_id)
        .unwrap_or(0);
    Ok(JsValue::from(build_dom_node_object(context, node_id)))
}

fn js_ytcfg_get(_: &JsValue, args: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(args.get(1).cloned().unwrap_or_else(JsValue::undefined))
}

fn call_js_callback(
    callback: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    if let Some(object) = callback.as_object()
        && let Some(function) = JsFunction::from_object(object.clone())
    {
        return function.call(&JsValue::undefined(), args, context);
    }

    Ok(JsValue::undefined())
}

fn performance_now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

fn js_value_to_string(value: &JsValue, context: &mut Context) -> JsResult<String> {
    Ok(value.to_string(context)?.to_std_string_escaped())
}

#[cfg(test)]
mod tests {
    use super::process_document_scripts;
    use crate::url::Url;

    #[test]
    fn expands_document_write_output() {
        let processed = process_document_scripts(
            "<html><body><script>document.write('<p>Hello</p>')</script></body></html>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert!(processed.html.contains("<p>Hello</p>"));
        assert!(
            !processed.html.contains("document.write"),
            "{}",
            processed.html
        );
    }

    #[test]
    fn updates_document_title_from_script() {
        let processed = process_document_scripts(
            "<html><head><title>Demo</title><script>document.title = 'Changed'</script></head><body></body></html>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert_eq!(processed.title_override.as_deref(), Some("Changed"));
        assert!(processed.html.contains("<title>Changed</title>"));
    }

    #[test]
    fn executes_script_written_scripts_recursively() {
        let processed = process_document_scripts(
            "<script>document.write('<script>document.write(\"<p>Nested</p>\")</' + 'script>')</script>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert!(
            processed.html.contains("<p>Nested</p>"),
            "html: {}\nlogs: {:?}",
            processed.html,
            processed.console_logs
        );
    }

    #[test]
    fn skips_non_javascript_script_types() {
        let processed = process_document_scripts(
            "<script type=\"application/ld+json\">{\"name\":\"demo\"}</script>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert!(processed.html.contains("application/ld+json"));
    }

    #[test]
    fn runs_set_timeout_callbacks_immediately() {
        let processed = process_document_scripts(
            "<script>setTimeout(function () { document.write('<p>Later</p>'); }, 1);</script>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert!(processed.html.contains("<p>Later</p>"));
    }

    #[test]
    fn skips_large_scripts_even_if_they_reference_supported_apis() {
        let large_script = format!(
            "<script>{}document.write('<p>Nope</p>')</script>",
            "x".repeat(super::MAX_SCRIPT_SOURCE_BYTES)
        );
        let processed =
            process_document_scripts(&large_script, &Url::parse("https://example.com").unwrap());

        assert!(!processed.html.contains("<p>Nope</p>"));
        assert!(
            processed
                .console_logs
                .iter()
                .any(|entry| entry.contains("script policy rejected source"))
        );
    }

    #[test]
    fn reports_navigation_target_when_location_changes() {
        let processed = process_document_scripts(
            "<script>location.href = '/next?from=test';</script>",
            &Url::parse("https://example.com/start").unwrap(),
        );

        assert_eq!(
            processed.navigation_target.as_deref(),
            Some("https://example.com/next?from=test")
        );
    }

    #[test]
    fn supports_lightweight_dom_like_scripts() {
        let processed = process_document_scripts(
            "<div id=\"demo\"></div><script>document.addEventListener('DOMContentLoaded', function () { var el = document.querySelector('#demo'); if (el) { document.write('<p>Ready</p>'); } });</script>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert!(processed.html.contains("<p>Ready</p>"));
    }

    #[test]
    fn supports_dom_append_child_and_text_content() {
        let processed = process_document_scripts(
            "<html><body><div id=\"app\"></div><script>var app = document.getElementById('app'); var card = document.createElement('section'); card.textContent = 'Hello DOM'; app.appendChild(card);</script></body></html>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert!(processed.html.contains("<section>Hello DOM</section>"));
    }

    #[test]
    fn supports_inner_html_and_class_list_mutations() {
        let processed = process_document_scripts(
            "<html><body><div id=\"app\" class=\"shell\"></div><script>var app = document.querySelector('#app'); app.classList.add('ready'); app.innerHTML = '<p>Rendered</p>';</script></body></html>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert!(processed.html.contains("class=\"shell ready\""));
        assert!(processed.html.contains("<p>Rendered</p>"));
    }

    #[test]
    fn supports_create_text_node_and_property_reflection() {
        let processed = process_document_scripts(
            "<html><body><div id=\"app\"></div><script>var app = document.getElementById('app'); var span = document.createElement('span'); span.className = 'chip'; var text = document.createTextNode('Hello'); span.appendChild(text); var img = document.createElement('img'); img.src = '/avatar.png'; app.appendChild(span); app.appendChild(img);</script></body></html>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert!(processed.html.contains("<span class=\"chip\">Hello</span>"));
        assert!(processed.html.contains("<img src=\"/avatar.png\">"));
    }
}
