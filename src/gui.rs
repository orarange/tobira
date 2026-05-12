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
use crate::error::{BrowserError, Result};
use crate::url::Url;

const WINDOW_WIDTH: u32 = 1100;
const WINDOW_HEIGHT: u32 = 760;
const FRAME_PADDING: u32 = 18;
const TITLE_SCALE: u32 = 2;
const BODY_SCALE: u32 = 2;
const FONT_WIDTH: u32 = 8;
const FONT_HEIGHT: u32 = 8;
const TITLE_HEIGHT: u32 = FONT_HEIGHT * TITLE_SCALE;
const BODY_LINE_HEIGHT: u32 = FONT_HEIGHT * BODY_SCALE + 6;
const HEADER_HEIGHT: u32 = 92;

const COLOR_BACKGROUND: u32 = 0xF5F1E8;
const COLOR_SURFACE: u32 = 0xFFFDF8;
const COLOR_HEADER: u32 = 0x1F3A5F;
const COLOR_TEXT: u32 = 0x1D232E;
const COLOR_HEADER_TEXT: u32 = 0xF6F7FB;
const COLOR_ACCENT: u32 = 0xE6A53A;
const COLOR_ERROR: u32 = 0x8C2F39;

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
    scroll_lines: usize,
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
            scroll_lines: 0,
        }
    }

    fn reload(&mut self) {
        self.document = DocumentView::load(self.current_url.clone());
        self.current_url = self.document.url.clone();
        self.scroll_lines = 0;
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

    fn scroll_by(&mut self, delta: isize, viewport_lines: usize) {
        let max_scroll = self.max_scroll(viewport_lines);
        if max_scroll == 0 {
            self.scroll_lines = 0;
            return;
        }

        let next = if delta.is_negative() {
            self.scroll_lines.saturating_sub(delta.unsigned_abs())
        } else {
            self.scroll_lines.saturating_add(delta as usize)
        };

        self.scroll_lines = next.min(max_scroll);
    }

    fn max_scroll(&self, viewport_lines: usize) -> usize {
        let total = self.document.body_lines.len();
        total.saturating_sub(viewport_lines)
    }

    fn visible_body_lines(&self, width: u32, height: u32) -> (Vec<String>, usize) {
        let body_top = HEADER_HEIGHT + FRAME_PADDING;
        let available_height = height.saturating_sub(body_top + FRAME_PADDING);
        let visible_lines = (available_height / BODY_LINE_HEIGHT).max(1) as usize;
        let max_chars = body_columns(width);
        let wrapped = wrap_lines(&self.document.body_lines, max_chars);
        let max_scroll = wrapped.len().saturating_sub(visible_lines);
        let scroll = self.scroll_lines.min(max_scroll);
        let end = (scroll + visible_lines).min(wrapped.len());

        (wrapped[scroll..end].to_vec(), visible_lines)
    }

    fn draw(&mut self) -> Result<()> {
        let Some(window) = &self.window else {
            return Ok(());
        };

        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return Ok(());
        }

        let (body_lines, visible_lines) = self.visible_body_lines(size.width, size.height);
        let max_scroll = self.max_scroll(visible_lines);
        self.scroll_lines = self.scroll_lines.min(max_scroll);

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
        paint_background(&mut buffer, size.width, size.height);
        paint_header(
            &mut buffer,
            size.width,
            size.height,
            &self.document,
            self.scroll_lines,
        );
        paint_body(
            &mut buffer,
            size.width,
            size.height,
            &body_lines,
            self.document.is_error,
        );

        buffer
            .present()
            .map_err(|error| BrowserError::message(error.to_string()))
    }

    fn handle_key(&mut self, key_code: KeyCode, window_size: PhysicalSize<u32>) -> bool {
        let viewport_lines = estimate_viewport_lines(window_size.height);
        match key_code {
            KeyCode::Escape => return true,
            KeyCode::ArrowDown => self.scroll_by(1, viewport_lines),
            KeyCode::ArrowUp => self.scroll_by(-1, viewport_lines),
            KeyCode::PageDown => self.scroll_by(viewport_lines as isize, viewport_lines),
            KeyCode::PageUp => self.scroll_by(-(viewport_lines as isize), viewport_lines),
            KeyCode::Home => self.scroll_lines = 0,
            KeyCode::End => self.scroll_lines = self.max_scroll(viewport_lines),
            KeyCode::KeyR => self.reload(),
            _ => return false,
        }

        self.request_redraw();
        false
    }

    fn handle_wheel(&mut self, delta: MouseScrollDelta, window_size: PhysicalSize<u32>) {
        let viewport_lines = estimate_viewport_lines(window_size.height);
        match delta {
            MouseScrollDelta::LineDelta(_, y) => {
                self.scroll_by(-(y.round() as isize), viewport_lines);
            }
            MouseScrollDelta::PixelDelta(position) => {
                let lines = (position.y / BODY_LINE_HEIGHT as f64).round() as isize;
                if lines != 0 {
                    self.scroll_by(-lines, viewport_lines);
                }
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
                    self.scroll_lines = 0;
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
    body_lines: Vec<String>,
    is_error: bool,
}

impl DocumentView {
    fn load(url: Url) -> Self {
        match load_page(&url) {
            Ok(page) => Self::from_page(page),
            Err(error) => Self::error(url, error.to_string()),
        }
    }

    fn from_page(page: BrowserPage) -> Self {
        let title = first_non_empty_line(page.body_text())
            .unwrap_or_else(|| "Scratch Browser".to_string())
            .trim_start_matches('#')
            .trim()
            .to_string();

        let status_line = format!("Status: {}", page.status_text());
        let content_type = page
            .content_type
            .clone()
            .unwrap_or_else(|| "unknown".to_string());
        let subtitle = format!("{} | {}", page.url, content_type);
        let body_lines = page
            .body_text()
            .lines()
            .map(|line| line.to_string())
            .collect::<Vec<_>>();

        Self {
            url: page.url,
            title,
            status_line,
            subtitle,
            body_lines,
            is_error: false,
        }
    }

    fn error(url: Url, message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            url,
            title: "Load Error".to_string(),
            status_line: "Status: request failed".to_string(),
            subtitle: "The browser core could not load this page.".to_string(),
            body_lines: vec![
                "# Load Error".to_string(),
                String::new(),
                message,
                String::new(),
                "Hints:".to_string(),
                "- only http:// is supported right now".to_string(),
                "- https:// is the next big milestone".to_string(),
                "- press R after you change the URL argument and restart".to_string(),
            ],
            is_error: true,
        }
    }

    fn window_title(&self) -> String {
        format!("Scratch Browser - {}", self.title)
    }
}

fn paint_background(buffer: &mut [u32], width: u32, height: u32) {
    draw_rect(buffer, width, height, 0, 0, width, height, COLOR_BACKGROUND);
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
        COLOR_SURFACE,
    );
}

fn paint_header(
    buffer: &mut [u32],
    width: u32,
    height: u32,
    document: &DocumentView,
    scroll_lines: usize,
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
    );
    draw_text(
        buffer,
        width,
        height,
        FRAME_PADDING,
        16 + TITLE_HEIGHT + 10,
        &document.status_line,
        1,
        if document.is_error {
            COLOR_ACCENT
        } else {
            COLOR_HEADER_TEXT
        },
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
    );

    let controls = format!("Keys: R reload | Esc quit | scroll lines: {scroll_lines}");
    draw_text(
        buffer,
        width,
        height,
        FRAME_PADDING,
        HEADER_HEIGHT - 18,
        &controls,
        1,
        COLOR_HEADER_TEXT,
    );
}

fn paint_body(buffer: &mut [u32], width: u32, height: u32, lines: &[String], is_error: bool) {
    let x = FRAME_PADDING;
    let mut y = HEADER_HEIGHT + FRAME_PADDING;
    let color = if is_error { COLOR_ERROR } else { COLOR_TEXT };

    for line in lines {
        draw_text(buffer, width, height, x, y, line, BODY_SCALE, color);
        y = y.saturating_add(BODY_LINE_HEIGHT);
    }
}

fn draw_text(
    buffer: &mut [u32],
    width: u32,
    height: u32,
    x: u32,
    y: u32,
    text: &str,
    scale: u32,
    color: u32,
) {
    let mut cursor_x = x;
    let step = FONT_WIDTH * scale;

    for character in text.chars() {
        if character == '\n' {
            continue;
        }
        draw_glyph(buffer, width, height, cursor_x, y, character, scale, color);
        cursor_x = cursor_x.saturating_add(step);
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
    color: u32,
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
    color: u32,
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
    color: u32,
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

fn put_pixel(buffer: &mut [u32], width: u32, height: u32, x: u32, y: u32, color: u32) {
    if x >= width || y >= height {
        return;
    }

    buffer[y as usize * width as usize + x as usize] = color;
}

fn body_columns(width: u32) -> usize {
    let available = width.saturating_sub(FRAME_PADDING * 2);
    (available / (FONT_WIDTH * BODY_SCALE)).max(1) as usize
}

fn estimate_viewport_lines(height: u32) -> usize {
    let body_top = HEADER_HEIGHT + FRAME_PADDING;
    let available_height = height.saturating_sub(body_top + FRAME_PADDING);
    (available_height / BODY_LINE_HEIGHT).max(1) as usize
}

fn first_non_empty_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

fn wrap_lines(lines: &[String], max_chars: usize) -> Vec<String> {
    let mut wrapped = Vec::new();

    for line in lines {
        if line.trim().is_empty() {
            wrapped.push(String::new());
            continue;
        }

        let mut current = String::new();
        for word in line.split_whitespace() {
            let word_len = word.chars().count();

            if current.is_empty() {
                append_or_split_word(word, max_chars, &mut current, &mut wrapped);
                continue;
            }

            let current_len = current.chars().count();
            if current_len + 1 + word_len <= max_chars {
                current.push(' ');
                current.push_str(word);
            } else {
                wrapped.push(current);
                current = String::new();
                append_or_split_word(word, max_chars, &mut current, &mut wrapped);
            }
        }

        if !current.is_empty() {
            wrapped.push(current);
        }
    }

    wrapped
}

fn append_or_split_word(
    word: &str,
    max_chars: usize,
    current: &mut String,
    wrapped: &mut Vec<String>,
) {
    if word.chars().count() <= max_chars {
        current.push_str(word);
        return;
    }

    let mut chunk = String::new();
    for character in word.chars() {
        chunk.push(character);
        if chunk.chars().count() == max_chars {
            wrapped.push(chunk);
            chunk = String::new();
        }
    }

    if !chunk.is_empty() {
        current.push_str(&chunk);
    }
}

#[cfg(test)]
mod tests {
    use super::{body_columns, draw_bitmap_glyph, wrap_lines};

    #[test]
    fn wraps_long_lines_at_word_boundaries() {
        let lines = vec!["alpha beta gamma delta".to_string()];
        let wrapped = wrap_lines(&lines, 10);

        assert_eq!(wrapped, vec!["alpha beta", "gamma", "delta"]);
    }

    #[test]
    fn splits_single_long_words() {
        let lines = vec!["supercalifragilistic".to_string()];
        let wrapped = wrap_lines(&lines, 6);

        assert_eq!(wrapped, vec!["superc", "alifra", "gilist", "ic"]);
    }

    #[test]
    fn body_column_count_never_drops_below_one() {
        assert_eq!(body_columns(0), 1);
    }

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
}
