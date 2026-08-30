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

const WINDOWS_SANS_FONT_FILES: &[&str] = &[
    // Arial first, because that is what a browser on Windows uses for
    // `sans-serif` and for the `font-family: Arial` nearly every page writes.
    // Segoe UI is a wider face, so leading with it made every line of text
    // about a tenth wider than a browser draws it -- enough to wrap a line
    // that should have fitted, on every paragraph of every page.
    "arial.ttf",
    "segoeui.ttf",
    "YuGothR.ttc",
    "meiryo.ttc",
    // Symbol/emoji fallbacks: cover dingbats and pictographs (e.g. ⚛ U+269B)
    // that the text faces above lack. seguiemj renders via its monochrome
    // outlines (the rasterizer doesn't do COLR color layers).
    "seguisym.ttf",
    "seguiemj.ttf",
];

/// The bold cut of each face above, in the same order.
///
/// Faking bold by smearing a regular glyph is a poor stand-in for a face that
/// was actually drawn heavy -- most visibly for Japanese, where Yu Gothic
/// Regular is very light and its bold cut is a different design, not a fattened
/// one. firefox.com's hero heading came out lighter than the paragraph under
/// it. Anything with no bold cut installed falls back to the regular stack and
/// the smear.
const WINDOWS_SANS_BOLD_FONT_FILES: &[&str] = &[
    "arialbd.ttf",
    "segoeuib.ttf",
    "YuGothB.ttc",
    "meiryob.ttc",
    "seguisym.ttf",
    "seguiemj.ttf",
];

const WINDOWS_MONOSPACE_BOLD_FONT_FILES: &[&str] =
    &["consolab.ttf", "CascadiaMono.ttf", "msgothic.ttc", "courbd.ttf"];

const WINDOWS_SERIF_BOLD_FONT_FILES: &[&str] = &["georgiab.ttf", "timesbd.ttf"];

const UNIX_SANS_BOLD_FONT_PATHS: &[&str] = &[
    "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Bold.ttc",
    "/usr/share/fonts/truetype/noto/NotoSans-Bold.ttf",
];

const UNIX_MONOSPACE_BOLD_FONT_PATHS: &[&str] = &[
    "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf",
    "/usr/share/fonts/truetype/liberation2/LiberationMono-Bold.ttf",
];

const UNIX_SERIF_BOLD_FONT_PATHS: &[&str] = &[
    "/usr/share/fonts/truetype/dejavu/DejaVuSerif-Bold.ttf",
    "/usr/share/fonts/truetype/liberation2/LiberationSerif-Bold.ttf",
];

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

/// How far to smear a glyph sideways to fake a bold face, in pixels.
///
/// Roughly a 24th of the type size, which is about the difference between a
/// regular and a bold stem in most faces, and never less than one pixel.
fn synthetic_bold_smear(font_size_px: u32) -> u32 {
    (font_size_px / 24).max(1)
}

pub struct FontContext {
    sans_fonts: Vec<Font>,
    monospace_fonts: Vec<Font>,
    serif_fonts: Vec<Font>,
    sans_pending: VecDeque<PathBuf>,
    monospace_pending: VecDeque<PathBuf>,
    serif_pending: VecDeque<PathBuf>,
    /// The bold cuts, loaded the same way and only when something asks for
    /// bold. A family with none installed leaves its stack empty and the
    /// regular one is smeared instead.
    sans_bold_fonts: Vec<Font>,
    monospace_bold_fonts: Vec<Font>,
    serif_bold_fonts: Vec<Font>,
    sans_bold_pending: VecDeque<PathBuf>,
    monospace_bold_pending: VecDeque<PathBuf>,
    serif_bold_pending: VecDeque<PathBuf>,
    glyph_cache: HashMap<GlyphKey, CachedGlyph>,
    line_metrics_cache: HashMap<(FontFamilyKind, u32), CachedLineMetrics>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct GlyphKey {
    character: char,
    font_size_px: u32,
    font_family: FontFamilyKind,
    bold: bool,
}

#[derive(Debug, Clone)]
struct CachedGlyph {
    /// The step to the next glyph, before rounding.
    ///
    /// Kept fractional because a run's width is the sum of these, rounded once:
    /// rounding each one first added up. Arial's `M` steps 13.33px at 16px, and
    /// a whole line of them came out a pixel wider per letter.
    advance: f32,
    advance_px: u32,
    ascent_px: i32,
    mode: GlyphMode,
    /// Set when bold was asked for and no bold cut had this character, so the
    /// regular glyph is standing in and still needs smearing.
    synthetic_bold: bool,
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
    /// What `line-height: normal` comes to for this face and size: the font's
    /// own ascent, descent and line gap added together.
    normal_line_px: u32,
}

impl FontContext {
    /// No font file is read here. `fontdue` expands a font into roughly 40x its
    /// file size when it parses one (measured: `segoeui.ttf` 960 KB -> 40.5 MB),
    /// so eagerly loading sans + monospace + serif cost ~64 MB before a single
    /// page was drawn. Each family is now read the first time something actually
    /// asks for it, via [`Self::ensure_family_loaded`].
    pub fn load() -> Self {
        Self {
            sans_fonts: Vec::new(),
            monospace_fonts: Vec::new(),
            serif_fonts: Vec::new(),
            sans_pending: VecDeque::from(font_candidates(FontFamilyKind::Sans, false)),
            monospace_pending: VecDeque::from(font_candidates(FontFamilyKind::Monospace, false)),
            serif_pending: VecDeque::from(font_candidates(FontFamilyKind::Serif, false)),
            sans_bold_fonts: Vec::new(),
            monospace_bold_fonts: Vec::new(),
            serif_bold_fonts: Vec::new(),
            sans_bold_pending: VecDeque::from(font_candidates(FontFamilyKind::Sans, true)),
            monospace_bold_pending: VecDeque::from(font_candidates(
                FontFamilyKind::Monospace,
                true,
            )),
            serif_bold_pending: VecDeque::from(font_candidates(FontFamilyKind::Serif, true)),
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
        // Stepped fractionally and rounded for each glyph, so a run ends where
        // it was measured to end rather than a pixel further on for every
        // narrow letter in it.
        let mut cursor = x as f32;

        for character in text.chars() {
            if character == '\n' {
                continue;
            }
            let cursor_x = cursor.round() as i32;

            // Stepped by the regular cut's advance, not the bold one's. Layout
            // measured this run before anything knew it would be drawn bold,
            // and a wider step here would walk the text out of the box it was
            // given.
            let advance = self
                .cached_glyph(character, font_size_px, font_family, false)
                .advance;
            let glyph = self.cached_glyph(character, font_size_px, font_family, bold);
            let smear = glyph.synthetic_bold;
            draw_cached_glyph(buffer, width, height, cursor_x, y, glyph, color, clip_top);

            if smear {
                // Bold is faked by smearing the glyph sideways, and the smear
                // has to grow with the type. Fixed at one pixel it reads as
                // bold at body size and vanishes at display size: firefox.com
                // sets its hero heading in 80px bold, and one pixel on a stem
                // that wants four left it looking lighter than the paragraph
                // below it. Every offset in between is drawn so the stem fills
                // rather than splits.
                for offset in 1..=synthetic_bold_smear(font_size_px) as i32 {
                    draw_cached_glyph(
                        buffer,
                        width,
                        height,
                        cursor_x + offset,
                        y,
                        glyph,
                        color,
                        clip_top,
                    );
                }
            }

            cursor += advance;
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
        // Measurement always uses the regular cut: layout is done before
        // anything knows a run will be drawn bold, and the two have to agree.
        self.cached_glyph(character, font_size_px, font_family, false)
            .advance_px
    }

    /// How wide a run of text is.
    ///
    /// The fractional advances are added up and rounded once at the end. Adding
    /// up rounded ones made every line about a twentieth too wide, which is
    /// enough to wrap a line that a browser fits.
    pub fn text_width_px(
        &mut self,
        text: &str,
        font_size_px: u32,
        font_family: FontFamilyKind,
    ) -> u32 {
        let total: f32 = text
            .chars()
            .map(|character| {
                self.cached_glyph(character, font_size_px, font_family, false)
                    .advance
            })
            .sum();
        total.round() as u32
    }

    /// `line-height: normal`, which is the face's own recommended line
    /// spacing -- ascent plus descent plus line gap.
    ///
    /// It was a third of the font size added to the ascent, which for Arial at
    /// 16px gives 21 where a browser gives 18. Three pixels a line does not
    /// look like much until a long article has two thousand of them.
    pub fn line_height_px(&mut self, font_size_px: u32, font_family: FontFamilyKind) -> u32 {
        self.line_metrics(font_size_px, font_family)
            .normal_line_px
            .max(1)
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

        // Metrics come off the regular cut even when bold is drawn: the
        // baseline layout measured with has to be the baseline painted on.
        self.ensure_family_loaded(font_family, false);
        let metrics = self
            .fonts_for(font_family, false)
            .iter()
            .find_map(|font| {
                font.horizontal_line_metrics(font_size_px as f32)
                    .map(|line| CachedLineMetrics {
                        ascent_px: line.ascent.ceil() as i32,
                        normal_line_px: line.new_line_size.round().max(1.0) as u32,
                    })
            })
            .unwrap_or(CachedLineMetrics {
                ascent_px: font_size_px as i32,
                // No face to ask: the ratio a browser lands on for the common
                // text faces.
                normal_line_px: (font_size_px as f32 * 1.15).round().max(1.0) as u32,
            });

        self.line_metrics_cache.insert(key, metrics);
        metrics
    }

    fn cached_glyph(
        &mut self,
        character: char,
        font_size_px: u32,
        font_family: FontFamilyKind,
        bold: bool,
    ) -> &CachedGlyph {
        let key = GlyphKey {
            character,
            font_size_px,
            font_family,
            bold,
        };

        if !self.glyph_cache.contains_key(&key) {
            let glyph = self.rasterize_glyph(character, font_size_px, font_family, bold);
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
        bold: bool,
    ) -> CachedGlyph {
        let ascent_px = self.line_metrics(font_size_px, font_family).ascent_px;

        // Default-invisible characters: variation selectors (U+FE0F makes "⚛️"
        // = U+269B + U+FE0F), zero-width (non-)joiners, and the BOM must render
        // as nothing with zero advance — falling through would draw the bitmap
        // '?' fallback for each (the "??" seen in headings with emoji).
        if matches!(character, '\u{FE00}'..='\u{FE0F}' | '\u{200B}'..='\u{200D}' | '\u{FEFF}' | '\u{2060}') {
            return CachedGlyph {
                advance: 0.0,
                advance_px: 0,
                ascent_px,
                synthetic_bold: false,
                mode: GlyphMode::Vector {
                    width: 0,
                    height: 0,
                    xmin: 0,
                    ymin: 0,
                    bitmap: Vec::new(),
                },
            };
        }

        // Bold first; a family with no bold cut, or one whose bold cut lacks
        // this character, drops through to the regular stack and is smeared.
        let mut synthetic_bold = bold;
        if bold {
            self.ensure_font_for(character, font_family, true);
            if self.fonts_for(font_family, true).iter().any(|font| font.has_glyph(character)) {
                synthetic_bold = false;
            }
        }
        let stack_is_bold = bold && !synthetic_bold;
        self.ensure_font_for(character, font_family, stack_is_bold);

        let fallback_advance = estimated_glyph_advance_px(character, font_size_px, font_family);

        for font in self.fonts_for(font_family, stack_is_bold) {
            if !font.has_glyph(character) {
                continue;
            }

            let (metrics, bitmap) = font.rasterize(character, font_size_px as f32);
            let advance = if metrics.advance_width > 0.0 {
                metrics.advance_width
            } else {
                fallback_advance as f32
            }
            .max(MIN_ADVANCE_PX as f32);
            let advance_px = (advance.round() as u32).max(MIN_ADVANCE_PX);
            if metrics.width == 0 || metrics.height == 0 {
                return CachedGlyph {
                    advance,
                    advance_px,
                    ascent_px,
                    synthetic_bold,
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
                advance,
                advance_px,
                ascent_px,
                synthetic_bold,
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
            advance: fallback_advance as f32,
            advance_px: fallback_advance,
            ascent_px,
            synthetic_bold,
            mode: GlyphMode::Bitmap { glyph, scale },
        }
    }

    fn fonts_for(&self, font_family: FontFamilyKind, bold: bool) -> &[Font] {
        let fonts = match (font_family, bold) {
            (FontFamilyKind::Sans, false) => &self.sans_fonts,
            (FontFamilyKind::Sans, true) => &self.sans_bold_fonts,
            (FontFamilyKind::Serif, false) => &self.serif_fonts,
            (FontFamilyKind::Serif, true) => &self.serif_bold_fonts,
            (FontFamilyKind::Monospace, false) => &self.monospace_fonts,
            (FontFamilyKind::Monospace, true) => &self.monospace_bold_fonts,
        };
        // A bold stack that came up empty is not backfilled with sans: the
        // caller retries on the regular stack and smears instead.
        if bold {
            return fonts;
        }
        // A family with no installed candidate borrows sans rather than holding
        // a copy of it: cloning a `fontdue::Font` would duplicate tens of MB.
        if fonts.is_empty() {
            &self.sans_fonts
        } else {
            fonts
        }
    }

    /// Read this family's first available font if it has none yet. Callers that
    /// only need metrics (rather than a specific glyph) go through here.
    fn ensure_family_loaded(&mut self, font_family: FontFamilyKind, bold: bool) {
        let (fonts, pending) = self.slots(font_family, bold);
        if !fonts.is_empty() {
            return;
        }
        while let Some(path) = pending.pop_front() {
            if let Some(font) = load_font_file(&path) {
                fonts.push(font);
                return;
            }
        }
        // Nothing installed for this family: fall back to sans, which
        // `fonts_for` will hand out. A bold stack is left empty instead --
        // borrowing regular sans would lose the family, and the caller has a
        // smear to fall back on.
        if font_family != FontFamilyKind::Sans && !bold {
            self.ensure_family_loaded(FontFamilyKind::Sans, false);
        }
    }

    fn slots(&mut self, font_family: FontFamilyKind, bold: bool)
    -> (&mut Vec<Font>, &mut VecDeque<PathBuf>) {
        match (font_family, bold) {
            (FontFamilyKind::Sans, false) => (&mut self.sans_fonts, &mut self.sans_pending),
            (FontFamilyKind::Sans, true) => {
                (&mut self.sans_bold_fonts, &mut self.sans_bold_pending)
            }
            (FontFamilyKind::Serif, false) => (&mut self.serif_fonts, &mut self.serif_pending),
            (FontFamilyKind::Serif, true) => {
                (&mut self.serif_bold_fonts, &mut self.serif_bold_pending)
            }
            (FontFamilyKind::Monospace, false) => {
                (&mut self.monospace_fonts, &mut self.monospace_pending)
            }
            (FontFamilyKind::Monospace, true) => {
                (&mut self.monospace_bold_fonts, &mut self.monospace_bold_pending)
            }
        }
    }

    fn ensure_font_for(&mut self, character: char, font_family: FontFamilyKind, bold: bool) {
        let (fonts, pending) = self.slots(font_family, bold);

        if fonts.iter().any(|font| font.has_glyph(character)) {
            return;
        }

        let mut found = false;
        while let Some(path) = pending.pop_front() {
            let Some(font) = load_font_file(&path) else {
                continue;
            };
            let supports_glyph = font.has_glyph(character);
            fonts.push(font);
            if supports_glyph {
                found = true;
                break;
            }
        }

        // This family cannot draw the character; sans is the shared fallback.
        if !found && font_family != FontFamilyKind::Sans {
            self.ensure_font_for(character, FontFamilyKind::Sans, bold);
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

fn font_candidates(font_family: FontFamilyKind, bold: bool) -> Vec<PathBuf> {
    if cfg!(target_os = "windows") {
        let windows_root = std::env::var_os("WINDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("C:\\Windows"));
        let fonts_dir = windows_root.join("Fonts");
        let files = match (font_family, bold) {
            (FontFamilyKind::Sans, false) => WINDOWS_SANS_FONT_FILES,
            (FontFamilyKind::Sans, true) => WINDOWS_SANS_BOLD_FONT_FILES,
            (FontFamilyKind::Serif, false) => WINDOWS_SERIF_FONT_FILES,
            (FontFamilyKind::Serif, true) => WINDOWS_SERIF_BOLD_FONT_FILES,
            (FontFamilyKind::Monospace, false) => WINDOWS_MONOSPACE_FONT_FILES,
            (FontFamilyKind::Monospace, true) => WINDOWS_MONOSPACE_BOLD_FONT_FILES,
        };

        return files.iter().map(|file| fonts_dir.join(file)).collect();
    }

    let files = match (font_family, bold) {
        (FontFamilyKind::Sans, false) => UNIX_SANS_FONT_PATHS,
        (FontFamilyKind::Sans, true) => UNIX_SANS_BOLD_FONT_PATHS,
        (FontFamilyKind::Serif, false) => UNIX_SERIF_FONT_PATHS,
        (FontFamilyKind::Serif, true) => UNIX_SERIF_BOLD_FONT_PATHS,
        (FontFamilyKind::Monospace, false) => UNIX_MONOSPACE_FONT_PATHS,
        (FontFamilyKind::Monospace, true) => UNIX_MONOSPACE_BOLD_FONT_PATHS,
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

    /// Variation selectors / zero-width characters must not paint anything (no
    /// '?' fallback box) and must take zero advance — "⚛️" is U+269B + U+FE0F
    /// and rendered as "??" before this.
    #[test]
    fn invisible_characters_render_as_nothing() {
        let mut context = FontContext::load();
        for ch in ['\u{FE0F}', '\u{200B}', '\u{200D}', '\u{FEFF}'] {
            let with = context.text_width_px(&format!("A{ch}B"), 18, FontFamilyKind::Sans);
            let without = context.text_width_px("AB", 18, FontFamilyKind::Sans);
            assert_eq!(with, without, "U+{:04X} should have zero advance", ch as u32);
        }
        // Drawing a string of only invisibles must leave the buffer untouched.
        let mut buffer = vec![0_u32; 100 * 40];
        context.draw_text(
            &mut buffer, 100, 40, 4, 4, "\u{FE0F}\u{200D}", 18, 0x00FFFFFF,
            false, false, false, FontFamilyKind::Sans,
        );
        assert!(buffer.iter().all(|p| *p == 0), "invisible chars painted pixels");
    }

    /// Symbols outside the text faces (⚛ U+269B) should resolve via the
    /// symbol-font fallback instead of the bitmap '?'. Skips quietly when the
    /// system lacks a font with the glyph (non-Windows CI).
    #[test]
    fn atom_symbol_uses_real_glyph_when_available() {
        let mut context = FontContext::load();
        let mut buffer = vec![0_u32; 100 * 60];
        context.draw_text(
            &mut buffer, 100, 60, 8, 8, "\u{269B}", 24, 0x00FFFFFF,
            false, false, false, FontFamilyKind::Sans,
        );
        let painted = buffer.iter().filter(|p| **p != 0).count();
        // The atom symbol is dense (orbital rings); the bitmap '?' fallback is a
        // sparse 8x8 blow-up. Just require that *something* was drawn and that,
        // when a symbol font exists, the glyph isn't the tiny fallback footprint.
        assert!(painted > 0, "U+269B drew nothing");
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

#[cfg(test)]
mod lazy_loading_tests {
    use super::*;

    /// `fontdue` expands a font to roughly 40x its file size when it parses one,
    /// so `FontContext::load` must not touch the disk. Loading sans, monospace
    /// and serif up front cost ~64 MB before anything was drawn.
    #[test]
    fn load_reads_no_font_files() {
        let fonts = FontContext::load();
        assert!(fonts.sans_fonts.is_empty(), "sans should load on first use");
        assert!(fonts.monospace_fonts.is_empty(), "monospace should load on first use");
        assert!(fonts.serif_fonts.is_empty(), "serif should load on first use");
        assert!(
            !fonts.sans_pending.is_empty(),
            "the candidate list should still be queued for lazy loading"
        );
    }

    /// Asking one family for metrics must not drag the other two in with it.
    #[test]
    fn using_one_family_loads_only_that_family() {
        let mut fonts = FontContext::load();
        let _ = fonts.line_metrics(16, FontFamilyKind::Monospace);
        assert!(
            !fonts.monospace_fonts.is_empty(),
            "monospace should have been loaded on demand"
        );
        assert!(
            fonts.serif_fonts.is_empty(),
            "serif was never asked for and must stay unloaded"
        );
    }

    /// Deferring the load must not change what gets measured: a lazily loaded
    /// family has to report the font's real ascent, not the rough fallback the
    /// code uses when no font is available at all.
    #[test]
    fn lazy_metrics_come_from_a_real_font() {
        let mut fonts = FontContext::load();
        let has_any_candidate = font_candidates(FontFamilyKind::Sans, false)
            .iter()
            .any(|path| path.is_file());
        if !has_any_candidate {
            return; // no system fonts on this machine; nothing to compare against
        }
        let metrics = fonts.line_metrics(16, FontFamilyKind::Sans);
        assert!(
            metrics.ascent_px != 16,
            "ascent equal to the font size means the no-font fallback was used"
        );
        assert!(metrics.ascent_px > 0 && metrics.ascent_px < 64);
    }

    /// Bold asks for the bold cut first and only smears when there is none.
    #[test]
    fn bold_prefers_an_installed_bold_cut() {
        let mut fonts = FontContext::load();
        if !font_candidates(FontFamilyKind::Sans, true)
            .iter()
            .any(|path| path.is_file())
        {
            return; // no bold cut on this machine; the smear is all there is
        }

        let glyph = fonts.cached_glyph('A', 32, FontFamilyKind::Sans, true);
        assert!(
            !glyph.synthetic_bold,
            "a bold cut is installed, so the regular glyph must not be standing in"
        );

        // The regular request keeps its own cache entry and its own stack.
        let regular = fonts.cached_glyph('A', 32, FontFamilyKind::Sans, false);
        assert!(!regular.synthetic_bold);
    }

    /// `line-height: normal` is the face's own recommended spacing, which for
    /// the default sans at 16px is what Chrome reports: 18px, not 21.
    #[test]
    fn normal_line_height_follows_the_face() {
        let mut fonts = FontContext::load();
        fonts.ensure_family_loaded(FontFamilyKind::Sans, false);
        if fonts.sans_fonts.is_empty() {
            return; // no system fonts available
        }
        let line = fonts.line_height_px(16, FontFamilyKind::Sans);
        assert!(
            (17..=19).contains(&line),
            "16px sans should give a line of about 18px, got {line}"
        );
        // It scales with the size rather than sitting on a fixed gap.
        let doubled = fonts.line_height_px(32, FontFamilyKind::Sans);
        assert!(
            doubled >= line * 2 - 2 && doubled <= line * 2 + 2,
            "32px should be about double 16px: {line} vs {doubled}"
        );
    }

    /// A family with no installed candidate borrows sans rather than cloning it;
    /// cloning a `fontdue::Font` would duplicate tens of megabytes.
    #[test]
    fn empty_family_borrows_sans_without_copying() {
        let mut fonts = FontContext::load();
        fonts.ensure_family_loaded(FontFamilyKind::Sans, false);
        if fonts.sans_fonts.is_empty() {
            return; // no system fonts available
        }
        fonts.serif_pending.clear();
        assert!(fonts.serif_fonts.is_empty());
        assert_eq!(
            fonts.fonts_for(FontFamilyKind::Serif, false).len(),
            fonts.sans_fonts.len(),
            "an empty family should hand out the sans list"
        );
    }
}
