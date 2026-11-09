use super::error;
use crate::Result;
use cssimpler_core::{
    Color, CornerRadius, ExtractedPaintKind, ExtractedScene, LayoutBox, RenderNode,
};

fn intersects(a: LayoutBox, b: LayoutBox) -> bool {
    a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
}

fn contains(a: LayoutBox, b: LayoutBox) -> bool {
    b.x >= a.x && b.y >= a.y && b.x + b.width <= a.x + a.width && b.y + b.height <= a.y + a.height
}

/// Return a fully visible content rectangle. Unsupported composition hides it.
pub(super) fn locate(
    roots: &[RenderNode],
    id: &str,
    window: LayoutBox,
) -> Result<Option<LayoutBox>> {
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
    let mut after = false;
    for item in extracted.items {
        if item.path == path {
            if item.kind == ExtractedPaintKind::Background {
                after = true;
            }
            continue;
        }
        if !after || path.starts_with(&item.path) {
            continue;
        }
        let visible = match item.kind {
            ExtractedPaintKind::Background => {
                item.style.background.is_some_and(|c| c.a != 0)
                    || !item.style.background_layers.is_empty()
            }
            _ => true,
        };
        if visible && intersects(bounds, item.layout) {
            return Ok(None);
        }
    }
    Ok(Some(bounds))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn covering_ui_hides_surface_and_removal_restores_it() {
        let viewport = RenderNode::container(LayoutBox::new(10.0, 10.0, 100.0, 100.0))
            .with_element_id("scene");
        let mut modal = RenderNode::container(LayoutBox::new(20.0, 20.0, 50.0, 50.0));
        modal.style.background = Some(Color::BLACK);
        let window = LayoutBox::new(0.0, 0.0, 200.0, 200.0);
        assert!(
            locate(&[viewport.clone(), modal], "scene", window)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            locate(&[viewport], "scene", window).unwrap().unwrap().width,
            100.0
        );
    }
    #[test]
    fn partial_clipping_and_duplicate_ids_are_not_presented() {
        let viewport =
            RenderNode::container(LayoutBox::new(90.0, 0.0, 20.0, 20.0)).with_element_id("scene");
        let window = LayoutBox::new(0.0, 0.0, 100.0, 100.0);
        assert!(
            locate(&[viewport.clone()], "scene", window)
                .unwrap()
                .is_none()
        );
        assert!(locate(&[viewport.clone(), viewport], "scene", window).is_err());
    }
}
