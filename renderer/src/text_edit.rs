//! Map visual lines back to source UTF-8 ranges. Wrapping drops spaces and text
//! transforms can expand a character, so concatenating displayed line lengths
//! does not produce valid source positions.
use cssimpler_core::fonts::{
    PreparedTextLayout, TextLayout, TextStyle, TextTransform, WhiteSpace, layout_text_block,
};
use cssimpler_core::{
    ElementPath, LayoutBox, RenderKind, RenderNode, TextEditDecoration, TransformMatrix3d,
};

pub(super) fn composition_underlines(
    layout: LayoutBox,
    text: &str,
    prepared: Option<&PreparedTextLayout>,
    style: &TextStyle,
    edit: &TextEditDecoration,
) -> Vec<LayoutBox> {
    let Some(range) = &edit.composition else {
        return Vec::new();
    };
    super::text_range_segments(layout, text, prepared, style, range.start, range.end)
        .into_iter()
        .map(|segment| {
            LayoutBox::new(
                segment.layout.x,
                segment.layout.y + segment.layout.height - 1.0,
                segment.layout.width,
                1.0,
            )
        })
        .collect()
}

/// Logical window coordinates, before device scaling. Scroll offsets have
/// already been applied to scene children by the scrollbar controller.
pub(super) fn ime_cursor_area(scene: &[RenderNode], target: &ElementPath) -> Option<LayoutBox> {
    fn visit(
        node: &RenderNode,
        target: &ElementPath,
        parent: TransformMatrix3d,
        perspective: Option<super::PerspectiveContext>,
        in_3d: bool,
    ) -> Option<LayoutBox> {
        let world = parent.multiply(super::node_local_transform_matrix(
            node.layout,
            &node.style.transform,
        ));
        if node.element_path.as_ref() == Some(target)
            && let Some(edit) = &node.text_edit
        {
            let index = edit
                .caret
                .or_else(|| edit.selection.as_ref().map(|selection| selection.end))
                .or_else(|| edit.composition.as_ref().map(|range| range.start))?;
            let layout = super::scrollbar::text_layout(node);
            let rect = match &node.kind {
                RenderKind::Text(text) => super::text_caret_rect(
                    layout,
                    text,
                    node.text_layout.as_ref(),
                    &node.style.text,
                    index,
                )?,
                _ => super::empty_text_caret_rect(layout, &node.style.text),
            };
            let matrix = super::project_world_transform_matrix(node.layout, world, perspective)?;
            let bounds = super::transform_layout_bounds(rect, matrix)?;
            return Some(LayoutBox::new(
                bounds.x0,
                bounds.y0,
                bounds.width(),
                bounds.height(),
            ));
        }
        let child_context = super::child_3d_context(node, in_3d);
        let child_perspective = super::active_child_perspective(node, perspective, child_context);
        node.children
            .iter()
            .find_map(|child| visit(child, target, world, child_perspective, child_context))
    }
    scene
        .iter()
        .find_map(|node| visit(node, target, TransformMatrix3d::IDENTITY, None, false))
}

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
    fn ime_candidate_area_tracks_alignment_padding_scroll_and_parent_transform() {
        let path = ElementPath::root(0);
        let mut node = RenderNode::text(LayoutBox::new(10.0, 20.0, 200.0, 30.0), "ab日本")
            .with_element_path(path.clone())
            .with_text_edit(TextEditDecoration {
                composition: Some(2..8),
                caret: Some(8),
                ..Default::default()
            });
        node.content_inset.left = 8.0;
        node.style.text.text_align = cssimpler_core::fonts::TextAlign::Center;
        let rect = ime_cursor_area(&[node.clone()], &path).unwrap();
        assert!(rect.x > 18.0);
        assert_eq!(rect.y, 20.0);
        node.scrollbars = Some(cssimpler_core::ScrollbarData {
            overflow_x: Default::default(),
            overflow_y: Default::default(),
            style: Default::default(),
            interaction: Default::default(),
            metrics: cssimpler_core::ScrollbarMetrics {
                offset_x: 5.0,
                max_offset_x: 20.0,
                ..Default::default()
            },
        });
        let scrolled = ime_cursor_area(&[node.clone()], &path).unwrap();
        // Centered text gains half of the extra content width, then scrolls left.
        assert!((scrolled.x - rect.x - 5.0).abs() < 0.001);
        let rect = scrolled;
        let mut parent = RenderNode::container(LayoutBox::default()).with_child(node);
        parent
            .style
            .transform
            .operations
            .push(cssimpler_core::TransformOperation::Scale { x: 2.0, y: 2.0 });
        let scaled = ime_cursor_area(&[parent], &path).unwrap();
        assert_eq!(
            scaled,
            LayoutBox::new(
                rect.x * 2.0,
                rect.y * 2.0,
                rect.width * 2.0,
                rect.height * 2.0
            )
        );
        let mut empty = RenderNode::container(LayoutBox::new(7.0, 9.0, 100.0, 30.0))
            .with_element_path(path.clone())
            .with_text_edit(TextEditDecoration {
                caret: Some(0),
                ..Default::default()
            });
        empty.content_inset.left = 4.0;
        assert_eq!(ime_cursor_area(&[empty], &path).unwrap().x, 11.0);
    }

    #[test]
    fn ime_underlines_are_visible_even_without_a_caret_or_selection() {
        let node = RenderNode::text(LayoutBox::new(10.0, 10.0, 120.0, 1.0), "    ").with_style(
            VisualStyle {
                foreground: Color::BLACK,
                text: TextStyle {
                    white_space: WhiteSpace::Pre,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        let mut buffer = vec![0; 160 * 80];
        super::super::render_to_buffer(&[node.clone()], &mut buffer, 160, 80, Color::WHITE);
        assert!(
            buffer
                .iter()
                .all(|pixel| *pixel == super::super::pack_rgb(Color::WHITE))
        );
        let edit = TextEditDecoration {
            composition: Some(0..4),
            ..Default::default()
        };
        let line = composition_underlines(node.layout, "    ", None, &node.style.text, &edit)[0];
        let composing = node.clone().with_text_edit(edit);
        super::super::render_to_buffer(&[composing.clone()], &mut buffer, 160, 80, Color::WHITE);
        let x = (line.x + line.width * 0.5) as usize;
        let y = (line.y + 0.5) as usize;
        assert_ne!(buffer[y * 160 + x], super::super::pack_rgb(Color::WHITE));
        super::super::render_scene_update(
            &[composing],
            &[node],
            &mut buffer,
            160,
            80,
            Color::WHITE,
        );
        assert!(
            buffer
                .iter()
                .all(|pixel| *pixel == super::super::pack_rgb(Color::WHITE))
        );
    }

    #[test]
    fn ime_underlines_render_and_incremental_updates_match_full_paint_at_every_scale() {
        let node = RenderNode::text(LayoutBox::new(10.0, 10.0, 150.0, 40.0), "ab日本").with_style(
            VisualStyle {
                foreground: Color::BLACK,
                ..Default::default()
            },
        );
        let composing = node.clone().with_text_edit(TextEditDecoration {
            composition: Some(2..8),
            caret: Some(8),
            selection: Some(TextSelectionRange {
                start: 2,
                end: 5,
                style: Default::default(),
            }),
            backgrounds: None,
        });
        let underlines = composition_underlines(
            node.layout,
            "ab日本",
            None,
            &node.style.text,
            composing.text_edit.as_ref().unwrap(),
        );
        assert_eq!(underlines.len(), 1);
        assert!(underlines[0].width > 0.0);
        for scale in [1.0, 1.25, 2.0] {
            let original = super::super::dpi::device_scene(vec![node.clone()], scale);
            let preview = super::super::dpi::device_scene(vec![composing.clone()], scale);
            let mut actual = vec![0; 400 * 160];
            let mut expected = actual.clone();
            super::super::render_to_buffer(&original, &mut actual, 400, 160, Color::WHITE);
            let before = actual.clone();
            super::super::render_scene_update(
                &original,
                &preview,
                &mut actual,
                400,
                160,
                Color::WHITE,
            );
            super::super::render_to_buffer(&preview, &mut expected, 400, 160, Color::WHITE);
            assert_ne!(actual, before);
            assert_eq!(actual, expected, "preedit at {scale}x");
            super::super::render_scene_update(
                &preview,
                &original,
                &mut actual,
                400,
                160,
                Color::WHITE,
            );
            assert_eq!(actual, before, "cancel at {scale}x");
        }
    }

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
                    composition: None,
                    backgrounds: None,
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
        assert_ne!(pixels[9 * 120 + 8], super::super::pack_rgb(color));
    }
}
