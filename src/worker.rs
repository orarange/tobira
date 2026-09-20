//! Dedicated workers: another script, on another thread, exchanging copies.
//!
//! A worker is the second place in this browser where JavaScript runs at the
//! same time as the document's. The first is the document's own engine
//! thread (`js.rs`), and this follows its shape exactly: the `Vm` is not
//! `Send`, so it is built **inside** the thread that owns it, and nothing but
//! plain data crosses the channel between them.
//!
//! That the channel carries [`HostData`] rather than `Value` is what makes it
//! safe. A `Value` points into one `Vm`'s heap and two threads must never
//! share one; the type refuses the mistake rather than a test catching it
//! afterwards. It is also exactly what the specification asks for: a worker
//! receives a *structured clone*, not the object the sender still holds.

use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::thread::JoinHandle;

use tobira_engine::engine::{
    ConsoleMessage, DomEventRequest, DomEventResult, DomMutation, DomMutationResult, DomRead,
    DomReadResult, FetchRequest, FetchResponse, FrameId, Heap, HistoryAction, HistoryOutcome,
    Host, HostData, HostError, HostEvent, HostResult, HostTimeSnapshot, LocationSnapshot,
    NavigationAction, NavigationOutcome, NetworkRequestId, NoopHost, ObserverOp, ObserverResult,
    StorageOp, StorageResult, TimerId, TimerRequest, Vm, WindowId, WindowMetrics, WorkerEvent,
    WorkerId,
};

/// What the document's side of a worker holds.
struct WorkerThread {
    to_worker: Sender<ToWorker>,
    handle: Option<JoinHandle<()>>,
}

enum ToWorker {
    Message(HostData),
    Terminate,
}

/// Every worker a document has started, and the messages they have sent back.
#[derive(Default)]
pub struct WorkerPool {
    workers: Vec<(WorkerId, WorkerThread)>,
    next_id: u32,
    from_workers: Option<Receiver<(WorkerId, WorkerEvent)>>,
    sender: Option<Sender<(WorkerId, WorkerEvent)>>,
}

impl WorkerPool {
    /// How many workers one document may start. A page that makes them in a
    /// loop -- by accident or otherwise -- should run out of workers, not out
    /// of threads.
    const MAX_WORKERS: usize = 32;

    pub fn spawn(&mut self, source: String, url: String) -> HostResult<WorkerId> {
        if self.workers.len() >= Self::MAX_WORKERS {
            return Err(HostError::Unsupported);
        }
        if self.sender.is_none() {
            let (sender, receiver) = channel();
            self.sender = Some(sender);
            self.from_workers = Some(receiver);
        }
        let back = self.sender.clone().expect("just made");
        self.next_id += 1;
        let id = WorkerId(self.next_id);
        let (to_worker, inbox) = channel();
        let handle = std::thread::Builder::new()
            .name(format!("tobira-worker-{}", id.0))
            .stack_size(16 * 1024 * 1024)
            .spawn(move || run_worker(id, source, url, inbox, back))
            .map_err(|_| HostError::Unsupported)?;
        self.workers.push((
            id,
            WorkerThread {
                to_worker,
                handle: Some(handle),
            },
        ));
        Ok(id)
    }

    pub fn post(&mut self, worker: WorkerId, data: HostData) -> HostResult<()> {
        let Some((_, thread)) = self.workers.iter().find(|(id, _)| *id == worker) else {
            return Err(HostError::Unsupported);
        };
        thread
            .to_worker
            .send(ToWorker::Message(data))
            .map_err(|_| HostError::Unsupported)
    }

    /// `terminate()`: ask the worker to stop, and wait for it.
    ///
    /// Waiting matters. A thread still running when the process wants to
    /// finish keeps a screenshot or a `--cli` run from ever returning, and
    /// the worker only checks between turns anyway, so the wait is short.
    pub fn terminate(&mut self, worker: WorkerId) -> HostResult<()> {
        let Some(index) = self.workers.iter().position(|(id, _)| *id == worker) else {
            return Ok(());
        };
        let (_, mut thread) = self.workers.remove(index);
        let _ = thread.to_worker.send(ToWorker::Terminate);
        if let Some(handle) = thread.handle.take() {
            let _ = handle.join();
        }
        Ok(())
    }

    pub fn take_events(&mut self) -> Vec<(WorkerId, WorkerEvent)> {
        let mut events = Vec::new();
        if let Some(receiver) = &self.from_workers {
            loop {
                match receiver.try_recv() {
                    Ok(event) => events.push(event),
                    Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
                }
            }
        }
        events
    }

    /// Stop every worker. Called when the document goes away.
    pub fn terminate_all(&mut self) {
        let ids: Vec<WorkerId> = self.workers.iter().map(|(id, _)| *id).collect();
        for id in ids {
            let _ = self.terminate(id);
        }
    }
}

impl Drop for WorkerPool {
    fn drop(&mut self) {
        self.terminate_all();
    }
}

/// The host a worker's own `Vm` talks to: it can send a message back and do
/// nothing else. No DOM, because a worker has none -- that is the
/// specification, not a shortcut taken here.
struct WorkerHost {
    id: WorkerId,
    back: Sender<(WorkerId, WorkerEvent)>,
    /// Everything a worker does *not* have answers the way an empty host
    /// does. Delegating rather than re-deciding each one keeps "a worker has
    /// no DOM" in one place instead of nineteen.
    empty: NoopHost,
}

macro_rules! to_empty {
    ($($name:ident($($arg:ident: $ty:ty),*) -> $ret:ty;)*) => {
        $(fn $name(&mut self $(, $arg: $ty)*) -> $ret { self.empty.$name($($arg),*) })*
    };
}

impl Host for WorkerHost {
    fn post_from_worker(&mut self, data: HostData) -> HostResult<()> {
        self.back
            .send((self.id, WorkerEvent::Message(data)))
            .map_err(|_| HostError::Unsupported)
    }

    fn console(&mut self, message: ConsoleMessage) -> HostResult<()> {
        if std::env::var_os("TOBIRA_DEBUG_CONSOLE").is_some() {
            eprintln!("[worker {}] {}", self.id.0, message.parts.join(" "));
        }
        Ok(())
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn window(&self) -> WindowId {
        self.empty.window()
    }
    fn window_metrics(&self, window: WindowId) -> HostResult<WindowMetrics> {
        self.empty.window_metrics(window)
    }
    fn location(&self, window: WindowId) -> HostResult<LocationSnapshot> {
        self.empty.location(window)
    }
    fn read_dom(&self, request: DomRead) -> HostResult<DomReadResult> {
        self.empty.read_dom(request)
    }
    fn now(&self) -> HostTimeSnapshot {
        self.empty.now()
    }

    to_empty! {
        navigate(action: NavigationAction) -> HostResult<NavigationOutcome>;
        history(action: HistoryAction) -> HostResult<HistoryOutcome>;
        mutate_dom(mutation: DomMutation) -> HostResult<DomMutationResult>;
        dispatch_dom_event(request: DomEventRequest) -> HostResult<DomEventResult>;
        schedule_timer(request: TimerRequest) -> HostResult<TimerId>;
        cancel_timer(timer: TimerId) -> HostResult<bool>;
        request_animation_frame(window: WindowId) -> HostResult<FrameId>;
        cancel_animation_frame(frame: FrameId) -> HostResult<bool>;
        fetch(request: FetchRequest) -> HostResult<NetworkRequestId>;
        abort_fetch(request: NetworkRequestId) -> HostResult<bool>;
        storage(op: StorageOp) -> HostResult<StorageResult>;
        observer(op: ObserverOp) -> HostResult<ObserverResult>;
        wait_for_host_events(timeout: Option<u64>) -> HostResult<Vec<HostEvent>>;
    }
}

fn run_worker(
    id: WorkerId,
    source: String,
    url: String,
    inbox: Receiver<ToWorker>,
    back: Sender<(WorkerId, WorkerEvent)>,
) {
    let host = WorkerHost {
        id,
        back: back.clone(),
        empty: NoopHost,
    };
    let mut vm = Vm::with_host(Heap::new(), Box::new(host));
    vm.install_worker_globals();

    // The script runs once, at the top, exactly as a document's does. A
    // failure here is the page's to hear about: a worker that dies quietly
    // leaves whoever started it waiting for a reply that is never coming.
    if let Err(error) = vm.eval_source(&source) {
        let _ = back.send((
            id,
            WorkerEvent::Error(format!("{url}: {error}")),
        ));
        return;
    }
    let mut now_ms = 0u64;
    loop {
        match inbox.recv() {
            Ok(ToWorker::Message(data)) => {
                if vm.worker_is_closed() {
                    continue;
                }
                if let Err(error) = vm.dispatch_worker_message(data) {
                    let _ = back.send((id, WorkerEvent::Error(format!("{error}"))));
                }
                // Timers and promises the handler queued, before the next
                // message is taken: a worker has its own event loop.
                now_ms += 1;
                vm.pump_event_loop(now_ms, 10_000);
            }
            Ok(ToWorker::Terminate) | Err(_) => break,
        }
        if vm.worker_is_closed() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A worker runs its script on its own thread and answers what it was
    /// sent -- and what crosses between them is a copy, because [`HostData`]
    /// is the only thing the channel can carry.
    #[test]
    fn a_worker_runs_its_script_and_answers() {
        let mut pool = WorkerPool::default();
        let id = pool
            .spawn(
                "self.postMessage('ready'); self.onmessage = function (e) {                  self.postMessage({ doubled: e.data.n * 2 }); };"
                    .to_string(),
                "http://localhost/worker.js".to_string(),
            )
            .expect("the worker should start");

        let mut seen: Vec<HostData> = Vec::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        pool.post(id, HostData::Object(vec![("n".to_string(), HostData::Number(21.0))]))
            .expect("the message should send");
        while seen.len() < 2 && std::time::Instant::now() < deadline {
            for (_, event) in pool.take_events() {
                match event {
                    WorkerEvent::Message(data) => seen.push(data),
                    WorkerEvent::Error(message) => panic!("worker failed: {message}"),
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(seen.len(), 2, "expected the greeting and the reply: {seen:?}");
        assert_eq!(seen[0], HostData::String("ready".to_string()));
        assert_eq!(
            seen[1],
            HostData::Object(vec![("doubled".to_string(), HostData::Number(42.0))])
        );
        pool.terminate(id).expect("the worker should stop");
    }

    /// A script that throws says so, rather than leaving the page waiting for
    /// a reply that is never coming.
    #[test]
    fn a_worker_that_fails_to_start_reports_it() {
        let mut pool = WorkerPool::default();
        let id = pool
            .spawn("throw new Error('nope');".to_string(), "http://localhost/w.js".to_string())
            .expect("the thread should start");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut reported = None;
        while reported.is_none() && std::time::Instant::now() < deadline {
            for (_, event) in pool.take_events() {
                if let WorkerEvent::Error(message) = event {
                    reported = Some(message);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let message = reported.expect("the failure should be reported");
        assert!(message.contains("nope"), "{message}");
        let _ = pool.terminate(id);
    }
}
