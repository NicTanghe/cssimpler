use std::collections::HashMap;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

use ab_glyph::{Font, FontArc, FontVec, GlyphId, ScaleFont};
use unicode_segmentation::UnicodeSegmentation;

const DEFAULT_FONT_SIZE_PX: f32 = 16.0;
const DEFAULT_FONT_WEIGHT: u16 = 400;
const DEFAULT_LINE_HEIGHT_SCALE: f32 = 1.2;

const BITMAP_CELL_SIZE_PX: f32 = 8.0;
const BITMAP_GLYPH_ADVANCE_PX: f32 = 9.0;

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextTransform {
    #[default]
    None,
    Uppercase,
    Lowercase,
    Capitalize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WhiteSpace {
    #[default]
    Normal,
    NoWrap,
    Pre,
    PreWrap,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OverflowWrap {
    #[default]
    Normal,
    Anywhere,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WordBreak {
    #[default]
    Normal,
    BreakAll,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextAlign {
    #[default]
    Start,
    Left,
    Center,
    Right,
    End,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextStyle {
    pub families: Vec<FontFamily>,
    pub size_px: f32,
    pub weight: u16,
    pub style: FontStyle,
    pub line_height: LineHeight,
    pub letter_spacing_px: f32,
    pub text_transform: TextTransform,
    pub white_space: WhiteSpace,
    pub overflow_wrap: OverflowWrap,
    pub word_break: WordBreak,
    pub text_align: TextAlign,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            families: vec![FontFamily::Generic(GenericFontFamily::SansSerif)],
            size_px: DEFAULT_FONT_SIZE_PX,
            weight: DEFAULT_FONT_WEIGHT,
            style: FontStyle::Normal,
            line_height: LineHeight::Normal,
            letter_spacing_px: 0.0,
            text_transform: TextTransform::None,
            white_space: WhiteSpace::Normal,
            overflow_wrap: OverflowWrap::Normal,
            word_break: WordBreak::Normal,
            text_align: TextAlign::Start,
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

    pub fn transformed_text(&self, text: &str) -> String {
        transform_text(text, self.text_transform)
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextCaretStop {
    /// UTF-8 byte offset in the original, untransformed text.
    pub byte_index: usize,
    /// Horizontal width of the rendered prefix ending at `byte_index`.
    pub offset_px: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PreparedTextLayout {
    pub wrap_width: Option<f32>,
    pub layout: TextLayout,
}

impl PreparedTextLayout {
    pub fn new(wrap_width: Option<f32>, layout: TextLayout) -> Self {
        Self { wrap_width, layout }
    }

    pub fn matches_wrap_width(&self, wrap_width: Option<f32>) -> bool {
        wrap_width_bits(self.wrap_width) == wrap_width_bits(wrap_width)
    }
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

    pub fn ascent_px(&self) -> f32 {
        self.font.as_scaled(self.size_px).ascent()
    }

    pub fn descent_px(&self) -> f32 {
        self.font.as_scaled(self.size_px).descent()
    }

    pub fn glyph_height_px(&self) -> f32 {
        self.ascent_px() - self.descent_px()
    }

    pub fn baseline_offset_px(&self) -> f32 {
        half_leading_offset(self.line_height_px, self.glyph_height_px()) + self.ascent_px()
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

struct FontRegistry {
    database: RwLock<fontdb::Database>,
    system_fonts_loaded: AtomicBool,
    cache: RwLock<HashMap<fontdb::ID, FontArc>>,
}

impl FontRegistry {
    fn ensure_system_fonts_loaded(&self) -> Result<(), FontError> {
        if self.system_fonts_loaded.load(Ordering::Acquire) {
            return Ok(());
        }

        let mut database = self
            .database
            .write()
            .map_err(|_| FontError::RegistryPoisoned)?;
        if !self.system_fonts_loaded.load(Ordering::Relaxed) {
            database.load_system_fonts();
            self.system_fonts_loaded.store(true, Ordering::Release);
        }
        Ok(())
    }

    fn register_font_bytes(&self, data: Vec<u8>) -> Result<Vec<String>, FontError> {
        let mut database = self
            .database
            .write()
            .map_err(|_| FontError::RegistryPoisoned)?;
        let ids = database.load_font_source(fontdb::Source::Binary(Arc::new(data)));
        if ids.is_empty() {
            return Err(FontError::NoFacesLoaded);
        }

        Ok(discovered_family_names(&database, ids.as_slice()))
    }

    fn register_font_file(&self, path: &Path) -> Result<Vec<String>, FontError> {
        let mut database = self
            .database
            .write()
            .map_err(|_| FontError::RegistryPoisoned)?;
        let ids = database.load_font_source(fontdb::Source::File(path.to_path_buf()));
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

        Ok(discovered_family_names(&database, ids.as_slice()))
    }

    fn query_font_id(&self, style: &TextStyle) -> Option<fontdb::ID> {
        self.ensure_system_fonts_loaded().ok()?;
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
        let database = self.database.read().ok()?;
        database.query(&query)
    }

    fn build_resolved_font(font: FontArc, style: &TextStyle) -> ResolvedFont {
        ResolvedFont {
            font,
            size_px: style.size_px.max(1.0),
            line_height_px: style.resolved_line_height_px().max(0.0),
        }
    }

    fn cached_font_by_id(&self, font_id: fontdb::ID, style: &TextStyle) -> Option<ResolvedFont> {
        let cache = self.cache.read().ok()?;
        cache
            .get(&font_id)
            .cloned()
            .map(|font| Self::build_resolved_font(font, style))
    }

    fn load_font_by_id(&self, font_id: fontdb::ID, style: &TextStyle) -> Option<ResolvedFont> {
        if let Some(font) = self.cached_font_by_id(font_id, style) {
            return Some(font);
        }

        let database = self.database.read().ok()?;
        let loaded = database.with_face_data(font_id, |data, face_index| {
            FontVec::try_from_vec_and_index(data.to_vec(), face_index)
                .map(FontArc::new)
                .ok()
        })??;
        drop(database);

        let mut cache = self.cache.write().ok()?;
        let font = cache
            .entry(font_id)
            .or_insert_with(|| loaded.clone())
            .clone();

        Some(Self::build_resolved_font(font, style))
    }
}

impl Default for FontRegistry {
    fn default() -> Self {
        Self {
            database: RwLock::new(fontdb::Database::new()),
            system_fonts_loaded: AtomicBool::new(false),
            cache: RwLock::new(HashMap::new()),
        }
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

fn registry() -> &'static FontRegistry {
    static REGISTRY: OnceLock<FontRegistry> = OnceLock::new();
    REGISTRY.get_or_init(FontRegistry::default)
}

pub fn register_font_bytes(data: Vec<u8>) -> Result<Vec<String>, FontError> {
    registry().register_font_bytes(data)
}

pub fn register_font_file(path: impl AsRef<Path>) -> Result<Vec<String>, FontError> {
    registry().register_font_file(path.as_ref())
}

pub fn resolve_font(style: &TextStyle) -> Option<ResolvedFont> {
    let registry = registry();
    let font_id = registry.query_font_id(style)?;
    if let Some(font) = registry.cached_font_by_id(font_id, style) {
        return Some(font);
    }
    registry.load_font_by_id(font_id, style)
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

    let transformed_text = style.transformed_text(text);
    let backend = resolve_font(style)
        .map(MeasurementBackend::Real)
        .unwrap_or_else(|| MeasurementBackend::Bitmap(BitmapFontMetrics::from_style(style)));
    let line_height = backend.line_height();
    let lines = wrap_text_lines(
        &transformed_text,
        wrap_width,
        &backend,
        style.letter_spacing_px,
        style.white_space,
        style.overflow_wrap,
        style.word_break,
    );
    let width = lines.iter().map(|line| line.width).fold(0.0_f32, f32::max);
    let height = lines.len() as f32 * line_height;

    TextLayout {
        lines,
        width,
        height,
        line_height,
    }
}

/// Measures every UTF-8 character caret position in one glyph-metrics pass.
///
/// The byte indices refer to `text`, while offsets reflect the rendered text after applying the
/// style's text transform and whitespace rules. This makes transform expansions such as `ß` to
/// `SS` safe to use for editing without losing the source UTF-8 position.
pub fn layout_text_caret_stops(text: &str, style: &TextStyle) -> Vec<TextCaretStop> {
    let mut stops = Vec::with_capacity(text.chars().count().saturating_add(1));
    stops.push(TextCaretStop {
        byte_index: 0,
        offset_px: 0.0,
    });
    if text.is_empty() {
        return stops;
    }

    let backend = resolve_font(style)
        .map(MeasurementBackend::Real)
        .unwrap_or_else(|| MeasurementBackend::Bitmap(BitmapFontMetrics::from_style(style)));
    let mut measurement =
        CaretTextMeasurement::new(&backend, style.letter_spacing_px, style.white_space);
    let mut transform = IncrementalTextTransform::new(style.text_transform);

    for (character_start, character) in text.char_indices() {
        transform.emit(character, |transformed| measurement.push(transformed));
        stops.push(TextCaretStop {
            byte_index: character_start + character.len_utf8(),
            offset_px: measurement.width(),
        });
    }

    stops
}

fn wrap_width_bits(wrap_width: Option<f32>) -> Option<u32> {
    wrap_width.map(f32::to_bits)
}

enum MeasurementBackend {
    Real(ResolvedFont),
    Bitmap(BitmapFontMetrics),
}

#[derive(Clone, Copy, Default)]
struct TextWidthAccumulator {
    base_width: f32,
    character_count: usize,
    previous_glyph: Option<GlyphId>,
}

impl TextWidthAccumulator {
    fn push(&mut self, backend: &MeasurementBackend, character: char) {
        match backend {
            MeasurementBackend::Real(font) => {
                let scaled_font = font.font.as_scaled(font.size_px);
                let glyph = scaled_font.glyph_id(character);
                if let Some(previous) = self.previous_glyph {
                    self.base_width += scaled_font.kern(previous, glyph);
                }
                self.base_width += scaled_font.h_advance(glyph);
                self.previous_glyph = Some(glyph);
            }
            MeasurementBackend::Bitmap(metrics) => {
                self.base_width += metrics.glyph_advance_px;
            }
        }
        self.character_count += 1;
    }

    fn width(self, letter_spacing_px: f32) -> f32 {
        self.base_width + self.character_count.saturating_sub(1) as f32 * letter_spacing_px
    }
}

struct CaretTextMeasurement<'a> {
    backend: &'a MeasurementBackend,
    letter_spacing_px: f32,
    collapse_whitespace: bool,
    current_line: TextWidthAccumulator,
    widest_completed_line: f32,
    pending_collapsed_space: bool,
    pending_carriage_return: bool,
}

impl<'a> CaretTextMeasurement<'a> {
    fn new(
        backend: &'a MeasurementBackend,
        letter_spacing_px: f32,
        white_space: WhiteSpace,
    ) -> Self {
        Self {
            backend,
            letter_spacing_px,
            collapse_whitespace: matches!(white_space, WhiteSpace::Normal | WhiteSpace::NoWrap),
            current_line: TextWidthAccumulator::default(),
            widest_completed_line: 0.0,
            pending_collapsed_space: false,
            pending_carriage_return: false,
        }
    }

    fn push(&mut self, character: char) {
        if self.pending_carriage_return {
            self.pending_carriage_return = false;
            if character == '\n' {
                self.finish_line();
                return;
            }
            self.push_to_line('\r');
        }

        match character {
            '\r' => self.pending_carriage_return = true,
            '\n' => self.finish_line(),
            _ => self.push_to_line(character),
        }
    }

    fn push_to_line(&mut self, character: char) {
        if self.collapse_whitespace && character.is_whitespace() {
            self.pending_collapsed_space = self.current_line.character_count != 0;
            return;
        }

        if self.pending_collapsed_space {
            self.current_line.push(self.backend, ' ');
            self.pending_collapsed_space = false;
        }
        self.current_line.push(self.backend, character);
    }

    fn finish_line(&mut self) {
        self.widest_completed_line = self.widest_completed_line.max(self.current_line_width());
        self.current_line = TextWidthAccumulator::default();
        self.pending_collapsed_space = false;
    }

    fn current_line_width(&self) -> f32 {
        self.current_line.width(self.letter_spacing_px)
    }

    fn width(&self) -> f32 {
        let current_width = if self.pending_carriage_return {
            let mut preview = self.current_line;
            if self.collapse_whitespace {
                // A trailing collapsible carriage return is omitted just like any trailing space.
                preview.width(self.letter_spacing_px)
            } else {
                preview.push(self.backend, '\r');
                preview.width(self.letter_spacing_px)
            }
        } else {
            self.current_line_width()
        };
        self.widest_completed_line.max(current_width)
    }
}

struct IncrementalTextTransform {
    kind: TextTransform,
    capitalize_next: bool,
}

impl IncrementalTextTransform {
    fn new(kind: TextTransform) -> Self {
        Self {
            kind,
            capitalize_next: true,
        }
    }

    fn emit(&mut self, character: char, mut emit: impl FnMut(char)) {
        match self.kind {
            TextTransform::None => emit(character),
            TextTransform::Uppercase => character.to_uppercase().for_each(emit),
            TextTransform::Lowercase => character.to_lowercase().for_each(emit),
            TextTransform::Capitalize => {
                if self.capitalize_next && character.is_alphabetic() {
                    character.to_uppercase().for_each(emit);
                    self.capitalize_next = false;
                    return;
                }

                emit(character);
                self.capitalize_next = !(character.is_alphanumeric() || character == '\'');
            }
        }
    }
}

impl MeasurementBackend {
    fn line_height(&self) -> f32 {
        match self {
            Self::Real(font) => font.line_height_px(),
            Self::Bitmap(metrics) => metrics.line_height_px,
        }
    }

    fn measure_text_width(&self, text: &str, letter_spacing_px: f32) -> f32 {
        let base_width = match self {
            Self::Real(font) => font.measure_text_width(text),
            Self::Bitmap(metrics) => metrics.measure_text_width(text),
        };

        base_width + letter_spacing_adjustment(text, letter_spacing_px)
    }
}

#[derive(Clone, Copy)]
pub struct BitmapFontMetrics {
    raster_scale: i32,
    glyph_advance_px: f32,
    glyph_height_px: f32,
    line_height_px: f32,
}

impl BitmapFontMetrics {
    pub fn from_style(style: &TextStyle) -> Self {
        let font_size_px = style.size_px.max(1.0);
        let raster_scale = ((font_size_px / BITMAP_CELL_SIZE_PX).round() as i32).max(1);
        let raster_scale_px = raster_scale as f32;

        Self {
            raster_scale,
            glyph_advance_px: BITMAP_GLYPH_ADVANCE_PX * raster_scale_px,
            glyph_height_px: BITMAP_CELL_SIZE_PX * raster_scale_px,
            line_height_px: style.resolved_line_height_px().max(0.0),
        }
    }

    pub fn raster_scale(self) -> i32 {
        self.raster_scale
    }

    pub fn glyph_height_px(self) -> f32 {
        self.glyph_height_px
    }

    pub fn line_height_px(self) -> f32 {
        self.line_height_px
    }

    pub fn glyph_offset_y(self) -> f32 {
        half_leading_offset(self.line_height_px, self.glyph_height_px)
    }

    pub fn measure_text_width(self, text: &str) -> f32 {
        text.chars().count() as f32 * self.glyph_advance_px
    }
}

pub fn half_leading_offset(line_height: f32, glyph_height: f32) -> f32 {
    (line_height - glyph_height) * 0.5
}

fn wrap_text_lines(
    text: &str,
    wrap_width: Option<f32>,
    backend: &MeasurementBackend,
    letter_spacing_px: f32,
    white_space: WhiteSpace,
    overflow_wrap: OverflowWrap,
    word_break: WordBreak,
) -> Vec<TextLineLayout> {
    let mut wrapped = Vec::new();
    for source_line in text.lines() {
        let source_line = match white_space {
            WhiteSpace::Normal | WhiteSpace::NoWrap => collapse_whitespace(source_line),
            WhiteSpace::Pre | WhiteSpace::PreWrap => source_line.to_string(),
        };
        let max_width = match white_space {
            WhiteSpace::NoWrap | WhiteSpace::Pre => None,
            WhiteSpace::Normal | WhiteSpace::PreWrap => wrap_width.filter(|width| *width > 0.0),
        };

        match (white_space, max_width) {
            (WhiteSpace::Normal, Some(max_width)) => wrap_source_line(
                &source_line,
                max_width,
                backend,
                letter_spacing_px,
                overflow_wrap,
                word_break,
                &mut wrapped,
            ),
            (WhiteSpace::PreWrap, Some(max_width)) => wrap_preserved_source_line(
                &source_line,
                max_width,
                backend,
                letter_spacing_px,
                overflow_wrap,
                word_break,
                &mut wrapped,
            ),
            _ => push_measured_line(&source_line, backend, letter_spacing_px, &mut wrapped),
        }
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
    letter_spacing_px: f32,
    overflow_wrap: OverflowWrap,
    word_break: WordBreak,
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
    let break_long_words =
        overflow_wrap == OverflowWrap::Anywhere || word_break == WordBreak::BreakAll;

    for word in line.split_whitespace() {
        if current.is_empty() {
            if !break_long_words || backend.measure_text_width(word, letter_spacing_px) <= max_width
            {
                current.push_str(word);
            } else {
                wrap_long_word(word, max_width, backend, letter_spacing_px, wrapped);
            }
            continue;
        }

        let candidate = format!("{current} {word}");
        if backend.measure_text_width(&candidate, letter_spacing_px) <= max_width {
            current = candidate;
        } else {
            let width = backend.measure_text_width(&current, letter_spacing_px);
            wrapped.push(TextLineLayout {
                text: std::mem::take(&mut current),
                width,
            });

            if !break_long_words || backend.measure_text_width(word, letter_spacing_px) <= max_width
            {
                current.push_str(word);
            } else {
                wrap_long_word(word, max_width, backend, letter_spacing_px, wrapped);
            }
        }
    }

    if !current.is_empty() {
        let width = backend.measure_text_width(&current, letter_spacing_px);
        wrapped.push(TextLineLayout {
            text: current,
            width,
        });
    }
}

fn wrap_preserved_source_line(
    line: &str,
    max_width: f32,
    backend: &MeasurementBackend,
    letter_spacing_px: f32,
    overflow_wrap: OverflowWrap,
    word_break: WordBreak,
    wrapped: &mut Vec<TextLineLayout>,
) {
    if line.is_empty() {
        push_measured_line(line, backend, letter_spacing_px, wrapped);
        return;
    }

    let break_long_words =
        overflow_wrap == OverflowWrap::Anywhere || word_break == WordBreak::BreakAll;
    let mut current = String::new();

    for segment in line.split_inclusive(char::is_whitespace) {
        let candidate = format!("{current}{segment}");
        if backend.measure_text_width(&candidate, letter_spacing_px) <= max_width {
            current.push_str(segment);
            continue;
        }

        if !current.is_empty() {
            push_measured_line(&current, backend, letter_spacing_px, wrapped);
            current.clear();
        }

        if break_long_words && backend.measure_text_width(segment, letter_spacing_px) > max_width {
            wrap_long_word(segment, max_width, backend, letter_spacing_px, wrapped);
        } else {
            current.push_str(segment);
        }
    }

    if !current.is_empty() {
        push_measured_line(&current, backend, letter_spacing_px, wrapped);
    }
}

fn wrap_long_word(
    word: &str,
    max_width: f32,
    backend: &MeasurementBackend,
    letter_spacing_px: f32,
    wrapped: &mut Vec<TextLineLayout>,
) {
    let mut segment = String::new();

    for grapheme in word.graphemes(true) {
        let candidate = format!("{segment}{grapheme}");
        if segment.is_empty()
            || backend.measure_text_width(&candidate, letter_spacing_px) <= max_width
        {
            segment.push_str(grapheme);
            continue;
        }

        let width = backend.measure_text_width(&segment, letter_spacing_px);
        wrapped.push(TextLineLayout {
            text: std::mem::take(&mut segment),
            width,
        });
        segment.push_str(grapheme);
    }

    if !segment.is_empty() {
        let width = backend.measure_text_width(&segment, letter_spacing_px);
        wrapped.push(TextLineLayout {
            text: segment,
            width,
        });
    }
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn push_measured_line(
    text: &str,
    backend: &MeasurementBackend,
    letter_spacing_px: f32,
    lines: &mut Vec<TextLineLayout>,
) {
    lines.push(TextLineLayout {
        text: text.to_string(),
        width: backend.measure_text_width(text, letter_spacing_px),
    });
}

fn letter_spacing_adjustment(text: &str, letter_spacing_px: f32) -> f32 {
    if letter_spacing_px == 0.0 {
        return 0.0;
    }

    let gaps = text.chars().count().saturating_sub(1) as f32;
    gaps * letter_spacing_px
}

fn transform_text(text: &str, text_transform: TextTransform) -> String {
    match text_transform {
        TextTransform::None => text.to_string(),
        TextTransform::Uppercase => text.chars().flat_map(char::to_uppercase).collect(),
        TextTransform::Lowercase => text.chars().flat_map(char::to_lowercase).collect(),
        TextTransform::Capitalize => capitalize_text(text),
    }
}

fn capitalize_text(text: &str) -> String {
    let mut transformed = String::with_capacity(text.len());
    let mut capitalize_next = true;

    for character in text.chars() {
        if capitalize_next && character.is_alphabetic() {
            transformed.extend(character.to_uppercase());
            capitalize_next = false;
            continue;
        }

        transformed.push(character);

        if character.is_alphanumeric() || character == '\'' {
            capitalize_next = false;
        } else {
            capitalize_next = true;
        }
    }

    transformed
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::thread;

    use super::{
        BitmapFontMetrics, FontFamily, GenericFontFamily, LineHeight, OverflowWrap, TextStyle,
        TextTransform, WhiteSpace, WordBreak, layout_text_block, layout_text_caret_stops,
        query_families, register_font_file, resolve_font,
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
    fn default_wrapping_does_not_split_long_words() {
        let style = TextStyle {
            families: vec![FontFamily::Named(
                "cssimpler-missing-font-for-bitmap-tests".to_string(),
            )],
            ..TextStyle::default()
        };
        let layout = layout_text_block("abcdefgh", &style, Some(40.0));

        assert_eq!(layout.lines.len(), 1);
        assert_eq!(layout.lines[0].text, "abcdefgh");
        assert!(layout.width > 40.0);
    }

    #[test]
    fn overflow_wrap_anywhere_splits_only_at_grapheme_boundaries() {
        let style = TextStyle {
            families: vec![FontFamily::Named(
                "cssimpler-missing-font-for-bitmap-tests".to_string(),
            )],
            overflow_wrap: OverflowWrap::Anywhere,
            ..TextStyle::default()
        };
        let text = "a\u{301}b";
        let first_grapheme_width = layout_text_block("a\u{301}", &style, None).width;
        let layout = layout_text_block(text, &style, Some(first_grapheme_width));

        assert_eq!(layout.lines.len(), 2);
        assert_eq!(layout.lines[0].text, "a\u{301}");
        assert_eq!(layout.lines[1].text, "b");
        assert_eq!(
            layout
                .lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<String>(),
            text
        );
    }

    #[test]
    fn word_break_break_all_enables_grapheme_safe_word_splitting() {
        let style = TextStyle {
            families: vec![FontFamily::Named(
                "cssimpler-missing-font-for-bitmap-tests".to_string(),
            )],
            word_break: WordBreak::BreakAll,
            ..TextStyle::default()
        };
        let layout = layout_text_block("abcdef", &style, Some(40.0));

        assert!(layout.lines.len() > 1);
        assert_eq!(
            layout
                .lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<String>(),
            "abcdef"
        );
    }

    #[test]
    fn normal_wrapping_still_wraps_at_collapsed_spaces() {
        let style = TextStyle {
            families: vec![FontFamily::Named(
                "cssimpler-missing-font-for-bitmap-tests".to_string(),
            )],
            ..TextStyle::default()
        };
        let word_width = layout_text_block("alpha", &style, None).width;
        let layout = layout_text_block("alpha   beta", &style, Some(word_width + 1.0));

        assert_eq!(
            layout
                .lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha", "beta"]
        );
    }

    #[test]
    fn nowrap_collapses_spaces_without_automatic_wrapping() {
        let style = TextStyle {
            families: vec![FontFamily::Named(
                "cssimpler-missing-font-for-bitmap-tests".to_string(),
            )],
            white_space: WhiteSpace::NoWrap,
            ..TextStyle::default()
        };
        let layout = layout_text_block("alpha   beta", &style, Some(20.0));

        assert_eq!(layout.lines.len(), 1);
        assert_eq!(layout.lines[0].text, "alpha beta");
        assert!(layout.width > 20.0);
    }

    #[test]
    fn bitmap_line_box_offset_uses_half_leading_including_negative_leading() {
        let roomy = TextStyle {
            size_px: 16.0,
            line_height: LineHeight::Px(32.0),
            ..TextStyle::default()
        };
        let tight = TextStyle {
            line_height: LineHeight::Px(12.0),
            ..roomy.clone()
        };
        let roomy_metrics = BitmapFontMetrics::from_style(&roomy);
        let tight_metrics = BitmapFontMetrics::from_style(&tight);

        assert_eq!(roomy_metrics.glyph_height_px(), 16.0);
        assert_eq!(roomy_metrics.glyph_offset_y(), 8.0);
        assert_eq!(tight_metrics.glyph_offset_y(), -2.0);
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

    #[test]
    fn uppercase_transform_changes_measured_content() {
        let baseline = TextStyle::default();
        let uppercase = TextStyle {
            text_transform: TextTransform::Uppercase,
            ..TextStyle::default()
        };

        let baseline_layout = layout_text_block("Straße", &baseline, None);
        let uppercase_layout = layout_text_block("Straße", &uppercase, None);

        assert_eq!(uppercase_layout.lines[0].text, "STRASSE");
        assert!(uppercase_layout.width > baseline_layout.width);
    }

    #[test]
    fn capitalize_transform_updates_each_word_in_layout_output() {
        let style = TextStyle {
            text_transform: TextTransform::Capitalize,
            ..TextStyle::default()
        };
        let layout = layout_text_block("hello-world from cssimpler", &style, None);

        assert_eq!(layout.lines[0].text, "Hello-World From Cssimpler");
    }

    #[test]
    fn letter_spacing_increases_measured_width() {
        let baseline = TextStyle::default();
        let spaced = TextStyle {
            letter_spacing_px: 2.0,
            ..TextStyle::default()
        };

        let baseline_layout = layout_text_block("ABCD", &baseline, None);
        let spaced_layout = layout_text_block("ABCD", &spaced, None);

        assert!((spaced_layout.width - (baseline_layout.width + 6.0)).abs() < 0.01);
    }

    #[test]
    fn caret_stops_match_unicode_prefix_layout_after_transform_and_whitespace() {
        let text = "  Stra\u{df}e  a\u{301}\r\n\u{130} \u{1f469}\u{200d}\u{1f680}  ";

        for white_space in [
            WhiteSpace::Normal,
            WhiteSpace::NoWrap,
            WhiteSpace::Pre,
            WhiteSpace::PreWrap,
        ] {
            for text_transform in [
                TextTransform::None,
                TextTransform::Uppercase,
                TextTransform::Lowercase,
                TextTransform::Capitalize,
            ] {
                let style = TextStyle {
                    families: vec![FontFamily::Named(
                        "cssimpler-missing-font-for-caret-tests".to_string(),
                    )],
                    letter_spacing_px: 1.25,
                    text_transform,
                    white_space,
                    ..TextStyle::default()
                };
                let stops = layout_text_caret_stops(text, &style);

                assert_eq!(stops.first().map(|stop| stop.byte_index), Some(0));
                assert_eq!(stops.last().map(|stop| stop.byte_index), Some(text.len()));
                for stop in stops {
                    let prefix_width =
                        layout_text_block(&text[..stop.byte_index], &style, None).width;
                    assert!(
                        (stop.offset_px - prefix_width).abs() < 0.001,
                        "caret width mismatch at byte {} for {text_transform:?}/{white_space:?}: {} != {}",
                        stop.byte_index,
                        stop.offset_px,
                        prefix_width,
                    );
                }
            }
        }
    }

    #[test]
    fn caret_stops_match_real_font_kerning_and_utf8_boundaries() {
        let style = TextStyle {
            families: vec![FontFamily::Named(bundled_font_family())],
            size_px: 24.0,
            letter_spacing_px: 0.75,
            white_space: WhiteSpace::Pre,
            ..TextStyle::default()
        };
        let text = "AVATAR a\u{301} \u{1f469}\u{200d}\u{1f680}";
        let stops = layout_text_caret_stops(text, &style);

        assert!(stops.iter().any(|stop| stop.byte_index == 8));
        assert!(
            stops
                .iter()
                .all(|stop| text.is_char_boundary(stop.byte_index))
        );
        for stop in stops {
            let prefix_width = layout_text_block(&text[..stop.byte_index], &style, None).width;
            assert!((stop.offset_px - prefix_width).abs() < 0.001);
        }
    }

    #[test]
    fn long_unicode_caret_layout_emits_one_source_character_stop() {
        let unit = "a\u{301}\u{1f469}\u{200d}\u{1f680}\u{df}";
        let repetitions = 2_048;
        let text = unit.repeat(repetitions);
        let style = TextStyle {
            families: vec![FontFamily::Named(
                "cssimpler-missing-font-for-long-caret-test".to_string(),
            )],
            text_transform: TextTransform::Uppercase,
            white_space: WhiteSpace::Pre,
            ..TextStyle::default()
        };

        let stops = layout_text_caret_stops(&text, &style);

        assert_eq!(stops.len(), repetitions * 6 + 1);
        assert_eq!(stops.last().map(|stop| stop.byte_index), Some(text.len()));
        assert!(
            stops
                .iter()
                .all(|stop| text.is_char_boundary(stop.byte_index))
        );
        assert!(stops.windows(2).all(|pair| {
            pair[0].byte_index < pair[1].byte_index && pair[0].offset_px < pair[1].offset_px
        }));
    }

    #[test]
    fn resolve_font_supports_parallel_reads_after_registration() {
        let bundled_family = bundled_font_family();
        let style = TextStyle {
            families: vec![FontFamily::Named(bundled_family)],
            size_px: 24.0,
            ..TextStyle::default()
        };

        let handles = (0..4)
            .map(|_| {
                let style = style.clone();
                thread::spawn(move || {
                    for _ in 0..8 {
                        let font = resolve_font(&style)
                            .expect("registered bundled font should resolve in parallel");
                        assert_eq!(font.size_px(), 24.0);
                    }
                })
            })
            .collect::<Vec<_>>();

        for handle in handles {
            handle
                .join()
                .expect("parallel font resolution should not panic");
        }
    }
}
