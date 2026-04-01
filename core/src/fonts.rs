use std::collections::HashMap;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::path::Path;
use std::sync::{Arc, OnceLock, RwLock};

use ab_glyph::{Font, FontArc, FontVec, ScaleFont};

const DEFAULT_FONT_SIZE_PX: f32 = 16.0;
const DEFAULT_FONT_WEIGHT: u16 = 400;
const DEFAULT_LINE_HEIGHT_SCALE: f32 = 1.2;

const BITMAP_BASE_FONT_SIZE_PX: f32 = 16.0;
const BITMAP_GLYPH_ADVANCE_PX: f32 = 18.0;
const BITMAP_LINE_HEIGHT_PX: f32 = 20.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GenericFontFamily {
    Serif,
    SansSerif,
    Cursive,
    Fantasy,
    Monospace,
    SystemUi,
    Emoji,
    Math,
    FangSong,
    UiSerif,
    UiSansSerif,
    UiMonospace,
    UiRounded,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FontFamily {
    Named(String),
    Generic(GenericFontFamily),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FontStyle {
    #[default]
    Normal,
    Italic,
    Oblique,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LineHeight {
    Normal,
    Px(f32),
    Scale(f32),
}

impl Default for LineHeight {
    fn default() -> Self {
        Self::Normal
    }
}

impl LineHeight {
    pub fn resolve_px(&self, font_size_px: f32) -> f32 {
        match self {
            Self::Normal => font_size_px * DEFAULT_LINE_HEIGHT_SCALE,
            Self::Px(px) => *px,
            Self::Scale(scale) => font_size_px * *scale,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextStyle {
    pub families: Vec<FontFamily>,
    pub size_px: f32,
    pub weight: u16,
    pub style: FontStyle,
    pub line_height: LineHeight,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            families: vec![FontFamily::Generic(GenericFontFamily::SansSerif)],
            size_px: DEFAULT_FONT_SIZE_PX,
            weight: DEFAULT_FONT_WEIGHT,
            style: FontStyle::Normal,
            line_height: LineHeight::Normal,
        }
    }
}

impl TextStyle {
    pub fn with_family(mut self, family: FontFamily) -> Self {
        self.families = vec![family];
        self
    }

    pub fn resolved_line_height_px(&self) -> f32 {
        self.line_height.resolve_px(self.size_px.max(1.0))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextLineLayout {
    pub text: String,
    pub width: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextLayout {
    pub lines: Vec<TextLineLayout>,
    pub width: f32,
    pub height: f32,
    pub line_height: f32,
}

#[derive(Clone)]
pub struct ResolvedFont {
    font: FontArc,
    size_px: f32,
    line_height_px: f32,
}

impl std::fmt::Debug for ResolvedFont {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedFont")
            .field("size_px", &self.size_px)
            .field("line_height_px", &self.line_height_px)
            .finish()
    }
}

impl ResolvedFont {
    pub fn font(&self) -> &FontArc {
        &self.font
    }

    pub fn size_px(&self) -> f32 {
        self.size_px
    }

    pub fn line_height_px(&self) -> f32 {
        self.line_height_px
    }

    pub fn measure_text_width(&self, text: &str) -> f32 {
        if text.is_empty() {
            return 0.0;
        }

        let scaled_font = self.font.as_scaled(self.size_px);
        let mut width = 0.0;
        let mut previous = None;

        for character in text.chars() {
            let glyph_id = scaled_font.glyph_id(character);
            if let Some(previous) = previous {
                width += scaled_font.kern(previous, glyph_id);
            }
            width += scaled_font.h_advance(glyph_id);
            previous = Some(glyph_id);
        }

        width
    }
}

#[derive(Debug)]
pub enum FontError {
    Io(std::io::Error),
    InvalidFontData,
    NoFacesLoaded,
    RegistryPoisoned,
}

impl Display for FontError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(source) => write!(f, "font IO failed: {source}"),
            Self::InvalidFontData => write!(f, "font data could not be parsed"),
            Self::NoFacesLoaded => write!(f, "no usable font faces were loaded"),
            Self::RegistryPoisoned => write!(f, "font registry lock was poisoned"),
        }
    }
}

impl Error for FontError {}

impl From<std::io::Error> for FontError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

#[derive(Default)]
struct FontRegistry {
    database: fontdb::Database,
    system_fonts_loaded: bool,
    cache: HashMap<fontdb::ID, FontArc>,
}

impl FontRegistry {
    fn ensure_system_fonts_loaded(&mut self) {
        if !self.system_fonts_loaded {
            self.database.load_system_fonts();
            self.system_fonts_loaded = true;
        }
    }

    fn register_font_bytes(&mut self, data: Vec<u8>) -> Result<Vec<String>, FontError> {
        let ids = self
            .database
            .load_font_source(fontdb::Source::Binary(Arc::new(data)));
        if ids.is_empty() {
            return Err(FontError::NoFacesLoaded);
        }

        Ok(discovered_family_names(&self.database, ids.as_slice()))
    }

    fn register_font_file(&mut self, path: &Path) -> Result<Vec<String>, FontError> {
        let ids = self
            .database
            .load_font_source(fontdb::Source::File(path.to_path_buf()));
        if ids.is_empty() {
            return if path.exists() {
                Err(FontError::NoFacesLoaded)
            } else {
                Err(FontError::Io(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("font file not found: {}", path.display()),
                )))
            };
        }

        Ok(discovered_family_names(&self.database, ids.as_slice()))
    }

    fn resolve_font(&mut self, style: &TextStyle) -> Option<ResolvedFont> {
        self.ensure_system_fonts_loaded();

        let query_families = query_families(style);
        let query = fontdb::Query {
            families: &query_families,
            weight: fontdb::Weight(style.weight.clamp(1, 1_000)),
            style: match style.style {
                FontStyle::Normal => fontdb::Style::Normal,
                FontStyle::Italic => fontdb::Style::Italic,
                FontStyle::Oblique => fontdb::Style::Oblique,
            },
            ..fontdb::Query::default()
        };
        let font_id = self.database.query(&query)?;

        let font = if let Some(existing) = self.cache.get(&font_id) {
            existing.clone()
        } else {
            let loaded = self.database.with_face_data(font_id, |data, face_index| {
                FontVec::try_from_vec_and_index(data.to_vec(), face_index)
                    .map(FontArc::new)
                    .ok()
            })??;
            self.cache.insert(font_id, loaded.clone());
            loaded
        };

        Some(ResolvedFont {
            font,
            size_px: style.size_px.max(1.0),
            line_height_px: style.resolved_line_height_px().max(style.size_px.max(1.0)),
        })
    }
}

fn discovered_family_names(database: &fontdb::Database, ids: &[fontdb::ID]) -> Vec<String> {
    let mut names = Vec::new();

    for id in ids {
        let Some(face) = database.face(*id) else {
            continue;
        };

        for (name, _) in &face.families {
            if !names.iter().any(|existing| existing == name) {
                names.push(name.clone());
            }
        }
    }

    names
}

fn query_families(style: &TextStyle) -> Vec<fontdb::Family<'_>> {
    let mut families = Vec::new();

    for family in &style.families {
        match family {
            FontFamily::Named(name) => families.push(fontdb::Family::Name(name.as_str())),
            FontFamily::Generic(generic) => match generic {
                GenericFontFamily::Serif | GenericFontFamily::UiSerif => {
                    families.push(fontdb::Family::Serif);
                }
                GenericFontFamily::SansSerif
                | GenericFontFamily::UiSansSerif
                | GenericFontFamily::Emoji
                | GenericFontFamily::Math
                | GenericFontFamily::FangSong
                | GenericFontFamily::UiRounded => {
                    families.push(fontdb::Family::SansSerif);
                }
                GenericFontFamily::Cursive => families.push(fontdb::Family::Cursive),
                GenericFontFamily::Fantasy => families.push(fontdb::Family::Fantasy),
                GenericFontFamily::Monospace | GenericFontFamily::UiMonospace => {
                    families.push(fontdb::Family::Monospace);
                }
                GenericFontFamily::SystemUi => {
                    #[cfg(target_os = "windows")]
                    families.push(fontdb::Family::Name("Segoe UI"));
                    #[cfg(target_os = "macos")]
                    families.push(fontdb::Family::Name(".SF NS Text"));
                    families.push(fontdb::Family::SansSerif);
                }
            },
        }
    }

    if families.is_empty() {
        families.push(fontdb::Family::SansSerif);
    }

    families
}

fn registry() -> &'static RwLock<FontRegistry> {
    static REGISTRY: OnceLock<RwLock<FontRegistry>> = OnceLock::new();
    REGISTRY.get_or_init(|| RwLock::new(FontRegistry::default()))
}

pub fn register_font_bytes(data: Vec<u8>) -> Result<Vec<String>, FontError> {
    let mut registry = registry()
        .write()
        .map_err(|_| FontError::RegistryPoisoned)?;
    registry.register_font_bytes(data)
}

pub fn register_font_file(path: impl AsRef<Path>) -> Result<Vec<String>, FontError> {
    let mut registry = registry()
        .write()
        .map_err(|_| FontError::RegistryPoisoned)?;
    registry.register_font_file(path.as_ref())
}

pub fn resolve_font(style: &TextStyle) -> Option<ResolvedFont> {
    let mut registry = registry().write().ok()?;
    registry.resolve_font(style)
}

pub fn layout_text_block(text: &str, style: &TextStyle, wrap_width: Option<f32>) -> TextLayout {
    if text.is_empty() {
        return TextLayout {
            lines: Vec::new(),
            width: 0.0,
            height: 0.0,
            line_height: style.resolved_line_height_px(),
        };
    }

    let backend = resolve_font(style)
        .map(MeasurementBackend::Real)
        .unwrap_or_else(|| MeasurementBackend::Bitmap(BitmapFontMetrics::from_style(style)));
    let line_height = backend.line_height();
    let lines = wrap_text_lines(text, wrap_width, &backend);
    let width = lines.iter().map(|line| line.width).fold(0.0_f32, f32::max);
    let height = lines.len() as f32 * line_height;

    TextLayout {
        lines,
        width,
        height,
        line_height,
    }
}

enum MeasurementBackend {
    Real(ResolvedFont),
    Bitmap(BitmapFontMetrics),
}

impl MeasurementBackend {
    fn line_height(&self) -> f32 {
        match self {
            Self::Real(font) => font.line_height_px(),
            Self::Bitmap(metrics) => metrics.line_height_px,
        }
    }

    fn measure_text_width(&self, text: &str) -> f32 {
        match self {
            Self::Real(font) => font.measure_text_width(text),
            Self::Bitmap(metrics) => metrics.measure_text_width(text),
        }
    }
}

#[derive(Clone, Copy)]
struct BitmapFontMetrics {
    glyph_advance_px: f32,
    line_height_px: f32,
}

impl BitmapFontMetrics {
    fn from_style(style: &TextStyle) -> Self {
        let font_size_px = style.size_px.max(1.0);
        let scale = font_size_px / BITMAP_BASE_FONT_SIZE_PX;
        let default_line_height = BITMAP_LINE_HEIGHT_PX * scale;

        Self {
            glyph_advance_px: BITMAP_GLYPH_ADVANCE_PX * scale,
            line_height_px: style.resolved_line_height_px().max(default_line_height),
        }
    }

    fn measure_text_width(self, text: &str) -> f32 {
        text.chars().count() as f32 * self.glyph_advance_px
    }
}

fn wrap_text_lines(
    text: &str,
    wrap_width: Option<f32>,
    backend: &MeasurementBackend,
) -> Vec<TextLineLayout> {
    let max_width = wrap_width.filter(|width| *width > 0.0);
    let Some(max_width) = max_width else {
        let mut lines = Vec::new();
        for source_line in text.lines() {
            lines.push(TextLineLayout {
                text: source_line.to_string(),
                width: backend.measure_text_width(source_line),
            });
        }
        if lines.is_empty() {
            lines.push(TextLineLayout {
                text: String::new(),
                width: 0.0,
            });
        }
        return lines;
    };

    let mut wrapped = Vec::new();
    for source_line in text.lines() {
        wrap_source_line(source_line, max_width, backend, &mut wrapped);
    }

    if wrapped.is_empty() {
        wrapped.push(TextLineLayout {
            text: String::new(),
            width: 0.0,
        });
    }

    wrapped
}

fn wrap_source_line(
    line: &str,
    max_width: f32,
    backend: &MeasurementBackend,
    wrapped: &mut Vec<TextLineLayout>,
) {
    if line.is_empty() {
        wrapped.push(TextLineLayout {
            text: String::new(),
            width: 0.0,
        });
        return;
    }

    let mut current = String::new();

    for word in line.split_whitespace() {
        if current.is_empty() {
            if backend.measure_text_width(word) <= max_width {
                current.push_str(word);
            } else {
                wrap_long_word(word, max_width, backend, wrapped);
            }
            continue;
        }

        let candidate = format!("{current} {word}");
        if backend.measure_text_width(&candidate) <= max_width {
            current = candidate;
        } else {
            let width = backend.measure_text_width(&current);
            wrapped.push(TextLineLayout {
                text: std::mem::take(&mut current),
                width,
            });

            if backend.measure_text_width(word) <= max_width {
                current.push_str(word);
            } else {
                wrap_long_word(word, max_width, backend, wrapped);
            }
        }
    }

    if !current.is_empty() {
        let width = backend.measure_text_width(&current);
        wrapped.push(TextLineLayout {
            text: current,
            width,
        });
    }
}

fn wrap_long_word(
    word: &str,
    max_width: f32,
    backend: &MeasurementBackend,
    wrapped: &mut Vec<TextLineLayout>,
) {
    let mut segment = String::new();

    for character in word.chars() {
        let candidate = format!("{segment}{character}");
        if segment.is_empty() || backend.measure_text_width(&candidate) <= max_width {
            segment.push(character);
            continue;
        }

        let width = backend.measure_text_width(&segment);
        wrapped.push(TextLineLayout {
            text: std::mem::take(&mut segment),
            width,
        });
        segment.push(character);
    }

    if !segment.is_empty() {
        let width = backend.measure_text_width(&segment);
        wrapped.push(TextLineLayout {
            text: segment,
            width,
        });
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        FontFamily, GenericFontFamily, LineHeight, TextStyle, layout_text_block, query_families,
        register_font_file,
    };

    fn bundled_font_family() -> String {
        let asset_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples/assets/powerline-demo.ttf");
        let families = register_font_file(&asset_path)
            .expect("bundled powerline demo font should register during typography tests");
        families
            .into_iter()
            .next()
            .expect("bundled powerline font should expose at least one family name")
    }

    #[test]
    fn default_text_style_prefers_generic_sans_serif() {
        let style = TextStyle::default();

        assert_eq!(
            style.families,
            vec![FontFamily::Generic(GenericFontFamily::SansSerif)]
        );
        assert_eq!(style.size_px, 16.0);
        assert_eq!(style.weight, 400);
    }

    #[test]
    fn line_height_scales_from_font_size() {
        let style = TextStyle {
            size_px: 20.0,
            line_height: LineHeight::Scale(1.4),
            ..TextStyle::default()
        };

        assert_eq!(style.resolved_line_height_px(), 28.0);
    }

    #[test]
    fn bitmap_fallback_wraps_long_words_when_width_is_small() {
        let style = TextStyle::default();
        let layout = layout_text_block("abcdefgh", &style, Some(40.0));

        assert!(layout.lines.len() > 1);
        assert!(layout.width <= 40.0);
    }

    #[test]
    fn system_ui_queries_include_a_generic_fallback() {
        let style = TextStyle {
            families: vec![FontFamily::Generic(GenericFontFamily::SystemUi)],
            ..TextStyle::default()
        };
        let families = query_families(&style);

        assert!(!families.is_empty());
    }

    #[test]
    fn bundled_font_changes_wrapping_and_measurement() {
        let bundled_family = bundled_font_family();
        let sample = "WWW iii WWW iii WWW iii WWW iii";
        let baseline = TextStyle {
            size_px: 24.0,
            ..TextStyle::default()
        };
        let bundled = TextStyle {
            families: vec![FontFamily::Named(bundled_family)],
            size_px: 24.0,
            ..TextStyle::default()
        };

        let baseline_single_line = layout_text_block(sample, &baseline, None);
        let bundled_single_line = layout_text_block(sample, &bundled, None);
        let wrap_width = (baseline_single_line.width.min(bundled_single_line.width)
            + baseline_single_line.width.max(bundled_single_line.width))
            / 2.0;
        let baseline_wrapped = layout_text_block(sample, &baseline, Some(wrap_width));
        let bundled_wrapped = layout_text_block(sample, &bundled, Some(wrap_width));

        assert_ne!(baseline_single_line.width, bundled_single_line.width);
        assert_ne!(baseline_wrapped.lines.len(), bundled_wrapped.lines.len());
    }
}
