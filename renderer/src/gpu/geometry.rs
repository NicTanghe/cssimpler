use super::error;
use crate::Result;
use cssimpler_core::{
    Color, CornerRadius, ExtractedPaintKind, ExtractedScene, LayoutBox, MixBlendMode, RenderKind,
    RenderNode, VisualStyle,
};
use std::collections::BTreeSet;

fn intersects(a: LayoutBox, b: LayoutBox) -> bool {
    a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
}

fn contains(a: LayoutBox, b: LayoutBox) -> bool {
    b.x >= a.x && b.y >= a.y && b.x + b.width <= a.x + a.width && b.y + b.height <= a.y + a.height
}

pub(super) struct Location {
    pub bounds: LayoutBox,
    pub background: Color,
    pub glass: bool,
    pub overlay: Vec<RenderNode>,
}

fn over(top: Color, bottom: Color) -> Color {
    let a = u32::from(top.a);
    let channel = |t: u8, b: u8| ((u32::from(t) * a + u32::from(b) * (255 - a) + 127) / 255) as u8;
    Color::rgb(
        channel(top.r, bottom.r),
        channel(top.g, bottom.g),
        channel(top.b, bottom.b),
    )
}

/// Locate the GPU rectangle and the later-painted CPU layer over it.
pub(super) fn locate(
    roots: &[RenderNode],
    id: &str,
    window: LayoutBox,
    fallback: Color,
) -> Result<Option<Location>> {
    fn find<'a>(
        nodes: &'a [RenderNode],
        id: &str,
        path: &mut Vec<usize>,
        matches: &mut Vec<(Vec<usize>, &'a RenderNode, bool)>,
        eligible: bool,
        clip: LayoutBox,
    ) {
        for (index, node) in nodes.iter().enumerate() {
            path.push(index);
            let supported = eligible
                && node.style.transform.is_identity()
                && node.style.perspective.is_none()
                && node.style.filter_drop_shadows.is_empty();
            let inset = node.content_inset;
            let content = LayoutBox::new(
                node.layout.x + inset.left,
                node.layout.y + inset.top,
                (node.layout.width - inset.left - inset.right).max(0.0),
                (node.layout.height - inset.top - inset.bottom).max(0.0),
            );
            if node.element_id.as_deref() == Some(id) {
                matches.push((
                    path.clone(),
                    node,
                    supported
                        && node.children.is_empty()
                        && node.style.corner_radius == CornerRadius::ZERO
                        && contains(clip, content),
                ));
            }
            let clips = node.style.overflow.clips_any_axis();
            let child_clip = if clips {
                let x = clip.x.max(content.x);
                let y = clip.y.max(content.y);
                LayoutBox::new(
                    x,
                    y,
                    (clip.x + clip.width).min(content.x + content.width) - x,
                    (clip.y + clip.height).min(content.y + content.height) - y,
                )
            } else {
                clip
            };
            find(
                &node.children,
                id,
                path,
                matches,
                supported && (!clips || node.style.corner_radius == CornerRadius::ZERO),
                child_clip,
            );
            path.pop();
        }
    }
    let mut matches = Vec::new();
    find(roots, id, &mut Vec::new(), &mut matches, true, window);
    if matches.len() > 1 {
        return Err(error(format!("duplicate element ID {id:?}")));
    }
    let Some((path, node, supported)) = matches.pop() else {
        return Ok(None);
    };
    if !supported {
        return Ok(None);
    }
    let inset = node.content_inset;
    let bounds = LayoutBox::new(
        node.layout.x + inset.left,
        node.layout.y + inset.top,
        node.layout.width - inset.left - inset.right,
        node.layout.height - inset.top - inset.bottom,
    );
    if ![bounds.x, bounds.y, bounds.width, bounds.height]
        .iter()
        .all(|v| v.is_finite())
        || bounds.width <= 0.0
        || bounds.height <= 0.0
        || !contains(window, bounds)
    {
        return Ok(None);
    }
    // Insert a transparent marker only into this temporary paint-order snapshot.
    let mut marked = roots.to_vec();
    let mut nodes = &mut marked;
    for (depth, index) in path.iter().enumerate() {
        if depth + 1 == path.len() {
            nodes[*index].style.background = Some(Color::rgba(0, 0, 0, 0));
            break;
        }
        nodes = &mut nodes[*index].children;
    }
    let extracted = ExtractedScene::from_render_roots_owned(marked);
    let mut overlay_start = None;
    let mut background = fallback;
    let mut glass = false;
    let mut has_overlap = false;
    for (index, item) in extracted.items.iter().enumerate() {
        if overlay_start.is_none() && contains(item.layout, bounds) {
            if item.kind == ExtractedPaintKind::GlassReveal {
                glass = true;
            }
            if item.kind == ExtractedPaintKind::Background {
                let color = if item.path == path {
                    node.style.background
                } else {
                    item.style.background
                };
                if let Some(color) = color {
                    background = over(color, background);
                    if color.a == 255 {
                        glass = false;
                    }
                }
            }
        }
        if item.path == path {
            if item.kind == ExtractedPaintKind::Background {
                overlay_start = Some(index + 1);
            }
            continue;
        }
        if overlay_start.is_none() || path.starts_with(&item.path) {
            continue;
        }
        if visible(item) && intersects(bounds, item.layout) {
            // These effects need pixels from beneath the overlay. The GPU
            // image is not available to the CPU rasterizer.
            if matches!(
                item.kind,
                ExtractedPaintKind::GlassReveal
                    | ExtractedPaintKind::BackdropOcclude
                    | ExtractedPaintKind::BackdropBlur
            ) || item.style.mix_blend_mode != MixBlendMode::Normal
            {
                return Ok(None);
            }
            has_overlap = true;
        }
    }
    let overlay = if has_overlap {
        let overlay_paths = extracted.items[overlay_start.unwrap()..]
            .iter()
            .filter(|item| {
                !path.starts_with(&item.path) && visible(item) && intersects(bounds, item.layout)
            })
            .map(|item| item.path.clone())
            .collect();
        let mut roots = extracted.roots;
        retain_paint(&mut roots, &mut Vec::new(), &overlay_paths);
        roots
    } else {
        Vec::new()
    };
    Ok(Some(Location {
        bounds,
        background,
        glass,
        overlay,
    }))
}

fn visible(item: &cssimpler_core::ExtractedPaintItem) -> bool {
    item.kind != ExtractedPaintKind::Background
        || item.style.background.is_some_and(|color| color.a != 0)
        || !item.style.background_layers.is_empty()
}

// Retain the original clip and stacking ancestors, clearing their own paint
// when it precedes the viewport. Paint order among surviving siblings stays
// the same, including positioned UI such as a dimmer and its dialog.
fn retain_paint(nodes: &mut Vec<RenderNode>, path: &mut Vec<usize>, paint: &BTreeSet<Vec<usize>>) {
    let mut index = 0;
    nodes.retain_mut(|node| {
        path.push(index);
        index += 1;
        let painted = paint.contains(path);
        retain_paint(&mut node.children, path, paint);
        path.pop();
        if !painted && !node.children.is_empty() {
            let style = &node.style;
            node.style = VisualStyle {
                cache_layer: style.cache_layer,
                isolate_blending: style.isolate_blending,
                corner_radius: style.corner_radius,
                positioned: style.positioned,
                z_index: style.z_index,
                overflow: style.overflow,
                transform: style.transform.clone(),
                perspective: style.perspective,
                transform_style: style.transform_style,
                ..VisualStyle::default()
            };
            node.kind = RenderKind::Container;
            node.text_layout = None;
            node.text_edit = None;
            node.scrollbars = None;
        }
        painted || !node.children.is_empty()
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn backdrop_follows_paint_order_and_opaque_background_covers_glass() {
        let window = LayoutBox::new(0.0, 0.0, 200.0, 200.0);
        let viewport = RenderNode::container(window).with_element_id("scene");
        let mut underneath = RenderNode::container(window);
        underneath.style.background = Some(Color::rgb(24, 38, 56));
        let result = locate(
            &[underneath.clone(), viewport.clone()],
            "scene",
            window,
            Color::WHITE,
        )
        .unwrap()
        .unwrap();
        assert_eq!(result.background, Color::rgb(24, 38, 56));
        assert!(!result.glass);
        underneath.style.native_material = cssimpler_core::NativeMaterial::Glass;
        underneath.style.background = None;
        assert!(
            locate(
                &[underneath.clone(), viewport.clone()],
                "scene",
                window,
                Color::WHITE
            )
            .unwrap()
            .unwrap()
            .glass
        );
        let mut opaque = viewport;
        opaque.style.background = Some(Color::BLACK);
        let result = locate(&[underneath, opaque], "scene", window, Color::WHITE)
            .unwrap()
            .unwrap();
        assert!(!result.glass);
        assert_eq!(result.background, Color::BLACK);
    }

    #[test]
    fn covering_ui_is_painted_over_the_surface() {
        let viewport = RenderNode::container(LayoutBox::new(10.0, 10.0, 100.0, 100.0))
            .with_element_id("scene");
        let mut modal = RenderNode::container(LayoutBox::new(20.0, 20.0, 50.0, 50.0));
        modal.style.background = Some(Color::BLACK);
        let window = LayoutBox::new(0.0, 0.0, 200.0, 200.0);
        let location = locate(&[viewport.clone(), modal], "scene", window, Color::WHITE)
            .unwrap()
            .unwrap();
        assert_eq!(location.bounds, viewport.layout);
        assert_eq!(location.overlay.len(), 1);
        assert_eq!(location.overlay[0].style.background, Some(Color::BLACK));
        let uncovered = locate(&[viewport], "scene", window, Color::WHITE)
            .unwrap()
            .unwrap();
        assert_eq!(uncovered.bounds, location.bounds);
        assert!(uncovered.overlay.is_empty());
    }
    #[test]
    fn partial_clipping_and_duplicate_ids_are_not_presented() {
        let viewport =
            RenderNode::container(LayoutBox::new(90.0, 0.0, 20.0, 20.0)).with_element_id("scene");
        let window = LayoutBox::new(0.0, 0.0, 100.0, 100.0);
        assert!(
            locate(&[viewport.clone()], "scene", window, Color::WHITE)
                .unwrap()
                .is_none()
        );
        assert!(locate(&[viewport.clone(), viewport], "scene", window, Color::WHITE).is_err());
    }
}
