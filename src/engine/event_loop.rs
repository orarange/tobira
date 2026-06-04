use std::cmp::{Ordering, Reverse};
use std::collections::{BinaryHeap, HashSet, VecDeque};

use indexmap::IndexMap;

use super::heap::GcRef;
use super::value::{JsObject, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskSource {
    Timer,
    Networking,
    UserInteraction,
    Dom,
    Rendering,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskEntry {
    pub source: TaskSource,
    pub callback: GcRef<JsObject>,
    pub args: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MicrotaskJob {
    PromiseReaction {
        handler: Option<GcRef<JsObject>>,
        result_promise: Option<GcRef<JsObject>>,
        value: Value,
        is_reject: bool,
    },
    QueueMicrotask(GcRef<JsObject>),
    AsyncResume {
        resumer: GcRef<JsObject>,
        value: Value,
        is_throw: bool,
    },
}

#[derive(Debug, Clone)]
pub struct TimerEntry {
    pub id: u32,
    pub due_ms: u64,
    pub interval_ms: Option<u64>,
    pub callback: GcRef<JsObject>,
    pub args: Vec<Value>,
    pub nesting_level: u32,
}

impl PartialEq for TimerEntry {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.due_ms == other.due_ms
    }
}

impl Eq for TimerEntry {}

impl PartialOrd for TimerEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TimerEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        self.due_ms
            .cmp(&other.due_ms)
            .then_with(|| self.id.cmp(&other.id))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RafEntry {
    pub id: u32,
    pub callback: GcRef<JsObject>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TickResult {
    DidWork,
    NeedsRender,
    Idle,
}

#[derive(Debug, Default)]
pub struct EventLoop {
    pub macrotask_queue: VecDeque<TaskEntry>,
    pub microtask_queue: VecDeque<MicrotaskJob>,
    pub timer_heap: BinaryHeap<Reverse<TimerEntry>>,
    pub raf_callbacks: IndexMap<u32, RafEntry>,
    pub next_timer_id: u32,
    pub next_raf_id: u32,
    pub resize_observer_depth: u32,
    pub cancelled_timers: HashSet<u32>,
    pub current_time_ms: u64,
}

impl EventLoop {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}
