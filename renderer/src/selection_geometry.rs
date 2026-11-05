//! Build a non-overlapping selection backdrop before painting the original glyphs.
use cssimpler_core::{
    Color, CornerRadius, LayoutBox, RenderKind, RenderNode, TextSelectionBackground,
};

const ALIGN_DISTANCE: f32 = 20.0;
const PAD_X: f32 = 2.0;
const PAD_Y: f32 = 1.0;
const EPS: f32 = 0.001;

#[derive(Clone)]
struct Fragment {
    owner: usize,
    group: usize,
    rect: LayoutBox,
    color: Color,
    radius: f32,
}

/// Prepare connected selection backgrounds across text nodes in the same paint
/// and clip context. Geometry stays relative to the owner's text origin, so
/// scrolling, transforms and display density do not move the original glyphs.
pub fn prepare_text_selection_backgrounds(scene: &mut [RenderNode]) {
    fn collect(
        node: &RenderNode,
        parent_group: usize,
        next_group: &mut usize,
        owners: &mut Vec<(LayoutBox, Vec<TextSelectionBackground>)>,
        fragments: &mut Vec<Fragment>,
    ) {
        let separate = !node.style.transform.is_identity()
            || node.style.overflow.clips_any_axis()
            || node.style.background.is_some()
            || !node.style.border.widths.is_zero();
        let group = if separate {
            *next_group += 1;
            *next_group
        } else {
            parent_group
        };
        if let RenderKind::Text(text) = &node.kind
            && let Some(selection) = node
                .text_edit
                .as_ref()
                .and_then(|edit| edit.selection.as_ref())
        {
            let owner = owners.len();
            let layout = super::scrollbar::text_layout(node);
            owners.push((layout, Vec::new()));
            let segments = super::text_range_segments(
                layout,
                text,
                node.text_layout.as_ref(),
                &node.style.text,
                selection.start,
                selection.end,
            );
            fragments.extend(segments.into_iter().map(|segment| Fragment {
                owner,
                group,
                rect: segment.layout,
                color: selection.style.background,
                radius: node.style.text.size_px * 0.1,
            }));
        }
        for child in &node.children {
            collect(child, group, next_group, owners, fragments);
        }
    }
    fn apply(
        node: &mut RenderNode,
        next_owner: &mut usize,
        owners: &mut [(LayoutBox, Vec<TextSelectionBackground>)],
    ) {
        if matches!(node.kind, RenderKind::Text(_))
            && let Some(edit) = &mut node.text_edit
        {
            if edit.selection.is_some() {
                edit.backgrounds = Some(std::mem::take(&mut owners[*next_owner].1));
                *next_owner += 1;
            } else {
                edit.backgrounds = None;
            }
        }
        for child in &mut node.children {
            apply(child, next_owner, owners);
        }
    }
    let mut owners = Vec::new();
    let mut fragments = Vec::new();
    let mut next_group = 0;
    for node in scene.iter() {
        next_group += 1;
        collect(
            node,
            next_group,
            &mut next_group,
            &mut owners,
            &mut fragments,
        );
    }
    for (owner, rect, radius) in build(fragments) {
        let origin = owners[owner].0;
        owners[owner].1.push(TextSelectionBackground {
            rect: LayoutBox::new(
                rect.x - origin.x,
                rect.y - origin.y,
                rect.width,
                rect.height,
            ),
            radius,
        });
    }
    let mut next_owner = 0;
    for node in scene {
        apply(node, &mut next_owner, &mut owners);
    }
}

pub(super) fn local_backgrounds(
    layout: LayoutBox,
    segments: &[super::TextRangeSegment],
    radius: f32,
) -> Vec<TextSelectionBackground> {
    let fragments = segments
        .iter()
        .map(|segment| Fragment {
            owner: 0,
            group: 0,
            rect: segment.layout,
            color: Color::BLACK,
            radius,
        })
        .collect();
    build(fragments)
        .into_iter()
        .map(|(_, rect, radius)| TextSelectionBackground {
            rect: LayoutBox::new(
                rect.x - layout.x,
                rect.y - layout.y,
                rect.width,
                rect.height,
            ),
            radius,
        })
        .collect()
}

pub(super) fn absolute(origin: LayoutBox, piece: &TextSelectionBackground) -> LayoutBox {
    LayoutBox::new(
        origin.x + piece.rect.x,
        origin.y + piece.rect.y,
        piece.rect.width,
        piece.rect.height,
    )
}

fn right(rect: LayoutBox) -> f32 {
    rect.x + rect.width
}
fn bottom(rect: LayoutBox) -> f32 {
    rect.y + rect.height
}
fn touches(a: LayoutBox, b: LayoutBox) -> bool {
    a.x <= right(b) + EPS
        && b.x <= right(a) + EPS
        && a.y <= bottom(b) + EPS
        && b.y <= bottom(a) + EPS
}

fn align_edges(component: &mut [Fragment], left: bool) {
    let mut indices: Vec<_> = (0..component.len()).collect();
    let edge = |fragment: &Fragment| {
        if left {
            fragment.rect.x
        } else {
            right(fragment.rect)
        }
    };
    indices.sort_by(|&a, &b| edge(&component[a]).total_cmp(&edge(&component[b])));
    let mut start = 0;
    while start < indices.len() {
        let minimum = edge(&component[indices[start]]);
        let mut end = start + 1;
        // A bounded cluster avoids a 0,19,38px chain snapping all the way to 0.
        while end < indices.len()
            && edge(&component[indices[end]]) - minimum <= ALIGN_DISTANCE + EPS
        {
            end += 1;
        }
        let maximum = edge(&component[indices[end - 1]]);
        for &index in &indices[start..end] {
            let rect = &mut component[index].rect;
            if left {
                rect.width = right(*rect) - minimum;
                rect.x = minimum;
            } else {
                rect.width = maximum - rect.x;
            }
        }
        start = end;
    }
}

fn build(mut fragments: Vec<Fragment>) -> Vec<(usize, LayoutBox, CornerRadius)> {
    for fragment in &mut fragments {
        fragment.rect.x -= PAD_X;
        fragment.rect.y -= PAD_Y;
        fragment.rect.width += 2.0 * PAD_X;
        fragment.rect.height += 2.0 * PAD_Y;
    }
    let mut result = Vec::new();
    let mut seen = vec![false; fragments.len()];
    for first in 0..fragments.len() {
        if seen[first] {
            continue;
        }
        let mut indices = vec![first];
        seen[first] = true;
        let mut cursor = 0;
        while cursor < indices.len() {
            let current = indices[cursor];
            for next in 0..fragments.len() {
                if !seen[next]
                    && fragments[next].group == fragments[first].group
                    && fragments[next].color == fragments[first].color
                    && touches(fragments[current].rect, fragments[next].rect)
                {
                    seen[next] = true;
                    indices.push(next);
                }
            }
            cursor += 1;
        }
        let mut component: Vec<_> = indices.iter().map(|&i| fragments[i].clone()).collect();
        align_edges(&mut component, true);
        align_edges(&mut component, false);
        let owner = component
            .iter()
            .map(|fragment| fragment.owner)
            .min()
            .unwrap();
        let radius = component
            .iter()
            .map(|fragment| fragment.radius)
            .fold(f32::INFINITY, f32::min);
        let mut edges: Vec<_> = component
            .iter()
            .flat_map(|fragment| [fragment.rect.y, bottom(fragment.rect)])
            .collect();
        edges.sort_by(f32::total_cmp);
        edges.dedup_by(|a, b| (*a - *b).abs() < EPS);
        let mut bands: Vec<LayoutBox> = Vec::new();
        for pair in edges.windows(2) {
            let middle = (pair[0] + pair[1]) * 0.5;
            let mut spans: Vec<_> = component
                .iter()
                .filter(|fragment| fragment.rect.y <= middle && bottom(fragment.rect) >= middle)
                .map(|fragment| (fragment.rect.x, right(fragment.rect)))
                .collect();
            spans.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut merged: Vec<(f32, f32)> = Vec::new();
            for (left, right) in spans {
                if let Some(last) = merged.last_mut()
                    && left <= last.1 + EPS
                {
                    last.1 = last.1.max(right);
                } else {
                    merged.push((left, right));
                }
            }
            for (left, right) in merged {
                if let Some(previous) = bands.iter_mut().rev().find(|previous| {
                    (previous.x - left).abs() < EPS
                        && (super::selection_geometry::right(**previous) - right).abs() < EPS
                        && (bottom(**previous) - pair[0]).abs() < EPS
                }) {
                    previous.height = pair[1] - previous.y;
                } else {
                    bands.push(LayoutBox::new(
                        left,
                        pair[0],
                        right - left,
                        pair[1] - pair[0],
                    ));
                }
            }
        }
        for rect in &bands {
            let covered = |x, y| {
                bands.iter().any(|other| {
                    x >= other.x && x < right(*other) && y >= other.y && y < bottom(*other)
                })
            };
            result.push((
                owner,
                *rect,
                CornerRadius {
                    top_left: if covered(rect.x + EPS, rect.y - EPS) {
                        0.0
                    } else {
                        radius
                    },
                    top_right: if covered(right(*rect) - EPS, rect.y - EPS) {
                        0.0
                    } else {
                        radius
                    },
                    bottom_left: if covered(rect.x + EPS, bottom(*rect) + EPS) {
                        0.0
                    } else {
                        radius
                    },
                    bottom_right: if covered(right(*rect) - EPS, bottom(*rect) + EPS) {
                        0.0
                    } else {
                        radius
                    },
                },
            ));
        }
    }
    result
}

/// Rasterize the union once. Compositing antialiased rectangles separately can
/// leave a faint shared edge at fractional scales, even when their geometry only
/// touches and their interiors never overlap.
pub(super) fn paint_background(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    origin: LayoutBox,
    pieces: &[TextSelectionBackground],
    color: Color,
    matrix: super::AffineTransform,
    clip: &super::ClipState,
) {
    if color.a == 0 || pieces.is_empty() {
        return;
    }
    if let [piece] = pieces {
        super::draw_transformed_rounded_rect(
            buffer,
            width,
            height,
            absolute(origin, piece),
            piece.radius,
            color,
            matrix,
            clip,
        );
        return;
    }
    let Some(inverse) = matrix.invert() else {
        return;
    };
    let pieces: Vec<_> = pieces
        .iter()
        .map(|piece| (absolute(origin, piece), piece.radius))
        .collect();
    let bounds = pieces
        .iter()
        .filter_map(|(rect, _)| super::transform_layout_bounds(*rect, matrix))
        .reduce(|a, b| a.union(b))
        .and_then(|bounds| bounds.intersect(clip.coarse));
    let Some(bounds) = bounds else {
        return;
    };
    let Some((x0, y0, x1, y1)) = super::clip_pixel_bounds(bounds, width, height) else {
        return;
    };
    let color_prepared = super::PreparedBlendColor::new(color);
    let rows = super::current_render_buffer_rows();
    let offsets = [0.125, 0.375, 0.625, 0.875];
    for y in y0.max(rows.start as i32)..y1.min(rows.end as i32) {
        for x in x0..x1 {
            let mut samples = 0;
            for offset_y in offsets {
                for offset_x in offsets {
                    let screen_x = x as f32 + offset_x;
                    let screen_y = y as f32 + offset_y;
                    if !clip.contains(screen_x, screen_y) {
                        continue;
                    }
                    let (source_x, source_y) = inverse.transform_point(screen_x, screen_y);
                    samples += u32::from(pieces.iter().any(|(rect, radius)| {
                        super::shapes::point_in_rounded_rect(source_x, source_y, *rect, *radius)
                    }));
                }
            }
            if samples > 0 {
                let coverage = ((samples * 255 + 8) / 16) as u8;
                super::blend_prepared_pixel_with_coverage(
                    buffer,
                    width,
                    height,
                    x,
                    y,
                    color_prepared,
                    color.a,
                    coverage,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fragment(owner: usize, x: f32, y: f32, width: f32) -> Fragment {
        Fragment {
            owner,
            group: 0,
            rect: LayoutBox::new(x, y, width, 20.0),
            color: Color::BLACK,
            radius: 2.0,
        }
    }

    #[test]
    fn close_edges_align_and_padding_produces_one_block() {
        let shape = build(vec![
            fragment(0, 10.0, 10.0, 100.0),
            fragment(1, 25.0, 30.0, 100.0),
        ]);
        assert_eq!(shape.len(), 1);
        assert_eq!(shape[0].0, 0);
        assert_eq!(shape[0].1, LayoutBox::new(8.0, 9.0, 119.0, 42.0));
        assert_eq!(shape[0].2, CornerRadius::all(2.0));
    }

    #[test]
    fn stepped_blocks_round_only_exposed_corners_and_never_overlap() {
        let shape = build(vec![
            fragment(0, 10.0, 10.0, 100.0),
            fragment(1, 10.0, 30.0, 200.0),
        ]);
        assert_eq!(shape.len(), 2);
        assert_eq!(shape[0].2.bottom_left, 0.0);
        assert_eq!(shape[0].2.bottom_right, 0.0);
        assert_eq!(shape[1].2.top_left, 0.0);
        assert_eq!(shape[1].2.top_right, 2.0);
        assert_eq!(bottom(shape[0].1), shape[1].1.y);
    }

    #[test]
    fn overlapping_rows_are_painted_once_and_large_edge_steps_remain() {
        let shape = build(vec![
            fragment(0, 10.0, 10.0, 100.0),
            fragment(1, 40.0, 20.0, 160.0),
        ]);
        for (i, (_, a, _)) in shape.iter().enumerate() {
            for (_, b, _) in &shape[i + 1..] {
                assert!(
                    right(*a) <= b.x || right(*b) <= a.x || bottom(*a) <= b.y || bottom(*b) <= a.y
                );
            }
        }
        assert!(shape.iter().any(|(_, rect, _)| rect.x == 38.0));
    }

    #[test]
    fn edge_alignment_has_a_bounded_twenty_pixel_range() {
        let mut fragments = vec![
            fragment(0, 0.0, 0.0, 100.0),
            fragment(0, 19.0, 20.0, 100.0),
            fragment(0, 38.0, 40.0, 100.0),
        ];
        align_edges(&mut fragments, true);
        assert_eq!(fragments[0].rect.x, 0.0);
        assert_eq!(fragments[1].rect.x, 0.0);
        assert_eq!(fragments[2].rect.x, 38.0);
    }

    fn selected_rows() -> Vec<RenderNode> {
        use cssimpler_core::fonts::{TextStyle, WhiteSpace};
        use cssimpler_core::{
            TextEditDecoration, TextSelectionRange, TextSelectionStyle, VisualStyle,
        };
        let children = ["     ", "      ", "     "]
            .into_iter()
            .enumerate()
            .map(|(i, text)| {
                RenderNode::text(
                    LayoutBox::new(20.0, 20.0 + i as f32 * 20.0, 180.0, 20.0),
                    text,
                )
                .with_style(VisualStyle {
                    text: TextStyle {
                        size_px: 16.0,
                        white_space: WhiteSpace::Pre,
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .with_text_edit(TextEditDecoration {
                    caret: None,
                    backgrounds: None,
                    selection: Some(TextSelectionRange {
                        start: 0,
                        end: text.len(),
                        style: TextSelectionStyle {
                            background: Color::rgba(100, 180, 240, 128),
                            ..Default::default()
                        },
                    }),
                })
            })
            .collect::<Vec<_>>();
        vec![RenderNode::container(LayoutBox::new(0.0, 0.0, 220.0, 110.0)).with_children(children)]
    }

    #[test]
    fn selections_across_nodes_share_one_translucent_backdrop() {
        let mut scene = selected_rows();
        prepare_text_selection_backgrounds(&mut scene);
        let first = scene[0].children[0].text_edit.as_ref().unwrap();
        assert_eq!(first.backgrounds.as_ref().unwrap().len(), 1);
        for node in &scene[0].children[1..] {
            assert!(
                node.text_edit
                    .as_ref()
                    .unwrap()
                    .backgrounds
                    .as_ref()
                    .unwrap()
                    .is_empty()
            );
        }
        let mut pixels = vec![0; 220 * 110];
        crate::render_to_buffer(&scene, &mut pixels, 220, 110, Color::WHITE);
        for y in 21..79 {
            assert_eq!(
                pixels[y * 220 + 22],
                pixels[30 * 220 + 22],
                "join at y={y} must not have a rounded gap or doubled opacity"
            );
        }
    }

    #[test]
    fn clearing_a_selection_erases_the_whole_padded_connected_shape() {
        let mut selected = selected_rows();
        prepare_text_selection_backgrounds(&mut selected);
        let mut clear = selected.clone();
        for node in &mut clear[0].children {
            node.text_edit = None;
        }
        let mut actual = vec![0; 220 * 110];
        crate::render_to_buffer(&selected, &mut actual, 220, 110, Color::WHITE);
        crate::render_scene_update(&selected, &clear, &mut actual, 220, 110, Color::WHITE);
        let mut expected = vec![0; actual.len()];
        crate::render_to_buffer(&clear, &mut expected, 220, 110, Color::WHITE);
        assert_eq!(
            actual, expected,
            "no padded or merged highlight pixels may remain"
        );
    }

    #[test]
    fn translucent_stepped_blocks_have_no_shared_edge_at_fractional_scales() {
        let mut scene = selected_rows();
        if let RenderKind::Text(text) = &mut scene[0].children[1].kind {
            *text = "                                        ".into();
        }
        let node = &mut scene[0].children[1];
        node.text_edit
            .as_mut()
            .unwrap()
            .selection
            .as_mut()
            .unwrap()
            .end = 40;
        prepare_text_selection_backgrounds(&mut scene);
        assert!(
            scene[0].children[0]
                .text_edit
                .as_ref()
                .unwrap()
                .backgrounds
                .as_ref()
                .unwrap()
                .len()
                > 1
        );
        for scale in [1.0, 1.2, 1.25, 1.5, 2.0] {
            let mut pixels = vec![0; 600 * 220];
            crate::render_to_buffer_at_scale(&scene, &mut pixels, 600, 220, Color::WHITE, scale);
            let x = (23.0 * scale) as usize;
            let reference = pixels[(30.0 * scale) as usize * 600 + x];
            for y in (23.0 * scale).ceil() as usize..(77.0 * scale).floor() as usize {
                assert_eq!(
                    pixels[y * 600 + x],
                    reference,
                    "highlight seam at {scale}x, row {y}"
                );
            }
        }
    }
}
