use std::num::NonZeroU32;
use std::rc::Rc;

use softbuffer::{Context, Surface};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, OwnedDisplayHandle};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::Window;

use crate::browser::{BrowserPage, load_page};
use crate::css::{Color, DEFAULT_BACKGROUND_COLOR, DEFAULT_TEXT_COLOR, FontFamilyKind};
use crate::error::{BrowserError, Result};
use crate::font::FontContext;
use crate::image::DecodedImage;
use crate::layout::{LayoutDocument, TextCommand, layout_styled_document};
use crate::url::Url;

const WINDOW_WIDTH: u32 = 1100;
const WINDOW_HEIGHT: u32 = 760;
const FRAME_PADDING: u32 = 18;
const TITLE_FONT_SIZE: u32 = 28;
const HEADER_FONT_SIZE: u32 = 14;
const HEADER_TOP_PADDING: u32 = 14;
const HEADER_BOTTOM_PADDING: u32 = 14;
const HEADER_TITLE_GAP: u32 = 6;
const HEADER_LINE_GAP: u32 = 3;
const HEADER_BORDER_HEIGHT: u32 = 5;

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
    fonts: FontContext,
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
            fonts: FontContext::load(),
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

        let header = header_layout_metrics(&mut self.fonts);
        let body_top = header.height + FRAME_PADDING;
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
            header.height,
            layout.background_color,
        );
        paint_header(
            &mut self.fonts,
            &mut buffer,
            size.width,
            size.height,
            &header,
            &self.document,
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
        );

        buffer
            .present()
            .map_err(|error| BrowserError::message(error.to_string()))
    }

    fn content_metrics(&mut self, window_size: PhysicalSize<u32>) -> (u32, u32) {
        let header = header_layout_metrics(&mut self.fonts);
        let body_top = header.height + FRAME_PADDING;
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

    fn layout(&self, width: u32, fonts: &mut FontContext) -> LayoutDocument {
        match &self.content {
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
        });

        cursor_y = cursor_y.saturating_add(height);
    }

    LayoutDocument {
        background_color: DEFAULT_BACKGROUND_COLOR,
        content_height: cursor_y,
        rects: Vec::new(),
        texts,
        images: Vec::new(),
    }
}

#[derive(Debug, Clone, Copy)]
struct HeaderLayoutMetrics {
    height: u32,
    title_y: u32,
    status_y: u32,
    subtitle_y: u32,
    controls_y: u32,
}

fn header_layout_metrics(fonts: &mut FontContext) -> HeaderLayoutMetrics {
    let title_height = fonts.line_height_px(TITLE_FONT_SIZE, FontFamilyKind::Sans);
    let header_line_height = fonts.line_height_px(HEADER_FONT_SIZE, FontFamilyKind::Sans);
    let title_y = HEADER_TOP_PADDING;
    let status_y = title_y.saturating_add(title_height + HEADER_TITLE_GAP);
    let subtitle_y = status_y.saturating_add(header_line_height + HEADER_LINE_GAP);
    let controls_y = subtitle_y.saturating_add(header_line_height + HEADER_LINE_GAP);
    let height = controls_y
        .saturating_add(header_line_height + HEADER_BOTTOM_PADDING + HEADER_BORDER_HEIGHT);

    HeaderLayoutMetrics {
        height,
        title_y,
        status_y,
        subtitle_y,
        controls_y,
    }
}

fn paint_background(
    buffer: &mut [u32],
    width: u32,
    height: u32,
    header_height: u32,
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
    draw_rect(
        buffer,
        width,
        height,
        FRAME_PADDING / 2,
        header_height / 2,
        width.saturating_sub(FRAME_PADDING),
        height
            .saturating_sub(header_height / 2)
            .saturating_sub(FRAME_PADDING / 2),
        content_background,
    );
}

fn paint_header(
    fonts: &mut FontContext,
    buffer: &mut [u32],
    width: u32,
    height: u32,
    header: &HeaderLayoutMetrics,
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
        header.height,
        COLOR_HEADER,
    );
    draw_rect(
        buffer,
        width,
        height,
        0,
        header.height.saturating_sub(HEADER_BORDER_HEIGHT),
        width,
        HEADER_BORDER_HEIGHT,
        COLOR_ACCENT,
    );

    fonts.draw_text(
        buffer,
        width,
        height,
        FRAME_PADDING,
        header.title_y,
        "SCRATCH BROWSER",
        TITLE_FONT_SIZE,
        COLOR_HEADER_TEXT,
        true,
        false,
        FontFamilyKind::Sans,
    );
    fonts.draw_text(
        buffer,
        width,
        height,
        FRAME_PADDING,
        header.status_y,
        &document.status_line,
        HEADER_FONT_SIZE,
        if document.is_error() {
            COLOR_ACCENT
        } else {
            COLOR_HEADER_TEXT
        },
        false,
        false,
        FontFamilyKind::Sans,
    );
    fonts.draw_text(
        buffer,
        width,
        height,
        FRAME_PADDING,
        header.subtitle_y,
        &document.subtitle,
        HEADER_FONT_SIZE,
        COLOR_HEADER_TEXT,
        false,
        false,
        FontFamilyKind::Sans,
    );

    let controls = format!(
        "Keys: R reload | Esc quit | scroll: {} / {} px",
        scroll_y, max_scroll_y
    );
    fonts.draw_text(
        buffer,
        width,
        height,
        FRAME_PADDING,
        header.controls_y,
        &controls,
        HEADER_FONT_SIZE,
        COLOR_HEADER_TEXT,
        false,
        false,
        FontFamilyKind::Sans,
    );
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
}

fn max_scroll(viewport_height: u32, content_height: u32) -> u32 {
    content_height.saturating_sub(viewport_height)
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
    use super::{layout_error_document, max_scroll};
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
}
