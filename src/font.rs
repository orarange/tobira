use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

use font8x8::{
    BASIC_FONTS, BLOCK_FONTS, BOX_FONTS, GREEK_FONTS, HIRAGANA_FONTS, LATIN_FONTS, MISC_FONTS,
    UnicodeFonts,
};
use fontdue::{Font, FontSettings};
use unicode_width::UnicodeWidthChar;

use crate::css::{Color, FontFamilyKind};

const MIN_ADVANCE_PX: u32 = 4;

const WINDOWS_SANS_FONT_FILES: &[&str] = &["segoeui.ttf", "YuGothR.ttc", "meiryo.ttc", "arial.ttf"];

const WINDOWS_MONOSPACE_FONT_FILES: &[&str] = &[
    "consola.ttf",
    "CascadiaMono.ttf",
    "msgothic.ttc",
    "cour.ttf",
];

const WINDOWS_SERIF_FONT_FILES: &[&str] = &["georgia.ttf", "times.ttf", "timesbd.ttf"];

const UNIX_SANS_FONT_PATHS: &[&str] = &[
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
    "/Library/Fonts/Arial Unicode.ttf",
    "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
];

const UNIX_MONOSPACE_FONT_PATHS: &[&str] = &[
    "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
    "/usr/share/fonts/truetype/liberation2/LiberationMono-Regular.ttf",
    "/Library/Fonts/Courier New.ttf",
    "/System/Library/Fonts/Supplemental/Courier New.ttf",
];

const UNIX_SERIF_FONT_PATHS: &[&str] = &[
    "/usr/share/fonts/truetype/dejavu/DejaVuSerif.ttf",
    "/usr/share/fonts/truetype/liberation2/LiberationSerif-Regular.ttf",
    "/Library/Fonts/Times New Roman.ttf",
];

pub struct FontContext {
    sans_fonts: Vec<Font>,
    monospace_fonts: Vec<Font>,
    serif_fonts: Vec<Font>,
    sans_pending: VecDeque<PathBuf>,
    monospace_pending: VecDeque<PathBuf>,
    serif_pending: VecDeque<PathBuf>,
    glyph_cache: HashMap<GlyphKey, CachedGlyph>,
    line_metrics_cache: HashMap<(FontFamilyKind, u32), CachedLineMetrics>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct GlyphKey {
    character: char,
    font_size_px: u32,
    font_family: FontFamilyKind,
}

#[derive(Debug, Clone)]
struct CachedGlyph {
    advance_px: u32,
    ascent_px: i32,
    mode: GlyphMode,
}

#[derive(Debug, Clone)]
enum GlyphMode {
    Vector {
        width: u32,
        height: u32,
        xmin: i32,
        ymin: i32,
        bitmap: Vec<u8>,
    },
    Bitmap {
        glyph: [u8; 8],
        scale: u32,
    },
}

#[derive(Debug, Clone, Copy)]
struct CachedLineMetrics {
    ascent_px: i32,
}

impl FontContext {
    pub fn load() -> Self {
        let mut sans_pending = VecDeque::from(font_candidates(FontFamilyKind::Sans));
        let sans_fonts = load_initial_fonts(&mut sans_pending, 1);
        let mut monospace_pending = VecDeque::from(font_candidates(FontFamilyKind::Monospace));
        let mut monospace_fonts = load_initial_fonts(&mut monospace_pending, 1);
        let mut serif_pending = VecDeque::from(font_candidates(FontFamilyKind::Serif));
        let mut serif_fonts = load_initial_fonts(&mut serif_pending, 1);

        if monospace_fonts.is_empty() {
            monospace_fonts = sans_fonts.clone();
        }
        if serif_fonts.is_empty() {
            serif_fonts = sans_fonts.clone();
        }

        Self {
            sans_fonts,
            monospace_fonts,
            serif_fonts,
            sans_pending,
            monospace_pending,
            serif_pending,
            glyph_cache: HashMap::new(),
            line_metrics_cache: HashMap::new(),
        }
    }

    pub fn draw_text(
        &mut self,
        buffer: &mut [u32],
        width: u32,
        height: u32,
        x: u32,
        y: u32,
        text: &str,
        font_size_px: u32,
        color: Color,
        bold: bool,
        underline: bool,
        line_through: bool,
        font_family: FontFamilyKind,
    ) {
        self.draw_text_i32(
            buffer,
            width,
            height,
            x as i32,
            y as i32,
            text,
            font_size_px,
            color,
            bold,
            underline,
            line_through,
            font_family,
            i32::MIN,
        );
    }

    /// Same as [`draw_text`] but accepts a signed top-y so callers can draw text
    /// that straddles the top edge of the viewport (negative y). Glyph rows above
    /// the buffer are clipped by `draw_cached_glyph`/`blend_pixel`. This is required
    /// for correct scrolling: a line whose top is above the viewport must be drawn
    /// shifted up (partially clipped), NOT clamped to y=0 — clamping pins the line at
    /// the top edge so following lines collide with it, producing the "content
    /// crushed/ghosted toward the top while scrolling" artifact.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_text_i32(
        &mut self,
        buffer: &mut [u32],
        width: u32,
        height: u32,
        x: i32,
        y: i32,
        text: &str,
        font_size_px: u32,
        color: Color,
        bold: bool,
        underline: bool,
        line_through: bool,
        font_family: FontFamilyKind,
        clip_top: i32,
    ) {
        let mut cursor_x = x;

        for character in text.chars() {
            if character == '\n' {
                continue;
            }

            let glyph = self.cached_glyph(character, font_size_px, font_family);
            draw_cached_glyph(buffer, width, height, cursor_x, y, glyph, color, clip_top);

            if bold {
                draw_cached_glyph(buffer, width, height, cursor_x + 1, y, glyph, color, clip_top);
            }

            cursor_x = cursor_x.saturating_add(glyph.advance_px as i32);
        }

        if underline && !text.is_empty() {
            let underline_y = y
                .saturating_add(font_size_px as i32)
                .saturating_add((font_size_px / 10).max(1) as i32);
            if underline_y >= 0 {
                draw_rect(
                    buffer,
                    width,
                    height,
                    x.max(0) as u32,
                    underline_y as u32,
                    self.text_width_px(text, font_size_px, font_family),
                    (font_size_px / 12).max(1),
                    color,
                );
            }
        }

        if line_through && !text.is_empty() {
            let line_through_y = y.saturating_add((font_size_px * 55 / 100) as i32);
            if line_through_y >= 0 {
                draw_rect(
                    buffer,
                    width,
                    height,
                    x.max(0) as u32,
                    line_through_y as u32,
                    self.text_width_px(text, font_size_px, font_family),
                    (font_size_px / 12).max(1),
                    color,
                );
            }
        }
    }

    pub fn glyph_advance_px(
        &mut self,
        character: char,
        font_size_px: u32,
        font_family: FontFamilyKind,
    ) -> u32 {
        self.cached_glyph(character, font_size_px, font_family)
            .advance_px
    }

    pub fn text_width_px(
        &mut self,
        text: &str,
        font_size_px: u32,
        font_family: FontFamilyKind,
    ) -> u32 {
        text.chars()
            .map(|character| self.glyph_advance_px(character, font_size_px, font_family))
            .sum()
    }

    pub fn line_height_px(&mut self, font_size_px: u32, font_family: FontFamilyKind) -> u32 {
        let ascent = self
            .line_metrics(font_size_px, font_family)
            .ascent_px
            .max(font_size_px as i32);
        let gap = (font_size_px / 3).max(4);
        ascent.max(0) as u32 + gap
    }

    fn line_metrics(
        &mut self,
        font_size_px: u32,
        font_family: FontFamilyKind,
    ) -> CachedLineMetrics {
        let key = (font_family, font_size_px);
        if let Some(metrics) = self.line_metrics_cache.get(&key) {
            return *metrics;
        }

        let metrics = self
            .fonts_for(font_family)
            .iter()
            .find_map(|font| {
                font.horizontal_line_metrics(font_size_px as f32)
                    .map(|line| CachedLineMetrics {
                        ascent_px: line.ascent.ceil() as i32,
                    })
            })
            .unwrap_or(CachedLineMetrics {
                ascent_px: font_size_px as i32,
            });

        self.line_metrics_cache.insert(key, metrics);
        metrics
    }

    fn cached_glyph(
        &mut self,
        character: char,
        font_size_px: u32,
        font_family: FontFamilyKind,
    ) -> &CachedGlyph {
        let key = GlyphKey {
            character,
            font_size_px,
            font_family,
        };

        if !self.glyph_cache.contains_key(&key) {
            let glyph = self.rasterize_glyph(character, font_size_px, font_family);
            self.glyph_cache.insert(key, glyph);
        }

        self.glyph_cache
            .get(&key)
            .expect("glyph should be present after insertion")
    }

    fn rasterize_glyph(
        &mut self,
        character: char,
        font_size_px: u32,
        font_family: FontFamilyKind,
    ) -> CachedGlyph {
        self.ensure_font_for(character, font_family);

        let fallback_advance = estimated_glyph_advance_px(character, font_size_px, font_family);
        let ascent_px = self.line_metrics(font_size_px, font_family).ascent_px;

        for font in self.fonts_for(font_family) {
            if !font.has_glyph(character) {
                continue;
            }

            let (metrics, bitmap) = font.rasterize(character, font_size_px as f32);
            let advance_px = if metrics.advance_width > 0.0 {
                metrics.advance_width.ceil() as u32
            } else {
                fallback_advance
            }
            .max(MIN_ADVANCE_PX);
            if metrics.width == 0 || metrics.height == 0 {
                return CachedGlyph {
                    advance_px,
                    ascent_px,
                    mode: GlyphMode::Vector {
                        width: 0,
                        height: 0,
                        xmin: 0,
                        ymin: 0,
                        bitmap,
                    },
                };
            }

            return CachedGlyph {
                advance_px,
                ascent_px,
                mode: GlyphMode::Vector {
                    width: metrics.width as u32,
                    height: metrics.height as u32,
                    xmin: metrics.xmin,
                    ymin: metrics.ymin,
                    bitmap,
                },
            };
        }

        let scale = ((font_size_px + 7) / 8).max(1);
        let glyph = lookup_bitmap_glyph(character).unwrap_or_else(|| {
            lookup_bitmap_glyph('?').unwrap_or([
                0b00111100, 0b01000010, 0b00000100, 0b00001000, 0b00010000, 0, 0b00010000, 0,
            ])
        });

        CachedGlyph {
            advance_px: fallback_advance,
            ascent_px,
            mode: GlyphMode::Bitmap { glyph, scale },
        }
    }

    fn fonts_for(&self, font_family: FontFamilyKind) -> &[Font] {
        match font_family {
            FontFamilyKind::Sans => &self.sans_fonts,
            FontFamilyKind::Serif => &self.serif_fonts,
            FontFamilyKind::Monospace => &self.monospace_fonts,
        }
    }

    fn ensure_font_for(&mut self, character: char, font_family: FontFamilyKind) {
        let (fonts, pending) = match font_family {
            FontFamilyKind::Sans => (&mut self.sans_fonts, &mut self.sans_pending),
            FontFamilyKind::Serif => (&mut self.serif_fonts, &mut self.serif_pending),
            FontFamilyKind::Monospace => (&mut self.monospace_fonts, &mut self.monospace_pending),
        };

        if fonts.iter().any(|font| font.has_glyph(character)) {
            return;
        }

        while let Some(path) = pending.pop_front() {
            let Some(font) = load_font_file(&path) else {
                continue;
            };
            let supports_glyph = font.has_glyph(character);
            fonts.push(font);
            if supports_glyph {
                break;
            }
        }
    }
}

#[cfg(test)]
pub fn estimated_text_width_px(text: &str, font_size_px: u32, font_family: FontFamilyKind) -> u32 {
    text.chars()
        .map(|character| estimated_glyph_advance_px(character, font_size_px, font_family))
        .sum()
}

pub fn estimated_glyph_advance_px(
    character: char,
    font_size_px: u32,
    font_family: FontFamilyKind,
) -> u32 {
    let base = match font_family {
        FontFamilyKind::Sans => ((font_size_px as f32) * 0.56).round() as u32,
        FontFamilyKind::Serif => ((font_size_px as f32) * 0.56).round() as u32,
        FontFamilyKind::Monospace => ((font_size_px as f32) * 0.62).round() as u32,
    }
    .max(MIN_ADVANCE_PX);

    match character {
        ' ' => (base / 2).max(3),
        '\t' => base.saturating_mul(4),
        _ => {
            let cells = UnicodeWidthChar::width(character).unwrap_or(1).max(1) as u32;
            base.saturating_mul(cells)
        }
    }
}

fn load_initial_fonts(paths: &mut VecDeque<PathBuf>, limit: usize) -> Vec<Font> {
    let mut fonts = Vec::new();
    while fonts.len() < limit {
        let Some(candidate) = paths.pop_front() else {
            break;
        };
        if let Some(font) = load_font_file(&candidate) {
            fonts.push(font);
        }
    }
    fonts
}

fn font_candidates(font_family: FontFamilyKind) -> Vec<PathBuf> {
    if cfg!(target_os = "windows") {
        let windows_root = std::env::var_os("WINDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("C:\\Windows"));
        let fonts_dir = windows_root.join("Fonts");
        let files = match font_family {
            FontFamilyKind::Sans => WINDOWS_SANS_FONT_FILES,
            FontFamilyKind::Serif => WINDOWS_SERIF_FONT_FILES,
            FontFamilyKind::Monospace => WINDOWS_MONOSPACE_FONT_FILES,
        };

        return files.iter().map(|file| fonts_dir.join(file)).collect();
    }

    let files = match font_family {
        FontFamilyKind::Sans => UNIX_SANS_FONT_PATHS,
        FontFamilyKind::Serif => UNIX_SERIF_FONT_PATHS,
        FontFamilyKind::Monospace => UNIX_MONOSPACE_FONT_PATHS,
    };

    files.iter().map(PathBuf::from).collect()
}

fn load_font_file(path: &Path) -> Option<Font> {
    if !path.is_file() {
        return None;
    }

    let bytes = fs::read(path).ok()?;
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if matches!(extension.as_str(), "ttc" | "otc") {
        for collection_index in 0..4 {
            if let Ok(font) = Font::from_bytes(
                bytes.clone(),
                FontSettings {
                    collection_index,
                    ..FontSettings::default()
                },
            ) {
                return Some(font);
            }
        }
        return None;
    }

    Font::from_bytes(bytes, FontSettings::default()).ok()
}

fn draw_cached_glyph(
    buffer: &mut [u32],
    width: u32,
    height: u32,
    x: i32,
    y: i32,
    glyph: &CachedGlyph,
    color: Color,
    clip_top: i32,
) {
    match &glyph.mode {
        GlyphMode::Vector {
            width: glyph_width,
            height: glyph_height,
            xmin,
            ymin,
            bitmap,
        } => {
            let baseline_y = y + glyph.ascent_px;
            let draw_y = baseline_y - *glyph_height as i32 - *ymin;
            let draw_x = x + *xmin;

            blend_bitmap(
                buffer,
                width,
                height,
                draw_x,
                draw_y,
                *glyph_width,
                *glyph_height,
                bitmap,
                color,
                clip_top,
            );
        }
        GlyphMode::Bitmap { glyph, scale } => {
            draw_bitmap_fallback(buffer, width, height, x, y, *glyph, *scale, color, clip_top);
        }
    }
}

fn blend_bitmap(
    buffer: &mut [u32],
    width: u32,
    height: u32,
    x: i32,
    y: i32,
    glyph_width: u32,
    glyph_height: u32,
    bitmap: &[u8],
    color: Color,
    clip_top: i32,
) {
    for row in 0..glyph_height {
        let py = y + row as i32;
        // Clip rows above the content viewport top so text straddling the top edge
        // doesn't bleed up into the chrome (address bar) area that was painted first.
        if py < clip_top {
            continue;
        }
        for column in 0..glyph_width {
            let alpha = bitmap[row as usize * glyph_width as usize + column as usize];
            if alpha == 0 {
                continue;
            }

            blend_pixel(
                buffer,
                width,
                height,
                x + column as i32,
                py,
                color,
                alpha,
            );
        }
    }
}

fn blend_pixel(
    buffer: &mut [u32],
    width: u32,
    height: u32,
    x: i32,
    y: i32,
    color: Color,
    alpha: u8,
) {
    if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
        return;
    }

    let index = y as usize * width as usize + x as usize;
    let background = buffer[index];
    let fg_r = ((color >> 16) & 0xFF) as u32;
    let fg_g = ((color >> 8) & 0xFF) as u32;
    let fg_b = (color & 0xFF) as u32;
    let bg_r = ((background >> 16) & 0xFF) as u32;
    let bg_g = ((background >> 8) & 0xFF) as u32;
    let bg_b = (background & 0xFF) as u32;
    let alpha = alpha as u32;
    let inverse = 255_u32.saturating_sub(alpha);

    let red = (fg_r * alpha + bg_r * inverse) / 255;
    let green = (fg_g * alpha + bg_g * inverse) / 255;
    let blue = (fg_b * alpha + bg_b * inverse) / 255;

    buffer[index] = (red << 16) | (green << 8) | blue;
}

fn draw_bitmap_fallback(
    buffer: &mut [u32],
    width: u32,
    height: u32,
    x: i32,
    y: i32,
    glyph: [u8; 8],
    scale: u32,
    color: Color,
    clip_top: i32,
) {
    for (row_index, row) in glyph.into_iter().enumerate() {
        for column in 0..8 {
            if ((row >> column) & 1) == 0 {
                continue;
            }

            let draw_x = x + (column * scale) as i32;
            let draw_y = y + (row_index as u32 * scale) as i32;

            for offset_y in 0..scale {
                let py = draw_y + offset_y as i32;
                if py < clip_top {
                    continue;
                }
                for offset_x in 0..scale {
                    blend_pixel(
                        buffer,
                        width,
                        height,
                        draw_x + offset_x as i32,
                        py,
                        color,
                        255,
                    );
                }
            }
        }
    }
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

fn lookup_bitmap_glyph(character: char) -> Option<[u8; 8]> {
    BASIC_FONTS
        .get(character)
        .or_else(|| LATIN_FONTS.get(character))
        .or_else(|| GREEK_FONTS.get(character))
        .or_else(|| BOX_FONTS.get(character))
        .or_else(|| BLOCK_FONTS.get(character))
        .or_else(|| HIRAGANA_FONTS.get(character))
        .or_else(|| MISC_FONTS.get(character))
}

#[cfg(test)]
mod tests {
    use super::{FontContext, estimated_glyph_advance_px, estimated_text_width_px};
    use crate::css::FontFamilyKind;

    #[test]
    fn wide_characters_take_more_space() {
        let latin = estimated_glyph_advance_px('A', 20, FontFamilyKind::Sans);
        let wide = estimated_glyph_advance_px('漢', 20, FontFamilyKind::Sans);

        assert!(wide >= latin * 2);
    }

    #[test]
    fn text_width_adds_character_advances() {
        let width = estimated_text_width_px("Hi", 16, FontFamilyKind::Sans);
        assert!(width >= 16);
    }

    #[test]
    fn font_context_can_draw_text_without_panicking() {
        let mut context = FontContext::load();
        let mut buffer = vec![0_u32; 200 * 80];

        context.draw_text(
            &mut buffer,
            200,
            80,
            8,
            8,
            "Hello",
            18,
            0x00112233,
            false,
            false,
            false,
            FontFamilyKind::Sans,
        );

        assert!(buffer.iter().any(|pixel| *pixel != 0));
    }
}
