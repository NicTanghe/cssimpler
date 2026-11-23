use cssimpler_core::{RenderNode, ScrollDirections, ScrollState};

mod targets;

pub(crate) fn snapshot(scene: &[RenderNode]) -> Vec<ScrollState> {
    fn visit(node: &RenderNode, out: &mut Vec<ScrollState>) {
        if let Some(element) = &node.element_path
            && (node.scrollbars.is_some() || node.style.scroll_state_container)
        {
            let metrics = node.scrollbars.map(|bar| bar.metrics).unwrap_or_default();
            let allowed_x = node
                .scrollbars
                .is_some_and(|bar| bar.overflow_x.allows_scrolling());
            let allowed_y = node
                .scrollbars
                .is_some_and(|bar| bar.overflow_y.allows_scrolling());
            out.push(ScrollState {
                element: element.clone(),
                query_container: node.style.scroll_state_container,
                names: node.style.container_names.clone(),
                target_relation: None,
                scrollable: ScrollDirections {
                    left: allowed_x && metrics.offset_x > 0.0,
                    right: allowed_x && metrics.offset_x < metrics.max_offset_x,
                    up: allowed_y && metrics.offset_y > 0.0,
                    down: allowed_y && metrics.offset_y < metrics.max_offset_y,
                },
            });
        }
        for child in &node.children {
            visit(child, out);
        }
    }
    let mut states = Vec::new();
    for node in scene {
        visit(node, &mut states);
    }
    targets::append_relations(scene, &mut states);
    states
}

#[cfg(test)]
mod tests {
    use cssimpler_core::{
        ElementPath, LayoutBox, OverflowMode, ScrollbarData, ScrollbarMetrics, ScrollbarStyle,
    };

    use super::*;

    #[test]
    fn offset_and_overflow_control_available_scroll_directions() {
        let mut node = RenderNode::container(LayoutBox::new(0.0, 0.0, 100.0, 100.0))
            .with_element_path(ElementPath::root(0))
            .with_scrollbars(ScrollbarData::new(
                OverflowMode::Hidden,
                OverflowMode::Auto,
                ScrollbarStyle::default(),
                ScrollbarMetrics {
                    max_offset_x: 15.0,
                    max_offset_y: 25.0,
                    ..ScrollbarMetrics::default()
                },
            ));
        assert_eq!(
            snapshot(&[node.clone()])[0].scrollable,
            ScrollDirections {
                down: true,
                ..ScrollDirections::default()
            }
        );
        node.scrollbars.as_mut().unwrap().metrics.offset_y = 25.0;
        assert_eq!(
            snapshot(&[node])[0].scrollable,
            ScrollDirections {
                up: true,
                ..ScrollDirections::default()
            }
        );
    }
}
