use std::sync::mpsc::{self, Sender};
use std::thread;

use crate::url::Url;

const JS_THREAD_STACK_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProcessedScriptHtml {
    pub html: String,
    pub title_override: Option<String>,
    pub console_logs: Vec<String>,
    pub navigation_target: Option<String>,
    pub soft_navigation_target: Option<String>,
    pub scroll_y: u32,
    /// Whether the JS engine still has pending event-loop work (timers / RAF /
    /// queued tasks). When true, the GUI keeps ticking the session so
    /// `setInterval` / `setTimeout(fn, delay)` / animation loops fire over time.
    /// Always false for the boa backend (it drains synchronously).
    pub has_pending_work: bool,
}

#[derive(Debug, Clone, Default)]
pub struct DomEventRequest {
    pub target_node_id: usize,
    pub event_type: String,
    pub bubbles: bool,
    pub cancelable: bool,
    pub composed: bool,
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
    /// Feed element geometry from the browser's latest layout so
    /// `getBoundingClientRect`/`offsetWidth` return real values (engine path).
    SetGeometry {
        rects: Vec<(usize, f32, f32, f32, f32)>,
    },
    SetAttribute {
        node_id: usize,
        name: String,
        value: String,
    },
    Snapshot {
        response_tx: Sender<ProcessedScriptHtml>,
    },
    /// Advance time and run due timers / RAF. Reply with `Some(snapshot)` only
    /// when something actually ran this frame (so an idle-but-pending page does
    /// not pay a full DOM serialize every tick), plus whether more event-loop
    /// work remains (so the caller keeps pumping).
    Tick {
        now_ms: u64,
        response_tx: Sender<(Option<ProcessedScriptHtml>, bool)>,
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

    /// Advance time and run due timers / RAF. The inner `Option` is `Some` only
    /// when the frame did work (so callers skip re-applying an unchanged
    /// snapshot); the `bool` reports whether more event-loop work remains. The
    /// outer `Option` is `None` if the worker is gone.
    pub(crate) fn tick(&self, now_ms: u64) -> Option<(Option<ProcessedScriptHtml>, bool)> {
        let (response_tx, response_rx) = mpsc::channel();
        if self
            .command_tx
            .send(JavaScriptSessionCommand::Tick { now_ms, response_tx })
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

    pub(crate) fn set_geometry(&self, rects: Vec<(usize, f32, f32, f32, f32)>) -> bool {
        self.command_tx
            .send(JavaScriptSessionCommand::SetGeometry { rects })
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

// NOTE: deliberately NO `Drop` that sends `Shutdown`. `JavaScriptSession` is
// `Clone` (e.g. `set_dom_attribute` clones it to dodge a borrow), and a `Drop`
// that sent `Shutdown` would let any short-lived clone kill the shared worker —
// which is exactly what broke interaction after focusing an input. The worker
// already exits cleanly when every sender is dropped (its `recv()` returns
// `Err` once the page's session goes away), so no explicit shutdown is needed.
pub fn process_document_scripts(html: &str, base_url: &Url) -> ProcessedScriptHtml {
    let (processed, session) = start_document_script_session(html, base_url);
    drop(session);
    processed
}

/// Run a document's scripts on the self-built engine and adapt the result to
/// the `ProcessedScriptHtml` shape the browser expects. This is the Stage 2
/// flag-gated path: it produces the initial post-script snapshot but does not
/// yet drive an interactive event session (hence the caller returns `None` for
/// the `JavaScriptSession`).
/// Adapt an engine `EngineRunResult` into the browser's `ProcessedScriptHtml`,
/// surfacing any uncaught engine error as a console line.
fn engine_result_to_processed(result: crate::engine_host::EngineRunResult) -> ProcessedScriptHtml {
    let mut console_logs = result.console_logs;
    if let Some(error) = result.error {
        console_logs.push(format!("[tobira-engine] uncaught error: {error}"));
    }
    ProcessedScriptHtml {
        html: result.html,
        title_override: result.title,
        console_logs,
        navigation_target: result.navigation_target,
        soft_navigation_target: result.soft_navigation_target,
        scroll_y: result.scroll_y,
        has_pending_work: result.has_pending_work,
    }
}

fn process_document_scripts_with_engine(html: &str, base_url: &Url) -> ProcessedScriptHtml {
    engine_result_to_processed(crate::engine_host::run_document_scripts(
        html,
        &base_url.to_string(),
    ))
}

/// Start an interactive document-script session on the self-built engine
/// (the `TOBIRA_ENGINE` path). Spawns a worker thread that owns the `Vm` +
/// `BrowserHost` (the `Vm` is not `Send`, so it is created inside the thread)
/// and services the same `JavaScriptSessionCommand` protocol the boa worker
/// does, so the returned `JavaScriptSession` is a drop-in for callers.
fn start_engine_script_session(
    html: &str,
    base_url: &Url,
) -> (ProcessedScriptHtml, Option<JavaScriptSession>) {
    let html_owned = html.to_string();
    let url_str = base_url.to_string();
    let (ready_tx, ready_rx) = mpsc::channel();
    let (command_tx, command_rx) = mpsc::channel();
    let worker = thread::Builder::new()
        .name("tobira-engine-js".to_string())
        .stack_size(JS_THREAD_STACK_BYTES)
        .spawn(move || {
            let (mut session, initial) =
                crate::engine_host::EngineSession::start(&html_owned, &url_str);
            let _ = ready_tx.send(engine_result_to_processed(initial));
            while let Ok(command) = command_rx.recv() {
                match command {
                    JavaScriptSessionCommand::DispatchEvent {
                        request,
                        response_tx,
                    } => {
                        let init = tobira_engine::engine::DomEventInit {
                            bubbles: request.bubbles,
                            cancelable: request.cancelable,
                            key: request.key.clone(),
                            code: request.code.clone(),
                            data: request.data.clone(),
                            input_type: request.input_type.clone(),
                            client_x: request.client_x,
                            client_y: request.client_y,
                            button: request.button,
                            buttons: request.buttons,
                            alt_key: request.alt_key,
                            ctrl_key: request.ctrl_key,
                            shift_key: request.shift_key,
                            meta_key: request.meta_key,
                            repeat: request.repeat,
                            is_composing: request.is_composing,
                        };
                        let result = session.dispatch_event(
                            request.target_node_id,
                            &request.event_type,
                            &init,
                        );
                        let default_prevented = result.default_prevented;
                        let _ = response_tx.send(DomEventDispatchResult {
                            snapshot: engine_result_to_processed(result),
                            default_prevented,
                        });
                    }
                    JavaScriptSessionCommand::DispatchGlobalEvent {
                        event_type,
                        response_tx,
                        ..
                    } => {
                        // When nothing listens for the event, drop `response_tx`
                        // without sending: the caller's `recv()` then yields
                        // `None`, signalling "no-op" so it skips re-applying a
                        // snapshot (and the relayout that follows). This keeps
                        // scroll/resize cheap on listener-free pages.
                        if let Some(result) = session.dispatch_global_event(&event_type) {
                            let _ = response_tx.send(DomEventDispatchResult {
                                snapshot: engine_result_to_processed(result),
                                default_prevented: false,
                            });
                        }
                    }
                    JavaScriptSessionCommand::SetAttribute {
                        node_id,
                        name,
                        value,
                    } => {
                        session.set_attribute(node_id, &name, &value);
                    }
                    JavaScriptSessionCommand::Snapshot { response_tx } => {
                        let _ = response_tx.send(engine_result_to_processed(session.snapshot()));
                    }
                    JavaScriptSessionCommand::Tick { now_ms, response_tx } => {
                        // Only serialize a snapshot when the frame did work; a
                        // page with a pending interval but nothing due this frame
                        // shouldn't pay a full DOM serialize every ~16ms.
                        let did_work = session.pump(now_ms);
                        let has_more = session.has_pending_work();
                        let snapshot = did_work
                            .then(|| engine_result_to_processed(session.snapshot()));
                        let _ = response_tx.send((snapshot, has_more));
                    }
                    JavaScriptSessionCommand::SetScrollPosition { y } => {
                        session.set_scroll_position(y);
                    }
                    JavaScriptSessionCommand::SetViewportSize { width, height } => {
                        session.set_viewport_size(width, height);
                    }
                    JavaScriptSessionCommand::SetGeometry { rects } => {
                        session.set_geometry(&rects);
                    }
                    JavaScriptSessionCommand::Shutdown => break,
                }
            }
        });
    match worker {
        Ok(_) => match ready_rx.recv() {
            Ok(processed) => (processed, Some(JavaScriptSession { command_tx })),
            Err(_) => (
                process_document_scripts_error(
                    html.to_string(),
                    "engine error: worker failed to initialize".to_string(),
                ),
                None,
            ),
        },
        Err(_) => (
            process_document_scripts_error(
                html.to_string(),
                "engine error: failed to start worker".to_string(),
            ),
            None,
        ),
    }
}

pub fn start_document_script_session(
    html: &str,
    base_url: &Url,
) -> (ProcessedScriptHtml, Option<JavaScriptSession>) {
    start_engine_script_session(html, base_url)
}

fn process_document_scripts_error(html: String, message: String) -> ProcessedScriptHtml {
    ProcessedScriptHtml {
        html,
        title_override: None,
        console_logs: vec![message],
        navigation_target: None,
        soft_navigation_target: None,
        scroll_y: 0,
        has_pending_work: false,
    }
}

#[cfg(test)]
mod tests {

    use super::{
        DomEventRequest, process_document_scripts,
        process_document_scripts_with_engine, start_document_script_session,
        start_engine_script_session,
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

    // Stage 2: the self-built engine backend adapter. These exercise the
    // engine path directly (without mutating the TOBIRA_ENGINE env var, which
    // would race with the parallel boa-backed tests in this binary).
    #[test]
    fn engine_backend_runs_dom_mutation_and_console() {
        let processed = process_document_scripts_with_engine(
            r#"<html><body><div id="app">old</div><script>
                document.getElementById('app').textContent = 'new';
                console.log('ran on engine');
            </script></body></html>"#,
            &Url::parse("https://example.com/page").unwrap(),
        );
        assert!(
            processed.html.contains(">new</div>"),
            "html: {}",
            processed.html
        );
        assert!(!processed.html.contains(">old</div>"));
        assert_eq!(processed.console_logs, vec!["ran on engine".to_string()]);
    }

    #[test]
    fn engine_backend_maps_title_override() {
        let processed = process_document_scripts_with_engine(
            "<html><head><title>EngineTitle</title></head><body></body></html>",
            &Url::parse("https://example.com").unwrap(),
        );
        assert_eq!(processed.title_override.as_deref(), Some("EngineTitle"));
    }

    #[test]
    fn engine_backend_reports_uncaught_error_to_console() {
        let processed = process_document_scripts_with_engine(
            "<html><body><script>throw new Error('boom')</script></body></html>",
            &Url::parse("https://example.com").unwrap(),
        );
        assert!(
            processed
                .console_logs
                .iter()
                .any(|line| line.contains("tobira-engine") && line.contains("boom")),
            "expected an engine error line, got: {:?}",
            processed.console_logs
        );
    }

    #[test]
    fn engine_session_click_through_javascript_session_api() {
        use crate::browser::annotate_node_ids;
        use crate::html::{Node, parse_document};

        fn find_btn(node: &Node) -> Option<usize> {
            if let Node::Element(el) = node {
                if el.attributes.get("id").map(String::as_str) == Some("btn") {
                    return el
                        .attributes
                        .get("data-tobira-node-id")
                        .and_then(|s| s.parse().ok());
                }
                for child in &el.children {
                    if let Some(found) = find_btn(child) {
                        return Some(found);
                    }
                }
            }
            None
        }

        // Drive the engine path directly (no TOBIRA_ENGINE env mutation, which
        // would race the parallel boa tests).
        let html = r#"<html><body>
            <button id="btn">go</button>
            <div id="out">idle</div>
            <script>
                document.getElementById('btn').addEventListener('click', () => {
                    document.getElementById('out').textContent = 'clicked';
                });
            </script>
        </body></html>"#;
        let (initial, session) =
            start_engine_script_session(html, &Url::parse("http://localhost/").unwrap());
        let session = session.expect("engine session present");
        assert!(initial.html.contains(">idle</div>"));

        let mut tree = parse_document(&initial.html);
        annotate_node_ids(&mut tree);
        let btn_id = find_btn(&tree).expect("button node id");

        let result = session
            .dispatch_event(DomEventRequest {
                target_node_id: btn_id,
                event_type: "click".to_string(),
                ..Default::default()
            })
            .expect("dispatch result");
        assert!(
            result.snapshot.html.contains(">clicked</div>"),
            "expected click via the session API to mutate the DOM, got: {}",
            result.snapshot.html
        );
    }

    #[test]
    fn engine_session_survives_dropped_clone() {
        use crate::browser::annotate_node_ids;
        use crate::html::{Node, parse_document};

        fn find_btn(node: &Node) -> Option<usize> {
            if let Node::Element(el) = node {
                if el.attributes.get("id").map(String::as_str) == Some("btn") {
                    return el
                        .attributes
                        .get("data-tobira-node-id")
                        .and_then(|s| s.parse().ok());
                }
                for child in &el.children {
                    if let Some(found) = find_btn(child) {
                        return Some(found);
                    }
                }
            }
            None
        }

        // Regression: `set_dom_attribute` clones the session and the clone is
        // dropped right after use (focusing an input does this). A `Drop` that
        // sent `Shutdown` would kill the shared worker — breaking every later
        // event. Cloning + dropping must leave the worker alive.
        let html = r#"<html><body>
            <button id="btn">go</button>
            <div id="out">idle</div>
            <script>
                document.getElementById('btn').addEventListener('click', () => {
                    document.getElementById('out').textContent = 'clicked';
                });
            </script>
        </body></html>"#;
        let (initial, session) =
            start_engine_script_session(html, &Url::parse("http://localhost/").unwrap());
        let session = session.expect("engine session present");

        // Use and drop a clone, exactly like set_dom_attribute does.
        {
            let clone = session.clone();
            let _ = clone.snapshot();
        }

        let mut tree = parse_document(&initial.html);
        annotate_node_ids(&mut tree);
        let btn_id = find_btn(&tree).expect("button node id");

        let result = session
            .dispatch_event(DomEventRequest {
                target_node_id: btn_id,
                event_type: "click".to_string(),
                ..Default::default()
            })
            .expect("session must still be alive after a clone was dropped");
        assert!(
            result.snapshot.html.contains(">clicked</div>"),
            "click after clone-drop should still work, got: {}",
            result.snapshot.html
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
    fn supports_character_data_mutation_observers_and_split_text() {
        let processed = process_document_scripts(
            "<html><body><div id=\"box\"></div><script>var box = document.getElementById('box'); var text = document.createTextNode('abc'); box.appendChild(text); var log = []; var observer = new MutationObserver(function(records) { log.push(records.map(function(record) { return record.type + ':' + record.oldValue; }).join(',')); }); observer.observe(text, { characterData: true, characterDataOldValue: true }); text.nodeValue = 'xyz'; var tail = text.splitText(1); document.body.setAttribute('data-char', [text.data, text.length, tail.data, tail.length, text.nextSibling === tail, tail.previousSibling === text, log.join(';')].join('|'));</script></body></html>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert!(
            processed
                .html
                .contains("data-char=\"x|1|yz|2|true|true|characterData:abc;characterData:xyz\""),
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
    fn supports_custom_elements_upgrade_and_attribute_callbacks() {
        let processed = process_document_scripts(
            "<html><body><x-box id=\"box\" data-ready=\"yes\"></x-box><script>class XBox extends HTMLElement { connectedCallback() { document.title = [this.getAttribute('data-ready'), String(this instanceof XBox)].join('|'); } } customElements.define('x-box', XBox);</script></body></html>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert_eq!(processed.title_override.as_deref(), Some("yes|true"));
    }

    #[test]
    fn supports_custom_elements_attribute_changed_callback() {
        let processed = process_document_scripts(
            "<html><body><x-attr id=\"attr\"></x-attr><script>class XAttr extends HTMLElement { static get observedAttributes() { return ['data-state']; } attributeChangedCallback(name, oldValue, newValue) { document.title = [name, oldValue === null ? 'null' : oldValue, newValue].join('|'); } } customElements.define('x-attr', XAttr); document.getElementById('attr').setAttribute('data-state', 'live');</script></body></html>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert_eq!(
            processed.title_override.as_deref(),
            Some("data-state|null|live")
        );
    }

    #[test]
    fn supports_attach_shadow_get_root_node_and_slots() {
        let processed = process_document_scripts(
            "<html><body><div id=\"host\"><span slot=\"lead\">Light</span></div><script>var host = document.getElementById('host'); var shadow = host.attachShadow({ mode: 'open' }); shadow.innerHTML = '<slot name=\"lead\"></slot><span id=\"shadow\">S</span>'; var slot = shadow.querySelector('slot'); document.title = [String(host.shadowRoot === shadow), String(shadow.host === host), String(shadow.getRootNode() === shadow), String(shadow.getRootNode({ composed: true }) === document), String(slot.assignedNodes().length), String(slot.assignedElements().length)].join('|');</script></body></html>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert_eq!(
            processed.title_override.as_deref(),
            Some("true|true|true|true|1|1")
        );
    }

    #[test]
    fn supports_assigned_slot_and_slotchange_events() {
        let processed = process_document_scripts(
            "<html><body><div id=\"host\"></div><script>var host = document.getElementById('host'); var shadow = host.attachShadow({ mode: 'open' }); shadow.innerHTML = '<slot name=\"lead\"></slot>'; var slot = shadow.querySelector('slot'); var light = document.createElement('span'); light.id = 'light'; light.setAttribute('slot', 'lead'); light.textContent = 'Light'; slot.addEventListener('slotchange', function () { document.body.setAttribute('data-slotchange', [String(slot.assignedNodes().length), String(slot.assignedElements().length), String(light.assignedSlot === slot)].join('|')); }); host.appendChild(light); document.body.setAttribute('data-assigned', String(light.assignedSlot === slot));</script></body></html>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert!(
            processed.html.contains("data-slotchange=\"1|1|true\""),
            "{}",
            processed.html
        );
        assert!(
            processed.html.contains("data-assigned=\"true\""),
            "{}",
            processed.html
        );
    }

    #[test]
    fn retargets_shadow_dom_events_and_exposes_composed_path() {
        let processed = process_document_scripts(
            "<html><body><div id=\"host\"></div><script>var host = document.getElementById('host'); var shadow = host.attachShadow({ mode: 'open' }); shadow.innerHTML = '<button id=\"btn\">Go</button>'; var btn = shadow.querySelector('#btn'); host.addEventListener('click', function (event) { document.title = [String(typeof event.composedPath), String(event.composedPath().length), String(event.target === host), String(event.currentTarget === host), String(event.composedPath()[0] === btn)].join('|'); }); btn.dispatchEvent(new MouseEvent('click', { bubbles: true, composed: true }));</script></body></html>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert_eq!(
            processed.title_override.as_deref(),
            Some("function|6|true|true|true")
        );
    }
}
