use std::collections::HashMap;

use cssimpler_core::{RenderNode, ScrollState, ScrollTargetRelation};

pub(super) fn append_relations(scene: &[RenderNode], states: &mut Vec<ScrollState>) {
    let mut groups = Vec::new();
    collect_groups(scene, &mut groups);
    if groups.is_empty() {
        return;
    }

    let mut targets = HashMap::new();
    index_targets(scene, None, &mut targets);
    for group in groups {
        let mut markers = Vec::new();
        collect_markers(&group.children, &targets, &mut markers);
        let Some((_, _, scroller)) = markers.first().copied() else {
            continue;
        };
        markers.retain(|(_, _, owner)| std::ptr::eq(*owner, scroller));
        let metrics = scroller
            .scrollbars
            .expect("indexed scroll container")
            .metrics;
        let vertical = metrics.can_scroll_y() || !metrics.can_scroll_x();
        let position = |node: &RenderNode| {
            if vertical {
                node.layout.y
            } else {
                node.layout.x
            }
        };
        let offset = if vertical {
            metrics.offset_y
        } else {
            metrics.offset_x
        };
        let max_offset = if vertical {
            metrics.max_offset_y
        } else {
            metrics.max_offset_x
        };
        markers.sort_by(|(_, a, _), (_, b, _)| position(a).total_cmp(&position(b)));
        let edge = position(scroller)
            + if vertical {
                scroller.content_inset.top
            } else {
                scroller.content_inset.left
            };
        let active = if max_offset > 0.0 && offset >= max_offset {
            markers.len() - 1
        } else {
            markers
                .iter()
                .rposition(|(_, target, _)| position(target) <= edge + 0.5)
                .unwrap_or(0)
        };
        for (index, (marker, _, _)) in markers.into_iter().enumerate() {
            let Some(path) = &marker.element_path else {
                continue;
            };
            states.push(ScrollState {
                element: path.clone(),
                scrollable: Default::default(),
                query_container: false,
                names: Vec::new(),
                target_relation: Some(match index.cmp(&active) {
                    std::cmp::Ordering::Less => ScrollTargetRelation::Before,
                    std::cmp::Ordering::Equal => ScrollTargetRelation::Current,
                    std::cmp::Ordering::Greater => ScrollTargetRelation::After,
                }),
            });
        }
    }
}

fn collect_groups<'a>(nodes: &'a [RenderNode], groups: &mut Vec<&'a RenderNode>) {
    for node in nodes {
        if node.style.scroll_target_group {
            groups.push(node);
        }
        collect_groups(&node.children, groups);
    }
}

type Target<'a> = (&'a RenderNode, &'a RenderNode);

fn index_targets<'a>(
    nodes: &'a [RenderNode],
    scroller: Option<&'a RenderNode>,
    targets: &mut HashMap<&'a str, Target<'a>>,
) {
    for node in nodes {
        if let (Some(id), Some(owner)) = (node.element_id.as_deref(), scroller) {
            targets.entry(id).or_insert((node, owner));
        }
        let owner = if node.scrollbars.is_some() {
            Some(node)
        } else {
            scroller
        };
        index_targets(&node.children, owner, targets);
    }
}

fn collect_markers<'a>(
    nodes: &'a [RenderNode],
    targets: &HashMap<&str, Target<'a>>,
    markers: &mut Vec<(&'a RenderNode, &'a RenderNode, &'a RenderNode)>,
) {
    for node in nodes {
        if node.style.scroll_target_group {
            continue;
        }
        if let Some((target, owner)) = node
            .style
            .scroll_target_href
            .as_deref()
            .and_then(|href| targets.get(href))
        {
            markers.push((node, *target, *owner));
        }
        collect_markers(&node.children, targets, markers);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cssimpler_core::{
        ElementPath, LayoutBox, OverflowMode, ScrollbarData, ScrollbarMetrics, ScrollbarStyle,
    };

    #[test]
    fn scroll_geometry_identifies_the_current_marker_and_neighbors() {
        let mut scroller = RenderNode::container(LayoutBox::new(0.0, 0.0, 70.0, 50.0))
            .with_scrollbars(ScrollbarData::new(
                OverflowMode::Hidden,
                OverflowMode::Auto,
                ScrollbarStyle::default(),
                ScrollbarMetrics {
                    offset_y: 60.0,
                    max_offset_y: 120.0,
                    ..Default::default()
                },
            ));
        let mut group = RenderNode::container(LayoutBox::new(80.0, 0.0, 30.0, 50.0));
        group.style.scroll_target_group = true;
        for (index, y) in [-60.0, 0.0, 60.0].into_iter().enumerate() {
            let name = format!("target-{index}");
            scroller.children.push(
                RenderNode::container(LayoutBox::new(0.0, y, 60.0, 45.0))
                    .with_element_id(name.clone()),
            );
            let mut marker =
                RenderNode::container(LayoutBox::new(80.0, 10.0 * index as f32, 30.0, 10.0))
                    .with_element_path(ElementPath::root(1).with_child(index));
            marker.style.scroll_target_href = Some(name);
            group.children.push(marker);
        }
        let mut states = Vec::new();
        append_relations(&[scroller, group], &mut states);
        assert_eq!(
            states
                .into_iter()
                .map(|state| state.target_relation.unwrap())
                .collect::<Vec<_>>(),
            [
                ScrollTargetRelation::Before,
                ScrollTargetRelation::Current,
                ScrollTargetRelation::After
            ]
        );
    }
}
