use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::mem;
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use boa_engine::object::{
    JsObject, ObjectInitializer,
    builtins::{JsArray, JsFunction, JsPromise},
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
    pub detail: Option<String>,
    pub data: Option<String>,
    pub input_type: Option<String>,
    pub client_x: Option<i32>,
    pub client_y: Option<i32>,
    pub button: Option<i32>,
    pub buttons: Option<i32>,
    pub is_composing: bool,
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
    history_entries: Vec<HistoryEntry>,
    history_index: usize,
    current_script: Option<usize>,
    network_request_count: usize,
    network_response_bytes: usize,
    viewport_width: u32,
    viewport_height: u32,
    scroll_y: u32,
    active_element_node_id: Option<usize>,
    pending_tasks: VecDeque<PendingTask>,
    next_task_handle: usize,
    dom: DomState,
}

#[derive(Debug, Clone)]
struct HistoryEntry {
    href: String,
    scroll_y: u32,
    state: JsValue,
}

#[derive(Debug, Clone)]
struct PendingTask {
    handle: usize,
    kind: PendingTaskKind,
    action: PendingTaskAction,
}

#[derive(Debug, Clone)]
enum PendingTaskKind {
    Microtask,
    AnimationFrame,
    Timeout { repeat: bool },
}

#[derive(Debug, Clone)]
enum PendingTaskAction {
    Callback {
        callback: JsValue,
        args: Vec<JsValue>,
    },
    Script(String),
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
    Fragment,
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
struct DomDatasetHandle {
    #[unsafe_ignore_trace]
    node_id: usize,
}

#[derive(Debug, Clone, Trace, Finalize, JsData)]
struct DomAttributesHandle {
    #[unsafe_ignore_trace]
    node_id: usize,
}

#[derive(Debug, Clone, Trace, Finalize, JsData)]
struct DomStyleHandle {
    #[unsafe_ignore_trace]
    node_id: usize,
}

#[derive(Debug, Clone, Trace, Finalize, JsData)]
struct ComputedStyleHandle {
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
    response_headers: BTreeMap<String, String>,
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
                history_entries: vec![HistoryEntry {
                    href: base_url.to_string(),
                    scroll_y: 0,
                    state: JsValue::null(),
                }],
                history_index: 0,
                current_script: None,
                network_request_count: 0,
                network_response_bytes: 0,
                viewport_width: DEFAULT_VIEWPORT_WIDTH,
                viewport_height: DEFAULT_VIEWPORT_HEIGHT,
                scroll_y: 0,
                active_element_node_id: None,
                pending_tasks: VecDeque::new(),
                next_task_handle: 1,
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

    fn settle_pending_state(&mut self) {
        self.flush_pending_document_writes();
        self.process_loaded_document();
        self.flush_pending_tasks();
        self.flush_pending_document_writes();
        self.process_loaded_document();
    }

    fn queue_pending_task(&self, kind: PendingTaskKind, action: PendingTaskAction) -> usize {
        let Some(host) = self.context.get_data::<JavaScriptHostData>() else {
            return 0;
        };
        let mut state = host.state.borrow_mut();
        let handle = state.next_task_handle;
        state.next_task_handle = state.next_task_handle.checked_add(1).unwrap_or(1);
        state.pending_tasks.push_back(PendingTask {
            handle,
            kind,
            action,
        });
        handle
    }

    fn take_pending_tasks(&self) -> Vec<PendingTask> {
        let Some(host) = self.context.get_data::<JavaScriptHostData>() else {
            return Vec::new();
        };
        let mut state = host.state.borrow_mut();
        mem::take(&mut state.pending_tasks).into_iter().collect()
    }

    fn flush_pending_tasks(&mut self) {
        let pending_tasks = self.take_pending_tasks();
        if pending_tasks.is_empty() {
            return;
        }

        let mut microtasks = Vec::new();
        let mut animation_frames = Vec::new();
        let mut timeouts = Vec::new();

        for task in pending_tasks {
            match task.kind {
                PendingTaskKind::Microtask => microtasks.push(task),
                PendingTaskKind::AnimationFrame => animation_frames.push(task),
                PendingTaskKind::Timeout { .. } => timeouts.push(task),
            }
        }

        for task in microtasks {
            self.run_pending_task(task);
        }
        for task in animation_frames {
            self.run_pending_task(task);
        }
        for task in timeouts {
            let repeat = matches!(&task.kind, PendingTaskKind::Timeout { repeat: true });
            self.run_pending_task(task.clone());
            if repeat {
                self.queue_pending_task(task.kind.clone(), task.action.clone());
            }
        }
    }

    fn run_pending_task(&mut self, task: PendingTask) {
        let result = match task.action {
            PendingTaskAction::Callback { callback, mut args } => {
                if matches!(task.kind, PendingTaskKind::AnimationFrame) {
                    args.insert(0, JsValue::new(performance_now_ms()));
                }
                let this_value = if matches!(task.kind, PendingTaskKind::Microtask) {
                    JsValue::undefined()
                } else {
                    JsValue::from(self.context.global_object().clone())
                };
                call_js_callback_with_this(&callback, &this_value, &args, &mut self.context)
                    .map(|_| ())
            }
            PendingTaskAction::Script(source) => self
                .context
                .eval(Source::from_bytes(source.as_str()))
                .map(|_| ()),
        };

        if let Err(error) = result {
            self.push_log(format!("js task error: {error}"));
        }
        if let Err(error) = self.context.run_jobs() {
            self.push_log(format!("js job error: {error}"));
        }
        flush_mutation_observers(&mut self.context);
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
        self.settle_pending_state();
        flush_mutation_observers(&mut self.context);
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
            if changed {
                sync_current_history_entry_scroll(&mut state);
            }
            return changed;
        }
        false
    }

    fn set_dom_attribute(&mut self, node_id: usize, name: &str, value: &str) {
        if let Some(host) = self.context.get_data::<JavaScriptHostData>() {
            let old_value = host.state.borrow().dom.get_attribute(node_id, name);
            host.state
                .borrow_mut()
                .dom
                .set_attribute(node_id, name, value);
            record_dom_attribute_mutation(&mut self.context, node_id, name, old_value);
        }
        flush_mutation_observers(&mut self.context);
    }

    fn dispatch_dom_event(&mut self, request: DomEventRequest) -> DomEventDispatchResult {
        let default_prevented = self.dispatch_dom_event_request(request).unwrap_or(false);
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
                flush_mutation_observers(&mut self.context);
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
        let request = DomEventRequest {
            target_node_id: self.document_id(),
            event_type: event_type.to_string(),
            bubbles,
            cancelable,
            ..Default::default()
        };
        let target = self.context.global_object();
        let event = build_dom_event_object(&mut self.context, &request, &target);
        dispatch_global_event_object(&mut self.context, event_type, bubbles, cancelable, &event)
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

    fn create_document_fragment(&mut self) -> usize {
        let node_id = self.nodes.len();
        self.nodes.push(DomNode {
            parent: None,
            children: Vec::new(),
            kind: DomNodeKind::Fragment,
        });
        node_id
    }

    fn is_document_node(&self, node_id: usize) -> bool {
        node_id == self.document_id && self.node(node_id).is_some()
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
            DomNodeKind::Text(_) | DomNodeKind::Fragment => None,
        }
    }

    fn element_mut(&mut self, node_id: usize) -> Option<&mut DomElementData> {
        match &mut self.node_mut(node_id)?.kind {
            DomNodeKind::Element(element) => Some(element),
            DomNodeKind::Text(_) | DomNodeKind::Fragment => None,
        }
    }

    fn node_type(&self, node_id: usize) -> u16 {
        if !self.nodes.get(node_id).is_some() {
            return 0;
        }
        if self.is_document_node(node_id) {
            return 9;
        }
        match self.node(node_id).map(|node| &node.kind) {
            Some(DomNodeKind::Element(_)) => 1,
            Some(DomNodeKind::Text(_)) => 3,
            Some(DomNodeKind::Fragment) => 11,
            None => 0,
        }
    }

    fn node_name(&self, node_id: usize) -> Option<String> {
        if self.is_document_node(node_id) {
            return Some("#document".to_string());
        }
        match self.node(node_id).map(|node| &node.kind) {
            Some(DomNodeKind::Element(element)) => Some(element.tag_name.to_ascii_uppercase()),
            Some(DomNodeKind::Text(_)) => Some("#text".to_string()),
            Some(DomNodeKind::Fragment) => Some("#document-fragment".to_string()),
            None => None,
        }
    }

    fn node_value(&self, node_id: usize) -> Option<String> {
        match self.node(node_id).map(|node| &node.kind) {
            Some(DomNodeKind::Text(text)) => Some(text.clone()),
            Some(DomNodeKind::Element(_)) | Some(DomNodeKind::Fragment) => None,
            None => None,
        }
    }

    fn set_node_value(&mut self, node_id: usize, value: &str) {
        if let Some(DomNodeKind::Text(text)) = self.node_mut(node_id).map(|node| &mut node.kind) {
            *text = value.to_string();
        }
    }

    fn first_child(&self, node_id: usize) -> Option<usize> {
        self.node(node_id)?.children.first().copied()
    }

    fn last_child(&self, node_id: usize) -> Option<usize> {
        self.node(node_id)?.children.last().copied()
    }

    fn previous_sibling(&self, node_id: usize) -> Option<usize> {
        let parent_id = self.node(node_id)?.parent?;
        let parent = self.node(parent_id)?;
        let index = parent
            .children
            .iter()
            .position(|child_id| *child_id == node_id)?;
        index
            .checked_sub(1)
            .and_then(|previous_index| parent.children.get(previous_index).copied())
    }

    fn next_sibling(&self, node_id: usize) -> Option<usize> {
        let parent_id = self.node(node_id)?.parent?;
        let parent = self.node(parent_id)?;
        let index = parent
            .children
            .iter()
            .position(|child_id| *child_id == node_id)?;
        parent.children.get(index + 1).copied()
    }

    fn is_connected(&self, node_id: usize) -> bool {
        self.contains_node(self.document_id, node_id)
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
        let fragment_children = matches!(
            self.node(child_id).map(|node| &node.kind),
            Some(DomNodeKind::Fragment)
        )
        .then(|| {
            self.node(child_id)
                .map(|node| node.children.clone())
                .unwrap_or_default()
        });
        self.detach_node(child_id);
        if let Some(fragment_children) = fragment_children {
            for fragment_child in fragment_children {
                self.append_child(parent_id, fragment_child);
            }
            return;
        }
        if let Some(parent) = self.node_mut(parent_id) {
            parent.children.push(child_id);
        }
        if let Some(child) = self.node_mut(child_id) {
            child.parent = Some(parent_id);
        }
    }

    fn clone_node(&mut self, node_id: usize, deep: bool) -> Option<usize> {
        let node = self.node(node_id)?.clone();
        let cloned_id = match node.kind {
            DomNodeKind::Text(text) => self.create_text_node(&text),
            DomNodeKind::Element(element) => {
                let cloned_id = self.nodes.len();
                self.nodes.push(DomNode {
                    parent: None,
                    children: Vec::new(),
                    kind: DomNodeKind::Element(element),
                });
                cloned_id
            }
            DomNodeKind::Fragment => self.create_document_fragment(),
        };
        if deep {
            for child_id in node.children {
                let cloned_child = self.clone_node(child_id, true)?;
                self.append_child(cloned_id, cloned_child);
            }
        }
        Some(cloned_id)
    }

    fn replace_child(
        &mut self,
        parent_id: usize,
        new_child_id: usize,
        old_child_id: usize,
    ) -> Option<usize> {
        if new_child_id == old_child_id {
            return Some(old_child_id);
        }

        let new_child_is_fragment = matches!(
            self.node(new_child_id).map(|node| &node.kind),
            Some(DomNodeKind::Fragment)
        );
        if new_child_is_fragment {
            let fragment_children = self
                .node(new_child_id)
                .map(|node| node.children.clone())
                .unwrap_or_default();
            self.detach_node(new_child_id);
            for child_id in fragment_children {
                self.insert_before(parent_id, child_id, Some(old_child_id));
            }
            self.detach_node(old_child_id);
            return Some(old_child_id);
        }

        self.detach_node(new_child_id);
        let current_index = self
            .node(parent_id)?
            .children
            .iter()
            .position(|child_id| *child_id == old_child_id)?;
        if let Some(parent) = self.node_mut(parent_id) {
            parent.children[current_index] = new_child_id;
        }
        if let Some(child) = self.node_mut(new_child_id) {
            child.parent = Some(parent_id);
        }
        if let Some(old_child) = self.node_mut(old_child_id) {
            old_child.parent = None;
        }
        Some(old_child_id)
    }

    fn remove_child(&mut self, parent_id: usize, child_id: usize) -> Option<usize> {
        let parent = self.node(parent_id)?;
        if parent.children.iter().any(|node_id| *node_id == child_id) {
            self.detach_node(child_id);
            return Some(child_id);
        }
        None
    }

    fn insert_before(&mut self, parent_id: usize, child_id: usize, before_id: Option<usize>) {
        let fragment_children = matches!(
            self.node(child_id).map(|node| &node.kind),
            Some(DomNodeKind::Fragment)
        )
        .then(|| {
            self.node(child_id)
                .map(|node| node.children.clone())
                .unwrap_or_default()
        });
        self.detach_node(child_id);
        if let Some(fragment_children) = fragment_children {
            for fragment_child in fragment_children {
                self.insert_before(parent_id, fragment_child, before_id);
            }
            return;
        }
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

    fn insert_fragment_after(&mut self, target_id: usize, html: &str) {
        let parent_id = self.node(target_id).and_then(|node| node.parent);
        let Some(parent_id) = parent_id else {
            return;
        };
        let next_sibling_id = self.node(parent_id).and_then(|parent| {
            parent
                .children
                .iter()
                .position(|child_id| *child_id == target_id)
                .and_then(|index| parent.children.get(index + 1).copied())
        });
        let fragment = parse_document(html);
        let fragment_root_id = self.push_node(None, &fragment);
        let fragment_children = self
            .node(fragment_root_id)
            .map(|node| node.children.clone())
            .unwrap_or_default();
        for child_id in fragment_children {
            self.insert_before(parent_id, child_id, next_sibling_id);
        }
    }

    fn insert_fragment_at_start(&mut self, parent_id: usize, html: &str) {
        let fragment = parse_document(html);
        let fragment_root_id = self.push_node(None, &fragment);
        let fragment_children = self
            .node(fragment_root_id)
            .map(|node| node.children.clone())
            .unwrap_or_default();
        let first_child = self
            .node(parent_id)
            .and_then(|node| node.children.first().copied());
        for child_id in fragment_children {
            self.insert_before(parent_id, child_id, first_child);
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
            DomNodeKind::Element(_) | DomNodeKind::Fragment => node
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
            DomNodeKind::Fragment => node
                .children
                .iter()
                .map(|child_id| self.serialize_node(*child_id))
                .collect(),
        }
    }

    fn get_attribute(&self, node_id: usize, name: &str) -> Option<String> {
        self.element(node_id)
            .and_then(|element| element.attributes.get(name))
            .cloned()
    }

    fn has_attribute(&self, node_id: usize, name: &str) -> bool {
        self.get_attribute(node_id, name).is_some()
    }

    fn attribute_names(&self, node_id: usize) -> Vec<String> {
        self.element(node_id)
            .map(|element| element.attributes.keys().cloned().collect())
            .unwrap_or_default()
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

    fn tree_root(&self, node_id: usize) -> usize {
        let mut current = node_id;
        while let Some(parent_id) = self.node(current).and_then(|node| node.parent) {
            current = parent_id;
        }
        current
    }

    fn contains_node(&self, ancestor_id: usize, node_id: usize) -> bool {
        if ancestor_id == node_id {
            return self.node(ancestor_id).is_some();
        }
        let mut cursor = self.node(node_id).and_then(|node| node.parent);
        while let Some(parent_id) = cursor {
            if parent_id == ancestor_id {
                return true;
            }
            cursor = self.node(parent_id).and_then(|node| node.parent);
        }
        false
    }

    fn element_children(&self, node_id: usize) -> Vec<usize> {
        self.node(node_id)
            .map(|node| {
                node.children
                    .iter()
                    .copied()
                    .filter(|child_id| self.element(*child_id).is_some())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn first_element_child(&self, node_id: usize) -> Option<usize> {
        self.node(node_id).and_then(|node| {
            node.children
                .iter()
                .copied()
                .find(|child_id| self.element(*child_id).is_some())
        })
    }

    fn last_element_child(&self, node_id: usize) -> Option<usize> {
        self.node(node_id).and_then(|node| {
            node.children
                .iter()
                .rev()
                .copied()
                .find(|child_id| self.element(*child_id).is_some())
        })
    }

    fn previous_element_sibling(&self, node_id: usize) -> Option<usize> {
        let parent_id = self.node(node_id)?.parent?;
        let parent = self.node(parent_id)?;
        let index = parent
            .children
            .iter()
            .position(|child_id| *child_id == node_id)?;
        parent.children[..index]
            .iter()
            .rev()
            .copied()
            .find(|sibling_id| self.element(*sibling_id).is_some())
    }

    fn next_element_sibling(&self, node_id: usize) -> Option<usize> {
        let parent_id = self.node(node_id)?.parent?;
        let parent = self.node(parent_id)?;
        let index = parent
            .children
            .iter()
            .position(|child_id| *child_id == node_id)?;
        parent.children[index + 1..]
            .iter()
            .copied()
            .find(|sibling_id| self.element(*sibling_id).is_some())
    }

    fn matches_selector(&self, node_id: usize, selector: &str) -> bool {
        let Some(selector) = ParsedSelector::parse(selector) else {
            return false;
        };
        let scope_id = self.tree_root(node_id);
        self.matches_selector_in_scope(node_id, scope_id, &selector)
    }

    fn closest_selector(&self, node_id: usize, selector: &str) -> Option<usize> {
        let selector = ParsedSelector::parse(selector)?;
        let scope_id = self.tree_root(node_id);
        let mut current = Some(node_id);
        while let Some(candidate_id) = current {
            if self.element(candidate_id).is_some()
                && self.matches_selector_in_scope(candidate_id, scope_id, &selector)
            {
                return Some(candidate_id);
            }
            current = self.node(candidate_id).and_then(|node| node.parent);
        }
        None
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

    fn replace_class(&mut self, node_id: usize, old_class: &str, new_class: &str) -> bool {
        if !self.has_class(node_id, old_class) {
            return false;
        }
        if old_class == new_class {
            return true;
        }
        self.remove_class(node_id, old_class);
        self.add_class(node_id, new_class);
        true
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
    let node_list_stub = build_simple_node_list_stub(context);
    let document_fonts = ObjectInitializer::new(context)
        .function(NativeFunction::from_fn_ptr(js_noop), js_string!("load"), 2)
        .build();
    let get_node_name =
        NativeFunction::from_fn_ptr(js_dom_get_node_name).to_js_function(context.realm());
    let get_node_type =
        NativeFunction::from_fn_ptr(js_dom_get_node_type).to_js_function(context.realm());
    let get_node_value =
        NativeFunction::from_fn_ptr(js_dom_get_node_value).to_js_function(context.realm());
    let set_node_value =
        NativeFunction::from_fn_ptr(js_dom_set_node_value).to_js_function(context.realm());
    let get_first_child =
        NativeFunction::from_fn_ptr(js_dom_get_first_child).to_js_function(context.realm());
    let get_last_child =
        NativeFunction::from_fn_ptr(js_dom_get_last_child).to_js_function(context.realm());
    let get_previous_sibling =
        NativeFunction::from_fn_ptr(js_dom_get_previous_sibling).to_js_function(context.realm());
    let get_next_sibling =
        NativeFunction::from_fn_ptr(js_dom_get_next_sibling).to_js_function(context.realm());
    let get_is_connected =
        NativeFunction::from_fn_ptr(js_dom_get_is_connected).to_js_function(context.realm());
    let get_parent_node =
        NativeFunction::from_fn_ptr(js_dom_get_parent_node).to_js_function(context.realm());
    let get_parent_element =
        NativeFunction::from_fn_ptr(js_dom_get_parent_element).to_js_function(context.realm());
    let get_owner_document =
        NativeFunction::from_fn_ptr(js_dom_get_owner_document).to_js_function(context.realm());
    let cookie_getter =
        NativeFunction::from_fn_ptr(js_document_get_cookie).to_js_function(context.realm());
    let cookie_setter =
        NativeFunction::from_fn_ptr(js_document_set_cookie).to_js_function(context.realm());
    let active_element_getter =
        NativeFunction::from_fn_ptr(js_document_get_active_element).to_js_function(context.realm());
    let body_getter =
        NativeFunction::from_fn_ptr(js_document_get_body).to_js_function(context.realm());
    let head_getter =
        NativeFunction::from_fn_ptr(js_document_get_head).to_js_function(context.realm());
    let document_element_getter = NativeFunction::from_fn_ptr(js_document_get_document_element)
        .to_js_function(context.realm());

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
        NativeFunction::from_fn_ptr(js_document_create_document_fragment),
        js_string!("createDocumentFragment"),
        0,
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
    .accessor(
        js_string!("nodeName"),
        Some(get_node_name.clone()),
        None,
        Attribute::all(),
    )
    .accessor(
        js_string!("nodeType"),
        Some(get_node_type.clone()),
        None,
        Attribute::all(),
    )
    .accessor(
        js_string!("nodeValue"),
        Some(get_node_value.clone()),
        Some(set_node_value.clone()),
        Attribute::all(),
    )
    .accessor(
        js_string!("firstChild"),
        Some(get_first_child.clone()),
        None,
        Attribute::all(),
    )
    .accessor(
        js_string!("lastChild"),
        Some(get_last_child.clone()),
        None,
        Attribute::all(),
    )
    .accessor(
        js_string!("previousSibling"),
        Some(get_previous_sibling.clone()),
        None,
        Attribute::all(),
    )
    .accessor(
        js_string!("nextSibling"),
        Some(get_next_sibling.clone()),
        None,
        Attribute::all(),
    )
    .accessor(
        js_string!("isConnected"),
        Some(get_is_connected.clone()),
        None,
        Attribute::all(),
    )
    .accessor(
        js_string!("parentNode"),
        Some(get_parent_node.clone()),
        None,
        Attribute::all(),
    )
    .accessor(
        js_string!("parentElement"),
        Some(get_parent_element.clone()),
        None,
        Attribute::all(),
    )
    .accessor(
        js_string!("ownerDocument"),
        Some(get_owner_document.clone()),
        None,
        Attribute::all(),
    )
    .property(js_string!("location"), location.clone(), Attribute::all())
    .accessor(
        js_string!("body"),
        Some(body_getter),
        None,
        Attribute::all(),
    )
    .accessor(
        js_string!("head"),
        Some(head_getter),
        None,
        Attribute::all(),
    )
    .accessor(
        js_string!("documentElement"),
        Some(document_element_getter),
        None,
        Attribute::all(),
    )
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
    let history_state_getter =
        NativeFunction::from_fn_ptr(js_history_state).to_js_function(context.realm());
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
        .accessor(
            js_string!("state"),
            Some(history_state_getter),
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
            NativeFunction::from_fn_ptr(js_set_interval),
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
            NativeFunction::from_fn_ptr(js_clear_animation_frame),
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
            js_string!("getComputedStyle"),
            2,
            NativeFunction::from_fn_ptr(js_window_get_computed_style),
        )
        .expect("getComputedStyle should be installable");
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
    // Note: innerHeight is dynamically retrieved via inner_height_getter (which aligns with the CSS 800px vh base in css.rs).
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
    context
        .register_global_builtin_callable(
            js_string!("__tobiraGetNodeById"),
            1,
            NativeFunction::from_fn_ptr(js_get_node_by_id),
        )
        .expect("__tobiraGetNodeById should be installable");
    context
        .register_global_builtin_callable(
            js_string!("__tobiraCreateMutationObserver"),
            1,
            NativeFunction::from_fn_ptr(js_create_mutation_observer),
        )
        .expect("__tobiraCreateMutationObserver should be installable");
    context
        .eval(Source::from_bytes(
            r#"
globalThis.__tobiraMutationObservers = [];
globalThis.MutationObserver = function MutationObserver(callback) {
  var observer = __tobiraCreateMutationObserver(callback);
  observer.observe = function (target, options) {
    if (!target || typeof target !== 'object') {
      return;
    }
    var normalized = options && typeof options === 'object' ? options : {};
    this.observations.push({
      target: target,
      childList: !!normalized.childList,
      attributes: !!normalized.attributes,
      characterData: !!normalized.characterData,
      subtree: !!normalized.subtree,
      attributeOldValue: !!normalized.attributeOldValue,
      characterDataOldValue: !!normalized.characterDataOldValue,
      attributeFilter: Array.isArray(normalized.attributeFilter)
        ? normalized.attributeFilter.map(String)
        : null,
    });
  };
  observer.disconnect = function () {
    this.observations.length = 0;
    this.records.length = 0;
  };
  observer.takeRecords = function () {
    var records = this.records.slice();
    this.records.length = 0;
    return records;
  };
  observer.matchesMutation = function (mutationType, target, attributeName) {
    return this.observations.some(function (observation) {
      if (!observation) {
        return false;
      }
      if (mutationType === 'attributes' && !observation.attributes) {
        return false;
      }
      if (mutationType === 'childList' && !observation.childList) {
        return false;
      }
      if (mutationType === 'characterData' && !observation.characterData) {
        return false;
      }
      if (observation.target !== target && !(observation.subtree && observation.target.contains(target))) {
        return false;
      }
      if (mutationType === 'attributes' && observation.attributeFilter && !observation.attributeFilter.includes(attributeName)) {
        return false;
      }
      return true;
    });
  };
  __tobiraMutationObservers.push(observer);
  return observer;
};
globalThis.__tobiraRecordMutation = function (mutationType, targetId, attributeName, oldValue, addedNodeIds, removedNodeIds) {
  if (!globalThis.__tobiraMutationObservers || !globalThis.__tobiraMutationObservers.length) {
    return;
  }
  var target = __tobiraGetNodeById(targetId);
  if (!target) {
    return;
  }
  var addedNodes = Array.isArray(addedNodeIds)
    ? addedNodeIds.map(__tobiraGetNodeById).filter(Boolean)
    : [];
  var removedNodes = Array.isArray(removedNodeIds)
    ? removedNodeIds.map(__tobiraGetNodeById).filter(Boolean)
    : [];
  var record = {
    type: mutationType,
    target: target,
    attributeName: attributeName == null ? null : String(attributeName),
    oldValue: oldValue == null ? null : String(oldValue),
    addedNodes: addedNodes,
    removedNodes: removedNodes,
  };
  for (var i = 0; i < __tobiraMutationObservers.length; i += 1) {
    var observer = __tobiraMutationObservers[i];
    if (observer && observer.matchesMutation(mutationType, target, record.attributeName)) {
      observer.records.push(record);
    }
  }
};
globalThis.__tobiraFlushMutationObservers = function () {
  if (!globalThis.__tobiraMutationObservers || !globalThis.__tobiraMutationObservers.length) {
    return false;
  }
  var delivered = false;
  for (var i = 0; i < __tobiraMutationObservers.length; i += 1) {
    var observer = __tobiraMutationObservers[i];
    if (observer && observer.records.length) {
      delivered = true;
      var records = observer.takeRecords();
      observer.callback.call(observer, records, observer);
    }
  }
  return delivered;
};
if (typeof globalThis.Event !== 'function') {
  globalThis.__tobiraCreateEventObject = function (ctor, type, init, extra) {
    var event = Object.create(ctor.prototype);
    init = init && typeof init === 'object' ? init : {};
    event.type = String(type == null ? '' : type);
    event.bubbles = !!init.bubbles;
    event.cancelable = !!init.cancelable;
    event.composed = !!init.composed;
    event.defaultPrevented = false;
    event.eventPhase = 0;
    event.target = null;
    event.currentTarget = null;
    event.cancelBubble = false;
    event.propagationStopped = false;
    event.immediatePropagationStopped = false;
    if (typeof extra === 'function') {
      extra(event, init);
    }
    return event;
  };
  globalThis.Event = function Event(type, init) {
    if (!(this instanceof Event)) {
      return new Event(type, init);
    }
    return __tobiraCreateEventObject(Event, type, init);
  };
  Event.prototype.preventDefault = function () {
    if (!this.cancelable || this.__tobiraPassiveListener) {
      return;
    }
    this.defaultPrevented = true;
  };
  Event.prototype.stopPropagation = function () {
    this.propagationStopped = true;
    this.cancelBubble = true;
  };
  Event.prototype.stopImmediatePropagation = function () {
    this.immediatePropagationStopped = true;
    this.propagationStopped = true;
    this.cancelBubble = true;
  };
  globalThis.CustomEvent = function CustomEvent(type, init) {
    if (!(this instanceof CustomEvent)) {
      return new CustomEvent(type, init);
    }
    return __tobiraCreateEventObject(CustomEvent, type, init, function (event, initValue) {
      event.detail = initValue.detail == null ? null : String(initValue.detail);
    });
  };
  CustomEvent.prototype = Object.create(Event.prototype);
  CustomEvent.prototype.constructor = CustomEvent;
  globalThis.KeyboardEvent = function KeyboardEvent(type, init) {
    if (!(this instanceof KeyboardEvent)) {
      return new KeyboardEvent(type, init);
    }
    return __tobiraCreateEventObject(KeyboardEvent, type, init, function (event, initValue) {
      event.key = initValue.key == null ? '' : String(initValue.key);
      event.code = initValue.code == null ? '' : String(initValue.code);
      event.repeat = !!initValue.repeat;
      event.altKey = !!initValue.altKey;
      event.ctrlKey = !!initValue.ctrlKey;
      event.shiftKey = !!initValue.shiftKey;
      event.metaKey = !!initValue.metaKey;
    });
  };
  KeyboardEvent.prototype = Object.create(Event.prototype);
  KeyboardEvent.prototype.constructor = KeyboardEvent;
  globalThis.InputEvent = function InputEvent(type, init) {
    if (!(this instanceof InputEvent)) {
      return new InputEvent(type, init);
    }
    return __tobiraCreateEventObject(InputEvent, type, init, function (event, initValue) {
      event.data = initValue.data == null ? null : String(initValue.data);
      event.inputType = initValue.inputType == null ? '' : String(initValue.inputType);
      event.isComposing = !!initValue.isComposing;
      event.altKey = !!initValue.altKey;
      event.ctrlKey = !!initValue.ctrlKey;
      event.shiftKey = !!initValue.shiftKey;
      event.metaKey = !!initValue.metaKey;
    });
  };
  InputEvent.prototype = Object.create(Event.prototype);
  InputEvent.prototype.constructor = InputEvent;
  globalThis.MouseEvent = function MouseEvent(type, init) {
    if (!(this instanceof MouseEvent)) {
      return new MouseEvent(type, init);
    }
    return __tobiraCreateEventObject(MouseEvent, type, init, function (event, initValue) {
      event.clientX = Number(initValue.clientX || 0);
      event.clientY = Number(initValue.clientY || 0);
      event.button = Number(initValue.button || 0);
      event.buttons = Number(initValue.buttons || 0);
      event.altKey = !!initValue.altKey;
      event.ctrlKey = !!initValue.ctrlKey;
      event.shiftKey = !!initValue.shiftKey;
      event.metaKey = !!initValue.metaKey;
    });
  };
  MouseEvent.prototype = Object.create(Event.prototype);
  MouseEvent.prototype.constructor = MouseEvent;
  globalThis.FocusEvent = function FocusEvent(type, init) {
    if (!(this instanceof FocusEvent)) {
      return new FocusEvent(type, init);
    }
    return __tobiraCreateEventObject(FocusEvent, type, init, function (event, initValue) {
      event.relatedTarget = initValue.relatedTarget == null ? null : initValue.relatedTarget;
    });
  };
  FocusEvent.prototype = Object.create(Event.prototype);
  FocusEvent.prototype.constructor = FocusEvent;
  globalThis.SubmitEvent = function SubmitEvent(type, init) {
    if (!(this instanceof SubmitEvent)) {
      return new SubmitEvent(type, init);
    }
    return __tobiraCreateEventObject(SubmitEvent, type, init, function (event, initValue) {
      event.submitter = initValue.submitter == null ? null : initValue.submitter;
    });
  };
  SubmitEvent.prototype = Object.create(Event.prototype);
  SubmitEvent.prototype.constructor = SubmitEvent;
  globalThis.AbortSignal = function AbortSignal() {
    if (!(this instanceof AbortSignal)) {
      return new AbortSignal();
    }
    this.aborted = false;
    this.reason = null;
    this.__tobiraAbortListeners = [];
  };
  AbortSignal.prototype.addEventListener = function (type, callback) {
    if (String(type).toLowerCase() !== 'abort' || typeof callback !== 'function') {
      return;
    }
    this.__tobiraAbortListeners.push(callback);
  };
  AbortSignal.prototype.removeEventListener = function (type, callback) {
    if (String(type).toLowerCase() !== 'abort' || typeof callback !== 'function') {
      return;
    }
    this.__tobiraAbortListeners = this.__tobiraAbortListeners.filter(function (listener) {
      return listener !== callback;
    });
  };
  AbortSignal.prototype.dispatchEvent = function (event) {
    if (!event || event.type !== 'abort') {
      return true;
    }
    var listeners = this.__tobiraAbortListeners.slice();
    for (var i = 0; i < listeners.length; i += 1) {
      try {
        listeners[i].call(this, event);
      } catch (error) {
      }
    }
    return true;
  };
  globalThis.AbortController = function AbortController() {
    if (!(this instanceof AbortController)) {
      return new AbortController();
    }
    this.signal = new AbortSignal();
  };
  AbortController.prototype.abort = function (reason) {
    if (this.signal.aborted) {
      return;
    }
    this.signal.aborted = true;
    this.signal.reason = reason == null ? null : reason;
    this.signal.dispatchEvent(new Event('abort', { bubbles: false, cancelable: false }));
  };
}
"#,
        ))
        .expect("MutationObserver bootstrap should evaluate");
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

fn dom_dataset_cache(context: &mut Context) -> JsResult<boa_engine::object::JsObject> {
    let global = context.global_object();
    let cache_key = js_string!("__tobiraDomDatasetCache");
    let existing = global.get(cache_key.clone(), context)?;
    if let Some(object) = existing.as_object() {
        return Ok(object.clone());
    }

    let cache = ObjectInitializer::new(context).build();
    global.set(cache_key, cache.clone(), true, context)?;
    Ok(cache)
}

fn dom_attributes_cache(context: &mut Context) -> JsResult<boa_engine::object::JsObject> {
    let global = context.global_object();
    let cache_key = js_string!("__tobiraDomAttributesCache");
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

fn cached_dom_dataset_object(
    context: &mut Context,
    node_id: usize,
) -> Option<boa_engine::object::JsObject> {
    let cache = dom_dataset_cache(context).ok()?;
    let key = js_string!(node_id.to_string());
    cache
        .get(key, context)
        .ok()
        .and_then(|value| value.as_object())
}

fn store_dom_dataset_object(
    context: &mut Context,
    node_id: usize,
    object: &boa_engine::object::JsObject,
) {
    if let Ok(cache) = dom_dataset_cache(context) {
        let _ = cache.set(
            js_string!(node_id.to_string()),
            object.clone(),
            true,
            context,
        );
    }
}

fn cached_dom_attributes_object(
    context: &mut Context,
    node_id: usize,
) -> Option<boa_engine::object::JsObject> {
    let cache = dom_attributes_cache(context).ok()?;
    let key = js_string!(node_id.to_string());
    cache
        .get(key, context)
        .ok()
        .and_then(|value| value.as_object())
}

fn store_dom_attributes_object(
    context: &mut Context,
    node_id: usize,
    object: &boa_engine::object::JsObject,
) {
    if let Ok(cache) = dom_attributes_cache(context) {
        let _ = cache.set(
            js_string!(node_id.to_string()),
            object.clone(),
            true,
            context,
        );
    }
}

fn build_dom_dataset_object(context: &mut Context, node_id: usize) -> boa_engine::object::JsObject {
    if let Some(cached) = cached_dom_dataset_object(context, node_id) {
        return cached;
    }

    let target = ObjectInitializer::new(context)
        .property(
            js_string!("__tobiraNodeId"),
            js_string!(node_id.to_string()),
            Attribute::all(),
        )
        .build();
    let handler = ObjectInitializer::with_native_data(DomDatasetHandle { node_id }, context)
        .function(
            NativeFunction::from_fn_ptr(js_dom_dataset_get),
            js_string!("get"),
            3,
        )
        .function(
            NativeFunction::from_fn_ptr(js_dom_dataset_set),
            js_string!("set"),
            3,
        )
        .function(
            NativeFunction::from_fn_ptr(js_dom_dataset_has),
            js_string!("has"),
            2,
        )
        .function(
            NativeFunction::from_fn_ptr(js_dom_dataset_delete_property),
            js_string!("deleteProperty"),
            2,
        )
        .function(
            NativeFunction::from_fn_ptr(js_dom_dataset_own_keys),
            js_string!("ownKeys"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_dom_dataset_get_own_property_descriptor),
            js_string!("getOwnPropertyDescriptor"),
            2,
        )
        .build();

    let global = context.global_object();
    let target_key = js_string!("__tobiraDatasetTarget");
    let handler_key = js_string!("__tobiraDatasetHandler");
    let proxy = (|| -> JsResult<JsObject> {
        global.set(target_key.clone(), target.clone(), true, context)?;
        global.set(handler_key.clone(), handler.clone(), true, context)?;
        let proxy_value = context.eval(Source::from_bytes(
            "new Proxy(globalThis.__tobiraDatasetTarget, globalThis.__tobiraDatasetHandler);",
        ))?;
        proxy_value.as_object().ok_or_else(|| {
            JsNativeError::typ()
                .with_message("dataset proxy bootstrap did not return an object")
                .into()
        })
    })();

    let _ = global.delete_property_or_throw(target_key, context);
    let _ = global.delete_property_or_throw(handler_key, context);

    match proxy {
        Ok(proxy) => {
            store_dom_dataset_object(context, node_id, &proxy);
            proxy
        }
        Err(_) => target,
    }
}

fn build_dom_attributes_object(
    context: &mut Context,
    node_id: usize,
) -> boa_engine::object::JsObject {
    if let Some(cached) = cached_dom_attributes_object(context, node_id) {
        return cached;
    }

    let target = ObjectInitializer::with_native_data(DomAttributesHandle { node_id }, context)
        .property(
            js_string!("__tobiraNodeId"),
            js_string!(node_id.to_string()),
            Attribute::all(),
        )
        .function(
            NativeFunction::from_fn_ptr(js_dom_attributes_item),
            js_string!("item"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_dom_attributes_get_named_item),
            js_string!("getNamedItem"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_dom_attributes_get_named_item),
            js_string!("namedItem"),
            1,
        )
        .build();
    let handler = ObjectInitializer::new(context)
        .function(
            NativeFunction::from_fn_ptr(js_dom_attributes_get),
            js_string!("get"),
            2,
        )
        .function(
            NativeFunction::from_fn_ptr(js_dom_attributes_set),
            js_string!("set"),
            3,
        )
        .function(
            NativeFunction::from_fn_ptr(js_dom_attributes_has),
            js_string!("has"),
            2,
        )
        .function(
            NativeFunction::from_fn_ptr(js_dom_attributes_own_keys),
            js_string!("ownKeys"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_dom_attributes_get_own_property_descriptor),
            js_string!("getOwnPropertyDescriptor"),
            2,
        )
        .build();

    let global = context.global_object();
    let target_key = js_string!("__tobiraAttributesTarget");
    let handler_key = js_string!("__tobiraAttributesHandler");
    let proxy = (|| -> JsResult<JsObject> {
        global.set(target_key.clone(), target.clone(), true, context)?;
        global.set(handler_key.clone(), handler.clone(), true, context)?;
        let proxy_value = context.eval(Source::from_bytes(
            "new Proxy(globalThis.__tobiraAttributesTarget, globalThis.__tobiraAttributesHandler);",
        ))?;
        proxy_value.as_object().ok_or_else(|| {
            JsNativeError::typ()
                .with_message("attributes proxy bootstrap did not return an object")
                .into()
        })
    })();

    let _ = global.delete_property_or_throw(target_key, context);
    let _ = global.delete_property_or_throw(handler_key, context);

    match proxy {
        Ok(proxy) => {
            store_dom_attributes_object(context, node_id, &proxy);
            proxy
        }
        Err(_) => target,
    }
}

const COMPUTED_STYLE_PROPERTIES: &[(&str, &str)] = &[
    ("display", "display"),
    ("position", "position"),
    ("visibility", "visibility"),
    ("color", "color"),
    ("backgroundColor", "background-color"),
    ("fontSize", "font-size"),
    ("fontWeight", "font-weight"),
    ("fontFamily", "font-family"),
    ("fontStyle", "font-style"),
    ("textDecoration", "text-decoration"),
    ("textTransform", "text-transform"),
    ("textIndent", "text-indent"),
    ("letterSpacing", "letter-spacing"),
    ("lineHeight", "line-height"),
    ("textAlign", "text-align"),
    ("whiteSpace", "white-space"),
    ("pointerEvents", "pointer-events"),
    ("opacity", "opacity"),
    ("overflow", "overflow"),
    ("width", "width"),
    ("height", "height"),
    ("maxWidth", "max-width"),
    ("minWidth", "min-width"),
    ("maxHeight", "max-height"),
    ("minHeight", "min-height"),
    ("verticalAlign", "vertical-align"),
    ("marginTop", "margin-top"),
    ("marginRight", "margin-right"),
    ("marginBottom", "margin-bottom"),
    ("marginLeft", "margin-left"),
    ("margin", "margin"),
    ("paddingTop", "padding-top"),
    ("paddingRight", "padding-right"),
    ("paddingBottom", "padding-bottom"),
    ("paddingLeft", "padding-left"),
    ("padding", "padding"),
    ("borderTopWidth", "border-top-width"),
    ("borderRightWidth", "border-right-width"),
    ("borderBottomWidth", "border-bottom-width"),
    ("borderLeftWidth", "border-left-width"),
    ("borderWidth", "border-width"),
    ("borderTopStyle", "border-top-style"),
    ("borderRightStyle", "border-right-style"),
    ("borderBottomStyle", "border-bottom-style"),
    ("borderLeftStyle", "border-left-style"),
    ("borderStyle", "border-style"),
    ("borderTopColor", "border-top-color"),
    ("borderRightColor", "border-right-color"),
    ("borderBottomColor", "border-bottom-color"),
    ("borderLeftColor", "border-left-color"),
    ("borderColor", "border-color"),
];

fn build_computed_style_object(
    context: &mut Context,
    node_id: usize,
) -> boa_engine::object::JsObject {
    let object = ObjectInitializer::with_native_data(ComputedStyleHandle { node_id }, context)
        .function(
            NativeFunction::from_fn_ptr(js_computed_style_get_property_value),
            js_string!("getPropertyValue"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_computed_style_get_property_priority),
            js_string!("getPropertyPriority"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_computed_style_item),
            js_string!("item"),
            1,
        )
        .build();

    let mut property_count = 0;
    for (js_name, css_name) in COMPUTED_STYLE_PROPERTIES {
        let value = computed_style_property_value(context, node_id, css_name);
        let _ = object.set(
            js_string!(*js_name),
            js_string!(value.clone()),
            true,
            context,
        );
        if js_name != css_name {
            let _ = object.set(js_string!(*css_name), js_string!(value), true, context);
        }
        property_count += 1;
    }

    let css_text = COMPUTED_STYLE_PROPERTIES
        .iter()
        .map(|(_, css_name)| {
            let value = computed_style_property_value(context, node_id, css_name);
            format!("{css_name}: {value}")
        })
        .collect::<Vec<_>>()
        .join("; ");
    let _ = object.set(js_string!("cssText"), js_string!(css_text), true, context);
    let _ = object.set(
        js_string!("length"),
        JsValue::new(property_count as u32),
        true,
        context,
    );

    object
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
    if let Some(detail) = &request.detail {
        let _ = event.set(
            js_string!("detail"),
            js_string!(detail.as_str()),
            true,
            context,
        );
    }
    if let Some(data) = &request.data {
        let _ = event.set(js_string!("data"), js_string!(data.as_str()), true, context);
    }
    if let Some(input_type) = &request.input_type {
        let _ = event.set(
            js_string!("inputType"),
            js_string!(input_type.as_str()),
            true,
            context,
        );
    }
    if let Some(client_x) = request.client_x {
        let _ = event.set(js_string!("clientX"), JsValue::new(client_x), true, context);
    }
    if let Some(client_y) = request.client_y {
        let _ = event.set(js_string!("clientY"), JsValue::new(client_y), true, context);
    }
    if let Some(button) = request.button {
        let _ = event.set(js_string!("button"), JsValue::new(button), true, context);
    }
    if let Some(buttons) = request.buttons {
        let _ = event.set(js_string!("buttons"), JsValue::new(buttons), true, context);
    }
    let _ = event.set(
        js_string!("isComposing"),
        JsValue::new(request.is_composing),
        true,
        context,
    );
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
        if changed {
            sync_current_history_entry_scroll(&mut state);
        }
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
        if changed {
            sync_current_history_entry_scroll(&mut state);
        }
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
        if changed {
            sync_current_history_entry_scroll(&mut state);
        }
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

fn document_tag_node_id(context: &mut Context, tag_name: &str) -> Option<usize> {
    let host = context.get_data::<JavaScriptHostData>()?;
    let state = host.state.borrow();
    state.dom.find_first_tag(state.dom.document_id, tag_name)
}

fn js_document_get_body(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(node_id) = document_tag_node_id(context, "body") else {
        return Ok(JsValue::null());
    };
    Ok(JsValue::from(build_dom_node_object(context, node_id)))
}

fn js_document_get_head(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(node_id) = document_tag_node_id(context, "head") else {
        return Ok(JsValue::null());
    };
    Ok(JsValue::from(build_dom_node_object(context, node_id)))
}

fn js_document_get_document_element(
    _: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(node_id) = document_tag_node_id(context, "html") else {
        return Ok(JsValue::null());
    };
    Ok(JsValue::from(build_dom_node_object(context, node_id)))
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
    let get_first_element_child =
        NativeFunction::from_fn_ptr(js_dom_get_first_element_child).to_js_function(context.realm());
    let get_last_element_child =
        NativeFunction::from_fn_ptr(js_dom_get_last_element_child).to_js_function(context.realm());
    let get_previous_element_sibling =
        NativeFunction::from_fn_ptr(js_dom_get_previous_element_sibling)
            .to_js_function(context.realm());
    let get_next_element_sibling = NativeFunction::from_fn_ptr(js_dom_get_next_element_sibling)
        .to_js_function(context.realm());
    let get_text_content =
        NativeFunction::from_fn_ptr(js_dom_get_text_content).to_js_function(context.realm());
    let set_text_content =
        NativeFunction::from_fn_ptr(js_dom_set_text_content).to_js_function(context.realm());
    let get_inner_html =
        NativeFunction::from_fn_ptr(js_dom_get_inner_html).to_js_function(context.realm());
    let set_inner_html =
        NativeFunction::from_fn_ptr(js_dom_set_inner_html).to_js_function(context.realm());
    let get_outer_html =
        NativeFunction::from_fn_ptr(js_dom_get_outer_html).to_js_function(context.realm());
    let set_outer_html =
        NativeFunction::from_fn_ptr(js_dom_set_outer_html).to_js_function(context.realm());
    let get_id = NativeFunction::from_fn_ptr(js_dom_get_id).to_js_function(context.realm());
    let set_id = NativeFunction::from_fn_ptr(js_dom_set_id).to_js_function(context.realm());
    let get_class_name =
        NativeFunction::from_fn_ptr(js_dom_get_class_name).to_js_function(context.realm());
    let set_class_name =
        NativeFunction::from_fn_ptr(js_dom_set_class_name).to_js_function(context.realm());
    let get_attributes =
        NativeFunction::from_fn_ptr(js_dom_get_attributes).to_js_function(context.realm());
    let get_dataset =
        NativeFunction::from_fn_ptr(js_dom_get_dataset).to_js_function(context.realm());
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
    let get_node_name =
        NativeFunction::from_fn_ptr(js_dom_get_node_name).to_js_function(context.realm());
    let get_node_type =
        NativeFunction::from_fn_ptr(js_dom_get_node_type).to_js_function(context.realm());
    let get_node_value =
        NativeFunction::from_fn_ptr(js_dom_get_node_value).to_js_function(context.realm());
    let set_node_value =
        NativeFunction::from_fn_ptr(js_dom_set_node_value).to_js_function(context.realm());
    let get_first_child =
        NativeFunction::from_fn_ptr(js_dom_get_first_child).to_js_function(context.realm());
    let get_last_child =
        NativeFunction::from_fn_ptr(js_dom_get_last_child).to_js_function(context.realm());
    let get_previous_sibling =
        NativeFunction::from_fn_ptr(js_dom_get_previous_sibling).to_js_function(context.realm());
    let get_next_sibling =
        NativeFunction::from_fn_ptr(js_dom_get_next_sibling).to_js_function(context.realm());
    let get_is_connected =
        NativeFunction::from_fn_ptr(js_dom_get_is_connected).to_js_function(context.realm());
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
            NativeFunction::from_fn_ptr(js_dom_matches),
            js_string!("matches"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_dom_closest),
            js_string!("closest"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_dom_contains),
            js_string!("contains"),
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
            NativeFunction::from_fn_ptr(js_dom_replace_child),
            js_string!("replaceChild"),
            2,
        )
        .function(
            NativeFunction::from_fn_ptr(js_dom_remove_child),
            js_string!("removeChild"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_dom_append),
            js_string!("append"),
            0,
        )
        .function(
            NativeFunction::from_fn_ptr(js_dom_prepend),
            js_string!("prepend"),
            0,
        )
        .function(
            NativeFunction::from_fn_ptr(js_dom_before),
            js_string!("before"),
            0,
        )
        .function(
            NativeFunction::from_fn_ptr(js_dom_after),
            js_string!("after"),
            0,
        )
        .function(
            NativeFunction::from_fn_ptr(js_dom_replace_with),
            js_string!("replaceWith"),
            0,
        )
        .function(
            NativeFunction::from_fn_ptr(js_dom_replace_children),
            js_string!("replaceChildren"),
            0,
        )
        .function(
            NativeFunction::from_fn_ptr(js_dom_clone_node),
            js_string!("cloneNode"),
            1,
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
            NativeFunction::from_fn_ptr(js_dom_has_attribute),
            js_string!("hasAttribute"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_dom_has_attributes),
            js_string!("hasAttributes"),
            0,
        )
        .function(
            NativeFunction::from_fn_ptr(js_dom_get_attribute_names),
            js_string!("getAttributeNames"),
            0,
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
            NativeFunction::from_fn_ptr(js_dom_toggle_attribute),
            js_string!("toggleAttribute"),
            2,
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
            NativeFunction::from_fn_ptr(js_dom_insert_adjacent_html),
            js_string!("insertAdjacentHTML"),
            2,
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
            js_string!("firstElementChild"),
            Some(get_first_element_child),
            None,
            Attribute::all(),
        )
        .accessor(
            js_string!("lastElementChild"),
            Some(get_last_element_child),
            None,
            Attribute::all(),
        )
        .accessor(
            js_string!("previousElementSibling"),
            Some(get_previous_element_sibling),
            None,
            Attribute::all(),
        )
        .accessor(
            js_string!("nextElementSibling"),
            Some(get_next_element_sibling),
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
            js_string!("outerHTML"),
            Some(get_outer_html),
            Some(set_outer_html),
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
            js_string!("attributes"),
            Some(get_attributes),
            None,
            Attribute::all(),
        )
        .accessor(
            js_string!("dataset"),
            Some(get_dataset),
            None,
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
            Some(get_node_name),
            None,
            Attribute::all(),
        )
        .accessor(
            js_string!("nodeType"),
            Some(get_node_type),
            None,
            Attribute::all(),
        )
        .accessor(
            js_string!("nodeValue"),
            Some(get_node_value),
            Some(set_node_value),
            Attribute::all(),
        )
        .accessor(
            js_string!("firstChild"),
            Some(get_first_child),
            None,
            Attribute::all(),
        )
        .accessor(
            js_string!("lastChild"),
            Some(get_last_child),
            None,
            Attribute::all(),
        )
        .accessor(
            js_string!("previousSibling"),
            Some(get_previous_sibling),
            None,
            Attribute::all(),
        )
        .accessor(
            js_string!("nextSibling"),
            Some(get_next_sibling),
            None,
            Attribute::all(),
        )
        .accessor(
            js_string!("isConnected"),
            Some(get_is_connected),
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
    let get_value_getter =
        NativeFunction::from_fn_ptr(js_dom_class_list_get_value).to_js_function(context.realm());
    let set_value_setter =
        NativeFunction::from_fn_ptr(js_dom_class_list_set_value).to_js_function(context.realm());
    let get_length_getter =
        NativeFunction::from_fn_ptr(js_dom_class_list_get_length).to_js_function(context.realm());
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
        .function(
            NativeFunction::from_fn_ptr(js_dom_class_list_replace),
            js_string!("replace"),
            2,
        )
        .function(
            NativeFunction::from_fn_ptr(js_dom_class_list_item),
            js_string!("item"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_dom_class_list_to_string),
            js_string!("toString"),
            0,
        )
        .accessor(
            js_string!("value"),
            Some(get_value_getter),
            Some(set_value_setter),
            Attribute::all(),
        )
        .accessor(
            js_string!("length"),
            Some(get_length_getter),
            None,
            Attribute::all(),
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
        .function(
            NativeFunction::from_fn_ptr(js_response_headers_entries),
            js_string!("entries"),
            0,
        )
        .function(
            NativeFunction::from_fn_ptr(js_response_headers_keys),
            js_string!("keys"),
            0,
        )
        .function(
            NativeFunction::from_fn_ptr(js_response_headers_values),
            js_string!("values"),
            0,
        )
        .function(
            NativeFunction::from_fn_ptr(js_response_headers_for_each),
            js_string!("forEach"),
            2,
        )
        .function(
            NativeFunction::from_fn_ptr(js_response_headers_to_string),
            js_string!("toString"),
            0,
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
    .function(
        NativeFunction::from_fn_ptr(js_xhr_get_all_response_headers),
        js_string!("getAllResponseHeaders"),
        0,
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
        js_string!("onloadend"),
        JsValue::undefined(),
        Attribute::all(),
    )
    .property(
        js_string!("onerror"),
        JsValue::undefined(),
        Attribute::all(),
    )
    .property(
        js_string!("onabort"),
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
    navigate_location_href(&href, true, context);
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
    let state = args.first().cloned();
    record_soft_navigation_href(&resolved, false, state, false, context);
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
    let state = args.first().cloned();
    record_soft_navigation_href(&resolved, true, state, false, context);
    Ok(JsValue::undefined())
}

fn js_history_back(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let previous_href = current_location_url(context).map(|url| url.to_string());
    if let Some(target) = navigate_history(context, -1) {
        apply_soft_navigation_entry(&target, context);
        dispatch_history_popstate_event(context, target.state.clone());
        dispatch_hashchange_if_needed(previous_href.as_deref(), &target.href, context);
    }
    Ok(JsValue::undefined())
}

fn js_history_forward(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let previous_href = current_location_url(context).map(|url| url.to_string());
    if let Some(target) = navigate_history(context, 1) {
        apply_soft_navigation_entry(&target, context);
        dispatch_history_popstate_event(context, target.state.clone());
        dispatch_hashchange_if_needed(previous_href.as_deref(), &target.href, context);
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

fn js_history_state(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    Ok(current_history_entry_state(context).unwrap_or_else(JsValue::null))
}

fn record_soft_navigation_href(
    href: &str,
    replace_current: bool,
    state_override: Option<JsValue>,
    fire_hashchange: bool,
    context: &mut Context,
) {
    let previous_href = current_location_url(context).map(|url| url.to_string());
    if let Some(host) = context.get_data::<JavaScriptHostData>() {
        let mut state = host.state.borrow_mut();
        let current_scroll_y = state.scroll_y;
        let entry_state = state_override.unwrap_or_else(|| {
            state
                .history_entries
                .get(state.history_index)
                .map(|entry| entry.state.clone())
                .unwrap_or_else(JsValue::null)
        });
        if replace_current {
            if state.history_entries.is_empty() {
                state.history_entries.push(HistoryEntry {
                    href: href.to_string(),
                    scroll_y: current_scroll_y,
                    state: entry_state,
                });
                state.history_index = 0;
            } else {
                let index = state.history_index;
                if let Some(entry) = state.history_entries.get_mut(index) {
                    entry.href = href.to_string();
                    entry.scroll_y = current_scroll_y;
                    entry.state = entry_state;
                }
            }
        } else {
            let next_index = state.history_index.saturating_add(1);
            state.history_entries.truncate(next_index);
            state.history_entries.push(HistoryEntry {
                href: href.to_string(),
                scroll_y: current_scroll_y,
                state: entry_state,
            });
            state.history_index = state.history_entries.len().saturating_sub(1);
        }
    }
    apply_soft_navigation_href_resolved(href, context);
    if fire_hashchange {
        dispatch_hashchange_if_needed(previous_href.as_deref(), href, context);
    }
}

fn navigate_history(context: &mut Context, delta: isize) -> Option<HistoryEntry> {
    let host = context.get_data::<JavaScriptHostData>()?;
    let mut state = host.state.borrow_mut();
    let next = state.history_index as isize + delta;
    if next < 0 || next as usize >= state.history_entries.len() {
        return None;
    }
    state.history_index = next as usize;
    state.history_entries.get(state.history_index).cloned()
}

fn current_history_entry_state(context: &mut Context) -> Option<JsValue> {
    let host = context.get_data::<JavaScriptHostData>()?;
    let state = host.state.borrow();
    state
        .history_entries
        .get(state.history_index)
        .map(|entry| entry.state.clone())
}

fn same_document_href_base(href: &str) -> &str {
    href.split('#').next().unwrap_or(href)
}

fn set_location_href(href: &str, context: &mut Context) {
    navigate_location_href(href, false, context);
}

fn navigate_location_href(href: &str, replace_current: bool, context: &mut Context) {
    let previous_href = current_location_url(context).map(|url| url.to_string());
    let resolved = current_document_url(context)
        .and_then(|url| url.resolve(href).ok())
        .map(|url| url.to_string())
        .or_else(|| Url::parse(href).ok().map(|url| url.to_string()))
        .unwrap_or_else(|| href.to_string());
    let same_document = previous_href
        .as_deref()
        .map(|previous| same_document_href_base(previous) == same_document_href_base(&resolved))
        .unwrap_or(false);
    if same_document {
        record_soft_navigation_href(&resolved, replace_current, None, true, context);
        return;
    }
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
    record_soft_navigation_href(&resolved, false, None, true, context);
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

fn apply_soft_navigation_entry(entry: &HistoryEntry, context: &mut Context) {
    if let Some(host) = context.get_data::<JavaScriptHostData>() {
        let mut state = host.state.borrow_mut();
        state.location_href = entry.href.clone();
        state.scroll_y = entry.scroll_y;
        if let Ok(url) = Url::parse(&entry.href) {
            state.document_url = url;
        }
        state.soft_navigation_target = Some(entry.href.clone());
    }
}

fn dispatch_global_event_object(
    context: &mut Context,
    event_type: &str,
    bubbles: bool,
    cancelable: bool,
    event: &boa_engine::object::JsObject,
) -> JsResult<bool> {
    let target = context.global_object();
    dispatch_listeners_on_target(
        &target,
        &event_type.to_ascii_lowercase(),
        event,
        true,
        EventDispatchPhase::AtTarget,
        context,
    )?;
    if !event_flag_value(event, "immediatePropagationStopped", context) {
        dispatch_listeners_on_target(
            &target,
            &event_type.to_ascii_lowercase(),
            event,
            false,
            EventDispatchPhase::AtTarget,
            context,
        )?;
    }
    let _ = (bubbles, cancelable);
    Ok(event_flag_value(event, "defaultPrevented", context))
}

fn dispatch_history_popstate_event(context: &mut Context, state: JsValue) {
    let target = context.global_object();
    let request = DomEventRequest {
        target_node_id: 0,
        event_type: "popstate".to_string(),
        bubbles: false,
        cancelable: false,
        ..Default::default()
    };
    let event = build_dom_event_object(context, &request, &target);
    let _ = event.set(js_string!("state"), state, true, context);
    let _ = dispatch_global_event_object(context, "popstate", false, false, &event);
}

fn dispatch_hashchange_if_needed(
    previous_href: Option<&str>,
    next_href: &str,
    context: &mut Context,
) {
    let Some(previous_href) = previous_href else {
        return;
    };
    if same_document_href_base(previous_href) != same_document_href_base(next_href) {
        return;
    }
    if previous_href == next_href {
        return;
    }
    let target = context.global_object();
    let request = DomEventRequest {
        target_node_id: 0,
        event_type: "hashchange".to_string(),
        bubbles: false,
        cancelable: false,
        ..Default::default()
    };
    let event = build_dom_event_object(context, &request, &target);
    let _ = event.set(
        js_string!("oldURL"),
        js_string!(previous_href),
        true,
        context,
    );
    let _ = event.set(js_string!("newURL"), js_string!(next_href), true, context);
    let _ = dispatch_global_event_object(context, "hashchange", false, false, &event);
}

fn sync_current_history_entry_scroll(state: &mut JavaScriptState) {
    if let Some(entry) = state.history_entries.get_mut(state.history_index) {
        entry.scroll_y = state.scroll_y;
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
    let options = args.get(1).cloned().unwrap_or_else(JsValue::undefined);
    if let Some(options_object) = options.as_object()
        && let Ok(signal_value) = options_object.get(js_string!("signal"), context)
        && let Some(signal_object) = signal_value.as_object()
    {
        if let Ok(aborted_value) = signal_object.get(js_string!("aborted"), context)
            && aborted_value.to_boolean()
        {
            return Ok(JsValue::from(JsPromise::reject(
                JsNativeError::error().with_message("fetch aborted"),
                context,
            )));
        }
    }
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

fn response_headers_entries(handle: &ResponseHeadersHandle) -> Vec<(String, String)> {
    handle
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

fn js_response_headers_entries(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(object) = this.as_object() else {
        return Ok(JsValue::from(JsArray::new(context)));
    };
    let Some(handle) = object.downcast_ref::<ResponseHeadersHandle>() else {
        return Ok(JsValue::from(JsArray::new(context)));
    };
    let mut entries = Vec::new();
    for (name, value) in response_headers_entries(&handle) {
        let pair = JsArray::from_iter(
            [
                JsValue::from(js_string!(name)),
                JsValue::from(js_string!(value)),
            ],
            context,
        );
        entries.push(JsValue::from(pair));
    }
    Ok(JsValue::from(JsArray::from_iter(entries, context)))
}

fn js_response_headers_keys(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(object) = this.as_object() else {
        return Ok(JsValue::from(JsArray::new(context)));
    };
    let Some(handle) = object.downcast_ref::<ResponseHeadersHandle>() else {
        return Ok(JsValue::from(JsArray::new(context)));
    };
    let keys = response_headers_entries(&handle)
        .into_iter()
        .map(|(name, _)| JsValue::from(js_string!(name)));
    Ok(JsValue::from(JsArray::from_iter(keys, context)))
}

fn js_response_headers_values(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(object) = this.as_object() else {
        return Ok(JsValue::from(JsArray::new(context)));
    };
    let Some(handle) = object.downcast_ref::<ResponseHeadersHandle>() else {
        return Ok(JsValue::from(JsArray::new(context)));
    };
    let values = response_headers_entries(&handle)
        .into_iter()
        .map(|(_, value)| JsValue::from(js_string!(value)));
    Ok(JsValue::from(JsArray::from_iter(values, context)))
}

fn js_response_headers_for_each(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(callback) = args.first() else {
        return Ok(JsValue::undefined());
    };
    let this_arg = args.get(1).cloned().unwrap_or_else(JsValue::undefined);
    let Some(object) = this.as_object() else {
        return Ok(JsValue::undefined());
    };
    let Some(handle) = object.downcast_ref::<ResponseHeadersHandle>() else {
        return Ok(JsValue::undefined());
    };
    for (name, value) in response_headers_entries(&handle) {
        let _ = call_js_callback_with_this(
            callback,
            &this_arg,
            &[
                JsValue::from(js_string!(value)),
                JsValue::from(js_string!(name)),
                JsValue::from(object.clone()),
            ],
            context,
        )?;
    }
    Ok(JsValue::undefined())
}

fn js_response_headers_to_string(
    this: &JsValue,
    _: &[JsValue],
    _context: &mut Context,
) -> JsResult<JsValue> {
    let Some(object) = this.as_object() else {
        return Ok(JsValue::from(js_string!("")));
    };
    let Some(handle) = object.downcast_ref::<ResponseHeadersHandle>() else {
        return Ok(JsValue::from(js_string!("")));
    };
    let text = response_headers_entries(&handle)
        .into_iter()
        .map(|(name, value)| format!("{name}: {value}"))
        .collect::<Vec<_>>()
        .join("\r\n");
    Ok(JsValue::from(js_string!(text)))
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
        state.response_headers.clear();
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
                state.response_headers = response
                    .headers
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone()))
                    .collect();
            }
            trigger_xhr_handler(&object, "onreadystatechange", context)?;
            trigger_xhr_handler(&object, "onload", context)?;
            trigger_xhr_handler(&object, "onloadend", context)?;
        }
        Err(error) => {
            {
                let mut state = handle.state.borrow_mut();
                state.ready_state = 4;
                state.status = 0;
                state.status_text = error.to_string();
                state.response_text.clear();
                state.response_url.clear();
                state.response_headers.clear();
            }
            trigger_xhr_handler(&object, "onreadystatechange", context)?;
            trigger_xhr_handler(&object, "onerror", context)?;
            trigger_xhr_handler(&object, "onloadend", context)?;
        }
    }

    Ok(JsValue::undefined())
}

fn js_xhr_abort(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    if let Some(object) = this.as_object()
        && let Some(handle) = object.downcast_ref::<XmlHttpRequestHandle>()
    {
        let mut state = handle.state.borrow_mut();
        state.ready_state = 0;
        state.status = 0;
        state.status_text.clear();
        state.response_text.clear();
        state.response_url.clear();
        state.response_headers.clear();
    }
    if let Some(object) = this.as_object() {
        let _ = trigger_xhr_handler(&object, "onabort", context);
        let _ = trigger_xhr_handler(&object, "onloadend", context);
    }
    Ok(JsValue::undefined())
}

fn js_xhr_get_response_header(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let name = js_value_to_string(args.first().unwrap_or(&JsValue::undefined()), context)?
        .to_ascii_lowercase();
    let value = xhr_state_value(this, |state| state.response_headers.get(&name).cloned())
        .unwrap_or_default();
    Ok(value
        .map(|value| JsValue::from(js_string!(value)))
        .unwrap_or_else(JsValue::null))
}

fn js_xhr_get_all_response_headers(
    this: &JsValue,
    _: &[JsValue],
    _context: &mut Context,
) -> JsResult<JsValue> {
    let headers = xhr_state_value(this, |state| state.response_headers.clone()).unwrap_or_default();
    let text = headers
        .into_iter()
        .map(|(name, value)| format!("{name}: {value}"))
        .collect::<Vec<_>>()
        .join("\r\n");
    Ok(JsValue::from(js_string!(text)))
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

fn queue_pending_task(
    context: &mut Context,
    kind: PendingTaskKind,
    action: PendingTaskAction,
) -> usize {
    let Some(host) = context.get_data::<JavaScriptHostData>() else {
        return 0;
    };
    let mut state = host.state.borrow_mut();
    let handle = state.next_task_handle;
    state.next_task_handle = state.next_task_handle.checked_add(1).unwrap_or(1);
    state.pending_tasks.push_back(PendingTask {
        handle,
        kind,
        action,
    });
    handle
}

fn clear_pending_task(context: &mut Context, handle: usize) {
    if let Some(host) = context.get_data::<JavaScriptHostData>() {
        host.state
            .borrow_mut()
            .pending_tasks
            .retain(|task| task.handle != handle);
    }
}

fn pending_task_handle_from_value(
    value: Option<&JsValue>,
    context: &mut Context,
) -> JsResult<Option<usize>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let number = value.to_number(context)?;
    if !number.is_finite() || number < 0.0 {
        return Ok(None);
    }
    Ok(Some(number.round() as usize))
}

fn js_request_animation_frame(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(callback) = args.first().cloned() else {
        return Ok(JsValue::new(0));
    };
    if callback.as_object().is_none() {
        return Ok(JsValue::new(0));
    }
    let handle = queue_pending_task(
        context,
        PendingTaskKind::AnimationFrame,
        PendingTaskAction::Callback {
            callback,
            args: Vec::new(),
        },
    );
    Ok(JsValue::new(handle as i32))
}

fn js_queue_microtask(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(callback) = args.first().cloned() else {
        return Ok(JsValue::undefined());
    };
    if callback.as_object().is_none() {
        return Ok(JsValue::undefined());
    }
    let _ = queue_pending_task(
        context,
        PendingTaskKind::Microtask,
        PendingTaskAction::Callback {
            callback,
            args: Vec::new(),
        },
    );
    Ok(JsValue::undefined())
}

fn js_set_timeout(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    schedule_timer(false, args, context)
}

fn js_set_interval(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    schedule_timer(true, args, context)
}

fn schedule_timer(repeat: bool, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(callback) = args.first().cloned() else {
        return Ok(JsValue::new(0));
    };
    let action = if callback.as_object().is_some() {
        let callback_args = if args.len() > 2 {
            args[2..].to_vec()
        } else {
            Vec::new()
        };
        PendingTaskAction::Callback {
            callback,
            args: callback_args,
        }
    } else if callback.is_string() {
        PendingTaskAction::Script(js_value_to_string(&callback, context)?)
    } else {
        return Ok(JsValue::new(0));
    };
    let handle = queue_pending_task(context, PendingTaskKind::Timeout { repeat }, action);

    Ok(JsValue::new(handle as i32))
}

fn js_clear_timeout(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    if let Some(handle) = pending_task_handle_from_value(args.first(), context)? {
        clear_pending_task(context, handle);
    }
    Ok(JsValue::undefined())
}

fn js_clear_animation_frame(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    js_clear_timeout(this, args, context)
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
    let (
        event_type,
        bubbles,
        cancelable,
        key,
        code,
        detail,
        data,
        input_type,
        client_x,
        client_y,
        button,
        buttons,
        is_composing,
        repeat,
        alt_key,
        ctrl_key,
        shift_key,
        meta_key,
    ) = if let Some(object) = event_arg.as_object() {
        let event_type = js_value_to_string(&object.get(js_string!("type"), context)?, context)?;
        let bubbles = object.get(js_string!("bubbles"), context)?.to_boolean();
        let cancelable = object.get(js_string!("cancelable"), context)?.to_boolean();
        let key = js_optional_string_property(&object, "key", context)?;
        let code = js_optional_string_property(&object, "code", context)?;
        let detail = js_optional_string_property(&object, "detail", context)?;
        let data = js_optional_string_property(&object, "data", context)?;
        let input_type = js_optional_string_property(&object, "inputType", context)?;
        let client_x = js_optional_i32_property(&object, "clientX", context)?;
        let client_y = js_optional_i32_property(&object, "clientY", context)?;
        let button = js_optional_i32_property(&object, "button", context)?;
        let buttons = js_optional_i32_property(&object, "buttons", context)?;
        let is_composing = js_optional_bool_property(&object, "isComposing", context)?;
        let repeat = js_optional_bool_property(&object, "repeat", context)?;
        let alt_key = js_optional_bool_property(&object, "altKey", context)?;
        let ctrl_key = js_optional_bool_property(&object, "ctrlKey", context)?;
        let shift_key = js_optional_bool_property(&object, "shiftKey", context)?;
        let meta_key = js_optional_bool_property(&object, "metaKey", context)?;
        (
            event_type,
            bubbles,
            cancelable,
            key,
            code,
            detail,
            data,
            input_type,
            client_x,
            client_y,
            button,
            buttons,
            is_composing,
            repeat,
            alt_key,
            ctrl_key,
            shift_key,
            meta_key,
        )
    } else {
        let event_type = js_value_to_string(&event_arg, context)?;
        let bubbles = default_event_bubbles(&event_type);
        let cancelable = default_event_cancelable(&event_type);
        (
            event_type, bubbles, cancelable, None, None, None, None, None, None, None, None, None,
            false, false, false, false, false, false,
        )
    };
    let request = DomEventRequest {
        target_node_id: target
            .downcast_ref::<DomNodeHandle>()
            .map(|handle| handle.node_id)
            .unwrap_or(0),
        event_type,
        bubbles,
        cancelable,
        key,
        code,
        detail,
        data,
        input_type,
        client_x,
        client_y,
        button,
        buttons,
        is_composing,
        repeat,
        alt_key,
        ctrl_key,
        shift_key,
        meta_key,
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

fn js_window_get_computed_style(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(node_id) = node_id_argument(args.first()) else {
        return Err(JsNativeError::typ()
            .with_message("getComputedStyle requires an Element")
            .into());
    };

    if let Some(pseudo) = args.get(1) {
        if !pseudo.is_undefined() && !pseudo.is_null() {
            let pseudo_text = js_value_to_string(pseudo, context)?;
            if !pseudo_text.trim().is_empty() {
                return Err(JsNativeError::typ()
                    .with_message("pseudo-element computed styles are unsupported")
                    .into());
            }
        }
    }

    Ok(JsValue::from(build_computed_style_object(context, node_id)))
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

fn js_value_to_dom_node_id(value: &JsValue, context: &mut Context) -> JsResult<usize> {
    if let Some(node_id) = node_id_argument(Some(value)) {
        return Ok(node_id);
    }

    let text = js_value_to_string(value, context)?;
    let node_id = context
        .get_data::<JavaScriptHostData>()
        .map(|host| host.state.borrow_mut().dom.create_text_node(&text))
        .unwrap_or(0);
    Ok(node_id)
}

fn js_values_to_dom_node_ids(args: &[JsValue], context: &mut Context) -> JsResult<Vec<usize>> {
    args.iter()
        .map(|value| js_value_to_dom_node_id(value, context))
        .collect()
}

fn js_dom_get_node_name(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node_id = this_node_id(this).unwrap_or(0);
    let node_name = context
        .get_data::<JavaScriptHostData>()
        .and_then(|host| host.state.borrow().dom.node_name(node_id))
        .unwrap_or_default();
    Ok(JsValue::from(js_string!(node_name)))
}

fn js_dom_get_node_type(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node_id = this_node_id(this).unwrap_or(0);
    let node_type = context
        .get_data::<JavaScriptHostData>()
        .map(|host| host.state.borrow().dom.node_type(node_id))
        .unwrap_or(0);
    Ok(JsValue::new(node_type as i32))
}

fn js_dom_get_node_value(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this).unwrap_or(0);
    let value = context
        .get_data::<JavaScriptHostData>()
        .and_then(|host| host.state.borrow().dom.node_value(node_id));
    Ok(value
        .map(|value| JsValue::from(js_string!(value)))
        .unwrap_or_else(JsValue::null))
}

fn js_dom_set_node_value(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this).unwrap_or(0);
    let value = js_value_to_string(args.first().unwrap_or(&JsValue::undefined()), context)?;
    if let Some(host) = context.get_data::<JavaScriptHostData>() {
        host.state.borrow_mut().dom.set_node_value(node_id, &value);
    }
    Ok(JsValue::undefined())
}

fn js_dom_get_first_child(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this).unwrap_or(0);
    let child_id = context
        .get_data::<JavaScriptHostData>()
        .and_then(|host| host.state.borrow().dom.first_child(node_id));
    Ok(child_id
        .map(|child_id| JsValue::from(build_dom_node_object(context, child_id)))
        .unwrap_or_else(JsValue::null))
}

fn js_dom_get_last_child(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this).unwrap_or(0);
    let child_id = context
        .get_data::<JavaScriptHostData>()
        .and_then(|host| host.state.borrow().dom.last_child(node_id));
    Ok(child_id
        .map(|child_id| JsValue::from(build_dom_node_object(context, child_id)))
        .unwrap_or_else(JsValue::null))
}

fn js_dom_get_previous_sibling(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this).unwrap_or(0);
    let sibling_id = context
        .get_data::<JavaScriptHostData>()
        .and_then(|host| host.state.borrow().dom.previous_sibling(node_id));
    Ok(sibling_id
        .map(|sibling_id| JsValue::from(build_dom_node_object(context, sibling_id)))
        .unwrap_or_else(JsValue::null))
}

fn js_dom_get_next_sibling(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this).unwrap_or(0);
    let sibling_id = context
        .get_data::<JavaScriptHostData>()
        .and_then(|host| host.state.borrow().dom.next_sibling(node_id));
    Ok(sibling_id
        .map(|sibling_id| JsValue::from(build_dom_node_object(context, sibling_id)))
        .unwrap_or_else(JsValue::null))
}

fn js_dom_get_is_connected(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this).unwrap_or(usize::MAX);
    let is_connected = context
        .get_data::<JavaScriptHostData>()
        .map(|host| host.state.borrow().dom.is_connected(node_id))
        .unwrap_or(false);
    Ok(JsValue::new(is_connected))
}

fn js_dom_clone_node(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(node_id) = this_node_id(this) else {
        return Ok(JsValue::undefined());
    };
    let deep = args.first().map(JsValue::to_boolean).unwrap_or(false);
    let cloned = context
        .get_data::<JavaScriptHostData>()
        .and_then(|host| host.state.borrow_mut().dom.clone_node(node_id, deep));
    Ok(cloned
        .map(|node_id| JsValue::from(build_dom_node_object(context, node_id)))
        .unwrap_or_else(JsValue::undefined))
}

fn js_dom_replace_child(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(parent_id) = this_node_id(this) else {
        return Ok(JsValue::undefined());
    };
    let Some(new_child_id) = node_id_argument(args.first()) else {
        return Ok(JsValue::undefined());
    };
    let Some(old_child_id) = node_id_argument(args.get(1)) else {
        return Ok(JsValue::undefined());
    };
    let old_children = snapshot_dom_children(context, parent_id);
    let replaced = context.get_data::<JavaScriptHostData>().and_then(|host| {
        host.state
            .borrow_mut()
            .dom
            .replace_child(parent_id, new_child_id, old_child_id)
    });
    let new_children = snapshot_dom_children(context, parent_id);
    record_dom_child_list_mutation(context, parent_id, &old_children, &new_children);
    flush_mutation_observers(context);
    Ok(replaced
        .map(|node_id| JsValue::from(build_dom_node_object(context, node_id)))
        .unwrap_or_else(JsValue::undefined))
}

fn js_dom_remove_child(
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
    let old_children = snapshot_dom_children(context, parent_id);
    let removed = context.get_data::<JavaScriptHostData>().and_then(|host| {
        host.state
            .borrow_mut()
            .dom
            .remove_child(parent_id, child_id)
    });
    let new_children = snapshot_dom_children(context, parent_id);
    record_dom_child_list_mutation(context, parent_id, &old_children, &new_children);
    flush_mutation_observers(context);
    Ok(removed
        .map(|node_id| JsValue::from(build_dom_node_object(context, node_id)))
        .unwrap_or_else(JsValue::undefined))
}

fn js_dom_append(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(parent_id) = this_node_id(this) else {
        return Ok(JsValue::undefined());
    };
    let node_ids = js_values_to_dom_node_ids(args, context)?;
    let old_children = snapshot_dom_children(context, parent_id);
    if let Some(host) = context.get_data::<JavaScriptHostData>() {
        let mut state = host.state.borrow_mut();
        for node_id in node_ids {
            state.dom.append_child(parent_id, node_id);
        }
        drop(state);
    }
    let new_children = snapshot_dom_children(context, parent_id);
    record_dom_child_list_mutation(context, parent_id, &old_children, &new_children);
    flush_mutation_observers(context);
    Ok(JsValue::undefined())
}

fn js_dom_prepend(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(parent_id) = this_node_id(this) else {
        return Ok(JsValue::undefined());
    };
    let mut node_ids = js_values_to_dom_node_ids(args, context)?;
    node_ids.reverse();
    let old_children = snapshot_dom_children(context, parent_id);
    if let Some(host) = context.get_data::<JavaScriptHostData>() {
        let mut state = host.state.borrow_mut();
        let mut before_id = state.dom.first_child(parent_id);
        for node_id in node_ids {
            state.dom.insert_before(parent_id, node_id, before_id);
            before_id = Some(node_id);
        }
        drop(state);
    }
    let new_children = snapshot_dom_children(context, parent_id);
    record_dom_child_list_mutation(context, parent_id, &old_children, &new_children);
    flush_mutation_observers(context);
    Ok(JsValue::undefined())
}

fn js_dom_before(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(target_id) = this_node_id(this) else {
        return Ok(JsValue::undefined());
    };
    let Some(parent_id) = context.get_data::<JavaScriptHostData>().and_then(|host| {
        host.state
            .borrow()
            .dom
            .node(target_id)
            .and_then(|node| node.parent)
    }) else {
        return Ok(JsValue::undefined());
    };
    let mut node_ids = js_values_to_dom_node_ids(args, context)?;
    node_ids.reverse();
    let old_children = snapshot_dom_children(context, parent_id);
    if let Some(host) = context.get_data::<JavaScriptHostData>() {
        let mut state = host.state.borrow_mut();
        let mut before_id = Some(target_id);
        for node_id in node_ids {
            state.dom.insert_before(parent_id, node_id, before_id);
            before_id = Some(node_id);
        }
        drop(state);
    }
    let new_children = snapshot_dom_children(context, parent_id);
    record_dom_child_list_mutation(context, parent_id, &old_children, &new_children);
    flush_mutation_observers(context);
    Ok(JsValue::undefined())
}

fn js_dom_after(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(target_id) = this_node_id(this) else {
        return Ok(JsValue::undefined());
    };
    let Some(parent_id) = context.get_data::<JavaScriptHostData>().and_then(|host| {
        host.state
            .borrow()
            .dom
            .node(target_id)
            .and_then(|node| node.parent)
    }) else {
        return Ok(JsValue::undefined());
    };
    let next_sibling = context
        .get_data::<JavaScriptHostData>()
        .and_then(|host| host.state.borrow().dom.next_sibling(target_id));
    let node_ids = js_values_to_dom_node_ids(args, context)?;
    let old_children = snapshot_dom_children(context, parent_id);
    if let Some(host) = context.get_data::<JavaScriptHostData>() {
        let mut state = host.state.borrow_mut();
        for node_id in node_ids {
            state.dom.insert_before(parent_id, node_id, next_sibling);
        }
        drop(state);
    }
    let new_children = snapshot_dom_children(context, parent_id);
    record_dom_child_list_mutation(context, parent_id, &old_children, &new_children);
    flush_mutation_observers(context);
    Ok(JsValue::undefined())
}

fn js_dom_replace_with(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(target_id) = this_node_id(this) else {
        return Ok(JsValue::undefined());
    };
    let Some(parent_id) = context.get_data::<JavaScriptHostData>().and_then(|host| {
        host.state
            .borrow()
            .dom
            .node(target_id)
            .and_then(|node| node.parent)
    }) else {
        return Ok(JsValue::undefined());
    };
    let node_ids = js_values_to_dom_node_ids(args, context)?;
    let old_children = snapshot_dom_children(context, parent_id);
    if let Some(host) = context.get_data::<JavaScriptHostData>() {
        let mut state = host.state.borrow_mut();
        for node_id in node_ids {
            state.dom.insert_before(parent_id, node_id, Some(target_id));
        }
        state.dom.detach_node(target_id);
        drop(state);
    }
    let new_children = snapshot_dom_children(context, parent_id);
    record_dom_child_list_mutation(context, parent_id, &old_children, &new_children);
    flush_mutation_observers(context);
    Ok(JsValue::undefined())
}

fn js_dom_replace_children(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(node_id) = this_node_id(this) else {
        return Ok(JsValue::undefined());
    };
    let node_ids = js_values_to_dom_node_ids(args, context)?;
    let old_children = snapshot_dom_children(context, node_id);
    if let Some(host) = context.get_data::<JavaScriptHostData>() {
        host.state
            .borrow_mut()
            .dom
            .replace_children(node_id, node_ids);
    }
    let new_children = snapshot_dom_children(context, node_id);
    record_dom_child_list_mutation(context, node_id, &old_children, &new_children);
    flush_mutation_observers(context);
    Ok(JsValue::undefined())
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

fn js_dom_matches(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let selector = js_value_to_string(args.first().unwrap_or(&JsValue::undefined()), context)?;
    let Some(node_id) = this_node_id(this) else {
        return Ok(JsValue::new(false));
    };
    let matches = context
        .get_data::<JavaScriptHostData>()
        .map(|host| host.state.borrow().dom.matches_selector(node_id, &selector))
        .unwrap_or(false);
    Ok(JsValue::new(matches))
}

fn js_dom_closest(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let selector = js_value_to_string(args.first().unwrap_or(&JsValue::undefined()), context)?;
    let Some(node_id) = this_node_id(this) else {
        return Ok(JsValue::null());
    };
    let closest = context
        .get_data::<JavaScriptHostData>()
        .and_then(|host| host.state.borrow().dom.closest_selector(node_id, &selector));
    Ok(closest
        .map(|node_id| JsValue::from(build_dom_node_object(context, node_id)))
        .unwrap_or_else(JsValue::null))
}

fn js_dom_contains(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(node_id) = this_node_id(this) else {
        return Ok(JsValue::new(false));
    };
    let Some(target_id) = node_id_argument(args.first()) else {
        return Ok(JsValue::new(false));
    };
    let contains = context
        .get_data::<JavaScriptHostData>()
        .map(|host| host.state.borrow().dom.contains_node(node_id, target_id))
        .unwrap_or(false);
    Ok(JsValue::new(contains))
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

fn js_document_create_document_fragment(
    _: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = context
        .get_data::<JavaScriptHostData>()
        .map(|host| host.state.borrow_mut().dom.create_document_fragment())
        .unwrap_or(0);
    Ok(JsValue::from(build_dom_node_object(context, node_id)))
}

fn js_get_node_by_id(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node_id = js_value_to_string(args.first().unwrap_or(&JsValue::undefined()), context)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let Some(host) = context.get_data::<JavaScriptHostData>() else {
        return Ok(JsValue::null());
    };
    if host.state.borrow().dom.node(node_id).is_none() {
        return Ok(JsValue::null());
    }
    Ok(JsValue::from(build_dom_node_object(context, node_id)))
}

fn js_create_mutation_observer(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(callback) = args.first().cloned() else {
        return Err(JsNativeError::typ()
            .with_message("MutationObserver callback is required")
            .into());
    };
    let Some(callback_object) = callback.as_object() else {
        return Err(JsNativeError::typ()
            .with_message("MutationObserver callback must be callable")
            .into());
    };
    if JsFunction::from_object(callback_object.clone()).is_none() {
        return Err(JsNativeError::typ()
            .with_message("MutationObserver callback must be callable")
            .into());
    }

    let records = JsArray::new(context);
    let observations = JsArray::new(context);
    let observer = ObjectInitializer::new(context)
        .property(js_string!("callback"), callback, Attribute::all())
        .property(js_string!("records"), records, Attribute::all())
        .property(js_string!("observations"), observations, Attribute::all())
        .build();
    Ok(JsValue::from(observer))
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
    let old_children = snapshot_dom_children(context, parent_id);
    if let Some(host) = context.get_data::<JavaScriptHostData>() {
        host.state
            .borrow_mut()
            .dom
            .append_child(parent_id, child_id);
    }
    let new_children = snapshot_dom_children(context, parent_id);
    record_dom_child_list_mutation(context, parent_id, &old_children, &new_children);
    flush_mutation_observers(context);
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
    let old_children = snapshot_dom_children(context, parent_id);
    if let Some(host) = context.get_data::<JavaScriptHostData>() {
        host.state
            .borrow_mut()
            .dom
            .insert_before(parent_id, child_id, before_id);
    }
    let new_children = snapshot_dom_children(context, parent_id);
    record_dom_child_list_mutation(context, parent_id, &old_children, &new_children);
    flush_mutation_observers(context);
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

fn js_dom_has_attribute(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let name = js_value_to_string(args.first().unwrap_or(&JsValue::undefined()), context)?;
    let Some(node_id) = this_node_id(this) else {
        return Ok(JsValue::new(false));
    };
    let has_attribute = context
        .get_data::<JavaScriptHostData>()
        .map(|host| host.state.borrow().dom.has_attribute(node_id, &name))
        .unwrap_or(false);
    Ok(JsValue::new(has_attribute))
}

fn js_dom_has_attributes(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(node_id) = this_node_id(this) else {
        return Ok(JsValue::new(false));
    };
    let has_attributes = context
        .get_data::<JavaScriptHostData>()
        .map(|host| !host.state.borrow().dom.attribute_names(node_id).is_empty())
        .unwrap_or(false);
    Ok(JsValue::new(has_attributes))
}

fn js_dom_get_attributes(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this).unwrap_or(0);
    Ok(JsValue::from(build_dom_attributes_object(context, node_id)))
}

fn js_dom_get_attribute_names(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(node_id) = this_node_id(this) else {
        return Ok(JsValue::from(JsArray::new(context)));
    };
    let names = context
        .get_data::<JavaScriptHostData>()
        .map(|host| host.state.borrow().dom.attribute_names(node_id))
        .unwrap_or_default();
    let array = JsArray::from_iter(
        names
            .into_iter()
            .map(|name| JsValue::from(js_string!(name))),
        context,
    );
    Ok(JsValue::from(array))
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
        let old_value = host.state.borrow().dom.get_attribute(node_id, &name);
        host.state
            .borrow_mut()
            .dom
            .set_attribute(node_id, &name, &value);
        record_dom_attribute_mutation(context, node_id, &name, old_value);
        flush_mutation_observers(context);
    }
    Ok(JsValue::undefined())
}

fn js_dom_toggle_attribute(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let name = js_value_to_string(args.first().unwrap_or(&JsValue::undefined()), context)?;
    let force = args.get(1).map(JsValue::to_boolean);
    let Some(node_id) = this_node_id(this) else {
        return Ok(JsValue::new(false));
    };
    let toggled = if let Some(host) = context.get_data::<JavaScriptHostData>() {
        let old_value = host.state.borrow().dom.get_attribute(node_id, &name);
        let mut state = host.state.borrow_mut();
        let toggled = if let Some(force) = force {
            if force {
                state.dom.set_attribute(node_id, &name, "");
                true
            } else {
                state.dom.remove_attribute(node_id, &name);
                false
            }
        } else if state.dom.has_attribute(node_id, &name) {
            state.dom.remove_attribute(node_id, &name);
            false
        } else {
            state.dom.set_attribute(node_id, &name, "");
            true
        };
        drop(state);
        record_dom_attribute_mutation(context, node_id, &name, old_value);
        flush_mutation_observers(context);
        toggled
    } else {
        false
    };
    Ok(JsValue::new(toggled))
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
        let old_value = host.state.borrow().dom.get_attribute(node_id, name);
        host.state
            .borrow_mut()
            .dom
            .set_attribute(node_id, name, &value);
        record_dom_attribute_mutation(context, node_id, name, old_value);
        flush_mutation_observers(context);
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
        let old_value = host.state.borrow().dom.get_attribute(node_id, &name);
        host.state.borrow_mut().dom.remove_attribute(node_id, &name);
        record_dom_attribute_mutation(context, node_id, &name, old_value);
        flush_mutation_observers(context);
    }
    Ok(JsValue::undefined())
}

fn js_dom_remove(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    if let Some(node_id) = this_node_id(this) {
        let parent_id = context.get_data::<JavaScriptHostData>().and_then(|host| {
            host.state
                .borrow()
                .dom
                .node(node_id)
                .and_then(|node| node.parent)
        });
        if let Some(parent_id) = parent_id {
            let old_children = snapshot_dom_children(context, parent_id);
            if let Some(host) = context.get_data::<JavaScriptHostData>() {
                host.state.borrow_mut().dom.detach_node(node_id);
            }
            let new_children = snapshot_dom_children(context, parent_id);
            record_dom_child_list_mutation(context, parent_id, &old_children, &new_children);
            flush_mutation_observers(context);
        } else if let Some(host) = context.get_data::<JavaScriptHostData>() {
            host.state.borrow_mut().dom.detach_node(node_id);
        }
    }
    Ok(JsValue::undefined())
}

fn js_dom_insert_adjacent_html(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let position = js_value_to_string(args.first().unwrap_or(&JsValue::undefined()), context)?
        .to_ascii_lowercase();
    let html = js_value_to_string(args.get(1).unwrap_or(&JsValue::undefined()), context)?;
    let Some(node_id) = this_node_id(this) else {
        return Ok(JsValue::undefined());
    };
    let target_id = if matches!(position.as_str(), "beforebegin" | "afterend") {
        context
            .get_data::<JavaScriptHostData>()
            .and_then(|host| {
                host.state
                    .borrow()
                    .dom
                    .node(node_id)
                    .and_then(|node| node.parent)
            })
            .unwrap_or(node_id)
    } else {
        node_id
    };
    let old_children = snapshot_dom_children(context, target_id);
    if let Some(host) = context.get_data::<JavaScriptHostData>() {
        let mut state = host.state.borrow_mut();
        match position.as_str() {
            "beforebegin" => state.dom.insert_fragment_before(node_id, &html),
            "afterbegin" => state.dom.insert_fragment_at_start(node_id, &html),
            "beforeend" => state.dom.append_fragment(node_id, &html),
            "afterend" => state.dom.insert_fragment_after(node_id, &html),
            _ => {}
        }
        drop(state);
    }
    let new_children = snapshot_dom_children(context, target_id);
    record_dom_child_list_mutation(context, target_id, &old_children, &new_children);
    flush_mutation_observers(context);
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
        .map(|host| host.state.borrow().dom.element_children(node_id))
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

fn js_dom_get_first_element_child(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this).unwrap_or(0);
    let child_id = context
        .get_data::<JavaScriptHostData>()
        .and_then(|host| host.state.borrow().dom.first_element_child(node_id));
    Ok(child_id
        .map(|child_id| JsValue::from(build_dom_node_object(context, child_id)))
        .unwrap_or_else(JsValue::null))
}

fn js_dom_get_last_element_child(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this).unwrap_or(0);
    let child_id = context
        .get_data::<JavaScriptHostData>()
        .and_then(|host| host.state.borrow().dom.last_element_child(node_id));
    Ok(child_id
        .map(|child_id| JsValue::from(build_dom_node_object(context, child_id)))
        .unwrap_or_else(JsValue::null))
}

fn js_dom_get_previous_element_sibling(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this).unwrap_or(0);
    let sibling_id = context
        .get_data::<JavaScriptHostData>()
        .and_then(|host| host.state.borrow().dom.previous_element_sibling(node_id));
    Ok(sibling_id
        .map(|sibling_id| JsValue::from(build_dom_node_object(context, sibling_id)))
        .unwrap_or_else(JsValue::null))
}

fn js_dom_get_next_element_sibling(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this).unwrap_or(0);
    let sibling_id = context
        .get_data::<JavaScriptHostData>()
        .and_then(|host| host.state.borrow().dom.next_element_sibling(node_id));
    Ok(sibling_id
        .map(|sibling_id| JsValue::from(build_dom_node_object(context, sibling_id)))
        .unwrap_or_else(JsValue::null))
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
    let old_children = snapshot_dom_children(context, node_id);
    if let Some(host) = context.get_data::<JavaScriptHostData>() {
        host.state.borrow_mut().dom.set_text_content(node_id, &text);
    }
    let new_children = snapshot_dom_children(context, node_id);
    record_dom_child_list_mutation(context, node_id, &old_children, &new_children);
    flush_mutation_observers(context);
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
    let old_children = snapshot_dom_children(context, node_id);
    if let Some(host) = context.get_data::<JavaScriptHostData>() {
        host.state
            .borrow_mut()
            .dom
            .replace_children_with_fragment(node_id, &html);
    }
    let new_children = snapshot_dom_children(context, node_id);
    record_dom_child_list_mutation(context, node_id, &old_children, &new_children);
    flush_mutation_observers(context);
    Ok(JsValue::undefined())
}

fn js_dom_get_outer_html(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this).unwrap_or(0);
    let html = context
        .get_data::<JavaScriptHostData>()
        .map(|host| host.state.borrow().dom.serialize_node(node_id))
        .unwrap_or_default();
    Ok(JsValue::from(js_string!(html)))
}

fn js_dom_set_outer_html(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this).unwrap_or(0);
    let html = js_value_to_string(args.first().unwrap_or(&JsValue::undefined()), context)?;
    let parent_id = context.get_data::<JavaScriptHostData>().and_then(|host| {
        host.state
            .borrow()
            .dom
            .node(node_id)
            .and_then(|node| node.parent)
    });
    let target_id = parent_id.unwrap_or(node_id);
    let old_children = snapshot_dom_children(context, target_id);
    if let Some(host) = context.get_data::<JavaScriptHostData>() {
        let mut state = host.state.borrow_mut();
        if parent_id.is_some() {
            state.dom.insert_fragment_before(node_id, &html);
            state.dom.detach_node(node_id);
        } else {
            state.dom.replace_children_with_fragment(node_id, &html);
        }
        drop(state);
    }
    let new_children = snapshot_dom_children(context, target_id);
    record_dom_child_list_mutation(context, target_id, &old_children, &new_children);
    flush_mutation_observers(context);
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

fn js_dom_get_dataset(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node_id = this_node_id(this).unwrap_or(0);
    Ok(JsValue::from(build_dom_dataset_object(context, node_id)))
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
            let state = host.state.borrow();
            if state.dom.is_document_node(node_id) {
                return None;
            }
            state
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
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(node_id) = this_node_id(this) else {
        return Ok(JsValue::null());
    };
    let is_document = context
        .get_data::<JavaScriptHostData>()
        .map(|host| host.state.borrow().dom.is_document_node(node_id))
        .unwrap_or(false);
    if is_document {
        return Ok(JsValue::null());
    }
    let document_id = context
        .get_data::<JavaScriptHostData>()
        .map(|host| host.state.borrow().dom.document_id)
        .unwrap_or(0);
    Ok(JsValue::from(build_dom_node_object(context, document_id)))
}

fn dataset_node_id(this: &JsValue, context: &mut Context) -> Option<usize> {
    let object = this.as_object()?;
    if let Some(handle) = object.downcast_ref::<DomDatasetHandle>() {
        return Some(handle.node_id);
    }
    let value = object.get(js_string!("__tobiraNodeId"), context).ok()?;
    let node_id = js_value_to_string(&value, context).ok()?;
    node_id.parse().ok()
}

fn attributes_node_id(this: &JsValue, context: &mut Context) -> Option<usize> {
    let object = this.as_object()?;
    if let Some(handle) = object.downcast_ref::<DomAttributesHandle>() {
        return Some(handle.node_id);
    }
    let value = object.get(js_string!("__tobiraNodeId"), context).ok()?;
    let node_id = js_value_to_string(&value, context).ok()?;
    node_id.parse().ok()
}

fn attribute_entries(context: &mut Context, node_id: usize) -> Vec<(String, String)> {
    context
        .get_data::<JavaScriptHostData>()
        .and_then(|host| host.state.borrow().dom.node_attributes(node_id).cloned())
        .unwrap_or_default()
        .into_iter()
        .collect()
}

fn attribute_entry_at(
    context: &mut Context,
    node_id: usize,
    index: usize,
) -> Option<(String, String)> {
    attribute_entries(context, node_id).into_iter().nth(index)
}

fn attribute_entry_named(
    context: &mut Context,
    node_id: usize,
    name: &str,
) -> Option<(String, String)> {
    context.get_data::<JavaScriptHostData>().and_then(|host| {
        host.state
            .borrow()
            .dom
            .node_attributes(node_id)
            .cloned()
            .and_then(|attributes| {
                attributes
                    .get(name)
                    .cloned()
                    .map(|value| (name.to_string(), value))
            })
    })
}

fn build_dom_attribute_object(
    context: &mut Context,
    name: &str,
    value: &str,
) -> boa_engine::object::JsObject {
    ObjectInitializer::new(context)
        .property(js_string!("name"), js_string!(name), Attribute::all())
        .property(js_string!("nodeName"), js_string!(name), Attribute::all())
        .property(js_string!("localName"), js_string!(name), Attribute::all())
        .property(js_string!("value"), js_string!(value), Attribute::all())
        .property(js_string!("nodeValue"), js_string!(value), Attribute::all())
        .property(js_string!("specified"), true, Attribute::all())
        .build()
}

fn dataset_property_to_attribute_name(property: &str) -> Option<String> {
    if property.is_empty() {
        return None;
    }

    let mut attribute = String::from("data-");
    for character in property.chars() {
        if character.is_ascii_uppercase() {
            attribute.push('-');
            attribute.push(character.to_ascii_lowercase());
        } else {
            attribute.push(character);
        }
    }
    Some(attribute)
}

fn dataset_attribute_to_property_name(attribute: &str) -> Option<String> {
    let remainder = attribute.strip_prefix("data-")?;
    if remainder.is_empty() {
        return None;
    }

    let mut property = String::with_capacity(remainder.len());
    let mut characters = remainder.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '-'
            && let Some(next) = characters.peek().copied()
            && next.is_ascii_lowercase()
        {
            property.push(next.to_ascii_uppercase());
            let _ = characters.next();
            continue;
        }
        property.push(character);
    }

    if property.is_empty() {
        None
    } else {
        Some(property)
    }
}

fn js_dom_dataset_get(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(target) = args.first() else {
        return Ok(JsValue::undefined());
    };
    let Some(node_id) = dataset_node_id(target, context) else {
        return Ok(JsValue::undefined());
    };
    let target = args.get(1).cloned().unwrap_or_else(JsValue::undefined);
    if !target.is_string() {
        return Ok(JsValue::undefined());
    }

    let property = js_value_to_string(&target, context)?;
    let Some(attribute_name) = dataset_property_to_attribute_name(&property) else {
        return Ok(args
            .first()
            .and_then(JsValue::as_object)
            .map(|object| object.get(js_string!(property), context))
            .transpose()?
            .unwrap_or_else(JsValue::undefined));
    };
    let value = context.get_data::<JavaScriptHostData>().and_then(|host| {
        host.state
            .borrow()
            .dom
            .get_attribute(node_id, &attribute_name)
    });
    if let Some(value) = value {
        Ok(JsValue::from(js_string!(value)))
    } else {
        Ok(args
            .first()
            .and_then(JsValue::as_object)
            .map(|object| object.get(js_string!(property), context))
            .transpose()?
            .unwrap_or_else(JsValue::undefined))
    }
}

fn js_dom_dataset_set(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(target) = args.first() else {
        return Ok(JsValue::new(false));
    };
    let Some(node_id) = dataset_node_id(target, context) else {
        return Ok(JsValue::new(false));
    };
    let target = args.get(1).cloned().unwrap_or_else(JsValue::undefined);
    if !target.is_string() {
        return Ok(JsValue::new(false));
    }

    let property = js_value_to_string(&target, context)?;
    let Some(attribute_name) = dataset_property_to_attribute_name(&property) else {
        return Ok(JsValue::new(false));
    };
    let value = js_value_to_string(args.get(2).unwrap_or(&JsValue::undefined()), context)?;
    if let Some(host) = context.get_data::<JavaScriptHostData>() {
        host.state
            .borrow_mut()
            .dom
            .set_attribute(node_id, &attribute_name, &value);
    }
    Ok(JsValue::new(true))
}

fn js_dom_dataset_has(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(target) = args.first() else {
        return Ok(JsValue::new(false));
    };
    let Some(node_id) = dataset_node_id(target, context) else {
        return Ok(JsValue::new(false));
    };
    let target = args.get(1).cloned().unwrap_or_else(JsValue::undefined);
    if !target.is_string() {
        return Ok(JsValue::new(false));
    }

    let property = js_value_to_string(&target, context)?;
    let Some(attribute_name) = dataset_property_to_attribute_name(&property) else {
        return Ok(JsValue::new(false));
    };
    let has_attribute = context
        .get_data::<JavaScriptHostData>()
        .map(|host| {
            host.state
                .borrow()
                .dom
                .has_attribute(node_id, &attribute_name)
        })
        .unwrap_or(false);
    Ok(JsValue::new(has_attribute))
}

fn js_dom_dataset_delete_property(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(target) = args.first() else {
        return Ok(JsValue::new(false));
    };
    let Some(node_id) = dataset_node_id(target, context) else {
        return Ok(JsValue::new(false));
    };
    let target = args.get(1).cloned().unwrap_or_else(JsValue::undefined);
    if !target.is_string() {
        return Ok(JsValue::new(false));
    }

    let property = js_value_to_string(&target, context)?;
    let Some(attribute_name) = dataset_property_to_attribute_name(&property) else {
        return Ok(JsValue::new(false));
    };
    if let Some(host) = context.get_data::<JavaScriptHostData>() {
        host.state
            .borrow_mut()
            .dom
            .remove_attribute(node_id, &attribute_name);
    }
    Ok(JsValue::new(true))
}

fn js_dom_dataset_own_keys(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(target) = args.first() else {
        return Ok(JsValue::from(JsArray::new(context)));
    };
    let Some(node_id) = dataset_node_id(target, context) else {
        return Ok(JsValue::from(JsArray::new(context)));
    };
    let keys = context
        .get_data::<JavaScriptHostData>()
        .map(|host| {
            host.state
                .borrow()
                .dom
                .attribute_names(node_id)
                .into_iter()
                .filter_map(|attribute| dataset_attribute_to_property_name(&attribute))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let array = JsArray::from_iter(
        keys.into_iter().map(|key| JsValue::from(js_string!(key))),
        context,
    );
    Ok(JsValue::from(array))
}

fn js_dom_dataset_get_own_property_descriptor(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(target) = args.first() else {
        return Ok(JsValue::undefined());
    };
    let Some(node_id) = dataset_node_id(target, context) else {
        return Ok(JsValue::undefined());
    };
    let target = args.get(1).cloned().unwrap_or_else(JsValue::undefined);
    if !target.is_string() {
        return Ok(JsValue::undefined());
    }

    let property = js_value_to_string(&target, context)?;
    let Some(attribute_name) = dataset_property_to_attribute_name(&property) else {
        return Ok(JsValue::undefined());
    };
    let Some(value) = context.get_data::<JavaScriptHostData>().and_then(|host| {
        host.state
            .borrow()
            .dom
            .get_attribute(node_id, &attribute_name)
    }) else {
        return Ok(JsValue::undefined());
    };
    let descriptor = ObjectInitializer::new(context)
        .property(js_string!("value"), js_string!(value), Attribute::all())
        .property(js_string!("writable"), true, Attribute::all())
        .property(js_string!("enumerable"), true, Attribute::all())
        .property(js_string!("configurable"), true, Attribute::all())
        .build();
    Ok(JsValue::from(descriptor))
}

fn js_dom_attributes_item(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(node_id) = attributes_node_id(this, context) else {
        return Ok(JsValue::null());
    };
    let Some(index_value) = args.first() else {
        return Ok(JsValue::null());
    };
    let index = js_value_to_string(index_value, context)
        .ok()
        .and_then(|value| value.parse::<usize>().ok());
    let Some(index) = index else {
        return Ok(JsValue::null());
    };
    Ok(attribute_entry_at(context, node_id, index)
        .map(|(name, value)| JsValue::from(build_dom_attribute_object(context, &name, &value)))
        .unwrap_or_else(JsValue::null))
}

fn js_dom_attributes_get_named_item(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(node_id) = attributes_node_id(this, context) else {
        return Ok(JsValue::null());
    };
    let name = js_value_to_string(args.first().unwrap_or(&JsValue::undefined()), context)?;
    Ok(attribute_entry_named(context, node_id, &name)
        .map(|(name, value)| JsValue::from(build_dom_attribute_object(context, &name, &value)))
        .unwrap_or_else(JsValue::null))
}

fn js_dom_attributes_get(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(target) = args.first() else {
        return Ok(JsValue::undefined());
    };
    let Some(node_id) = attributes_node_id(target, context) else {
        return Ok(JsValue::undefined());
    };
    let Some(property) = args.get(1) else {
        return Ok(JsValue::undefined());
    };
    if !property.is_string() {
        return Ok(JsValue::undefined());
    }

    let property = js_value_to_string(property, context)?;
    if property == "length" {
        return Ok(JsValue::new(
            attribute_entries(context, node_id).len() as u32
        ));
    }

    if let Some((name, value)) = property
        .parse::<usize>()
        .ok()
        .and_then(|index| attribute_entry_at(context, node_id, index))
    {
        return Ok(JsValue::from(build_dom_attribute_object(
            context, &name, &value,
        )));
    }

    if let Some((name, value)) = attribute_entry_named(context, node_id, &property) {
        return Ok(JsValue::from(build_dom_attribute_object(
            context, &name, &value,
        )));
    }

    Ok(target
        .as_object()
        .map(|object| object.get(js_string!(property), context))
        .transpose()?
        .unwrap_or_else(JsValue::undefined))
}

fn js_dom_attributes_set(
    _this: &JsValue,
    _args: &[JsValue],
    _context: &mut Context,
) -> JsResult<JsValue> {
    Ok(JsValue::new(false))
}

fn js_dom_attributes_has(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(target) = args.first() else {
        return Ok(JsValue::new(false));
    };
    let Some(node_id) = attributes_node_id(target, context) else {
        return Ok(JsValue::new(false));
    };
    let Some(property) = args.get(1) else {
        return Ok(JsValue::new(false));
    };
    if !property.is_string() {
        return Ok(JsValue::new(false));
    }

    let property = js_value_to_string(property, context)?;
    if property == "length" {
        return Ok(JsValue::new(true));
    }
    if property.parse::<usize>().is_ok() {
        return Ok(JsValue::new(
            attribute_entry_at(context, node_id, property.parse().unwrap_or(usize::MAX)).is_some(),
        ));
    }
    if attribute_entry_named(context, node_id, &property).is_some() {
        return Ok(JsValue::new(true));
    }

    Ok(target
        .as_object()
        .map(|object| object.get(js_string!(property), context))
        .transpose()?
        .map(|value| !value.is_undefined())
        .unwrap_or(false)
        .into())
}

fn js_dom_attributes_own_keys(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(target) = args.first() else {
        return Ok(JsValue::from(JsArray::new(context)));
    };
    let Some(node_id) = attributes_node_id(target, context) else {
        return Ok(JsValue::from(JsArray::new(context)));
    };
    let length = attribute_entries(context, node_id).len();
    let mut keys = Vec::with_capacity(length + 4);
    keys.push(JsValue::from(js_string!("length")));
    keys.push(JsValue::from(js_string!("item")));
    keys.push(JsValue::from(js_string!("getNamedItem")));
    keys.push(JsValue::from(js_string!("namedItem")));
    for index in 0..length {
        keys.push(JsValue::from(js_string!(index.to_string())));
    }
    let array = JsArray::from_iter(keys, context);
    Ok(JsValue::from(array))
}

fn js_dom_attributes_get_own_property_descriptor(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(target) = args.first() else {
        return Ok(JsValue::undefined());
    };
    let Some(node_id) = attributes_node_id(target, context) else {
        return Ok(JsValue::undefined());
    };
    let Some(property) = args.get(1) else {
        return Ok(JsValue::undefined());
    };
    if !property.is_string() {
        return Ok(JsValue::undefined());
    }

    let property = js_value_to_string(property, context)?;
    if property == "length" {
        let length = attribute_entries(context, node_id).len() as u32;
        let descriptor = ObjectInitializer::new(context)
            .property(js_string!("value"), JsValue::new(length), Attribute::all())
            .property(js_string!("writable"), false, Attribute::all())
            .property(js_string!("enumerable"), false, Attribute::all())
            .property(js_string!("configurable"), true, Attribute::all())
            .build();
        return Ok(JsValue::from(descriptor));
    }

    if property == "item" || property == "getNamedItem" || property == "namedItem" {
        let function = target
            .as_object()
            .map(|object| object.get(js_string!(property), context))
            .transpose()?;
        if let Some(function) = function {
            let descriptor = ObjectInitializer::new(context)
                .property(js_string!("value"), function, Attribute::all())
                .property(js_string!("writable"), true, Attribute::all())
                .property(js_string!("enumerable"), false, Attribute::all())
                .property(js_string!("configurable"), true, Attribute::all())
                .build();
            return Ok(JsValue::from(descriptor));
        }
        return Ok(JsValue::undefined());
    }

    if let Some((name, value)) = property
        .parse::<usize>()
        .ok()
        .and_then(|index| attribute_entry_at(context, node_id, index))
    {
        let attribute = build_dom_attribute_object(context, &name, &value);
        let descriptor = ObjectInitializer::new(context)
            .property(
                js_string!("value"),
                JsValue::from(attribute),
                Attribute::all(),
            )
            .property(js_string!("writable"), false, Attribute::all())
            .property(js_string!("enumerable"), true, Attribute::all())
            .property(js_string!("configurable"), true, Attribute::all())
            .build();
        return Ok(JsValue::from(descriptor));
    }

    if let Some((name, value)) = attribute_entry_named(context, node_id, &property) {
        let attribute = build_dom_attribute_object(context, &name, &value);
        let descriptor = ObjectInitializer::new(context)
            .property(
                js_string!("value"),
                JsValue::from(attribute),
                Attribute::all(),
            )
            .property(js_string!("writable"), false, Attribute::all())
            .property(js_string!("enumerable"), true, Attribute::all())
            .property(js_string!("configurable"), true, Attribute::all())
            .build();
        return Ok(JsValue::from(descriptor));
    }

    Ok(JsValue::undefined())
}

fn style_node_id_from_this(this: &JsValue) -> Option<usize> {
    let object = this.as_object()?;
    let handle = object.downcast_ref::<DomStyleHandle>()?;
    Some(handle.node_id)
}

fn computed_style_node_id_from_this(this: &JsValue) -> Option<usize> {
    let object = this.as_object()?;
    let handle = object.downcast_ref::<ComputedStyleHandle>()?;
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

fn default_display_for_tag(tag_name: &str) -> &'static str {
    match tag_name {
        "html" | "body" | "div" | "section" | "article" | "aside" | "main" | "header"
        | "footer" | "nav" | "p" | "ul" | "ol" | "form" | "fieldset" | "legend" | "pre"
        | "blockquote" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => "block",
        "table" => "table",
        "tr" => "table-row",
        "td" | "th" => "table-cell",
        "thead" | "tbody" | "tfoot" => "table-row-group",
        "li" => "list-item",
        "img" | "button" | "input" | "select" | "textarea" => "inline-block",
        "span" | "a" | "b" | "i" | "u" | "strong" | "em" | "small" | "code" | "abbr" | "label"
        | "sup" | "sub" | "mark" => "inline",
        "script" | "style" | "head" | "meta" | "link" | "title" | "template" => "none",
        _ => "inline",
    }
}

fn default_font_size_for_tag(tag_name: &str) -> &'static str {
    match tag_name {
        "h1" => "2em",
        "h2" => "1.5em",
        "h3" => "1.17em",
        "h4" => "1em",
        "h5" => "0.83em",
        "h6" => "0.67em",
        _ => "16px",
    }
}

fn default_font_weight_for_tag(tag_name: &str) -> &'static str {
    match tag_name {
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "strong" | "b" | "th" => "700",
        _ => "400",
    }
}

fn default_background_color_for_tag(tag_name: &str) -> &'static str {
    match tag_name {
        "html" | "body" => "rgb(255, 255, 255)",
        _ => "rgba(0, 0, 0, 0)",
    }
}

fn box_shorthand_value(top: &str, right: &str, bottom: &str, left: &str) -> String {
    if top == right && right == bottom && bottom == left {
        top.to_string()
    } else if top == bottom && right == left {
        format!("{top} {right}")
    } else if right == left {
        format!("{top} {right} {bottom}")
    } else {
        format!("{top} {right} {bottom} {left}")
    }
}

fn computed_style_parent_value(
    context: &mut Context,
    node_id: usize,
    property_name: &str,
) -> Option<String> {
    let parent_id = context.get_data::<JavaScriptHostData>().and_then(|host| {
        host.state
            .borrow()
            .dom
            .node(node_id)
            .and_then(|node| node.parent)
    })?;
    Some(computed_style_property_value(
        context,
        parent_id,
        property_name,
    ))
}

fn computed_style_property_value(
    context: &mut Context,
    node_id: usize,
    property_name: &str,
) -> String {
    let normalized = normalize_css_property_name(property_name);
    let inline = inline_style_property_value(context, node_id, &normalized);
    if !inline.is_empty() {
        if inline.eq_ignore_ascii_case("inherit") {
            return computed_style_parent_value(context, node_id, &normalized).unwrap_or_default();
        }
        return inline;
    }

    let tag_name = context
        .get_data::<JavaScriptHostData>()
        .and_then(|host| {
            host.state
                .borrow()
                .dom
                .element(node_id)
                .map(|element| element.tag_name.clone())
        })
        .unwrap_or_default();

    match normalized.as_str() {
        "display" => {
            if context
                .get_data::<JavaScriptHostData>()
                .map(|host| host.state.borrow().dom.has_attribute(node_id, "hidden"))
                .unwrap_or(false)
            {
                "none".to_string()
            } else {
                default_display_for_tag(&tag_name).to_string()
            }
        }
        "position" => "static".to_string(),
        "visibility" => computed_style_parent_value(context, node_id, "visibility")
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "visible".to_string()),
        "color" => computed_style_parent_value(context, node_id, "color")
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "rgb(0, 0, 0)".to_string()),
        "background-color" => default_background_color_for_tag(&tag_name).to_string(),
        "font-size" => match tag_name.as_str() {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                default_font_size_for_tag(&tag_name).to_string()
            }
            _ => computed_style_parent_value(context, node_id, "font-size")
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "16px".to_string()),
        },
        "font-weight" => match tag_name.as_str() {
            "strong" | "b" | "th" => default_font_weight_for_tag(&tag_name).to_string(),
            _ => computed_style_parent_value(context, node_id, "font-weight")
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "400".to_string()),
        },
        "font-family" => computed_style_parent_value(context, node_id, "font-family")
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "sans-serif".to_string()),
        "font-style" => "normal".to_string(),
        "line-height" => computed_style_parent_value(context, node_id, "line-height")
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "normal".to_string()),
        "text-align" => computed_style_parent_value(context, node_id, "text-align")
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "left".to_string()),
        "white-space" => computed_style_parent_value(context, node_id, "white-space")
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "normal".to_string()),
        "text-decoration" => "none".to_string(),
        "text-transform" => "none".to_string(),
        "text-indent" => "0px".to_string(),
        "letter-spacing" => "normal".to_string(),
        "pointer-events" => "auto".to_string(),
        "opacity" => "1".to_string(),
        "overflow" => "visible".to_string(),
        "width" | "height" => "auto".to_string(),
        "max-width" | "min-width" | "max-height" | "min-height" => "none".to_string(),
        "margin-top" | "margin-right" | "margin-bottom" | "margin-left" | "padding-top"
        | "padding-right" | "padding-bottom" | "padding-left" => "0px".to_string(),
        "margin" => box_shorthand_value(
            &computed_style_property_value(context, node_id, "margin-top"),
            &computed_style_property_value(context, node_id, "margin-right"),
            &computed_style_property_value(context, node_id, "margin-bottom"),
            &computed_style_property_value(context, node_id, "margin-left"),
        ),
        "padding" => box_shorthand_value(
            &computed_style_property_value(context, node_id, "padding-top"),
            &computed_style_property_value(context, node_id, "padding-right"),
            &computed_style_property_value(context, node_id, "padding-bottom"),
            &computed_style_property_value(context, node_id, "padding-left"),
        ),
        "border-top-width" | "border-right-width" | "border-bottom-width" | "border-left-width" => {
            "0px".to_string()
        }
        "border-width" => box_shorthand_value(
            &computed_style_property_value(context, node_id, "border-top-width"),
            &computed_style_property_value(context, node_id, "border-right-width"),
            &computed_style_property_value(context, node_id, "border-bottom-width"),
            &computed_style_property_value(context, node_id, "border-left-width"),
        ),
        "border-top-style" | "border-right-style" | "border-bottom-style" | "border-left-style" => {
            "none".to_string()
        }
        "border-style" => box_shorthand_value(
            &computed_style_property_value(context, node_id, "border-top-style"),
            &computed_style_property_value(context, node_id, "border-right-style"),
            &computed_style_property_value(context, node_id, "border-bottom-style"),
            &computed_style_property_value(context, node_id, "border-left-style"),
        ),
        "border-top-color" | "border-right-color" | "border-bottom-color" | "border-left-color" => {
            "currentcolor".to_string()
        }
        "border-color" => box_shorthand_value(
            &computed_style_property_value(context, node_id, "border-top-color"),
            &computed_style_property_value(context, node_id, "border-right-color"),
            &computed_style_property_value(context, node_id, "border-bottom-color"),
            &computed_style_property_value(context, node_id, "border-left-color"),
        ),
        "vertical-align" => "baseline".to_string(),
        "cursor" => "auto".to_string(),
        _ => String::new(),
    }
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

fn js_computed_style_get_property_value(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let name = args
        .first()
        .map(|value| js_value_to_string(value, context))
        .transpose()?
        .unwrap_or_default();
    let value = computed_style_node_id_from_this(this)
        .map(|node_id| computed_style_property_value(context, node_id, &name))
        .unwrap_or_default();
    Ok(JsValue::from(js_string!(value)))
}

fn js_computed_style_get_property_priority(
    _this: &JsValue,
    _: &[JsValue],
    _context: &mut Context,
) -> JsResult<JsValue> {
    Ok(JsValue::from(js_string!("")))
}

fn js_computed_style_item(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    if computed_style_node_id_from_this(this).is_none() {
        return Ok(JsValue::undefined());
    }
    let index = args
        .first()
        .map(|value| js_value_to_string(value, context))
        .transpose()?
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(usize::MAX);
    if index == usize::MAX || index >= COMPUTED_STYLE_PROPERTIES.len() {
        return Ok(JsValue::undefined());
    }
    let (_, css_name) = COMPUTED_STYLE_PROPERTIES[index];
    Ok(JsValue::from(js_string!(css_name)))
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
    let has_force = args.get(1).is_some();
    let force = args.get(1).map(JsValue::to_boolean);
    let toggled = if let Some(object) = this.as_object()
        && let Some(handle) = object.downcast_ref::<DomClassListHandle>()
        && let Some(host) = context.get_data::<JavaScriptHostData>()
    {
        let mut state = host.state.borrow_mut();
        if has_force {
            if force.unwrap_or(false) {
                state.dom.add_class(handle.node_id, &class_name);
                true
            } else {
                state.dom.remove_class(handle.node_id, &class_name);
                false
            }
        } else {
            state.dom.toggle_class(handle.node_id, &class_name)
        }
    } else {
        false
    };
    Ok(JsValue::new(toggled))
}

fn js_dom_class_list_replace(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let old_class = js_value_to_string(args.first().unwrap_or(&JsValue::undefined()), context)?;
    let new_class = js_value_to_string(args.get(1).unwrap_or(&JsValue::undefined()), context)?;
    let replaced = if let Some(object) = this.as_object()
        && let Some(handle) = object.downcast_ref::<DomClassListHandle>()
        && let Some(host) = context.get_data::<JavaScriptHostData>()
    {
        host.state
            .borrow_mut()
            .dom
            .replace_class(handle.node_id, &old_class, &new_class)
    } else {
        false
    };
    Ok(JsValue::new(replaced))
}

fn class_list_node_id_from_this(this: &JsValue) -> Option<usize> {
    this.as_object()?
        .downcast_ref::<DomClassListHandle>()
        .map(|handle| handle.node_id)
}

fn class_list_value(context: &mut Context, node_id: usize) -> String {
    context
        .get_data::<JavaScriptHostData>()
        .and_then(|host| host.state.borrow().dom.get_attribute(node_id, "class"))
        .unwrap_or_default()
}

fn set_class_list_value(context: &mut Context, node_id: usize, value: &str) {
    if let Some(host) = context.get_data::<JavaScriptHostData>() {
        let mut state = host.state.borrow_mut();
        let value = value.trim();
        if value.is_empty() {
            state.dom.remove_attribute(node_id, "class");
        } else {
            state.dom.set_attribute(node_id, "class", value);
        }
    }
}

fn js_dom_class_list_get_value(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(node_id) = class_list_node_id_from_this(this) else {
        return Ok(JsValue::from(js_string!("")));
    };
    Ok(JsValue::from(js_string!(class_list_value(
        context, node_id
    ))))
}

fn js_dom_class_list_set_value(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let value = js_value_to_string(args.first().unwrap_or(&JsValue::undefined()), context)?;
    if let Some(node_id) = class_list_node_id_from_this(this) {
        set_class_list_value(context, node_id, &value);
    }
    Ok(JsValue::undefined())
}

fn js_dom_class_list_get_length(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(node_id) = class_list_node_id_from_this(this) else {
        return Ok(JsValue::new(0));
    };
    let length = class_list_value(context, node_id)
        .split_ascii_whitespace()
        .filter(|class_name| !class_name.is_empty())
        .count();
    Ok(JsValue::new(length as i32))
}

fn js_dom_class_list_item(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let index = args
        .first()
        .and_then(JsValue::as_number)
        .map(|value| value as usize)
        .unwrap_or(usize::MAX);
    let Some(node_id) = class_list_node_id_from_this(this) else {
        return Ok(JsValue::null());
    };
    let token = class_list_value(context, node_id)
        .split_ascii_whitespace()
        .filter(|class_name| !class_name.is_empty())
        .nth(index)
        .map(|class_name| JsValue::from(js_string!(class_name)));
    Ok(token.unwrap_or_else(JsValue::null))
}

fn js_dom_class_list_to_string(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(node_id) = class_list_node_id_from_this(this) else {
        return Ok(JsValue::from(js_string!("")));
    };
    Ok(JsValue::from(js_string!(class_list_value(
        context, node_id
    ))))
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

fn js_string_literal(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}

fn js_usize_array_literal(values: &[usize]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn record_dom_mutation(
    context: &mut Context,
    mutation_type: &str,
    target_node_id: usize,
    attribute_name: Option<&str>,
    old_value: Option<&str>,
    added_node_ids: &[usize],
    removed_node_ids: &[usize],
) {
    let script = format!(
        "if (typeof __tobiraRecordMutation === 'function') {{ __tobiraRecordMutation({}, {}, {}, {}, {}, {}); }}",
        js_string_literal(mutation_type),
        target_node_id,
        attribute_name
            .map(js_string_literal)
            .unwrap_or_else(|| "null".to_string()),
        old_value
            .map(js_string_literal)
            .unwrap_or_else(|| "null".to_string()),
        js_usize_array_literal(added_node_ids),
        js_usize_array_literal(removed_node_ids),
    );
    let _ = context.eval(Source::from_bytes(script.as_str()));
}

fn snapshot_dom_children(context: &mut Context, node_id: usize) -> Vec<usize> {
    context
        .get_data::<JavaScriptHostData>()
        .and_then(|host| {
            host.state
                .borrow()
                .dom
                .node(node_id)
                .map(|node| node.children.clone())
        })
        .unwrap_or_default()
}

fn js_optional_string_property(
    object: &boa_engine::object::JsObject,
    name: &str,
    context: &mut Context,
) -> JsResult<Option<String>> {
    let value = object.get(js_string!(name), context)?;
    if value.is_null() || value.is_undefined() {
        return Ok(None);
    }
    Ok(Some(js_value_to_string(&value, context)?))
}

fn js_optional_i32_property(
    object: &boa_engine::object::JsObject,
    name: &str,
    context: &mut Context,
) -> JsResult<Option<i32>> {
    let value = object.get(js_string!(name), context)?;
    if value.is_null() || value.is_undefined() {
        return Ok(None);
    }
    let number = value.to_number(context)?;
    if !number.is_finite() {
        return Ok(None);
    }
    Ok(Some(number.round() as i32))
}

fn js_optional_bool_property(
    object: &boa_engine::object::JsObject,
    name: &str,
    context: &mut Context,
) -> JsResult<bool> {
    Ok(object.get(js_string!(name), context)?.to_boolean())
}

fn record_dom_child_list_mutation(
    context: &mut Context,
    target_node_id: usize,
    old_children: &[usize],
    new_children: &[usize],
) {
    if old_children == new_children {
        return;
    }
    let reorder_only = old_children.len() == new_children.len()
        && old_children
            .iter()
            .all(|child_id| new_children.contains(child_id))
        && new_children
            .iter()
            .all(|child_id| old_children.contains(child_id));
    let (added, removed) = if reorder_only {
        (new_children.to_vec(), old_children.to_vec())
    } else {
        let added: Vec<usize> = new_children
            .iter()
            .copied()
            .filter(|child_id| !old_children.contains(child_id))
            .collect();
        let removed: Vec<usize> = old_children
            .iter()
            .copied()
            .filter(|child_id| !new_children.contains(child_id))
            .collect();
        (added, removed)
    };
    if added.is_empty() && removed.is_empty() {
        return;
    }
    record_dom_mutation(
        context,
        "childList",
        target_node_id,
        None,
        None,
        &added,
        &removed,
    );
}

fn record_dom_attribute_mutation(
    context: &mut Context,
    target_node_id: usize,
    attribute_name: &str,
    old_value: Option<String>,
) {
    record_dom_mutation(
        context,
        "attributes",
        target_node_id,
        Some(attribute_name),
        old_value.as_deref(),
        &[],
        &[],
    );
}

fn flush_mutation_observers(context: &mut Context) {
    for _ in 0..8 {
        let delivered = context
            .eval(Source::from_bytes(
                "typeof __tobiraFlushMutationObservers === 'function' && __tobiraFlushMutationObservers()",
            ))
            .ok()
            .map(|value| value.to_boolean())
            .unwrap_or(false);
        if !delivered {
            break;
        }
    }
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
        DomEventRequest, HttpResponse, JavaScriptRuntime, XmlHttpRequestHandle,
        build_fetch_response_object, build_xml_http_request_object, current_location_url,
        ensure_same_origin_script_url, fetch_for_script, js_value_to_string,
        process_document_scripts, resolve_requested_url, set_location_href,
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
    fn runs_set_timeout_callbacks_after_script_turn() {
        let processed = process_document_scripts(
            "<script>setTimeout(function () { document.write('<p>Later</p>'); }, 1);</script>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert!(processed.html.contains("<p>Later</p>"));
    }

    #[test]
    fn defers_nested_timeouts_until_the_next_turn() {
        let processed = process_document_scripts(
            "<script>setTimeout(function () { document.title = 'first'; setTimeout(function () { document.title = 'second'; }, 1); }, 1);</script>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert_eq!(processed.title_override.as_deref(), Some("first"));
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
    fn dispatches_hashchange_for_location_hash_updates() {
        let processed = process_document_scripts(
            "<script>window.addEventListener('hashchange', function () { document.title = location.href + '|' + location.hash; }); location.hash = '#frag';</script>",
            &Url::parse("https://example.com/start").unwrap(),
        );

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
    fn history_state_tracks_push_state_and_popstate() {
        let processed = process_document_scripts(
            "<script>window.addEventListener('popstate', function () { document.title = location.href + '|' + String(history.state.page); }); history.pushState({ page: 1 }, '', '/one'); history.pushState({ page: 2 }, '', '/two'); history.back();</script>",
            &Url::parse("https://example.com/start").unwrap(),
        );

        assert_eq!(
            processed.soft_navigation_target.as_deref(),
            Some("https://example.com/one")
        );
        assert_eq!(
            processed.title_override.as_deref(),
            Some("https://example.com/one|1")
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
    fn history_back_and_forward_restore_scroll_positions() {
        let processed = process_document_scripts(
            "<script>window.scrollTo(0, 120); history.pushState({}, '', '/one'); window.scrollTo(0, 240); history.pushState({}, '', '/two'); history.back(); var firstBack = location.href + '|' + String(window.scrollY); history.back(); var secondBack = location.href + '|' + String(window.scrollY); history.forward(); var forward = location.href + '|' + String(window.scrollY); document.title = firstBack + '||' + secondBack + '||' + forward;</script>",
            &Url::parse("https://example.com/start").unwrap(),
        );

        assert_eq!(
            processed.title_override.as_deref(),
            Some(
                "https://example.com/one|240||https://example.com/start|120||https://example.com/one|240"
            )
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
            ..Default::default()
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
    fn supports_dynamic_document_body_head_and_document_element_getters() {
        let processed = process_document_scripts(
            "<html><script>var root = document.documentElement; var initialBody = document.body; var initialHead = document.head; var head = document.createElement('head'); var body = document.createElement('body'); root.appendChild(head); root.appendChild(body); document.body.setAttribute('data-live', [initialBody === null, initialHead === null, document.documentElement === root, document.body === body, document.head === head].join('|'));</script></html>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert!(
            processed
                .html
                .contains("data-live=\"true|true|true|true|true\"")
        );
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
    fn supports_get_computed_style_snapshot_and_inheritance() {
        let processed = process_document_scripts(
            "<html><body><div id=\"outer\" style=\"color: rgb(1, 2, 3); font-size: 24px; text-align: center; white-space: pre;\"><span id=\"inner\">Hello</span><strong id=\"bold\">Bold</strong></div><script>var outer = document.getElementById('outer'); var inner = document.getElementById('inner'); var bold = document.getElementById('bold'); var outerStyle = getComputedStyle(outer); var innerStyle = getComputedStyle(inner); var boldStyle = getComputedStyle(bold); document.body.setAttribute('data-computed', [outerStyle.display, innerStyle.display, innerStyle.fontSize, innerStyle.color, innerStyle.getPropertyValue('font-size'), innerStyle.getPropertyValue('color'), boldStyle.fontWeight, boldStyle.getPropertyValue('font-weight')].join('|'));</script></body></html>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert!(
            processed.html.contains(
                "data-computed=\"block|inline|24px|rgb(1, 2, 3)|24px|rgb(1, 2, 3)|700|700\""
            ),
            "{}",
            processed.html
        );
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
    fn supports_node_introspection_and_sibling_accessors() {
        let processed = process_document_scripts(
            "<html><body><div id=\"box\"></div><script>var box = document.getElementById('box'); var first = document.createTextNode('one'); var second = document.createTextNode('two'); box.append(first); box.append(second); document.body.setAttribute('data-node', [document.nodeType, document.nodeName, document.ownerDocument === null, box.nodeType, box.nodeName, first.nodeType, first.nodeName, first.nodeValue, first.isConnected, first.previousSibling === null, first.nextSibling === second, second.previousSibling === first, second.nextSibling === null, box.firstChild === first, box.lastChild === second, box.isConnected].join('|'));</script></body></html>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert!(
            processed
                .html
                .contains("data-node=\"9|#document|true|1|DIV|3|#text|one|true|true|true|true|true|true|true|true\""),
            "{}",
            processed.html
        );
    }

    #[test]
    fn supports_document_fragment_flattening_and_clone_node() {
        let processed = process_document_scripts(
            "<html><body><div id=\"box\"></div><script>var box = document.getElementById('box'); var frag = document.createDocumentFragment(); var span = document.createElement('span'); span.textContent = 'B'; frag.append('A', span); var template = document.createElement('section'); template.append('X', document.createElement('strong')); template.lastChild.textContent = 'Y'; var copy = template.cloneNode(true); copy.id = 'copy'; box.append(frag); box.append(copy); document.body.setAttribute('data-frag', [frag.nodeType, frag.nodeName, frag.childNodes.length].join('|'));</script></body></html>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert!(
            processed
                .html
                .contains("data-frag=\"11|#document-fragment|0\""),
            "{}",
            processed.html
        );
        assert!(
            processed
                .html
                .contains("<div id=\"box\">A<span>B</span><section id=\"copy\">X<strong>Y</strong></section></div>"),
            "{}",
            processed.html
        );
    }

    #[test]
    fn supports_replace_child_remove_child_and_replace_children() {
        let processed = process_document_scripts(
            "<html><body><div id=\"box\"></div><script>var box = document.getElementById('box'); var first = document.createElement('i'); first.textContent = '1'; var second = document.createElement('b'); second.textContent = '2'; box.append(first); box.append(second); var fresh = document.createElement('u'); fresh.textContent = '3'; box.replaceChild(fresh, first); box.removeChild(fresh); box.replaceChildren('N', document.createElement('em'), 'M');</script></body></html>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert!(
            processed.html.contains("<div id=\"box\">N<em></em>M</div>"),
            "{}",
            processed.html
        );
    }

    #[test]
    fn supports_outer_html_and_insert_adjacent_html() {
        let processed = process_document_scripts(
            "<html><body><div id=\"app\"><p id=\"a\">A</p></div><script>var app = document.getElementById('app'); var p = document.getElementById('a'); var before = p.outerHTML; p.outerHTML = '<span id=\"b\">B</span>'; app.insertAdjacentHTML('afterbegin', '<em id=\"c\">C</em>'); app.insertAdjacentHTML('beforeend', '<strong id=\"d\">D</strong>'); document.body.setAttribute('data-html', [before, app.innerHTML].join('|'));</script></body></html>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert!(processed.html.contains("data-html="), "{}", processed.html);
        assert!(
            processed
                .html
                .contains("<em id=\"c\">C</em><span id=\"b\">B</span><strong id=\"d\">D</strong>"),
            "{}",
            processed.html
        );
    }

    #[test]
    fn supports_dom_matches_closest_contains_and_element_siblings() {
        let processed = process_document_scripts(
            "<html><body><section id=\"outer\"><div id=\"wrap\" class=\"wrap\"><button id=\"btn\">Go</button><span id=\"tail\">Later</span></div></section><script>var btn = document.getElementById('btn'); var wrap = btn.closest('#wrap'); var outer = btn.closest('section'); var tail = btn.nextElementSibling; outer.setAttribute('data-dom', [btn.matches('button#btn'), !btn.matches('.wrap'), wrap === document.getElementById('wrap'), outer === document.getElementById('outer'), outer.contains(btn), btn.contains(btn), outer.firstElementChild === wrap, outer.lastElementChild === wrap, wrap.firstElementChild === btn, wrap.lastElementChild === tail, btn.previousElementSibling === null, tail && tail.id === 'tail', tail.previousElementSibling === btn, wrap.nextElementSibling === null].join('|'));</script></body></html>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert!(processed.html.contains(
            "data-dom=\"true|true|true|true|true|true|true|true|true|true|true|true|true|true\""
        ));
    }

    #[test]
    fn supports_attribute_introspection_helpers() {
        let processed = process_document_scripts(
            "<html><body><div id=\"box\" data-a=\"1\" title=\"x\"></div><script>var box = document.getElementById('box'); box.setAttribute('aria-label', 'demo'); box.setAttribute('data-b', '2'); box.setAttribute('data-c', '3'); box.setAttribute('role', 'button'); box.removeAttribute('title'); document.body.setAttribute('data-attrs', [box.hasAttribute('title'), box.hasAttribute('data-b'), box.getAttributeNames().join('|')].join('|'));</script></body></html>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert!(
            processed
                .html
                .contains("data-attrs=\"false|true|aria-label|data-a|data-b|data-c|id|role\"")
        );
    }

    #[test]
    fn supports_attribute_collection_accessors_and_iteration() {
        let processed = process_document_scripts(
            "<html><body><div id=\"box\" data-foo-bar=\"one\" title=\"x\"></div><script>var box = document.getElementById('box'); var attrs = box.attributes; var named = attrs.getNamedItem('data-foo-bar'); var from = Array.from(attrs).map(function(attr) { return attr.name + '=' + attr.value; }).join('|'); document.body.setAttribute('data-attrs', [attrs.length, attrs.item(0).name, named.value, attrs['title'].value, from].join('|'));</script></body></html>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert!(
            processed
                .html
                .contains("data-attrs=\"3|data-foo-bar|one|x|data-foo-bar=one|id=box|title=x\""),
            "{}",
            processed.html
        );
    }

    #[test]
    fn supports_toggle_attribute_and_class_list_replace() {
        let processed = process_document_scripts(
            "<html><body><div id=\"box\" class=\"one two\" data-a=\"1\"></div><script>var box = document.getElementById('box'); var toggledOn = box.toggleAttribute('hidden'); var toggledOff = box.toggleAttribute('hidden', false); var replaced = box.classList.replace('one', 'uno'); var forced = box.classList.toggle('two', false); var value = box.classList.value; var length = box.classList.length; var first = box.classList.item(0); var asString = box.classList.toString(); document.body.setAttribute('data-toggle', [toggledOn, toggledOff, box.hasAttributes(), replaced, value, length, first, asString, box.getAttribute('class'), box.classList.contains('uno'), box.classList.contains('two'), box.toggleAttribute('data-swap', true), box.toggleAttribute('data-swap')].join('|'));</script></body></html>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert!(
            processed.html.contains(
                "data-toggle=\"true|false|true|true|uno|1|uno|uno|uno|true|false|true|false\""
            ),
            "{}",
            processed.html
        );
    }

    #[test]
    fn supports_mutation_observer_callbacks_for_attributes_and_child_list() {
        let processed = process_document_scripts(
            "<html><body><div id=\"box\"></div><script>var box = document.getElementById('box'); var log = []; var observer = new MutationObserver(function(records) { log.push(records.map(function(record) { return record.type + ':' + record.target.id + ':' + record.addedNodes.length + ':' + record.removedNodes.length; }).join(',')); document.body.setAttribute('data-log', log.join(';')); }); observer.observe(box, { attributes: true, childList: true }); box.setAttribute('data-x', '1'); var child = document.createElement('span'); child.textContent = 'hello'; box.appendChild(child);</script></body></html>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert!(
            processed
                .html
                .contains("data-log=\"attributes:box:0:0;childList:box:1:0\""),
            "{}",
            processed.html
        );
    }

    #[test]
    fn supports_event_constructors_and_dispatch_event_payloads() {
        let processed = process_document_scripts(
            "<html><body><div id=\"box\"></div><script>var box = document.getElementById('box'); box.addEventListener('keydown', function(event) { document.body.setAttribute('data-key', [event.type, event.key, event.code, String(event.ctrlKey), String(event.repeat)].join('|')); }); box.addEventListener('custom', function(event) { document.body.setAttribute('data-detail', event.detail); }); box.dispatchEvent(new KeyboardEvent('keydown', { key: 'a', code: 'KeyA', ctrlKey: true, repeat: true, bubbles: true, cancelable: true })); box.dispatchEvent(new CustomEvent('custom', { detail: 'hello', bubbles: true }));</script></body></html>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert!(
            processed
                .html
                .contains("data-key=\"keydown|a|KeyA|true|true\""),
            "{}",
            processed.html
        );
        assert!(
            processed.html.contains("data-detail=\"hello\""),
            "{}",
            processed.html
        );
    }

    #[test]
    fn supports_abort_controller_and_fetch_signal_abort() {
        let processed = process_document_scripts(
            "<html><body><script>var controller = new AbortController(); controller.signal.addEventListener('abort', function () { document.body.setAttribute('data-signal', String(controller.signal.aborted)); }); controller.abort('stop'); fetch('https://example.com/', { signal: controller.signal }).then(function () { document.body.setAttribute('data-fetch', 'ok'); }).catch(function () { document.body.setAttribute('data-fetch', 'aborted'); });</script></body></html>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert!(
            processed.html.contains("data-signal=\"true\""),
            "{}",
            processed.html
        );
        assert!(
            processed.html.contains("data-fetch=\"aborted\""),
            "{}",
            processed.html
        );
    }

    #[test]
    fn supports_dataset_live_reflection_and_updates() {
        let processed = process_document_scripts(
            "<html><body><div id=\"box\" data-foo-bar=\"one\"></div><script>var box = document.getElementById('box'); var before = box.dataset.fooBar; var builtin = box.dataset.toString === Object.prototype.toString; box.dataset.fooBar = 'updated'; var after = box.getAttribute('data-foo-bar'); var live = box.dataset.fooBar; document.body.setAttribute('data-before', before); document.body.setAttribute('data-after', after); document.body.setAttribute('data-live', live); document.body.setAttribute('data-builtin', builtin);</script></body></html>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert!(
            processed.html.contains("data-before=\"one\""),
            "{}",
            processed.html
        );
        assert!(
            processed.html.contains("data-after=\"updated\""),
            "{}",
            processed.html
        );
        assert!(
            processed.html.contains("data-live=\"updated\""),
            "{}",
            processed.html
        );
        assert!(
            processed.html.contains("data-builtin=\"true\""),
            "{}",
            processed.html
        );
    }

    #[test]
    fn response_clone_errors_on_invalid_receiver() {
        let mut context = Context::default();
        let result = super::js_fetch_response_clone(&JsValue::undefined(), &[], &mut context);

        assert!(result.is_err());
    }

    #[test]
    fn supports_response_headers_iteration_and_xhr_header_access() {
        let mut runtime = JavaScriptRuntime::new(
            &Url::parse("https://example.com").unwrap(),
            "<html><body></body></html>",
        );
        let response = HttpResponse {
            final_url: Url::parse("https://example.com/data").unwrap(),
            status_code: 200,
            reason_phrase: "OK".to_string(),
            headers: std::collections::HashMap::from([
                ("content-type".to_string(), "application/json".to_string()),
                ("x-demo".to_string(), "one".to_string()),
            ]),
            set_cookie_headers: Vec::new(),
            body: br#"{"ok":true}"#.to_vec(),
        };
        let response_object = build_fetch_response_object(&mut runtime.context, response);
        runtime
            .context
            .global_object()
            .set(
                js_string!("resp"),
                JsValue::from(response_object),
                true,
                &mut runtime.context,
            )
            .unwrap();

        let fetch_summary = runtime
            .context
            .eval(Source::from_bytes(
                "(()=>{var h=resp.headers; var seen=[]; h.forEach(function(value, name, source){ seen.push(name+'='+value+'='+(source===h)); }); return [h.get('content-type'), h.has('x-demo'), h.keys().join('|'), h.values().join('|'), h.entries().map(function(pair){ return pair[0]+'='+pair[1]; }).join('|'), h.toString(), seen.join('|')].join('||'); })()",
            ))
            .unwrap();
        let fetch_summary = js_value_to_string(&fetch_summary, &mut runtime.context).unwrap();
        assert!(fetch_summary.contains(
            "application/json||true||content-type|x-demo||application/json|one||content-type=application/json|x-demo=one||content-type: application/json"
        ));

        let xhr_object = build_xml_http_request_object(&mut runtime.context);
        {
            let handle = xhr_object
                .downcast_ref::<XmlHttpRequestHandle>()
                .expect("xhr handle");
            let mut state = handle.state.borrow_mut();
            state.ready_state = 4;
            state.status = 200;
            state.status_text = "OK".to_string();
            state.response_text = "payload".to_string();
            state.response_url = "https://example.com/data".to_string();
            state.response_headers = std::collections::HashMap::from([
                ("content-type".to_string(), "text/plain".to_string()),
                ("x-demo".to_string(), "two".to_string()),
            ])
            .into_iter()
            .collect();
        }
        runtime
            .context
            .global_object()
            .set(
                js_string!("xhr"),
                JsValue::from(xhr_object),
                true,
                &mut runtime.context,
            )
            .unwrap();

        let xhr_summary = runtime
            .context
            .eval(Source::from_bytes(
                "(()=>[xhr.getResponseHeader('content-type'), xhr.getResponseHeader('x-demo'), xhr.getAllResponseHeaders()].join('|'))()",
            ))
            .unwrap();
        let xhr_summary = js_value_to_string(&xhr_summary, &mut runtime.context).unwrap();
        assert!(xhr_summary.contains("text/plain|two|content-type: text/plain"));
    }
}
