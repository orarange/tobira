use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::rc::Rc;

use arboard::Clipboard;
use softbuffer::{Context, Surface};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, OwnedDisplayHandle};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::window::{CursorIcon, ResizeDirection, Window};

use crate::browser::{BrowserPage, load_page};
use crate::css::{Color, DEFAULT_BACKGROUND_COLOR, DEFAULT_TEXT_COLOR, FontFamilyKind};
use crate::error::{BrowserError, Result};
use crate::font::FontContext;
use crate::image::DecodedImage;
use crate::layout::{FormControlCommand, FormControlKind, LayoutDocument, TextCommand, layout_styled_document};
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
    let event_loop = EventLoop::new().map_err(|error| BrowserError::message(error.to_string()))?;
    event_loop.set_control_flow(ControlFlow::Wait);

    let context = Context::new(event_loop.owned_display_handle())
        .map_err(|error| BrowserError::message(error.to_string()))?;
    let mut app = BrowserApp::new(initial_url, context);

    event_loop
        .run_app(&mut app)
        .map_err(|error| BrowserError::message(error.to_string()))
}

struct BrowserApp {
    current_url: Option<Url>,
    document: DocumentView,
    fonts: FontContext,
    context: Context<OwnedDisplayHandle>,
    window: Option<WindowHandle>,
    surface: Option<SurfaceHandle>,
    scroll_y: u32,
    modifiers: ModifiersState,
    cursor_position: PhysicalPosition<f64>,
    address_bar: AddressBarState,
    page_control_values: BTreeMap<usize, String>,
    focused_page_input: Option<FocusedPageInput>,
    hovered_target: HitTarget,
    hovered_link_url: Option<String>,
    ime_composing: bool,
}

impl BrowserApp {
    fn new(initial_url: Option<Url>, context: Context<OwnedDisplayHandle>) -> Self {
        let (current_url, document, address_bar) = match initial_url {
            Some(url) => {
                let document = DocumentView::load(url.clone());
                let address_bar = AddressBarState::new(url.to_string());
                (Some(url), document, address_bar)
            }
            None => {
                let mut address_bar = AddressBarState::new(String::new());
                address_bar.focus_at(0);
                (None, DocumentView::blank(), address_bar)
            }
        };

        Self {
            current_url,
            document,
            fonts: FontContext::load(),
            context,
            window: None,
            surface: None,
            scroll_y: 0,
            modifiers: ModifiersState::default(),
            cursor_position: PhysicalPosition::new(0.0, 0.0),
            address_bar,
            page_control_values: BTreeMap::new(),
            focused_page_input: None,
            hovered_target: HitTarget::None,
            hovered_link_url: None,
            ime_composing: false,
        }
    }

    fn load_url(&mut self, url: Url) {
        self.document = DocumentView::load(url.clone());
        self.current_url = Some(url.clone());
        self.address_bar.set_text(url.to_string());
        self.address_bar.blur();
        self.clear_page_control_state();
        self.scroll_y = 0;
        self.sync_window_title();
        self.sync_input_method();
        self.request_redraw();
    }

    fn reload(&mut self) {
        let Some(url) = self.current_url.clone() else {
            self.document = DocumentView::blank();
            self.scroll_y = 0;
            self.sync_window_title();
            self.request_redraw();
            return;
        };

        self.document = DocumentView::load(url.clone());
        self.current_url = Some(url.clone());
        self.address_bar.set_text(url.to_string());
        self.clear_page_control_state();
        self.scroll_y = 0;
        self.sync_window_title();
        self.sync_input_method();
        self.request_redraw();
    }

    fn navigate_to_address(&mut self) {
        let entered = self.address_bar.text().trim().to_string();
        if entered.is_empty() {
            self.current_url = None;
            self.document = DocumentView::blank();
            self.clear_page_control_state();
            self.scroll_y = 0;
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
                self.sync_window_title();
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

    fn sync_input_method(&mut self) {
        let Some(window) = &self.window else {
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

        let size = window.inner_size();
        let chrome = chrome_layout_metrics(&mut self.fonts, size.width);
        let body_top = chrome.height + FRAME_PADDING;
        let content_width = size.width.saturating_sub(FRAME_PADDING * 2).max(1);
        let layout = self.document.layout(content_width, &mut self.fonts);
        let Some(control) = layout.controls.iter().find(|control| control.id == focused.control_id)
        else {
            return;
        };

        let view = text_editor_view(
            &focused.editor,
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
        let caret_x = FRAME_PADDING
            .saturating_add(control.x)
            .saturating_add(CONTROL_PADDING_X)
            .saturating_add(view.caret_x);

        window.set_ime_cursor_area(
            PhysicalPosition::new(caret_x as i32, text_y as i32),
            PhysicalSize::new(1, line_height.max(1)),
        );
    }

    fn scroll_by(&mut self, delta: i32, viewport_height: u32, content_height: u32) {
        let max_scroll = max_scroll(viewport_height, content_height);
        let next = if delta.is_negative() {
            self.scroll_y.saturating_sub(delta.unsigned_abs())
        } else {
            self.scroll_y.saturating_add(delta as u32)
        };
        self.scroll_y = next.min(max_scroll);
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
        let body_top = chrome.height + FRAME_PADDING;
        let content_width = size.width.saturating_sub(FRAME_PADDING * 2).max(1);
        let viewport_height = size.height.saturating_sub(body_top + FRAME_PADDING).max(1);
        let layout = self.document.layout(content_width, &mut self.fonts);
        let max_scroll_y = max_scroll(viewport_height, layout.content_height);
        self.scroll_y = self.scroll_y.min(max_scroll_y);

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
            self.scroll_y,
            max_scroll_y,
        );
        paint_layout(
            &self.document,
            &mut self.fonts,
            &mut buffer,
            size.width,
            size.height,
            FRAME_PADDING,
            body_top,
            viewport_height,
            self.scroll_y,
            &layout,
            &self.page_control_values,
            self.focused_page_input.as_ref(),
            self.hovered_target,
        );

        buffer
            .present()
            .map_err(|error| BrowserError::message(error.to_string()))
    }

    fn content_metrics(&mut self, window_size: PhysicalSize<u32>) -> (u32, u32) {
        let chrome = chrome_layout_metrics(&mut self.fonts, window_size.width);
        let body_top = chrome.height + FRAME_PADDING;
        let content_width = window_size.width.saturating_sub(FRAME_PADDING * 2).max(1);
        let viewport_height = window_size
            .height
            .saturating_sub(body_top + FRAME_PADDING)
            .max(1);
        let content_height = self
            .document
            .layout(content_width, &mut self.fonts)
            .content_height;

        (viewport_height, content_height)
    }

    fn find_hovered_link(&mut self, window_size: PhysicalSize<u32>) -> Option<String> {
        let chrome = chrome_layout_metrics(&mut self.fonts, window_size.width);
        let body_top = chrome.height + FRAME_PADDING;
        let pos_x = self.cursor_position.x;
        let pos_y = self.cursor_position.y;
        if pos_y < body_top as f64 {
            return None;
        }
        let content_width = window_size.width.saturating_sub(FRAME_PADDING * 2).max(1);
        let layout = self.document.layout(content_width, &mut self.fonts);
        let content_y = (pos_y as u32).saturating_sub(body_top).saturating_add(self.scroll_y);
        let content_x = (pos_x as u32).saturating_sub(FRAME_PADDING);
        for link in &layout.links {
            if content_x >= link.x
                && content_x < link.x.saturating_add(link.width)
                && content_y >= link.y
                && content_y < link.y.saturating_add(link.height)
            {
                return Some(link.href.clone());
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
        let content_width = window_size.width.saturating_sub(FRAME_PADDING * 2).max(1);
        let layout = self.document.layout(content_width, &mut self.fonts);
        let content_y = (pos_y as u32)
            .saturating_sub(body_top)
            .saturating_add(self.scroll_y);
        let content_x = (pos_x as u32).saturating_sub(FRAME_PADDING);
        for control in &layout.controls {
            if content_x >= control.x
                && content_x < control.x.saturating_add(control.width)
                && content_y >= control.y
                && content_y < control.y.saturating_add(control.height)
            {
                return Some(match control.kind {
                    FormControlKind::TextInput => HitTarget::PageTextInput(control.id),
                    FormControlKind::Button => HitTarget::PageButton(control.id),
                });
            }
        }
        None
    }

    fn update_hover(&mut self, window_size: PhysicalSize<u32>) {
        let next = self.hit_test(window_size, self.cursor_position);
        let link_url = if next == HitTarget::None {
            self.find_hovered_link(window_size)
        } else {
            None
        };
        let changed = next != self.hovered_target || link_url != self.hovered_link_url;
        self.hovered_target = next;
        self.hovered_link_url = link_url;
        if changed {
            self.request_redraw();
        }
        if let Some(window) = &self.window {
            let icon = if self.hovered_link_url.is_some() {
                CursorIcon::Pointer
            } else {
                cursor_icon_for_target(next)
            };
            window.set_cursor(icon);
        }
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
            let focused_control_id = self.focused_page_input.as_ref().map(|focused| focused.control_id);
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
            KeyCode::ArrowDown => self.scroll_by(24, viewport_height, content_height),
            KeyCode::ArrowUp => self.scroll_by(-24, viewport_height, content_height),
            KeyCode::PageDown => self.scroll_by(
                viewport_height.saturating_sub(32) as i32,
                viewport_height,
                content_height,
            ),
            KeyCode::PageUp => self.scroll_by(
                -(viewport_height.saturating_sub(32) as i32),
                viewport_height,
                content_height,
            ),
            KeyCode::Home => self.scroll_y = 0,
            KeyCode::End => self.scroll_y = max_scroll(viewport_height, content_height),
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
        self.page_control_values.clear();
        self.focused_page_input = None;
        self.ime_composing = false;
    }

    fn blur_page_input(&mut self) {
        if self.focused_page_input.is_none() {
            return;
        }

        self.focused_page_input = None;
        self.ime_composing = false;
        self.sync_input_method();
        self.request_redraw();
    }

    fn control_current_value(&self, control: &FormControlCommand) -> String {
        if let Some(focused) = &self.focused_page_input
            && focused.control_id == control.id
        {
            return focused.editor.text().to_string();
        }

        self.page_control_values
            .get(&control.id)
            .cloned()
            .unwrap_or_else(|| control.value.clone())
    }

    fn focus_page_input_at(&mut self, control: &FormControlCommand, char_index: Option<usize>) {
        self.blur_address_bar();
        let mut editor = AddressBarState::new(self.control_current_value(control));
        editor.focus_at(char_index.unwrap_or_else(|| editor.char_len()));
        self.focused_page_input = Some(FocusedPageInput {
            control_id: control.id,
            editor,
        });
        self.sync_page_input_value();
        self.sync_input_method();
        self.request_redraw();
    }

    fn sync_page_input_value(&mut self) {
        if let Some(focused) = &self.focused_page_input {
            self.page_control_values
                .insert(focused.control_id, focused.editor.text().to_string());
        }
    }

    fn focused_page_editor_mut(&mut self) -> Option<&mut AddressBarState> {
        self.focused_page_input.as_mut().map(|focused| &mut focused.editor)
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

        self.request_redraw();
    }

    fn page_base_url(&self) -> Option<&Url> {
        match &self.document.content {
            DocumentContent::Loaded(page) => Some(&page.url),
            _ => self.current_url.as_ref(),
        }
    }

    fn current_layout(&mut self) -> LayoutDocument {
        let window_size = self
            .window
            .as_ref()
            .map(|window| window.inner_size())
            .unwrap_or_else(|| PhysicalSize::new(WINDOW_WIDTH, WINDOW_HEIGHT));
        let content_width = window_size.width.saturating_sub(FRAME_PADDING * 2).max(1);
        self.document.layout(content_width, &mut self.fonts)
    }

    fn current_page_control(&mut self, id: usize) -> Option<FormControlCommand> {
        self.current_layout()
            .controls
            .into_iter()
            .find(|control| control.id == id)
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
        }
        changed
    }

    fn submit_page_form(&mut self, trigger_control_id: usize) {
        let layout = self.current_layout();
        let Some(trigger) = layout
            .controls
            .iter()
            .find(|control| control.id == trigger_control_id)
            .cloned()
        else {
            return;
        };
        if trigger.disabled {
            return;
        }
        if matches!(trigger.kind, FormControlKind::Button) && !trigger.activates_submit {
            return;
        }
        if !trigger.form_method.eq_ignore_ascii_case("get") {
            self.document.status_line = format!(
                "Status: {} forms are not supported yet",
                trigger.form_method.to_ascii_uppercase()
            );
            self.request_redraw();
            return;
        }

        let mut fields = Vec::new();
        for control in &layout.controls {
            if control.form_id != trigger.form_id || control.disabled {
                continue;
            }
            if matches!(control.kind, FormControlKind::TextInput)
                && let Some(name) = &control.name
                && !name.is_empty()
            {
                fields.push((name.clone(), self.control_current_value(control)));
            }
        }
        if matches!(trigger.kind, FormControlKind::Button)
            && let Some(name) = &trigger.name
            && !name.is_empty()
        {
            fields.push((name.clone(), trigger.value.clone()));
        }

        let base = self.page_base_url().or(self.current_url.as_ref());
        let action = trigger.form_action.as_deref().unwrap_or("");
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
                    let local_x = self
                        .cursor_position
                        .x
                        .max((FRAME_PADDING + control.x + CONTROL_PADDING_X) as f64)
                        - (FRAME_PADDING + control.x + CONTROL_PADDING_X) as f64;
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
                self.blur_address_bar();
                self.blur_page_input();
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
            ChromeButton::Reload => self.reload(),
            ChromeButton::Navigate => self.navigate_to_address(),
            ChromeButton::Minimize => window.set_minimized(true),
            ChromeButton::ToggleMaximize => window.set_maximized(!window.is_maximized()),
            ChromeButton::Close => return true,
        }

        false
    }
}

impl ApplicationHandler for BrowserApp {
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
        self.sync_window_title();
        self.sync_input_method();
        self.request_redraw();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        let Some(window) = &self.window else {
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
                self.update_hover(size);
                self.sync_input_method();
                self.request_redraw();
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_position = position;
                self.update_hover(window.inner_size());
            }
            WindowEvent::CursorLeft { .. } => {
                self.hovered_target = HitTarget::None;
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
                if let PhysicalKey::Code(key_code) = event.physical_key {
                    if self.handle_key(key_code, window.inner_size(), event.repeat) {
                        event_loop.exit();
                        return;
                    }
                }

                if let Some(text) = event.text.as_deref() {
                    if !self.modifiers.control_key()
                        && !self.modifiers.alt_key()
                        && !self.modifiers.super_key()
                    {
                        self.handle_text_input(text);
                    }
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
}

#[derive(Debug, Clone)]
enum DocumentContent {
    Blank,
    Loaded(BrowserPage),
    Error(ErrorDocument),
}

#[derive(Debug, Clone)]
struct ErrorDocument {
    lines: Vec<String>,
}

impl DocumentView {
    fn blank() -> Self {
        Self {
            title: "New Tab".to_string(),
            status_line: "Status: ready".to_string(),
            subtitle: "Type a URL in the address bar and press Enter.".to_string(),
            content: DocumentContent::Blank,
        }
    }

    fn load(url: Url) -> Self {
        match load_page(&url) {
            Ok(page) => Self::from_page(page),
            Err(error) => Self::error(error.to_string()),
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

    fn layout(&self, width: u32, fonts: &mut FontContext) -> LayoutDocument {
        match &self.content {
            DocumentContent::Blank => LayoutDocument {
                background_color: DEFAULT_BACKGROUND_COLOR,
                content_height: 0,
                rects: Vec::new(),
                texts: Vec::new(),
                images: Vec::new(),
                links: Vec::new(),
                controls: Vec::new(),
            },
            DocumentContent::Loaded(page) => {
                layout_styled_document(&page.styled_document, &page.images, width, fonts)
            }
            DocumentContent::Error(error) => layout_error_document(error, width, fonts),
        }
    }
}

fn layout_error_document(
    document: &ErrorDocument,
    _width: u32,
    fonts: &mut FontContext,
) -> LayoutDocument {
    let mut texts = Vec::new();
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

        texts.push(TextCommand {
            x: 0,
            y: cursor_y,
            width: fonts.text_width_px(line, font_size_px, FontFamilyKind::Sans),
            text: line.clone(),
            font_size_px,
            font_family: FontFamilyKind::Sans,
            color,
            underline: false,
            bold: scale >= 3,
            italic: false,
        });

        cursor_y = cursor_y.saturating_add(height);
    }

    LayoutDocument {
        background_color: DEFAULT_BACKGROUND_COLOR,
        content_height: cursor_y,
        rects: Vec::new(),
        texts,
        images: Vec::new(),
        links: Vec::new(),
        controls: Vec::new(),
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
    // While a page input is focused, this native editor state is the source of truth
    // until we wire live DOM/event synchronization for script-driven value changes.
    control_id: usize,
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
    let reload_button = Rect {
        x: FRAME_PADDING,
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
        (action_url.split_once('#').map(|(head, _)| head).unwrap_or(action_url), String::new())
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
            FontFamilyKind::Sans,
        );
    }

    paint_button(
        fonts,
        buffer,
        width,
        height,
        chrome.reload_button,
        "R",
        hovered_target == HitTarget::Button(ChromeButton::Reload),
        false,
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
) {
    let color = if destructive {
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
        COLOR_HEADER_TEXT,
        true,
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
    page_control_values: &BTreeMap<usize, String>,
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

    let background = if matches!(control.kind, FormControlKind::Button) && is_hovered && !control.disabled
    {
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
            let text_y =
                absolute_y.saturating_add(control.height.saturating_sub(line_height) / 2);
            let available_width = control.width.saturating_sub(CONTROL_PADDING_X * 2);

            let render_value = focused
                .map(|focused| focused.editor.text().to_string())
                .or_else(|| page_control_values.get(&control.id).cloned())
                .unwrap_or_else(|| control.value.clone());
            let show_placeholder = render_value.is_empty()
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
                fonts.draw_text(
                    buffer,
                    width,
                    height,
                    absolute_x.saturating_add(CONTROL_PADDING_X),
                    text_y,
                    &placeholder,
                    control.font_size_px,
                    COLOR_CONTROL_PLACEHOLDER,
                    false,
                    false,
                    control.font_family,
                );
            } else {
                let mut editor = focused
                    .map(|focused| focused.editor.clone())
                    .unwrap_or_else(|| AddressBarState::new(render_value));
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
                        COLOR_CONTROL_SELECTION,
                    );
                }
                fonts.draw_text(
                    buffer,
                    width,
                    height,
                    absolute_x.saturating_add(CONTROL_PADDING_X),
                    text_y,
                    &view.text,
                    control.font_size_px,
                    control.text_color,
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
            let text_y =
                absolute_y.saturating_add(control.height.saturating_sub(line_height) / 2);
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
                control.font_family,
            );
        }
    }
}

fn paint_layout(
    document: &DocumentView,
    fonts: &mut FontContext,
    buffer: &mut [u32],
    width: u32,
    height: u32,
    offset_x: u32,
    offset_y: u32,
    viewport_height: u32,
    scroll_y: u32,
    layout: &LayoutDocument,
    page_control_values: &BTreeMap<usize, String>,
    focused_page_input: Option<&FocusedPageInput>,
    hovered_target: HitTarget,
) {
    let viewport_bottom = scroll_y.saturating_add(viewport_height);

    for rect in &layout.rects {
        let rect_bottom = rect.y.saturating_add(rect.height);
        if rect_bottom < scroll_y || rect.y > viewport_bottom {
            continue;
        }

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

    if let DocumentContent::Loaded(page) = &document.content {
        for image in &layout.images {
            let image_bottom = image.y.saturating_add(image.height);
            if image_bottom < scroll_y || image.y > viewport_bottom {
                continue;
            }

            let Some(decoded) = page.images.get(&image.src) else {
                continue;
            };

            draw_scaled_image(
                buffer,
                width,
                height,
                offset_x.saturating_add(image.x),
                offset_y.saturating_add(image.y.saturating_sub(scroll_y)),
                image.width,
                image.height,
                decoded,
            );
        }
    }

    for text in &layout.texts {
        let text_bottom = text
            .y
            .saturating_add(fonts.line_height_px(text.font_size_px, text.font_family));
        if text_bottom < scroll_y || text.y > viewport_bottom {
            continue;
        }

        fonts.draw_text(
            buffer,
            width,
            height,
            offset_x.saturating_add(text.x),
            offset_y.saturating_add(text.y.saturating_sub(scroll_y)),
            &text.text,
            text.font_size_px,
            text.color,
            text.bold,
            text.underline,
            text.font_family,
        );
    }

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
            page_control_values,
            focused_page_input,
            hovered_target,
        );
    }
}

fn max_scroll(viewport_height: u32, content_height: u32) -> u32 {
    content_height.saturating_sub(viewport_height)
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

fn draw_scaled_image(
    buffer: &mut [u32],
    width: u32,
    height: u32,
    x: u32,
    y: u32,
    draw_width: u32,
    draw_height: u32,
    image: &DecodedImage,
) {
    if draw_width == 0 || draw_height == 0 || image.width == 0 || image.height == 0 {
        return;
    }

    let max_x = x.saturating_add(draw_width).min(width);
    let max_y = y.saturating_add(draw_height).min(height);

    for dest_y in y..max_y {
        let source_y = ((dest_y - y) as u64 * image.height as u64 / draw_height as u64) as u32;
        let row_offset = dest_y as usize * width as usize;

        for dest_x in x..max_x {
            let source_x = ((dest_x - x) as u64 * image.width as u64 / draw_width as u64) as u32;
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
        AddressBarState, build_get_form_submission_url, cursor_index_for_address_x,
        layout_error_document,
        looks_like_local_address, max_scroll, parse_address_input,
    };
    use crate::font::FontContext;

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

        assert!(layout.texts.len() >= 2);
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
