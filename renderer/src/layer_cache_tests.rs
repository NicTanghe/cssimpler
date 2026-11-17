use super::*;
use cssimpler_core::{LengthPercentageValue, Transform2D, TransformOperation};

fn layer(color: Color, enabled: bool) -> RenderNode {
    RenderNode::container(LayoutBox::new(8.0, 8.0, 48.0, 32.0))
        .with_style(VisualStyle {
            background: Some(color),
            ..VisualStyle::default()
        })
        .with_cached_layer(enabled)
}

#[test]
fn explicit_stationary_layer_reuses_pixels_while_a_sibling_overlay_changes() {
    let _guard = lock_subtree_surface_cache_for_tests();
    clear_subtree_surface_cache_for_tests();
    let mut buffer = vec![0; 80 * 64];
    render_to_buffer(
        &[layer(Color::BLACK, false)],
        &mut buffer,
        80,
        64,
        Color::WHITE,
    );
    assert_eq!(layer_cache_stats().builds, 0);
    let board = layer(Color::BLACK, true);
    for x in [12.0, 20.0, 30.0] {
        let overlay =
            RenderNode::container(LayoutBox::new(x, 16.0, 4.0, 4.0)).with_style(VisualStyle {
                background: Some(Color::rgb(240, 10, 20)),
                ..VisualStyle::default()
            });
        render_to_buffer(&[board.clone(), overlay], &mut buffer, 80, 64, Color::WHITE);
        assert_eq!(layer_cache_stats().builds, 1);
        assert_eq!(
            buffer[17 * 80 + x as usize],
            pack_rgb(Color::rgb(240, 10, 20))
        );
    }
    render_to_buffer(
        &[layer(Color::rgb(10, 200, 30), true)],
        &mut buffer,
        80,
        64,
        Color::WHITE,
    );
    assert_eq!(layer_cache_stats().builds, 2);
    assert_eq!(buffer[20 * 80 + 20], pack_rgb(Color::rgb(10, 200, 30)));
}

#[test]
fn explicit_layer_reuses_translation_but_rerasterizes_when_magnified() {
    let _guard = lock_subtree_surface_cache_for_tests();
    clear_subtree_surface_cache_for_tests();
    let mut board = layer(Color::BLACK, true);
    let mut buffer = vec![0; 80 * 64];
    render_to_buffer(&[board.clone()], &mut buffer, 80, 64, Color::WHITE);
    board.style.transform = Transform2D {
        operations: vec![TransformOperation::Translate {
            x: LengthPercentageValue::from_px(10.0),
            y: LengthPercentageValue::from_px(4.0),
        }],
        ..Transform2D::default()
    };
    render_to_buffer(&[board.clone()], &mut buffer, 80, 64, Color::WHITE);
    assert_eq!(layer_cache_stats().builds, 1);
    assert_eq!(buffer[16 * 80 + 20], pack_rgb(Color::BLACK));
    board.style.transform = Transform2D {
        operations: vec![TransformOperation::Scale { x: 1.2, y: 1.2 }],
        ..Transform2D::default()
    };
    render_to_buffer(&[board], &mut buffer, 80, 64, Color::WHITE);
    assert_eq!(layer_cache_stats().builds, 1);
}

#[test]
fn layer_limits_evict_and_zero_budget_disables_builds() {
    let _guard = lock_subtree_surface_cache_for_tests();
    clear_subtree_surface_cache_for_tests();
    let layer_bytes = 48 * 32 * size_of::<LinearRgba>();
    set_layer_cache_limits(LayerCacheLimits {
        total_bytes: layer_bytes,
        layer_bytes,
    });
    let mut buffer = vec![0; 80 * 64];
    for color in [Color::BLACK, Color::WHITE] {
        render_to_buffer(&[layer(color, true)], &mut buffer, 80, 64, Color::WHITE);
    }
    assert_eq!(layer_cache_stats().builds, 2);
    assert_eq!(layer_cache_stats().entries, 1);
    assert_eq!(layer_cache_stats().bytes, layer_bytes);
    set_layer_cache_limits(LayerCacheLimits {
        total_bytes: 0,
        layer_bytes: 0,
    });
    render_to_buffer(
        &[layer(Color::BLACK, true)],
        &mut buffer,
        80,
        64,
        Color::WHITE,
    );
    assert_eq!(layer_cache_stats().entries, 0);
    assert_eq!(layer_cache_stats().bytes, 0);
    assert_eq!(layer_cache_stats().builds, 2);
    clear_subtree_surface_cache_for_tests();
}

#[test]
fn explicit_layer_can_exceed_default_size_only_with_an_application_budget() {
    let _guard = lock_subtree_surface_cache_for_tests();
    clear_subtree_surface_cache_for_tests();
    let board = RenderNode::container(LayoutBox::new(0.0, 0.0, 600.0, 500.0))
        .with_style(VisualStyle {
            background: Some(Color::BLACK),
            ..VisualStyle::default()
        })
        .with_cached_layer(true);
    let mut buffer = vec![0; 600 * 500];
    render_to_buffer(&[board.clone()], &mut buffer, 600, 500, Color::WHITE);
    assert_eq!(layer_cache_stats().builds, 0);
    set_layer_cache_limits(LayerCacheLimits {
        total_bytes: 8 * 1024 * 1024,
        layer_bytes: 8 * 1024 * 1024,
    });
    render_to_buffer(&[board], &mut buffer, 600, 500, Color::WHITE);
    assert_eq!(
        layer_cache_stats().builds,
        1,
        "parallel workers should build a cold layer once"
    );
    assert_eq!(
        layer_cache_stats().bytes,
        600 * 500 * size_of::<LinearRgba>()
    );
    clear_layer_cache();
    assert_eq!(layer_cache_stats().bytes, 0);
    clear_subtree_surface_cache_for_tests();
}

#[test]
fn cached_layer_build_does_not_write_into_the_outer_alpha_target() {
    let _guard = lock_subtree_surface_cache_for_tests();
    clear_subtree_surface_cache_for_tests();
    let scene = vec![layer(Color::rgba(240, 10, 20, 128), true)];
    let cached_bounds = cache_scene_subtree_bounds(&scene);
    let mut buffer = vec![0; 80 * 64];
    let mut alpha = vec![0; buffer.len()];
    render_to_buffer_serial(
        &scene,
        &cached_bounds.roots,
        &mut buffer,
        80,
        64,
        Color::rgba(0, 0, 0, 0),
        GlassRenderMode::Fallback,
        Some(&mut alpha),
    );
    assert_eq!(layer_cache_stats().builds, 1);
    assert!(alpha[20 * 80 + 20].abs_diff(128) <= 1);
    assert_eq!(alpha[0], 0);
    assert_eq!(alpha[7 * 80 + 20], 0);
    assert_eq!(unpack_rgb(buffer[20 * 80 + 20]), Color::rgb(240, 10, 20));
}

#[test]
fn svg_leaf_layers_reuse_the_same_cache_and_invalidate_after_geometry_edits() {
    use cssimpler_core::{
        SvgContour, SvgPathGeometry, SvgPathInstance, SvgPathPaint, SvgPoint, SvgScene, SvgViewBox,
    };
    let _guard = lock_subtree_surface_cache_for_tests();
    clear_subtree_surface_cache_for_tests();
    let mut board = RenderNode::svg(
        LayoutBox::new(0.0, 0.0, 64.0, 64.0),
        SvgScene::new(
            SvgViewBox::new(0.0, 0.0, 64.0, 64.0),
            vec![SvgPathInstance {
                geometry: SvgPathGeometry::new(vec![SvgContour {
                    points: vec![
                        SvgPoint::new(8.0, 8.0),
                        SvgPoint::new(56.0, 8.0),
                        SvgPoint::new(56.0, 56.0),
                        SvgPoint::new(8.0, 56.0),
                    ],
                    closed: true,
                }]),
                paint: SvgPathPaint::default(),
            }],
        ),
    )
    .with_cached_layer(true);
    let mut buffer = vec![0; 64 * 64];
    for _ in 0..2 {
        render_to_buffer(&[board.clone()], &mut buffer, 64, 64, Color::WHITE);
        assert_eq!(layer_cache_stats().builds, 1);
        assert_eq!(buffer[20 * 64 + 20], pack_rgb(Color::BLACK));
    }
    if let RenderKind::Svg(svg) = &mut board.kind {
        let points = &mut svg.paths[0].geometry.contours[0].points;
        points[1].x = 30.0;
        points[2].x = 30.0;
    }
    render_to_buffer(&[board], &mut buffer, 64, 64, Color::WHITE);
    assert_eq!(layer_cache_stats().builds, 2);
    assert_eq!(buffer[20 * 64 + 40], pack_rgb(Color::WHITE));
}

#[test]
fn cached_board_survives_incremental_overlay_damage() {
    let _guard = lock_subtree_surface_cache_for_tests();
    clear_subtree_surface_cache_for_tests();
    let board = layer(Color::BLACK, true);
    let overlay = |x| {
        RenderNode::container(LayoutBox::new(x, 16.0, 4.0, 4.0)).with_style(VisualStyle {
            background: Some(Color::rgb(240, 10, 20)),
            ..VisualStyle::default()
        })
    };
    let previous = vec![board.clone(), overlay(12.0)];
    let next = vec![board, overlay(30.0)];
    let mut incremental = vec![0; 80 * 64];
    let mut full = incremental.clone();
    render_to_buffer(&previous, &mut incremental, 80, 64, Color::WHITE);
    let stats = render_scene_update_internal_from_roots(
        &previous,
        &next,
        &mut incremental,
        80,
        64,
        Color::WHITE,
    );
    render_to_buffer(&next, &mut full, 80, 64, Color::WHITE);
    assert!(
        incremental == full,
        "incremental overlay damage should match a full repaint"
    );
    assert_eq!(stats.mode, FramePaintMode::Incremental);
    assert_eq!(layer_cache_stats().builds, 1);
}

#[test]
fn native_shared_geometry_cache_invalidates_paint_and_replacement() {
    use cssimpler_core::{
        PathCommand, SvgPathPaintSource, VectorPaint, VectorPath, VectorPoint, VectorScene,
        VectorViewBox,
    };
    let _guard = lock_subtree_surface_cache_for_tests();
    clear_subtree_surface_cache_for_tests();
    let path = |right| {
        VectorPath::new(
            vec![
                PathCommand::MoveTo(VectorPoint::new(8., 8.)),
                PathCommand::LineTo(VectorPoint::new(right, 8.)),
                PathCommand::LineTo(VectorPoint::new(right, 56.)),
                PathCommand::LineTo(VectorPoint::new(8., 56.)),
                PathCommand::Close,
            ],
            VectorPaint::default(),
        )
        .unwrap()
    };
    let scene = VectorScene::new(VectorViewBox::new(0., 0., 64., 64.), vec![path(56.)]).unwrap();
    let mut node =
        RenderNode::vector(LayoutBox::new(0., 0., 64., 64.), scene).with_cached_layer(true);
    let mut pixels = vec![0; 64 * 64];
    render_to_buffer(&[node.clone()], &mut pixels, 64, 64, Color::WHITE);
    render_to_buffer(&[node.clone()], &mut pixels, 64, 64, Color::WHITE);
    assert_eq!(layer_cache_stats().builds, 1);
    if let RenderKind::Svg(scene) = &mut node.kind {
        scene.paths[0].paint.fill = Some(SvgPathPaintSource::Color(Color::rgb(255, 0, 0)));
    }
    render_to_buffer(&[node.clone()], &mut pixels, 64, 64, Color::WHITE);
    assert_eq!(layer_cache_stats().builds, 2);
    assert_eq!(pixels[20 * 64 + 40], pack_rgb(Color::rgb(255, 0, 0)));
    if let RenderKind::Svg(scene) = &mut node.kind {
        scene.paths[0].geometry.commands = path(30.).geometry().clone();
        scene.paths[0].geometry.bounds = scene.paths[0].geometry.commands.bounds();
    }
    render_to_buffer(&[node], &mut pixels, 64, 64, Color::WHITE);
    assert_eq!(layer_cache_stats().builds, 3);
    assert_eq!(pixels[20 * 64 + 40], pack_rgb(Color::WHITE));
}
