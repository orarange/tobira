use std::num::NonZeroU32;
use std::rc::Rc;

use font8x8::{
    BASIC_FONTS, BLOCK_FONTS, BOX_FONTS, GREEK_FONTS, HIRAGANA_FONTS, LATIN_FONTS, MISC_FONTS,
    UnicodeFonts,
};
use softbuffer::{Context, Surface};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, OwnedDisplayHandle};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::Window;

use crate::browser::{BrowserPage, load_page};
use crate::css::{Color, DEFAULT_BACKGROUND_COLOR, DEFAULT_TEXT_COLOR};
use crate::error::{BrowserError, Result};
use crate::layout::{LayoutDocument, TextCommand, layout_styled_document};
use crate::url::Url;

const WINDOW_WIDTH: u32 = 1100;
const WINDOW_HEIGHT: u32 = 760;
const FRAME_PADDING: u32 = 18;
const TITLE_SCALE: u32 = 2;
const FONT_WIDTH: u32 = 8;
const FONT_HEIGHT: u32 = 8;
const TITLE_HEIGHT: u32 = FONT_HEIGHT * TITLE_SCALE;
const HEADER_HEIGHT: u32 = 92;

const COLOR_WINDOW_BACKGROUND: Color = 0xE7E0D4;
const COLOR_HEADER: Color = 0x1F3A5F;
const COLOR_HEADER_TEXT: Color = 0xF6F7FB;
const COLOR_ACCENT: Color = 0xE6A53A;
const COLOR_ERROR: Color = 0x8C2F39;

type WindowHandle = Rc<Window>;
type SurfaceHandle = Surface<OwnedDisplayHandle, WindowHandle>;

pub fn run(initial_url: Url) -> Result<()> {
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
    current_url: Url,
    document: DocumentView,
    context: Context<OwnedDisplayHandle>,
    window: Option<WindowHandle>,
    surface: Option<SurfaceHandle>,
    scroll_y: u32,
}

impl BrowserApp {
    fn new(initial_url: Url, context: Context<OwnedDisplayHandle>) -> Self {
        let document = DocumentView::load(initial_url.clone());

        Self {
            current_url: document.url.clone(),
            document,
            context,
            window: None,
            surface: None,
            scroll_y: 0,
        }
    }

    fn reload(&mut self) {
        self.document = DocumentView::load(self.current_url.clone());
        self.current_url = self.document.url.clone();
        self.scroll_y = 0;
        self.sync_window_title();
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

        let body_top = HEADER_HEIGHT + FRAME_PADDING;
        let content_width = size.width.saturating_sub(FRAME_PADDING * 2).max(1);
        let viewport_height = size.height.saturating_sub(body_top + FRAME_PADDING).max(1);
        let layout = self.document.layout(content_width);
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
            layout.background_color,
        );
        paint_header(
            &mut buffer,
            size.width,
            size.height,
            &self.document,
            self.scroll_y,
            max_scroll_y,
        );
        paint_layout(
            &mut buffer,
            size.width,
            size.height,
            FRAME_PADDING,
            body_top,
            viewport_height,
            self.scroll_y,
            &layout,
        );

        buffer
            .present()
            .map_err(|error| BrowserError::message(error.to_string()))
    }

    fn content_metrics(&self, window_size: PhysicalSize<u32>) -> (u32, u32) {
        let body_top = HEADER_HEIGHT + FRAME_PADDING;
        let content_width = window_size.width.saturating_sub(FRAME_PADDING * 2).max(1);
        let viewport_height = window_size
            .height
            .saturating_sub(body_top + FRAME_PADDING)
            .max(1);
        let content_height = self.document.layout(content_width).content_height;

        (viewport_height, content_height)
    }

    fn handle_key(&mut self, key_code: KeyCode, window_size: PhysicalSize<u32>) -> bool {
        let (viewport_height, content_height) = self.content_metrics(window_size);
        match key_code {
            KeyCode::Escape => return true,
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
            KeyCode::KeyR => self.reload(),
            _ => return false,
        }

        self.request_redraw();
        false
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
                    .with_inner_size(LogicalSize::new(WINDOW_WIDTH as f64, WINDOW_HEIGHT as f64))
                    .with_min_inner_size(LogicalSize::new(720.0, 480.0)),
            )
            .expect("window creation should succeed");

        let window = Rc::new(window);
        let surface =
            Surface::new(&self.context, window.clone()).expect("surface creation should succeed");

        self.surface = Some(surface);
        self.window = Some(window);
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
                    self.document = DocumentView::error(
                        self.current_url.clone(),
                        format!("drawing failed: {error}"),
                    );
                    self.current_url = self.document.url.clone();
                    self.scroll_y = 0;
                    self.sync_window_title();
                    let _ = self.draw();
                }
            }
            WindowEvent::Resized(_) => self.request_redraw(),
            WindowEvent::MouseWheel { delta, .. } => self.handle_wheel(delta, window.inner_size()),
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed && !event.repeat =>
            {
                if let PhysicalKey::Code(key_code) = event.physical_key {
                    if self.handle_key(key_code, window.inner_size()) {
                        event_loop.exit();
                    }
                }
            }
            _ => {}
        }
    }
}

#[derive(Debug, Clone)]
struct DocumentView {
    url: Url,
    title: String,
    status_line: String,
    subtitle: String,
    content: DocumentContent,
}

#[derive(Debug, Clone)]
enum DocumentContent {
    Loaded(BrowserPage),
    Error(ErrorDocument),
}

#[derive(Debug, Clone)]
struct ErrorDocument {
    lines: Vec<String>,
}

impl DocumentView {
    fn load(url: Url) -> Self {
        match load_page(&url) {
            Ok(page) => Self::from_page(page),
            Err(error) => Self::error(url, error.to_string()),
        }
    }

    fn from_page(page: BrowserPage) -> Self {
        let content_type = page
            .content_type
            .clone()
            .unwrap_or_else(|| "unknown".to_string());

        Self {
            url: page.url.clone(),
            title: page.title.clone(),
            status_line: format!("Status: {}", page.status_text()),
            subtitle: format!("{} | {}", page.url, content_type),
            content: DocumentContent::Loaded(page),
        }
    }

    fn error(url: Url, message: impl Into<String>) -> Self {
        Self {
            url,
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
                    "- CSS support now works, but networking is still http:// only".to_string(),
                    "- https:// is still the next big milestone".to_string(),
                    "- press R to try the same URL again".to_string(),
                ],
            }),
        }
    }

    fn window_title(&self) -> String {
        format!("Scratch Browser - {}", self.title)
    }

    fn is_error(&self) -> bool {
        matches!(self.content, DocumentContent::Error(_))
    }

    fn layout(&self, width: u32) -> LayoutDocument {
        match &self.content {
            DocumentContent::Loaded(page) => layout_styled_document(&page.styled_document, width),
            DocumentContent::Error(error) => layout_error_document(error, width),
        }
    }
}

fn layout_error_document(document: &ErrorDocument, _width: u32) -> LayoutDocument {
    let mut texts = Vec::new();
    let mut cursor_y: u32 = 0;

    for line in &document.lines {
        let scale = if line.starts_with('#') { 3 } else { 2 };
        let color = if line.starts_with('#') {
            COLOR_ERROR
        } else {
            DEFAULT_TEXT_COLOR
        };
        let height = FONT_HEIGHT * scale + 6;

        if line.is_empty() {
            cursor_y = cursor_y.saturating_add(height / 2);
            continue;
        }

        texts.push(TextCommand {
            x: 0,
            y: cursor_y,
            width: line.chars().count() as u32 * FONT_WIDTH * scale,
            text: line.clone(),
            scale,
            color,
            underline: false,
            bold: scale >= 3,
        });

        cursor_y = cursor_y.saturating_add(height);
    }

    LayoutDocument {
        background_color: DEFAULT_BACKGROUND_COLOR,
        content_height: cursor_y,
        rects: Vec::new(),
        texts,
    }
}

fn paint_background(buffer: &mut [u32], width: u32, height: u32, content_background: Color) {
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
    draw_rect(
        buffer,
        width,
        height,
        FRAME_PADDING / 2,
        HEADER_HEIGHT / 2,
        width.saturating_sub(FRAME_PADDING),
        height
            .saturating_sub(HEADER_HEIGHT / 2)
            .saturating_sub(FRAME_PADDING / 2),
        content_background,
    );
}

fn paint_header(
    buffer: &mut [u32],
    width: u32,
    height: u32,
    document: &DocumentView,
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
        HEADER_HEIGHT,
        COLOR_HEADER,
    );
    draw_rect(
        buffer,
        width,
        height,
        0,
        HEADER_HEIGHT - 5,
        width,
        5,
        COLOR_ACCENT,
    );

    draw_text(
        buffer,
        width,
        height,
        FRAME_PADDING,
        16,
        "SCRATCH BROWSER",
        TITLE_SCALE,
        COLOR_HEADER_TEXT,
        true,
        false,
    );
    draw_text(
        buffer,
        width,
        height,
        FRAME_PADDING,
        16 + TITLE_HEIGHT + 10,
        &document.status_line,
        1,
        if document.is_error() {
            COLOR_ACCENT
        } else {
            COLOR_HEADER_TEXT
        },
        false,
        false,
    );
    draw_text(
        buffer,
        width,
        height,
        FRAME_PADDING,
        16 + TITLE_HEIGHT + 28,
        &document.subtitle,
        1,
        COLOR_HEADER_TEXT,
        false,
        false,
    );

    let controls = format!(
        "Keys: R reload | Esc quit | scroll: {} / {} px",
        scroll_y, max_scroll_y
    );
    draw_text(
        buffer,
        width,
        height,
        FRAME_PADDING,
        HEADER_HEIGHT - 18,
        &controls,
        1,
        COLOR_HEADER_TEXT,
        false,
        false,
    );
}

fn paint_layout(
    buffer: &mut [u32],
    width: u32,
    height: u32,
    offset_x: u32,
    offset_y: u32,
    viewport_height: u32,
    scroll_y: u32,
    layout: &LayoutDocument,
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

    for text in &layout.texts {
        let text_bottom = text
            .y
            .saturating_add(FONT_HEIGHT * text.scale)
            .saturating_add(6);
        if text_bottom < scroll_y || text.y > viewport_bottom {
            continue;
        }

        draw_text(
            buffer,
            width,
            height,
            offset_x.saturating_add(text.x),
            offset_y.saturating_add(text.y.saturating_sub(scroll_y)),
            &text.text,
            text.scale,
            text.color,
            text.bold,
            text.underline,
        );
    }
}

fn max_scroll(viewport_height: u32, content_height: u32) -> u32 {
    content_height.saturating_sub(viewport_height)
}

fn draw_text(
    buffer: &mut [u32],
    width: u32,
    height: u32,
    x: u32,
    y: u32,
    text: &str,
    scale: u32,
    color: Color,
    bold: bool,
    underline: bool,
) {
    let mut cursor_x = x;
    let step = FONT_WIDTH * scale;

    for character in text.chars() {
        if character == '\n' {
            continue;
        }
        draw_glyph(buffer, width, height, cursor_x, y, character, scale, color);
        if bold {
            draw_glyph(
                buffer,
                width,
                height,
                cursor_x.saturating_add(1),
                y,
                character,
                scale,
                color,
            );
        }
        cursor_x = cursor_x.saturating_add(step);
    }

    if underline && !text.is_empty() {
        let underline_y = y
            .saturating_add(FONT_HEIGHT * scale)
            .saturating_add(scale / 2);
        draw_rect(
            buffer,
            width,
            height,
            x,
            underline_y,
            text.chars().count() as u32 * step,
            scale.max(1),
            color,
        );
    }
}

fn draw_glyph(
    buffer: &mut [u32],
    width: u32,
    height: u32,
    x: u32,
    y: u32,
    character: char,
    scale: u32,
    color: Color,
) {
    let glyph = lookup_glyph(character).unwrap_or_else(|| {
        lookup_glyph('?').unwrap_or([
            0b00111100, 0b01000010, 0b00000100, 0b00001000, 0b00010000, 0, 0b00010000, 0,
        ])
    });

    draw_bitmap_glyph(buffer, width, height, x, y, glyph, scale, color);
}

fn draw_bitmap_glyph(
    buffer: &mut [u32],
    width: u32,
    height: u32,
    x: u32,
    y: u32,
    glyph: [u8; 8],
    scale: u32,
    color: Color,
) {
    for (row_index, row) in glyph.into_iter().enumerate() {
        for column in 0..8 {
            let bit = (row >> column) & 1;
            if bit == 0 {
                continue;
            }

            let draw_x = x + (column * scale);
            let draw_y = y + (row_index as u32 * scale);

            for offset_y in 0..scale {
                for offset_x in 0..scale {
                    put_pixel(
                        buffer,
                        width,
                        height,
                        draw_x + offset_x,
                        draw_y + offset_y,
                        color,
                    );
                }
            }
        }
    }
}

fn lookup_glyph(character: char) -> Option<[u8; 8]> {
    BASIC_FONTS
        .get(character)
        .or_else(|| LATIN_FONTS.get(character))
        .or_else(|| GREEK_FONTS.get(character))
        .or_else(|| BOX_FONTS.get(character))
        .or_else(|| BLOCK_FONTS.get(character))
        .or_else(|| HIRAGANA_FONTS.get(character))
        .or_else(|| MISC_FONTS.get(character))
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

fn put_pixel(buffer: &mut [u32], width: u32, height: u32, x: u32, y: u32, color: Color) {
    if x >= width || y >= height {
        return;
    }

    buffer[y as usize * width as usize + x as usize] = color;
}

#[cfg(test)]
mod tests {
    use super::{draw_bitmap_glyph, layout_error_document, max_scroll};

    #[test]
    fn glyph_bits_draw_left_to_right() {
        let mut buffer = vec![0_u32; 8 * 8];

        draw_bitmap_glyph(
            &mut buffer,
            8,
            8,
            0,
            0,
            [0b0000_0001, 0, 0, 0, 0, 0, 0, 0],
            1,
            0x00FF_FFFF,
        );

        assert_eq!(buffer[0], 0x00FF_FFFF);
        assert_eq!(buffer[7], 0);
    }

    #[test]
    fn max_scroll_stops_at_zero() {
        assert_eq!(max_scroll(400, 100), 0);
        assert_eq!(max_scroll(100, 400), 300);
    }

    #[test]
    fn error_layout_contains_text_commands() {
        let layout = layout_error_document(
            &super::ErrorDocument {
                lines: vec!["# Oops".to_string(), "hello".to_string()],
            },
            320,
        );

        assert!(layout.texts.len() >= 2);
    }
}
