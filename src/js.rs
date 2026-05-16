use std::cell::RefCell;
use std::collections::BTreeMap;
use std::mem;
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use boa_engine::object::{
    JsObject, ObjectInitializer,
    builtins::{JsFunction, JsPromise},
};
use boa_engine::property::{Attribute, PropertyDescriptor};
use boa_engine::{
    Context, Finalize, JsData, JsNativeError, JsResult, JsValue, NativeFunction, Source, Trace,
    js_string,
};

use crate::html::{Node, parse_document};
use crate::http::{HttpResponse, fetch, fetch_with_limits_same_origin};
use crate::site_state::{self, StorageKind};
use crate::text::decode_text_response;
use crate::url::Url;

const MAX_SCRIPT_SOURCE_BYTES: usize = 2 * 1024 * 1024;
const MAX_TOTAL_SCRIPT_BYTES: usize = 16 * 1024 * 1024;
const MAX_SCRIPT_ITERATIONS: usize = 1024;
const JS_THREAD_STACK_BYTES: usize = 32 * 1024 * 1024;
const JS_LOOP_ITERATION_LIMIT: u64 = 100_000;
const JS_MAX_NETWORK_REQUESTS: usize = 8;
const JS_MAX_NETWORK_RESPONSE_BYTES: usize = 256 * 1024;
const JS_MAX_NETWORK_TOTAL_RESPONSE_BYTES: usize = 512 * 1024;
const DEFAULT_VIEWPORT_WIDTH: u32 = 1280;
const DEFAULT_VIEWPORT_HEIGHT: u32 = 720;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProcessedScriptHtml {
    pub html: String,
    pub title_override: Option<String>,
    pub console_logs: Vec<String>,
    pub navigation_target: Option<String>,
    pub soft_navigation_target: Option<String>,
    pub scroll_y: u32,
}

#[derive(Debug, Clone, Default)]
pub struct DomEventRequest {
    pub target_node_id: usize,
    pub event_type: String,
    pub bubbles: bool,
    pub cancelable: bool,
    pub key: Option<String>,
    pub code: Option<String>,
    pub repeat: bool,
    pub alt_key: bool,
    pub ctrl_key: bool,
    pub shift_key: bool,
    pub meta_key: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DomEventDispatchResult {
    pub snapshot: ProcessedScriptHtml,
    pub default_prevented: bool,
}

#[derive(Debug, Clone)]
pub struct JavaScriptSession {
    command_tx: Sender<JavaScriptSessionCommand>,
}

#[derive(Debug)]
enum JavaScriptSessionCommand {
    DispatchEvent {
        request: DomEventRequest,
        response_tx: Sender<DomEventDispatchResult>,
    },
    DispatchGlobalEvent {
        event_type: String,
        bubbles: bool,
        cancelable: bool,
        response_tx: Sender<DomEventDispatchResult>,
    },
    SetScrollPosition {
        y: u32,
    },
    SetViewportSize {
        width: u32,
        height: u32,
    },
    SetAttribute {
        node_id: usize,
        name: String,
        value: String,
    },
    Snapshot {
        response_tx: Sender<ProcessedScriptHtml>,
    },
    Shutdown,
}

impl JavaScriptSession {
    pub(crate) fn dispatch_event(
        &self,
        request: DomEventRequest,
    ) -> Option<DomEventDispatchResult> {
        let (response_tx, response_rx) = mpsc::channel();
        if self
            .command_tx
            .send(JavaScriptSessionCommand::DispatchEvent {
                request,
                response_tx,
            })
            .is_err()
        {
            return None;
        }

        response_rx.recv().ok()
    }

    pub(crate) fn snapshot(&self) -> Option<ProcessedScriptHtml> {
        let (response_tx, response_rx) = mpsc::channel();
        if self
            .command_tx
            .send(JavaScriptSessionCommand::Snapshot { response_tx })
            .is_err()
        {
            return None;
        }

        response_rx.recv().ok()
    }

    pub(crate) fn set_attribute(&self, node_id: usize, name: &str, value: &str) -> bool {
        self.command_tx
            .send(JavaScriptSessionCommand::SetAttribute {
                node_id,
                name: name.to_string(),
                value: value.to_string(),
            })
            .is_ok()
    }

    pub(crate) fn set_viewport_size(&self, width: u32, height: u32) -> bool {
        self.command_tx
            .send(JavaScriptSessionCommand::SetViewportSize { width, height })
            .is_ok()
    }

    pub(crate) fn set_scroll_position(&self, y: u32) -> bool {
        self.command_tx
            .send(JavaScriptSessionCommand::SetScrollPosition { y })
            .is_ok()
    }

    pub(crate) fn dispatch_global_event(
        &self,
        event_type: &str,
        bubbles: bool,
        cancelable: bool,
    ) -> Option<DomEventDispatchResult> {
        let (response_tx, response_rx) = mpsc::channel();
        if self
            .command_tx
            .send(JavaScriptSessionCommand::DispatchGlobalEvent {
                event_type: event_type.to_string(),
                bubbles,
                cancelable,
                response_tx,
            })
            .is_err()
        {
            return None;
        }

        response_rx.recv().ok()
    }
}

impl Drop for JavaScriptSession {
    fn drop(&mut self) {
        let _ = self.command_tx.send(JavaScriptSessionCommand::Shutdown);
    }
}

#[derive(Debug)]
struct JavaScriptState {
    current_title: String,
    title_dirty: bool,
    write_buffer: String,
    console_logs: Vec<String>,
    document_url: Url,
    location_href: String,
    soft_navigation_target: Option<String>,
    history_entries: Vec<String>,
    history_index: usize,
    current_script: Option<usize>,
    network_request_count: usize,
    network_response_bytes: usize,
    viewport_width: u32,
    viewport_height: u32,
    scroll_y: u32,
    active_element_node_id: Option<usize>,
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
struct DomStyleHandle {
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

#[derive(Debug, Clone, Trace, Finalize, JsData)]
struct StorageHandle {
    #[unsafe_ignore_trace]
    kind: StorageKind,
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

#[derive(Debug, Clone, Default)]
struct EventListenerOptions {
    capture: bool,
    once: bool,
    passive: bool,
}

#[derive(Debug, Clone)]
struct EventListenerEntry {
    callback: JsValue,
    options: EventListenerOptions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EventDispatchPhase {
    Capturing = 1,
    AtTarget = 2,
    Bubbling = 3,
}

pub fn process_document_scripts(html: &str, base_url: &Url) -> ProcessedScriptHtml {
    let (processed, session) = start_document_script_session(html, base_url);
    drop(session);
    processed
}

pub fn start_document_script_session(
    html: &str,
    base_url: &Url,
) -> (ProcessedScriptHtml, Option<JavaScriptSession>) {
    let html_owned = html.to_string();
    let base_url_owned = base_url.clone();
    let (ready_tx, ready_rx) = mpsc::channel();
    let (command_tx, command_rx) = mpsc::channel();
    let worker = thread::Builder::new()
        .name("tobira-js".to_string())
        .stack_size(JS_THREAD_STACK_BYTES)
        .spawn({
            let html = html_owned.clone();
            let base_url = base_url_owned.clone();
            move || {
                let mut runtime = JavaScriptRuntime::new(&base_url, &html);
                runtime.process_loaded_document();
                runtime.dispatch_initial_load_events();
                let processed = runtime.snapshot();
                let _ = ready_tx.send(processed);

                while let Ok(command) = command_rx.recv() {
                    match command {
                        JavaScriptSessionCommand::DispatchEvent {
                            request,
                            response_tx,
                        } => {
                            let result = runtime.dispatch_dom_event(request);
                            let _ = response_tx.send(result);
                        }
                        JavaScriptSessionCommand::DispatchGlobalEvent {
                            event_type,
                            bubbles,
                            cancelable,
                            response_tx,
                        } => {
                            let result = runtime.dispatch_global_event_request(
                                &event_type,
                                bubbles,
                                cancelable,
                            );
                            let _ = response_tx.send(result);
                        }
                        JavaScriptSessionCommand::SetScrollPosition { y } => {
                            runtime.set_scroll_position(y);
                        }
                        JavaScriptSessionCommand::SetViewportSize { width, height } => {
                            runtime.set_viewport_size(width, height);
                        }
                        JavaScriptSessionCommand::SetAttribute {
                            node_id,
                            name,
                            value,
                        } => {
                            runtime.set_dom_attribute(node_id, &name, &value);
                        }
                        JavaScriptSessionCommand::Snapshot { response_tx } => {
                            let _ = response_tx.send(runtime.snapshot());
                        }
                        JavaScriptSessionCommand::Shutdown => break,
                    }
                }
            }
        });

    match worker {
        Ok(_) => match ready_rx.recv() {
            Ok(processed) => (processed, Some(JavaScriptSession { command_tx })),
            Err(_) => (
                process_document_scripts_error(
                    html_owned,
                    "js error: runtime worker failed to initialize".to_string(),
                ),
                None,
            ),
        },
        Err(_) => (
            process_document_scripts_error(
                html_owned,
                "js error: failed to start runtime worker".to_string(),
            ),
            None,
        ),
    }
}

fn process_document_scripts_error(html: String, message: String) -> ProcessedScriptHtml {
    ProcessedScriptHtml {
        html,
        title_override: None,
        console_logs: vec![message],
        navigation_target: None,
        soft_navigation_target: None,
        scroll_y: 0,
    }
}

fn process_document_scripts_impl(html: &str, base_url: &Url) -> ProcessedScriptHtml {
    let mut runtime = JavaScriptRuntime::new(base_url, html);
    runtime.process_loaded_document();
    runtime.dispatch_initial_load_events();
    runtime.snapshot()
}

struct JavaScriptRuntime {
    context: Context,
    executed_bytes: usize,
    host: String,
}

impl JavaScriptRuntime {
    fn new(base_url: &Url, html: &str) -> Self {
        let mut context = Context::default();
        context
            .runtime_limits_mut()
            .set_loop_iteration_limit(JS_LOOP_ITERATION_LIMIT);
        let dom = DomState::from_html(html);
        let initial_title = dom.title_text().unwrap_or_default();
        context.insert_data(JavaScriptHostData {
            state: RefCell::new(JavaScriptState {
                current_title: initial_title,
                title_dirty: false,
                write_buffer: String::new(),
                console_logs: Vec::new(),
                document_url: base_url.clone(),
                location_href: base_url.to_string(),
                soft_navigation_target: None,
                history_entries: vec![base_url.to_string()],
                history_index: 0,
                current_script: None,
                network_request_count: 0,
                network_response_bytes: 0,
                viewport_width: DEFAULT_VIEWPORT_WIDTH,
                viewport_height: DEFAULT_VIEWPORT_HEIGHT,
                scroll_y: 0,
                active_element_node_id: None,
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

    fn process_loaded_document(&mut self) {
        let mut iterations = 0;

        while let Some((script_id, attributes, inline_source)) = self.next_script() {
            iterations += 1;
            if iterations > MAX_SCRIPT_ITERATIONS {
                self.push_log("js skip: script iteration limit reached".to_string());
                break;
            }
            let document_url = self.document_url();
            if let Some(source) = load_script_source(&inline_source, &attributes, &document_url) {
                self.set_current_script(script_id);
                self.execute(&source);
                self.flush_document_writes(script_id);
                self.clear_current_script();
            }
            self.remove_script_node(script_id);
        }
    }

    fn dispatch_initial_load_events(&mut self) {
        let document_id = self.document_id();
        let _ = self.dispatch_dom_event_to_node(document_id, "readystatechange", false, false);
        let _ = self.dispatch_dom_event_to_node(document_id, "DOMContentLoaded", false, false);
        let _ = self.dispatch_dom_event_to_node(document_id, "load", false, false);
        let _ = self.dispatch_global_event("load", false, false);
        self.flush_pending_document_writes();
        self.process_loaded_document();
    }

    fn snapshot(&mut self) -> ProcessedScriptHtml {
        ProcessedScriptHtml {
            html: self.serialize_html(),
            title_override: self.title_override(),
            console_logs: self.take_logs(),
            navigation_target: self.navigation_target(),
            soft_navigation_target: self.take_soft_navigation_target(),
            scroll_y: scroll_position(&mut self.context),
        }
    }

    fn set_viewport_size(&mut self, width: u32, height: u32) {
        if let Some(host) = self.context.get_data::<JavaScriptHostData>() {
            let mut state = host.state.borrow_mut();
            state.viewport_width = width;
            state.viewport_height = height;
        }
    }

    fn set_scroll_position(&mut self, y: u32) -> bool {
        if let Some(host) = self.context.get_data::<JavaScriptHostData>() {
            let mut state = host.state.borrow_mut();
            let changed = state.scroll_y != y;
            state.scroll_y = y;
            return changed;
        }
        false
    }

    fn set_dom_attribute(&mut self, node_id: usize, name: &str, value: &str) {
        if let Some(host) = self.context.get_data::<JavaScriptHostData>() {
            host.state
                .borrow_mut()
                .dom
                .set_attribute(node_id, name, value);
        }
    }

    fn dispatch_dom_event(&mut self, request: DomEventRequest) -> DomEventDispatchResult {
        let default_prevented = self.dispatch_dom_event_request(request).unwrap_or(false);
        self.flush_pending_document_writes();
        self.process_loaded_document();
        DomEventDispatchResult {
            snapshot: self.snapshot(),
            default_prevented,
        }
    }

    fn dispatch_dom_event_request(&mut self, request: DomEventRequest) -> JsResult<bool> {
        let target = build_dom_node_object(&mut self.context, request.target_node_id);
        dispatch_dom_event_on_target(target, &request, &mut self.context)
    }

    fn dispatch_global_event_request(
        &mut self,
        event_type: &str,
        bubbles: bool,
        cancelable: bool,
    ) -> DomEventDispatchResult {
        let default_prevented = self
            .dispatch_global_event(event_type, bubbles, cancelable)
            .unwrap_or(false);
        self.flush_pending_document_writes();
        self.process_loaded_document();
        DomEventDispatchResult {
            snapshot: self.snapshot(),
            default_prevented,
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

    fn take_logs(&mut self) -> Vec<String> {
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

    fn navigation_target(&self) -> Option<String> {
        let host = self.context.get_data::<JavaScriptHostData>()?;
        let state = host.state.borrow();
        if state.soft_navigation_target.is_some() {
            return None;
        }
        let href = state.location_href.clone();
        let document_url = self.document_url().to_string();
        (href != document_url).then_some(href)
    }

    fn take_soft_navigation_target(&self) -> Option<String> {
        let host = self.context.get_data::<JavaScriptHostData>()?;
        host.state.borrow_mut().soft_navigation_target.take()
    }

    fn document_url(&self) -> Url {
        self.context
            .get_data::<JavaScriptHostData>()
            .map(|host| host.state.borrow().document_url.clone())
            .expect("document URL should exist in JS runtime")
    }

    fn document_id(&self) -> usize {
        self.context
            .get_data::<JavaScriptHostData>()
            .map(|host| host.state.borrow().dom.document_id)
            .unwrap_or(0)
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
            state
                .dom
                .node_attributes(script_id)
                .cloned()
                .unwrap_or_default(),
            state
                .dom
                .script_inline_source(script_id)
                .unwrap_or_default(),
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

    fn flush_pending_document_writes(&self) {
        let written = self.take_written_html();
        if written.trim().is_empty() {
            return;
        }
        if let Some(host) = self.context.get_data::<JavaScriptHostData>() {
            let mut state = host.state.borrow_mut();
            let parent_id = state
                .dom
                .body_id
                .or(state.dom.html_id)
                .unwrap_or(state.dom.document_id);
            state.dom.append_fragment(parent_id, &written);
        }
    }

    fn remove_script_node(&self, script_id: usize) {
        if let Some(host) = self.context.get_data::<JavaScriptHostData>() {
            host.state.borrow_mut().dom.detach_node(script_id);
        }
    }

    fn dispatch_dom_event_to_node(
        &mut self,
        node_id: usize,
        event_type: &str,
        bubbles: bool,
        cancelable: bool,
    ) -> JsResult<bool> {
        self.dispatch_dom_event_request(DomEventRequest {
            target_node_id: node_id,
            event_type: event_type.to_string(),
            bubbles,
            cancelable,
            ..Default::default()
        })
    }

    fn dispatch_global_event(
        &mut self,
        event_type: &str,
        bubbles: bool,
        cancelable: bool,
    ) -> JsResult<bool> {
        let target = self.context.global_object();
        let request = DomEventRequest {
            target_node_id: self.document_id(),
            event_type: event_type.to_string(),
            bubbles,
            cancelable,
            ..Default::default()
        };
        let event = build_dom_event_object(&mut self.context, &request, &target);
        dispatch_listeners_on_target(
            &target,
            &event_type.to_ascii_lowercase(),
            &event,
            true,
            EventDispatchPhase::AtTarget,
            &mut self.context,
        )?;
        if !event_flag_value(&event, "immediatePropagationStopped", &mut self.context) {
            dispatch_listeners_on_target(
                &target,
                &event_type.to_ascii_lowercase(),
                &event,
                false,
                EventDispatchPhase::AtTarget,
                &mut self.context,
            )?;
        }
        Ok(event_flag_value(
            &event,
            "defaultPrevented",
            &mut self.context,
        ))
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
            .unwrap_or_else(|| {
                self.node(parent_id)
                    .map(|parent| parent.children.len())
                    .unwrap_or(0)
            });
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

    fn append_fragment(&mut self, parent_id: usize, html: &str) {
        let fragment = parse_document(html);
        let fragment_root_id = self.push_node(None, &fragment);
        let fragment_children = self
            .node(fragment_root_id)
            .map(|node| node.children.clone())
            .unwrap_or_default();
        for child_id in fragment_children {
            self.append_child(parent_id, child_id);
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
            .map(
                |child_id| match self.node(*child_id).map(|child| &child.kind) {
                    Some(DomNodeKind::Text(text)) => text.clone(),
                    _ => self.serialize_node(*child_id),
                },
            )
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

    fn is_disabled(&self, node_id: usize) -> bool {
        self.get_attribute(node_id, "disabled").is_some()
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

    fn query_selector(
        &self,
        scope_id: usize,
        selector: &str,
        include_scope: bool,
    ) -> Option<usize> {
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
        self.descendant_nodes(scope_id, true)
            .into_iter()
            .find(|node_id| {
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
                    if self.get_attribute(node_id, &attribute.name).as_deref()
                        != Some(value.as_str())
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
            while index < bytes.len() && !matches!(bytes[index], b'#' | b'.' | b'[' | b':') {
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
                    while index < bytes.len() && is_selector_name_char(bytes[index]) {
                        index += 1;
                    }
                    selector.id = Some(token[start..index].to_string());
                }
                b'.' => {
                    index += 1;
                    let start = index;
                    while index < bytes.len() && is_selector_name_char(bytes[index]) {
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
    let (name, value) = input
        .split_once('=')
        .map_or((input, None), |(name, value)| {
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
    let hash_setter =
        NativeFunction::from_fn_ptr(js_location_set_hash).to_js_function(context.realm());
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
            Some(hash_setter),
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
    let cookie_getter =
        NativeFunction::from_fn_ptr(js_document_get_cookie).to_js_function(context.realm());
    let cookie_setter =
        NativeFunction::from_fn_ptr(js_document_set_cookie).to_js_function(context.realm());
    let active_element_getter =
        NativeFunction::from_fn_ptr(js_document_get_active_element).to_js_function(context.realm());

    let global_object = context.global_object();
    let document = ObjectInitializer::with_native_data(
        DomNodeHandle {
            node_id: document_id,
        },
        context,
    )
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
        NativeFunction::from_fn_ptr(js_document_has_focus),
        js_string!("hasFocus"),
        0,
    )
    .function(
        NativeFunction::from_fn_ptr(js_add_event_listener),
        js_string!("addEventListener"),
        2,
    )
    .function(
        NativeFunction::from_fn_ptr(js_remove_event_listener),
        js_string!("removeEventListener"),
        2,
    )
    .property(js_string!("location"), location.clone(), Attribute::all())
    .property(js_string!("body"), body_object, Attribute::all())
    .property(js_string!("head"), head_object, Attribute::all())
    .property(js_string!("documentElement"), html_object, Attribute::all())
    .accessor(
        js_string!("activeElement"),
        Some(active_element_getter),
        None,
        Attribute::all(),
    )
    .property(js_string!("fonts"), document_fonts, Attribute::all())
    .accessor(
        js_string!("cookie"),
        Some(cookie_getter),
        Some(cookie_setter),
        Attribute::all(),
    )
    .property(
        js_string!("readyState"),
        js_string!("complete"),
        Attribute::all(),
    )
    .property(
        js_string!("compatMode"),
        js_string!("CSS1Compat"),
        Attribute::all(),
    )
    .property(js_string!("hidden"), false, Attribute::all())
    .property(
        js_string!("visibilityState"),
        js_string!("visible"),
        Attribute::all(),
    )
    .property(
        js_string!("defaultView"),
        global_object.clone(),
        Attribute::all(),
    )
    .build();
    store_dom_node_object(context, document_id, &document);

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
        .property(js_string!("cookieEnabled"), true, Attribute::all())
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
    let history_length_getter =
        NativeFunction::from_fn_ptr(js_history_length).to_js_function(context.realm());
    let history = ObjectInitializer::new(context)
        .function(
            NativeFunction::from_fn_ptr(js_history_push_state),
            js_string!("pushState"),
            3,
        )
        .function(
            NativeFunction::from_fn_ptr(js_history_replace_state),
            js_string!("replaceState"),
            3,
        )
        .function(
            NativeFunction::from_fn_ptr(js_history_back),
            js_string!("back"),
            0,
        )
        .function(
            NativeFunction::from_fn_ptr(js_history_forward),
            js_string!("forward"),
            0,
        )
        .accessor(
            js_string!("length"),
            Some(history_length_getter),
            None,
            Attribute::all(),
        )
        .build();
    let local_storage = build_storage_object(context, StorageKind::Local);
    let session_storage = build_storage_object(context, StorageKind::Session);
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
        .register_global_property(js_string!("localStorage"), local_storage, Attribute::all())
        .expect("localStorage should be installable");
    context
        .register_global_property(
            js_string!("sessionStorage"),
            session_storage,
            Attribute::all(),
        )
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
            NativeFunction::from_fn_ptr(js_remove_event_listener),
        )
        .expect("removeEventListener should be installable");
    context
        .register_global_builtin_callable(
            js_string!("dispatchEvent"),
            1,
            NativeFunction::from_fn_ptr(js_dom_dispatch_event),
        )
        .expect("dispatchEvent should be installable");
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
            js_string!("scrollTo"),
            2,
            NativeFunction::from_fn_ptr(js_window_scroll_to),
        )
        .expect("scrollTo should be installable");
    context
        .register_global_builtin_callable(
            js_string!("scrollBy"),
            2,
            NativeFunction::from_fn_ptr(js_window_scroll_by),
        )
        .expect("scrollBy should be installable");
    context
        .register_global_builtin_callable(
            js_string!("scroll"),
            2,
            NativeFunction::from_fn_ptr(js_window_scroll_to),
        )
        .expect("scroll should be installable");
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
    let inner_width_getter =
        NativeFunction::from_fn_ptr(js_window_get_inner_width).to_js_function(context.realm());
    let inner_height_getter =
        NativeFunction::from_fn_ptr(js_window_get_inner_height).to_js_function(context.realm());
    let scroll_y_getter =
        NativeFunction::from_fn_ptr(js_window_get_scroll_y).to_js_function(context.realm());
    let page_y_offset_getter =
        NativeFunction::from_fn_ptr(js_window_get_page_y_offset).to_js_function(context.realm());
    context
        .global_object()
        .define_property_or_throw(
            js_string!("innerWidth"),
            PropertyDescriptor::builder()
                .get(inner_width_getter)
                .enumerable(false)
                .configurable(true),
            context,
        )
        .expect("innerWidth should be installable");
    context
        .global_object()
        .define_property_or_throw(
            js_string!("innerHeight"),
            PropertyDescriptor::builder()
                .get(inner_height_getter)
                .enumerable(false)
                .configurable(true),
            context,
        )
        .expect("innerHeight should be installable");
    context
        .global_object()
        .define_property_or_throw(
            js_string!("scrollY"),
            PropertyDescriptor::builder()
                .get(scroll_y_getter.clone())
                .enumerable(false)
                .configurable(true),
            context,
        )
        .expect("scrollY should be installable");
    context
        .global_object()
        .define_property_or_throw(
            js_string!("pageYOffset"),
            PropertyDescriptor::builder()
                .get(page_y_offset_getter)
                .enumerable(false)
                .configurable(true),
            context,
        )
        .expect("pageYOffset should be installable");

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

    context
        .eval(Source::from_bytes(
            // Lightweight constructor shim: enough for `new XMLHttpRequest()`, but
            // prototype/instanceof behavior is still intentionally incomplete.
            "globalThis.XMLHttpRequest = function XMLHttpRequest(){ return __tobiraCreateXMLHttpRequest(); };",
        ))
        .expect("XMLHttpRequest bootstrap should evaluate");
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

fn build_event_listener_list_stub(context: &mut Context) -> boa_engine::object::JsObject {
    ObjectInitializer::new(context)
        .property(js_string!("length"), 0, Attribute::all())
        .build()
}

fn build_event_listener_record(
    context: &mut Context,
    callback: JsValue,
    options: &EventListenerOptions,
) -> boa_engine::object::JsObject {
    ObjectInitializer::new(context)
        .property(js_string!("callback"), callback, Attribute::all())
        .property(js_string!("capture"), options.capture, Attribute::all())
        .property(js_string!("once"), options.once, Attribute::all())
        .property(js_string!("passive"), options.passive, Attribute::all())
        .build()
}

fn dom_node_cache(context: &mut Context) -> JsResult<boa_engine::object::JsObject> {
    let global = context.global_object();
    let cache_key = js_string!("__tobiraDomNodeCache");
    let existing = global.get(cache_key.clone(), context)?;
    if let Some(object) = existing.as_object() {
        return Ok(object.clone());
    }

    let cache = ObjectInitializer::new(context).build();
    global.set(cache_key, cache.clone(), true, context)?;
    Ok(cache)
}

fn dom_style_cache(context: &mut Context) -> JsResult<boa_engine::object::JsObject> {
    let global = context.global_object();
    let cache_key = js_string!("__tobiraDomStyleCache");
    let existing = global.get(cache_key.clone(), context)?;
    if let Some(object) = existing.as_object() {
        return Ok(object.clone());
    }

    let cache = ObjectInitializer::new(context).build();
    global.set(cache_key, cache.clone(), true, context)?;
    Ok(cache)
}

fn cached_dom_node_object(
    context: &mut Context,
    node_id: usize,
) -> Option<boa_engine::object::JsObject> {
    let cache = dom_node_cache(context).ok()?;
    let key = js_string!(node_id.to_string());
    cache
        .get(key, context)
        .ok()
        .and_then(|value| value.as_object())
}

fn store_dom_node_object(
    context: &mut Context,
    node_id: usize,
    object: &boa_engine::object::JsObject,
) {
    if let Ok(cache) = dom_node_cache(context) {
        let _ = cache.set(
            js_string!(node_id.to_string()),
            object.clone(),
            true,
            context,
        );
    }
}

fn cached_dom_style_object(
    context: &mut Context,
    node_id: usize,
) -> Option<boa_engine::object::JsObject> {
    let cache = dom_style_cache(context).ok()?;
    let key = js_string!(node_id.to_string());
    cache
        .get(key, context)
        .ok()
        .and_then(|value| value.as_object())
}

fn store_dom_style_object(
    context: &mut Context,
    node_id: usize,
    object: &boa_engine::object::JsObject,
) {
    if let Ok(cache) = dom_style_cache(context) {
        let _ = cache.set(
            js_string!(node_id.to_string()),
            object.clone(),
            true,
            context,
        );
    }
}

fn hidden_event_listener_store(
    target: &boa_engine::object::JsObject,
    context: &mut Context,
) -> JsResult<boa_engine::object::JsObject> {
    let key = js_string!("__tobiraEventListeners");
    let existing = target.get(key.clone(), context)?;
    if let Some(object) = existing.as_object() {
        return Ok(object.clone());
    }

    let store = ObjectInitializer::new(context).build();
    target.set(key, store.clone(), true, context)?;
    Ok(store)
}

fn hidden_event_listener_list(
    target: &boa_engine::object::JsObject,
    event_name: &str,
    context: &mut Context,
) -> JsResult<boa_engine::object::JsObject> {
    let store = hidden_event_listener_store(target, context)?;
    let key = js_string!(event_name.to_ascii_lowercase());
    let existing = store.get(key.clone(), context)?;
    if let Some(object) = existing.as_object() {
        return Ok(object.clone());
    }

    let list = build_event_listener_list_stub(context);
    store.set(key, list.clone(), true, context)?;
    Ok(list)
}

fn event_listener_options_from_value(
    value: Option<&JsValue>,
    context: &mut Context,
) -> JsResult<EventListenerOptions> {
    let Some(value) = value else {
        return Ok(EventListenerOptions::default());
    };

    if value.is_undefined() || value.is_null() {
        return Ok(EventListenerOptions::default());
    }

    if let Some(object) = value.as_object() {
        let capture = object.get(js_string!("capture"), context)?.to_boolean();
        let once = object.get(js_string!("once"), context)?.to_boolean();
        let passive = object.get(js_string!("passive"), context)?.to_boolean();
        return Ok(EventListenerOptions {
            capture,
            once,
            passive,
        });
    }

    Ok(EventListenerOptions {
        capture: value.to_boolean(),
        once: false,
        passive: false,
    })
}

fn is_same_js_callback(lhs: &JsValue, rhs: &JsValue) -> bool {
    match (lhs.as_object(), rhs.as_object()) {
        (Some(lhs), Some(rhs)) => boa_engine::object::JsObject::equals(&lhs, &rhs),
        _ => false,
    }
}

fn append_event_listener(
    target: &boa_engine::object::JsObject,
    event_name: &str,
    callback: JsValue,
    options: EventListenerOptions,
    context: &mut Context,
) -> JsResult<()> {
    let list = hidden_event_listener_list(target, event_name, context)?;
    let length = list
        .get(js_string!("length"), context)?
        .to_length(context)? as usize;
    for index in 0..length {
        let existing = list.get(js_string!(index.to_string()), context)?;
        let Some(existing_record) = existing.as_object() else {
            continue;
        };
        let existing_callback = existing_record.get(js_string!("callback"), context)?;
        let existing_capture = existing_record
            .get(js_string!("capture"), context)
            .unwrap_or_else(|_| JsValue::undefined())
            .to_boolean();
        if existing_capture == options.capture && is_same_js_callback(&existing_callback, &callback)
        {
            return Ok(());
        }
    }

    list.set(
        js_string!(length.to_string()),
        JsValue::from(build_event_listener_record(context, callback, &options)),
        true,
        context,
    )?;
    list.set(
        js_string!("length"),
        JsValue::new((length + 1) as i32),
        true,
        context,
    )?;
    Ok(())
}

fn remove_event_listener(
    target: &boa_engine::object::JsObject,
    event_name: &str,
    callback: &JsValue,
    capture: bool,
    context: &mut Context,
) -> JsResult<()> {
    let store = hidden_event_listener_store(target, context)?;
    let key = js_string!(event_name.to_ascii_lowercase());
    let existing = store.get(key.clone(), context)?;
    let Some(list) = existing.as_object() else {
        return Ok(());
    };

    let length = list
        .get(js_string!("length"), context)?
        .to_length(context)? as usize;
    let mut retained = Vec::new();
    for index in 0..length {
        let value = list.get(js_string!(index.to_string()), context)?;
        let Some(record) = value.as_object() else {
            retained.push(value);
            continue;
        };
        let record_callback = record.get(js_string!("callback"), context)?;
        let record_capture = record
            .get(js_string!("capture"), context)
            .unwrap_or_else(|_| JsValue::undefined())
            .to_boolean();
        if record_capture != capture || !is_same_js_callback(&record_callback, callback) {
            retained.push(value);
        }
    }
    let retained_len = retained.len();

    let new_list = build_event_listener_list_stub(context);
    for (index, value) in retained.into_iter().enumerate() {
        let _ = new_list.set(js_string!(index.to_string()), value, true, context);
    }
    new_list.set(
        js_string!("length"),
        JsValue::new(retained_len as i32),
        true,
        context,
    )?;
    store.set(key, new_list, true, context)?;
    Ok(())
}

fn event_listener_entries(
    target: &boa_engine::object::JsObject,
    event_name: &str,
    context: &mut Context,
) -> JsResult<Vec<EventListenerEntry>> {
    let store = hidden_event_listener_store(target, context)?;
    let key = js_string!(event_name);
    let existing = store.get(key, context)?;
    let Some(list) = existing.as_object() else {
        return Ok(Vec::new());
    };

    let length = list
        .get(js_string!("length"), context)?
        .to_length(context)? as usize;
    let mut entries = Vec::with_capacity(length);
    for index in 0..length {
        let value = list.get(js_string!(index.to_string()), context)?;
        let Some(record) = value.as_object() else {
            continue;
        };
        let callback = record.get(js_string!("callback"), context)?;
        let options = EventListenerOptions {
            capture: record
                .get(js_string!("capture"), context)
                .unwrap_or_else(|_| JsValue::undefined())
                .to_boolean(),
            once: record
                .get(js_string!("once"), context)
                .unwrap_or_else(|_| JsValue::undefined())
                .to_boolean(),
            passive: record
                .get(js_string!("passive"), context)
                .unwrap_or_else(|_| JsValue::undefined())
                .to_boolean(),
        };
        entries.push(EventListenerEntry { callback, options });
    }
    Ok(entries)
}

fn build_dom_event_object(
    context: &mut Context,
    request: &DomEventRequest,
    target: &boa_engine::object::JsObject,
) -> boa_engine::object::JsObject {
    let event = ObjectInitializer::new(context)
        .function(
            NativeFunction::from_fn_ptr(js_event_prevent_default),
            js_string!("preventDefault"),
            0,
        )
        .function(
            NativeFunction::from_fn_ptr(js_event_stop_propagation),
            js_string!("stopPropagation"),
            0,
        )
        .function(
            NativeFunction::from_fn_ptr(js_event_stop_immediate_propagation),
            js_string!("stopImmediatePropagation"),
            0,
        )
        .property(
            js_string!("type"),
            js_string!(request.event_type.as_str()),
            Attribute::all(),
        )
        .property(js_string!("bubbles"), request.bubbles, Attribute::all())
        .property(
            js_string!("cancelable"),
            request.cancelable,
            Attribute::all(),
        )
        .property(js_string!("eventPhase"), 0, Attribute::all())
        .property(js_string!("defaultPrevented"), false, Attribute::all())
        .property(js_string!("cancelBubble"), false, Attribute::all())
        .property(js_string!("propagationStopped"), false, Attribute::all())
        .property(
            js_string!("immediatePropagationStopped"),
            false,
            Attribute::all(),
        )
        .property(js_string!("target"), target.clone(), Attribute::all())
        .property(
            js_string!("currentTarget"),
            target.clone(),
            Attribute::all(),
        )
        .build();
    if let Some(key) = &request.key {
        let _ = event.set(js_string!("key"), js_string!(key.as_str()), true, context);
    }
    if let Some(code) = &request.code {
        let _ = event.set(js_string!("code"), js_string!(code.as_str()), true, context);
    }
    let _ = event.set(
        js_string!("repeat"),
        JsValue::new(request.repeat),
        true,
        context,
    );
    let _ = event.set(
        js_string!("altKey"),
        JsValue::new(request.alt_key),
        true,
        context,
    );
    let _ = event.set(
        js_string!("ctrlKey"),
        JsValue::new(request.ctrl_key),
        true,
        context,
    );
    let _ = event.set(
        js_string!("shiftKey"),
        JsValue::new(request.shift_key),
        true,
        context,
    );
    let _ = event.set(
        js_string!("metaKey"),
        JsValue::new(request.meta_key),
        true,
        context,
    );
    event
}

fn event_bool_property(this: &JsValue, name: &str, context: &mut Context) -> bool {
    this.as_object()
        .and_then(|object| object.get(js_string!(name), context).ok())
        .and_then(|value| value.to_boolean().then_some(true).or(Some(false)))
        .unwrap_or(false)
}

fn viewport_size(context: &mut Context) -> (u32, u32) {
    context
        .get_data::<JavaScriptHostData>()
        .map(|host| {
            let state = host.state.borrow();
            (state.viewport_width, state.viewport_height)
        })
        .unwrap_or((DEFAULT_VIEWPORT_WIDTH, DEFAULT_VIEWPORT_HEIGHT))
}

fn viewport_width(context: &mut Context) -> u32 {
    viewport_size(context).0
}

fn viewport_height(context: &mut Context) -> u32 {
    viewport_size(context).1
}

fn scroll_position(context: &mut Context) -> u32 {
    context
        .get_data::<JavaScriptHostData>()
        .map(|host| host.state.borrow().scroll_y)
        .unwrap_or(0)
}

fn js_window_get_inner_width(
    _: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    Ok(JsValue::new(viewport_width(context)))
}

fn js_window_get_inner_height(
    _: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    Ok(JsValue::new(viewport_height(context)))
}

fn js_window_get_scroll_y(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::new(scroll_position(context)))
}

fn js_window_get_page_y_offset(
    _: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    js_window_get_scroll_y(&JsValue::undefined(), &[], context)
}

fn scroll_offset_from_value(value: &JsValue, context: &mut Context) -> JsResult<i64> {
    let number = value.to_number(context)?;
    if !number.is_finite() {
        return Ok(0);
    }
    Ok(number.round() as i64)
}

fn scroll_offset_from_args(args: &[JsValue], context: &mut Context) -> JsResult<i64> {
    if let Some(first) = args.first()
        && let Some(object) = first.as_object()
        && args.len() == 1
    {
        let top = object.get(js_string!("top"), context)?;
        if !top.is_undefined() {
            return scroll_offset_from_value(&top, context);
        }
        let y = object.get(js_string!("y"), context)?;
        if !y.is_undefined() {
            return scroll_offset_from_value(&y, context);
        }
        return Ok(0);
    }

    if let Some(second) = args.get(1) {
        return scroll_offset_from_value(second, context);
    }
    if let Some(first) = args.first() {
        return scroll_offset_from_value(first, context);
    }
    Ok(0)
}

fn js_window_scroll_to(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let target = scroll_offset_from_args(args, context)?.max(0) as u32;
    let changed = if let Some(host) = context.get_data::<JavaScriptHostData>() {
        let mut state = host.state.borrow_mut();
        let changed = state.scroll_y != target;
        state.scroll_y = target;
        changed
    } else {
        false
    };
    if changed {
        let _ = runtime_scroll_event(context);
    }
    Ok(JsValue::undefined())
}

fn js_window_scroll_by(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let delta = scroll_offset_from_args(args, context)?;
    let changed = if let Some(host) = context.get_data::<JavaScriptHostData>() {
        let current = host.state.borrow().scroll_y;
        let magnitude = delta.unsigned_abs().min(u32::MAX as u64) as u32;
        let target = if delta.is_negative() {
            current.saturating_sub(magnitude)
        } else {
            current.saturating_add(magnitude)
        };
        let mut state = host.state.borrow_mut();
        let changed = state.scroll_y != target;
        state.scroll_y = target;
        changed
    } else {
        false
    };
    if changed {
        let _ = runtime_scroll_event(context);
    }
    Ok(JsValue::undefined())
}

fn js_dom_set_scroll_top(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    if this_node_id(this).is_none() {
        return Ok(JsValue::undefined());
    }
    let target = scroll_offset_from_value(args.first().unwrap_or(&JsValue::undefined()), context)?
        .max(0) as u32;
    let changed = if let Some(host) = context.get_data::<JavaScriptHostData>() {
        let mut state = host.state.borrow_mut();
        let changed = state.scroll_y != target;
        state.scroll_y = target;
        changed
    } else {
        false
    };
    if changed {
        let _ = runtime_scroll_event(context);
    }
    Ok(JsValue::undefined())
}

fn runtime_scroll_event(context: &mut Context) -> JsResult<()> {
    let target_node_id = context
        .get_data::<JavaScriptHostData>()
        .map(|host| host.state.borrow().dom.document_id)
        .unwrap_or(0);
    let request = DomEventRequest {
        target_node_id,
        event_type: "scroll".to_string(),
        bubbles: false,
        cancelable: false,
        ..Default::default()
    };
    let target = context.global_object().clone();
    let _ = dispatch_dom_event_on_target(target, &request, context)?;
    Ok(())
}

fn js_dom_get_scroll_top(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let value = if this_node_id(this).is_some() {
        scroll_position(context)
    } else {
        0
    };
    Ok(JsValue::new(value))
}

fn js_dom_get_client_width(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let width = if this_node_id(this).is_some() {
        viewport_width(context)
    } else {
        DEFAULT_VIEWPORT_WIDTH
    };
    Ok(JsValue::new(width))
}

fn js_dom_get_client_height(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let height = if this_node_id(this).is_some() {
        viewport_height(context)
    } else {
        DEFAULT_VIEWPORT_HEIGHT
    };
    Ok(JsValue::new(height))
}

fn js_dom_get_scroll_width(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    js_dom_get_client_width(this, &[], context)
}

fn js_dom_get_scroll_height(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    js_dom_get_client_height(this, &[], context)
}

fn active_element_node_id(context: &mut Context) -> Option<usize> {
    let host = context.get_data::<JavaScriptHostData>()?;
    let state = host.state.borrow();
    state
        .active_element_node_id
        .or(state.dom.body_id)
        .or(state.dom.html_id)
        .or(Some(state.dom.document_id))
}

fn js_document_get_active_element(
    _: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(node_id) = active_element_node_id(context) else {
        return Ok(JsValue::null());
    };
    Ok(JsValue::from(build_dom_node_object(context, node_id)))
}

fn js_document_has_focus(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::new(
        context
            .get_data::<JavaScriptHostData>()
            .map(|host| host.state.borrow().active_element_node_id.is_some())
            .unwrap_or(false),
    ))
}

fn set_event_bool_property(
    this: &JsValue,
    name: &str,
    value: bool,
    context: &mut Context,
) -> JsResult<()> {
    let Some(object) = this.as_object() else {
        return Ok(());
    };
    object.set(js_string!(name), JsValue::new(value), true, context)?;
    Ok(())
}

fn set_event_internal_bool_property(
    event: &boa_engine::object::JsObject,
    name: &str,
    value: bool,
    context: &mut Context,
) -> JsResult<()> {
    event.set(js_string!(name), JsValue::new(value), true, context)?;
    Ok(())
}

fn build_dom_node_object(context: &mut Context, node_id: usize) -> boa_engine::object::JsObject {
    if let Some(cached) = cached_dom_node_object(context, node_id) {
        return cached;
    }

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
    let get_client_width =
        NativeFunction::from_fn_ptr(js_dom_get_client_width).to_js_function(context.realm());
    let get_client_height =
        NativeFunction::from_fn_ptr(js_dom_get_client_height).to_js_function(context.realm());
    let get_scroll_width =
        NativeFunction::from_fn_ptr(js_dom_get_scroll_width).to_js_function(context.realm());
    let get_scroll_height =
        NativeFunction::from_fn_ptr(js_dom_get_scroll_height).to_js_function(context.realm());
    let get_scroll_top =
        NativeFunction::from_fn_ptr(js_dom_get_scroll_top).to_js_function(context.realm());
    let set_scroll_top =
        NativeFunction::from_fn_ptr(js_dom_set_scroll_top).to_js_function(context.realm());
    let style = build_dom_style_object(context, node_id);
    let object = ObjectInitializer::with_native_data(DomNodeHandle { node_id }, context)
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
            NativeFunction::from_fn_ptr(js_add_event_listener),
            js_string!("addEventListener"),
            2,
        )
        .function(
            NativeFunction::from_fn_ptr(js_remove_event_listener),
            js_string!("removeEventListener"),
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
            NativeFunction::from_fn_ptr(js_dom_dispatch_event),
            js_string!("dispatchEvent"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_dom_focus),
            js_string!("focus"),
            0,
        )
        .function(
            NativeFunction::from_fn_ptr(js_dom_blur),
            js_string!("blur"),
            0,
        )
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
        .accessor(
            js_string!("clientWidth"),
            Some(get_client_width),
            None,
            Attribute::all(),
        )
        .accessor(
            js_string!("clientHeight"),
            Some(get_client_height),
            None,
            Attribute::all(),
        )
        .accessor(
            js_string!("scrollTop"),
            Some(get_scroll_top),
            Some(set_scroll_top),
            Attribute::all(),
        )
        .accessor(
            js_string!("scrollWidth"),
            Some(get_scroll_width),
            None,
            Attribute::all(),
        )
        .accessor(
            js_string!("scrollHeight"),
            Some(get_scroll_height),
            None,
            Attribute::all(),
        )
        .build();
    store_dom_node_object(context, node_id, &object);
    object
}

fn build_dom_style_object(context: &mut Context, node_id: usize) -> boa_engine::object::JsObject {
    if let Some(cached) = cached_dom_style_object(context, node_id) {
        return cached;
    }

    let css_text_getter =
        NativeFunction::from_fn_ptr(js_dom_style_get_css_text).to_js_function(context.realm());
    let css_text_setter =
        NativeFunction::from_fn_ptr(js_dom_style_set_css_text).to_js_function(context.realm());
    let display_getter =
        NativeFunction::from_fn_ptr(js_dom_style_get_display).to_js_function(context.realm());
    let display_setter =
        NativeFunction::from_fn_ptr(js_dom_style_set_display).to_js_function(context.realm());
    let color_getter =
        NativeFunction::from_fn_ptr(js_dom_style_get_color).to_js_function(context.realm());
    let color_setter =
        NativeFunction::from_fn_ptr(js_dom_style_set_color).to_js_function(context.realm());
    let font_style_getter =
        NativeFunction::from_fn_ptr(js_dom_style_get_font_style).to_js_function(context.realm());
    let font_style_setter =
        NativeFunction::from_fn_ptr(js_dom_style_set_font_style).to_js_function(context.realm());
    let text_decoration_getter = NativeFunction::from_fn_ptr(js_dom_style_get_text_decoration)
        .to_js_function(context.realm());
    let text_decoration_setter = NativeFunction::from_fn_ptr(js_dom_style_set_text_decoration)
        .to_js_function(context.realm());
    let text_transform_getter = NativeFunction::from_fn_ptr(js_dom_style_get_text_transform)
        .to_js_function(context.realm());
    let text_transform_setter = NativeFunction::from_fn_ptr(js_dom_style_set_text_transform)
        .to_js_function(context.realm());
    let text_indent_getter =
        NativeFunction::from_fn_ptr(js_dom_style_get_text_indent).to_js_function(context.realm());
    let text_indent_setter =
        NativeFunction::from_fn_ptr(js_dom_style_set_text_indent).to_js_function(context.realm());
    let letter_spacing_getter = NativeFunction::from_fn_ptr(js_dom_style_get_letter_spacing)
        .to_js_function(context.realm());
    let letter_spacing_setter = NativeFunction::from_fn_ptr(js_dom_style_set_letter_spacing)
        .to_js_function(context.realm());
    let max_width_getter =
        NativeFunction::from_fn_ptr(js_dom_style_get_max_width).to_js_function(context.realm());
    let max_width_setter =
        NativeFunction::from_fn_ptr(js_dom_style_set_max_width).to_js_function(context.realm());
    let min_width_getter =
        NativeFunction::from_fn_ptr(js_dom_style_get_min_width).to_js_function(context.realm());
    let min_width_setter =
        NativeFunction::from_fn_ptr(js_dom_style_set_min_width).to_js_function(context.realm());
    let max_height_getter =
        NativeFunction::from_fn_ptr(js_dom_style_get_max_height).to_js_function(context.realm());
    let max_height_setter =
        NativeFunction::from_fn_ptr(js_dom_style_set_max_height).to_js_function(context.realm());
    let min_height_getter =
        NativeFunction::from_fn_ptr(js_dom_style_get_min_height).to_js_function(context.realm());
    let min_height_setter =
        NativeFunction::from_fn_ptr(js_dom_style_set_min_height).to_js_function(context.realm());
    let border_width_getter =
        NativeFunction::from_fn_ptr(js_dom_style_get_border_width).to_js_function(context.realm());
    let border_width_setter =
        NativeFunction::from_fn_ptr(js_dom_style_set_border_width).to_js_function(context.realm());
    let border_color_getter =
        NativeFunction::from_fn_ptr(js_dom_style_get_border_color).to_js_function(context.realm());
    let border_color_setter =
        NativeFunction::from_fn_ptr(js_dom_style_set_border_color).to_js_function(context.realm());
    let border_style_getter =
        NativeFunction::from_fn_ptr(js_dom_style_get_border_style).to_js_function(context.realm());
    let border_style_setter =
        NativeFunction::from_fn_ptr(js_dom_style_set_border_style).to_js_function(context.realm());
    let background_color_getter = NativeFunction::from_fn_ptr(js_dom_style_get_background_color)
        .to_js_function(context.realm());
    let background_color_setter = NativeFunction::from_fn_ptr(js_dom_style_set_background_color)
        .to_js_function(context.realm());
    let width_getter =
        NativeFunction::from_fn_ptr(js_dom_style_get_width).to_js_function(context.realm());
    let width_setter =
        NativeFunction::from_fn_ptr(js_dom_style_set_width).to_js_function(context.realm());
    let height_getter =
        NativeFunction::from_fn_ptr(js_dom_style_get_height).to_js_function(context.realm());
    let height_setter =
        NativeFunction::from_fn_ptr(js_dom_style_set_height).to_js_function(context.realm());
    let font_size_getter =
        NativeFunction::from_fn_ptr(js_dom_style_get_font_size).to_js_function(context.realm());
    let font_size_setter =
        NativeFunction::from_fn_ptr(js_dom_style_set_font_size).to_js_function(context.realm());
    let font_weight_getter =
        NativeFunction::from_fn_ptr(js_dom_style_get_font_weight).to_js_function(context.realm());
    let font_weight_setter =
        NativeFunction::from_fn_ptr(js_dom_style_set_font_weight).to_js_function(context.realm());
    let font_family_getter =
        NativeFunction::from_fn_ptr(js_dom_style_get_font_family).to_js_function(context.realm());
    let font_family_setter =
        NativeFunction::from_fn_ptr(js_dom_style_set_font_family).to_js_function(context.realm());
    let text_align_getter =
        NativeFunction::from_fn_ptr(js_dom_style_get_text_align).to_js_function(context.realm());
    let text_align_setter =
        NativeFunction::from_fn_ptr(js_dom_style_set_text_align).to_js_function(context.realm());
    let vertical_align_getter = NativeFunction::from_fn_ptr(js_dom_style_get_vertical_align)
        .to_js_function(context.realm());
    let vertical_align_setter = NativeFunction::from_fn_ptr(js_dom_style_set_vertical_align)
        .to_js_function(context.realm());
    let margin_getter =
        NativeFunction::from_fn_ptr(js_dom_style_get_margin).to_js_function(context.realm());
    let margin_setter =
        NativeFunction::from_fn_ptr(js_dom_style_set_margin).to_js_function(context.realm());
    let padding_getter =
        NativeFunction::from_fn_ptr(js_dom_style_get_padding).to_js_function(context.realm());
    let padding_setter =
        NativeFunction::from_fn_ptr(js_dom_style_set_padding).to_js_function(context.realm());
    let opacity_getter =
        NativeFunction::from_fn_ptr(js_dom_style_get_opacity).to_js_function(context.realm());
    let opacity_setter =
        NativeFunction::from_fn_ptr(js_dom_style_set_opacity).to_js_function(context.realm());
    let line_height_getter =
        NativeFunction::from_fn_ptr(js_dom_style_get_line_height).to_js_function(context.realm());
    let line_height_setter =
        NativeFunction::from_fn_ptr(js_dom_style_set_line_height).to_js_function(context.realm());
    let white_space_getter =
        NativeFunction::from_fn_ptr(js_dom_style_get_white_space).to_js_function(context.realm());
    let white_space_setter =
        NativeFunction::from_fn_ptr(js_dom_style_set_white_space).to_js_function(context.realm());
    let cursor_getter =
        NativeFunction::from_fn_ptr(js_dom_style_get_cursor).to_js_function(context.realm());
    let cursor_setter =
        NativeFunction::from_fn_ptr(js_dom_style_set_cursor).to_js_function(context.realm());
    let overflow_getter =
        NativeFunction::from_fn_ptr(js_dom_style_get_overflow).to_js_function(context.realm());
    let overflow_setter =
        NativeFunction::from_fn_ptr(js_dom_style_set_overflow).to_js_function(context.realm());
    let position_getter =
        NativeFunction::from_fn_ptr(js_dom_style_get_position).to_js_function(context.realm());
    let position_setter =
        NativeFunction::from_fn_ptr(js_dom_style_set_position).to_js_function(context.realm());
    let object = ObjectInitializer::with_native_data(DomStyleHandle { node_id }, context)
        .function(
            NativeFunction::from_fn_ptr(js_dom_style_get_property_value),
            js_string!("getPropertyValue"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_dom_style_set_property),
            js_string!("setProperty"),
            2,
        )
        .function(
            NativeFunction::from_fn_ptr(js_dom_style_remove_property),
            js_string!("removeProperty"),
            1,
        )
        .accessor(
            js_string!("cssText"),
            Some(css_text_getter),
            Some(css_text_setter),
            Attribute::all(),
        )
        .accessor(
            js_string!("display"),
            Some(display_getter),
            Some(display_setter),
            Attribute::all(),
        )
        .accessor(
            js_string!("color"),
            Some(color_getter),
            Some(color_setter),
            Attribute::all(),
        )
        .accessor(
            js_string!("fontStyle"),
            Some(font_style_getter),
            Some(font_style_setter),
            Attribute::all(),
        )
        .accessor(
            js_string!("textDecoration"),
            Some(text_decoration_getter),
            Some(text_decoration_setter),
            Attribute::all(),
        )
        .accessor(
            js_string!("textTransform"),
            Some(text_transform_getter),
            Some(text_transform_setter),
            Attribute::all(),
        )
        .accessor(
            js_string!("textIndent"),
            Some(text_indent_getter),
            Some(text_indent_setter),
            Attribute::all(),
        )
        .accessor(
            js_string!("letterSpacing"),
            Some(letter_spacing_getter),
            Some(letter_spacing_setter),
            Attribute::all(),
        )
        .accessor(
            js_string!("backgroundColor"),
            Some(background_color_getter),
            Some(background_color_setter),
            Attribute::all(),
        )
        .accessor(
            js_string!("width"),
            Some(width_getter),
            Some(width_setter),
            Attribute::all(),
        )
        .accessor(
            js_string!("height"),
            Some(height_getter),
            Some(height_setter),
            Attribute::all(),
        )
        .accessor(
            js_string!("maxWidth"),
            Some(max_width_getter),
            Some(max_width_setter),
            Attribute::all(),
        )
        .accessor(
            js_string!("minWidth"),
            Some(min_width_getter),
            Some(min_width_setter),
            Attribute::all(),
        )
        .accessor(
            js_string!("maxHeight"),
            Some(max_height_getter),
            Some(max_height_setter),
            Attribute::all(),
        )
        .accessor(
            js_string!("minHeight"),
            Some(min_height_getter),
            Some(min_height_setter),
            Attribute::all(),
        )
        .accessor(
            js_string!("fontSize"),
            Some(font_size_getter),
            Some(font_size_setter),
            Attribute::all(),
        )
        .accessor(
            js_string!("fontWeight"),
            Some(font_weight_getter),
            Some(font_weight_setter),
            Attribute::all(),
        )
        .accessor(
            js_string!("fontFamily"),
            Some(font_family_getter),
            Some(font_family_setter),
            Attribute::all(),
        )
        .accessor(
            js_string!("textAlign"),
            Some(text_align_getter),
            Some(text_align_setter),
            Attribute::all(),
        )
        .accessor(
            js_string!("verticalAlign"),
            Some(vertical_align_getter),
            Some(vertical_align_setter),
            Attribute::all(),
        )
        .accessor(
            js_string!("margin"),
            Some(margin_getter),
            Some(margin_setter),
            Attribute::all(),
        )
        .accessor(
            js_string!("padding"),
            Some(padding_getter),
            Some(padding_setter),
            Attribute::all(),
        )
        .accessor(
            js_string!("borderWidth"),
            Some(border_width_getter),
            Some(border_width_setter),
            Attribute::all(),
        )
        .accessor(
            js_string!("borderColor"),
            Some(border_color_getter),
            Some(border_color_setter),
            Attribute::all(),
        )
        .accessor(
            js_string!("borderStyle"),
            Some(border_style_getter),
            Some(border_style_setter),
            Attribute::all(),
        )
        .accessor(
            js_string!("opacity"),
            Some(opacity_getter),
            Some(opacity_setter),
            Attribute::all(),
        )
        .accessor(
            js_string!("lineHeight"),
            Some(line_height_getter),
            Some(line_height_setter),
            Attribute::all(),
        )
        .accessor(
            js_string!("whiteSpace"),
            Some(white_space_getter),
            Some(white_space_setter),
            Attribute::all(),
        )
        .accessor(
            js_string!("cursor"),
            Some(cursor_getter),
            Some(cursor_setter),
            Attribute::all(),
        )
        .accessor(
            js_string!("overflow"),
            Some(overflow_getter),
            Some(overflow_setter),
            Attribute::all(),
        )
        .accessor(
            js_string!("position"),
            Some(position_getter),
            Some(position_setter),
            Attribute::all(),
        )
        .build();
    store_dom_style_object(context, node_id, &object);
    object
}

fn build_dom_node_list_object(
    context: &mut Context,
    node_ids: Vec<usize>,
) -> boa_engine::object::JsObject {
    let object = ObjectInitializer::with_native_data(
        DomNodeListHandle {
            node_ids: node_ids.clone(),
        },
        context,
    )
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
    .property(
        js_string!("length"),
        node_ids.len() as i32,
        Attribute::all(),
    )
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

fn build_storage_object(context: &mut Context, kind: StorageKind) -> boa_engine::object::JsObject {
    let length_getter =
        NativeFunction::from_fn_ptr(js_storage_get_length).to_js_function(context.realm());
    ObjectInitializer::with_native_data(StorageHandle { kind }, context)
        .function(
            NativeFunction::from_fn_ptr(js_storage_get_item),
            js_string!("getItem"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_storage_set_item),
            js_string!("setItem"),
            2,
        )
        .function(
            NativeFunction::from_fn_ptr(js_storage_remove_item),
            js_string!("removeItem"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_storage_clear),
            js_string!("clear"),
            0,
        )
        .function(
            NativeFunction::from_fn_ptr(js_storage_key),
            js_string!("key"),
            1,
        )
        .accessor(
            js_string!("length"),
            Some(length_getter),
            None,
            Attribute::all(),
        )
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
        .property(
            js_string!("statusText"),
            js_string!(status_text),
            Attribute::all(),
        )
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
    .function(
        NativeFunction::from_fn_ptr(js_xhr_open),
        js_string!("open"),
        3,
    )
    .function(
        NativeFunction::from_fn_ptr(js_xhr_set_request_header),
        js_string!("setRequestHeader"),
        2,
    )
    .function(
        NativeFunction::from_fn_ptr(js_xhr_send),
        js_string!("send"),
        1,
    )
    .function(
        NativeFunction::from_fn_ptr(js_xhr_abort),
        js_string!("abort"),
        0,
    )
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
    .accessor(
        js_string!("status"),
        Some(get_status),
        None,
        Attribute::all(),
    )
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
    .property(
        js_string!("onreadystatechange"),
        JsValue::undefined(),
        Attribute::all(),
    )
    .property(js_string!("onload"), JsValue::undefined(), Attribute::all())
    .property(
        js_string!("onerror"),
        JsValue::undefined(),
        Attribute::all(),
    )
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

fn js_location_get_hash(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let hash = current_location_url(context)
        .and_then(|url| {
            url.path
                .split_once('#')
                .map(|(_, fragment)| format!("#{fragment}"))
        })
        .unwrap_or_default();
    Ok(JsValue::from(js_string!(hash)))
}

fn js_location_set_hash(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let mut fragment = js_value_to_string(args.first().unwrap_or(&JsValue::undefined()), context)?;
    if fragment.starts_with('#') {
        fragment.remove(0);
    }

    let href = if fragment.is_empty() {
        current_location_url(context)
            .or_else(|| current_document_url(context))
            .map(|url| {
                let base = url.path.split('#').next().unwrap_or(&url.path).to_string();
                format!("{base}")
            })
            .unwrap_or_else(|| "/".to_string())
    } else {
        format!("#{fragment}")
    };
    set_soft_navigation_href(&href, context);
    Ok(JsValue::undefined())
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

fn js_history_push_state(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(target) = args.get(2) else {
        return Ok(JsValue::undefined());
    };
    if target.is_undefined() || target.is_null() {
        return Ok(JsValue::undefined());
    }
    let href = js_value_to_string(target, context)?;
    let resolved = resolve_same_origin_soft_navigation_href(&href, context)?;
    record_soft_navigation_href(&resolved, false, context);
    Ok(JsValue::undefined())
}

fn js_history_replace_state(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(target) = args.get(2) else {
        return Ok(JsValue::undefined());
    };
    if target.is_undefined() || target.is_null() {
        return Ok(JsValue::undefined());
    }
    let href = js_value_to_string(target, context)?;
    let resolved = resolve_same_origin_soft_navigation_href(&href, context)?;
    record_soft_navigation_href(&resolved, true, context);
    Ok(JsValue::undefined())
}

fn js_history_back(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    if let Some(target) = navigate_history(context, -1) {
        apply_soft_navigation_href_resolved(&target, context);
    }
    Ok(JsValue::undefined())
}

fn js_history_forward(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    if let Some(target) = navigate_history(context, 1) {
        apply_soft_navigation_href_resolved(&target, context);
    }
    Ok(JsValue::undefined())
}

fn js_history_length(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let length = context
        .get_data::<JavaScriptHostData>()
        .map(|host| host.state.borrow().history_entries.len())
        .unwrap_or(0);
    Ok(JsValue::new(length as i32))
}

fn record_soft_navigation_href(href: &str, replace_current: bool, context: &mut Context) {
    if let Some(host) = context.get_data::<JavaScriptHostData>() {
        let mut state = host.state.borrow_mut();
        if replace_current {
            if state.history_entries.is_empty() {
                state.history_entries.push(href.to_string());
                state.history_index = 0;
            } else {
                let index = state.history_index;
                if let Some(entry) = state.history_entries.get_mut(index) {
                    *entry = href.to_string();
                }
            }
        } else {
            let next_index = state.history_index.saturating_add(1);
            state.history_entries.truncate(next_index);
            state.history_entries.push(href.to_string());
            state.history_index = state.history_entries.len().saturating_sub(1);
        }
    }
    apply_soft_navigation_href_resolved(href, context);
}

fn navigate_history(context: &mut Context, delta: isize) -> Option<String> {
    let host = context.get_data::<JavaScriptHostData>()?;
    let mut state = host.state.borrow_mut();
    let next = state.history_index as isize + delta;
    if next < 0 || next as usize >= state.history_entries.len() {
        return None;
    }
    state.history_index = next as usize;
    state.history_entries.get(state.history_index).cloned()
}

fn set_location_href(href: &str, context: &mut Context) {
    let resolved = current_document_url(context)
        .and_then(|url| url.resolve(href).ok())
        .map(|url| url.to_string())
        .or_else(|| Url::parse(href).ok().map(|url| url.to_string()))
        .unwrap_or_else(|| href.to_string());
    if let Some(host) = context.get_data::<JavaScriptHostData>() {
        let mut state = host.state.borrow_mut();
        state.location_href = resolved;
        state.soft_navigation_target = None;
    }
}

fn set_soft_navigation_href(href: &str, context: &mut Context) {
    let resolved = current_document_url(context)
        .and_then(|url| url.resolve(href).ok())
        .map(|url| url.to_string())
        .or_else(|| Url::parse(href).ok().map(|url| url.to_string()))
        .unwrap_or_else(|| href.to_string());
    record_soft_navigation_href(&resolved, false, context);
}

fn apply_soft_navigation_href_resolved(resolved: &str, context: &mut Context) {
    if let Some(host) = context.get_data::<JavaScriptHostData>() {
        let mut state = host.state.borrow_mut();
        state.location_href = resolved.to_string();
        if let Ok(url) = Url::parse(&resolved) {
            state.document_url = url;
        }
        state.soft_navigation_target = Some(resolved.to_string());
    }
}

fn resolve_same_origin_soft_navigation_href(href: &str, context: &mut Context) -> JsResult<String> {
    let base = current_document_url(context).ok_or_else(|| {
        boa_engine::JsError::from(
            JsNativeError::error().with_message("missing document URL for history navigation"),
        )
    })?;
    if href.trim().is_empty() {
        return Ok(base.to_string());
    }
    let resolved = base
        .resolve(href)
        .ok()
        .or_else(|| Url::parse(href).ok())
        .ok_or_else(|| {
            boa_engine::JsError::from(
                JsNativeError::error()
                    .with_message(format!("invalid history navigation target: {href}")),
            )
        })?;
    if !base.shares_origin(&resolved) {
        return Err(boa_engine::JsError::from(
            JsNativeError::error().with_message("history API target must stay same-origin"),
        ));
    }
    Ok(resolved.to_string())
}

fn current_location_url(context: &mut Context) -> Option<Url> {
    let href = context
        .get_data::<JavaScriptHostData>()
        .map(|host| host.state.borrow().location_href.clone())?;
    Url::parse(&href).ok()
}

fn current_document_url(context: &mut Context) -> Option<Url> {
    context
        .get_data::<JavaScriptHostData>()
        .map(|host| host.state.borrow().document_url.clone())
}

fn resolve_requested_url(request: &JsValue, context: &mut Context) -> JsResult<Url> {
    let request_url = if let Some(object) = request.as_object() {
        let value = object.get(js_string!("url"), context)?;
        if !value.is_undefined() && !value.is_null() {
            js_value_to_string(&value, context)?
        } else {
            js_value_to_string(request, context)?
        }
    } else {
        js_value_to_string(request, context)?
    };

    if let Ok(url) = Url::parse(&request_url) {
        return Ok(url);
    }

    if let Some(base) = current_document_url(context)
        && let Ok(url) = base.resolve(&request_url)
    {
        return Ok(url);
    }

    Err(JsNativeError::error()
        .with_message(format!("unsupported fetch url: {request_url}"))
        .into())
}

fn reserve_js_network_budget(context: &mut Context) -> JsResult<usize> {
    let Some(host) = context.get_data::<JavaScriptHostData>() else {
        return Err(JsNativeError::error()
            .with_message("missing JS runtime host state")
            .into());
    };

    let mut state = host.state.borrow_mut();
    if state.network_request_count >= JS_MAX_NETWORK_REQUESTS {
        return Err(JsNativeError::error()
            .with_message(format!(
                "JS network request limit reached ({JS_MAX_NETWORK_REQUESTS})"
            ))
            .into());
    }

    let remaining =
        JS_MAX_NETWORK_TOTAL_RESPONSE_BYTES.saturating_sub(state.network_response_bytes);
    if remaining == 0 {
        return Err(JsNativeError::error()
            .with_message(format!(
                "JS network response budget exhausted ({JS_MAX_NETWORK_TOTAL_RESPONSE_BYTES} bytes)"
            ))
            .into());
    }

    state.network_request_count += 1;
    Ok(remaining.min(JS_MAX_NETWORK_RESPONSE_BYTES))
}

fn record_js_network_response_bytes(response_len: usize, context: &mut Context) {
    if let Some(host) = context.get_data::<JavaScriptHostData>() {
        let mut state = host.state.borrow_mut();
        state.network_response_bytes = state.network_response_bytes.saturating_add(response_len);
        debug_assert!(state.network_response_bytes <= JS_MAX_NETWORK_TOTAL_RESPONSE_BYTES);
    }
}

fn xhr_body_is_supported(body: Option<&JsValue>, context: &mut Context) -> JsResult<bool> {
    let Some(body) = body else {
        return Ok(true);
    };
    if body.is_undefined() || body.is_null() {
        return Ok(true);
    }

    if body.is_string() {
        return Ok(js_value_to_string(body, context)?.is_empty());
    }

    Ok(false)
}

fn ensure_same_origin_script_url(current: &Url, target: &Url, reason: &str) -> JsResult<()> {
    if current.shares_origin(target) {
        return Ok(());
    }

    Err(JsNativeError::error()
        .with_message(format!("{reason}: {} -> {}", current, target))
        .into())
}

fn fetch_for_script(url: &Url, context: &mut Context) -> JsResult<HttpResponse> {
    let current = current_document_url(context).ok_or_else(|| {
        JsNativeError::error().with_message("missing current page origin for JS request")
    })?;
    ensure_same_origin_script_url(&current, url, "cross-origin JS requests are blocked")?;

    let max_response_bytes = reserve_js_network_budget(context)?;
    let response = fetch_with_limits_same_origin(url, max_response_bytes, &current)
        .map_err(|error| JsNativeError::error().with_message(error.to_string()))?;
    record_js_network_response_bytes(response.body.len(), context);
    Ok(response)
}

fn js_fetch(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let request = args.first().cloned().unwrap_or_else(JsValue::undefined);
    let url = resolve_requested_url(&request, context);
    let promise = match url {
        Ok(url) => JsPromise::from_result(
            fetch_for_script(&url, context)
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
            .and_then(|json| {
                JsValue::from_json(&json, context)
                    .map_err(|error| JsNativeError::error().with_message(error.to_string()))
            });
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

    Err(JsNativeError::typ()
        .with_message("Response.clone called on non-response object")
        .into())
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
        handle
            .state
            .borrow_mut()
            .request_headers
            .insert(name, value);
    }
    Ok(JsValue::undefined())
}

fn js_xhr_send(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
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

    let result = if !xhr_body_is_supported(args.first(), context)? {
        Err(JsNativeError::error()
            .with_message("XMLHttpRequest send(body) is not supported yet")
            .into())
    } else if method.is_empty() || method == "GET" {
        resolve_requested_url(&JsValue::from(js_string!(target_url)), context)
            .and_then(|url| fetch_for_script(&url, context))
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

fn js_xhr_get_response_header(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
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
        xhr_state_value(this, |state| state.status_text.clone()).unwrap_or_default()
    )))
}

fn js_xhr_get_response_text(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::from(js_string!(
        xhr_state_value(this, |state| state.response_text.clone()).unwrap_or_default()
    )))
}

fn js_xhr_get_response(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::from(js_string!(
        xhr_state_value(this, |state| state.response_text.clone()).unwrap_or_default()
    )))
}

fn js_xhr_get_response_url(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::from(js_string!(
        xhr_state_value(this, |state| state.response_url.clone()).unwrap_or_default()
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
    call_js_callback_with_this(&callback, &JsValue::from(object.clone()), &[], context)?;
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
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let event_name = args
        .first()
        .map(|value| js_value_to_string(value, context))
        .transpose()?
        .unwrap_or_default()
        .to_ascii_lowercase();
    let Some(callback) = args.get(1).cloned() else {
        return Ok(JsValue::undefined());
    };
    let Some(target) = this.as_object() else {
        return Ok(JsValue::undefined());
    };
    if callback.as_object().is_none() {
        return Ok(JsValue::undefined());
    }
    let options = event_listener_options_from_value(args.get(2), context)?;
    append_event_listener(&target, &event_name, callback, options, context)?;

    Ok(JsValue::undefined())
}

fn js_remove_event_listener(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let event_name = args
        .first()
        .map(|value| js_value_to_string(value, context))
        .transpose()?
        .unwrap_or_default()
        .to_ascii_lowercase();
    let Some(callback) = args.get(1) else {
        return Ok(JsValue::undefined());
    };
    let Some(target) = this.as_object() else {
        return Ok(JsValue::undefined());
    };
    let options = event_listener_options_from_value(args.get(2), context)?;
    remove_event_listener(&target, &event_name, callback, options.capture, context)?;
    Ok(JsValue::undefined())
}

fn js_dom_dispatch_event(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(target) = this.as_object() else {
        return Ok(JsValue::new(true));
    };
    let event_arg = args.first().cloned().unwrap_or_else(JsValue::undefined);
    let (event_type, bubbles, cancelable) = if let Some(object) = event_arg.as_object() {
        let event_type = js_value_to_string(&object.get(js_string!("type"), context)?, context)?;
        let bubbles = object.get(js_string!("bubbles"), context)?.to_boolean();
        let cancelable = object.get(js_string!("cancelable"), context)?.to_boolean();
        (event_type, bubbles, cancelable)
    } else {
        let event_type = js_value_to_string(&event_arg, context)?;
        let bubbles = default_event_bubbles(&event_type);
        let cancelable = default_event_cancelable(&event_type);
        (event_type, bubbles, cancelable)
    };
    let request = DomEventRequest {
        target_node_id: target
            .downcast_ref::<DomNodeHandle>()
            .map(|handle| handle.node_id)
            .unwrap_or(0),
        event_type,
        bubbles,
        cancelable,
        ..Default::default()
    };
    let prevented = dispatch_dom_event_on_target(target.clone(), &request, context)?;
    Ok(JsValue::new(!prevented))
}

fn js_dom_focus(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(target) = this.as_object() else {
        return Ok(JsValue::undefined());
    };
    if let Some(node_id) = target
        .downcast_ref::<DomNodeHandle>()
        .map(|handle| handle.node_id)
        && context
            .get_data::<JavaScriptHostData>()
            .map(|host| host.state.borrow().dom.is_disabled(node_id))
            .unwrap_or(false)
    {
        return Ok(JsValue::undefined());
    }
    let request = DomEventRequest {
        target_node_id: target
            .downcast_ref::<DomNodeHandle>()
            .map(|handle| handle.node_id)
            .unwrap_or(0),
        event_type: "focus".to_string(),
        ..Default::default()
    };
    let _ = dispatch_dom_event_on_target(target.clone(), &request, context)?;
    Ok(JsValue::undefined())
}

fn js_dom_blur(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(target) = this.as_object() else {
        return Ok(JsValue::undefined());
    };
    let request = DomEventRequest {
        target_node_id: target
            .downcast_ref::<DomNodeHandle>()
            .map(|handle| handle.node_id)
            .unwrap_or(0),
        event_type: "blur".to_string(),
        ..Default::default()
    };
    let _ = dispatch_dom_event_on_target(target.clone(), &request, context)?;
    Ok(JsValue::undefined())
}

fn js_event_prevent_default(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    if let Some(object) = this.as_object()
        && event_flag_value(&object, "__tobiraPassiveListener", context)
    {
        return Ok(JsValue::undefined());
    }
    if let Some(object) = this.as_object()
        && !event_flag_value(&object, "cancelable", context)
    {
        return Ok(JsValue::undefined());
    }
    set_event_bool_property(this, "defaultPrevented", true, context)?;
    Ok(JsValue::undefined())
}

fn js_event_stop_propagation(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    set_event_bool_property(this, "propagationStopped", true, context)?;
    set_event_bool_property(this, "cancelBubble", true, context)?;
    Ok(JsValue::undefined())
}

fn js_event_stop_immediate_propagation(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    set_event_bool_property(this, "propagationStopped", true, context)?;
    set_event_bool_property(this, "immediatePropagationStopped", true, context)?;
    set_event_bool_property(this, "cancelBubble", true, context)?;
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
    arg.and_then(JsValue::as_object).and_then(|object| {
        object
            .downcast_ref::<DomNodeHandle>()
            .map(|handle| handle.node_id)
    })
}

fn js_dom_query_selector(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let selector = js_value_to_string(args.first().unwrap_or(&JsValue::undefined()), context)?;
    let Some(scope_id) = this_node_id(this) else {
        return Ok(JsValue::null());
    };
    let found = context.get_data::<JavaScriptHostData>().and_then(|host| {
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
        return Ok(JsValue::from(build_dom_node_list_object(
            context,
            Vec::new(),
        )));
    };
    let node_ids = context
        .get_data::<JavaScriptHostData>()
        .map(|host| {
            let state = host.state.borrow();
            let include_scope = scope_id == state.dom.document_id;
            state
                .dom
                .query_selector_all(scope_id, &selector, include_scope)
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
    let found = context.get_data::<JavaScriptHostData>().and_then(|host| {
        host.state
            .borrow()
            .dom
            .get_element_by_id(scope_id, &target_id)
    });
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

fn js_dom_append_child(
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
    if let Some(host) = context.get_data::<JavaScriptHostData>() {
        host.state
            .borrow_mut()
            .dom
            .append_child(parent_id, child_id);
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

fn js_dom_get_attribute(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
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

fn js_dom_set_attribute(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let name = js_value_to_string(args.first().unwrap_or(&JsValue::undefined()), context)?;
    let value = js_value_to_string(args.get(1).unwrap_or(&JsValue::undefined()), context)?;
    if let Some(node_id) = this_node_id(this)
        && let Some(host) = context.get_data::<JavaScriptHostData>()
    {
        host.state
            .borrow_mut()
            .dom
            .set_attribute(node_id, &name, &value);
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
        host.state
            .borrow_mut()
            .dom
            .set_attribute(node_id, name, &value);
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

fn js_dom_get_class_list(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
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

fn js_dom_get_child_nodes(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this).unwrap_or(0);
    let node_ids = context
        .get_data::<JavaScriptHostData>()
        .and_then(|host| {
            host.state
                .borrow()
                .dom
                .node(node_id)
                .map(|node| node.children.clone())
        })
        .unwrap_or_default();
    Ok(JsValue::from(build_dom_node_list_object(context, node_ids)))
}

fn js_dom_get_text_content(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
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

fn js_dom_get_inner_html(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
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
    js_dom_get_attribute(this, &[JsValue::from(js_string!("id"))], context)
}

fn js_dom_set_id(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let value = args.first().cloned().unwrap_or_else(JsValue::undefined);
    js_dom_set_attribute(this, &[JsValue::from(js_string!("id")), value], context)
}

fn js_dom_get_class_name(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    js_dom_get_attribute(this, &[JsValue::from(js_string!("class"))], context)
}

fn js_dom_set_class_name(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let value = args.first().cloned().unwrap_or_else(JsValue::undefined);
    js_dom_set_attribute(this, &[JsValue::from(js_string!("class")), value], context)
}

fn js_dom_get_value(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    // The DOM attribute stays in step with GUI edits, so `value` reflects the
    // current live control state for both script-driven and native input paths.
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
        .and_then(|host| {
            host.state
                .borrow()
                .dom
                .element(node_id)
                .map(|element| element.tag_name.to_ascii_uppercase())
        })
        .unwrap_or_default();
    Ok(JsValue::from(js_string!(tag_name)))
}

fn js_dom_get_parent_node(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this).unwrap_or(0);
    let parent_id = context.get_data::<JavaScriptHostData>().and_then(|host| {
        host.state
            .borrow()
            .dom
            .node(node_id)
            .and_then(|node| node.parent)
    });
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
    let parent_id = context.get_data::<JavaScriptHostData>().and_then(|host| {
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

fn style_node_id_from_this(this: &JsValue) -> Option<usize> {
    let object = this.as_object()?;
    let handle = object.downcast_ref::<DomStyleHandle>()?;
    Some(handle.node_id)
}

fn normalize_css_property_name(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let mut normalized = String::with_capacity(trimmed.len());
    for character in trimmed.chars() {
        if character.is_ascii_uppercase() {
            if !normalized.is_empty() && !normalized.ends_with('-') {
                normalized.push('-');
            }
            normalized.push(character.to_ascii_lowercase());
        } else {
            normalized.push(character);
        }
    }
    normalized.to_ascii_lowercase()
}

fn parse_inline_style_entries(input: &str) -> Vec<(String, String)> {
    input
        .split(';')
        .filter_map(|entry| {
            let (property, value) = entry.split_once(':')?;
            let property = normalize_css_property_name(property);
            let value = value.trim().to_string();
            if property.is_empty() || value.is_empty() {
                return None;
            }
            Some((property, value))
        })
        .collect()
}

fn serialize_inline_style_entries(entries: &[(String, String)]) -> String {
    entries
        .iter()
        .map(|(property, value)| format!("{property}: {value}"))
        .collect::<Vec<_>>()
        .join("; ")
}

fn inline_style_text(context: &mut Context, node_id: usize) -> String {
    context
        .get_data::<JavaScriptHostData>()
        .and_then(|host| host.state.borrow().dom.get_attribute(node_id, "style"))
        .unwrap_or_default()
}

fn set_inline_style_text(context: &mut Context, node_id: usize, text: &str) {
    if let Some(host) = context.get_data::<JavaScriptHostData>() {
        let mut state = host.state.borrow_mut();
        let value = text.trim();
        if value.is_empty() {
            state.dom.remove_attribute(node_id, "style");
        } else {
            state.dom.set_attribute(node_id, "style", value);
        }
    }
}

fn inline_style_property_value(
    context: &mut Context,
    node_id: usize,
    property_name: &str,
) -> String {
    let target = normalize_css_property_name(property_name);
    let Some(value) = context
        .get_data::<JavaScriptHostData>()
        .and_then(|host| host.state.borrow().dom.get_attribute(node_id, "style"))
    else {
        return String::new();
    };

    parse_inline_style_entries(&value)
        .into_iter()
        .rev()
        .find(|(property, _)| *property == target)
        .map(|(_, value)| value)
        .unwrap_or_default()
}

fn set_inline_style_property(
    context: &mut Context,
    node_id: usize,
    property_name: &str,
    value: &str,
) {
    let target = normalize_css_property_name(property_name);
    if target.is_empty() {
        return;
    }

    if let Some(host) = context.get_data::<JavaScriptHostData>() {
        let mut state = host.state.borrow_mut();
        let current = state
            .dom
            .get_attribute(node_id, "style")
            .unwrap_or_default();
        let mut entries = parse_inline_style_entries(&current);
        entries.retain(|(property, _)| *property != target);

        let value = value.trim();
        if !value.is_empty() {
            entries.push((target, value.to_string()));
        }

        if entries.is_empty() {
            state.dom.remove_attribute(node_id, "style");
        } else {
            state
                .dom
                .set_attribute(node_id, "style", &serialize_inline_style_entries(&entries));
        }
    }
}

fn remove_inline_style_property(
    context: &mut Context,
    node_id: usize,
    property_name: &str,
) -> String {
    let target = normalize_css_property_name(property_name);
    if target.is_empty() {
        return String::new();
    }

    let Some(host) = context.get_data::<JavaScriptHostData>() else {
        return String::new();
    };
    let mut state = host.state.borrow_mut();
    let current = state
        .dom
        .get_attribute(node_id, "style")
        .unwrap_or_default();
    let mut entries = parse_inline_style_entries(&current);
    let mut removed = String::new();
    entries.retain(|(property, value)| {
        if *property == target {
            removed = value.clone();
            false
        } else {
            true
        }
    });

    if entries.is_empty() {
        state.dom.remove_attribute(node_id, "style");
    } else {
        state
            .dom
            .set_attribute(node_id, "style", &serialize_inline_style_entries(&entries));
    }

    removed
}

macro_rules! define_style_accessors {
    ($(($getter:ident, $setter:ident, $css_name:literal)),+ $(,)?) => {
        $(
            fn $getter(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
                let value = style_node_id_from_this(this)
                    .map(|node_id| inline_style_property_value(context, node_id, $css_name))
                    .unwrap_or_default();
                Ok(JsValue::from(js_string!(value)))
            }

            fn $setter(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
                let value = args
                    .first()
                    .map(|value| js_value_to_string(value, context))
                    .transpose()?
                    .unwrap_or_default();
                if let Some(node_id) = style_node_id_from_this(this) {
                    set_inline_style_property(context, node_id, $css_name, &value);
                }
                Ok(JsValue::undefined())
            }
        )+
    };
}

define_style_accessors!(
    (
        js_dom_style_get_display,
        js_dom_style_set_display,
        "display"
    ),
    (js_dom_style_get_color, js_dom_style_set_color, "color"),
    (
        js_dom_style_get_font_style,
        js_dom_style_set_font_style,
        "font-style"
    ),
    (
        js_dom_style_get_text_decoration,
        js_dom_style_set_text_decoration,
        "text-decoration"
    ),
    (
        js_dom_style_get_text_transform,
        js_dom_style_set_text_transform,
        "text-transform"
    ),
    (
        js_dom_style_get_text_indent,
        js_dom_style_set_text_indent,
        "text-indent"
    ),
    (
        js_dom_style_get_letter_spacing,
        js_dom_style_set_letter_spacing,
        "letter-spacing"
    ),
    (
        js_dom_style_get_background_color,
        js_dom_style_set_background_color,
        "background-color"
    ),
    (js_dom_style_get_width, js_dom_style_set_width, "width"),
    (js_dom_style_get_height, js_dom_style_set_height, "height"),
    (
        js_dom_style_get_max_width,
        js_dom_style_set_max_width,
        "max-width"
    ),
    (
        js_dom_style_get_min_width,
        js_dom_style_set_min_width,
        "min-width"
    ),
    (
        js_dom_style_get_max_height,
        js_dom_style_set_max_height,
        "max-height"
    ),
    (
        js_dom_style_get_min_height,
        js_dom_style_set_min_height,
        "min-height"
    ),
    (
        js_dom_style_get_font_size,
        js_dom_style_set_font_size,
        "font-size"
    ),
    (
        js_dom_style_get_font_weight,
        js_dom_style_set_font_weight,
        "font-weight"
    ),
    (
        js_dom_style_get_font_family,
        js_dom_style_set_font_family,
        "font-family"
    ),
    (
        js_dom_style_get_text_align,
        js_dom_style_set_text_align,
        "text-align"
    ),
    (
        js_dom_style_get_vertical_align,
        js_dom_style_set_vertical_align,
        "vertical-align"
    ),
    (js_dom_style_get_margin, js_dom_style_set_margin, "margin"),
    (
        js_dom_style_get_padding,
        js_dom_style_set_padding,
        "padding"
    ),
    (
        js_dom_style_get_border_width,
        js_dom_style_set_border_width,
        "border-width"
    ),
    (
        js_dom_style_get_border_color,
        js_dom_style_set_border_color,
        "border-color"
    ),
    (
        js_dom_style_get_border_style,
        js_dom_style_set_border_style,
        "border-style"
    ),
    (
        js_dom_style_get_opacity,
        js_dom_style_set_opacity,
        "opacity"
    ),
    (
        js_dom_style_get_line_height,
        js_dom_style_set_line_height,
        "line-height"
    ),
    (
        js_dom_style_get_white_space,
        js_dom_style_set_white_space,
        "white-space"
    ),
    (js_dom_style_get_cursor, js_dom_style_set_cursor, "cursor"),
    (
        js_dom_style_get_overflow,
        js_dom_style_set_overflow,
        "overflow"
    ),
    (
        js_dom_style_get_position,
        js_dom_style_set_position,
        "position"
    ),
);

fn js_dom_style_get_css_text(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let text = style_node_id_from_this(this)
        .map(|node_id| inline_style_text(context, node_id))
        .unwrap_or_default();
    Ok(JsValue::from(js_string!(text)))
}

fn js_dom_style_set_css_text(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let text = args
        .first()
        .map(|value| js_value_to_string(value, context))
        .transpose()?
        .unwrap_or_default();
    if let Some(node_id) = style_node_id_from_this(this) {
        set_inline_style_text(context, node_id, &text);
    }
    Ok(JsValue::undefined())
}

fn js_dom_style_get_property_value(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let name = args
        .first()
        .map(|value| js_value_to_string(value, context))
        .transpose()?
        .unwrap_or_default();
    let value = style_node_id_from_this(this)
        .map(|node_id| inline_style_property_value(context, node_id, &name))
        .unwrap_or_default();
    Ok(JsValue::from(js_string!(value)))
}

fn js_dom_style_set_property(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let name = args
        .first()
        .map(|value| js_value_to_string(value, context))
        .transpose()?
        .unwrap_or_default();
    let value = args
        .get(1)
        .map(|value| js_value_to_string(value, context))
        .transpose()?
        .unwrap_or_default();
    if let Some(node_id) = style_node_id_from_this(this) {
        set_inline_style_property(context, node_id, &name, &value);
    }
    Ok(JsValue::undefined())
}

fn js_dom_style_remove_property(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let name = args
        .first()
        .map(|value| js_value_to_string(value, context))
        .transpose()?
        .unwrap_or_default();
    let removed = style_node_id_from_this(this)
        .map(|node_id| remove_inline_style_property(context, node_id, &name))
        .unwrap_or_default();
    Ok(JsValue::from(js_string!(removed)))
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
        host.state
            .borrow_mut()
            .dom
            .add_class(handle.node_id, &class_name);
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
        host.state
            .borrow_mut()
            .dom
            .remove_class(handle.node_id, &class_name);
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
        .and_then(|object| {
            object
                .downcast_ref::<DomClassListHandle>()
                .map(|handle| handle.node_id)
        })
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
        let list_value =
            JsValue::from(build_dom_node_list_object(context, handle.node_ids.clone()));
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

fn js_document_get_cookie(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let cookie = current_document_url(context)
        .map(|url| site_state::document_cookie_get(&url))
        .unwrap_or_default();
    Ok(JsValue::from(js_string!(cookie)))
}

fn js_document_set_cookie(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(value) = args.first() else {
        return Ok(JsValue::undefined());
    };
    let cookie_line = js_value_to_string(value, context)?;
    if let Some(url) = current_document_url(context) {
        site_state::document_cookie_set(&url, &cookie_line);
    }
    Ok(JsValue::undefined())
}

fn js_noop(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::undefined())
}

fn storage_kind_from_this(this: &JsValue) -> Option<StorageKind> {
    let object = this.as_object()?;
    let handle = object.downcast_ref::<StorageHandle>()?;
    Some(handle.kind)
}

fn js_storage_get_length(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(kind) = storage_kind_from_this(this) else {
        return Ok(JsValue::new(0));
    };
    let Some(url) = current_document_url(context) else {
        return Ok(JsValue::new(0));
    };
    Ok(JsValue::new(site_state::storage_length(kind, &url) as i32))
}

fn js_storage_get_item(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(kind) = storage_kind_from_this(this) else {
        return Ok(JsValue::null());
    };
    let Some(url) = current_document_url(context) else {
        return Ok(JsValue::null());
    };
    let key = args
        .first()
        .map(|value| js_value_to_string(value, context))
        .transpose()?
        .unwrap_or_default();
    Ok(site_state::storage_get_item(kind, &url, &key)
        .map(|value| JsValue::from(js_string!(value)))
        .unwrap_or_else(JsValue::null))
}

fn js_storage_set_item(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(kind) = storage_kind_from_this(this) else {
        return Ok(JsValue::undefined());
    };
    let Some(url) = current_document_url(context) else {
        return Ok(JsValue::undefined());
    };
    let key = args
        .first()
        .map(|value| js_value_to_string(value, context))
        .transpose()?
        .unwrap_or_default();
    let value = args
        .get(1)
        .map(|value| js_value_to_string(value, context))
        .transpose()?
        .unwrap_or_default();
    site_state::storage_set_item(kind, &url, key, value);
    Ok(JsValue::undefined())
}

fn js_storage_remove_item(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(kind) = storage_kind_from_this(this) else {
        return Ok(JsValue::undefined());
    };
    let Some(url) = current_document_url(context) else {
        return Ok(JsValue::undefined());
    };
    let key = args
        .first()
        .map(|value| js_value_to_string(value, context))
        .transpose()?
        .unwrap_or_default();
    site_state::storage_remove_item(kind, &url, &key);
    Ok(JsValue::undefined())
}

fn js_storage_clear(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(kind) = storage_kind_from_this(this) else {
        return Ok(JsValue::undefined());
    };
    let Some(url) = current_document_url(context) else {
        return Ok(JsValue::undefined());
    };
    site_state::storage_clear(kind, &url);
    Ok(JsValue::undefined())
}

fn js_storage_key(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(kind) = storage_kind_from_this(this) else {
        return Ok(JsValue::null());
    };
    let Some(url) = current_document_url(context) else {
        return Ok(JsValue::null());
    };
    let index = args
        .first()
        .and_then(JsValue::as_number)
        .map(|value| value as usize)
        .unwrap_or(0);
    Ok(site_state::storage_key(kind, &url, index)
        .map(|value| JsValue::from(js_string!(value)))
        .unwrap_or_else(JsValue::null))
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

fn call_js_callback_with_this(
    callback: &JsValue,
    this_value: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    if let Some(object) = callback.as_object()
        && let Some(function) = JsFunction::from_object(object.clone())
    {
        return function.call(this_value, args, context);
    }

    Ok(JsValue::undefined())
}

fn call_js_callback(
    callback: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    call_js_callback_with_this(callback, &JsValue::undefined(), args, context)
}

fn default_event_bubbles(event_type: &str) -> bool {
    matches!(
        event_type,
        "click" | "input" | "change" | "submit" | "keydown" | "keyup"
    )
}

fn default_event_cancelable(event_type: &str) -> bool {
    matches!(event_type, "click" | "submit" | "keydown" | "keyup")
}

fn event_flag_value(this: &JsObject, name: &str, context: &mut Context) -> bool {
    this.get(js_string!(name), context)
        .ok()
        .map(|value| value.to_boolean())
        .unwrap_or(false)
}

fn set_event_current_target(
    event: &boa_engine::object::JsObject,
    target: &boa_engine::object::JsObject,
    context: &mut Context,
) -> JsResult<()> {
    event.set(js_string!("currentTarget"), target.clone(), true, context)?;
    Ok(())
}

fn set_event_phase(
    event: &boa_engine::object::JsObject,
    phase: EventDispatchPhase,
    context: &mut Context,
) -> JsResult<()> {
    event.set(
        js_string!("eventPhase"),
        JsValue::new(phase as i32),
        true,
        context,
    )?;
    Ok(())
}

fn dispatch_listeners_on_target(
    target: &boa_engine::object::JsObject,
    event_type: &str,
    event: &boa_engine::object::JsObject,
    capture_phase: bool,
    phase: EventDispatchPhase,
    context: &mut Context,
) -> JsResult<()> {
    set_event_phase(event, phase, context)?;
    let entries = event_listener_entries(target, event_type, context)?;
    let target_value = JsValue::from(target.clone());
    let event_value = JsValue::from(event.clone());
    for entry in entries {
        if entry.options.capture != capture_phase {
            continue;
        }
        let _ = set_event_internal_bool_property(
            event,
            "__tobiraPassiveListener",
            entry.options.passive,
            context,
        );
        let callback_result = call_js_callback_with_this(
            &entry.callback,
            &target_value,
            &[event_value.clone()],
            context,
        );
        let _ = set_event_internal_bool_property(event, "__tobiraPassiveListener", false, context);
        callback_result?;
        if entry.options.once {
            let _ = remove_event_listener(
                target,
                event_type,
                &entry.callback,
                entry.options.capture,
                context,
            );
        }
        if event_flag_value(event, "immediatePropagationStopped", context) {
            break;
        }
    }
    Ok(())
}

fn dom_event_path(node_id: usize, context: &mut Context) -> Vec<usize> {
    let mut path = vec![node_id];
    let mut current = node_id;

    loop {
        let Some(parent_id) = context.get_data::<JavaScriptHostData>().and_then(|host| {
            host.state
                .borrow()
                .dom
                .node(current)
                .and_then(|node| node.parent)
        }) else {
            break;
        };
        path.push(parent_id);
        current = parent_id;
    }

    path.reverse();
    path
}

fn dispatch_dom_event_on_target(
    target: boa_engine::object::JsObject,
    request: &DomEventRequest,
    context: &mut Context,
) -> JsResult<bool> {
    if let Some(host) = context.get_data::<JavaScriptHostData>() {
        let mut state = host.state.borrow_mut();
        if request.event_type.eq_ignore_ascii_case("focus") {
            state.active_element_node_id = Some(request.target_node_id);
        } else if request.event_type.eq_ignore_ascii_case("blur")
            && state.active_element_node_id == Some(request.target_node_id)
        {
            state.active_element_node_id = None;
        }
    }
    let event = build_dom_event_object(context, request, &target);
    let listener_event_type = request.event_type.to_ascii_lowercase();
    if let Some(target_node_id) = target
        .downcast_ref::<DomNodeHandle>()
        .map(|handle| handle.node_id)
    {
        let path = dom_event_path(target_node_id, context);
        let ancestors = path
            .len()
            .checked_sub(1)
            .map(|end| &path[..end])
            .unwrap_or(&[]);

        for ancestor_id in ancestors.iter().copied() {
            let current_target = build_dom_node_object(context, ancestor_id);
            set_event_current_target(&event, &current_target, context)?;
            dispatch_listeners_on_target(
                &current_target,
                &listener_event_type,
                &event,
                true,
                EventDispatchPhase::Capturing,
                context,
            )?;
            if event_flag_value(&event, "immediatePropagationStopped", context)
                || event_flag_value(&event, "propagationStopped", context)
            {
                return Ok(event_flag_value(&event, "defaultPrevented", context));
            }
        }

        let current_target = target.clone();
        set_event_current_target(&event, &current_target, context)?;
        dispatch_listeners_on_target(
            &current_target,
            &listener_event_type,
            &event,
            true,
            EventDispatchPhase::AtTarget,
            context,
        )?;
        if event_flag_value(&event, "immediatePropagationStopped", context) {
            return Ok(event_flag_value(&event, "defaultPrevented", context));
        }
        dispatch_listeners_on_target(
            &current_target,
            &listener_event_type,
            &event,
            false,
            EventDispatchPhase::AtTarget,
            context,
        )?;
        if event_flag_value(&event, "immediatePropagationStopped", context)
            || event_flag_value(&event, "propagationStopped", context)
        {
            return Ok(event_flag_value(&event, "defaultPrevented", context));
        }

        if request.bubbles {
            for ancestor_id in ancestors.iter().rev().copied() {
                let current_target = build_dom_node_object(context, ancestor_id);
                set_event_current_target(&event, &current_target, context)?;
                dispatch_listeners_on_target(
                    &current_target,
                    &listener_event_type,
                    &event,
                    false,
                    EventDispatchPhase::Bubbling,
                    context,
                )?;
                if event_flag_value(&event, "immediatePropagationStopped", context)
                    || event_flag_value(&event, "propagationStopped", context)
                {
                    break;
                }
            }
        }
    } else {
        let current_target = target.clone();
        set_event_current_target(&event, &current_target, context)?;
        dispatch_listeners_on_target(
            &current_target,
            &listener_event_type,
            &event,
            true,
            EventDispatchPhase::AtTarget,
            context,
        )?;
        if !event_flag_value(&event, "immediatePropagationStopped", context) {
            dispatch_listeners_on_target(
                &current_target,
                &listener_event_type,
                &event,
                false,
                EventDispatchPhase::AtTarget,
                context,
            )?;
        }
    }

    Ok(event_flag_value(&event, "defaultPrevented", context))
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
    use boa_engine::{Context, JsValue, Source, js_string};

    use super::{
        DomEventRequest, JavaScriptRuntime, current_location_url, ensure_same_origin_script_url,
        fetch_for_script, process_document_scripts, resolve_requested_url, set_location_href,
        start_document_script_session,
    };
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
    fn updates_location_hash_without_full_navigation() {
        let processed = process_document_scripts(
            "<script>location.hash = '#frag'; document.title = location.href + '|' + location.hash;</script>",
            &Url::parse("https://example.com/start").unwrap(),
        );

        assert_eq!(
            processed.soft_navigation_target.as_deref(),
            Some("https://example.com/start#frag")
        );
        assert!(processed.navigation_target.is_none());
        assert_eq!(
            processed.title_override.as_deref(),
            Some("https://example.com/start#frag|#frag")
        );
    }

    #[test]
    fn push_state_updates_location_without_reload() {
        let processed = process_document_scripts(
            "<script>history.pushState({ page: 1 }, '', '/next?from=test#frag'); document.title = location.href + '|' + location.hash;</script>",
            &Url::parse("https://example.com/start").unwrap(),
        );

        assert_eq!(
            processed.soft_navigation_target.as_deref(),
            Some("https://example.com/next?from=test#frag")
        );
        assert!(processed.navigation_target.is_none());
        assert_eq!(
            processed.title_override.as_deref(),
            Some("https://example.com/next?from=test#frag|#frag")
        );
    }

    #[test]
    fn history_back_and_forward_follow_soft_navigation_stack() {
        let processed = process_document_scripts(
            "<script>history.pushState({}, '', '/one'); history.pushState({}, '', '/two'); history.back(); history.forward(); document.title = location.href + '|' + location.hash + '|' + String(history.length);</script>",
            &Url::parse("https://example.com/start").unwrap(),
        );

        assert_eq!(
            processed.soft_navigation_target.as_deref(),
            Some("https://example.com/two")
        );
        assert!(processed.navigation_target.is_none());
        assert_eq!(
            processed.title_override.as_deref(),
            Some("https://example.com/two||3")
        );
    }

    #[test]
    fn resolves_script_requests_against_document_url_after_location_changes() {
        let base_url = Url::parse("https://example.com/start").unwrap();
        let mut runtime = JavaScriptRuntime::new(&base_url, "<html><body></body></html>");
        set_location_href("https://other.example/app", &mut runtime.context);

        let resolved = resolve_requested_url(
            &JsValue::from(js_string!("/api/data")),
            &mut runtime.context,
        )
        .unwrap();

        assert_eq!(
            resolved,
            Url::parse("https://example.com/api/data").unwrap()
        );
    }

    #[test]
    fn resolves_location_updates_against_original_document_url() {
        let base_url = Url::parse("https://example.com/notes/start.html").unwrap();
        let mut runtime = JavaScriptRuntime::new(&base_url, "<html><body></body></html>");
        set_location_href("/first", &mut runtime.context);
        set_location_href("next.html", &mut runtime.context);

        assert_eq!(
            current_location_url(&mut runtime.context).unwrap(),
            Url::parse("https://example.com/notes/next.html").unwrap()
        );
    }

    #[test]
    fn blocks_cross_origin_fetch_requests_even_after_location_changes() {
        let base_url = Url::parse("https://example.com/start").unwrap();
        let mut runtime = JavaScriptRuntime::new(&base_url, "<html><body></body></html>");
        set_location_href("https://other.example/app", &mut runtime.context);

        let error = fetch_for_script(
            &Url::parse("https://other.example/data").unwrap(),
            &mut runtime.context,
        )
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("cross-origin JS requests are blocked")
        );
    }

    #[test]
    fn blocks_cross_origin_redirect_targets() {
        let current = Url::parse("https://example.com/start").unwrap();
        let redirect = Url::parse("https://other.example/data").unwrap();

        let error = ensure_same_origin_script_url(
            &current,
            &redirect,
            "cross-origin JS redirects are blocked",
        )
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("cross-origin JS redirects are blocked")
        );
    }

    #[test]
    fn propagates_request_url_getter_errors() {
        let base_url = Url::parse("https://example.com/start").unwrap();
        let mut runtime = JavaScriptRuntime::new(&base_url, "<html><body></body></html>");
        let request = runtime
            .context
            .eval(Source::from_bytes(
                "({ get url() { throw new Error('boom'); } })",
            ))
            .unwrap();

        let error = resolve_requested_url(&request, &mut runtime.context).unwrap_err();

        assert!(error.to_string().contains("boom"));
    }

    #[test]
    fn blocks_cross_origin_fetch_requests() {
        let processed = process_document_scripts(
            "<script>fetch('https://other.example/data').catch(function () { document.write('<p>blocked</p>'); });</script>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert!(processed.html.contains("<p>blocked</p>"));
    }

    #[test]
    fn aborts_runaway_loops_with_runtime_limit() {
        let processed = process_document_scripts(
            "<script>for (;;) {}</script>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert!(
            processed
                .console_logs
                .iter()
                .any(|entry| entry.contains("Maximum loop iteration limit")),
            "logs: {:?}",
            processed.console_logs
        );
    }

    #[test]
    fn rejects_xml_http_request_bodies() {
        let processed = process_document_scripts(
            "<script>var xhr = new XMLHttpRequest(); xhr.open('GET', '/api'); xhr.onerror = function () { document.write('<p>xhr blocked</p>'); }; xhr.send('payload');</script>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert!(processed.html.contains("<p>xhr blocked</p>"));
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
    fn updates_viewport_accessors_and_resize_events() {
        let (processed, session) = start_document_script_session(
            "<html><body><script>document.title = [window.innerWidth, window.innerHeight].join('x'); window.addEventListener('resize', function () { document.title = [window.innerWidth, window.innerHeight].join('x'); });</script></body></html>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert_eq!(processed.title_override.as_deref(), Some("1280x720"));

        let session = session.expect("session should exist");
        assert!(session.set_viewport_size(800, 600));
        let result = session
            .dispatch_global_event("resize", false, false)
            .expect("resize dispatch should succeed");

        assert_eq!(result.snapshot.title_override.as_deref(), Some("800x600"));
    }

    #[test]
    fn tracks_document_active_element_for_focus_and_blur() {
        let processed = process_document_scripts(
            "<html><body><button id=\"btn\">Go</button><script>var btn = document.getElementById('btn'); btn.focus(); document.title = document.activeElement.tagName; btn.blur(); document.title += '|' + document.activeElement.tagName;</script></body></html>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert_eq!(processed.title_override.as_deref(), Some("BUTTON|BODY"));
    }

    #[test]
    fn updates_scroll_accessors_and_scroll_events() {
        let (processed, session) = start_document_script_session(
            "<html><body><script>document.title = String(window.scrollY); window.addEventListener('scroll', function () { document.title = String(window.scrollY); });</script></body></html>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert_eq!(processed.title_override.as_deref(), Some("0"));

        let session = session.expect("session should exist");
        assert!(session.set_scroll_position(120));
        let result = session
            .dispatch_global_event("scroll", false, false)
            .expect("scroll dispatch should succeed");

        assert_eq!(result.snapshot.title_override.as_deref(), Some("120"));
    }

    #[test]
    fn supports_scroll_to_scroll_by_and_scroll_top_setter() {
        let processed = process_document_scripts(
            "<html><body><script>window.scrollTo({ top: 120 }); window.scrollBy({ top: 30 }); document.documentElement.scrollTop = 55; document.title = [String(window.scrollY), String(window.pageYOffset), String(document.documentElement.scrollTop)].join('|');</script></body></html>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert_eq!(processed.title_override.as_deref(), Some("55|55|55"));
        assert_eq!(processed.scroll_y, 55);
    }

    #[test]
    fn dispatches_keyboard_events_with_key_metadata() {
        let mut runtime = JavaScriptRuntime::new(
            &Url::parse("https://example.com").unwrap(),
            "<html><body><div id=\"demo\"></div><script>document.addEventListener('keydown', function (event) { document.title = [event.type, event.key, event.code, String(event.ctrlKey), String(event.shiftKey)].join('|'); });</script></body></html>",
        );
        runtime.process_loaded_document();
        runtime.dispatch_initial_load_events();

        let result = runtime.dispatch_dom_event(DomEventRequest {
            target_node_id: runtime.document_id(),
            event_type: "keydown".to_string(),
            bubbles: true,
            cancelable: true,
            key: Some("a".to_string()),
            code: Some("KeyA".to_string()),
            repeat: false,
            alt_key: false,
            ctrl_key: false,
            shift_key: false,
            meta_key: false,
        });

        assert!(!result.default_prevented);
        assert_eq!(
            result.snapshot.title_override.as_deref(),
            Some("keydown|a|KeyA|false|false")
        );
    }

    #[test]
    fn dispatches_dom_events_in_capture_target_and_bubble_order() {
        let mut runtime = JavaScriptRuntime::new(
            &Url::parse("https://example.com").unwrap(),
            "<html><body><div id=\"outer\"><button id=\"inner\">Go</button></div><script>var order = []; var outer = document.getElementById('outer'); var inner = document.getElementById('inner'); function record(label) { order.push(label); document.title = order.join('|'); } outer.addEventListener('click', function () { record('outer-capture'); }, true); outer.addEventListener('click', function () { record('outer-bubble'); }); inner.addEventListener('click', function () { record('inner-capture'); }, true); inner.addEventListener('click', function () { record('inner-bubble'); }); inner.addEventListener('click', function () { record('once'); }, { once: true });</script></body></html>",
        );
        runtime.process_loaded_document();
        runtime.dispatch_initial_load_events();

        let button_id = {
            let host = runtime
                .context
                .get_data::<super::JavaScriptHostData>()
                .unwrap();
            let document_id = {
                let state = host.state.borrow();
                state.dom.document_id
            };
            let state = host.state.borrow();
            state.dom.find_first_tag(document_id, "button").unwrap()
        };

        let result = runtime.dispatch_dom_event(DomEventRequest {
            target_node_id: button_id,
            event_type: "click".to_string(),
            bubbles: true,
            cancelable: true,
            ..Default::default()
        });

        assert!(!result.default_prevented);
        assert_eq!(
            result.snapshot.title_override.as_deref(),
            Some("outer-capture|inner-capture|inner-bubble|once|outer-bubble")
        );
    }

    #[test]
    fn once_event_listeners_are_removed_after_first_dispatch() {
        let mut runtime = JavaScriptRuntime::new(
            &Url::parse("https://example.com").unwrap(),
            "<html><body><button id=\"inner\">Go</button><script>var count = 0; var inner = document.getElementById('inner'); inner.addEventListener('click', function () { count += 1; document.title = String(count); }, { once: true });</script></body></html>",
        );
        runtime.process_loaded_document();
        runtime.dispatch_initial_load_events();

        let button_id = {
            let host = runtime
                .context
                .get_data::<super::JavaScriptHostData>()
                .unwrap();
            let document_id = {
                let state = host.state.borrow();
                state.dom.document_id
            };
            let state = host.state.borrow();
            state.dom.find_first_tag(document_id, "button").unwrap()
        };

        let first = runtime.dispatch_dom_event(DomEventRequest {
            target_node_id: button_id,
            event_type: "click".to_string(),
            bubbles: true,
            cancelable: true,
            ..Default::default()
        });
        assert_eq!(first.snapshot.title_override.as_deref(), Some("1"));

        let second = runtime.dispatch_dom_event(DomEventRequest {
            target_node_id: button_id,
            event_type: "click".to_string(),
            bubbles: true,
            cancelable: true,
            ..Default::default()
        });
        assert_eq!(second.snapshot.title_override.as_deref(), Some("1"));
    }

    #[test]
    fn passive_event_listeners_ignore_prevent_default() {
        let mut runtime = JavaScriptRuntime::new(
            &Url::parse("https://example.com").unwrap(),
            "<html><body><button id=\"inner\" type=\"button\">Go</button><script>var inner = document.getElementById('inner'); inner.addEventListener('click', function (event) { event.preventDefault(); document.title = String(event.defaultPrevented); }, { passive: true });</script></body></html>",
        );
        runtime.process_loaded_document();
        runtime.dispatch_initial_load_events();

        let button_id = {
            let host = runtime
                .context
                .get_data::<super::JavaScriptHostData>()
                .unwrap();
            let document_id = {
                let state = host.state.borrow();
                state.dom.document_id
            };
            let state = host.state.borrow();
            state.dom.find_first_tag(document_id, "button").unwrap()
        };

        let result = runtime.dispatch_dom_event(DomEventRequest {
            target_node_id: button_id,
            event_type: "click".to_string(),
            bubbles: true,
            cancelable: true,
            ..Default::default()
        });

        assert!(!result.default_prevented);
        assert_eq!(result.snapshot.title_override.as_deref(), Some("false"));
    }

    #[test]
    fn remove_event_listener_respects_capture_flag() {
        let mut runtime = JavaScriptRuntime::new(
            &Url::parse("https://example.com").unwrap(),
            "<html><body><button id=\"inner\">Go</button><script>var count = 0; var inner = document.getElementById('inner'); function record() { count += 1; document.title = String(count); } inner.addEventListener('click', record, true); inner.removeEventListener('click', record, false);</script></body></html>",
        );
        runtime.process_loaded_document();
        runtime.dispatch_initial_load_events();

        let button_id = {
            let host = runtime
                .context
                .get_data::<super::JavaScriptHostData>()
                .unwrap();
            let document_id = {
                let state = host.state.borrow();
                state.dom.document_id
            };
            let state = host.state.borrow();
            state.dom.find_first_tag(document_id, "button").unwrap()
        };

        let result = runtime.dispatch_dom_event(DomEventRequest {
            target_node_id: button_id,
            event_type: "click".to_string(),
            bubbles: true,
            cancelable: true,
            ..Default::default()
        });

        assert_eq!(result.snapshot.title_override.as_deref(), Some("1"));
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
    fn supports_inline_style_mutations() {
        let processed = process_document_scripts(
            "<html><body><div id=\"app\" style=\"color: #ff0000\"></div><script>var app = document.getElementById('app'); app.style.display = 'none'; app.style.backgroundColor = '#123456'; app.style.setProperty('margin-top', '8px'); app.style.fontStyle = 'italic'; app.style.textDecoration = 'underline'; app.style.textTransform = 'uppercase'; app.style.textIndent = '10px'; app.style.letterSpacing = '2px'; app.style.maxWidth = '120px'; app.style.minHeight = '24px'; app.style.borderWidth = '2px'; app.style.borderColor = '#abcdef'; app.style.borderStyle = 'solid'; app.style.removeProperty('display');</script></body></html>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert!(processed.html.contains("background-color: #123456"));
        assert!(processed.html.contains("margin-top: 8px"));
        assert!(processed.html.contains("font-style: italic"));
        assert!(processed.html.contains("text-decoration: underline"));
        assert!(processed.html.contains("text-transform: uppercase"));
        assert!(processed.html.contains("text-indent: 10px"));
        assert!(processed.html.contains("letter-spacing: 2px"));
        assert!(processed.html.contains("max-width: 120px"));
        assert!(processed.html.contains("min-height: 24px"));
        assert!(processed.html.contains("border-width: 2px"));
        assert!(processed.html.contains("border-color: #abcdef"));
        assert!(processed.html.contains("border-style: solid"));
        assert!(!processed.html.contains("display: none"));
    }

    #[test]
    fn supports_extended_inline_style_accessors() {
        let processed = process_document_scripts(
            "<html><body><div id=\"app\"></div><script>var app = document.getElementById('app'); app.style.maxWidth = '320px'; app.style.minWidth = '120px'; app.style.maxHeight = '480px'; app.style.minHeight = '64px'; app.style.borderWidth = '3px'; app.style.borderColor = '#112233'; app.style.borderStyle = 'dashed';</script></body></html>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert!(processed.html.contains("max-width: 320px"));
        assert!(processed.html.contains("min-width: 120px"));
        assert!(processed.html.contains("max-height: 480px"));
        assert!(processed.html.contains("min-height: 64px"));
        assert!(processed.html.contains("border-width: 3px"));
        assert!(processed.html.contains("border-color: #112233"));
        assert!(processed.html.contains("border-style: dashed"));
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

    #[test]
    fn response_clone_errors_on_invalid_receiver() {
        let mut context = Context::default();
        let result = super::js_fetch_response_clone(&JsValue::undefined(), &[], &mut context);

        assert!(result.is_err());
    }
}
