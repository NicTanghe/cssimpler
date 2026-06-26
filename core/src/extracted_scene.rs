use crate::{
    BackdropOcclusion, Color, ElementPath, EventHandlers, Insets, LayoutBox, NativeMaterial,
    PreparedTextLayout, RenderKind, RenderNode, ScrollbarData, SvgScene, Transform2D,
    TransitionStyle, VisualStyle, establishes_stacking_context, stacking_context_level,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtractedPaintKind {
    GlassReveal,
    BackdropOcclude,
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
        let mut stable_sort_key = 0;
        for (root_index, root) in roots.iter().enumerate() {
            collect_paint_items(root, vec![root_index], &mut stable_sort_key, &mut items);
        }
        items.sort_by_key(|item| item.stable_sort_key);

        Self {
            roots: roots.to_vec(),
            items,
        }
    }

    pub fn requires_native_glass(&self) -> bool {
        self.items
            .iter()
            .any(|item| item.kind == ExtractedPaintKind::GlassReveal)
    }

    pub fn glass_regions(&self) -> impl Iterator<Item = &ExtractedPaintItem> {
        self.items
            .iter()
            .filter(|item| item.kind == ExtractedPaintKind::GlassReveal)
    }

    pub fn preferred_glass_tint(&self) -> Option<Color> {
        self.glass_regions().find_map(|item| item.style.glass_tint)
    }
}

fn collect_paint_items(
    node: &RenderNode,
    path: Vec<usize>,
    stable_sort_key: &mut u64,
    items: &mut Vec<ExtractedPaintItem>,
) {
    collect_stacking_context_paint_items(node, path, stable_sort_key, items);
}

fn collect_stacking_context_paint_items(
    node: &RenderNode,
    path: Vec<usize>,
    stable_sort_key: &mut u64,
    items: &mut Vec<ExtractedPaintItem>,
) {
    push_node_paint_items_before_children(node, &path, stable_sort_key, items);

    let mut deferred = Vec::new();
    let mut order = 0;
    collect_deferred_stacking_contexts(node, &path, &mut order, &mut deferred);

    deferred.sort_by(|left, right| {
        left.level
            .cmp(&right.level)
            .then(left.order.cmp(&right.order))
    });

    for entry in deferred.iter().filter(|entry| entry.level < 0) {
        collect_stacking_context_paint_items(
            entry.node,
            entry.path.clone(),
            stable_sort_key,
            items,
        );
    }

    collect_normal_child_paint_items(node, &path, stable_sort_key, items);
    push_node_scrollbar_item(node, &path, stable_sort_key, items);

    for entry in deferred.iter().filter(|entry| entry.level >= 0) {
        collect_stacking_context_paint_items(
            entry.node,
            entry.path.clone(),
            stable_sort_key,
            items,
        );
    }
}

fn collect_normal_paint_items(
    node: &RenderNode,
    path: &[usize],
    stable_sort_key: &mut u64,
    items: &mut Vec<ExtractedPaintItem>,
) {
    push_node_paint_items_before_children(node, path, stable_sort_key, items);
    collect_normal_child_paint_items(node, path, stable_sort_key, items);
    push_node_scrollbar_item(node, path, stable_sort_key, items);
}

fn collect_normal_child_paint_items(
    node: &RenderNode,
    path: &[usize],
    stable_sort_key: &mut u64,
    items: &mut Vec<ExtractedPaintItem>,
) {
    for (child_index, child) in node.children.iter().enumerate() {
        if establishes_stacking_context(child) {
            continue;
        }
        let mut child_path = path.to_vec();
        child_path.push(child_index);
        collect_normal_paint_items(child, &child_path, stable_sort_key, items);
    }
}

#[derive(Clone)]
struct DeferredStackingContext<'a> {
    node: &'a RenderNode,
    path: Vec<usize>,
    level: i32,
    order: u64,
}

fn collect_deferred_stacking_contexts<'a>(
    node: &'a RenderNode,
    path: &[usize],
    order: &mut u64,
    deferred: &mut Vec<DeferredStackingContext<'a>>,
) {
    for (child_index, child) in node.children.iter().enumerate() {
        let mut child_path = path.to_vec();
        child_path.push(child_index);
        if establishes_stacking_context(child) {
            deferred.push(DeferredStackingContext {
                node: child,
                path: child_path,
                level: stacking_context_level(child),
                order: *order,
            });
            *order = order.wrapping_add(1);
        } else {
            collect_deferred_stacking_contexts(child, &child_path, order, deferred);
        }
    }
}

fn push_node_paint_items_before_children(
    node: &RenderNode,
    path: &[usize],
    stable_sort_key: &mut u64,
    items: &mut Vec<ExtractedPaintItem>,
) {
    let clip = node.style.overflow.clips_any_axis().then_some(node.layout);

    if node.style.native_material == NativeMaterial::Glass {
        push_item(
            node,
            path,
            0,
            ExtractedPaintKind::GlassReveal,
            clip,
            None,
            stable_sort_key,
            items,
        );
    }

    if node.style.backdrop_occlusion == BackdropOcclusion::Scene {
        push_item(
            node,
            path,
            4,
            ExtractedPaintKind::BackdropOcclude,
            clip,
            None,
            stable_sort_key,
            items,
        );
    }

    if node.style.backdrop_blur_radius > f32::EPSILON {
        push_item(
            node,
            path,
            8,
            ExtractedPaintKind::BackdropBlur,
            clip,
            None,
            stable_sort_key,
            items,
        );
    }

    for (index, _) in node.style.shadows.iter().enumerate() {
        push_item(
            node,
            path,
            16 + index as u8,
            ExtractedPaintKind::BoxShadow,
            clip,
            None,
            stable_sort_key,
            items,
        );
    }

    if !matches!(node.kind, RenderKind::Text(_)) {
        for (index, _) in node.style.filter_drop_shadows.iter().enumerate() {
            push_item(
                node,
                path,
                32 + index as u8,
                ExtractedPaintKind::FilterDropShadow,
                clip,
                None,
                stable_sort_key,
                items,
            );
        }
    }

    if node.style.background.is_some() || !node.style.background_layers.is_empty() {
        push_item(
            node,
            path,
            64,
            ExtractedPaintKind::Background,
            clip,
            None,
            stable_sort_key,
            items,
        );
    }

    if !node.style.border.widths.is_zero() {
        push_item(
            node,
            path,
            80,
            ExtractedPaintKind::Border,
            clip,
            None,
            stable_sort_key,
            items,
        );
    }

    for (index, _) in node.style.inset_shadows.iter().enumerate() {
        push_item(
            node,
            path,
            88 + index as u8,
            ExtractedPaintKind::BoxShadow,
            clip,
            None,
            stable_sort_key,
            items,
        );
    }

    match &node.kind {
        RenderKind::Container => {}
        RenderKind::Text(text) => {
            push_item(
                node,
                path,
                96,
                ExtractedPaintKind::TextRun,
                clip,
                Some(ExtractedPayload::Text(text.clone())),
                stable_sort_key,
                items,
            );
        }
        RenderKind::Svg(scene) => {
            push_item(
                node,
                path,
                112,
                ExtractedPaintKind::Svg,
                clip,
                Some(ExtractedPayload::Svg(scene.clone())),
                stable_sort_key,
                items,
            );
        }
    }
}

fn push_node_scrollbar_item(
    node: &RenderNode,
    path: &[usize],
    stable_sort_key: &mut u64,
    items: &mut Vec<ExtractedPaintItem>,
) {
    let clip = node.style.overflow.clips_any_axis().then_some(node.layout);
    if node.scrollbars.is_some() {
        push_item(
            node,
            path,
            240,
            ExtractedPaintKind::Scrollbar,
            clip,
            None,
            stable_sort_key,
            items,
        );
    }
}

enum ExtractedPayload {
    Text(String),
    Svg(SvgScene),
}

fn push_item(
    node: &RenderNode,
    path: &[usize],
    _phase: u8,
    kind: ExtractedPaintKind,
    clip: Option<LayoutBox>,
    payload: Option<ExtractedPayload>,
    stable_sort_key: &mut u64,
    items: &mut Vec<ExtractedPaintItem>,
) {
    let (text, svg_scene) = match payload {
        Some(ExtractedPayload::Text(text)) => (Some(text), None),
        Some(ExtractedPayload::Svg(scene)) => (None, Some(scene)),
        None => (None, None),
    };

    items.push(ExtractedPaintItem {
        stable_sort_key: next_stable_sort_key(stable_sort_key),
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
}

fn next_stable_sort_key(stable_sort_key: &mut u64) -> u64 {
    let key = *stable_sort_key;
    *stable_sort_key = stable_sort_key.wrapping_add(1);
    key
}

#[cfg(test)]
mod tests {
    use crate::{
        BackdropOcclusion, BoxShadow, Color, CornerRadius, Insets, LayoutBox, Overflow,
        PreparedTextLayout, RenderNode, ScrollbarData, ScrollbarMetrics, ScrollbarStyle,
        ScrollbarWidth, TextStyle, VisualStyle, ZIndex, fonts::TextLayout,
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
                    backdrop_occlusion: BackdropOcclusion::Scene,
                    border: crate::BorderStyle {
                        widths: Insets::all(1.0),
                        color: Color::rgb(226, 232, 240),
                        ..crate::BorderStyle::default()
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
        assert!(kinds.contains(&ExtractedPaintKind::BackdropOcclude));
        assert!(kinds.contains(&ExtractedPaintKind::Background));
        assert!(kinds.contains(&ExtractedPaintKind::Border));
        assert!(kinds.contains(&ExtractedPaintKind::TextRun));
        assert!(kinds.contains(&ExtractedPaintKind::Scrollbar));

        let occlude_index = kinds
            .iter()
            .position(|kind| *kind == ExtractedPaintKind::BackdropOcclude)
            .expect("occlusion item should be extracted");
        let background_index = kinds
            .iter()
            .position(|kind| *kind == ExtractedPaintKind::Background)
            .expect("background item should be extracted");
        assert!(occlude_index < background_index);
    }

    #[test]
    fn extracted_scene_reports_native_glass_regions_and_tint() {
        let scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 220.0, 160.0)).with_child(
                RenderNode::container(LayoutBox::new(0.0, 0.0, 72.0, 160.0)).with_style(
                    VisualStyle {
                        native_material: crate::NativeMaterial::Glass,
                        glass_tint: Some(Color::rgba(255, 255, 255, 96)),
                        ..VisualStyle::default()
                    },
                ),
            ),
        ];

        let extracted = ExtractedScene::from_render_roots(&scene);

        assert!(extracted.requires_native_glass());
        assert_eq!(
            extracted.preferred_glass_tint(),
            Some(Color::rgba(255, 255, 255, 96))
        );
        let regions = extracted.glass_regions().collect::<Vec<_>>();
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].layout, LayoutBox::new(0.0, 0.0, 72.0, 160.0));
    }

    #[test]
    fn extracted_scene_ignores_glass_tint_without_native_glass() {
        let scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 80.0, 40.0)).with_style(VisualStyle {
                glass_tint: Some(Color::rgba(255, 255, 255, 96)),
                ..VisualStyle::default()
            }),
        ];

        let extracted = ExtractedScene::from_render_roots(&scene);

        assert!(!extracted.requires_native_glass());
        assert_eq!(extracted.glass_regions().count(), 0);
        assert_eq!(extracted.preferred_glass_tint(), None);
    }

    #[test]
    fn extracted_scene_prefers_first_glass_tint_deterministically() {
        let scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 180.0, 80.0))
                .with_child(
                    RenderNode::container(LayoutBox::new(8.0, 8.0, 40.0, 40.0)).with_style(
                        VisualStyle {
                            native_material: crate::NativeMaterial::Glass,
                            glass_tint: Some(Color::rgba(255, 255, 255, 72)),
                            ..VisualStyle::default()
                        },
                    ),
                )
                .with_child(
                    RenderNode::container(LayoutBox::new(64.0, 8.0, 40.0, 40.0)).with_style(
                        VisualStyle {
                            native_material: crate::NativeMaterial::Glass,
                            glass_tint: Some(Color::rgba(24, 36, 54, 128)),
                            ..VisualStyle::default()
                        },
                    ),
                ),
        ];

        let left = ExtractedScene::from_render_roots(&scene);
        let right = ExtractedScene::from_render_roots(&scene);

        assert_eq!(left.glass_regions().count(), 2);
        assert_eq!(
            left.preferred_glass_tint(),
            Some(Color::rgba(255, 255, 255, 72))
        );
        assert_eq!(left.preferred_glass_tint(), right.preferred_glass_tint());
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
    fn extracted_scene_keeps_positioned_z_index_subtrees_together() {
        let scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 40.0, 40.0))
                .with_child(
                    RenderNode::container(LayoutBox::new(4.0, 4.0, 24.0, 24.0))
                        .with_style(VisualStyle {
                            background: Some(Color::rgb(34, 197, 94)),
                            positioned: true,
                            z_index: ZIndex::Integer(1000),
                            ..VisualStyle::default()
                        })
                        .with_child(
                            RenderNode::container(LayoutBox::new(8.0, 8.0, 8.0, 8.0)).with_style(
                                VisualStyle {
                                    background: Some(Color::rgb(37, 99, 235)),
                                    ..VisualStyle::default()
                                },
                            ),
                        ),
                )
                .with_child(
                    RenderNode::container(LayoutBox::new(6.0, 6.0, 24.0, 24.0)).with_style(
                        VisualStyle {
                            background: Some(Color::rgb(239, 68, 68)),
                            ..VisualStyle::default()
                        },
                    ),
                ),
        ];

        let extracted = ExtractedScene::from_render_roots(&scene);
        let background_paths = extracted
            .items
            .iter()
            .filter(|item| item.kind == ExtractedPaintKind::Background)
            .map(|item| item.path.clone())
            .collect::<Vec<_>>();

        assert_eq!(
            background_paths,
            vec![vec![0, 1], vec![0, 0], vec![0, 0, 0]]
        );
    }

    #[test]
    fn extracted_scene_promotes_nested_positioned_z_index_to_nearest_context() {
        let scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 40.0, 40.0))
                .with_child(
                    RenderNode::container(LayoutBox::new(0.0, 0.0, 32.0, 32.0))
                        .with_style(VisualStyle {
                            background: Some(Color::rgb(34, 197, 94)),
                            ..VisualStyle::default()
                        })
                        .with_child(
                            RenderNode::container(LayoutBox::new(8.0, 8.0, 12.0, 12.0)).with_style(
                                VisualStyle {
                                    background: Some(Color::rgb(37, 99, 235)),
                                    positioned: true,
                                    z_index: ZIndex::Integer(1000),
                                    ..VisualStyle::default()
                                },
                            ),
                        ),
                )
                .with_child(
                    RenderNode::container(LayoutBox::new(4.0, 4.0, 24.0, 24.0)).with_style(
                        VisualStyle {
                            background: Some(Color::rgb(239, 68, 68)),
                            ..VisualStyle::default()
                        },
                    ),
                ),
        ];

        let extracted = ExtractedScene::from_render_roots(&scene);
        let background_paths = extracted
            .items
            .iter()
            .filter(|item| item.kind == ExtractedPaintKind::Background)
            .map(|item| item.path.clone())
            .collect::<Vec<_>>();

        assert_eq!(
            background_paths,
            vec![vec![0, 0], vec![0, 1], vec![0, 0, 0]]
        );
    }
}
