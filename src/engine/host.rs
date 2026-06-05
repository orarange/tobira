use std::any::Any;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TimerId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NetworkRequestId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ObserverId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostError {
    Unsupported,
    InvalidHandle,
    NotFound,
    Security,
    Network,
    InvalidState,
}

pub type HostResult<T> = Result<T, HostError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleLevel {
    Debug,
    Log,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleMessage {
    pub level: ConsoleLevel,
    pub parts: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Document,
    Element,
    Text,
    DocumentFragment,
    ShadowRoot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SiblingDirection {
    Previous,
    Next,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageAreaKind {
    Local,
    Session,
    Cookie,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageAreaScope {
    Window(WindowId),
    Origin(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct LocationSnapshot {
    pub href: String,
    pub origin: String,
    pub protocol: String,
    pub host: String,
    pub hostname: String,
    pub port: String,
    pub pathname: String,
    pub search: String,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WindowMetrics {
    pub inner_width: f64,
    pub inner_height: f64,
    pub scroll_x: f64,
    pub scroll_y: f64,
    pub device_pixel_ratio: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HostData {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Node(NodeId),
    Array(Vec<HostData>),
    Object(Vec<(String, HostData)>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum NavigationAction {
    Navigate {
        window: WindowId,
        url: String,
        replace: bool,
    },
    SetHash {
        window: WindowId,
        hash: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavigationOutcome {
    pub committed: bool,
    pub same_document: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HistoryAction {
    PushState {
        window: WindowId,
        url: Option<String>,
        title: String,
        state: HostData,
    },
    ReplaceState {
        window: WindowId,
        url: Option<String>,
        title: String,
        state: HostData,
    },
    Back {
        window: WindowId,
    },
    Forward {
        window: WindowId,
    },
    Go {
        window: WindowId,
        delta: i32,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct HistoryOutcome {
    pub href: String,
    pub state: Option<HostData>,
    pub length: usize,
    pub restored_scroll_y: Option<f64>,
}

// Phase 0 note: these enums are intentionally broad command surfaces so the
// dead-code scaffold can capture the current `src/js.rs` host footprint without
// committing to final wrapper-object splitting yet.
#[derive(Debug, Clone, PartialEq)]
pub enum DomRead {
    DocumentRoot {
        window: WindowId,
    },
    DocumentHead {
        window: WindowId,
    },
    DocumentBody {
        window: WindowId,
    },
    ActiveElement {
        window: WindowId,
    },
    QuerySelector {
        root: NodeId,
        selectors: String,
    },
    QuerySelectorAll {
        root: NodeId,
        selectors: String,
    },
    Matches {
        node: NodeId,
        selectors: String,
    },
    Closest {
        node: NodeId,
        selectors: String,
    },
    Contains {
        ancestor: NodeId,
        descendant: NodeId,
    },
    Parent {
        node: NodeId,
    },
    Children {
        node: NodeId,
        elements_only: bool,
    },
    Sibling {
        node: NodeId,
        direction: SiblingDirection,
        elements_only: bool,
    },
    NodeKind {
        node: NodeId,
    },
    NodeName {
        node: NodeId,
    },
    NodeValue {
        node: NodeId,
    },
    TextContent {
        node: NodeId,
    },
    InnerHtml {
        node: NodeId,
    },
    Attribute {
        node: NodeId,
        name: String,
    },
    AttributeNames {
        node: NodeId,
    },
    ShadowRoot {
        host: NodeId,
    },
    AssignedNodes {
        slot: NodeId,
        flatten: bool,
    },
    BoundingClientRect {
        node: NodeId,
    },
    ScrollMetrics {
        node: NodeId,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum DomReadResult {
    None,
    Node(NodeId),
    Nodes(Vec<NodeId>),
    Bool(bool),
    Kind(NodeKind),
    String(String),
    StringList(Vec<String>),
    Number(f64),
    Rect(DomRect),
    ScrollMetrics(ScrollMetrics),
}

#[derive(Debug, Clone, PartialEq)]
pub enum DomMutation {
    CreateElement {
        window: WindowId,
        local_name: String,
    },
    CreateTextNode {
        window: WindowId,
        data: String,
    },
    CreateDocumentFragment {
        window: WindowId,
    },
    CloneNode {
        node: NodeId,
        deep: bool,
    },
    SetTextContent {
        node: NodeId,
        value: String,
    },
    SetInnerHtml {
        node: NodeId,
        html: String,
    },
    WriteHtml {
        window: WindowId,
        html: String,
    },
    SetAttribute {
        node: NodeId,
        name: String,
        value: String,
    },
    RemoveAttribute {
        node: NodeId,
        name: String,
    },
    ToggleAttribute {
        node: NodeId,
        name: String,
        force: Option<bool>,
    },
    Append {
        parent: NodeId,
        children: Vec<NodeId>,
    },
    Prepend {
        parent: NodeId,
        children: Vec<NodeId>,
    },
    InsertBefore {
        parent: NodeId,
        child: NodeId,
        reference: Option<NodeId>,
    },
    ReplaceChild {
        parent: NodeId,
        new_child: NodeId,
        old_child: NodeId,
    },
    SetScrollOffset {
        node: NodeId,
        x: f64,
        y: f64,
    },
    SetWindowScroll {
        window: WindowId,
        x: f64,
        y: f64,
    },
    Remove {
        node: NodeId,
    },
    AttachShadow {
        host: NodeId,
        open: bool,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum DomMutationResult {
    None,
    Node(NodeId),
    Bool(bool),
}

#[derive(Debug, Clone, PartialEq)]
pub struct DomRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScrollMetrics {
    pub scroll_left: f64,
    pub scroll_top: f64,
    pub scroll_width: f64,
    pub scroll_height: f64,
    pub client_width: f64,
    pub client_height: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventTarget {
    Window(WindowId),
    Node(NodeId),
}

#[derive(Debug, Clone, PartialEq)]
pub struct DomEventRequest {
    pub target: EventTarget,
    pub event_type: String,
    pub bubbles: bool,
    pub cancelable: bool,
    pub composed: bool,
    pub detail: Option<HostData>,
    pub key: Option<String>,
    pub code: Option<String>,
    pub client_x: Option<i32>,
    pub client_y: Option<i32>,
    pub button: Option<i16>,
    pub buttons: Option<i16>,
    pub repeat: bool,
    pub alt_key: bool,
    pub ctrl_key: bool,
    pub shift_key: bool,
    pub meta_key: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomEventResult {
    pub default_prevented: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerKind {
    Timeout,
    Interval,
    AnimationFrame,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TimerRequest {
    pub window: WindowId,
    pub kind: TimerKind,
    pub delay_ms: u32,
    pub nesting_level: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
    Head,
    Options,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchMode {
    SameOrigin,
    Cors,
    NoCors,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchBody {
    Empty,
    Bytes(Vec<u8>),
    Utf8(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchRequest {
    pub window: WindowId,
    pub url: String,
    pub method: HttpMethod,
    pub headers: Vec<(String, String)>,
    pub body: FetchBody,
    pub mode: FetchMode,
    pub keepalive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchResponse {
    pub final_url: String,
    pub status: u16,
    pub status_text: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchError {
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageOp {
    Get {
        kind: StorageAreaKind,
        scope: StorageAreaScope,
        key: String,
    },
    Set {
        kind: StorageAreaKind,
        scope: StorageAreaScope,
        key: String,
        value: String,
    },
    Remove {
        kind: StorageAreaKind,
        scope: StorageAreaScope,
        key: String,
    },
    Clear {
        kind: StorageAreaKind,
        scope: StorageAreaScope,
    },
    Keys {
        kind: StorageAreaKind,
        scope: StorageAreaScope,
    },
    Len {
        kind: StorageAreaKind,
        scope: StorageAreaScope,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageResult {
    None,
    Value(Option<String>),
    Keys(Vec<String>),
    Len(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObserverKind {
    Mutation,
    Resize,
    Intersection,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ObserverOptions {
    pub subtree: bool,
    pub child_list: bool,
    pub attributes: bool,
    pub character_data: bool,
    pub attribute_old_value: bool,
    pub character_data_old_value: bool,
    pub threshold: Option<Vec<f64>>,
    pub root: Option<NodeId>,
    pub root_margin: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ObserverOp {
    Create {
        kind: ObserverKind,
    },
    Observe {
        observer: ObserverId,
        target: NodeId,
        options: ObserverOptions,
    },
    Disconnect {
        observer: ObserverId,
    },
    TakeRecords {
        observer: ObserverId,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum ObserverResult {
    None,
    Created(ObserverId),
    Records(Vec<ObserverRecord>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ObserverRecord {
    pub target: NodeId,
    pub kind: ObserverKind,
    pub payload: HostData,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostTimeSnapshot {
    pub monotonic_ms: u64,
    pub unix_ms: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HostEvent {
    TimerFired {
        timer_id: TimerId,
        window: WindowId,
        kind: TimerKind,
    },
    AnimationFrame {
        frame_id: FrameId,
        window: WindowId,
        timestamp_ms: u64,
    },
    NetworkResponse {
        request_id: NetworkRequestId,
        result: Result<FetchResponse, FetchError>,
    },
    MutationObserverDelivery {
        observer_id: ObserverId,
        records: Vec<ObserverRecord>,
    },
    ResizeObserverDelivery {
        observer_id: ObserverId,
        records: Vec<ObserverRecord>,
    },
    IntersectionObserverTask {
        observer_id: ObserverId,
        records: Vec<ObserverRecord>,
    },
}

pub trait Host: Any {
    /// Downcast helper — every impl should return `self`.
    fn as_any_mut(&mut self) -> &mut dyn Any;

    fn window(&self) -> WindowId;
    fn window_metrics(&self, window: WindowId) -> HostResult<WindowMetrics>;
    fn location(&self, window: WindowId) -> HostResult<LocationSnapshot>;
    fn navigate(&mut self, action: NavigationAction) -> HostResult<NavigationOutcome>;
    fn history(&mut self, action: HistoryAction) -> HostResult<HistoryOutcome>;

    fn read_dom(&self, read: DomRead) -> HostResult<DomReadResult>;
    fn mutate_dom(&mut self, mutation: DomMutation) -> HostResult<DomMutationResult>;
    fn dispatch_dom_event(&mut self, request: DomEventRequest) -> HostResult<DomEventResult>;

    fn console(&mut self, message: ConsoleMessage) -> HostResult<()>;

    fn schedule_timer(&mut self, request: TimerRequest) -> HostResult<TimerId>;
    fn cancel_timer(&mut self, timer_id: TimerId) -> HostResult<bool>;

    fn request_animation_frame(&mut self, window: WindowId) -> HostResult<FrameId>;
    fn cancel_animation_frame(&mut self, frame_id: FrameId) -> HostResult<bool>;

    fn fetch(&mut self, request: FetchRequest) -> HostResult<NetworkRequestId>;
    fn abort_fetch(&mut self, request_id: NetworkRequestId) -> HostResult<bool>;

    /// Perform an HTTP request synchronously and return the response.
    ///
    /// This backs the engine's `fetch()` until the async host-event loop
    /// (`fetch` + `wait_for_host_events`/`NetworkResponse`) is wired. The
    /// default is unsupported; a real host (e.g. `BrowserHost`) overrides it.
    fn fetch_sync(&mut self, request: FetchRequest) -> HostResult<FetchResponse> {
        let _ = request;
        Err(HostError::Unsupported)
    }

    fn storage(&mut self, operation: StorageOp) -> HostResult<StorageResult>;
    fn observer(&mut self, operation: ObserverOp) -> HostResult<ObserverResult>;

    fn now(&self) -> HostTimeSnapshot;
    fn wait_for_host_events(&mut self, max_wait_ms: Option<u64>) -> HostResult<Vec<HostEvent>>;
}

// ---------------------------------------------------------------------------
// NoopHost — used by Vm::new() so tests need no changes
// ---------------------------------------------------------------------------

pub struct NoopHost;

impl Host for NoopHost {
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
    fn window(&self) -> WindowId { WindowId(0) }
    fn window_metrics(&self, _w: WindowId) -> HostResult<WindowMetrics> {
        Ok(WindowMetrics {
            inner_width: 0.0,
            inner_height: 0.0,
            scroll_x: 0.0,
            scroll_y: 0.0,
            device_pixel_ratio: 1.0,
        })
    }
    fn location(&self, _w: WindowId) -> HostResult<LocationSnapshot> {
        Ok(LocationSnapshot {
            href: String::new(),
            origin: String::new(),
            protocol: String::new(),
            host: String::new(),
            hostname: String::new(),
            port: String::new(),
            pathname: String::new(),
            search: String::new(),
            hash: String::new(),
        })
    }
    fn navigate(&mut self, _a: NavigationAction) -> HostResult<NavigationOutcome> {
        Ok(NavigationOutcome { committed: false, same_document: false })
    }
    fn history(&mut self, _a: HistoryAction) -> HostResult<HistoryOutcome> {
        Ok(HistoryOutcome { href: String::new(), state: None, length: 0, restored_scroll_y: None })
    }
    fn read_dom(&self, _r: DomRead) -> HostResult<DomReadResult> { Ok(DomReadResult::None) }
    fn mutate_dom(&mut self, _m: DomMutation) -> HostResult<DomMutationResult> {
        Ok(DomMutationResult::None)
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
    fn fetch(&mut self, _r: FetchRequest) -> HostResult<NetworkRequestId> {
        Err(HostError::Unsupported)
    }
    fn abort_fetch(&mut self, _id: NetworkRequestId) -> HostResult<bool> { Ok(false) }
    fn storage(&mut self, _op: StorageOp) -> HostResult<StorageResult> {
        Ok(StorageResult::None)
    }
    fn observer(&mut self, _op: ObserverOp) -> HostResult<ObserverResult> {
        Err(HostError::Unsupported)
    }
    fn now(&self) -> HostTimeSnapshot { HostTimeSnapshot { monotonic_ms: 0, unix_ms: 0 } }
    fn wait_for_host_events(&mut self, _ms: Option<u64>) -> HostResult<Vec<HostEvent>> {
        Ok(Vec::new())
    }
}
