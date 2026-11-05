//! OpenType shaping shared by measurement, wrapping, carets and rasterization.
use super::{
    ResolvedFont, TextCaretStop, TextStyle, WhiteSpace, collapse_whitespace, resolve_font,
};
use ab_glyph::{Font, ScaleFont};
use rustybuzz::{Face, UnicodeBuffer};
use std::collections::BTreeMap;
use unicode_script::{Script, UnicodeScript};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Debug, PartialEq)]
pub struct ShapedGlyph {
    pub id: u16,
    /// UTF-8 byte offset of the source cluster.
    pub cluster: usize,
    pub x_advance: f32,
    pub y_advance: f32,
    pub x_offset: f32,
    pub y_offset: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ShapedText {
    pub glyphs: Vec<ShapedGlyph>,
    pub width: f32,
}

/// Scripts that cannot be drawn correctly by a character-to-glyph loop.
/// Preserve the existing fast path for simple text and bitmap fonts.
pub fn requires_shaping(text: &str) -> bool {
    text.chars()
        .any(|c| matches!(c as u32, 0x0900..=0x0dff | 0x1cd0..=0x1cff | 0xa8e0..=0xa8ff))
}

impl ResolvedFont {
    /// Shape one already transformed/normalized line. Offsets and advances use
    /// the same pixel scale as ab_glyph, including nonzero collection face IDs.
    pub fn shape_text(&self, text: &str, letter_spacing_px: f32) -> Option<ShapedText> {
        let face = Face::from_slice(self.font.font_data(), self.face_index)?;
        let scale = self.font.as_scaled(self.size_px).scale_factor();
        let mut glyphs = Vec::new();
        let mut width = 0.0;
        let runs = script_runs(text);
        for (run_index, (start, end)) in runs.iter().copied().enumerate() {
            let mut buffer = UnicodeBuffer::new();
            buffer.push_str(&text[start..end]);
            buffer.guess_segment_properties();
            let output = rustybuzz::shape(&face, &[], buffer);
            let infos = output.glyph_infos();
            for (index, (info, pos)) in infos.iter().zip(output.glyph_positions()).enumerate() {
                // Add spacing between clusters, including script-run boundaries,
                // without separating a base glyph from its marks or conjuncts.
                let gap = infos
                    .get(index + 1)
                    .map_or(run_index + 1 < runs.len(), |next| {
                        next.cluster != info.cluster
                    });
                let x_advance = pos.x_advance as f32 * scale.horizontal
                    + if gap { letter_spacing_px } else { 0.0 };
                width += x_advance;
                glyphs.push(ShapedGlyph {
                    id: info.glyph_id as u16,
                    cluster: start + info.cluster as usize,
                    x_advance,
                    y_advance: pos.y_advance as f32 * scale.vertical,
                    x_offset: pos.x_offset as f32 * scale.horizontal,
                    y_offset: pos.y_offset as f32 * scale.vertical,
                });
            }
        }
        Some(ShapedText { glyphs, width })
    }
}

/// Itemize mixed Latin/Indic labels before choosing OpenType script features.
/// Common punctuation and inherited marks remain attached to their current run.
fn script_runs(text: &str) -> Vec<(usize, usize)> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut runs = Vec::new();
    let mut start = 0;
    let mut current = None;
    for (index, character) in text.char_indices() {
        let script = character.script();
        if matches!(script, Script::Common | Script::Inherited | Script::Unknown) {
            continue;
        }
        if current.is_some_and(|previous| previous != script) {
            runs.push((start, index));
            start = index;
        }
        current = Some(script);
    }
    runs.push((start, text.len()));
    runs
}

/// Map source grapheme boundaries to the *full* shaped line, so a caret never
/// splits a Devanagari conjunct or measures an isolated, differently shaped mark.
pub(super) fn complex_caret_stops(text: &str, style: &TextStyle) -> Option<Vec<TextCaretStop>> {
    if !requires_shaping(text) {
        return None;
    }
    let font = resolve_font(style)?;
    let normalize = |line: &str| match style.white_space {
        WhiteSpace::Normal | WhiteSpace::NoWrap => collapse_whitespace(line),
        WhiteSpace::Pre | WhiteSpace::PreWrap => line.to_string(),
    };
    let transformed = style.transformed_text(text).replace("\r\n", "\n");
    let lines: Vec<_> = transformed.split('\n').map(normalize).collect();
    let runs: Vec<_> = lines
        .iter()
        .map(|line| font.shape_text(line, style.letter_spacing_px))
        .collect::<Option<_>>()?;
    let backend = super::MeasurementBackend::Real(font.clone());
    let measured_widths: Vec<_> = lines
        .iter()
        .map(|line| backend.measure_text_width(line, style.letter_spacing_px))
        .collect();
    let cluster_positions: Vec<BTreeMap<usize, f32>> = runs
        .iter()
        .map(|run| {
            let mut x = 0.0;
            let mut positions = BTreeMap::new();
            for glyph in &run.glyphs {
                positions.entry(glyph.cluster).or_insert(x);
                x += glyph.x_advance;
            }
            positions
        })
        .collect();
    let mut stops = vec![TextCaretStop {
        byte_index: 0,
        offset_px: 0.0,
    }];
    for (start, grapheme) in text.grapheme_indices(true) {
        let end = start + grapheme.len();
        let prefix = style.transformed_text(&text[..end]).replace("\r\n", "\n");
        let prefix_lines: Vec<_> = prefix.split('\n').collect();
        let index = prefix_lines.len() - 1;
        let normalized = normalize(prefix_lines[index]);
        let boundary = normalized.len();
        let run = &runs[index];
        let position = if !requires_shaping(&lines[index]) {
            backend.measure_text_width(&normalized, style.letter_spacing_px)
        } else if boundary >= lines[index].len() {
            run.width
        } else {
            cluster_positions[index]
                .range(..=boundary)
                .next_back()
                .map_or(0.0, |(_, x)| *x)
        };
        let previous_width = measured_widths[..index]
            .iter()
            .copied()
            .fold(0.0_f32, f32::max);
        stops.push(TextCaretStop {
            byte_index: end,
            offset_px: previous_width.max(position),
        });
    }
    Some(stops)
}

#[cfg(test)]
mod tests {
    use super::super::{
        FontFamily, OverflowWrap, layout_text_block, layout_text_caret_stops, register_font_bytes,
    };
    use super::*;

    fn style() -> TextStyle {
        let families = register_font_bytes(
            include_bytes!("../../../examples/assets/NotoSansDevanagari.ttf").to_vec(),
        )
        .unwrap();
        TextStyle {
            families: vec![FontFamily::Named(families[0].clone())],
            size_px: 28.0,
            ..TextStyle::default()
        }
    }

    #[test]
    fn devanagari_conjuncts_and_prebase_matras_are_shaped() {
        let font = resolve_font(&style()).unwrap();
        let conjunct = font.shape_text("क्ष", 0.0).unwrap();
        assert!(conjunct.glyphs.len() < "क्ष".chars().count());
        assert!(conjunct.glyphs.iter().all(|g| g.id != 0 && g.cluster == 0));
        let matra = font.shape_text("कि", 0.0).unwrap();
        assert_ne!(matra.glyphs[0].id, font.font().glyph_id('क').0);
        assert!(matra.glyphs.iter().all(|g| g.cluster == 0));
    }

    #[test]
    fn mixed_script_labels_and_multiline_carets_use_the_right_script() {
        let style = style();
        let font = resolve_font(&style).unwrap();
        let run = font.shape_text("X क्ष", 0.0).unwrap();
        assert!(run.glyphs.len() < "X क्ष".chars().count());
        assert!(run.glyphs.iter().all(|g| g.id != 0));
        let text = "AVATAR\nक्ष";
        let stops = layout_text_caret_stops(text, &style);
        assert!(
            (stops.last().unwrap().offset_px - layout_text_block(text, &style, None).width).abs()
                < 0.001
        );
    }

    #[test]
    fn all_three_languages_measure_the_rendered_advances() {
        let style = style();
        let font = resolve_font(&style).unwrap();
        for text in [
            "नेपाली",
            "मैथिली",
            "भोजपुरी",
            "प्रबन्धक",
            "खानामा छुट",
            "EUR १२.५०",
        ] {
            let run = font.shape_text(text, 0.0).unwrap();
            assert!(
                run.glyphs.iter().all(|g| g.id != 0),
                "missing glyph: {text}"
            );
            assert!((layout_text_block(text, &style, None).width - run.width).abs() < 0.001);
        }
    }

    #[test]
    fn spacing_wrapping_and_carets_respect_clusters() {
        let mut style = style();
        let font = resolve_font(&style).unwrap();
        assert_eq!(
            font.shape_text("क्ष", 2.0).unwrap().width,
            font.shape_text("क्ष", 0.0).unwrap().width
        );
        let base = font.shape_text("क्ष क्ष", 0.0).unwrap().width;
        assert!((font.shape_text("क्ष क्ष", 2.0).unwrap().width - base - 4.0).abs() < 0.001);
        style.letter_spacing_px = 2.0;
        let text = "क्ष कि";
        let stops = layout_text_caret_stops(text, &style);
        assert_eq!(stops.last().unwrap().byte_index, text.len());
        assert!(
            (stops.last().unwrap().offset_px - layout_text_block(text, &style, None).width).abs()
                < 0.001
        );
        assert!(!stops.iter().any(|s| s.byte_index == "क".len()));
        style.overflow_wrap = OverflowWrap::Anywhere;
        let wrapped = layout_text_block("क्षक्ष", &style, Some(1.0));
        assert_eq!(
            wrapped
                .lines
                .iter()
                .map(|l| l.text.as_str())
                .collect::<Vec<_>>(),
            vec!["क्ष", "क्ष"]
        );
    }
}
