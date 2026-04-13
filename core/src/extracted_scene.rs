use crate::{
    ElementPath, EventHandlers, Insets, LayoutBox, PreparedTextLayout, RenderKind, RenderNode,
    ScrollbarData, SvgScene, Transform2D, TransitionStyle, VisualStyle,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExtractedPaintKind {
    BackdropBlur,
    BoxShadow,
    FilterDropShadow,
    Background,
    Border,
    TextRun,
    Svg,
    Scrollbar,
}

#[derive(Clone, Debug)]
pub struct ExtractedPaintItem {
    pub stable_sort_key: u64,
    pub path: Vec<usize>,
    pub kind: ExtractedPaintKind,
    pub layout: LayoutBox,
    pub clip: Option<LayoutBox>,
    pub transform: Transform2D,
    pub style: VisualStyle,
    pub transitions: TransitionStyle,
    pub text: Option<String>,
    pub text_layout: Option<PreparedTextLayout>,
    pub svg_scene: Option<SvgScene>,
    pub element_id: Option<String>,
    pub element_path: Option<ElementPath>,
    pub content_inset: Insets,
    pub scrollbars: Option<ScrollbarData>,
    pub handlers: EventHandlers,
}

#[derive(Clone, Debug, Default)]
pub struct ExtractedScene {
    pub roots: Vec<RenderNode>,
    pub items: Vec<ExtractedPaintItem>,
}

impl ExtractedScene {
    pub fn from_render_roots(roots: &[RenderNode]) -> Self {
        let mut items = Vec::new();
        let mut next_sort_key = 0_u64;
        for (root_index, root) in roots.iter().enumerate() {
            collect_paint_items(root, vec![root_index], None, &mut next_sort_key, &mut items);
        }

        Self {
            roots: roots.to_vec(),
            items,
        }
    }
}

fn collect_paint_items(
    node: &RenderNode,
    path: Vec<usize>,
    inherited_clip: Option<LayoutBox>,
    next_sort_key: &mut u64,
    items: &mut Vec<ExtractedPaintItem>,
) {
    let clip = combine_clips(
        inherited_clip,
        node.style.overflow.clips_any_axis().then_some(node.layout),
    );

    if node.style.backdrop_blur_radius > f32::EPSILON {
        push_item(
            node,
            &path,
            ExtractedPaintKind::BackdropBlur,
            clip,
            None,
            next_sort_key,
            items,
        );
    }

    for _ in &node.style.shadows {
        push_item(
            node,
            &path,
            ExtractedPaintKind::BoxShadow,
            clip,
            None,
            next_sort_key,
            items,
        );
    }

    if !matches!(node.kind, RenderKind::Text(_)) {
        for _ in &node.style.filter_drop_shadows {
            push_item(
                node,
                &path,
                ExtractedPaintKind::FilterDropShadow,
                clip,
                None,
                next_sort_key,
                items,
            );
        }
    }

    if node.style.background.is_some() || !node.style.background_layers.is_empty() {
        push_item(
            node,
            &path,
            ExtractedPaintKind::Background,
            clip,
            None,
            next_sort_key,
            items,
        );
    }

    if !node.style.border.widths.is_zero() {
        push_item(
            node,
            &path,
            ExtractedPaintKind::Border,
            clip,
            None,
            next_sort_key,
            items,
        );
    }

    match &node.kind {
        RenderKind::Container => {}
        RenderKind::Text(text) => {
            push_item(
                node,
                &path,
                ExtractedPaintKind::TextRun,
                clip,
                Some(ExtractedPayload::Text(text.clone())),
                next_sort_key,
                items,
            );
        }
        RenderKind::Svg(scene) => {
            push_item(
                node,
                &path,
                ExtractedPaintKind::Svg,
                clip,
                Some(ExtractedPayload::Svg(scene.clone())),
                next_sort_key,
                items,
            );
        }
    }

    for (child_index, child) in node.children.iter().enumerate() {
        let mut child_path = path.clone();
        child_path.push(child_index);
        collect_paint_items(child, child_path, clip, next_sort_key, items);
    }

    if node.scrollbars.is_some() {
        push_item(
            node,
            &path,
            ExtractedPaintKind::Scrollbar,
            clip,
            None,
            next_sort_key,
            items,
        );
    }
}

fn combine_clips(left: Option<LayoutBox>, right: Option<LayoutBox>) -> Option<LayoutBox> {
    match (left, right) {
        (Some(left), Some(right)) => Some(intersect_layout_boxes(left, right)),
        (Some(clip), None) | (None, Some(clip)) => Some(clip),
        (None, None) => None,
    }
}

fn intersect_layout_boxes(left: LayoutBox, right: LayoutBox) -> LayoutBox {
    let x0 = left.x.max(right.x);
    let y0 = left.y.max(right.y);
    let x1 = (left.x + left.width).min(right.x + right.width);
    let y1 = (left.y + left.height).min(right.y + right.height);

    LayoutBox::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0))
}

enum ExtractedPayload {
    Text(String),
    Svg(SvgScene),
}

fn push_item(
    node: &RenderNode,
    path: &[usize],
    kind: ExtractedPaintKind,
    clip: Option<LayoutBox>,
    payload: Option<ExtractedPayload>,
    next_sort_key: &mut u64,
    items: &mut Vec<ExtractedPaintItem>,
) {
    let (text, svg_scene) = match payload {
        Some(ExtractedPayload::Text(text)) => (Some(text), None),
        Some(ExtractedPayload::Svg(scene)) => (None, Some(scene)),
        None => (None, None),
    };

    items.push(ExtractedPaintItem {
        stable_sort_key: *next_sort_key,
        path: path.to_vec(),
        kind,
        layout: node.layout,
        clip,
        transform: node.style.transform.clone(),
        style: node.style.clone(),
        transitions: node.transitions.clone(),
        text,
        text_layout: node.text_layout.clone(),
        svg_scene,
        element_id: node.element_id.clone(),
        element_path: node.element_path.clone(),
        content_inset: node.content_inset,
        scrollbars: node.scrollbars,
        handlers: node.handlers,
    });
    *next_sort_key = next_sort_key.saturating_add(1);
}

#[cfg(test)]
mod tests {
    use crate::{
        BoxShadow, Color, CornerRadius, Insets, LayoutBox, Overflow, PreparedTextLayout,
        RenderNode, ScrollbarData, ScrollbarMetrics, ScrollbarStyle, ScrollbarWidth, TextStyle,
        VisualStyle, fonts::TextLayout,
    };

    use super::{ExtractedPaintKind, ExtractedScene};

    #[test]
    fn extracted_scene_collects_backend_facing_paint_items() {
        let scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 160.0, 120.0))
                .with_style(VisualStyle {
                    overflow: Overflow {
                        x: crate::OverflowMode::Hidden,
                        y: crate::OverflowMode::Scroll,
                    },
                    background: Some(Color::rgb(15, 23, 42)),
                    border: crate::BorderStyle {
                        widths: Insets::all(1.0),
                        color: Color::rgb(226, 232, 240),
                    },
                    shadows: vec![BoxShadow {
                        color: Color::rgba(15, 23, 42, 140),
                        offset_x: 4.0,
                        offset_y: 6.0,
                        blur_radius: 8.0,
                        spread: 0.0,
                    }],
                    corner_radius: CornerRadius::all(12.0),
                    ..VisualStyle::default()
                })
                .with_scrollbars(ScrollbarData::new(
                    crate::OverflowMode::Hidden,
                    crate::OverflowMode::Scroll,
                    ScrollbarStyle {
                        width: ScrollbarWidth::Px(12.0),
                        ..ScrollbarStyle::default()
                    },
                    ScrollbarMetrics {
                        max_offset_y: 240.0,
                        reserved_width: 12.0,
                        ..ScrollbarMetrics::default()
                    },
                ))
                .with_child(
                    RenderNode::text(LayoutBox::new(16.0, 20.0, 80.0, 24.0), "hello")
                        .with_style(VisualStyle {
                            foreground: Color::WHITE,
                            text: TextStyle {
                                size_px: 18.0,
                                ..TextStyle::default()
                            },
                            ..VisualStyle::default()
                        })
                        .with_text_layout(PreparedTextLayout::new(
                            Some(80.0),
                            TextLayout {
                                width: 42.0,
                                height: 24.0,
                                line_height: 24.0,
                                lines: Vec::new(),
                            },
                        )),
                ),
        ];

        let extracted = ExtractedScene::from_render_roots(&scene);
        let kinds = extracted
            .items
            .iter()
            .map(|item| item.kind)
            .collect::<Vec<_>>();

        assert!(kinds.contains(&ExtractedPaintKind::BoxShadow));
        assert!(kinds.contains(&ExtractedPaintKind::Background));
        assert!(kinds.contains(&ExtractedPaintKind::Border));
        assert!(kinds.contains(&ExtractedPaintKind::TextRun));
        assert!(kinds.contains(&ExtractedPaintKind::Scrollbar));
    }

    #[test]
    fn extracted_scene_sort_keys_stay_deterministic() {
        let left = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 100.0, 80.0))
                .with_child(RenderNode::container(LayoutBox::new(8.0, 8.0, 20.0, 20.0)))
                .with_child(RenderNode::text(
                    LayoutBox::new(16.0, 16.0, 30.0, 12.0),
                    "stable",
                )),
        ];
        let right = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 100.0, 80.0))
                .with_child(RenderNode::container(LayoutBox::new(8.0, 8.0, 20.0, 20.0)))
                .with_child(RenderNode::text(
                    LayoutBox::new(16.0, 16.0, 30.0, 12.0),
                    "stable",
                )),
        ];

        let left_keys = ExtractedScene::from_render_roots(&left)
            .items
            .into_iter()
            .map(|item| item.stable_sort_key)
            .collect::<Vec<_>>();
        let right_keys = ExtractedScene::from_render_roots(&right)
            .items
            .into_iter()
            .map(|item| item.stable_sort_key)
            .collect::<Vec<_>>();

        assert_eq!(left_keys, right_keys);
    }

    #[test]
    fn extracted_scene_preserves_tree_paint_order_across_descendants_and_siblings() {
        let scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 160.0, 120.0))
                .with_child(
                    RenderNode::container(LayoutBox::new(8.0, 8.0, 48.0, 48.0))
                        .with_style(VisualStyle {
                            background: Some(Color::rgb(15, 23, 42)),
                            ..VisualStyle::default()
                        })
                        .with_child(
                            RenderNode::text(LayoutBox::new(12.0, 12.0, 24.0, 16.0), "first")
                                .with_style(VisualStyle {
                                    foreground: Color::WHITE,
                                    ..VisualStyle::default()
                                }),
                        ),
                )
                .with_child(
                    RenderNode::container(LayoutBox::new(72.0, 8.0, 48.0, 48.0)).with_style(
                        VisualStyle {
                            background: Some(Color::rgb(30, 41, 59)),
                            ..VisualStyle::default()
                        },
                    ),
                ),
        ];

        let extracted = ExtractedScene::from_render_roots(&scene);
        let order = extracted
            .items
            .iter()
            .map(|item| (item.path.clone(), item.kind))
            .collect::<Vec<_>>();

        assert_eq!(
            order,
            vec![
                (vec![0, 0], ExtractedPaintKind::Background),
                (vec![0, 0, 0], ExtractedPaintKind::TextRun),
                (vec![0, 1], ExtractedPaintKind::Background),
            ]
        );
    }

    #[test]
    fn extracted_scene_accumulates_ancestor_clips_for_backend_consumers() {
        let scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 120.0, 120.0))
                .with_style(VisualStyle {
                    overflow: Overflow {
                        x: crate::OverflowMode::Hidden,
                        y: crate::OverflowMode::Hidden,
                    },
                    ..VisualStyle::default()
                })
                .with_child(
                    RenderNode::container(LayoutBox::new(16.0, 12.0, 72.0, 48.0))
                        .with_style(VisualStyle {
                            overflow: Overflow {
                                x: crate::OverflowMode::Clip,
                                y: crate::OverflowMode::Clip,
                            },
                            background: Some(Color::rgb(15, 23, 42)),
                            ..VisualStyle::default()
                        })
                        .with_child(
                            RenderNode::text(LayoutBox::new(40.0, 30.0, 120.0, 24.0), "clipped")
                                .with_style(VisualStyle {
                                    foreground: Color::WHITE,
                                    ..VisualStyle::default()
                                }),
                        ),
                ),
        ];

        let extracted = ExtractedScene::from_render_roots(&scene);
        let text_item = extracted
            .items
            .iter()
            .find(|item| matches!(item.kind, ExtractedPaintKind::TextRun))
            .expect("text item should be extracted");

        assert_eq!(text_item.clip, Some(LayoutBox::new(16.0, 12.0, 72.0, 48.0)));
    }
}
