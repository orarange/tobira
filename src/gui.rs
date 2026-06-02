use std::num::NonZeroU32;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use arboard::Clipboard;
use softbuffer::{Context, Surface};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{
    ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy, OwnedDisplayHandle,
};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::window::{CursorIcon, ResizeDirection, Window};

use crate::browser::{BrowserPage, LoadedDocumentSource, build_page_from_source, load_page_source};
use crate::css::{
    BackgroundRepeat, Color, CursorKind, DEFAULT_BACKGROUND_COLOR, DEFAULT_TEXT_COLOR,
    FontFamilyKind, GradientKind, InteractiveState, ObjectFit, StyledNode,
};
use crate::error::{BrowserError, Result};
use crate::font::FontContext;
use crate::image::{DecodedImage, ImageStore};
use crate::js::{
    DomEventDispatchResult, DomEventRequest, JS_FETCH_SETTLE_TIMEOUT, JavaScriptSession,
};
use crate::layout::{
    DrawCommand, ElementHitbox, FormControlCommand, FormControlKind, LayerCommand, LayoutDocument,
    StickyCommand, TextCommand, layout_styled_document,
};
use crate::url::Url;

const WINDOW_WIDTH: u32 = 1100;
const WINDOW_HEIGHT: u32 = 760;
const FRAME_PADDING: u32 = 18;
const CHROME_TOP_PADDING: u32 = 10;
const CHROME_BOTTOM_PADDING: u32 = 10;
const CHROME_ROW_GAP: u32 = 10;
const TITLE_BAR_HEIGHT: u32 = 38;
const ADDRESS_BAR_HEIGHT: u32 = 42;
const BUTTON_WIDTH: u32 = 46;
const MAX_RENDER_FEEDBACK_PASSES: usize = 2;
const BUTTON_HEIGHT: u32 = 30;
const BUTTON_GAP: u32 = 2;
const TOOL_BUTTON_WIDTH: u32 = 52;
const ADDRESS_BAR_PADDING_X: u32 = 12;
const CONTROL_PADDING_X: u32 = 8;
const CONTROL_PADDING_Y: u32 = 6;
const INFO_FONT_SIZE: u32 = 12;
const ADDRESS_BAR_FONT_SIZE: u32 = 16;
const APP_FONT_SIZE: u32 = 18;
const TITLE_FONT_SIZE: u32 = 14;
const TITLE_META_GAP: u32 = 18;
const HEADER_BORDER_HEIGHT: u32 = 4;
const RESIZE_BORDER: u32 = 6;

const COLOR_WINDOW_BACKGROUND: Color = 0xE7E0D4;
const COLOR_HEADER: Color = 0x1F3A5F;
const COLOR_HEADER_ROW: Color = 0x28466F;
const COLOR_HEADER_TEXT: Color = 0xF6F7FB;
const COLOR_HEADER_MUTED: Color = 0xC7D2E5;
const COLOR_ACCENT: Color = 0xE6A53A;
const COLOR_ERROR: Color = 0x8C2F39;
const COLOR_TOOL_BUTTON: Color = 0x35527C;
const COLOR_TOOL_BUTTON_HOVER: Color = 0x416392;
const COLOR_CLOSE_BUTTON: Color = 0x5D2A37;
const COLOR_CLOSE_BUTTON_HOVER: Color = 0xC64F5B;
const COLOR_ADDRESS_BAR: Color = 0xF5F8FC;
const COLOR_ADDRESS_BAR_BORDER: Color = 0x91A6C6;
const COLOR_ADDRESS_BAR_FOCUS: Color = 0xE6A53A;
const COLOR_ADDRESS_BAR_TEXT: Color = 0x122033;
const COLOR_ADDRESS_BAR_SELECTION: Color = 0xBBD5F7;
const COLOR_CONTROL_PLACEHOLDER: Color = 0x6D7788;
const COLOR_CONTROL_SELECTION: Color = 0xC5D8F5;
const COLOR_CONTROL_BUTTON_HOVER: Color = 0xD7E4F8;
const COLOR_PANEL_BORDER: Color = 0xD4C7B2;

type WindowHandle = Rc<Window>;
type SurfaceHandle = Surface<OwnedDisplayHandle, WindowHandle>;

pub fn run(initial_url: Option<Url>) -> Result<()> {
    let event_loop = EventLoop::<BrowserUserEvent>::with_user_event()
        .build()
        .map_err(|error| BrowserError::message(error.to_string()))?;
    event_loop.set_control_flow(ControlFlow::Wait);

    let context = Context::new(event_loop.owned_display_handle())
        .map_err(|error| BrowserError::message(error.to_string()))?;
    let event_proxy = event_loop.create_proxy();
    let mut app = BrowserApp::new(initial_url, context, event_proxy);

    event_loop
        .run_app(&mut app)
        .map_err(|error| BrowserError::message(error.to_string()))
}

struct BrowserApp {
    current_url: Option<Url>,
    history: Vec<HistoryEntry>,
    history_index: Option<usize>,
    document: DocumentView,
    fonts: FontContext,
    context: Context<OwnedDisplayHandle>,
    event_proxy: EventLoopProxy<BrowserUserEvent>,
    window: Option<WindowHandle>,
    surface: Option<SurfaceHandle>,
    scroll_y: u32,
    modifiers: ModifiersState,
    cursor_position: PhysicalPosition<f64>,
    address_bar: AddressBarState,
    render_request_tx: Sender<RenderRequest>,
    latest_render_frame: Option<RenderedContentFrame>,
    focused_page_input: Option<FocusedPageInput>,
    hovered_target: HitTarget,
    hovered_link_url: Option<String>,
    hovered_link_node_id: Option<usize>,
    hovered_element_node_id: Option<usize>,
    ime_composing: bool,
    pending_navigation: Option<PendingNavigation>,
    pending_render_id: Option<u64>,
    render_feedback_passes: usize,
    next_navigation_id: u64,
    next_render_id: u64,
    /// Depth-indexed offscreen buffer pool for layer compositing.
    /// scratch[depth] holds the pixel buffer used by the layer at that nesting depth.
    /// Buffers are reused across frames; no per-frame allocation after the first paint.
    scratch: Vec<Vec<u32>>,
    /// Anchor for CSS animation timing. Set the first frame an active animation is
    /// detected; cleared once all animations finish so the loop returns to idle.
    /// `now_ms` passed to the engine is `animation_epoch.elapsed()` in milliseconds.
    animation_epoch: Option<Instant>,
    /// Target interval between animation frames, derived from the display refresh rate
    /// (e.g. ~5.5ms on a 180Hz monitor). Falls back to 60fps until a monitor is known.
    frame_interval: Duration,
    /// When the last animation frame was presented, for pacing the frame loop independently
    /// of input events (so animations stay smooth while the cursor is moving).
    last_frame_at: Option<Instant>,
    /// Set true while an animation ticker thread is running; the thread posts `AnimationTick`
    /// events at the frame interval and exits when this clears.
    ticker_running: Arc<AtomicBool>,
    /// Coalesces ticks: the ticker only posts a new `AnimationTick` when the previous one has
    /// been processed, so a momentarily-busy main thread cannot accumulate a backlog of ticks.
    tick_pending: Arc<AtomicBool>,
}

#[derive(Debug, Clone)]
struct PendingNavigation {
    id: u64,
    restore_scroll: Option<u32>,
}

#[derive(Debug, Clone)]
enum BrowserUserEvent {
    NavigationFinished {
        navigation_id: u64,
        result: std::result::Result<LoadedDocumentSource, String>,
    },
    RenderFinished {
        render_id: u64,
        result: std::result::Result<RenderedContentFrame, String>,
    },
    JsSettleRequired,
    /// Posted by the animation ticker thread at the display refresh interval to drive the
    /// next animation frame. Unlike a `WaitUntil` timer, a posted event is delivered even
    /// while the message queue is busy with input (so animation keeps running as the cursor
    /// moves).
    AnimationTick,
}

#[derive(Debug, Clone)]
struct RenderRequest {
    id: u64,
    viewport_width: u32,
    page: RenderPageSnapshot,
    focused_page_input: Option<FocusedPageInput>,
    hovered_target: HitTarget,
}

#[derive(Debug, Clone)]
struct RenderPageSnapshot {
    styled_document: StyledNode,
    images: ImageStore,
}

#[derive(Debug, Clone)]
struct RenderedContentFrame {
    id: u64,
    content_width: u32,
    content_height: u32,
    layout: LayoutDocument,
    pixels: Vec<u32>,
}

#[derive(Debug, Clone, Copy)]
enum NavigationHistoryUpdate {
    Push,
    None,
}

#[derive(Debug, Clone)]
struct HistoryEntry {
    url: Url,
    scroll_y: u32,
}

fn empty_layout_document() -> LayoutDocument {
    LayoutDocument {
        background_color: DEFAULT_BACKGROUND_COLOR,
        content_height: 0,
        commands: Vec::new(),
        links: Vec::new(),
        controls: Vec::new(),
        element_hitboxes: Vec::new(),
    }
}

fn start_render_worker(event_proxy: EventLoopProxy<BrowserUserEvent>) -> Sender<RenderRequest> {
    let (request_tx, request_rx) = mpsc::channel::<RenderRequest>();
    thread::Builder::new()
        .name("tobira-render".to_string())
        .spawn(move || {
            let mut fonts = FontContext::load();
            while let Ok(request) = request_rx.recv() {
                let render_id = request.id;
                let result =
                    render_content_frame(request, &mut fonts).map_err(|error| error.to_string());
                let _ =
                    event_proxy.send_event(BrowserUserEvent::RenderFinished { render_id, result });
            }
        })
        .expect("render worker should start");

    request_tx
}

fn render_content_frame(
    request: RenderRequest,
    fonts: &mut FontContext,
) -> Result<RenderedContentFrame> {
    let content_width = request.viewport_width.saturating_sub(FRAME_PADDING).max(1);
    let layout = layout_styled_document(
        &request.page.styled_document,
        &request.page.images,
        content_width,
        fonts,
    );
    let content_height = layout.content_height;
    let mut buffer = if content_width == 0 || content_height == 0 {
        Vec::new()
    } else {
        vec![layout.background_color; content_width as usize * content_height as usize]
    };

    if !buffer.is_empty() {
        paint_background(
            &mut buffer,
            content_width,
            content_height,
            0,
            layout.background_color,
        );
        let mut scratch = Vec::new();
        paint_layout(
            Some(&request.page.images),
            fonts,
            &mut buffer,
            content_width,
            content_height,
            FRAME_PADDING / 2,
            0,
            content_height,
            0,
            &layout,
            request.focused_page_input.as_ref(),
            request.hovered_target,
            &mut scratch,
        );
    }

    Ok(RenderedContentFrame {
        id: request.id,
        content_width,
        content_height,
        layout,
        pixels: buffer,
    })
}

impl BrowserApp {
    fn new(
        initial_url: Option<Url>,
        context: Context<OwnedDisplayHandle>,
        event_proxy: EventLoopProxy<BrowserUserEvent>,
    ) -> Self {
        let render_request_tx = start_render_worker(event_proxy.clone());
        let initial_load_url = initial_url.clone();
        let (current_url, history, history_index, document, address_bar) = match initial_url {
            Some(url) => (
                Some(url.clone()),
                vec![HistoryEntry {
                    url: url.clone(),
                    scroll_y: 0,
                }],
                Some(0),
                DocumentView::blank(),
                AddressBarState::new(url.to_string()),
            ),
            None => {
                let mut address_bar = AddressBarState::new(String::new());
                address_bar.focus_at(0);
                (None, Vec::new(), None, DocumentView::blank(), address_bar)
            }
        };

        let mut app = Self {
            current_url,
            history,
            history_index,
            document,
            fonts: FontContext::load(),
            context,
            event_proxy,
            render_request_tx,
            latest_render_frame: None,
            window: None,
            surface: None,
            scroll_y: 0,
            modifiers: ModifiersState::default(),
            cursor_position: PhysicalPosition::new(0.0, 0.0),
            address_bar,
            focused_page_input: None,
            hovered_target: HitTarget::None,
            hovered_link_url: None,
            hovered_link_node_id: None,
            hovered_element_node_id: None,
            ime_composing: false,
            pending_navigation: None,
            pending_render_id: None,
            render_feedback_passes: 0,
            next_navigation_id: 1,
            next_render_id: 1,
            scratch: Vec::new(), // depth-indexed pool; grows lazily on first paint
            animation_epoch: None,
            frame_interval: Duration::from_micros(16_667), // 60fps default until monitor known
            last_frame_at: None,
            ticker_running: Arc::new(AtomicBool::new(false)),
            tick_pending: Arc::new(AtomicBool::new(false)),
        };

        if let Some(url) = initial_load_url {
            app.begin_navigation(url, None, NavigationHistoryUpdate::None);
        }

        app
    }

    fn load_url(&mut self, url: Url) {
        self.begin_navigation(url, None, NavigationHistoryUpdate::Push);
    }

    fn reload(&mut self) {
        let Some(url) = self.current_url.clone() else {
            self.document = DocumentView::blank();
            self.scroll_y = 0;
            self.scratch.clear();
            self.sync_window_title();
            self.request_redraw();
            return;
        };

        self.begin_navigation(
            url,
            self.current_history_scroll(),
            NavigationHistoryUpdate::None,
        );
    }

    fn navigate_to_address(&mut self) {
        let entered = self.address_bar.text().trim().to_string();
        if entered.is_empty() {
            self.current_url = None;
            self.document = DocumentView::blank();
            self.clear_page_control_state();
            self.scroll_y = 0;
            self.scratch.clear();
            self.sync_window_title();
            self.request_redraw();
            return;
        }

        match parse_address_input(&entered) {
            Ok(url) => self.load_url(url),
            Err(error) => {
                self.document =
                    DocumentView::error(format!("could not navigate to `{entered}`: {error}"));
                self.clear_page_control_state();
                self.scroll_y = 0;
                self.scratch.clear();
                self.sync_window_title();
                self.request_redraw();
            }
        }
    }

    fn go_back(&mut self) {
        self.navigate_history(-1);
    }

    fn go_forward(&mut self) {
        self.navigate_history(1);
    }

    fn can_go_back(&self) -> bool {
        self.history_index.is_some_and(|index| index > 0)
    }

    fn can_go_forward(&self) -> bool {
        self.history_index
            .is_some_and(|index| index.saturating_add(1) < self.history.len())
    }

    fn record_history_visit(&mut self, url: Url) {
        match self.history_index {
            Some(index) => {
                if self
                    .history
                    .get(index)
                    .is_some_and(|current| current.url == url)
                {
                    return;
                }
                let next_index = index.saturating_add(1);
                self.history.truncate(next_index);
                self.history.push(HistoryEntry { url, scroll_y: 0 });
                self.history_index = self.history.len().checked_sub(1);
            }
            None => {
                self.history.clear();
                self.history.push(HistoryEntry { url, scroll_y: 0 });
                self.history_index = Some(0);
            }
        }
    }

    fn replace_current_history_entry(&mut self, url: Url) {
        match self.history_index {
            Some(index) => {
                if let Some(entry) = self.history.get_mut(index) {
                    entry.url = url;
                } else {
                    self.history.push(HistoryEntry { url, scroll_y: 0 });
                    self.history_index = Some(self.history.len().saturating_sub(1));
                }
            }
            None => {
                self.history.clear();
                self.history.push(HistoryEntry { url, scroll_y: 0 });
                self.history_index = Some(0);
            }
        }
    }

    fn navigate_history(&mut self, delta: isize) {
        let Some(index) = self.history_index else {
            return;
        };
        let next = index as isize + delta;
        if next < 0 || next as usize >= self.history.len() {
            return;
        }
        let next_index = next as usize;
        let entry = self.history[next_index].clone();
        self.history_index = Some(next_index);
        self.begin_navigation(
            entry.url,
            Some(entry.scroll_y),
            NavigationHistoryUpdate::None,
        );
    }

    fn sync_current_history_scroll(&mut self) {
        let Some(index) = self.history_index else {
            return;
        };
        if let Some(entry) = self.history.get_mut(index) {
            entry.scroll_y = self.scroll_y;
        }
    }

    fn current_history_scroll(&self) -> Option<u32> {
        let index = self.history_index?;
        self.history.get(index).map(|entry| entry.scroll_y)
    }

    fn current_layout(&mut self) -> LayoutDocument {
        if matches!(&self.document.content, DocumentContent::Loaded(_))
            && self.document.refresh_loaded_page_from_script_session()
        {
            self.document.layout_cache = None;
        }
        match &self.document.content {
            // When the cache is invalidated (hover restyle, the animation/transition frame
            // loop, etc.) fall back to the last rendered frame's layout rather than an empty
            // one, so hit-testing (hover/link/cursor) keeps working between renders.
            DocumentContent::Loaded(_) => self
                .document
                .layout_cache
                .clone()
                .map(|cache| cache.layout)
                .or_else(|| self.latest_render_frame.as_ref().map(|f| f.layout.clone()))
                .unwrap_or_else(empty_layout_document),
            _ => {
                let window_size = self
                    .window
                    .as_ref()
                    .map(|window| window.inner_size())
                    .unwrap_or_else(|| PhysicalSize::new(WINDOW_WIDTH, WINDOW_HEIGHT));
                let content_width = window_size.width.saturating_sub(FRAME_PADDING).max(1);
                let mut fonts = FontContext::load();
                self.document.layout(content_width, &mut fonts)
            }
        }
    }

    fn current_page_control(&mut self, id: usize) -> Option<FormControlCommand> {
        self.current_layout()
            .controls
            .into_iter()
            .find(|control| control.id == id)
    }

    fn request_content_render(&mut self) {
        let Some(page) = self.document.render_snapshot() else {
            return;
        };

        let window_size = self
            .window
            .as_ref()
            .map(|window| window.inner_size())
            .unwrap_or_else(|| PhysicalSize::new(WINDOW_WIDTH, WINDOW_HEIGHT));
        let request_id = self.next_render_id;
        self.next_render_id = self.next_render_id.saturating_add(1);
        self.pending_render_id = Some(request_id);
        let _ = self.render_request_tx.send(RenderRequest {
            id: request_id,
            viewport_width: window_size.width,
            page,
            focused_page_input: self.focused_page_input.clone(),
            hovered_target: self.hovered_target,
        });
    }

    /// Update the animation frame interval from the current monitor's refresh rate, so
    /// animations run at the display rate (e.g. 180fps on a 180Hz panel) rather than a
    /// fixed 60fps. Falls back to 60fps when the rate is unknown.
    fn refresh_frame_interval(&mut self) {
        let millihertz = self
            .window
            .as_ref()
            .and_then(|window| window.current_monitor())
            .and_then(|monitor| monitor.refresh_rate_millihertz());
        self.frame_interval = match millihertz {
            // interval = 1 / rate; millihertz is Hz*1000, so period = 1e9 ns * 1000 / mHz.
            Some(mhz) if mhz > 0 => Duration::from_nanos(1_000_000_000_000u64 / mhz as u64),
            _ => Duration::from_micros(16_667),
        };
    }

    /// Spawn the animation ticker thread if one is not already running. It posts
    /// `AnimationTick` events at the frame interval, which drive `drive_animation_frame`.
    /// Posted events are delivered even while the OS message queue is busy with input, so
    /// animations keep running smoothly while the cursor moves (a `WaitUntil` timer would be
    /// starved by the input flood and the animation would freeze).
    fn ensure_animation_ticker(&mut self) {
        if self.ticker_running.swap(true, Ordering::SeqCst) {
            return; // a ticker is already running
        }
        let proxy = self.event_proxy.clone();
        let running = self.ticker_running.clone();
        let pending = self.tick_pending.clone();
        let interval = self.frame_interval.max(Duration::from_micros(1000));
        thread::Builder::new()
            .name("tobira-anim-tick".to_string())
            .spawn(move || {
                while running.load(Ordering::SeqCst) {
                    thread::sleep(interval);
                    // Only post when the previous tick has been consumed, so a busy main
                    // thread never accumulates a backlog of AnimationTick events.
                    if !pending.swap(true, Ordering::SeqCst) {
                        if proxy.send_event(BrowserUserEvent::AnimationTick).is_err() {
                            break; // event loop has shut down
                        }
                    }
                }
            })
            .ok();
    }

    /// Advance one animation frame and hand the new styled tree to the render worker.
    /// Called from the ticker (`AnimationTick`) and `about_to_wait`. The *advance* (cheap:
    /// interpolation, plus a relayout only for transitions) runs on the main thread; the
    /// heavy paint runs on the worker, so a high frame rate does not stall the UI. The render
    /// request is throttled to one in flight: the animation clock keeps advancing every tick
    /// regardless, so frames carry correct interpolated values even when some are dropped.
    fn drive_animation_frame(&mut self) {
        let Some(epoch) = self.animation_epoch else {
            self.ticker_running.store(false, Ordering::SeqCst);
            return;
        };

        let now_ms = epoch.elapsed().as_millis() as u64;
        let content_width = self
            .window
            .as_ref()
            .map(|window| Self::content_width_for(window.inner_size()))
            .unwrap_or(WINDOW_WIDTH);
        let interactive = self.current_interactive_state();

        let still_active = if let Some(page) = self.document.loaded_page_mut() {
            let trans_active = if page.has_transitions() {
                page.relayout(content_width, &interactive);
                page.advance_transitions(now_ms)
            } else {
                false
            };
            let anim_active = page.apply_animations(now_ms, 0);
            trans_active || anim_active
        } else {
            false
        };

        // Render off the main thread. Only enqueue when the worker is idle so requests do
        // not pile up; the next tick will enqueue the latest state once it finishes.
        if self.pending_render_id.is_none() {
            self.document.layout_cache = None;
            self.request_content_render();
        }

        if !still_active {
            self.animation_epoch = None;
            self.last_frame_at = None;
            self.ticker_running.store(false, Ordering::SeqCst);
            // Make sure the settled final frame is rendered even if a request was in flight.
            self.document.layout_cache = None;
            self.request_content_render();
        }
    }

    /// The InteractiveState matching the current hover, for CSS `:hover` restyling.
    fn current_interactive_state(&self) -> InteractiveState {
        InteractiveState {
            hovered_node_id: self.hovered_element_node_id,
            focused_node_id: None,
            active_node_ids: std::collections::HashSet::new(),
        }
    }

    /// Content width available for page layout at the given window size.
    fn content_width_for(window_size: PhysicalSize<u32>) -> u32 {
        window_size.width.saturating_sub(FRAME_PADDING).max(1)
    }

    /// Rebuild the loaded page's styled tree for the current hover state so CSS `:hover`
    /// applies immediately. If the page declares transitions, also arm the frame clock
    /// and kick the transition driver so the restyle animates from the pre-hover baseline.
    fn restyle_for_hover(&mut self, window_size: PhysicalSize<u32>) {
        let content_width = Self::content_width_for(window_size);
        let interactive = self.current_interactive_state();
        let (has_trans, has_hover_rules) = self
            .document
            .loaded_page()
            .map(|page| (page.has_transitions(), page.has_interactive_styling()))
            .unwrap_or((false, false));
        // Pages with no :hover/:focus/:active rules and no transitions have nothing to
        // restyle on hover. Skipping the relayout there avoids needless per-move work and,
        // crucially, avoids rebuilding the styled tree — which would reset any in-flight
        // animation to its initial frame each time the cursor crosses an element.
        if !has_trans && !has_hover_rules {
            return;
        }
        if has_trans {
            // Establish (or reuse) the frame clock so transition start times are consistent
            // with the animation timeline, then start the transition now.
            let _ = *self.animation_epoch.get_or_insert_with(Instant::now);
        }
        let now_ms = self
            .animation_epoch
            .map(|epoch| epoch.elapsed().as_millis() as u64);
        if let Some(page) = self.document.loaded_page_mut() {
            page.relayout(content_width, &interactive);
            // relayout() rebuilt the styled tree from scratch, discarding the current
            // animation/transition frame. Re-apply it at the current time so that a
            // hover-triggered relayout (e.g. the cursor moving over an animating element)
            // does not visibly reset the animation to its initial state.
            if let Some(now_ms) = now_ms {
                if has_trans {
                    page.advance_transitions(now_ms);
                }
                page.apply_animations(now_ms, 0);
            }
        }
        self.document.layout_cache = None;
    }

    fn finish_render(
        &mut self,
        render_id: u64,
        result: std::result::Result<RenderedContentFrame, String>,
    ) {
        if self
            .pending_render_id
            .is_none_or(|pending| pending != render_id)
        {
            return;
        }
        self.pending_render_id = None;

        match result {
            Ok(frame) => {
                let hitboxes = frame.layout.element_hitboxes.clone();
                self.document.layout_cache = Some(CachedLayout {
                    width: frame.content_width,
                    revision: self.document.layout_revision(),
                    layout: frame.layout.clone(),
                });
                self.latest_render_frame = Some(frame);
                let _ = self.document.set_scroll_position(self.scroll_y);
                self.scroll_y = self.document.scroll_position();
                self.sync_current_history_scroll();
                self.sync_window_title();
                if self.document.feed_layout_hitboxes(hitboxes) {
                    if self.render_feedback_passes < MAX_RENDER_FEEDBACK_PASSES {
                        self.render_feedback_passes += 1;
                        self.document.layout_cache = None;
                        self.request_content_render();
                        self.present_or_request_redraw();
                        return;
                    }
                    self.render_feedback_passes = 0;
                    self.present_or_request_redraw();
                    return;
                }
                self.render_feedback_passes = 0;
                self.present_or_request_redraw();
            }
            Err(error) => {
                self.document.status_line = format!("Status: render failed ({error})");
                self.render_feedback_passes = 0;
                self.request_redraw();
            }
        }
    }

    fn begin_navigation(
        &mut self,
        url: Url,
        restore_scroll: Option<u32>,
        history_update: NavigationHistoryUpdate,
    ) {
        match history_update {
            NavigationHistoryUpdate::Push => self.record_history_visit(url.clone()),
            NavigationHistoryUpdate::None => {}
        }

        let navigation_id = self.next_navigation_id;
        self.next_navigation_id = self.next_navigation_id.saturating_add(1);
        self.pending_navigation = Some(PendingNavigation {
            id: navigation_id,
            restore_scroll,
        });

        self.current_url = Some(url.clone());
        self.address_bar.set_text(url.to_string());
        self.address_bar.blur();
        self.clear_page_control_state();
        self.document = DocumentView::blank();
        self.latest_render_frame = None;
        self.document.layout_cache = None;
        self.pending_render_id = None;
        self.render_feedback_passes = 0;
        self.scroll_y = 0;
        self.scratch.clear();
        self.sync_window_title();
        self.sync_input_method();
        self.request_redraw();

        let proxy = self.event_proxy.clone();
        thread::spawn(move || {
            let result = load_page_source(&url).map_err(|error| error.to_string());
            let _ = proxy.send_event(BrowserUserEvent::NavigationFinished {
                navigation_id,
                result,
            });
        });
    }

    fn finish_navigation(
        &mut self,
        navigation_id: u64,
        result: std::result::Result<LoadedDocumentSource, String>,
    ) {
        if self
            .pending_navigation
            .as_ref()
            .is_none_or(|pending| pending.id != navigation_id)
        {
            return;
        }

        let pending = self.pending_navigation.take().unwrap();
        match result {
            Ok(source) => {
                let restore_scroll = pending.restore_scroll;
                let page = build_page_from_source(source, false);
                let final_url = page.url.clone();
                // Start the animation clock for this page if it declares any animations.
                // Anchoring here (rather than in about_to_wait) means a finished animation
                // is not restarted by unrelated idle wake-ups; only a new navigation resets it.
                let mut page = page;
                // Seed transition baselines with the no-hover styled tree so the first
                // hover/state change transitions instead of snapping.
                if page.has_transitions() {
                    page.advance_transitions(0);
                }
                self.animation_epoch = if page.has_active_animations() {
                    Some(Instant::now())
                } else {
                    None
                };
                self.document = DocumentView::from_page(page);
                self.current_url = Some(final_url.clone());
                self.address_bar.set_text(final_url.to_string());
                self.replace_current_history_entry(final_url);
                self.clear_page_control_state();
                self.scroll_y = restore_scroll.unwrap_or_else(|| self.document.scroll_position());
                if restore_scroll.is_some() {
                    let _ = self.document.set_scroll_position(self.scroll_y);
                }
                self.sync_current_history_scroll();
                self.latest_render_frame = None;
                self.document.layout_cache = None;
                self.render_feedback_passes = 0;
                self.sync_viewport_size();
                self.sync_window_title();
                self.sync_input_method();
                if self.window.is_none() {
                    self.request_content_render();
                }
                self.request_redraw();
            }
            Err(error) => {
                self.document = DocumentView::error(error);
                self.scroll_y = 0;
                self.clear_page_control_state();
                self.render_feedback_passes = 0;
                self.sync_window_title();
                self.sync_input_method();
                self.request_redraw();
            }
        }
    }

    fn focus_address_bar(&mut self, char_index: usize) {
        self.blur_page_input();
        self.address_bar.focus_at(char_index);
        self.sync_input_method();
        self.request_redraw();
    }

    fn focus_address_bar_select_all(&mut self) {
        self.blur_page_input();
        if self.address_bar.select_all() || !self.address_bar.focused {
            self.address_bar.focused = true;
            self.sync_input_method();
            self.request_redraw();
        }
    }

    fn blur_address_bar(&mut self) {
        if !self.address_bar.focused {
            return;
        }

        self.address_bar.blur();
        self.ime_composing = false;
        self.sync_input_method();
        self.request_redraw();
    }

    fn sync_window_title(&self) {
        if let Some(window) = &self.window {
            window.set_title(&self.document.window_title());
        }
    }

    fn request_redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Present the latest rendered frame. While an animation is running, paint and present
    /// immediately instead of requesting a redraw: a redraw request is serviced via WM_PAINT,
    /// which Windows starves while the message queue is flooded with input — so the on-screen
    /// animation would freeze as the cursor moves even though the worker keeps rendering.
    fn present_or_request_redraw(&mut self) {
        if self.animation_epoch.is_some() {
            let _ = self.draw();
        } else {
            self.request_redraw();
        }
    }

    fn sync_input_method(&mut self) {
        let Some(window) = self.window.as_ref().cloned() else {
            return;
        };

        let page_input_focused = self.focused_page_input.is_some();
        window.set_ime_allowed(self.address_bar.focused || page_input_focused);

        if self.address_bar.focused {
            let size = window.inner_size();
            let chrome = chrome_layout_metrics(&mut self.fonts, size.width);
            let view = address_bar_view(
                &self.address_bar,
                &mut self.fonts,
                chrome
                    .address_bar
                    .width
                    .saturating_sub(ADDRESS_BAR_PADDING_X * 2),
            );
            let line_height = self
                .fonts
                .line_height_px(ADDRESS_BAR_FONT_SIZE, FontFamilyKind::Sans);
            let text_y = chrome
                .address_bar
                .y
                .saturating_add(chrome.address_bar.height.saturating_sub(line_height) / 2);
            let caret_x = chrome
                .address_bar
                .x
                .saturating_add(ADDRESS_BAR_PADDING_X)
                .saturating_add(view.caret_x);

            window.set_ime_cursor_area(
                PhysicalPosition::new(caret_x as i32, text_y as i32),
                PhysicalSize::new(1, line_height.max(1)),
            );
            return;
        }

        let Some(focused) = self.focused_page_input.as_ref() else {
            return;
        };
        let focused_control_id = focused.control_id;
        let focused_editor = focused.editor.clone();

        let size = window.inner_size();
        let chrome = chrome_layout_metrics(&mut self.fonts, size.width);
        let body_top = chrome.height + FRAME_PADDING;
        let layout = self.current_layout();
        let Some(control) = layout
            .controls
            .iter()
            .find(|control| control.id == focused_control_id)
        else {
            return;
        };

        let view = text_editor_view(
            &focused_editor,
            &mut self.fonts,
            control.width.saturating_sub(CONTROL_PADDING_X * 2),
            control.font_size_px,
            control.font_family,
        );
        let line_height = self
            .fonts
            .line_height_px(control.font_size_px, control.font_family);
        let text_y = body_top
            .saturating_add(control.y.saturating_sub(self.scroll_y))
            .saturating_add(control.height.saturating_sub(line_height) / 2);
        let caret_x = (FRAME_PADDING / 2)
            .saturating_add(control.x)
            .saturating_add(CONTROL_PADDING_X)
            .saturating_add(view.caret_x);

        window.set_ime_cursor_area(
            PhysicalPosition::new(caret_x as i32, text_y as i32),
            PhysicalSize::new(1, line_height.max(1)),
        );
    }

    fn sync_viewport_size(&mut self) {
        let Some(window) = &self.window else {
            return;
        };

        let size = window.inner_size();
        if self.document.set_viewport_size(size.width, size.height) {
            self.latest_render_frame = None;
            let _ = self.document.dispatch_window_resize();
            self.scroll_y = self.document.scroll_position();
            self.sync_current_history_scroll();
            self.request_content_render();
        }
    }

    fn sync_scroll_position(&mut self) {
        let previous_revision = self.document.layout_revision();
        if self.document.set_scroll_position(self.scroll_y)
            && self.document.has_global_event_listener("scroll")
        {
            let _ = self.document.dispatch_scroll_event();
        }
        self.scroll_y = self.document.scroll_position();
        self.sync_current_history_scroll();
        if self.document.layout_revision() != previous_revision {
            self.request_content_render();
        }
        let _ = self.draw();
    }

    fn scroll_by(&mut self, delta: i32, viewport_height: u32, content_height: u32) {
        let max_scroll = max_scroll(viewport_height, content_height);
        let next = if delta.is_negative() {
            self.scroll_y.saturating_sub(delta.unsigned_abs())
        } else {
            self.scroll_y.saturating_add(delta as u32)
        };
        self.scroll_y = next.min(max_scroll);
        self.sync_current_history_scroll();
    }

    fn draw(&mut self) -> Result<()> {
        let Some(window) = &self.window else {
            return Ok(());
        };

        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return Ok(());
        }

        let chrome = chrome_layout_metrics(&mut self.fonts, size.width);
        let can_go_back = self.can_go_back();
        let can_go_forward = self.can_go_forward();
        let body_top = chrome.height + FRAME_PADDING;
        let content_width = size.width.saturating_sub(FRAME_PADDING).max(1);
        let viewport_height = size.height.saturating_sub(body_top + FRAME_PADDING).max(1);
        let layout = match &self.document.content {
            DocumentContent::Loaded(_) => self.current_layout(),
            _ => self.document.layout(content_width, &mut self.fonts),
        };
        let max_scroll_y = max_scroll(viewport_height, layout.content_height);
        let previous_scroll_y = self.scroll_y;
        self.scroll_y = self.scroll_y.min(max_scroll_y);
        if self.scroll_y != previous_scroll_y {
            let _ = self.document.set_scroll_position(self.scroll_y);
            self.sync_current_history_scroll();
        }

        let Some(surface) = self.surface.as_mut() else {
            return Ok(());
        };

        surface
            .resize(
                NonZeroU32::new(size.width).expect("width is checked above"),
                NonZeroU32::new(size.height).expect("height is checked above"),
            )
            .map_err(|error| BrowserError::message(error.to_string()))?;

        let mut buffer = surface
            .buffer_mut()
            .map_err(|error| BrowserError::message(error.to_string()))?;

        paint_background(
            &mut buffer,
            size.width,
            size.height,
            chrome.height,
            layout.background_color,
        );
        paint_chrome(
            &mut self.fonts,
            &mut buffer,
            size.width,
            size.height,
            &chrome,
            &self.document,
            &self.address_bar,
            self.hovered_target,
            can_go_back,
            can_go_forward,
            self.scroll_y,
            max_scroll_y,
        );
        if let Some(frame) = self
            .latest_render_frame
            .as_ref()
            .filter(|frame| frame.content_width == content_width)
        {
            blit_rendered_content_frame(
                &mut buffer,
                size.width,
                size.height,
                FRAME_PADDING / 2,
                body_top,
                viewport_height,
                self.scroll_y,
                frame,
            );
        } else if !matches!(self.document.content, DocumentContent::Loaded(_)) {
            paint_layout(
                None,
                &mut self.fonts,
                &mut buffer,
                size.width,
                size.height,
                FRAME_PADDING / 2,
                body_top,
                viewport_height,
                self.scroll_y,
                &layout,
                self.focused_page_input.as_ref(),
                self.hovered_target,
                &mut self.scratch,
            );
        }

        buffer
            .present()
            .map_err(|error| BrowserError::message(error.to_string()))
    }

    fn content_metrics(&mut self, window_size: PhysicalSize<u32>) -> (u32, u32) {
        let chrome = chrome_layout_metrics(&mut self.fonts, window_size.width);
        let body_top = chrome.height + FRAME_PADDING;
        let _content_width = window_size.width.saturating_sub(FRAME_PADDING).max(1);
        let viewport_height = window_size
            .height
            .saturating_sub(body_top + FRAME_PADDING)
            .max(1);
        let content_height = self.current_layout().content_height;

        (viewport_height, content_height)
    }

    fn find_hovered_link(
        &mut self,
        window_size: PhysicalSize<u32>,
    ) -> Option<(String, Option<usize>)> {
        let chrome = chrome_layout_metrics(&mut self.fonts, window_size.width);
        let body_top = chrome.height + FRAME_PADDING;
        let pos_x = self.cursor_position.x;
        let pos_y = self.cursor_position.y;
        if pos_y < body_top as f64 {
            return None;
        }
        let _content_width = window_size.width.saturating_sub(FRAME_PADDING).max(1);
        let layout = self.current_layout();
        let content_y = (pos_y as u32)
            .saturating_sub(body_top)
            .saturating_add(self.scroll_y);
        let content_x = (pos_x as u32).saturating_sub(FRAME_PADDING / 2);
        for link in &layout.links {
            if content_x >= link.x
                && content_x < link.x.saturating_add(link.width)
                && content_y >= link.y
                && content_y < link.y.saturating_add(link.height)
            {
                return Some((link.href.clone(), link.node_id));
            }
        }
        None
    }

    fn find_hovered_page_control(&mut self, window_size: PhysicalSize<u32>) -> Option<HitTarget> {
        let chrome = chrome_layout_metrics(&mut self.fonts, window_size.width);
        let body_top = chrome.height + FRAME_PADDING;
        let pos_x = self.cursor_position.x;
        let pos_y = self.cursor_position.y;
        if pos_y < body_top as f64 {
            return None;
        }
        let _content_width = window_size.width.saturating_sub(FRAME_PADDING).max(1);
        let layout = self.current_layout();
        let content_y = (pos_y as u32)
            .saturating_sub(body_top)
            .saturating_add(self.scroll_y);
        let content_x = (pos_x as u32).saturating_sub(FRAME_PADDING / 2);
        for control in &layout.controls {
            if content_x >= control.x
                && content_x < control.x.saturating_add(control.width)
                && content_y >= control.y
                && content_y < control.y.saturating_add(control.height)
            {
                return Some(match control.kind {
                    FormControlKind::TextInput => HitTarget::PageTextInput(control.id),
                    FormControlKind::Button => HitTarget::PageButton(control.id),
                    FormControlKind::Hidden => return None,
                });
            }
        }
        None
    }

    fn update_hover(&mut self, window_size: PhysicalSize<u32>) {
        let next = self.hit_test(window_size, self.cursor_position);
        let (link_url, link_node_id) = if next == HitTarget::None {
            self.find_hovered_link(window_size)
                .map(|(href, node_id)| (Some(href), node_id))
                .unwrap_or((None, None))
        } else {
            (None, None)
        };

        // Determine which element (by node_id) is under the cursor
        let hovered_element = self.find_hovered_element(window_size);

        let element_changed = hovered_element != self.hovered_element_node_id;
        // Only relayout when hovered element changes
        if element_changed {
            let previous_hovered_element = self.hovered_element_node_id;
            self.hovered_element_node_id = hovered_element;
            self.dispatch_hover_transition_events(
                window_size,
                previous_hovered_element,
                hovered_element,
            );
            // Rebuild the styled tree so CSS `:hover` applies, starting any transitions.
            self.restyle_for_hover(window_size);
            // While an animation/transition is running the frame loop already re-renders
            // every frame; an extra async render here would just contend with it (and could
            // briefly present a stale frame). Only request one when idle.
            if self.animation_epoch.is_none() {
                self.request_content_render();
            }
        }

        let changed = next != self.hovered_target
            || link_url != self.hovered_link_url
            || link_node_id != self.hovered_link_node_id
            || element_changed;
        self.hovered_target = next;
        self.hovered_link_url = link_url;
        self.hovered_link_node_id = link_node_id;
        if changed {
            self.request_redraw();
        }
        // Determine cursor icon before borrowing self.window
        let icon = if self.hovered_link_url.is_some() {
            CursorIcon::Pointer
        } else if next == HitTarget::None {
            let elem_cursor = self.find_hovered_element_cursor(window_size);
            match elem_cursor {
                CursorKind::Pointer => CursorIcon::Pointer,
                CursorKind::Text => CursorIcon::Text,
                CursorKind::Move => CursorIcon::Move,
                CursorKind::Crosshair => CursorIcon::Crosshair,
                CursorKind::Wait => CursorIcon::Wait,
                CursorKind::Help => CursorIcon::Help,
                CursorKind::NotAllowed => CursorIcon::NotAllowed,
                CursorKind::Grab => CursorIcon::Grab,
                CursorKind::Grabbing => CursorIcon::Grabbing,
                CursorKind::ZoomIn => CursorIcon::ZoomIn,
                CursorKind::ZoomOut => CursorIcon::ZoomOut,
                CursorKind::None | CursorKind::Default => CursorIcon::Default,
                CursorKind::Auto => cursor_icon_for_target(next),
            }
        } else {
            cursor_icon_for_target(next)
        };
        if let Some(window) = &self.window {
            window.set_cursor(icon);
        }
    }

    fn find_hovered_element_cursor(&mut self, window_size: PhysicalSize<u32>) -> CursorKind {
        let chrome = chrome_layout_metrics(&mut self.fonts, window_size.width);
        let body_top = chrome.height + FRAME_PADDING;
        let pos_x = self.cursor_position.x;
        let pos_y = self.cursor_position.y;
        if pos_y < body_top as f64 {
            return CursorKind::Auto;
        }
        let _content_width = window_size.width.saturating_sub(FRAME_PADDING).max(1);
        let layout = self.current_layout();
        let content_y = (pos_y as u32)
            .saturating_sub(body_top)
            .saturating_add(self.scroll_y);
        let content_x = (pos_x as u32).saturating_sub(FRAME_PADDING / 2);
        layout
            .element_hitboxes
            .iter()
            .rev()
            .find(|h| {
                content_x >= h.x
                    && content_x < h.x + h.width
                    && content_y >= h.y
                    && content_y < h.y + h.height
            })
            .map(|h| h.cursor_kind)
            .unwrap_or(CursorKind::Auto)
    }

    fn find_hovered_element(&mut self, window_size: PhysicalSize<u32>) -> Option<usize> {
        let chrome = chrome_layout_metrics(&mut self.fonts, window_size.width);
        let body_top = chrome.height + FRAME_PADDING;
        let pos_x = self.cursor_position.x;
        let pos_y = self.cursor_position.y;
        if pos_y < body_top as f64 {
            return None;
        }
        let content_y = (pos_y as u32)
            .saturating_sub(body_top)
            .saturating_add(self.scroll_y);
        let content_x = (pos_x as u32).saturating_sub(FRAME_PADDING / 2);

        let hit = |hitboxes: &[ElementHitbox]| -> Option<usize> {
            // Pick the smallest hitbox that contains the cursor — i.e. the most deeply
            // nested element. (Hitboxes are emitted post-order, parent after children, so
            // taking the last match would wrongly return the outermost ancestor, e.g. the
            // <body>, and CSS :hover would target that instead of the actual element.)
            hitboxes
                .iter()
                .filter(|h| {
                    content_x >= h.x
                        && content_x < h.x + h.width
                        && content_y >= h.y
                        && content_y < h.y + h.height
                })
                .min_by_key(|h| h.width as u64 * h.height as u64)
                .map(|h| h.node_id)
        };

        // Prefer the last rendered frame's hitboxes: they survive the layout-cache
        // invalidations that hover restyling and the animation frame loop trigger, so a
        // tiny cursor jitter mid-transition does not momentarily "lose" the hovered element
        // (which would cancel an in-flight :hover transition).
        if let Some(frame) = &self.latest_render_frame {
            return hit(&frame.layout.element_hitboxes);
        }
        let layout = self.current_layout();
        hit(&layout.element_hitboxes)
    }

    fn hit_test(
        &mut self,
        window_size: PhysicalSize<u32>,
        position: PhysicalPosition<f64>,
    ) -> HitTarget {
        let chrome = chrome_layout_metrics(&mut self.fonts, window_size.width);

        if chrome.minimize_button.contains(position) {
            return HitTarget::Button(ChromeButton::Minimize);
        }
        if chrome.maximize_button.contains(position) {
            return HitTarget::Button(ChromeButton::ToggleMaximize);
        }
        if chrome.close_button.contains(position) {
            return HitTarget::Button(ChromeButton::Close);
        }
        if chrome.back_button.contains(position) {
            return HitTarget::Button(ChromeButton::Back);
        }
        if chrome.forward_button.contains(position) {
            return HitTarget::Button(ChromeButton::Forward);
        }
        if chrome.reload_button.contains(position) {
            return HitTarget::Button(ChromeButton::Reload);
        }
        if chrome.go_button.contains(position) {
            return HitTarget::Button(ChromeButton::Navigate);
        }
        if chrome.address_bar.contains(position) {
            return HitTarget::AddressBar;
        }
        if let Some(target) = self.find_hovered_page_control(window_size) {
            return target;
        }
        if let Some(direction) = resize_direction_at(window_size, position) {
            return HitTarget::Resize(direction);
        }
        if chrome.drag_region.contains(position) {
            return HitTarget::TitleBar;
        }

        HitTarget::None
    }

    fn handle_key(
        &mut self,
        key_code: KeyCode,
        window_size: PhysicalSize<u32>,
        repeat: bool,
    ) -> bool {
        if !repeat && self.modifiers.alt_key() && !self.modifiers.control_key() {
            match key_code {
                KeyCode::ArrowLeft => {
                    self.go_back();
                    return false;
                }
                KeyCode::ArrowRight => {
                    self.go_forward();
                    return false;
                }
                _ => {}
            }
        }

        if self.address_bar.focused {
            let control = self.modifiers.control_key();
            let shift = self.modifiers.shift_key();
            match key_code {
                KeyCode::Escape if !repeat => {
                    self.blur_address_bar();
                    return false;
                }
                KeyCode::Enter | KeyCode::NumpadEnter if !repeat => {
                    self.navigate_to_address();
                    return false;
                }
                KeyCode::Backspace => {
                    let changed = if control {
                        self.address_bar.delete_word_backward()
                    } else {
                        self.address_bar.backspace()
                    };
                    if changed {
                        self.sync_input_method();
                        self.request_redraw();
                    }
                    return false;
                }
                KeyCode::Delete => {
                    let changed = if control {
                        self.address_bar.delete_word_forward()
                    } else {
                        self.address_bar.delete_forward()
                    };
                    if changed {
                        self.sync_input_method();
                        self.request_redraw();
                    }
                    return false;
                }
                KeyCode::ArrowLeft => {
                    let changed = if control {
                        self.address_bar.move_word_left(shift)
                    } else {
                        self.address_bar.move_left(shift)
                    };
                    if changed {
                        self.sync_input_method();
                        self.request_redraw();
                    }
                    return false;
                }
                KeyCode::ArrowRight => {
                    let changed = if control {
                        self.address_bar.move_word_right(shift)
                    } else {
                        self.address_bar.move_right(shift)
                    };
                    if changed {
                        self.sync_input_method();
                        self.request_redraw();
                    }
                    return false;
                }
                KeyCode::Home => {
                    if self.address_bar.move_home(shift) {
                        self.sync_input_method();
                        self.request_redraw();
                    }
                    return false;
                }
                KeyCode::End => {
                    if self.address_bar.move_end(shift) {
                        self.sync_input_method();
                        self.request_redraw();
                    }
                    return false;
                }
                KeyCode::KeyA if control && !repeat => {
                    if self.address_bar.select_all() {
                        self.refresh_address_bar_input();
                    }
                    return false;
                }
                KeyCode::KeyC if control && !repeat => {
                    self.copy_address_selection();
                    return false;
                }
                KeyCode::KeyX if control && !repeat => {
                    if self.cut_address_selection() {
                        self.refresh_address_bar_input();
                    }
                    return false;
                }
                KeyCode::KeyV if control && !repeat => {
                    if self.paste_into_address_bar() {
                        self.refresh_address_bar_input();
                    }
                    return false;
                }
                KeyCode::KeyL if control && !repeat => {
                    self.address_bar.select_all();
                    self.refresh_address_bar_input();
                    return false;
                }
                KeyCode::KeyR if control && !repeat => {
                    self.reload();
                    return false;
                }
                KeyCode::F5 if !repeat => {
                    self.reload();
                    return false;
                }
                _ => return false,
            }
        }

        if self.focused_page_input.is_some() {
            let control = self.modifiers.control_key();
            let shift = self.modifiers.shift_key();
            let focused_control_id = self
                .focused_page_input
                .as_ref()
                .map(|focused| focused.control_id);
            match key_code {
                KeyCode::Escape if !repeat => {
                    self.blur_page_input();
                    return false;
                }
                KeyCode::Enter | KeyCode::NumpadEnter if !repeat => {
                    if let Some(control_id) = focused_control_id {
                        self.submit_page_form(control_id);
                    }
                    return false;
                }
                KeyCode::Backspace => {
                    let changed = if control {
                        self.focused_page_editor_mut()
                            .map(AddressBarState::delete_word_backward)
                            .unwrap_or(false)
                    } else {
                        self.focused_page_editor_mut()
                            .map(AddressBarState::backspace)
                            .unwrap_or(false)
                    };
                    if changed {
                        self.sync_page_input_value();
                        self.dispatch_focused_page_input_event("input", true, false);
                        self.sync_input_method();
                        self.request_redraw();
                    }
                    return false;
                }
                KeyCode::Delete => {
                    let changed = if control {
                        self.focused_page_editor_mut()
                            .map(AddressBarState::delete_word_forward)
                            .unwrap_or(false)
                    } else {
                        self.focused_page_editor_mut()
                            .map(AddressBarState::delete_forward)
                            .unwrap_or(false)
                    };
                    if changed {
                        self.sync_page_input_value();
                        self.dispatch_focused_page_input_event("input", true, false);
                        self.sync_input_method();
                        self.request_redraw();
                    }
                    return false;
                }
                KeyCode::ArrowLeft => {
                    let changed = if control {
                        self.focused_page_editor_mut()
                            .map(|editor| editor.move_word_left(shift))
                            .unwrap_or(false)
                    } else {
                        self.focused_page_editor_mut()
                            .map(|editor| editor.move_left(shift))
                            .unwrap_or(false)
                    };
                    if changed {
                        self.sync_input_method();
                        self.request_redraw();
                    }
                    return false;
                }
                KeyCode::ArrowRight => {
                    let changed = if control {
                        self.focused_page_editor_mut()
                            .map(|editor| editor.move_word_right(shift))
                            .unwrap_or(false)
                    } else {
                        self.focused_page_editor_mut()
                            .map(|editor| editor.move_right(shift))
                            .unwrap_or(false)
                    };
                    if changed {
                        self.sync_input_method();
                        self.request_redraw();
                    }
                    return false;
                }
                KeyCode::Home => {
                    if self
                        .focused_page_editor_mut()
                        .map(|editor| editor.move_home(shift))
                        .unwrap_or(false)
                    {
                        self.sync_input_method();
                        self.request_redraw();
                    }
                    return false;
                }
                KeyCode::End => {
                    if self
                        .focused_page_editor_mut()
                        .map(|editor| editor.move_end(shift))
                        .unwrap_or(false)
                    {
                        self.sync_input_method();
                        self.request_redraw();
                    }
                    return false;
                }
                KeyCode::KeyA if control && !repeat => {
                    if self
                        .focused_page_editor_mut()
                        .map(AddressBarState::select_all)
                        .unwrap_or(false)
                    {
                        self.sync_input_method();
                        self.request_redraw();
                    }
                    return false;
                }
                KeyCode::KeyC if control && !repeat => {
                    self.copy_page_input_selection();
                    return false;
                }
                KeyCode::KeyX if control && !repeat => {
                    if self.cut_page_input_selection() {
                        self.sync_input_method();
                        self.request_redraw();
                    }
                    return false;
                }
                KeyCode::KeyV if control && !repeat => {
                    if self.paste_into_page_input() {
                        self.sync_input_method();
                        self.request_redraw();
                    }
                    return false;
                }
                KeyCode::KeyL if control && !repeat => {
                    self.blur_page_input();
                    self.focus_address_bar_select_all();
                    return false;
                }
                _ => return false,
            }
        }

        let (viewport_height, content_height) = self.content_metrics(window_size);
        match key_code {
            KeyCode::Escape if !repeat => return true,
            KeyCode::ArrowDown => {
                let previous_scroll_y = self.scroll_y;
                self.scroll_by(24, viewport_height, content_height);
                if self.scroll_y != previous_scroll_y {
                    self.sync_scroll_position();
                }
            }
            KeyCode::ArrowUp => {
                let previous_scroll_y = self.scroll_y;
                self.scroll_by(-24, viewport_height, content_height);
                if self.scroll_y != previous_scroll_y {
                    self.sync_scroll_position();
                }
            }
            KeyCode::PageDown => {
                let previous_scroll_y = self.scroll_y;
                self.scroll_by(
                    viewport_height.saturating_sub(32) as i32,
                    viewport_height,
                    content_height,
                );
                if self.scroll_y != previous_scroll_y {
                    self.sync_scroll_position();
                }
            }
            KeyCode::PageUp => {
                let previous_scroll_y = self.scroll_y;
                self.scroll_by(
                    -(viewport_height.saturating_sub(32) as i32),
                    viewport_height,
                    content_height,
                );
                if self.scroll_y != previous_scroll_y {
                    self.sync_scroll_position();
                }
            }
            KeyCode::Home => {
                let previous_scroll_y = self.scroll_y;
                self.scroll_y = 0;
                if self.scroll_y != previous_scroll_y {
                    self.sync_scroll_position();
                }
            }
            KeyCode::End => {
                let previous_scroll_y = self.scroll_y;
                self.scroll_y = max_scroll(viewport_height, content_height);
                if self.scroll_y != previous_scroll_y {
                    self.sync_scroll_position();
                }
            }
            KeyCode::KeyR | KeyCode::F5 if !repeat => self.reload(),
            KeyCode::KeyL if self.modifiers.control_key() && !repeat => {
                self.focus_address_bar_select_all();
                return false;
            }
            _ => return false,
        }

        self.request_redraw();
        false
    }

    fn handle_text_input(&mut self, text: &str) {
        if self.ime_composing {
            return;
        }

        if self.address_bar.focused && self.address_bar.insert_text(text) {
            self.refresh_address_bar_input();
        } else if self
            .focused_page_editor_mut()
            .map(|editor| editor.insert_text(text))
            .unwrap_or(false)
        {
            self.sync_page_input_value();
            self.dispatch_focused_page_input_event("input", true, false);
            self.sync_input_method();
            self.request_redraw();
        }
    }

    fn handle_ime(&mut self, ime: Ime) {
        match ime {
            Ime::Enabled => {}
            Ime::Disabled => self.ime_composing = false,
            Ime::Preedit(text, _) => {
                self.ime_composing = !text.is_empty();
            }
            Ime::Commit(text) => {
                self.ime_composing = false;
                if self.address_bar.focused && self.address_bar.insert_text(&text) {
                    self.refresh_address_bar_input();
                } else if self
                    .focused_page_editor_mut()
                    .map(|editor| editor.insert_text(&text))
                    .unwrap_or(false)
                {
                    self.sync_page_input_value();
                    self.dispatch_focused_page_input_event("input", true, false);
                    self.sync_input_method();
                    self.request_redraw();
                }
            }
        }
    }

    fn refresh_address_bar_input(&mut self) {
        self.sync_input_method();
        self.request_redraw();
    }

    fn clear_page_control_state(&mut self) {
        self.focused_page_input = None;
        self.ime_composing = false;
        self.hovered_target = HitTarget::None;
        self.hovered_link_url = None;
        self.hovered_link_node_id = None;
        self.hovered_element_node_id = None;
    }

    fn blur_page_input(&mut self) {
        let Some(focused) = self.focused_page_input.clone() else {
            return;
        };

        let changed = focused.initial_value != focused.editor.text();
        self.dispatch_page_dom_event(focused.node_id, "blur", false, false);
        if changed {
            self.dispatch_page_dom_event(focused.node_id, "change", true, false);
        }

        self.focused_page_input = None;
        self.ime_composing = false;
        self.sync_input_method();
        self.request_content_render();
        self.request_redraw();
    }

    fn control_current_value(&self, control: &FormControlCommand) -> String {
        resolve_text_input_value(
            self.focused_page_input
                .as_ref()
                .filter(|focused| focused.control_id == control.id)
                .map(|focused| &focused.editor),
            &control.value,
        )
    }

    fn focus_page_input_at(&mut self, control: &FormControlCommand, char_index: Option<usize>) {
        if control.disabled {
            return;
        }
        self.blur_address_bar();
        let mut editor = AddressBarState::new(self.control_current_value(control));
        editor.focus_at(char_index.unwrap_or_else(|| editor.char_len()));
        self.focused_page_input = Some(FocusedPageInput {
            control_id: control.id,
            node_id: control.node_id,
            initial_value: editor.text().to_string(),
            editor,
        });
        self.sync_page_input_value();
        self.dispatch_page_dom_event(control.node_id, "focus", false, false);
        self.refresh_focused_page_input_from_document();
        self.sync_input_method();
        self.request_content_render();
        self.request_redraw();
    }

    fn sync_page_input_value(&mut self) {
        let Some((node_id, value)) = self
            .focused_page_input
            .as_ref()
            .map(|focused| (focused.node_id, focused.editor.text().to_string()))
        else {
            return;
        };

        self.document.set_dom_attribute(node_id, "value", &value);
    }

    fn refresh_focused_page_input_from_document(&mut self) {
        let Some(focused_id) = self
            .focused_page_input
            .as_ref()
            .map(|focused| focused.control_id)
        else {
            return;
        };
        let Some(control) = self.current_page_control(focused_id) else {
            self.focused_page_input = None;
            return;
        };
        if let Some(focused) = self.focused_page_input.as_mut()
            && control.value != focused.initial_value
            && focused.editor.text() != control.value
        {
            let cursor = focused
                .editor
                .cursor_chars
                .min(control.value.chars().count());
            focused.editor.set_text(control.value.clone());
            focused.editor.focus_at(cursor);
            focused.initial_value = control.value.clone();
        }
        self.sync_page_input_value();
    }

    fn dispatch_page_dom_event(
        &mut self,
        node_id: Option<usize>,
        event_type: &str,
        bubbles: bool,
        cancelable: bool,
    ) -> Option<DomEventDispatchResult> {
        let node_id = node_id?;
        self.dispatch_page_dom_event_request(DomEventRequest {
            target_node_id: node_id,
            event_type: event_type.to_string(),
            bubbles,
            cancelable,
            ..Default::default()
        })
    }

    fn dispatch_page_mouse_event(
        &mut self,
        node_id: Option<usize>,
        event_type: &str,
        bubbles: bool,
        cancelable: bool,
        related_target_node_id: Option<usize>,
        window_size: PhysicalSize<u32>,
    ) -> Option<DomEventDispatchResult> {
        let node_id = node_id?;
        let chrome = chrome_layout_metrics(&mut self.fonts, window_size.width);
        let client_x = (self.cursor_position.x - (FRAME_PADDING / 2) as f64)
            .round()
            .clamp(i32::MIN as f64, i32::MAX as f64) as i32;
        let client_y = (self.cursor_position.y - (chrome.height + FRAME_PADDING) as f64)
            .round()
            .clamp(i32::MIN as f64, i32::MAX as f64) as i32;
        self.dispatch_page_dom_event_request(DomEventRequest {
            target_node_id: node_id,
            event_type: event_type.to_string(),
            bubbles,
            cancelable,
            related_target_node_id,
            client_x: Some(client_x),
            client_y: Some(client_y),
            button: Some(0),
            buttons: Some(0),
            ..Default::default()
        })
    }

    fn dispatch_hover_transition_events(
        &mut self,
        window_size: PhysicalSize<u32>,
        previous_hovered_element: Option<usize>,
        next_hovered_element: Option<usize>,
    ) {
        if previous_hovered_element == next_hovered_element {
            return;
        }

        let page_revision_before = self.document.layout_revision();
        let current_url_before = self.current_url.clone();

        if let Some(previous) = previous_hovered_element {
            let _ = self.dispatch_page_mouse_event(
                Some(previous),
                "mouseout",
                true,
                true,
                next_hovered_element,
                window_size,
            );
            if self.current_url != current_url_before
                || self.document.layout_revision() != page_revision_before
            {
                return;
            }

            let _ = self.dispatch_page_mouse_event(
                Some(previous),
                "mouseleave",
                false,
                false,
                next_hovered_element,
                window_size,
            );
            if self.current_url != current_url_before
                || self.document.layout_revision() != page_revision_before
            {
                return;
            }
        }

        if let Some(next) = next_hovered_element {
            let _ = self.dispatch_page_mouse_event(
                Some(next),
                "mouseover",
                true,
                true,
                previous_hovered_element,
                window_size,
            );
            if self.current_url != current_url_before
                || self.document.layout_revision() != page_revision_before
            {
                return;
            }

            let _ = self.dispatch_page_mouse_event(
                Some(next),
                "mouseenter",
                false,
                false,
                previous_hovered_element,
                window_size,
            );
        }
    }

    fn dispatch_page_dom_event_request(
        &mut self,
        request: DomEventRequest,
    ) -> Option<DomEventDispatchResult> {
        let result = self.document.dispatch_dom_event(request)?;

        if let Some(target_url) = result.snapshot.navigation_target.clone()
            && let Ok(url) = Url::parse(&target_url)
        {
            self.load_url(url);
            return Some(result);
        }
        if let Some(soft_target) = result.snapshot.soft_navigation_target.clone()
            && let Ok(url) = Url::parse(&soft_target)
        {
            self.current_url = Some(url.clone());
            self.address_bar.set_text(url.to_string());
            self.replace_current_history_entry(url);
        }

        self.scroll_y = self.document.scroll_position();
        self.sync_current_history_scroll();
        self.document.sync_from_loaded_page();
        self.sync_window_title();
        self.refresh_focused_page_input_from_document();
        self.sync_input_method();
        self.request_content_render();
        self.request_redraw();
        self.maybe_spawn_fetch_settle_watcher();
        Some(result)
    }

    fn maybe_spawn_fetch_settle_watcher(&self) {
        let Some(session) = self.document.javascript_session() else {
            return;
        };
        if !session.has_pending_fetches() {
            return;
        }

        let event_proxy = self.event_proxy.clone();
        thread::Builder::new()
            .name("tobira-fetch-settle-watch".to_string())
            .spawn(move || {
                let started = Instant::now();
                while session.has_pending_fetches() && started.elapsed() < JS_FETCH_SETTLE_TIMEOUT {
                    thread::sleep(Duration::from_millis(5));
                }
                let _ = event_proxy.send_event(BrowserUserEvent::JsSettleRequired);
            })
            .expect("fetch settle watcher should start");
    }

    fn dispatch_focused_page_input_event(
        &mut self,
        event_type: &str,
        bubbles: bool,
        cancelable: bool,
    ) -> Option<DomEventDispatchResult> {
        let node_id = self
            .focused_page_input
            .as_ref()
            .and_then(|focused| focused.node_id);
        self.dispatch_page_dom_event(node_id, event_type, bubbles, cancelable)
    }

    fn dispatch_focused_page_keyboard_event(
        &mut self,
        event_type: &str,
        key_code: KeyCode,
        repeat: bool,
    ) -> Option<DomEventDispatchResult> {
        let node_id = self
            .focused_page_input
            .as_ref()
            .and_then(|focused| focused.node_id)?;
        self.dispatch_page_dom_event_request(keyboard_dom_event_request(
            node_id,
            event_type,
            key_code,
            repeat,
            &self.modifiers,
        ))
    }

    fn focused_page_editor_mut(&mut self) -> Option<&mut AddressBarState> {
        self.focused_page_input
            .as_mut()
            .map(|focused| &mut focused.editor)
    }

    fn copy_address_selection(&self) -> bool {
        let Some(text) = self.address_bar.selected_text() else {
            return false;
        };

        write_clipboard_text(&text)
    }

    fn cut_address_selection(&mut self) -> bool {
        let Some(text) = self.address_bar.selected_text() else {
            return false;
        };
        if !write_clipboard_text(&text) {
            return false;
        }

        self.address_bar.cut_selection_text().is_some()
    }

    fn paste_into_address_bar(&mut self) -> bool {
        let Some(text) = read_clipboard_text() else {
            return false;
        };

        self.address_bar.insert_text(&text)
    }

    fn handle_wheel(&mut self, delta: MouseScrollDelta, window_size: PhysicalSize<u32>) {
        let (viewport_height, content_height) = self.content_metrics(window_size);
        let previous_scroll_y = self.scroll_y;
        match delta {
            MouseScrollDelta::LineDelta(_, y) => {
                self.scroll_by((-(y.round() as i32)) * 24, viewport_height, content_height);
            }
            MouseScrollDelta::PixelDelta(position) => {
                self.scroll_by(
                    -(position.y.round() as i32),
                    viewport_height,
                    content_height,
                );
            }
        }

        if self.scroll_y != previous_scroll_y {
            if self.document.has_global_event_listener("scroll") {
                self.sync_scroll_position();
            } else {
                let _ = self.document.set_scroll_position(self.scroll_y);
                self.sync_current_history_scroll();
                let _ = self.draw();
            }
            return;
        }
        self.request_redraw();
    }

    fn page_base_url(&self) -> Option<&Url> {
        match &self.document.content {
            DocumentContent::Loaded(page) => Some(&page.url),
            _ => self.current_url.as_ref(),
        }
    }

    fn copy_page_input_selection(&self) -> bool {
        let Some(text) = self
            .focused_page_input
            .as_ref()
            .and_then(|focused| focused.editor.selected_text())
        else {
            return false;
        };

        write_clipboard_text(&text)
    }

    fn cut_page_input_selection(&mut self) -> bool {
        let Some(text) = self
            .focused_page_input
            .as_ref()
            .and_then(|focused| focused.editor.selected_text())
        else {
            return false;
        };
        if !write_clipboard_text(&text) {
            return false;
        }

        let changed = self
            .focused_page_editor_mut()
            .and_then(AddressBarState::cut_selection_text)
            .is_some();
        if changed {
            self.sync_page_input_value();
            self.dispatch_focused_page_input_event("input", true, false);
        }
        changed
    }

    fn paste_into_page_input(&mut self) -> bool {
        let Some(text) = read_clipboard_text() else {
            return false;
        };

        let changed = self
            .focused_page_editor_mut()
            .map(|editor| editor.insert_text(&text))
            .unwrap_or(false);
        if changed {
            self.sync_page_input_value();
            self.dispatch_focused_page_input_event("input", true, false);
        }
        changed
    }

    fn submit_page_form(&mut self, trigger_control_id: usize) {
        let Some(trigger) = self
            .current_layout()
            .controls
            .into_iter()
            .find(|control| control.id == trigger_control_id)
        else {
            return;
        };
        if trigger.disabled {
            return;
        }
        if matches!(trigger.kind, FormControlKind::Button) && !trigger.activates_submit {
            return;
        }
        let Some(trigger_form_id) = trigger.form_id else {
            return;
        };
        let trigger_form_node_id = trigger.form_node_id;
        let trigger_form_action = trigger.form_action.clone();
        let trigger_kind = trigger.kind;
        let trigger_name = trigger.name.clone();
        let trigger_value = trigger.value.clone();

        if let Some(result) =
            self.dispatch_page_dom_event(trigger_form_node_id, "submit", true, true)
        {
            if result.default_prevented || result.snapshot.navigation_target.is_some() {
                return;
            }
        }

        if !trigger.form_method.eq_ignore_ascii_case("get") {
            self.document.status_line = format!(
                "Status: {} forms are not supported yet",
                trigger.form_method.to_ascii_uppercase()
            );
            self.request_redraw();
            return;
        }

        let layout = self.current_layout();
        let mut fields = Vec::new();
        for control in &layout.controls {
            if control.form_id != Some(trigger_form_id) || control.disabled {
                continue;
            }
            if matches!(
                control.kind,
                FormControlKind::TextInput | FormControlKind::Hidden
            ) && let Some(name) = &control.name
                && !name.is_empty()
            {
                fields.push((name.clone(), self.control_current_value(control)));
            }
        }
        if matches!(trigger_kind, FormControlKind::Button)
            && let Some(name) = &trigger_name
            && !name.is_empty()
        {
            fields.push((name.clone(), trigger_value.clone()));
        }

        let base = self.page_base_url().or(self.current_url.as_ref());
        let action = trigger_form_action.as_deref().unwrap_or("");
        let resolved = if action.is_empty() {
            base.map(ToString::to_string)
        } else {
            Some(resolve_content_href(action, base))
        };
        let Some(url_text) = resolved else {
            return;
        };
        if let Some(target_url) =
            build_get_form_submission_url(&url_text, &fields, action.is_empty())
            && let Ok(url) = Url::parse(&target_url)
        {
            self.load_url(url);
        }
    }

    fn handle_left_click(&mut self, window_size: PhysicalSize<u32>) -> bool {
        let hit = self.hit_test(window_size, self.cursor_position);
        let Some(window) = self.window.as_ref().cloned() else {
            return false;
        };

        match hit {
            HitTarget::Button(button) => return self.handle_button(button),
            HitTarget::AddressBar => {
                self.blur_page_input();
                let chrome = chrome_layout_metrics(&mut self.fonts, window_size.width);
                let char_index = cursor_index_for_address_x(
                    &self.address_bar,
                    &mut self.fonts,
                    chrome
                        .address_bar
                        .width
                        .saturating_sub(ADDRESS_BAR_PADDING_X * 2),
                    self.cursor_position.x.max(chrome.address_bar.x as f64)
                        - (chrome.address_bar.x + ADDRESS_BAR_PADDING_X) as f64,
                );
                self.focus_address_bar(char_index);
            }
            HitTarget::PageTextInput(control_id) => {
                if let Some(control) = self.current_page_control(control_id) {
                    if control.disabled {
                        return false;
                    }
                    let click_result =
                        self.dispatch_page_dom_event(control.node_id, "click", true, true);
                    if let Some(result) = click_result {
                        if result.default_prevented || result.snapshot.navigation_target.is_some() {
                            return false;
                        }
                    }
                    let Some(control) = self.current_page_control(control_id) else {
                        return false;
                    };
                    let local_x = self
                        .cursor_position
                        .x
                        .max((FRAME_PADDING / 2 + control.x + CONTROL_PADDING_X) as f64)
                        - (FRAME_PADDING / 2 + control.x + CONTROL_PADDING_X) as f64;
                    let mut editor = AddressBarState::new(self.control_current_value(&control));
                    editor.focus_at(editor.char_len());
                    let char_index = cursor_index_for_text_x(
                        &editor,
                        &mut self.fonts,
                        control.width.saturating_sub(CONTROL_PADDING_X * 2),
                        local_x,
                        control.font_size_px,
                        control.font_family,
                    );
                    self.focus_page_input_at(&control, Some(char_index));
                }
            }
            HitTarget::PageButton(control_id) => {
                let control = self.current_page_control(control_id);
                if control
                    .as_ref()
                    .map(|control| control.disabled)
                    .unwrap_or(false)
                {
                    return false;
                }
                self.blur_address_bar();
                self.blur_page_input();
                if let Some(control) = control {
                    let click_result =
                        self.dispatch_page_dom_event(control.node_id, "click", true, true);
                    if let Some(result) = click_result {
                        if result.default_prevented || result.snapshot.navigation_target.is_some() {
                            return false;
                        }
                    }
                }
                self.submit_page_form(control_id);
            }
            HitTarget::Resize(direction) => {
                self.blur_address_bar();
                self.blur_page_input();
                let _ = window.drag_resize_window(direction);
            }
            HitTarget::TitleBar => {
                self.blur_address_bar();
                self.blur_page_input();
                let _ = window.drag_window();
            }
            HitTarget::None => {
                self.blur_address_bar();
                self.blur_page_input();
                if let Some(href) = self.hovered_link_url.clone() {
                    let node_id = self.hovered_link_node_id;
                    let click_result = self.dispatch_page_dom_event(node_id, "click", true, true);
                    if let Some(result) = click_result {
                        if result.default_prevented || result.snapshot.navigation_target.is_some() {
                            return false;
                        }
                    }
                    let resolved = resolve_content_href(&href, self.page_base_url());
                    if let Ok(url) = parse_address_input(&resolved) {
                        self.load_url(url);
                    }
                }
            }
        }

        false
    }

    fn handle_button(&mut self, button: ChromeButton) -> bool {
        self.blur_address_bar();
        self.blur_page_input();
        let Some(window) = self.window.as_ref().cloned() else {
            return false;
        };
        match button {
            ChromeButton::Back if self.can_go_back() => self.go_back(),
            ChromeButton::Forward if self.can_go_forward() => self.go_forward(),
            ChromeButton::Reload => self.reload(),
            ChromeButton::Navigate => self.navigate_to_address(),
            ChromeButton::Minimize => window.set_minimized(true),
            ChromeButton::ToggleMaximize => window.set_maximized(!window.is_maximized()),
            ChromeButton::Close => return true,
            _ => {}
        }

        false
    }
}

impl ApplicationHandler<BrowserUserEvent> for BrowserApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::Wait);

        if self.window.is_some() {
            return;
        }

        let window = event_loop
            .create_window(
                Window::default_attributes()
                    .with_title(self.document.window_title())
                    .with_decorations(false)
                    .with_inner_size(LogicalSize::new(WINDOW_WIDTH as f64, WINDOW_HEIGHT as f64))
                    .with_min_inner_size(LogicalSize::new(720.0, 480.0)),
            )
            .expect("window creation should succeed");

        let window = Rc::new(window);
        window.set_ime_allowed(false);
        let surface =
            Surface::new(&self.context, window.clone()).expect("surface creation should succeed");

        self.surface = Some(surface);
        self.window = Some(window);
        self.refresh_frame_interval();
        self.sync_viewport_size();
        self.sync_window_title();
        self.sync_input_method();
        self.request_redraw();
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: BrowserUserEvent) {
        match event {
            BrowserUserEvent::NavigationFinished {
                navigation_id,
                result,
            } => {
                self.finish_navigation(navigation_id, result);
            }
            BrowserUserEvent::RenderFinished { render_id, result } => {
                self.finish_render(render_id, result);
            }
            BrowserUserEvent::JsSettleRequired => {
                if self.document.refresh_loaded_page_from_script_session() {
                    self.scroll_y = self.document.scroll_position();
                    self.sync_current_history_scroll();
                    self.sync_window_title();
                    self.refresh_focused_page_input_from_document();
                    self.sync_input_method();
                    self.request_content_render();
                    self.request_redraw();
                }
                if self.document.has_pending_fetches() {
                    self.maybe_spawn_fetch_settle_watcher();
                }
            }
            BrowserUserEvent::AnimationTick => {
                self.tick_pending.store(false, Ordering::SeqCst);
                self.drive_animation_frame();
            }
        }
    }

    /// Called by winit when the event queue drains. While an animation is active it makes
    /// sure the ticker thread is running (which then drives frames via `AnimationTick`,
    /// reliably even during input) and renders one frame immediately so the animation starts
    /// without waiting for the first tick. The control flow stays `Wait`; the ticker, not a
    /// `WaitUntil` timer, provides the cadence.
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.animation_epoch.is_some() {
            // At the start of an animation burst, re-read the display refresh rate (the
            // monitor may not have been known at window creation) before spawning the ticker;
            // last_frame_at doubles as the "burst started" marker.
            if self.last_frame_at.is_none() {
                self.refresh_frame_interval();
                self.last_frame_at = Some(Instant::now());
            }
            self.ensure_animation_ticker();
            self.drive_animation_frame();
        }
        event_loop.set_control_flow(ControlFlow::Wait);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        let Some(window) = self.window.as_ref().cloned() else {
            return;
        };

        if window.id() != window_id {
            return;
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => {
                if let Err(error) = self.draw() {
                    self.document = DocumentView::error(format!("drawing failed: {error}"));
                    self.scroll_y = 0;
                    self.sync_window_title();
                    let _ = self.draw();
                }
            }
            WindowEvent::Resized(size) => {
                // Clear scratch pool on resize: buffer dimensions change, old buffers may be wrong size.
                self.scratch.clear();
                self.update_hover(size);
                self.sync_viewport_size();
                self.sync_input_method();
                self.request_redraw();
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_position = position;
                self.update_hover(window.inner_size());
            }
            WindowEvent::CursorLeft { .. } => {
                self.hovered_target = HitTarget::None;
                self.hovered_link_url = None;
                self.hovered_link_node_id = None;
                self.hovered_element_node_id = None;
                self.request_content_render();
                window.set_cursor(CursorIcon::Default);
                self.request_redraw();
            }
            WindowEvent::MouseWheel { delta, .. } => self.handle_wheel(delta, window.inner_size()),
            WindowEvent::MouseInput {
                button: MouseButton::Left,
                state: ElementState::Pressed,
                ..
            } => {
                if self.handle_left_click(window.inner_size()) {
                    event_loop.exit();
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
            }
            WindowEvent::Ime(ime) => self.handle_ime(ime),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let physical_key = match event.physical_key {
                    PhysicalKey::Code(key_code) => Some(key_code),
                    _ => None,
                };
                let focused_page_control_id = self
                    .focused_page_input
                    .as_ref()
                    .map(|focused| focused.control_id);
                let keydown_result = physical_key.and_then(|key_code| {
                    focused_page_control_id.and_then(|_| {
                        self.dispatch_focused_page_keyboard_event("keydown", key_code, event.repeat)
                    })
                });

                if keydown_result
                    .as_ref()
                    .is_some_and(|result| result.snapshot.navigation_target.is_some())
                {
                    return;
                }

                if let Some(key_code) = physical_key
                    && !keydown_result
                        .as_ref()
                        .is_some_and(|result| result.default_prevented)
                    && self.handle_key(key_code, window.inner_size(), event.repeat)
                {
                    event_loop.exit();
                    return;
                }

                if let Some(text) = event.text.as_deref()
                    && !keydown_result
                        .as_ref()
                        .is_some_and(|result| result.default_prevented)
                    && !self.modifiers.control_key()
                    && !self.modifiers.alt_key()
                    && !self.modifiers.super_key()
                {
                    self.handle_text_input(text);
                }

                if let (Some(key_code), Some(focused_control_id)) =
                    (physical_key, focused_page_control_id)
                    && self
                        .focused_page_input
                        .as_ref()
                        .map(|focused| focused.control_id)
                        == Some(focused_control_id)
                {
                    let _ =
                        self.dispatch_focused_page_keyboard_event("keyup", key_code, event.repeat);
                }
            }
            _ => {}
        }
    }
}

#[derive(Debug, Clone)]
struct DocumentView {
    title: String,
    status_line: String,
    subtitle: String,
    content: DocumentContent,
    layout_cache: Option<CachedLayout>,
}

#[derive(Debug, Clone)]
enum DocumentContent {
    Blank,
    Loading(LoadingDocument),
    Loaded(BrowserPage),
    Error(ErrorDocument),
}

#[derive(Debug, Clone)]
struct CachedLayout {
    width: u32,
    revision: u64,
    layout: LayoutDocument,
}

#[derive(Debug, Clone)]
struct ErrorDocument {
    lines: Vec<String>,
}

#[derive(Debug, Clone)]
struct LoadingDocument {
    lines: Vec<String>,
}

impl DocumentView {
    fn blank() -> Self {
        Self {
            title: "New Tab".to_string(),
            status_line: "Status: ready".to_string(),
            subtitle: "Type a URL in the address bar and press Enter.".to_string(),
            content: DocumentContent::Blank,
            layout_cache: None,
        }
    }

    fn loading(message: impl Into<String>) -> Self {
        Self {
            title: "Loading...".to_string(),
            status_line: "Status: loading".to_string(),
            subtitle: message.into(),
            content: DocumentContent::Loading(LoadingDocument {
                lines: vec![
                    "# Loading...".to_string(),
                    String::new(),
                    "Please wait while Tobira finishes the page load.".to_string(),
                ],
            }),
            layout_cache: None,
        }
    }

    fn from_page(page: BrowserPage) -> Self {
        let content_type = page
            .content_type
            .clone()
            .unwrap_or_else(|| "unknown".to_string());

        Self {
            title: page.title.clone(),
            status_line: format!("Status: {}", page.status_text()),
            subtitle: format!("{} | {}", page.url, content_type),
            content: DocumentContent::Loaded(page),
            layout_cache: None,
        }
    }

    fn render_snapshot(&mut self) -> Option<RenderPageSnapshot> {
        self.refresh_loaded_page_from_script_session();
        match &self.content {
            DocumentContent::Loaded(page) => Some(RenderPageSnapshot {
                styled_document: page.styled_document.clone(),
                images: page.images.clone(),
            }),
            _ => None,
        }
    }

    /// Mutable access to the loaded page, if the document currently holds one.
    /// Used by the animation frame loop to advance interpolated styles in place.
    fn loaded_page_mut(&mut self) -> Option<&mut BrowserPage> {
        match &mut self.content {
            DocumentContent::Loaded(page) => Some(page),
            _ => None,
        }
    }

    /// Shared access to the loaded page, if any.
    fn loaded_page(&self) -> Option<&BrowserPage> {
        match &self.content {
            DocumentContent::Loaded(page) => Some(page),
            _ => None,
        }
    }

    fn refresh_loaded_page_from_script_session(&mut self) -> bool {
        let changed = match &mut self.content {
            DocumentContent::Loaded(page) => page.refresh_from_script_session(),
            _ => false,
        };
        if changed {
            self.sync_from_loaded_page();
            self.layout_cache = None;
        }
        changed
    }

    fn javascript_session(&self) -> Option<JavaScriptSession> {
        match &self.content {
            DocumentContent::Loaded(page) => page.javascript_session(),
            _ => None,
        }
    }

    fn has_pending_fetches(&self) -> bool {
        match &self.content {
            DocumentContent::Loaded(page) => page.has_pending_fetches(),
            _ => false,
        }
    }

    fn feed_layout_hitboxes(&mut self, hitboxes: Vec<ElementHitbox>) -> bool {
        let changed = match &mut self.content {
            DocumentContent::Loaded(page) => page.set_layout_hitboxes(hitboxes),
            _ => false,
        };
        if changed {
            self.sync_from_loaded_page();
            self.layout_cache = None;
        }
        changed
    }

    fn scroll_position(&self) -> u32 {
        match &self.content {
            DocumentContent::Loaded(page) => page.scroll_y(),
            _ => 0,
        }
    }

    fn dispatch_dom_event(&mut self, request: DomEventRequest) -> Option<DomEventDispatchResult> {
        match &mut self.content {
            DocumentContent::Loaded(page) => page.dispatch_dom_event(request),
            _ => None,
        }
    }

    fn set_dom_attribute(&mut self, node_id: Option<usize>, name: &str, value: &str) {
        if let DocumentContent::Loaded(page) = &mut self.content {
            page.set_dom_attribute(node_id, name, value);
        }
    }

    fn set_viewport_size(&mut self, width: u32, height: u32) -> bool {
        match &mut self.content {
            DocumentContent::Loaded(page) => page.set_viewport_size(width, height),
            _ => false,
        }
    }

    fn set_scroll_position(&mut self, y: u32) -> bool {
        match &mut self.content {
            DocumentContent::Loaded(page) => page.set_scroll_position(y),
            _ => false,
        }
    }

    fn dispatch_window_resize(&mut self) -> bool {
        if !self.has_global_event_listener("resize") {
            return false;
        }
        let resized = match &mut self.content {
            DocumentContent::Loaded(page) => page.dispatch_window_resize().is_some(),
            _ => false,
        };
        if resized {
            self.sync_from_loaded_page();
            self.layout_cache = None;
        }
        resized
    }

    fn dispatch_scroll_event(&mut self) -> bool {
        let scrolled = match &mut self.content {
            DocumentContent::Loaded(page) => page.dispatch_scroll_event().is_some(),
            _ => false,
        };
        if scrolled {
            self.sync_from_loaded_page();
        }
        scrolled
    }

    fn has_global_event_listener(&mut self, event_type: &str) -> bool {
        match &mut self.content {
            DocumentContent::Loaded(page) => page.has_global_event_listener(event_type),
            _ => false,
        }
    }

    fn sync_from_loaded_page(&mut self) {
        if let DocumentContent::Loaded(page) = &self.content {
            let content_type = page
                .content_type
                .clone()
                .unwrap_or_else(|| "unknown".to_string());
            self.title = page.title.clone();
            self.status_line = format!("Status: {}", page.status_text());
            self.subtitle = format!("{} | {}", page.url, content_type);
        }
    }

    fn error(message: impl Into<String>) -> Self {
        Self {
            title: "Load Error".to_string(),
            status_line: "Status: request failed".to_string(),
            subtitle: "The browser core could not load this page.".to_string(),
            content: DocumentContent::Error(ErrorDocument {
                lines: vec![
                    "# Load Error".to_string(),
                    String::new(),
                    message.into(),
                    String::new(),
                    "Hints:".to_string(),
                    "- press Enter in the address bar to try a new URL".to_string(),
                    "- press R or the reload button to fetch the current page again".to_string(),
                    "- some modern CSS and JavaScript features are still incomplete".to_string(),
                ],
            }),
            layout_cache: None,
        }
    }

    fn window_title(&self) -> String {
        if self.title.is_empty() {
            "Tobira".to_string()
        } else {
            format!("Tobira - {}", self.title)
        }
    }

    fn is_error(&self) -> bool {
        matches!(self.content, DocumentContent::Error(_))
    }

    fn layout(&mut self, width: u32, fonts: &mut FontContext) -> LayoutDocument {
        const MAX_LAYOUT_FEEDBACK_PASSES: usize = 2;
        let mut passes = 0;
        loop {
            let revision = self.layout_revision();
            if let Some(cache) = &self.layout_cache
                && cache.width == width
                && cache.revision == revision
            {
                return cache.layout.clone();
            }

            let layout = match &self.content {
                DocumentContent::Blank => LayoutDocument {
                    background_color: DEFAULT_BACKGROUND_COLOR,
                    content_height: 0,
                    commands: Vec::new(),
                    links: Vec::new(),
                    controls: Vec::new(),
                    element_hitboxes: Vec::new(),
                },
                DocumentContent::Loading(loading) => layout_loading_document(loading, width, fonts),
                DocumentContent::Loaded(page) => {
                    layout_styled_document(&page.styled_document, &page.images, width, fonts)
                }
                DocumentContent::Error(error) => layout_error_document(error, width, fonts),
            };

            let refreshed = if let DocumentContent::Loaded(page) = &mut self.content {
                page.set_layout_hitboxes(layout.element_hitboxes.clone())
            } else {
                false
            };

            let refreshed_revision = self.layout_revision();
            if refreshed && refreshed_revision != revision && passes < MAX_LAYOUT_FEEDBACK_PASSES {
                self.layout_cache = None;
                passes += 1;
                continue;
            }

            self.layout_cache = Some(CachedLayout {
                width,
                revision: refreshed_revision,
                layout: layout.clone(),
            });
            return layout;
        }
    }

    fn layout_revision(&self) -> u64 {
        match &self.content {
            DocumentContent::Loaded(page) => page.layout_revision(),
            _ => 0,
        }
    }
}

fn layout_loading_document(
    document: &LoadingDocument,
    width: u32,
    fonts: &mut FontContext,
) -> LayoutDocument {
    layout_error_document(
        &ErrorDocument {
            lines: document.lines.clone(),
        },
        width,
        fonts,
    )
}

fn layout_error_document(
    document: &ErrorDocument,
    _width: u32,
    fonts: &mut FontContext,
) -> LayoutDocument {
    let mut commands = Vec::new();
    let mut cursor_y: u32 = 0;

    for line in &document.lines {
        let scale = if line.starts_with('#') { 3 } else { 2 };
        let color = if line.starts_with('#') {
            COLOR_ERROR
        } else {
            DEFAULT_TEXT_COLOR
        };
        let font_size_px = if scale >= 3 { 28 } else { 18 };
        let height = fonts.line_height_px(font_size_px, FontFamilyKind::Sans);

        if line.is_empty() {
            cursor_y = cursor_y.saturating_add(height / 2);
            continue;
        }

        commands.push(DrawCommand::Text(TextCommand {
            x: 0,
            y: cursor_y,
            width: fonts.text_width_px(line, font_size_px, FontFamilyKind::Sans),
            text: line.clone(),
            font_size_px,
            line_height_px: height,
            font_family: FontFamilyKind::Sans,
            color,
            underline: false,
            line_through: false,
            text_decoration_color: None,
            bold: scale >= 3,
            italic: false,
            text_shadow: None,
        }));

        cursor_y = cursor_y.saturating_add(height);
    }

    LayoutDocument {
        background_color: DEFAULT_BACKGROUND_COLOR,
        content_height: cursor_y,
        commands,
        links: Vec::new(),
        controls: Vec::new(),
        element_hitboxes: Vec::new(),
    }
}

#[derive(Debug, Clone)]
struct AddressBarState {
    text: String,
    cursor_chars: usize,
    selection_anchor: Option<usize>,
    focused: bool,
}

#[derive(Debug, Clone)]
struct FocusedPageInput {
    // While a page input is focused, this native editor state is the immediate
    // source of truth; sync_page_input_value keeps the DOM attribute in step.
    control_id: usize,
    node_id: Option<usize>,
    initial_value: String,
    editor: AddressBarState,
}

impl AddressBarState {
    fn new(text: String) -> Self {
        let cursor_chars = text.chars().count();
        Self {
            text,
            cursor_chars,
            selection_anchor: None,
            focused: false,
        }
    }

    fn text(&self) -> &str {
        &self.text
    }

    fn set_text(&mut self, text: String) {
        self.text = text;
        self.cursor_chars = self.text.chars().count();
        self.selection_anchor = None;
    }

    fn char_len(&self) -> usize {
        self.text.chars().count()
    }

    fn focus_at(&mut self, char_index: usize) {
        self.focused = true;
        self.cursor_chars = char_index.min(self.char_len());
        self.selection_anchor = None;
    }

    fn blur(&mut self) {
        self.focused = false;
        self.selection_anchor = None;
    }

    fn selection_range(&self) -> Option<(usize, usize)> {
        let anchor = self.selection_anchor?;
        if anchor == self.cursor_chars {
            return None;
        }

        Some((anchor.min(self.cursor_chars), anchor.max(self.cursor_chars)))
    }

    fn select_all(&mut self) -> bool {
        let end = self.char_len();
        let changed = self.cursor_chars != end || self.selection_range() != Some((0, end));
        self.focused = true;
        self.cursor_chars = end;
        self.selection_anchor = Some(0);
        changed
    }

    fn delete_selection(&mut self) -> bool {
        let Some((start, end)) = self.selection_range() else {
            return false;
        };

        let start_byte = self.byte_index_for_char(start);
        let end_byte = self.byte_index_for_char(end);
        self.text.replace_range(start_byte..end_byte, "");
        self.cursor_chars = start;
        self.selection_anchor = None;
        true
    }

    fn selected_text(&self) -> Option<String> {
        let (start, end) = self.selection_range()?;
        Some(self.text.chars().skip(start).take(end - start).collect())
    }

    fn cut_selection_text(&mut self) -> Option<String> {
        let text = self.selected_text()?;
        self.delete_selection();
        Some(text)
    }

    fn insert_text(&mut self, input: &str) -> bool {
        let filtered: String = input
            .chars()
            .filter(|character| !character.is_control())
            .collect();
        if filtered.is_empty() {
            return false;
        }

        self.delete_selection();
        let byte_index = self.byte_index_for_char(self.cursor_chars);
        self.text.insert_str(byte_index, &filtered);
        self.cursor_chars = self.cursor_chars.saturating_add(filtered.chars().count());
        self.selection_anchor = None;
        true
    }

    fn backspace(&mut self) -> bool {
        if self.delete_selection() {
            return true;
        }

        if self.cursor_chars == 0 {
            return false;
        }

        let end = self.byte_index_for_char(self.cursor_chars);
        let start = self.byte_index_for_char(self.cursor_chars - 1);
        self.text.replace_range(start..end, "");
        self.cursor_chars -= 1;
        self.selection_anchor = None;
        true
    }

    fn delete_forward(&mut self) -> bool {
        if self.delete_selection() {
            return true;
        }

        if self.cursor_chars >= self.char_len() {
            return false;
        }

        let start = self.byte_index_for_char(self.cursor_chars);
        let end = self.byte_index_for_char(self.cursor_chars + 1);
        self.text.replace_range(start..end, "");
        self.selection_anchor = None;
        true
    }

    fn move_left(&mut self, extend_selection: bool) -> bool {
        if self.cursor_chars == 0 {
            return false;
        }

        self.set_cursor(self.cursor_chars - 1, extend_selection)
    }

    fn move_right(&mut self, extend_selection: bool) -> bool {
        if self.cursor_chars >= self.char_len() {
            return false;
        }

        self.set_cursor(self.cursor_chars + 1, extend_selection)
    }

    fn move_word_left(&mut self, extend_selection: bool) -> bool {
        let target = self.previous_word_boundary();
        if target == self.cursor_chars {
            return false;
        }

        self.set_cursor(target, extend_selection)
    }

    fn move_word_right(&mut self, extend_selection: bool) -> bool {
        let target = self.next_word_boundary();
        if target == self.cursor_chars {
            return false;
        }

        self.set_cursor(target, extend_selection)
    }

    fn move_home(&mut self, extend_selection: bool) -> bool {
        if self.cursor_chars == 0 {
            return false;
        }
        self.set_cursor(0, extend_selection)
    }

    fn move_end(&mut self, extend_selection: bool) -> bool {
        let end = self.char_len();
        if self.cursor_chars == end {
            return false;
        }
        self.set_cursor(end, extend_selection)
    }

    fn delete_word_backward(&mut self) -> bool {
        if self.delete_selection() {
            return true;
        }
        let target = self.previous_word_boundary();
        if target == self.cursor_chars {
            return false;
        }

        let start = self.byte_index_for_char(target);
        let end = self.byte_index_for_char(self.cursor_chars);
        self.text.replace_range(start..end, "");
        self.cursor_chars = target;
        self.selection_anchor = None;
        true
    }

    fn delete_word_forward(&mut self) -> bool {
        if self.delete_selection() {
            return true;
        }
        let target = self.next_word_boundary();
        if target == self.cursor_chars {
            return false;
        }

        let start = self.byte_index_for_char(self.cursor_chars);
        let end = self.byte_index_for_char(target);
        self.text.replace_range(start..end, "");
        self.selection_anchor = None;
        true
    }

    fn byte_index_for_char(&self, char_index: usize) -> usize {
        if char_index == 0 {
            return 0;
        }

        self.text
            .char_indices()
            .nth(char_index)
            .map(|(index, _)| index)
            .unwrap_or(self.text.len())
    }

    fn set_cursor(&mut self, target: usize, extend_selection: bool) -> bool {
        let target = target.min(self.char_len());
        let previous_cursor = self.cursor_chars;
        let previous_anchor = self.selection_anchor;

        if extend_selection {
            self.selection_anchor.get_or_insert(previous_cursor);
        } else {
            self.selection_anchor = None;
        }

        self.cursor_chars = target;
        if self.selection_anchor == Some(self.cursor_chars) {
            self.selection_anchor = None;
        }

        previous_cursor != self.cursor_chars || previous_anchor != self.selection_anchor
    }

    fn previous_word_boundary(&self) -> usize {
        if self.cursor_chars == 0 {
            return 0;
        }

        let characters: Vec<char> = self.text.chars().collect();
        let mut index = self.cursor_chars.min(characters.len());
        while index > 0 && characters[index - 1].is_whitespace() {
            index -= 1;
        }
        while index > 0 && !characters[index - 1].is_whitespace() {
            index -= 1;
        }
        index
    }

    fn next_word_boundary(&self) -> usize {
        let characters: Vec<char> = self.text.chars().collect();
        let mut index = self.cursor_chars.min(characters.len());
        while index < characters.len() && characters[index].is_whitespace() {
            index += 1;
        }
        while index < characters.len() && !characters[index].is_whitespace() {
            index += 1;
        }
        index
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HitTarget {
    None,
    TitleBar,
    AddressBar,
    PageTextInput(usize),
    PageButton(usize),
    Button(ChromeButton),
    Resize(ResizeDirection),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChromeButton {
    Back,
    Forward,
    Reload,
    Navigate,
    Minimize,
    ToggleMaximize,
    Close,
}

#[derive(Debug, Clone, Copy)]
struct Rect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl Rect {
    fn right(&self) -> u32 {
        self.x.saturating_add(self.width)
    }

    fn contains(&self, position: PhysicalPosition<f64>) -> bool {
        position.x >= self.x as f64
            && position.y >= self.y as f64
            && position.x < self.right() as f64
            && position.y < self.y.saturating_add(self.height) as f64
    }
}

#[derive(Debug, Clone, Copy)]
struct ChromeLayoutMetrics {
    height: u32,
    title_bar: Rect,
    drag_region: Rect,
    back_button: Rect,
    forward_button: Rect,
    reload_button: Rect,
    address_bar: Rect,
    go_button: Rect,
    minimize_button: Rect,
    maximize_button: Rect,
    close_button: Rect,
    info_y: u32,
}

#[derive(Debug, Clone)]
struct AddressBarView {
    text: String,
    start_char: usize,
    end_char: usize,
    caret_x: u32,
    selection_start_x: Option<u32>,
    selection_end_x: Option<u32>,
}

fn chrome_layout_metrics(fonts: &mut FontContext, window_width: u32) -> ChromeLayoutMetrics {
    let info_height = fonts.line_height_px(INFO_FONT_SIZE, FontFamilyKind::Sans);
    let title_y = CHROME_TOP_PADDING;
    let button_y = title_y.saturating_add(TITLE_BAR_HEIGHT.saturating_sub(BUTTON_HEIGHT) / 2);
    let right_edge = window_width.saturating_sub(FRAME_PADDING);
    let close_button = Rect {
        x: right_edge.saturating_sub(BUTTON_WIDTH),
        y: button_y,
        width: BUTTON_WIDTH,
        height: BUTTON_HEIGHT,
    };
    let maximize_button = Rect {
        x: close_button.x.saturating_sub(BUTTON_GAP + BUTTON_WIDTH),
        y: button_y,
        width: BUTTON_WIDTH,
        height: BUTTON_HEIGHT,
    };
    let minimize_button = Rect {
        x: maximize_button.x.saturating_sub(BUTTON_GAP + BUTTON_WIDTH),
        y: button_y,
        width: BUTTON_WIDTH,
        height: BUTTON_HEIGHT,
    };

    let address_y = title_y.saturating_add(TITLE_BAR_HEIGHT + CHROME_ROW_GAP);
    let back_button = Rect {
        x: FRAME_PADDING,
        y: address_y,
        width: TOOL_BUTTON_WIDTH,
        height: ADDRESS_BAR_HEIGHT,
    };
    let forward_button = Rect {
        x: back_button.right().saturating_add(BUTTON_GAP),
        y: address_y,
        width: TOOL_BUTTON_WIDTH,
        height: ADDRESS_BAR_HEIGHT,
    };
    let reload_button = Rect {
        x: forward_button.right().saturating_add(BUTTON_GAP),
        y: address_y,
        width: TOOL_BUTTON_WIDTH,
        height: ADDRESS_BAR_HEIGHT,
    };
    let go_button = Rect {
        x: right_edge.saturating_sub(TOOL_BUTTON_WIDTH),
        y: address_y,
        width: TOOL_BUTTON_WIDTH,
        height: ADDRESS_BAR_HEIGHT,
    };
    let address_bar = Rect {
        x: reload_button.right().saturating_add(CHROME_ROW_GAP),
        y: address_y,
        width: go_button
            .x
            .saturating_sub(reload_button.right().saturating_add(CHROME_ROW_GAP * 2)),
        height: ADDRESS_BAR_HEIGHT,
    };

    let info_y = address_bar.y.saturating_add(address_bar.height + 6);
    let height = info_y.saturating_add(info_height + CHROME_BOTTOM_PADDING + HEADER_BORDER_HEIGHT);

    ChromeLayoutMetrics {
        height,
        title_bar: Rect {
            x: FRAME_PADDING,
            y: title_y,
            width: window_width.saturating_sub(FRAME_PADDING * 2),
            height: TITLE_BAR_HEIGHT,
        },
        drag_region: Rect {
            x: FRAME_PADDING,
            y: title_y,
            width: minimize_button.x.saturating_sub(FRAME_PADDING + 8),
            height: TITLE_BAR_HEIGHT,
        },
        back_button,
        forward_button,
        reload_button,
        address_bar,
        go_button,
        minimize_button,
        maximize_button,
        close_button,
        info_y,
    }
}

fn text_editor_view(
    state: &AddressBarState,
    fonts: &mut FontContext,
    available_width: u32,
    font_size: u32,
    font_family: FontFamilyKind,
) -> AddressBarView {
    let characters: Vec<char> = state.text.chars().collect();
    let cursor = state.cursor_chars.min(characters.len());
    if characters.is_empty() || available_width == 0 {
        return AddressBarView {
            text: String::new(),
            start_char: 0,
            end_char: 0,
            caret_x: 0,
            selection_start_x: None,
            selection_end_x: None,
        };
    }

    let mut start = cursor;
    let mut end = cursor;
    let mut width: u32 = 0;

    while end < characters.len() {
        let advance = fonts.glyph_advance_px(characters[end], font_size, font_family);
        if width.saturating_add(advance) > available_width && end > start {
            break;
        }
        width = width.saturating_add(advance);
        end += 1;
        if width >= available_width {
            break;
        }
    }

    while start > 0 {
        let advance = fonts.glyph_advance_px(characters[start - 1], font_size, font_family);
        if width.saturating_add(advance) > available_width && end > start {
            break;
        }
        width = width.saturating_add(advance);
        start -= 1;
        if width >= available_width {
            break;
        }
    }

    if start == end && end < characters.len() {
        end += 1;
    }

    let text: String = characters[start..end].iter().collect();
    let caret_x = characters[start..cursor]
        .iter()
        .map(|character| fonts.glyph_advance_px(*character, font_size, font_family))
        .sum();

    let mut selection_start_x = None;
    let mut selection_end_x = None;
    if let Some((selection_start, selection_end)) = state.selection_range() {
        let visible_start = selection_start.max(start).min(end);
        let visible_end = selection_end.max(start).min(end);
        if visible_start < visible_end {
            selection_start_x = Some(
                characters[start..visible_start]
                    .iter()
                    .map(|character| fonts.glyph_advance_px(*character, font_size, font_family))
                    .sum(),
            );
            selection_end_x = Some(
                characters[start..visible_end]
                    .iter()
                    .map(|character| fonts.glyph_advance_px(*character, font_size, font_family))
                    .sum(),
            );
        }
    }

    AddressBarView {
        text,
        start_char: start,
        end_char: end,
        caret_x,
        selection_start_x,
        selection_end_x,
    }
}

fn address_bar_view(
    state: &AddressBarState,
    fonts: &mut FontContext,
    available_width: u32,
) -> AddressBarView {
    text_editor_view(
        state,
        fonts,
        available_width,
        ADDRESS_BAR_FONT_SIZE,
        FontFamilyKind::Sans,
    )
}

fn cursor_index_for_text_x(
    state: &AddressBarState,
    fonts: &mut FontContext,
    available_width: u32,
    local_x: f64,
    font_size: u32,
    font_family: FontFamilyKind,
) -> usize {
    let view = text_editor_view(state, fonts, available_width, font_size, font_family);
    let characters: Vec<char> = view.text.chars().collect();
    let mut cursor_x = 0_u32;
    let target_x = local_x.max(0.0) as u32;

    for (index, character) in characters.iter().enumerate() {
        let advance = fonts.glyph_advance_px(*character, font_size, font_family);
        let midpoint = cursor_x.saturating_add(advance / 2);
        if target_x < midpoint {
            return view.start_char + index;
        }
        cursor_x = cursor_x.saturating_add(advance);
    }

    view.end_char
}

fn cursor_index_for_address_x(
    state: &AddressBarState,
    fonts: &mut FontContext,
    available_width: u32,
    local_x: f64,
) -> usize {
    cursor_index_for_text_x(
        state,
        fonts,
        available_width,
        local_x,
        ADDRESS_BAR_FONT_SIZE,
        FontFamilyKind::Sans,
    )
}

fn resolve_content_href(href: &str, base: Option<&Url>) -> String {
    if href.starts_with("http://") || href.starts_with("https://") {
        return href.to_string();
    }
    if let Some(base) = base {
        if let Ok(resolved) = base.resolve(href) {
            return resolved.to_string();
        }
    }
    href.to_string()
}

fn build_get_form_submission_url(
    action_url: &str,
    fields: &[(String, String)],
    replace_existing_query: bool,
) -> Option<String> {
    if action_url.trim().is_empty() {
        return None;
    }

    let (without_fragment, fragment_suffix) = if replace_existing_query {
        (
            action_url
                .split_once('#')
                .map(|(head, _)| head)
                .unwrap_or(action_url),
            String::new(),
        )
    } else {
        action_url
            .split_once('#')
            .map(|(head, fragment)| (head, format!("#{fragment}")))
            .unwrap_or((action_url, String::new()))
    };

    let (base, existing_query) = without_fragment
        .split_once('?')
        .map(|(head, query)| (head, Some(query)))
        .unwrap_or((without_fragment, None));

    let mut query = if replace_existing_query {
        String::new()
    } else {
        existing_query.unwrap_or_default().to_string()
    };
    let mut needs_separator = !query.is_empty();
    for (name, value) in fields {
        if name.is_empty() {
            continue;
        }
        if needs_separator {
            query.push('&');
        }
        query.push_str(&percent_encode_form_component(name));
        query.push('=');
        query.push_str(&percent_encode_form_component(value));
        needs_separator = true;
    }

    let mut final_url = base.to_string();
    if !query.is_empty() {
        final_url.push('?');
        final_url.push_str(&query);
    }
    final_url.push_str(&fragment_suffix);
    Some(final_url)
}

fn percent_encode_form_component(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char)
            }
            b' ' => encoded.push('+'),
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

fn keyboard_dom_event_request(
    node_id: usize,
    event_type: &str,
    key_code: KeyCode,
    repeat: bool,
    modifiers: &ModifiersState,
) -> DomEventRequest {
    DomEventRequest {
        target_node_id: node_id,
        event_type: event_type.to_string(),
        bubbles: true,
        cancelable: matches!(event_type, "keydown"),
        key: Some(dom_key_value_for_key_code(key_code, modifiers.shift_key())),
        code: Some(dom_code_value_for_key_code(key_code)),
        repeat,
        alt_key: modifiers.alt_key(),
        ctrl_key: modifiers.control_key(),
        shift_key: modifiers.shift_key(),
        meta_key: modifiers.super_key(),
        ..Default::default()
    }
}

fn dom_key_value_for_key_code(key_code: KeyCode, shift: bool) -> String {
    match key_code {
        KeyCode::KeyA => if shift { "A" } else { "a" }.to_string(),
        KeyCode::KeyB => if shift { "B" } else { "b" }.to_string(),
        KeyCode::KeyC => if shift { "C" } else { "c" }.to_string(),
        KeyCode::KeyD => if shift { "D" } else { "d" }.to_string(),
        KeyCode::KeyE => if shift { "E" } else { "e" }.to_string(),
        KeyCode::KeyF => if shift { "F" } else { "f" }.to_string(),
        KeyCode::KeyG => if shift { "G" } else { "g" }.to_string(),
        KeyCode::KeyH => if shift { "H" } else { "h" }.to_string(),
        KeyCode::KeyI => if shift { "I" } else { "i" }.to_string(),
        KeyCode::KeyJ => if shift { "J" } else { "j" }.to_string(),
        KeyCode::KeyK => if shift { "K" } else { "k" }.to_string(),
        KeyCode::KeyL => if shift { "L" } else { "l" }.to_string(),
        KeyCode::KeyM => if shift { "M" } else { "m" }.to_string(),
        KeyCode::KeyN => if shift { "N" } else { "n" }.to_string(),
        KeyCode::KeyO => if shift { "O" } else { "o" }.to_string(),
        KeyCode::KeyP => if shift { "P" } else { "p" }.to_string(),
        KeyCode::KeyQ => if shift { "Q" } else { "q" }.to_string(),
        KeyCode::KeyR => if shift { "R" } else { "r" }.to_string(),
        KeyCode::KeyS => if shift { "S" } else { "s" }.to_string(),
        KeyCode::KeyT => if shift { "T" } else { "t" }.to_string(),
        KeyCode::KeyU => if shift { "U" } else { "u" }.to_string(),
        KeyCode::KeyV => if shift { "V" } else { "v" }.to_string(),
        KeyCode::KeyW => if shift { "W" } else { "w" }.to_string(),
        KeyCode::KeyX => if shift { "X" } else { "x" }.to_string(),
        KeyCode::KeyY => if shift { "Y" } else { "y" }.to_string(),
        KeyCode::KeyZ => if shift { "Z" } else { "z" }.to_string(),
        KeyCode::Digit1 => if shift { "!" } else { "1" }.to_string(),
        KeyCode::Digit2 => if shift { "@" } else { "2" }.to_string(),
        KeyCode::Digit3 => if shift { "#" } else { "3" }.to_string(),
        KeyCode::Digit4 => if shift { "$" } else { "4" }.to_string(),
        KeyCode::Digit5 => if shift { "%" } else { "5" }.to_string(),
        KeyCode::Digit6 => if shift { "^" } else { "6" }.to_string(),
        KeyCode::Digit7 => if shift { "&" } else { "7" }.to_string(),
        KeyCode::Digit8 => if shift { "*" } else { "8" }.to_string(),
        KeyCode::Digit9 => if shift { "(" } else { "9" }.to_string(),
        KeyCode::Digit0 => if shift { ")" } else { "0" }.to_string(),
        KeyCode::Minus => if shift { "_" } else { "-" }.to_string(),
        KeyCode::Equal => if shift { "+" } else { "=" }.to_string(),
        KeyCode::BracketLeft => if shift { "{" } else { "[" }.to_string(),
        KeyCode::BracketRight => if shift { "}" } else { "]" }.to_string(),
        KeyCode::Semicolon => if shift { ":" } else { ";" }.to_string(),
        KeyCode::Quote => if shift { "\"" } else { "'" }.to_string(),
        KeyCode::Comma => if shift { "<" } else { "," }.to_string(),
        KeyCode::Period => if shift { ">" } else { "." }.to_string(),
        KeyCode::Slash => if shift { "?" } else { "/" }.to_string(),
        KeyCode::Backquote => if shift { "~" } else { "`" }.to_string(),
        KeyCode::Backslash => if shift { "|" } else { "\\" }.to_string(),
        KeyCode::Space => " ".to_string(),
        KeyCode::Enter | KeyCode::NumpadEnter => "Enter".to_string(),
        KeyCode::Tab => "Tab".to_string(),
        KeyCode::Escape => "Escape".to_string(),
        KeyCode::Backspace => "Backspace".to_string(),
        KeyCode::Delete => "Delete".to_string(),
        KeyCode::ArrowLeft => "ArrowLeft".to_string(),
        KeyCode::ArrowRight => "ArrowRight".to_string(),
        KeyCode::ArrowUp => "ArrowUp".to_string(),
        KeyCode::ArrowDown => "ArrowDown".to_string(),
        KeyCode::Home => "Home".to_string(),
        KeyCode::End => "End".to_string(),
        KeyCode::PageUp => "PageUp".to_string(),
        KeyCode::PageDown => "PageDown".to_string(),
        other => format!("{other:?}"),
    }
}

fn dom_code_value_for_key_code(key_code: KeyCode) -> String {
    format!("{key_code:?}")
}

fn parse_address_input(input: &str) -> Result<Url> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(BrowserError::message("address bar is empty"));
    }

    if trimmed.contains("://") {
        return Url::parse(trimmed);
    }

    let scheme = if looks_like_local_address(trimmed) {
        "http"
    } else {
        "https"
    };
    Url::parse(&format!("{scheme}://{trimmed}"))
}

fn looks_like_local_address(input: &str) -> bool {
    let host = input
        .split('/')
        .next()
        .unwrap_or(input)
        .split('?')
        .next()
        .unwrap_or(input);

    host.eq_ignore_ascii_case("localhost")
        || host.starts_with("localhost:")
        || host.starts_with("127.")
        || host.starts_with("0.0.0.0")
        || host.starts_with("[::1]")
}

fn resize_direction_at(
    window_size: PhysicalSize<u32>,
    position: PhysicalPosition<f64>,
) -> Option<ResizeDirection> {
    let left = position.x <= RESIZE_BORDER as f64;
    let right = position.x >= window_size.width.saturating_sub(RESIZE_BORDER) as f64;
    let top = position.y <= RESIZE_BORDER as f64;
    let bottom = position.y >= window_size.height.saturating_sub(RESIZE_BORDER) as f64;

    match (left, right, top, bottom) {
        (true, _, true, _) => Some(ResizeDirection::NorthWest),
        (_, true, true, _) => Some(ResizeDirection::NorthEast),
        (true, _, _, true) => Some(ResizeDirection::SouthWest),
        (_, true, _, true) => Some(ResizeDirection::SouthEast),
        (true, _, _, _) => Some(ResizeDirection::West),
        (_, true, _, _) => Some(ResizeDirection::East),
        (_, _, true, _) => Some(ResizeDirection::North),
        (_, _, _, true) => Some(ResizeDirection::South),
        _ => None,
    }
}

fn cursor_icon_for_target(target: HitTarget) -> CursorIcon {
    match target {
        HitTarget::AddressBar => CursorIcon::Text,
        HitTarget::PageTextInput(_) => CursorIcon::Text,
        HitTarget::PageButton(_) => CursorIcon::Pointer,
        HitTarget::Button(_) => CursorIcon::Pointer,
        HitTarget::Resize(direction) => direction.into(),
        HitTarget::TitleBar | HitTarget::None => CursorIcon::Default,
    }
}

fn paint_background(
    buffer: &mut [u32],
    width: u32,
    height: u32,
    chrome_height: u32,
    content_background: Color,
) {
    draw_rect(
        buffer,
        width,
        height,
        0,
        0,
        width,
        height,
        COLOR_WINDOW_BACKGROUND,
    );

    let panel_y = chrome_height.saturating_sub(HEADER_BORDER_HEIGHT / 2);
    draw_rect(
        buffer,
        width,
        height,
        FRAME_PADDING / 2,
        panel_y,
        width.saturating_sub(FRAME_PADDING),
        height
            .saturating_sub(panel_y)
            .saturating_sub(FRAME_PADDING / 2),
        content_background,
    );
    draw_rect_outline(
        buffer,
        width,
        height,
        FRAME_PADDING / 2,
        panel_y,
        width.saturating_sub(FRAME_PADDING),
        height
            .saturating_sub(panel_y)
            .saturating_sub(FRAME_PADDING / 2),
        COLOR_PANEL_BORDER,
    );
    draw_rect_outline(buffer, width, height, 0, 0, width, height, COLOR_HEADER);
}

fn paint_chrome(
    fonts: &mut FontContext,
    buffer: &mut [u32],
    width: u32,
    height: u32,
    chrome: &ChromeLayoutMetrics,
    document: &DocumentView,
    address_bar: &AddressBarState,
    hovered_target: HitTarget,
    can_go_back: bool,
    can_go_forward: bool,
    scroll_y: u32,
    max_scroll_y: u32,
) {
    draw_rect(
        buffer,
        width,
        height,
        0,
        0,
        width,
        chrome.height,
        COLOR_HEADER,
    );
    draw_rect(
        buffer,
        width,
        height,
        0,
        chrome
            .title_bar
            .y
            .saturating_add(chrome.title_bar.height + 4),
        width,
        ADDRESS_BAR_HEIGHT + 8,
        COLOR_HEADER_ROW,
    );
    draw_rect(
        buffer,
        width,
        height,
        0,
        chrome.height.saturating_sub(HEADER_BORDER_HEIGHT),
        width,
        HEADER_BORDER_HEIGHT,
        COLOR_ACCENT,
    );

    let title_y = chrome.title_bar.y.saturating_add(
        chrome
            .title_bar
            .height
            .saturating_sub(fonts.line_height_px(APP_FONT_SIZE, FontFamilyKind::Sans))
            / 2,
    );
    fonts.draw_text(
        buffer,
        width,
        height,
        chrome.title_bar.x,
        title_y,
        "TOBIRA",
        APP_FONT_SIZE,
        COLOR_HEADER_TEXT,
        true,
        false,
        false,
        FontFamilyKind::Sans,
    );

    let app_width = fonts.text_width_px("TOBIRA", APP_FONT_SIZE, FontFamilyKind::Sans);
    let page_title_x = chrome
        .title_bar
        .x
        .saturating_add(app_width + TITLE_META_GAP);
    let page_title_max_width = chrome
        .minimize_button
        .x
        .saturating_sub(page_title_x.saturating_add(12));
    let page_title = fit_text_to_width(
        fonts,
        &document.title,
        page_title_max_width,
        TITLE_FONT_SIZE,
        FontFamilyKind::Sans,
    );
    if !page_title.is_empty() {
        let page_title_y = chrome.title_bar.y.saturating_add(
            chrome
                .title_bar
                .height
                .saturating_sub(fonts.line_height_px(TITLE_FONT_SIZE, FontFamilyKind::Sans))
                / 2,
        );
        fonts.draw_text(
            buffer,
            width,
            height,
            page_title_x,
            page_title_y,
            &page_title,
            TITLE_FONT_SIZE,
            COLOR_HEADER_MUTED,
            false,
            false,
            false,
            FontFamilyKind::Sans,
        );
    }

    paint_button(
        fonts,
        buffer,
        width,
        height,
        chrome.back_button,
        "←",
        hovered_target == HitTarget::Button(ChromeButton::Back),
        false,
        can_go_back,
    );
    paint_button(
        fonts,
        buffer,
        width,
        height,
        chrome.forward_button,
        "→",
        hovered_target == HitTarget::Button(ChromeButton::Forward),
        false,
        can_go_forward,
    );
    paint_button(
        fonts,
        buffer,
        width,
        height,
        chrome.reload_button,
        "R",
        hovered_target == HitTarget::Button(ChromeButton::Reload),
        false,
        true,
    );
    paint_button(
        fonts,
        buffer,
        width,
        height,
        chrome.go_button,
        "GO",
        hovered_target == HitTarget::Button(ChromeButton::Navigate),
        false,
        true,
    );
    paint_button(
        fonts,
        buffer,
        width,
        height,
        chrome.minimize_button,
        "-",
        hovered_target == HitTarget::Button(ChromeButton::Minimize),
        false,
        true,
    );
    paint_button(
        fonts,
        buffer,
        width,
        height,
        chrome.maximize_button,
        "[]",
        hovered_target == HitTarget::Button(ChromeButton::ToggleMaximize),
        false,
        true,
    );
    paint_button(
        fonts,
        buffer,
        width,
        height,
        chrome.close_button,
        "X",
        hovered_target == HitTarget::Button(ChromeButton::Close),
        true,
        true,
    );

    draw_rect(
        buffer,
        width,
        height,
        chrome.address_bar.x,
        chrome.address_bar.y,
        chrome.address_bar.width,
        chrome.address_bar.height,
        COLOR_ADDRESS_BAR,
    );
    draw_rect_outline(
        buffer,
        width,
        height,
        chrome.address_bar.x,
        chrome.address_bar.y,
        chrome.address_bar.width,
        chrome.address_bar.height,
        if address_bar.focused {
            COLOR_ADDRESS_BAR_FOCUS
        } else {
            COLOR_ADDRESS_BAR_BORDER
        },
    );

    let address_view = address_bar_view(
        address_bar,
        fonts,
        chrome
            .address_bar
            .width
            .saturating_sub(ADDRESS_BAR_PADDING_X * 2),
    );
    let address_text_y = chrome.address_bar.y.saturating_add(
        chrome
            .address_bar
            .height
            .saturating_sub(fonts.line_height_px(ADDRESS_BAR_FONT_SIZE, FontFamilyKind::Sans))
            / 2,
    );
    fonts.draw_text(
        buffer,
        width,
        height,
        chrome.address_bar.x.saturating_add(ADDRESS_BAR_PADDING_X),
        address_text_y,
        &address_view.text,
        ADDRESS_BAR_FONT_SIZE,
        COLOR_ADDRESS_BAR_TEXT,
        false,
        false,
        false,
        FontFamilyKind::Sans,
    );
    if address_bar.focused {
        if let (Some(selection_start_x), Some(selection_end_x)) =
            (address_view.selection_start_x, address_view.selection_end_x)
        {
            let selection_x = chrome
                .address_bar
                .x
                .saturating_add(ADDRESS_BAR_PADDING_X)
                .saturating_add(selection_start_x);
            let selection_width = selection_end_x.saturating_sub(selection_start_x).max(1);
            draw_rect(
                buffer,
                width,
                height,
                selection_x,
                chrome.address_bar.y.saturating_add(6),
                selection_width,
                chrome.address_bar.height.saturating_sub(12).max(1),
                COLOR_ADDRESS_BAR_SELECTION,
            );
            fonts.draw_text(
                buffer,
                width,
                height,
                chrome.address_bar.x.saturating_add(ADDRESS_BAR_PADDING_X),
                address_text_y,
                &address_view.text,
                ADDRESS_BAR_FONT_SIZE,
                COLOR_ADDRESS_BAR_TEXT,
                false,
                false,
                false,
                FontFamilyKind::Sans,
            );
        }

        let caret_height = chrome.address_bar.height.saturating_sub(14);
        let caret_y = chrome.address_bar.y.saturating_add(7);
        let caret_x = chrome
            .address_bar
            .x
            .saturating_add(ADDRESS_BAR_PADDING_X)
            .saturating_add(address_view.caret_x);
        draw_rect(
            buffer,
            width,
            height,
            caret_x,
            caret_y,
            1,
            caret_height.max(1),
            COLOR_ADDRESS_BAR_TEXT,
        );
    }

    let meta_right = format!(
        "Enter go | Ctrl+L focus | Ctrl+A/C/X/V edit | scroll: {} / {} px",
        scroll_y, max_scroll_y
    );
    let meta_right_width = fonts.text_width_px(&meta_right, INFO_FONT_SIZE, FontFamilyKind::Sans);
    let meta_right_x = width
        .saturating_sub(FRAME_PADDING)
        .saturating_sub(meta_right_width);
    fonts.draw_text(
        buffer,
        width,
        height,
        meta_right_x,
        chrome.info_y,
        &meta_right,
        INFO_FONT_SIZE,
        COLOR_HEADER_MUTED,
        false,
        false,
        false,
        FontFamilyKind::Sans,
    );

    let meta_left = format!("{} | {}", document.status_line, document.subtitle);
    let meta_left_max_width = meta_right_x.saturating_sub(FRAME_PADDING.saturating_mul(2));
    let meta_left_text = fit_text_to_width(
        fonts,
        &meta_left,
        meta_left_max_width,
        INFO_FONT_SIZE,
        FontFamilyKind::Sans,
    );
    fonts.draw_text(
        buffer,
        width,
        height,
        FRAME_PADDING,
        chrome.info_y,
        &meta_left_text,
        INFO_FONT_SIZE,
        if document.is_error() {
            COLOR_ACCENT
        } else {
            COLOR_HEADER_TEXT
        },
        false,
        false,
        false,
        FontFamilyKind::Sans,
    );
}

fn fit_text_to_width(
    fonts: &mut FontContext,
    text: &str,
    max_width: u32,
    font_size_px: u32,
    font_family: FontFamilyKind,
) -> String {
    if max_width == 0 {
        return String::new();
    }

    if fonts.text_width_px(text, font_size_px, font_family) <= max_width {
        return text.to_string();
    }

    let ellipsis = "...";
    let ellipsis_width = fonts.text_width_px(ellipsis, font_size_px, font_family);
    if ellipsis_width >= max_width {
        return ellipsis.to_string();
    }

    let mut fitted = String::new();
    let mut current_width: u32 = 0;
    for character in text.chars() {
        let advance = fonts.glyph_advance_px(character, font_size_px, font_family);
        if current_width
            .saturating_add(advance)
            .saturating_add(ellipsis_width)
            > max_width
        {
            break;
        }
        fitted.push(character);
        current_width = current_width.saturating_add(advance);
    }

    fitted.push_str(ellipsis);
    fitted
}

fn paint_button(
    fonts: &mut FontContext,
    buffer: &mut [u32],
    width: u32,
    height: u32,
    rect: Rect,
    label: &str,
    hovered: bool,
    destructive: bool,
    enabled: bool,
) {
    let color = if !enabled {
        COLOR_HEADER_ROW
    } else if destructive {
        if hovered {
            COLOR_CLOSE_BUTTON_HOVER
        } else {
            COLOR_CLOSE_BUTTON
        }
    } else if hovered {
        COLOR_TOOL_BUTTON_HOVER
    } else {
        COLOR_TOOL_BUTTON
    };

    draw_rect(
        buffer,
        width,
        height,
        rect.x,
        rect.y,
        rect.width,
        rect.height,
        color,
    );
    draw_rect_outline(
        buffer,
        width,
        height,
        rect.x,
        rect.y,
        rect.width,
        rect.height,
        COLOR_ADDRESS_BAR_BORDER,
    );

    let font_size = if label.len() > 1 { 14 } else { 18 };
    let text_width = fonts.text_width_px(label, font_size, FontFamilyKind::Sans);
    let text_height = fonts.line_height_px(font_size, FontFamilyKind::Sans);
    let text_x = rect
        .x
        .saturating_add(rect.width.saturating_sub(text_width) / 2);
    let text_y = rect
        .y
        .saturating_add(rect.height.saturating_sub(text_height) / 2);
    fonts.draw_text(
        buffer,
        width,
        height,
        text_x,
        text_y,
        label,
        font_size,
        if enabled {
            COLOR_HEADER_TEXT
        } else {
            COLOR_HEADER_MUTED
        },
        true,
        false,
        false,
        FontFamilyKind::Sans,
    );
}

fn paint_page_control(
    fonts: &mut FontContext,
    buffer: &mut [u32],
    width: u32,
    height: u32,
    offset_x: u32,
    offset_y: u32,
    scroll_y: u32,
    control: &FormControlCommand,
    focused_page_input: Option<&FocusedPageInput>,
    hovered_target: HitTarget,
) {
    let absolute_x = offset_x.saturating_add(control.x);
    let absolute_y = offset_y.saturating_add(control.y.saturating_sub(scroll_y));
    let is_hovered = matches!(
        hovered_target,
        HitTarget::PageTextInput(id) | HitTarget::PageButton(id) if id == control.id
    );
    let focused = focused_page_input
        .as_ref()
        .filter(|focused| focused.control_id == control.id)
        .copied();

    if matches!(control.kind, FormControlKind::Hidden) {
        return;
    }

    let background =
        if matches!(control.kind, FormControlKind::Button) && is_hovered && !control.disabled {
            COLOR_CONTROL_BUTTON_HOVER
        } else {
            control.background_color
        };
    let border = if focused.is_some() {
        COLOR_ADDRESS_BAR_FOCUS
    } else if is_hovered {
        COLOR_ADDRESS_BAR_BORDER
    } else {
        control.border_color
    };

    draw_rect(
        buffer,
        width,
        height,
        absolute_x,
        absolute_y,
        control.width,
        control.height,
        background,
    );
    draw_rect_outline(
        buffer,
        width,
        height,
        absolute_x,
        absolute_y,
        control.width,
        control.height,
        border,
    );

    match control.kind {
        FormControlKind::TextInput => {
            let line_height = fonts.line_height_px(control.font_size_px, control.font_family);
            let text_y = absolute_y.saturating_add(control.height.saturating_sub(line_height) / 2);
            let available_width = control.width.saturating_sub(CONTROL_PADDING_X * 2);

            let actual_value =
                resolve_text_input_value(focused.map(|focused| &focused.editor), &control.value);
            let display_value = if control.masked {
                "*".repeat(actual_value.chars().count())
            } else {
                actual_value.clone()
            };
            let show_placeholder = actual_value.is_empty()
                && control
                    .placeholder
                    .as_deref()
                    .map(|text| !text.is_empty())
                    .unwrap_or(false);

            if show_placeholder {
                let placeholder = fit_text_to_width(
                    fonts,
                    control.placeholder.as_deref().unwrap_or_default(),
                    available_width,
                    control.font_size_px,
                    control.font_family,
                );
                let placeholder_color = control
                    .placeholder_color
                    .unwrap_or(COLOR_CONTROL_PLACEHOLDER);
                fonts.draw_text(
                    buffer,
                    width,
                    height,
                    absolute_x.saturating_add(CONTROL_PADDING_X),
                    text_y,
                    &placeholder,
                    control.font_size_px,
                    placeholder_color,
                    false,
                    control.placeholder_italic,
                    false,
                    control.font_family,
                );
            } else {
                let mut editor = focused
                    .map(|focused| focused.editor.clone())
                    .unwrap_or_else(|| AddressBarState::new(actual_value));
                if focused.is_none() {
                    editor.focus_at(0);
                    editor.blur();
                }
                let view = text_editor_view(
                    &editor,
                    fonts,
                    available_width,
                    control.font_size_px,
                    control.font_family,
                );
                if focused.is_some()
                    && let (Some(selection_start_x), Some(selection_end_x)) =
                        (view.selection_start_x, view.selection_end_x)
                {
                    let selection_x = absolute_x
                        .saturating_add(CONTROL_PADDING_X)
                        .saturating_add(selection_start_x);
                    let selection_width = selection_end_x.saturating_sub(selection_start_x).max(1);
                    let selection_color =
                        control.selection_bg.unwrap_or(COLOR_CONTROL_SELECTION);
                    draw_rect(
                        buffer,
                        width,
                        height,
                        selection_x,
                        absolute_y.saturating_add(CONTROL_PADDING_Y),
                        selection_width,
                        control
                            .height
                            .saturating_sub(CONTROL_PADDING_Y.saturating_mul(2))
                            .max(1),
                        selection_color,
                    );
                }
                fonts.draw_text(
                    buffer,
                    width,
                    height,
                    absolute_x.saturating_add(CONTROL_PADDING_X),
                    text_y,
                    if control.masked {
                        &display_value
                    } else {
                        &view.text
                    },
                    control.font_size_px,
                    control.text_color,
                    false,
                    false,
                    false,
                    control.font_family,
                );
                if focused.is_some() {
                    let caret_x = absolute_x
                        .saturating_add(CONTROL_PADDING_X)
                        .saturating_add(view.caret_x);
                    draw_rect(
                        buffer,
                        width,
                        height,
                        caret_x,
                        absolute_y.saturating_add(CONTROL_PADDING_Y),
                        1,
                        control
                            .height
                            .saturating_sub(CONTROL_PADDING_Y.saturating_mul(2))
                            .max(1),
                        control.text_color,
                    );
                }
            }
        }
        FormControlKind::Button => {
            let label = if !control.label.trim().is_empty() {
                control.label.as_str()
            } else if !control.value.trim().is_empty() {
                control.value.as_str()
            } else {
                "Button"
            };
            let label = fit_text_to_width(
                fonts,
                label,
                control.width.saturating_sub(CONTROL_PADDING_X * 2),
                control.font_size_px,
                control.font_family,
            );
            let text_width = fonts.text_width_px(&label, control.font_size_px, control.font_family);
            let line_height = fonts.line_height_px(control.font_size_px, control.font_family);
            let text_x = absolute_x.saturating_add(control.width.saturating_sub(text_width) / 2);
            let text_y = absolute_y.saturating_add(control.height.saturating_sub(line_height) / 2);
            fonts.draw_text(
                buffer,
                width,
                height,
                text_x,
                text_y,
                &label,
                control.font_size_px,
                control.text_color,
                true,
                false,
                false,
                control.font_family,
            );
        }
        FormControlKind::Hidden => {}
    }
}

fn paint_layout(
    page_images: Option<&ImageStore>,
    fonts: &mut FontContext,
    buffer: &mut [u32],
    width: u32,
    height: u32,
    offset_x: u32,
    offset_y: u32,
    viewport_height: u32,
    scroll_y: u32,
    layout: &LayoutDocument,
    focused_page_input: Option<&FocusedPageInput>,
    hovered_target: HitTarget,
    scratch: &mut Vec<Vec<u32>>,
) {
    render_commands(
        buffer,
        width,
        height,
        offset_x,
        offset_y,
        viewport_height,
        scroll_y,
        &layout.commands,
        page_images,
        fonts,
        scratch,
        0,
    );

    let viewport_bottom = scroll_y.saturating_add(viewport_height);
    for control in &layout.controls {
        let control_bottom = control.y.saturating_add(control.height);
        if control_bottom < scroll_y || control.y > viewport_bottom {
            continue;
        }

        paint_page_control(
            fonts,
            buffer,
            width,
            height,
            offset_x,
            offset_y,
            scroll_y,
            control,
            focused_page_input,
            hovered_target,
        );
    }
}

fn blit_rendered_content_frame(
    buffer: &mut [u32],
    width: u32,
    _height: u32,
    offset_x: u32,
    offset_y: u32,
    viewport_height: u32,
    scroll_y: u32,
    frame: &RenderedContentFrame,
) {
    if frame.content_width == 0 || frame.content_height == 0 {
        return;
    }

    let start_row = scroll_y.min(frame.content_height);
    let visible_height = frame
        .content_height
        .saturating_sub(start_row)
        .min(viewport_height);
    let row_len = frame.content_width as usize;
    let src_stride = frame.content_width as usize;
    let dst_stride = width as usize;

    for row in 0..visible_height {
        let src_y = start_row.saturating_add(row);
        let src_start = (src_y as usize).saturating_mul(src_stride);
        let dst_y = offset_y.saturating_add(row);
        let dst_start = (dst_y as usize)
            .saturating_mul(dst_stride)
            .saturating_add(offset_x as usize);
        if src_start + row_len <= frame.pixels.len() && dst_start + row_len <= buffer.len() {
            buffer[dst_start..dst_start + row_len]
                .copy_from_slice(&frame.pixels[src_start..src_start + row_len]);
        }
    }
}

fn render_commands(
    buffer: &mut [u32],
    width: u32,
    height: u32,
    offset_x: u32,
    offset_y: u32,
    viewport_height: u32,
    scroll_y: u32,
    commands: &[DrawCommand],
    page_images: Option<&ImageStore>,
    fonts: &mut FontContext,
    scratch: &mut Vec<Vec<u32>>,
    depth: usize,
) {
    let viewport_bottom = scroll_y.saturating_add(viewport_height);

    for cmd in commands {
        match cmd {
            DrawCommand::Rect(rect) => {
                let rect_bottom = rect.y.saturating_add(rect.height);
                if rect_bottom < scroll_y || rect.y > viewport_bottom {
                    continue;
                }
                if rect.border_radius > 0 {
                    if rect.y < scroll_y {
                        // Partially above viewport: fall back to draw_rect to avoid rendering
                        // top rounded corners at the viewport edge (wrong position). The
                        // rounded-rect routine always draws corners from the top of the given
                        // rect, so clipping by reducing height produces distorted corners.
                        let clipped = scroll_y.saturating_sub(rect.y);
                        let draw_h = rect.height.saturating_sub(clipped);
                        if draw_h > 0 {
                            draw_rect(
                                buffer,
                                width,
                                height,
                                offset_x.saturating_add(rect.x),
                                offset_y, // y=0 since rect.y < scroll_y (fully at viewport top)
                                rect.width,
                                draw_h,
                                rect.color,
                            );
                        }
                    } else {
                        draw_rounded_rect(
                            buffer,
                            width,
                            height,
                            offset_x.saturating_add(rect.x),
                            offset_y.saturating_add(rect.y.saturating_sub(scroll_y)),
                            rect.width,
                            rect.height,
                            rect.border_radius,
                            rect.color,
                        );
                    }
                } else {
                    draw_rect(
                        buffer,
                        width,
                        height,
                        offset_x.saturating_add(rect.x),
                        offset_y.saturating_add(rect.y.saturating_sub(scroll_y)),
                        rect.width,
                        rect.height,
                        rect.color,
                    );
                }
            }
            DrawCommand::Text(text) => {
                let text_bottom = text
                    .y
                    .saturating_add(fonts.line_height_px(text.font_size_px, text.font_family));
                if text_bottom < scroll_y || text.y > viewport_bottom {
                    continue;
                }
                let text_draw_x = offset_x.saturating_add(text.x);
                let text_draw_y = offset_y.saturating_add(text.y.saturating_sub(scroll_y));
                // Draw text shadow first (behind main text)
                if let Some(ref shadow) = text.text_shadow {
                    let shadow_x = (text_draw_x as i64 + shadow.offset_x as i64).max(0) as u32;
                    let shadow_y = (text_draw_y as i64 + shadow.offset_y as i64).max(0) as u32;
                    if shadow.blur == 0 {
                        // Sharp shadow: draw directly into the main buffer.
                        fonts.draw_text(
                            buffer,
                            width,
                            height,
                            shadow_x,
                            shadow_y,
                            &text.text,
                            text.font_size_px,
                            shadow.color,
                            false,
                            false,
                            false,
                            text.font_family,
                        );
                    } else {
                        // Blurred shadow: render to offscreen buffer at color, box-blur it,
                        // then blit with alpha onto the main buffer. Padding the offscreen
                        // by `blur` on every side prevents the blur kernel from sampling
                        // outside the rasterized glyph region.
                        let pad = shadow.blur.min(32);
                        // Estimate width via line_height heuristics already used in layout;
                        // use the text command's bounding metrics.
                        let off_w = text.width.saturating_add(pad * 2).max(1);
                        let off_h = text.line_height_px.saturating_add(pad * 2).max(1);
                        let mut offscreen = vec![0u32; (off_w * off_h) as usize];
                        fonts.draw_text(
                            &mut offscreen,
                            off_w,
                            off_h,
                            pad,
                            pad,
                            &text.text,
                            text.font_size_px,
                            shadow.color,
                            false,
                            false,
                            false,
                            text.font_family,
                        );
                        apply_box_blur(&mut offscreen, off_w, off_h, shadow.blur);
                        // Blit non-zero pixels onto the main buffer with alpha derived
                        // from the offscreen pixel brightness.
                        let dest_x = shadow_x.saturating_sub(pad);
                        let dest_y = shadow_y.saturating_sub(pad);
                        let shadow_r = ((shadow.color >> 16) & 0xFF) as u32;
                        let shadow_g = ((shadow.color >> 8) & 0xFF) as u32;
                        let shadow_b = (shadow.color & 0xFF) as u32;
                        for oy in 0..off_h {
                            let dy = dest_y.saturating_add(oy);
                            if dy >= height {
                                break;
                            }
                            for ox in 0..off_w {
                                let dx = dest_x.saturating_add(ox);
                                if dx >= width {
                                    break;
                                }
                                let src = offscreen[(oy * off_w + ox) as usize];
                                if src == 0 {
                                    continue;
                                }
                                // Approximate alpha from rendered intensity vs shadow color.
                                let sr = ((src >> 16) & 0xFF) as u32;
                                let sg = ((src >> 8) & 0xFF) as u32;
                                let sb = (src & 0xFF) as u32;
                                let denom =
                                    shadow_r.max(shadow_g).max(shadow_b).max(1);
                                let a = (sr.max(sg).max(sb) * 255 / denom).min(255);
                                if a == 0 {
                                    continue;
                                }
                                let dst_idx = (dy * width + dx) as usize;
                                if dst_idx >= buffer.len() {
                                    continue;
                                }
                                let bg = buffer[dst_idx];
                                let br = (bg >> 16) & 0xFF;
                                let bgg = (bg >> 8) & 0xFF;
                                let bb = bg & 0xFF;
                                let out_r = (shadow_r * a + br * (255 - a)) / 255;
                                let out_g = (shadow_g * a + bgg * (255 - a)) / 255;
                                let out_b = (shadow_b * a + bb * (255 - a)) / 255;
                                buffer[dst_idx] = (out_r << 16) | (out_g << 8) | out_b;
                            }
                        }
                    }
                }
                fonts.draw_text(
                    buffer,
                    width,
                    height,
                    text_draw_x,
                    text_draw_y,
                    &text.text,
                    text.font_size_px,
                    text.color,
                    text.bold,
                    text.underline,
                    text.line_through,
                    text.font_family,
                );
            }
            DrawCommand::Image(image) => {
                let image_bottom = image.y.saturating_add(image.height);
                if image_bottom < scroll_y || image.y > viewport_bottom {
                    continue;
                }
                if let Some(images) = page_images
                    && let Some(decoded) = images.get(&image.src)
                {
                    // min_y = offset_y prevents images from bleeding into the chrome UI
                    let min_y = offset_y as i32;
                    if image.tile {
                        // Tiled background: draw at natural size, repeated across element
                        let sx = offset_x as i32 + image.x as i32;
                        let sy = offset_y as i32 + image.y as i32 - scroll_y as i32;
                        draw_tiled_image(
                            buffer,
                            width,
                            height,
                            sx,
                            sy,
                            image.width,
                            image.height,
                            decoded,
                            min_y,
                            image.tile_repeat,
                        );
                    } else {
                        let sx = offset_x as i32 + image.x as i32;
                        let sy = offset_y as i32 + image.y as i32 - scroll_y as i32;
                        draw_scaled_image(
                            buffer,
                            width,
                            height,
                            sx,
                            sy,
                            image.width,
                            image.height,
                            decoded,
                            image.object_fit,
                            image.object_position_x,
                            image.object_position_y,
                            min_y,
                        );
                    }
                }
            }
            DrawCommand::Layer(layer) => {
                render_layer(
                    buffer, width, height, offset_x, offset_y, scroll_y, layer,
                    None,
                    page_images, fonts, scratch, depth,
                );
            }
            DrawCommand::Sticky(sticky) => {
                let container_max_y = if sticky.container_bottom == u32::MAX {
                    u32::MAX
                } else {
                    sticky.container_bottom.saturating_sub(sticky.layer.height)
                };
                let effective_y = scroll_y
                    .saturating_add(sticky.sticky_top)
                    .max(sticky.normal_y)
                    .min(container_max_y);

                render_layer(
                    buffer, width, height, offset_x, offset_y, scroll_y,
                    &sticky.layer,
                    Some(effective_y),
                    page_images, fonts, scratch, depth,
                );
            }
            DrawCommand::Gradient(g) => {
                let grad_bottom = g.y.saturating_add(g.height);
                if grad_bottom < scroll_y || g.y > viewport_bottom {
                    continue;
                }

                // border_radius corner check setup
                let r = g.border_radius.min(g.width / 2).min(g.height / 2) as i64;
                let r_sq = r * r;
                let cx_left = g.x as i64 + r;
                let cx_right = (g.x + g.width) as i64 - r;
                let cy_top = g.y as i64 + r;
                let cy_bottom = (g.y + g.height) as i64 - r;

                let gx = g.x;
                let gy = g.y;
                let gw_u = g.width;
                let gh_u = g.height;
                let gw = g.width as f64;
                let gh = g.height as f64;

                // For linear gradient: precompute direction
                let (linear_dx, linear_dy, proj_len) = match &g.kind {
                    GradientKind::Linear { angle_deg_x1000 } => {
                        let angle_rad = (*angle_deg_x1000 as f64 / 1000.0_f64).to_radians();
                        let dx = angle_rad.sin();
                        let dy = angle_rad.cos();
                        let pl = (dx * gw).abs() + (dy * gh).abs();
                        let pl = if pl < 0.001 { 1.0 } else { pl };
                        (dx, dy, pl)
                    }
                    GradientKind::Radial { .. } => (0.0, 0.0, 1.0),
                };

                // For radial gradient: precompute center and max radius
                let (radial_cx, radial_cy, radial_max_r) = match &g.kind {
                    GradientKind::Radial { center_x, center_y } => {
                        let cx = *center_x as f64 / 1000.0 * gw;
                        let cy = *center_y as f64 / 1000.0 * gh;
                        let max_r = gw.hypot(gh).max(1.0);
                        (cx, cy, max_r)
                    }
                    GradientKind::Linear { .. } => (0.0, 0.0, 1.0),
                };

                let py_start = g.y.max(scroll_y);
                let py_end = grad_bottom.min(viewport_bottom);

                for py in py_start..py_end {
                    for px in gx..(gx + gw_u) {
                        // border_radius: skip corner pixels
                        if r > 0 {
                            let ipx = px as i64;
                            let ipy = py as i64;
                            let in_top_strip = ipy < cy_top;
                            let in_bot_strip = ipy >= cy_bottom;
                            let in_left_strip = ipx < cx_left;
                            let in_right_strip = ipx >= cx_right;
                            if in_top_strip && in_left_strip {
                                let ddx = cx_left - ipx;
                                let ddy = cy_top - ipy;
                                if ddx * ddx + ddy * ddy > r_sq {
                                    continue;
                                }
                            } else if in_top_strip && in_right_strip {
                                let ddx = ipx - cx_right + 1;
                                let ddy = cy_top - ipy;
                                if ddx * ddx + ddy * ddy > r_sq {
                                    continue;
                                }
                            } else if in_bot_strip && in_left_strip {
                                let ddx = cx_left - ipx;
                                let ddy = ipy - cy_bottom + 1;
                                if ddx * ddx + ddy * ddy > r_sq {
                                    continue;
                                }
                            } else if in_bot_strip && in_right_strip {
                                let ddx = ipx - cx_right + 1;
                                let ddy = ipy - cy_bottom + 1;
                                if ddx * ddx + ddy * ddy > r_sq {
                                    continue;
                                }
                            }
                        }

                        // Compute gradient t in [0,1]
                        let rel_x = px as f64 - gx as f64;
                        let rel_y = py as f64 - gy as f64;
                        let t = match &g.kind {
                            GradientKind::Linear { .. } => {
                                let dot = linear_dx * rel_x + linear_dy * rel_y;
                                (dot / proj_len).clamp(0.0, 1.0)
                            }
                            GradientKind::Radial { .. } => {
                                let dx = rel_x - radial_cx;
                                let dy = rel_y - radial_cy;
                                (dx.hypot(dy) / radial_max_r).clamp(0.0, 1.0)
                            }
                        };

                        // Interpolate color between stops
                        let stops = &g.stops;
                        let color = if stops.is_empty() {
                            0u32
                        } else if stops.len() == 1 {
                            stops[0].color
                        } else {
                            // find which two stops t falls between
                            let t_pos = (t * 1000.0) as u32;
                            let mut color = stops[0].color;
                            for i in 0..stops.len().saturating_sub(1) {
                                let s0 = &stops[i];
                                let s1 = &stops[i + 1];
                                if t_pos <= s0.position {
                                    color = s0.color;
                                    break;
                                }
                                if t_pos <= s1.position {
                                    // lerp between s0 and s1
                                    let range = s1.position.saturating_sub(s0.position);
                                    if range == 0 {
                                        color = s1.color;
                                    } else {
                                        let frac =
                                            (t_pos - s0.position) as u64 * 1000 / range as u64;
                                        let r0 = (s0.color >> 16) & 0xFF;
                                        let g0 = (s0.color >> 8) & 0xFF;
                                        let b0 = s0.color & 0xFF;
                                        let r1 = (s1.color >> 16) & 0xFF;
                                        let g1 = (s1.color >> 8) & 0xFF;
                                        let b1 = s1.color & 0xFF;
                                        let ri =
                                            (r0 as u64 * (1000 - frac) + r1 as u64 * frac) / 1000;
                                        let gi =
                                            (g0 as u64 * (1000 - frac) + g1 as u64 * frac) / 1000;
                                        let bi =
                                            (b0 as u64 * (1000 - frac) + b1 as u64 * frac) / 1000;
                                        color =
                                            ((ri as u32) << 16) | ((gi as u32) << 8) | (bi as u32);
                                    }
                                    break;
                                }
                                if i + 1 == stops.len() - 1 {
                                    color = s1.color;
                                }
                            }
                            color
                        };

                        let draw_px = offset_x.saturating_add(px);
                        let draw_py = offset_y.saturating_add(py.saturating_sub(scroll_y));
                        if draw_px < width && draw_py < height {
                            let idx = draw_py as usize * width as usize + draw_px as usize;
                            if idx < buffer.len() {
                                buffer[idx] = color;
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Apply a separable box blur to an ARGB pixel buffer in-place.
/// radius = 0 is a no-op.
fn apply_box_blur(pixels: &mut Vec<u32>, width: u32, height: u32, radius: u32) {
    if radius == 0 || width == 0 || height == 0 {
        return;
    }
    let w = width as usize;
    let h = height as usize;
    let r = radius as usize;
    let mut tmp = vec![0u32; w * h];

    // Horizontal pass: pixels → tmp
    for row in 0..h {
        let base = row * w;
        let mut sr: u32 = 0;
        let mut sg: u32 = 0;
        let mut sb: u32 = 0;
        let mut cnt: u32 = 0;
        // Seed window with pixels[0..=min(r, w-1)]
        let init_end = r.min(w.saturating_sub(1));
        for k in 0..=init_end {
            let px = pixels[base + k];
            sr += (px >> 16) & 0xFF;
            sg += (px >> 8) & 0xFF;
            sb += px & 0xFF;
            cnt += 1;
        }
        for x in 0..w {
            if cnt > 0 {
                tmp[base + x] = ((sr / cnt) << 16) | ((sg / cnt) << 8) | (sb / cnt);
            }
            // Advance: add right edge
            let add = x + r + 1;
            if add < w {
                let px = pixels[base + add];
                sr += (px >> 16) & 0xFF;
                sg += (px >> 8) & 0xFF;
                sb += px & 0xFF;
                cnt += 1;
            }
            // Remove left edge
            if x >= r {
                let rem = x - r;
                let px = pixels[base + rem];
                sr = sr.saturating_sub((px >> 16) & 0xFF);
                sg = sg.saturating_sub((px >> 8) & 0xFF);
                sb = sb.saturating_sub(px & 0xFF);
                cnt = cnt.saturating_sub(1);
            }
        }
    }

    // Vertical pass: tmp → pixels
    for col in 0..w {
        let mut sr: u32 = 0;
        let mut sg: u32 = 0;
        let mut sb: u32 = 0;
        let mut cnt: u32 = 0;
        let init_end = r.min(h.saturating_sub(1));
        for k in 0..=init_end {
            let px = tmp[k * w + col];
            sr += (px >> 16) & 0xFF;
            sg += (px >> 8) & 0xFF;
            sb += px & 0xFF;
            cnt += 1;
        }
        for y in 0..h {
            if cnt > 0 {
                pixels[y * w + col] = ((sr / cnt) << 16) | ((sg / cnt) << 8) | (sb / cnt);
            }
            let add = y + r + 1;
            if add < h {
                let px = tmp[add * w + col];
                sr += (px >> 16) & 0xFF;
                sg += (px >> 8) & 0xFF;
                sb += px & 0xFF;
                cnt += 1;
            }
            if y >= r {
                let rem = y - r;
                let px = tmp[rem * w + col];
                sr = sr.saturating_sub((px >> 16) & 0xFF);
                sg = sg.saturating_sub((px >> 8) & 0xFF);
                sb = sb.saturating_sub(px & 0xFF);
                cnt = cnt.saturating_sub(1);
            }
        }
    }
}

/// Apply brightness adjustment to an ARGB pixel buffer in-place.
/// brightness = 10000 is a no-op (100% brightness).
fn apply_brightness(pixels: &mut [u32], brightness: u32) {
    if brightness == 10000 {
        return;
    }
    for px in pixels.iter_mut() {
        let r = ((((*px >> 16) & 0xFF) * brightness / 10000).min(255)) as u32;
        let g = ((((*px >> 8) & 0xFF) * brightness / 10000).min(255)) as u32;
        let b = (((*px & 0xFF) * brightness / 10000).min(255)) as u32;
        *px = (r << 16) | (g << 8) | b;
    }
}

/// Zero out pixels outside the clip-path region. Operates in layer-local space.
fn apply_clip_path(pixels: &mut Vec<u32>, width: u32, height: u32, clip: &crate::css::ClipPath) {
    if width == 0 || height == 0 {
        return;
    }
    let w = width as i64;
    let h = height as i64;
    match clip {
        crate::css::ClipPath::Circle { radius_permille, cx_permille, cy_permille } => {
            let cx = (*cx_permille as i64) * w / 1000;
            let cy = (*cy_permille as i64) * h / 1000;
            // Radius is permille of min(width, height).
            let min_dim = w.min(h);
            let r = (*radius_permille as i64) * min_dim / 1000;
            let r2 = r * r;
            for y in 0..height {
                for x in 0..width {
                    let dx = x as i64 - cx;
                    let dy = y as i64 - cy;
                    if dx * dx + dy * dy > r2 {
                        let idx = (y * width + x) as usize;
                        if idx < pixels.len() {
                            pixels[idx] = 0;
                        }
                    }
                }
            }
        }
        crate::css::ClipPath::Inset { top, right, bottom, left } => {
            let l = (*left as i64) * w / 1000;
            let r = w - (*right as i64) * w / 1000;
            let t = (*top as i64) * h / 1000;
            let b = h - (*bottom as i64) * h / 1000;
            for y in 0..height {
                for x in 0..width {
                    let xi = x as i64;
                    let yi = y as i64;
                    if xi < l || xi >= r || yi < t || yi >= b {
                        let idx = (y * width + x) as usize;
                        if idx < pixels.len() {
                            pixels[idx] = 0;
                        }
                    }
                }
            }
        }
        crate::css::ClipPath::Polygon { points } => {
            // Convert permille points to pixel coordinates.
            let poly: Vec<(i64, i64)> = points
                .iter()
                .map(|(x, y)| (*x as i64 * w / 1000, *y as i64 * h / 1000))
                .collect();
            if poly.len() < 3 {
                return;
            }
            for y in 0..height {
                for x in 0..width {
                    if !point_in_polygon(x as i64, y as i64, &poly) {
                        let idx = (y * width + x) as usize;
                        if idx < pixels.len() {
                            pixels[idx] = 0;
                        }
                    }
                }
            }
        }
    }
}

/// Ray-casting point-in-polygon test for clip-path: polygon().
fn point_in_polygon(px: i64, py: i64, poly: &[(i64, i64)]) -> bool {
    let n = poly.len();
    if n < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = poly[i];
        let (xj, yj) = poly[j];
        let crosses = (yi > py) != (yj > py)
            && {
                // Avoid divide-by-zero when yj == yi; the (yi > py) != (yj > py) guard already
                // excludes that case but compute defensively.
                let denom = yj - yi;
                if denom == 0 {
                    false
                } else {
                    let x_intersect = xi + (py - yi) * (xj - xi) / denom;
                    px < x_intersect
                }
            };
        if crosses {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn render_layer(
    buffer: &mut [u32],
    buf_width: u32,
    buf_height: u32,
    offset_x: u32,
    offset_y: u32,
    scroll_y: u32,
    layer: &LayerCommand,
    y_override: Option<u32>,
    page_images: Option<&ImageStore>,
    fonts: &mut FontContext,
    scratch: &mut Vec<Vec<u32>>,
    depth: usize,
) {
    // Compute screen-space position using signed arithmetic to handle layers above viewport
    let layer_y = y_override.unwrap_or(layer.y);
    let layer_screen_y = layer_y as i64 + offset_y as i64 - scroll_y as i64;
    let layer_screen_x = layer.x as i64 + offset_x as i64;

    // Content viewport top/left: the chrome (address bar, etc.) occupies [0, offset_y) rows
    // and [0, offset_x) cols. Layers must not bleed into the chrome area.
    let content_top = offset_y as i64;
    let content_left = offset_x as i64;

    // Skip layers fully below or to the right of viewport/buffer
    if layer_screen_y >= buf_height as i64 {
        return;
    }
    if layer_screen_x >= buf_width as i64 {
        return;
    }
    // Skip layers fully within the chrome (no part reaches the content area)
    if layer_screen_y + layer.height as i64 <= content_top {
        return;
    }
    if layer_screen_x + layer.width as i64 <= content_left {
        return;
    }

    // Clip: how many rows/cols of the layer are above/left of the content viewport
    let src_y_start = (content_top - layer_screen_y).max(0) as u32;
    let src_x_start = (content_left - layer_screen_x).max(0) as u32;
    // Destination in the main buffer: at least content_top / content_left
    let dst_y = layer_screen_y.max(content_top) as u32;
    let dst_x = layer_screen_x.max(content_left) as u32;

    // Visible size: clipped by layer bounds and buffer bounds
    let visible_h = layer
        .height
        .saturating_sub(src_y_start)
        .min(buf_height.saturating_sub(dst_y));
    let visible_w = layer
        .width
        .saturating_sub(src_x_start)
        .min(buf_width.saturating_sub(dst_x));
    if visible_h == 0 || visible_w == 0 {
        return;
    }

    // Offscreen buffer uses the full layer dimensions (layer.width × layer.height) so that
    // sub-commands are rendered at their natural layer-relative coordinates with scroll_y=0.
    // This avoids the y-straddling bug where a command starting above src_y_start (e.g. a
    // large background rect or image) would be shifted to y=0 via saturating_sub, causing
    // the wrong portion of its content to appear in the visible slice.
    // We copy the backdrop into the visible rows only, render all commands (scroll_y=0),
    // then blend only the visible rows back to the main buffer.
    let ow = layer.width; // full layer width
    let oh = layer.height; // full layer height — natural coordinate space

    // Use checked_mul so pathological dimensions (which would overflow usize in release
    // or panic in debug) are caught safely before any allocation attempt.
    let Some(needed) = (ow as usize).checked_mul(oh as usize) else {
        return;
    };
    // Note: we allocate the full layer.width × layer.height even when only visible_w × visible_h
    // pixels are actually blended back to the screen. This is a deliberate trade-off: allocating
    // only the visible slice and translating sub-command coordinates by -src_y_start would be
    // more memory-efficient, but it reintroduces the y-straddling bug (commands that start above
    // the visible window are clamped to y=0 via saturating_sub, showing the wrong content).
    // The full-height approach keeps sub-commands at their natural layer-relative coordinates so
    // the existing viewport culling in render_commands() handles out-of-view commands correctly.

    // Safety guard: refuse to allocate an obviously pathological offscreen buffer.
    // 4096×4096 (~16 MP) is well above any screen size we realistically support (~64MB max).
    // A layer larger than this is almost certainly a bug in layout (e.g. height not clamped).
    const MAX_OFFSCREEN_PIXELS: usize = 4096 * 4096;
    if needed > MAX_OFFSCREEN_PIXELS {
        // Degraded fallback: the layer is too large to allocate an offscreen buffer.
        // Sub-commands are rendered directly into the main buffer WITHOUT applying
        // layer.opacity — the element will appear fully opaque rather than at its
        // declared opacity. This is a rare edge case (>4096×4096 px elements) and
        // is preferable to silently dropping the element entirely.
        // A production fix would tile the layer or use a clipped compositing path.
        //
        // Layer commands are layer-relative (rebased to origin 0,0 by rebase_commands at
        // layout time). Pass scroll_y=0 so commands render at their natural layer-relative
        // coordinates. Account for page scroll by adjusting offset_y by layer.y - scroll_y.
        render_commands(
            buffer,
            buf_width,
            buf_height,
            offset_x.saturating_add(layer.x),
            offset_y.saturating_add(layer.y).saturating_sub(scroll_y),
            layer.height, // viewport for layer is its own height
            0,            // layer-relative scroll = 0
            &layer.commands,
            page_images,
            fonts,
            scratch,
            depth + 1,
        );
        return;
    }

    // Depth-indexed pool: scratch[depth] is the reusable offscreen Vec for this level.
    // Nested DrawCommand::Layer calls use depth+1 so each nesting level has its own slot.
    // After this frame, the Vec is returned to the pool with its capacity intact,
    // so subsequent frames skip allocation entirely.
    if scratch.len() <= depth {
        scratch.resize_with(depth + 1, Vec::new);
    }

    // Take the offscreen Vec out of the pool temporarily.
    // This gives us an exclusive &mut to `offscreen` while leaving `scratch` free to pass
    // to render_commands for nested layers — no aliasing, no unsafe needed.
    let mut offscreen = std::mem::take(&mut scratch[depth]);
    offscreen.resize(needed, 0);

    let has_scale = (layer.scale_x != 0 && layer.scale_x != 1000)
        || (layer.scale_y != 0 && layer.scale_y != 1000);
    let has_rotate = layer.rotate_millideg != 0;
    let has_transform = has_scale || has_rotate;

    // Copy backdrop from the main framebuffer into the visible rows of the offscreen.
    // Offscreen row (src_y_start + r) ↔ buffer row (dst_y + r), for r in 0..visible_h.
    //
    // For transformed layers we skip this: the content is rendered over a transparent
    // (0) background and composited per-pixel afterward, so the transformed footprint can
    // grow beyond the element box (scale) and rotated-out corners show the page backdrop
    // instead of black. (Limitation: pure-black content in a transformed element is
    // treated as transparent.)
    if !has_transform {
        for row in 0..visible_h {
            let buf_row = dst_y + row;
            let off_row = src_y_start + row;
            let buf_start = (buf_row * buf_width + dst_x) as usize;
            let off_start = (off_row * ow + src_x_start) as usize;
            let len = visible_w as usize;
            if buf_start + len <= buffer.len() && off_start + len <= offscreen.len() {
                offscreen[off_start..off_start + len]
                    .copy_from_slice(&buffer[buf_start..buf_start + len]);
            }
        }
    }

    // Render layer's sub-commands into the full-size offscreen buffer at natural coordinates.
    // Sub-commands are layer-relative (rebased at layout time), so x/y offsets are 0.
    // scroll_y = 0: commands render at their layer-relative y without any shift.
    // scratch is free (offscreen was taken via mem::take), so nested layers can use it.
    render_commands(
        &mut offscreen,
        ow,
        oh,
        0,  // x offset: sub-commands are layer-relative
        0,  // y offset: sub-commands are layer-relative
        oh, // viewport height = full layer height (all commands visible)
        0,  // scroll_y = 0: render at natural layer-relative coordinates
        &layer.commands,
        page_images,
        fonts,
        scratch,
        depth + 1, // nested layers use the next depth slot
    );

    // Apply CSS filter effects to the offscreen buffer.
    // Blur: applied first (uses neighboring pixel values before brightness skews them)
    if layer.blur_px > 0 {
        apply_box_blur(&mut offscreen, ow, oh, layer.blur_px);
    }
    // Brightness: applied after blur
    if layer.brightness != 10000 {
        apply_brightness(&mut offscreen, layer.brightness);
    }

    // Apply CSS clip-path: blank out pixels outside the shape.
    if let Some(ref clip) = layer.clip_path {
        apply_clip_path(&mut offscreen, ow, oh, clip);
    }

    if has_transform {
        // Transform path: composite the content through the transform onto the main buffer,
        // iterating the transformed bounding box so scale grows the on-screen footprint and
        // rotation is not clipped to the element box. The element's top-left maps to
        // (layer_screen_x, layer_screen_y); pixels outside the content sample as transparent.
        composite_transformed_layer(
            buffer,
            buf_width,
            buf_height,
            &offscreen,
            ow,
            oh,
            layer_screen_x,
            layer_screen_y,
            layer,
            content_left,
            content_top,
        );
    } else {
        // Blend the visible rows of the offscreen back onto the main buffer with opacity.
        // Visible row r: offscreen row (src_y_start + r) → buffer row (dst_y + r).
        // Read from src_x_start in the offscreen (horizontal clip offset).
        let opacity = layer.opacity as u32;
        let buf_end = buffer.len();
        let off_end = offscreen.len();
        for row in 0..visible_h {
            let buf_row_start = (dst_y + row) as usize * buf_width as usize + dst_x as usize;
            let off_row_start = (src_y_start + row) as usize * ow as usize + src_x_start as usize;
            if buf_row_start + visible_w as usize > buf_end
                || off_row_start + visible_w as usize > off_end
            {
                continue;
            }
            for col in 0..visible_w as usize {
                let src = offscreen[off_row_start + col];
                let dst_px = buffer[buf_row_start + col];
                let r = ((src >> 16 & 0xFF) * opacity
                    + (dst_px >> 16 & 0xFF) * (255 - opacity)
                    + 127)
                    / 255;
                let g = ((src >> 8 & 0xFF) * opacity
                    + (dst_px >> 8 & 0xFF) * (255 - opacity)
                    + 127)
                    / 255;
                let b =
                    ((src & 0xFF) * opacity + (dst_px & 0xFF) * (255 - opacity) + 127) / 255;
                buffer[buf_row_start + col] = (r << 16) | (g << 8) | b;
            }
        }
    }

    // Return offscreen Vec to the pool so its capacity is reused next frame.
    // Trim if dramatically over-sized: if the buffer is more than 4× larger than what
    // this frame needed (and wastes >1 MB), release the excess so a previously-huge
    // layer (e.g. a tall scrollable body) doesn't permanently inflate RAM after navigation.
    const MAX_OVERAGE_PIXELS: usize = 256 * 1024; // 1 MB = 256K u32 pixels
    if offscreen.capacity() > needed * 4 && offscreen.capacity() - needed > MAX_OVERAGE_PIXELS {
        offscreen.shrink_to(needed * 2);
    }
    scratch[depth] = offscreen;
}

/// Composite an element-sized content buffer onto the main framebuffer through a CSS
/// transform (scale + rotate about the transform origin). The output is iterated over the
/// transformed bounding box, so `scale` grows the on-screen footprint and `rotate` is not
/// clipped to the element box. Content pixels equal to 0 are treated as transparent and
/// leave the backdrop intact. `elem_screen_x/y` is where the element's top-left sits on
/// screen; `clip_left/top` keep the layer out of the chrome area.
#[allow(clippy::too_many_arguments)]
fn composite_transformed_layer(
    buffer: &mut [u32],
    buf_width: u32,
    buf_height: u32,
    content: &[u32],
    cw: u32,
    ch: u32,
    elem_screen_x: i64,
    elem_screen_y: i64,
    layer: &LayerCommand,
    clip_left: i64,
    clip_top: i64,
) {
    if cw == 0 || ch == 0 {
        return;
    }
    let cwf = cw as f32;
    let chf = ch as f32;
    let sx = if layer.scale_x == 0 {
        1.0f32
    } else {
        layer.scale_x as f32 / 1000.0
    };
    let sy = if layer.scale_y == 0 {
        1.0f32
    } else {
        layer.scale_y as f32 / 1000.0
    };
    let angle = (layer.rotate_millideg as f32 / 1000.0).to_radians();
    let cos_a = angle.cos();
    let sin_a = angle.sin();
    // Transform origin in element-local pixels.
    let ox = layer.origin_x as f32 / 1000.0 * cwf;
    let oy = layer.origin_y as f32 / 1000.0 * chf;

    // Forward-map element-local (lx,ly) → output-local (about the origin): scale then rotate.
    let forward = |lx: f32, ly: f32| -> (f32, f32) {
        let s_x = (lx - ox) * sx;
        let s_y = (ly - oy) * sy;
        let rx = s_x * cos_a - s_y * sin_a;
        let ry = s_x * sin_a + s_y * cos_a;
        (rx + ox, ry + oy)
    };

    // Output bounding box from the four element corners.
    let corners = [
        forward(0.0, 0.0),
        forward(cwf, 0.0),
        forward(0.0, chf),
        forward(cwf, chf),
    ];
    let mut min_x = f32::MAX;
    let mut max_x = f32::MIN;
    let mut min_y = f32::MAX;
    let mut max_y = f32::MIN;
    for (x, y) in corners {
        min_x = min_x.min(x);
        max_x = max_x.max(x);
        min_y = min_y.min(y);
        max_y = max_y.max(y);
    }

    let opacity = layer.opacity as u32;
    let inv_sx = if sx.abs() < 1e-4 { 0.0 } else { 1.0 / sx };
    let inv_sy = if sy.abs() < 1e-4 { 0.0 } else { 1.0 / sy };

    for out_y in (min_y.floor() as i64)..(max_y.ceil() as i64) {
        let screen_y = elem_screen_y + out_y;
        if screen_y < clip_top || screen_y >= buf_height as i64 {
            continue;
        }
        for out_x in (min_x.floor() as i64)..(max_x.ceil() as i64) {
            let screen_x = elem_screen_x + out_x;
            if screen_x < clip_left || screen_x >= buf_width as i64 {
                continue;
            }
            // Inverse-map output-local → element-local source: undo rotate then scale.
            let lx = out_x as f32 - ox;
            let ly = out_y as f32 - oy;
            let inv_rx = lx * cos_a + ly * sin_a;
            let inv_ry = -lx * sin_a + ly * cos_a;
            let src_x = inv_rx * inv_sx + ox;
            let src_y = inv_ry * inv_sy + oy;
            let ix = src_x.round() as i64;
            let iy = src_y.round() as i64;
            if ix < 0 || iy < 0 || ix >= cw as i64 || iy >= ch as i64 {
                continue;
            }
            let src = content[iy as usize * cw as usize + ix as usize];
            if src == 0 {
                continue; // uncovered content → transparent, keep backdrop
            }
            let dst_idx = screen_y as usize * buf_width as usize + screen_x as usize;
            if dst_idx >= buffer.len() {
                continue;
            }
            let dst_px = buffer[dst_idx];
            let r =
                ((src >> 16 & 0xFF) * opacity + (dst_px >> 16 & 0xFF) * (255 - opacity) + 127) / 255;
            let g =
                ((src >> 8 & 0xFF) * opacity + (dst_px >> 8 & 0xFF) * (255 - opacity) + 127) / 255;
            let b = ((src & 0xFF) * opacity + (dst_px & 0xFF) * (255 - opacity) + 127) / 255;
            buffer[dst_idx] = (r << 16) | (g << 8) | b;
        }
    }
}

fn max_scroll(viewport_height: u32, content_height: u32) -> u32 {
    content_height.saturating_sub(viewport_height)
}

fn resolve_text_input_value(
    focused_editor: Option<&AddressBarState>,
    control_value: &str,
) -> String {
    focused_editor
        .map(|editor| editor.text().to_string())
        .unwrap_or_else(|| control_value.to_string())
}

fn read_clipboard_text() -> Option<String> {
    let mut clipboard = Clipboard::new().ok()?;
    clipboard.get_text().ok()
}

fn write_clipboard_text(text: &str) -> bool {
    Clipboard::new()
        .and_then(|mut clipboard| clipboard.set_text(text.to_string()))
        .is_ok()
}

fn draw_rect(
    buffer: &mut [u32],
    width: u32,
    height: u32,
    x: u32,
    y: u32,
    rect_width: u32,
    rect_height: u32,
    color: Color,
) {
    let max_x = x.saturating_add(rect_width).min(width);
    let max_y = y.saturating_add(rect_height).min(height);

    for row in y..max_y {
        let row_offset = row as usize * width as usize;
        for column in x..max_x {
            buffer[row_offset + column as usize] = color;
        }
    }
}

fn draw_rounded_rect(
    buffer: &mut [u32],
    buf_w: u32,
    buf_h: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    radius: u32,
    color: u32,
) {
    if radius == 0 || w == 0 || h == 0 {
        draw_rect(buffer, buf_w, buf_h, x, y, w, h, color);
        return;
    }
    let r = radius.min(w / 2).min(h / 2);
    if r == 0 {
        draw_rect(buffer, buf_w, buf_h, x, y, w, h, color);
        return;
    }

    let x2 = x.saturating_add(w);
    let y2 = y.saturating_add(h);
    let cx_left = x.saturating_add(r);
    let cx_right = x2.saturating_sub(r);
    let cy_top = y.saturating_add(r);
    let cy_bottom = y2.saturating_sub(r);
    let r_sq = (r as i64) * (r as i64);

    // Middle strip: full-width rows — no corner checks needed
    if cy_top < cy_bottom {
        draw_rect(
            buffer,
            buf_w,
            buf_h,
            x,
            cy_top,
            w,
            cy_bottom - cy_top,
            color,
        );
    }

    // Top corner strip: rows y..cy_top
    let py_end_top = cy_top.min(buf_h) as usize;
    for py in (y.min(buf_h) as usize)..py_end_top {
        let pv32 = py as u32;
        // Left corner: x..cx_left
        for px in (x.min(buf_w) as usize)..(cx_left.min(buf_w) as usize) {
            let pu32 = px as u32;
            let dx = cx_left.saturating_sub(pu32) as i64;
            let dy = cy_top.saturating_sub(pv32) as i64;
            if dx * dx + dy * dy <= r_sq {
                let idx = py * buf_w as usize + px;
                if idx < buffer.len() {
                    buffer[idx] = color;
                }
            }
        }
        // Middle of row: cx_left..cx_right (always inside)
        if cx_left < cx_right {
            draw_rect(
                buffer,
                buf_w,
                buf_h,
                cx_left,
                pv32,
                cx_right - cx_left,
                1,
                color,
            );
        }
        // Right corner: cx_right..x2
        for px in (cx_right.min(buf_w) as usize)..(x2.min(buf_w) as usize) {
            let pu32 = px as u32;
            let dx = pu32.saturating_sub(cx_right) as i64;
            let dy = cy_top.saturating_sub(pv32) as i64;
            if dx * dx + dy * dy <= r_sq {
                let idx = py * buf_w as usize + px;
                if idx < buffer.len() {
                    buffer[idx] = color;
                }
            }
        }
    }

    // Bottom corner strip: rows cy_bottom..y2
    let py_start_bot = cy_bottom.min(buf_h) as usize;
    let py_end_bot = y2.min(buf_h) as usize;
    for py in py_start_bot..py_end_bot {
        let pv32 = py as u32;
        // Left corner
        for px in (x.min(buf_w) as usize)..(cx_left.min(buf_w) as usize) {
            let pu32 = px as u32;
            let dx = cx_left.saturating_sub(pu32) as i64;
            let dy = pv32.saturating_sub(cy_bottom) as i64;
            if dx * dx + dy * dy <= r_sq {
                let idx = py * buf_w as usize + px;
                if idx < buffer.len() {
                    buffer[idx] = color;
                }
            }
        }
        // Middle of row
        if cx_left < cx_right {
            draw_rect(
                buffer,
                buf_w,
                buf_h,
                cx_left,
                pv32,
                cx_right - cx_left,
                1,
                color,
            );
        }
        // Right corner
        for px in (cx_right.min(buf_w) as usize)..(x2.min(buf_w) as usize) {
            let pu32 = px as u32;
            let dx = pu32.saturating_sub(cx_right) as i64;
            let dy = pv32.saturating_sub(cy_bottom) as i64;
            if dx * dx + dy * dy <= r_sq {
                let idx = py * buf_w as usize + px;
                if idx < buffer.len() {
                    buffer[idx] = color;
                }
            }
        }
    }
}

fn draw_rect_outline(
    buffer: &mut [u32],
    width: u32,
    height: u32,
    x: u32,
    y: u32,
    rect_width: u32,
    rect_height: u32,
    color: Color,
) {
    if rect_width == 0 || rect_height == 0 {
        return;
    }

    draw_rect(buffer, width, height, x, y, rect_width, 1, color);
    draw_rect(
        buffer,
        width,
        height,
        x,
        y.saturating_add(rect_height.saturating_sub(1)),
        rect_width,
        1,
        color,
    );
    draw_rect(buffer, width, height, x, y, 1, rect_height, color);
    draw_rect(
        buffer,
        width,
        height,
        x.saturating_add(rect_width.saturating_sub(1)),
        y,
        1,
        rect_height,
        color,
    );
}

/// Draw an image tiled at its natural pixel size to fill the region
/// [x, x+draw_width) × [y, y+draw_height) in the buffer.
/// x and y are signed so callers can pass scroll-adjusted coords that may be negative.
/// min_y is the minimum buffer y that may be written (pass offset_y to prevent drawing
/// into the chrome / UI area above the content viewport).
fn draw_tiled_image(
    buffer: &mut [u32],
    buf_width: u32,
    buf_height: u32,
    x: i32,
    y: i32,
    draw_width: u32,
    draw_height: u32,
    image: &DecodedImage,
    min_y: i32,
    repeat: BackgroundRepeat,
) {
    let tile_w = image.width as i32;
    let tile_h = image.height as i32;
    if tile_w == 0 || tile_h == 0 || draw_width == 0 || draw_height == 0 {
        return;
    }

    let tile_x_axis = matches!(repeat, BackgroundRepeat::Repeat | BackgroundRepeat::RepeatX);
    let tile_y_axis = matches!(repeat, BackgroundRepeat::Repeat | BackgroundRepeat::RepeatY);

    let start_x = x.max(0) as u32;
    let start_y = y.max(min_y) as u32;
    // When repeating only on one axis, limit the other axis to a single tile width/height.
    let x_max_extent = if tile_x_axis { draw_width as i32 } else { tile_w };
    let y_max_extent = if tile_y_axis { draw_height as i32 } else { tile_h };
    let end_x = (x + x_max_extent).max(0).min(buf_width as i32) as u32;
    let end_y = (y + y_max_extent).max(0).min(buf_height as i32) as u32;

    for sy in start_y..end_y {
        for sx in start_x..end_x {
            // Compute offset within this tile using rem_euclid to handle negative x/y
            let px = ((sx as i32 - x).rem_euclid(tile_w)) as u32;
            let py = ((sy as i32 - y).rem_euclid(tile_h)) as u32;
            let src_idx = (py * image.width + px) as usize * 4;
            if src_idx + 3 >= image.rgba.len() {
                continue;
            }
            let r = image.rgba[src_idx] as u32;
            let g = image.rgba[src_idx + 1] as u32;
            let b = image.rgba[src_idx + 2] as u32;
            let a = image.rgba[src_idx + 3] as u32;
            let dst_idx = (sy * buf_width + sx) as usize;
            if a == 255 {
                buffer[dst_idx] = (r << 16) | (g << 8) | b;
            } else if a > 0 {
                let bg = buffer[dst_idx];
                let bg_r = (bg >> 16) & 0xFF;
                let bg_g = (bg >> 8) & 0xFF;
                let bg_b = bg & 0xFF;
                let out_r = (r * a + bg_r * (255 - a)) / 255;
                let out_g = (g * a + bg_g * (255 - a)) / 255;
                let out_b = (b * a + bg_b * (255 - a)) / 255;
                buffer[dst_idx] = (out_r << 16) | (out_g << 8) | out_b;
            }
        }
    }
}

fn draw_scaled_image(
    buffer: &mut [u32],
    width: u32,
    height: u32,
    x: i32,
    y: i32,
    draw_width: u32,
    draw_height: u32,
    image: &DecodedImage,
    object_fit: ObjectFit,
    object_position_x: u32,
    object_position_y: u32,
    min_y: i32,
) {
    if draw_width == 0 || draw_height == 0 || image.width == 0 || image.height == 0 {
        return;
    }

    // Compute the effective source region and destination region based on object-fit
    // src_* = region within the source image to sample
    // render_* = region within the destination box to paint
    let (src_x_off, src_y_off, src_w, src_h, render_x, render_y, render_w, render_h) =
        match object_fit {
            ObjectFit::Fill => {
                // Stretch to fill: use full source, full dest
                (
                    0u32,
                    0u32,
                    image.width,
                    image.height,
                    0u32,
                    0u32,
                    draw_width,
                    draw_height,
                )
            }
            ObjectFit::None => {
                // No scaling: use natural size, positioned by object-position
                let natural_w = image.width.min(draw_width);
                let natural_h = image.height.min(draw_height);
                // If image is smaller than box, place at object-position
                let rx = if draw_width > image.width {
                    ((draw_width - image.width) as f32 * object_position_x as f32 / 100.0).round()
                        as u32
                } else {
                    0
                };
                let ry = if draw_height > image.height {
                    ((draw_height - image.height) as f32 * object_position_y as f32 / 100.0).round()
                        as u32
                } else {
                    0
                };
                // source crop when image is larger than box
                let sx = if image.width > draw_width {
                    ((image.width - draw_width) as f32 * object_position_x as f32 / 100.0).round()
                        as u32
                } else {
                    0
                };
                let sy = if image.height > draw_height {
                    ((image.height - draw_height) as f32 * object_position_y as f32 / 100.0).round()
                        as u32
                } else {
                    0
                };
                (sx, sy, natural_w, natural_h, rx, ry, natural_w, natural_h)
            }
            ObjectFit::Contain => {
                // Scale uniformly to fit inside, with letterboxing
                let scale_x = draw_width as f32 / image.width as f32;
                let scale_y = draw_height as f32 / image.height as f32;
                let scale = scale_x.min(scale_y);
                let scaled_w = (image.width as f32 * scale).round().max(1.0) as u32;
                let scaled_h = (image.height as f32 * scale).round().max(1.0) as u32;
                let rx = ((draw_width.saturating_sub(scaled_w)) as f32 * object_position_x as f32
                    / 100.0)
                    .round() as u32;
                let ry = ((draw_height.saturating_sub(scaled_h)) as f32 * object_position_y as f32
                    / 100.0)
                    .round() as u32;
                (0, 0, image.width, image.height, rx, ry, scaled_w, scaled_h)
            }
            ObjectFit::Cover => {
                // Scale uniformly to fill, cropping excess
                let scale_x = draw_width as f32 / image.width as f32;
                let scale_y = draw_height as f32 / image.height as f32;
                let scale = scale_x.max(scale_y);
                let scaled_w = (image.width as f32 * scale).round().max(1.0) as u32;
                let scaled_h = (image.height as f32 * scale).round().max(1.0) as u32;
                // compute source crop to match the visible portion
                let excess_x = scaled_w.saturating_sub(draw_width);
                let excess_y = scaled_h.saturating_sub(draw_height);
                let sx_px = (excess_x as f32 * object_position_x as f32 / 100.0).round() as u32;
                let sy_px = (excess_y as f32 * object_position_y as f32 / 100.0).round() as u32;
                // convert back to source coords
                let sx = (sx_px as f32 / scale).round() as u32;
                let sy = (sy_px as f32 / scale).round() as u32;
                let src_w_used = ((draw_width as f32 / scale).round() as u32)
                    .min(image.width.saturating_sub(sx))
                    .max(1);
                let src_h_used = ((draw_height as f32 / scale).round() as u32)
                    .min(image.height.saturating_sub(sy))
                    .max(1);
                (
                    sx,
                    sy,
                    src_w_used,
                    src_h_used,
                    0,
                    0,
                    draw_width,
                    draw_height,
                )
            }
            ObjectFit::ScaleDown => {
                // min(none, contain): use natural size if smaller, else contain
                if image.width <= draw_width && image.height <= draw_height {
                    // same as none (image fits naturally)
                    let rx = ((draw_width - image.width) as f32 * object_position_x as f32 / 100.0)
                        .round() as u32;
                    let ry = ((draw_height - image.height) as f32 * object_position_y as f32
                        / 100.0)
                        .round() as u32;
                    (
                        0,
                        0,
                        image.width,
                        image.height,
                        rx,
                        ry,
                        image.width,
                        image.height,
                    )
                } else {
                    // same as contain
                    let scale_x = draw_width as f32 / image.width as f32;
                    let scale_y = draw_height as f32 / image.height as f32;
                    let scale = scale_x.min(scale_y);
                    let scaled_w = (image.width as f32 * scale).round().max(1.0) as u32;
                    let scaled_h = (image.height as f32 * scale).round().max(1.0) as u32;
                    let rx = ((draw_width.saturating_sub(scaled_w)) as f32
                        * object_position_x as f32
                        / 100.0)
                        .round() as u32;
                    let ry = ((draw_height.saturating_sub(scaled_h)) as f32
                        * object_position_y as f32
                        / 100.0)
                        .round() as u32;
                    (0, 0, image.width, image.height, rx, ry, scaled_w, scaled_h)
                }
            }
        };

    if render_w == 0 || render_h == 0 || src_w == 0 || src_h == 0 {
        return;
    }

    // dest_start in global buffer coordinates (signed)
    let dest_start_x_signed = x + render_x as i32;
    let dest_start_y_signed = y + render_y as i32;

    // Clip to buffer and draw-box bounds
    let dest_end_x_signed = (dest_start_x_signed + render_w as i32)
        .min(x + draw_width as i32)
        .min(width as i32);
    let dest_end_y_signed = (dest_start_y_signed + render_h as i32)
        .min(y + draw_height as i32)
        .min(height as i32);

    // Actual buffer start (clamped to min_y / 0 to avoid writing into the chrome area)
    let dest_start_x = dest_start_x_signed.max(0) as u32;
    let dest_start_y = dest_start_y_signed.max(min_y) as u32;
    let max_dx = dest_end_x_signed.max(0) as u32;
    let max_dy = dest_end_y_signed.max(0) as u32;

    // How many dest rows/cols were skipped due to start being above min_y (used to advance source)
    let y_skip = (min_y - dest_start_y_signed).max(0) as u32;
    let x_skip = (-dest_start_x_signed).max(0) as u32;

    if max_dx <= dest_start_x || max_dy <= dest_start_y {
        return;
    }

    for dest_y in dest_start_y..max_dy {
        let local_y = (dest_y - dest_start_y) + y_skip; // includes skipped rows
        let source_y = (src_y_off as u64 + local_y as u64 * src_h as u64 / render_h as u64) as u32;
        let source_y = source_y.min(image.height.saturating_sub(1));
        let row_offset = dest_y as usize * width as usize;

        for dest_x in dest_start_x..max_dx {
            let local_x = (dest_x - dest_start_x) + x_skip; // includes skipped cols
            let source_x =
                (src_x_off as u64 + local_x as u64 * src_w as u64 / render_w as u64) as u32;
            let source_x = source_x.min(image.width.saturating_sub(1));
            let source_index = ((source_y * image.width + source_x) * 4) as usize;
            let source = &image.rgba[source_index..source_index + 4];
            let alpha = source[3] as u32;

            let pixel_index = row_offset + dest_x as usize;
            if alpha == 255 {
                buffer[pixel_index] =
                    ((source[0] as u32) << 16) | ((source[1] as u32) << 8) | source[2] as u32;
                continue;
            }

            let destination = buffer[pixel_index];
            let destination_r = (destination >> 16) & 0xFF;
            let destination_g = (destination >> 8) & 0xFF;
            let destination_b = destination & 0xFF;
            let inverse_alpha = 255 - alpha;

            let blended_r = (source[0] as u32 * alpha + destination_r * inverse_alpha) / 255;
            let blended_g = (source[1] as u32 * alpha + destination_g * inverse_alpha) / 255;
            let blended_b = (source[2] as u32 * alpha + destination_b * inverse_alpha) / 255;

            buffer[pixel_index] = (blended_r << 16) | (blended_g << 8) | blended_b;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AddressBarState, build_get_form_submission_url, composite_transformed_layer,
        cursor_index_for_address_x, layout_error_document, looks_like_local_address, max_scroll,
        parse_address_input, resolve_text_input_value,
    };
    use crate::font::FontContext;
    use crate::layout::LayerCommand;

    fn solid_layer(width: u32, height: u32, scale_milli: u32, rotate_millideg: i32) -> LayerCommand {
        LayerCommand {
            x: 0,
            y: 0,
            width,
            height,
            opacity: 255,
            blur_px: 0,
            brightness: 10000,
            scale_x: scale_milli,
            scale_y: scale_milli,
            rotate_millideg,
            origin_x: 500,
            origin_y: 500,
            clip_path: None,
            commands: Vec::new(),
        }
    }

    #[test]
    fn transform_scale_grows_on_screen_footprint() {
        // 4x4 solid-white content scaled 2x should cover ~8x8 pixels on the buffer,
        // not stay clamped to the 4x4 element box.
        let cw = 4u32;
        let ch = 4u32;
        let content = vec![0x00FF_FFFFu32 & 0xFF_FFFF; (cw * ch) as usize];
        let bw = 32u32;
        let bh = 32u32;
        let mut buffer = vec![0u32; (bw * bh) as usize];
        let layer = solid_layer(cw, ch, 2000, 0);
        // Place the element at (10,10) so the scaled box stays inside the buffer.
        composite_transformed_layer(&mut buffer, bw, bh, &content, cw, ch, 10, 10, &layer, 0, 0);
        let painted = buffer.iter().filter(|&&p| p != 0).count();
        // The 4x4 element box is 16px; scale(2) must grow the footprint well beyond that
        // (ideal 8x8 = 64px; nearest-neighbor edge rounding trims a few).
        assert!(
            painted > 36,
            "scale(2) of a 4x4 box should paint far more than its 16px box, got {painted}"
        );
    }

    #[test]
    fn transform_rotate_does_not_fill_corners_with_black() {
        // A rotated solid square must leave the backdrop in the corners (no black fill).
        let cw = 16u32;
        let ch = 16u32;
        let content = vec![0x00FF_FFFFu32 & 0xFF_FFFF; (cw * ch) as usize]; // white
        let bw = 48u32;
        let bh = 48u32;
        let backdrop = 0x0010_2030u32 & 0xFF_FFFF;
        let mut buffer = vec![backdrop; (bw * bh) as usize];
        let layer = solid_layer(cw, ch, 1000, 45_000); // 45deg, no scale
        composite_transformed_layer(&mut buffer, bw, bh, &content, cw, ch, 16, 16, &layer, 0, 0);
        // No pixel should have become pure black (0): corners keep the backdrop, center is white.
        let black = buffer.iter().filter(|&&p| p == 0).count();
        assert_eq!(black, 0, "rotated square must not introduce black corner pixels");
        // And the rotation must have painted some white pixels.
        let white = buffer.iter().filter(|&&p| p == 0x00FF_FFFF & 0xFF_FFFF).count();
        assert!(white > 0, "rotated white square should paint white pixels");
    }

    #[test]
    fn max_scroll_stops_at_zero() {
        assert_eq!(max_scroll(400, 100), 0);
        assert_eq!(max_scroll(100, 400), 300);
    }

    #[test]
    fn error_layout_contains_text_commands() {
        let mut fonts = FontContext::load();
        let layout = layout_error_document(
            &super::ErrorDocument {
                lines: vec!["# Oops".to_string(), "hello".to_string()],
            },
            320,
            &mut fonts,
        );

        assert!(layout.texts().len() >= 2);
    }

    #[test]
    fn bare_addresses_default_to_https() {
        let url = parse_address_input("google.com").unwrap();
        assert_eq!(url.to_string(), "https://google.com/");
    }

    #[test]
    fn localhost_defaults_to_http() {
        let url = parse_address_input("localhost:8000/demo").unwrap();
        assert_eq!(url.to_string(), "http://localhost:8000/demo");
        assert!(looks_like_local_address("127.0.0.1:3000"));
    }

    #[test]
    fn address_bar_backspace_handles_unicode() {
        let mut state = AddressBarState::new("阿部A".to_string());
        state.focus_at(state.char_len());

        assert!(state.backspace());
        assert_eq!(state.text(), "阿部");
        assert!(state.backspace());
        assert_eq!(state.text(), "阿");
    }

    #[test]
    fn address_bar_ignores_control_characters() {
        let mut state = AddressBarState::new("https://google.com".to_string());
        state.focus_at(state.char_len());

        assert!(!state.insert_text("\u{8}"));
        assert!(!state.insert_text("\r"));
        assert_eq!(state.text(), "https://google.com");
    }

    #[test]
    fn address_bar_select_all_replaces_text() {
        let mut state = AddressBarState::new("https://google.com".to_string());

        assert!(state.select_all());
        assert!(state.selection_range().is_some());
        assert!(state.insert_text("https://youtube.com"));
        assert_eq!(state.text(), "https://youtube.com");
        assert!(state.selection_range().is_none());
    }

    #[test]
    fn text_input_value_prefers_live_editor_text() {
        let mut editor = AddressBarState::new("live".to_string());
        editor.focus_at(editor.char_len());

        assert_eq!(resolve_text_input_value(Some(&editor), "dom"), "live");
        assert_eq!(resolve_text_input_value(None, "dom"), "dom");
    }

    #[test]
    fn address_bar_shift_navigation_creates_selection() {
        let mut state = AddressBarState::new("google.com".to_string());
        state.focus_at(state.char_len());

        assert!(state.move_left(true));
        assert!(state.selection_range().is_some());
        assert!(state.backspace());
        assert_eq!(state.text(), "google.co");
        assert!(state.selection_range().is_none());
    }

    #[test]
    fn address_bar_selected_text_returns_current_selection() {
        let mut state = AddressBarState::new("阿部寛 homepage".to_string());
        state.focus_at(state.char_len());

        assert!(state.move_word_left(true));
        assert_eq!(state.selected_text().as_deref(), Some("homepage"));
    }

    #[test]
    fn address_bar_cut_selection_returns_text_and_removes_it() {
        let mut state = AddressBarState::new("https://google.com".to_string());
        assert!(state.select_all());

        assert_eq!(
            state.cut_selection_text().as_deref(),
            Some("https://google.com")
        );
        assert_eq!(state.text(), "");
        assert!(state.selection_range().is_none());
    }

    #[test]
    fn clicking_text_x_resolves_cursor_position() {
        let mut fonts = FontContext::load();
        let mut state = AddressBarState::new("google.com".to_string());
        state.focus_at(state.char_len());

        let cursor = cursor_index_for_address_x(&state, &mut fonts, 300, 0.0);
        assert_eq!(cursor, 0);
    }

    #[test]
    fn build_get_form_submission_appends_encoded_query() {
        let target = build_get_form_submission_url(
            "https://www.google.com/search",
            &[("q".to_string(), "rust browser".to_string())],
            false,
        )
        .unwrap();

        assert_eq!(target, "https://www.google.com/search?q=rust+browser");
    }

    #[test]
    fn build_get_form_submission_preserves_existing_query_and_fragment() {
        let target = build_get_form_submission_url(
            "https://example.com/find?src=home#results",
            &[
                ("q".to_string(), "hello world".to_string()),
                ("lang".to_string(), "ja".to_string()),
            ],
            false,
        )
        .unwrap();

        assert_eq!(
            target,
            "https://example.com/find?src=home&q=hello+world&lang=ja#results"
        );
    }

    #[test]
    fn build_get_form_submission_replaces_existing_query_when_requested() {
        let target = build_get_form_submission_url(
            "https://example.com/find?src=home#results",
            &[("q".to_string(), "hello world".to_string())],
            true,
        )
        .unwrap();

        assert_eq!(target, "https://example.com/find?q=hello+world");
    }

    #[test]
    fn build_get_form_submission_preserves_fragment_only_actions() {
        let target = build_get_form_submission_url(
            "https://example.com/find#results",
            &[("q".to_string(), "hello world".to_string())],
            false,
        )
        .unwrap();

        assert_eq!(target, "https://example.com/find?q=hello+world#results");
    }
}
