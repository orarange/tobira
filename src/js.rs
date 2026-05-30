/// JavaScript integration — new tobira-engine only (boa removed in Phase 8).
///
/// Public surface is kept compatible with the previous boa-backed version so
/// that `browser.rs` call-sites need no changes.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::Duration;

pub use crate::js_common::ProcessedScriptHtml;

/// Kept for API compatibility with gui.rs (new engine has no pending fetches).
pub(crate) const JS_FETCH_SETTLE_TIMEOUT: Duration = Duration::from_secs(0);
use crate::layout::ElementHitbox;
use crate::url::Url;

// ---------------------------------------------------------------------------
// Public types (kept API-compatible with old boa version)
// ---------------------------------------------------------------------------

/// Stub type kept for API compatibility; no actual fetches are tracked yet.
#[derive(Debug, Clone)]
pub struct CompletedFetch {
    pub id: usize,
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
    pub related_target_node_id: Option<usize>,
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

// ---------------------------------------------------------------------------
// New-engine worker commands
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum NewEngineCommand {
    DispatchEvent {
        node_handle: u32,
        event_type: String,
        response_tx: Sender<ProcessedScriptHtml>,
    },
    DispatchGlobalEvent {
        event_type: String,
        response_tx: Sender<ProcessedScriptHtml>,
    },
    Snapshot {
        response_tx: Sender<ProcessedScriptHtml>,
    },
    SetAttribute {
        node_id: u32,
        name: String,
        value: String,
    },
    Shutdown,
}

// ---------------------------------------------------------------------------
// JavaScriptSession
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct JavaScriptSession {
    command_tx: Sender<NewEngineCommand>,
}

impl JavaScriptSession {
    pub(crate) fn dispatch_event(&self, request: DomEventRequest) -> Option<DomEventDispatchResult> {
        let (response_tx, response_rx) = mpsc::channel();
        self.command_tx.send(NewEngineCommand::DispatchEvent {
            node_handle: request.target_node_id as u32,
            event_type: request.event_type,
            response_tx,
        }).ok()?;
        let snapshot = response_rx.recv().ok()?;
        Some(DomEventDispatchResult { snapshot, default_prevented: false })
    }

    pub(crate) fn snapshot(&self) -> Option<ProcessedScriptHtml> {
        let (response_tx, response_rx) = mpsc::channel();
        self.command_tx.send(NewEngineCommand::Snapshot { response_tx }).ok()?;
        response_rx.recv().ok()
    }

    pub(crate) fn has_pending_fetches(&self) -> bool {
        false
    }

    pub(crate) fn fetch_result_queue(&self) -> Arc<Mutex<VecDeque<CompletedFetch>>> {
        Arc::new(Mutex::new(VecDeque::new()))
    }

    pub(crate) fn set_attribute(&self, node_id: usize, name: &str, value: &str) -> bool {
        self.command_tx.send(NewEngineCommand::SetAttribute {
            node_id: node_id as u32,
            name: name.to_string(),
            value: value.to_string(),
        }).is_ok()
    }

    pub(crate) fn set_layout_hitboxes(&self, _hitboxes: Vec<ElementHitbox>) -> bool {
        true
    }

    pub(crate) fn set_viewport_size(&self, _width: u32, _height: u32) -> bool {
        true
    }

    pub(crate) fn set_scroll_position(&self, _y: u32) -> bool {
        true
    }

    pub(crate) fn dispatch_global_event(
        &self,
        event_type: &str,
        _bubbles: bool,
        _cancelable: bool,
    ) -> Option<DomEventDispatchResult> {
        let (response_tx, response_rx) = mpsc::channel();
        self.command_tx.send(NewEngineCommand::DispatchGlobalEvent {
            event_type: event_type.to_string(),
            response_tx,
        }).ok()?;
        let snapshot = response_rx.recv().ok()?;
        Some(DomEventDispatchResult { snapshot, default_prevented: false })
    }

    pub(crate) fn has_global_event_listener(&self, _event_type: &str) -> bool {
        true
    }
}

impl Drop for JavaScriptSession {
    fn drop(&mut self) {
        let _ = self.command_tx.send(NewEngineCommand::Shutdown);
    }
}

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

pub fn process_document_scripts(html: &str, base_url: &Url) -> ProcessedScriptHtml {
    let (processed, _session) = start_document_script_session(html, base_url);
    processed
}

pub fn start_document_script_session(
    html: &str,
    base_url: &Url,
) -> (ProcessedScriptHtml, Option<JavaScriptSession>) {
    start_new_engine_session(html, base_url)
}

// ---------------------------------------------------------------------------
// Worker thread
// ---------------------------------------------------------------------------

fn start_new_engine_session(
    html: &str,
    base_url: &Url,
) -> (ProcessedScriptHtml, Option<JavaScriptSession>) {
    let html_owned = html.to_string();
    let base_url_owned = base_url.clone();
    let (init_tx, init_rx) = mpsc::channel::<ProcessedScriptHtml>();
    let (command_tx, command_rx) = mpsc::channel::<NewEngineCommand>();

    thread::Builder::new()
        .name("tobira-engine".to_string())
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            use crate::js_host::{dispatch_event_on_vm, snapshot_from_vm, start_new_engine_vm};
            use tobira_engine::engine::{DomMutation, NodeId};

            let (initial_snapshot, mut vm) = start_new_engine_vm(&html_owned, &base_url_owned);
            let _ = init_tx.send(initial_snapshot);

            for cmd in command_rx {
                match cmd {
                    NewEngineCommand::DispatchEvent { node_handle, event_type, response_tx } => {
                        let snap = dispatch_event_on_vm(&mut vm, node_handle, &event_type);
                        let _ = response_tx.send(snap);
                    }
                    NewEngineCommand::DispatchGlobalEvent { event_type, response_tx } => {
                        let snap = dispatch_event_on_vm(&mut vm, 0, &event_type);
                        let _ = response_tx.send(snap);
                    }
                    NewEngineCommand::Snapshot { response_tx } => {
                        let snap = snapshot_from_vm(&mut vm);
                        let _ = response_tx.send(snap);
                    }
                    NewEngineCommand::SetAttribute { node_id, name, value } => {
                        let _ = vm.host_mut().mutate_dom(DomMutation::SetAttribute {
                            node: NodeId(node_id),
                            name,
                            value,
                        });
                    }
                    NewEngineCommand::Shutdown => break,
                }
            }
        })
        .expect("failed to spawn tobira-engine worker thread");

    let initial_snapshot = match init_rx.recv() {
        Ok(s) => s,
        Err(_) => return (ProcessedScriptHtml { html: html.to_string(), ..Default::default() }, None),
    };

    (initial_snapshot, Some(JavaScriptSession { command_tx }))
}
