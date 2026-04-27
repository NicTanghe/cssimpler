use crate::{
    Color, ElementPath, EventHandlers, Insets, LayoutBox, NativeMaterial, PreparedTextLayout,
    RenderKind, RenderNode, ScrollbarData, SvgScene, Transform2D, TransitionStyle, VisualStyle,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtractedPaintKind {
    GlassReveal,
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
        for (root_index, root) in roots.iter().enumerate() {
            collect_paint_items(root, vec![root_index], &mut items);
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

fn collect_paint_items(node: &RenderNode, path: Vec<usize>, items: &mut Vec<ExtractedPaintItem>) {
    let clip = node.style.overflow.clips_any_axis().then_some(node.layout);

    if node.style.native_material == NativeMaterial::Glass {
        push_item(
            node,
            &path,
            0,
            ExtractedPaintKind::GlassReveal,
            clip,
            None,
            items,
        );
    }

    if node.style.backdrop_blur_radius > f32::EPSILON {
        push_item(
            node,
            &path,
            8,
            ExtractedPaintKind::BackdropBlur,
            clip,
            None,
            items,
        );
    }

    for (index, _) in node.style.shadows.iter().enumerate() {
        push_item(
            node,
            &path,
            16 + index as u8,
            ExtractedPaintKind::BoxShadow,
            clip,
            None,
            items,
        );
    }

    if !matches!(node.kind, RenderKind::Text(_)) {
        for (index, _) in node.style.filter_drop_shadows.iter().enumerate() {
            push_item(
                node,
                &path,
                32 + index as u8,
                ExtractedPaintKind::FilterDropShadow,
                clip,
                None,
                items,
            );
        }
    }

    if node.style.background.is_some() || !node.style.background_layers.is_empty() {
        push_item(
            node,
            &path,
            64,
            ExtractedPaintKind::Background,
            clip,
            None,
            items,
        );
    }

    if !node.style.border.widths.is_zero() {
        push_item(
            node,
            &path,
            80,
            ExtractedPaintKind::Border,
            clip,
            None,
            items,
        );
    }

    match &node.kind {
        RenderKind::Container => {}
        RenderKind::Text(text) => {
            push_item(
                node,
                &path,
                96,
                ExtractedPaintKind::TextRun,
                clip,
                Some(ExtractedPayload::Text(text.clone())),
                items,
            );
        }
        RenderKind::Svg(scene) => {
            push_item(
                node,
                &path,
                112,
                ExtractedPaintKind::Svg,
                clip,
                Some(ExtractedPayload::Svg(scene.clone())),
                items,
            );
        }
    }

    for (child_index, child) in node.children.iter().enumerate() {
        let mut child_path = path.clone();
        child_path.push(child_index);
        collect_paint_items(child, child_path, items);
    }

    if node.scrollbars.is_some() {
        push_item(
            node,
            &path,
            240,
            ExtractedPaintKind::Scrollbar,
            clip,
            None,
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
    phase: u8,
    kind: ExtractedPaintKind,
    clip: Option<LayoutBox>,
    payload: Option<ExtractedPayload>,
    items: &mut Vec<ExtractedPaintItem>,
) {
    let (text, svg_scene) = match payload {
        Some(ExtractedPayload::Text(text)) => (Some(text), None),
        Some(ExtractedPayload::Svg(scene)) => (None, Some(scene)),
        None => (None, None),
    };

    items.push(ExtractedPaintItem {
        stable_sort_key: stable_sort_key(path, phase),
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

fn stable_sort_key(path: &[usize], phase: u8) -> u64 {
    let mut key = phase as u64;
    for &segment in path {
        key = key
            .wrapping_mul(1_099_511_628_211)
            .wrapping_add(segment as u64 + 1);
    }
    key
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
        assert!(kinds.contains(&ExtractedPaintKind::Background));
        assert!(kinds.contains(&ExtractedPaintKind::Border));
        assert!(kinds.contains(&ExtractedPaintKind::TextRun));
        assert!(kinds.contains(&ExtractedPaintKind::Scrollbar));
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
}
