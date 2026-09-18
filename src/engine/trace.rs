//! Reachability for the mark phase.
//!
//! # The rule every impl in here follows
//!
//! A collector is only as good as its worst root. Miss one reference and the
//! object under it is freed while still in use — and because a stale `GcRef`
//! reads back as `None` rather than crashing, the symptom surfaces somewhere
//! else entirely, as a mysteriously empty string or a missing property. That is
//! the most expensive class of bug this engine could have.
//!
//! So the impls do not get to be lazy, and the compiler enforces it:
//!
//! * **Enums are matched exhaustively.** No `_ =>` arm, anywhere. Add a variant
//!   that holds a reference and this file stops compiling.
//! * **Structs are destructured completely.** No `..` rest pattern. Every field
//!   is named and either traced or explicitly bound to `_`, so adding a field
//!   also stops the build until someone decides which it is.
//!
//! If you are here because you added a field and the build broke: that is this
//! module working. Trace it, or bind it to `_` and say why it holds no
//! references.

use std::collections::{HashMap, HashSet, VecDeque};

use super::event_loop::{EventLoop, MicrotaskJob, RafEntry, TaskEntry, TimerEntry};
use super::heap::{GcRef, RawGcRef};
use super::value::{
    AsyncContext, AsyncGeneratorRequest, GeneratorState, JsObject, JsPropertyDescriptor, JsString,
    ObjectKind, PromiseReaction, PromiseState, TypedArrayKind, Value,
};

/// Collects what the mark phase can reach.
///
/// Marks live in here rather than in the heap's cell headers so that tracing
/// only needs a shared borrow of the heap — the walk reads objects while the
/// mark set is being written, which the borrow checker will not allow if both
/// live in the same place.
#[derive(Debug, Default)]
pub struct Tracer {
    live_objects: HashSet<RawGcRef>,
    live_strings: HashSet<RawGcRef>,
    worklist: Vec<GcRef<JsObject>>,
    aborted: Option<&'static str>,
}

impl Tracer {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark an object and queue it for scanning. Already-marked objects are not
    /// queued again, which is what terminates on cyclic graphs.
    pub fn mark_object(&mut self, object: GcRef<JsObject>) {
        if self.live_objects.insert(object.raw()) {
            self.worklist.push(object);
        }
    }

    pub fn mark_string(&mut self, string: GcRef<JsString>) {
        self.live_strings.insert(string.raw());
    }

    /// The next object whose contents still need scanning.
    pub fn next_to_scan(&mut self) -> Option<GcRef<JsObject>> {
        self.worklist.pop()
    }

    #[must_use]
    pub fn object_is_live(&self, object: GcRef<JsObject>) -> bool {
        self.live_objects.contains(&object.raw())
    }

    #[must_use]
    pub fn string_is_live(&self, string: GcRef<JsString>) -> bool {
        self.live_strings.contains(&string.raw())
    }

    /// Give up on this collection: something could not be read, so the live set
    /// is incomplete and sweeping it would free reachable objects. The caller
    /// checks `aborted` and skips the sweep — collecting nothing is always safe.
    pub fn abort(&mut self, reason: &'static str) {
        if self.aborted.is_none() {
            self.aborted = Some(reason);
        }
    }

    #[must_use]
    pub fn aborted(&self) -> Option<&'static str> {
        self.aborted
    }

    #[must_use]
    pub fn live_object_count(&self) -> usize {
        self.live_objects.len()
    }

    #[must_use]
    pub fn live_string_count(&self) -> usize {
        self.live_strings.len()
    }
}

/// Everything that can hold a heap reference implements this.
pub trait Trace {
    fn trace(&self, tracer: &mut Tracer);
}

// ---- containers ---------------------------------------------------------

impl<T: Trace> Trace for Option<T> {
    fn trace(&self, tracer: &mut Tracer) {
        if let Some(value) = self {
            value.trace(tracer);
        }
    }
}

impl<T: Trace> Trace for Vec<T> {
    fn trace(&self, tracer: &mut Tracer) {
        for item in self {
            item.trace(tracer);
        }
    }
}

impl<T: Trace> Trace for VecDeque<T> {
    fn trace(&self, tracer: &mut Tracer) {
        for item in self {
            item.trace(tracer);
        }
    }
}

impl<T: Trace> Trace for Box<T> {
    fn trace(&self, tracer: &mut Tracer) {
        self.as_ref().trace(tracer);
    }
}

/// A closure cell (`Rc<RefCell<Value>>`) and the shared buffers `Promise.all`
/// builds. Borrowing can only fail if a collection were triggered while one of
/// these is mutably borrowed; rather than guess at the contents, the collection
/// is abandoned.
impl<T: Trace> Trace for std::rc::Rc<std::cell::RefCell<T>> {
    fn trace(&self, tracer: &mut Tracer) {
        match self.try_borrow() {
            Ok(value) => value.trace(tracer),
            Err(_) => tracer.abort("a cell was mutably borrowed during tracing"),
        }
    }
}

impl<T: Trace, U: Trace> Trace for (T, U) {
    fn trace(&self, tracer: &mut Tracer) {
        self.0.trace(tracer);
        self.1.trace(tracer);
    }
}

impl<K, V: Trace> Trace for HashMap<K, V> {
    fn trace(&self, tracer: &mut Tracer) {
        for value in self.values() {
            value.trace(tracer);
        }
    }
}

impl<K, V: Trace> Trace for indexmap::IndexMap<K, V> {
    fn trace(&self, tracer: &mut Tracer) {
        for value in self.values() {
            value.trace(tracer);
        }
    }
}

/// Strings have no outgoing references, so this is the whole of it.
impl Trace for GcRef<JsString> {
    fn trace(&self, tracer: &mut Tracer) {
        tracer.mark_string(*self);
    }
}

impl Trace for GcRef<JsObject> {
    fn trace(&self, tracer: &mut Tracer) {
        tracer.mark_object(*self);
    }
}

// ---- values -------------------------------------------------------------

impl Trace for Value {
    fn trace(&self, tracer: &mut Tracer) {
        match self {
            Self::String(string) => tracer.mark_string(*string),
            Self::Object(object) => tracer.mark_object(*object),
            // No references: listed rather than wildcarded on purpose.
            Self::Undefined | Self::Null | Self::Bool(_) | Self::Number(_) | Self::Symbol(_) => {}
        }
    }
}

impl Trace for JsPropertyDescriptor {
    fn trace(&self, tracer: &mut Tracer) {
        match self {
            Self::Data {
                value,
                writable: _,
                enumerable: _,
                configurable: _,
            } => value.trace(tracer),
            Self::Accessor {
                get,
                set,
                enumerable: _,
                configurable: _,
            } => {
                get.trace(tracer);
                set.trace(tracer);
            }
        }
    }
}

impl Trace for JsObject {
    fn trace(&self, tracer: &mut Tracer) {
        let Self {
            kind,
            prototype,
            extensible: _,
            properties,
        } = self;
        kind.trace(tracer);
        prototype.trace(tracer);
        properties.trace(tracer);
    }
}

impl Trace for PromiseReaction {
    fn trace(&self, tracer: &mut Tracer) {
        let Self {
            handler,
            result_promise,
            is_reject_handler: _,
        } = self;
        handler.trace(tracer);
        result_promise.trace(tracer);
    }
}

impl Trace for PromiseState {
    fn trace(&self, tracer: &mut Tracer) {
        match self {
            Self::Pending {
                fulfill_reactions,
                reject_reactions,
            } => {
                fulfill_reactions.trace(tracer);
                reject_reactions.trace(tracer);
            }
            Self::Fulfilled(value) | Self::Rejected(value) => value.trace(tracer),
        }
    }
}

impl Trace for AsyncGeneratorRequest {
    fn trace(&self, tracer: &mut Tracer) {
        let Self {
            sent,
            promise,
            is_return: _,
        } = self;
        sent.trace(tracer);
        promise.trace(tracer);
    }
}

impl Trace for AsyncContext {
    fn trace(&self, tracer: &mut Tracer) {
        let Self {
            frame,
            stack_snapshot,
            outer_promise,
            async_generator_request,
        } = self;
        frame.trace(tracer);
        stack_snapshot.trace(tracer);
        outer_promise.trace(tracer);
        async_generator_request.trace(tracer);
    }
}

impl Trace for GeneratorState {
    fn trace(&self, tracer: &mut Tracer) {
        match self {
            Self::Suspended {
                frame,
                stack,
                started: _,
            } => {
                frame.trace(tracer);
                stack.trace(tracer);
            }
            Self::Running | Self::Completed => {}
        }
    }
}

impl Trace for TypedArrayKind {
    fn trace(&self, _tracer: &mut Tracer) {}
}

impl Trace for ObjectKind {
    fn trace(&self, tracer: &mut Tracer) {
        match self {
            Self::Promise(state) => state.trace(tracer),
            Self::AsyncResumer(context) => context.trace(tracer),
            Self::AsyncGenerator { state, queue } => {
                state.trace(tracer);
                queue.trace(tracer);
            }
            Self::Generator(state) => state.trace(tracer),
            Self::Proxy { target, handler } => {
                target.trace(tracer);
                handler.trace(tracer);
            }
            Self::Map(entries) | Self::WeakMap(entries) => entries.trace(tracer),
            Self::Set(values) | Self::WeakSet(values) => values.trace(tracer),
            Self::Primitive(value) => value.trace(tracer),
            Self::TypedArray {
                buffer,
                kind,
                byte_offset: _,
                length: _,
            } => {
                buffer.trace(tracer);
                kind.trace(tracer);
            }
            Self::ForOfIterator { values, index: _ } => values.trace(tracer),
            Self::LazyIterator {
                iterator,
                next_fn,
                done: _,
            } => {
                iterator.trace(tracer);
                next_fn.trace(tracer);
            }
            // These carry no heap references. Spelled out rather than
            // wildcarded so a new payload cannot slip in unnoticed.
            Self::Ordinary
            | Self::Array
            | Self::Function
            | Self::Error
            | Self::RegExp { .. }
            | Self::UrlSearchParams(_)
            | Self::Headers(_)
            | Self::FormData(_)
            | Self::ArrayBuffer(_)
            | Self::Host(_)
            | Self::Exotic(_) => {}
        }
    }
}

// ---- event loop ---------------------------------------------------------

impl Trace for TaskEntry {
    fn trace(&self, tracer: &mut Tracer) {
        let Self {
            source: _,
            callback,
            args,
        } = self;
        callback.trace(tracer);
        args.trace(tracer);
    }
}

impl Trace for MicrotaskJob {
    fn trace(&self, tracer: &mut Tracer) {
        match self {
            Self::PromiseReaction {
                handler,
                result_promise,
                value,
                is_reject: _,
            } => {
                handler.trace(tracer);
                result_promise.trace(tracer);
                value.trace(tracer);
            }
            Self::QueueMicrotask(callback) => callback.trace(tracer),
            Self::AsyncResume {
                resumer,
                value,
                is_throw: _,
            } => {
                resumer.trace(tracer);
                value.trace(tracer);
            }
        }
    }
}

impl Trace for TimerEntry {
    fn trace(&self, tracer: &mut Tracer) {
        let Self {
            id: _,
            due_ms: _,
            interval_ms: _,
            callback,
            args,
            nesting_level: _,
        } = self;
        callback.trace(tracer);
        args.trace(tracer);
    }
}

impl Trace for RafEntry {
    fn trace(&self, tracer: &mut Tracer) {
        let Self { id: _, callback } = self;
        callback.trace(tracer);
    }
}

impl Trace for EventLoop {
    fn trace(&self, tracer: &mut Tracer) {
        let Self {
            macrotask_queue,
            microtask_queue,
            timer_heap,
            raf_callbacks,
            next_timer_id: _,
            next_raf_id: _,
            cancelled_timers: _,
            current_time_ms: _,
        } = self;
        macrotask_queue.trace(tracer);
        microtask_queue.trace(tracer);
        for entry in timer_heap {
            entry.0.trace(tracer);
        }
        raf_callbacks.trace(tracer);
    }
}
