//! Map visual lines back to source UTF-8 ranges. Wrapping drops spaces and text
//! transforms can expand a character, so concatenating displayed line lengths
//! does not produce valid source positions.
use cssimpler_core::fonts::{TextLayout, TextStyle, TextTransform, WhiteSpace, layout_text_block};
use cssimpler_core::{RenderKind, RenderNode};

#[derive(Clone, Copy)]
pub(super) struct Span {
    pub display_start: usize,
    pub display_end: usize,
    pub source_start: usize,
    pub source_end: usize,
}

pub(super) fn source_spans(text: &str, style: &TextStyle, layout: &TextLayout) -> Vec<Vec<Span>> {
    let mut transformed = Vec::new();
    let mut capitalize = true;
    for (start, ch) in text.char_indices() {
        let end = start + ch.len_utf8();
        let display = match style.text_transform {
            TextTransform::Uppercase => ch.to_uppercase().collect::<String>(),
            TextTransform::Lowercase => ch.to_lowercase().collect(),
            TextTransform::Capitalize if capitalize && ch.is_alphabetic() => {
                capitalize = false;
                ch.to_uppercase().collect()
            }
            TextTransform::Capitalize => {
                capitalize = !(ch.is_alphanumeric() || ch == '\'');
                ch.to_string()
            }
            _ => ch.to_string(),
        };
        transformed.extend(display.chars().map(|display| (display, start, end)));
    }
    let mut cursor = 0;
    layout
        .lines
        .iter()
        .map(|line| {
            let mut spans = Vec::new();
            for (display_start, ch) in line.text.char_indices() {
                // Layout can omit leading/trailing whitespace, collapse a whitespace
                // run, or consume it at a wrap. It never removes a non-space glyph.
                while cursor < transformed.len()
                    && transformed[cursor].0 != ch
                    && transformed[cursor].0.is_whitespace()
                    && !(ch == ' '
                        && matches!(style.white_space, WhiteSpace::Normal | WhiteSpace::NoWrap))
                {
                    cursor += 1;
                }
                if let Some(&(actual, source_start, source_end)) = transformed.get(cursor) {
                    if actual == ch || actual.is_whitespace() && ch == ' ' {
                        spans.push(Span {
                            display_start,
                            display_end: display_start + ch.len_utf8(),
                            source_start,
                            source_end,
                        });
                        cursor += 1;
                        if ch == ' '
                            && matches!(style.white_space, WhiteSpace::Normal | WhiteSpace::NoWrap)
                        {
                            while cursor < transformed.len()
                                && transformed[cursor].0.is_whitespace()
                            {
                                cursor += 1;
                            }
                            if let Some(span) = spans.last_mut() {
                                span.source_end = transformed
                                    .get(cursor.wrapping_sub(1))
                                    .map_or(source_end, |entry| entry.2);
                            }
                        }
                    }
                }
            }
            spans
        })
        .collect()
}

pub(super) fn display_style(style: &TextStyle) -> TextStyle {
    let mut style = style.clone();
    style.text_transform = TextTransform::None;
    style.white_space = WhiteSpace::Pre;
    style
}

/// Closest source UTF-8 caret in a text node, including wrapping, alignment,
/// padding and the node's own scroll offset. Coordinates are in scene space.
pub fn text_index_at_position(node: &RenderNode, x: f32, y: f32) -> Option<usize> {
    let RenderKind::Text(text) = &node.kind else {
        return None;
    };
    let box_layout = super::scrollbar::text_layout(node);
    let owned;
    let layout = if let Some(prepared) = &node.text_layout {
        &prepared.layout
    } else {
        owned = layout_text_block(text, &node.style.text, Some(box_layout.width.max(1.0)));
        &owned
    };
    if layout.lines.is_empty() {
        return Some(0);
    }
    let row = (((y - box_layout.y) / layout.line_height).floor().max(0.0) as usize)
        .min(layout.lines.len() - 1);
    let line = &layout.lines[row];
    let spans = source_spans(text, &node.style.text, layout);
    let local_x =
        x - super::aligned_text_line_start_x(box_layout, line.width, node.style.text.text_align);
    let style = display_style(&node.style.text);
    let stops = super::fonts::line_caret_stops(&line.text, &style);
    let mut candidates = Vec::new();
    for (i, span) in spans[row].iter().enumerate() {
        // Only the outer boundaries of an expanded source character are editable.
        if i == 0 || spans[row][i - 1].source_start != span.source_start {
            candidates.push((span.display_start, span.source_start));
        }
        if i + 1 == spans[row].len() || spans[row][i + 1].source_end != span.source_end {
            candidates.push((span.display_end, span.source_end));
        }
    }
    let mut best = None;
    let mut distance = f32::INFINITY;
    for (display, source) in candidates {
        if let Ok(index) = stops.binary_search_by_key(&display, |stop| stop.byte_index) {
            let stop = &stops[index];
            let d = (stop.offset_px - local_x).abs();
            if d < distance {
                best = Some(source);
                distance = d;
            }
        }
    }
    Some(best.unwrap_or_else(|| {
        spans[..row]
            .iter()
            .flatten()
            .last()
            .map_or(0, |s| s.source_end)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cssimpler_core::fonts::FontFamily;
    use cssimpler_core::{
        Color, LayoutBox, TextEditDecoration, TextSelectionRange, TextSelectionStyle, VisualStyle,
    };

    #[test]
    fn wrapped_selection_and_hit_testing_preserve_source_spaces_and_unicode() {
        let text = "one   éक\nlast";
        let style = TextStyle {
            families: vec![FontFamily::Named("missing-selection-test-font".into())],
            size_px: 16.0,
            ..Default::default()
        };
        let node = RenderNode::text(LayoutBox::new(10.0, 10.0, 40.0, 100.0), text).with_style(
            VisualStyle {
                text: style.clone(),
                ..Default::default()
            },
        );
        let layout = layout_text_block(text, &style, Some(40.0));
        assert_eq!(layout.lines.len(), 3);
        let y = 10.0 + layout.line_height * 1.5;
        assert_eq!(text_index_at_position(&node, 10.0, y), Some("one   ".len()));
        assert_eq!(
            text_index_at_position(&node, 200.0, y),
            Some("one   éक".len())
        );
        let segments = super::super::text_range_segments(
            node.layout,
            text,
            None,
            &style,
            "one   ".len(),
            "one   éक".len(),
        );
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].text, "éक");
        assert_eq!(segments[0].layout.y, 10.0 + layout.line_height);
    }

    #[test]
    fn uppercase_expansion_selects_the_whole_source_character() {
        let style = TextStyle {
            text_transform: TextTransform::Uppercase,
            ..Default::default()
        };
        let text = "aßz";
        let layout = LayoutBox::new(0.0, 0.0, 100.0, 30.0);
        let segments = super::super::text_range_segments(layout, text, None, &style, 1, 3);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].text, "SS");
        let node = RenderNode::text(layout, text).with_style(VisualStyle {
            text: style,
            ..Default::default()
        });
        for x in 0..100 {
            assert!([0, 1, 3, 4].contains(&text_index_at_position(&node, x as f32, 10.0).unwrap()));
        }
    }

    #[test]
    fn selection_background_has_small_rounded_corners() {
        let color = Color::rgb(255, 0, 153);
        let scene = vec![
            RenderNode::text(LayoutBox::new(10.0, 10.0, 100.0, 40.0), "    ")
                .with_style(VisualStyle {
                    text: TextStyle {
                        size_px: 20.0,
                        white_space: WhiteSpace::Pre,
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .with_text_edit(TextEditDecoration {
                    caret: None,
                    selection: Some(TextSelectionRange {
                        start: 0,
                        end: 4,
                        style: TextSelectionStyle {
                            background: color,
                            ..Default::default()
                        },
                    }),
                }),
        ];
        let mut pixels = vec![0; 120 * 60];
        super::super::render_to_buffer(&scene, &mut pixels, 120, 60, Color::WHITE);
        assert_eq!(pixels[20 * 120 + 12], super::super::pack_rgb(color));
        assert_ne!(pixels[10 * 120 + 10], super::super::pack_rgb(color));
    }
}
