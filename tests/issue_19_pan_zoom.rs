//! Issue #19: native vector pan/zoom that reuses authored geometry.
//!
//! Covers inverse document/pointer mapping, cursor-anchored zoom, repeated
//! view-only updates (no command copying, no allocations), viewport clipping,
//! CSS transform + device-scale composition, dirty-region parity, cache
//! invalidation, and export/curve stability.

use cssimpler::{
    core::{
        Color, LayoutBox, Node, PathCommand, RenderKind, RenderNode, VectorGeometry, VectorPaint,
        VectorPath, VectorPoint, VectorScene, VectorView, VectorViewBox,
    },
    renderer::{
        LayerCacheLimits, clear_layer_cache, layer_cache_stats, render_scene_update,
        render_to_buffer, render_to_rgba8, render_to_rgba8_at_scale, set_layer_cache_limits,
    },
    style::{build_render_tree_in_viewport, parse_stylesheet},
};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    sync::Mutex,
};

// Counting allocator used only by `zero_allocation_view_updates`; the recording
// flag is thread-local so parallel tests cannot add counts.
struct CountingAllocator;
thread_local! {
    static RECORD_ALLOCATIONS: Cell<bool> = const { Cell::new(false) };
    static ALLOCATION_CALLS: Cell<u64> = const { Cell::new(0) };
    static ALLOCATION_BYTES: Cell<u64> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if RECORD_ALLOCATIONS.with(Cell::get) {
            ALLOCATION_CALLS.with(|calls| calls.set(calls.get() + 1));
            ALLOCATION_BYTES.with(|bytes| bytes.set(bytes.get() + layout.size() as u64));
        }
        // SAFETY: forwarded unchanged to the system allocator.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: forwarded unchanged to the system allocator.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator;

fn start_recording() -> (u64, u64) {
    let before = (
        ALLOCATION_CALLS.with(Cell::get),
        ALLOCATION_BYTES.with(Cell::get),
    );
    RECORD_ALLOCATIONS.with(|record| record.set(true));
    before
}

fn stop_recording(before: (u64, u64)) -> (u64, u64) {
    RECORD_ALLOCATIONS.with(|record| record.set(false));
    (
        ALLOCATION_CALLS.with(Cell::get) - before.0,
        ALLOCATION_BYTES.with(Cell::get) - before.1,
    )
}

fn rect(x: f32, y: f32, width: f32, height: f32, color: Color) -> VectorPath {
    VectorPath::new(
        vec![
            PathCommand::MoveTo(VectorPoint::new(x, y)),
            PathCommand::LineTo(VectorPoint::new(x + width, y)),
            PathCommand::LineTo(VectorPoint::new(x + width, y + height)),
            PathCommand::LineTo(VectorPoint::new(x, y + height)),
            PathCommand::Close,
        ],
        VectorPaint {
            fill: Some(color),
            stroke: None,
            stroke_width: 1.0,
        },
    )
    .unwrap()
}

fn curve(color: Color) -> VectorPath {
    VectorPath::new(
        vec![
            PathCommand::MoveTo(VectorPoint::new(10.0, 180.0)),
            PathCommand::CubicTo {
                control1: VectorPoint::new(20.0, 20.0),
                control2: VectorPoint::new(180.0, 10.0),
                end: VectorPoint::new(190.0, 90.0),
            },
            PathCommand::QuadTo {
                control: VectorPoint::new(120.0, 160.0),
                end: VectorPoint::new(40.0, 140.0),
            },
            PathCommand::Close,
        ],
        VectorPaint {
            fill: Some(color),
            stroke: None,
            stroke_width: 1.0,
        },
    )
    .unwrap()
}

const BLUE: Color = Color::rgb(20, 60, 200);

fn assert_close(actual: VectorPoint, expected: VectorPoint, context: &str) {
    let tolerance = 1e-3 + 1e-5 * expected.x.abs().max(expected.y.abs());
    let close =
        (actual.x - expected.x).abs() <= tolerance && (actual.y - expected.y).abs() <= tolerance;
    assert!(
        close,
        "{context}: expected ({}, {}), got ({}, {})",
        expected.x, expected.y, actual.x, actual.y
    );
}

fn assert_pixel(rgba: &[u8], width: usize, x: usize, y: usize, expected: [u8; 3], context: &str) {
    let got = pixel_rgb(rgba, width, x, y);
    assert!(
        got.iter()
            .zip(expected)
            .all(|(actual, expected)| (*actual as i32 - expected as i32).abs() <= 2),
        "{context}: expected {expected:?}, got {got:?} at ({x}, {y})"
    );
}

fn assert_white(rgba: &[u8], width: usize, x: usize, y: usize, context: &str) {
    assert_pixel(rgba, width, x, y, [255, 255, 255], context);
}

fn pixel_rgb(rgba: &[u8], width: usize, x: usize, y: usize) -> [u8; 3] {
    let index = (y * width + x) * 4;
    [rgba[index], rgba[index + 1], rgba[index + 2]]
}

fn render_node_rgba(node: RenderNode, width: usize, height: usize) -> Vec<u8> {
    let mut rgba = vec![0u8; width * height * 4];
    render_to_rgba8(&[node], &mut rgba, width, height, Color::WHITE);
    rgba
}

#[test]
fn inverse_pointer_mapping_round_trips() {
    // Deliberately non-uniform layout scale and a viewBox with negative origin.
    let view_box = VectorViewBox::new(-10.0, 20.0, 200.0, 100.0);
    let layout = LayoutBox::new(30.0, 40.0, 400.0, 250.0);
    let views = [
        VectorView::identity(),
        VectorView::new(2.5, VectorPoint::new(7.0, -13.0)).unwrap(),
        VectorView::new(0.05, VectorPoint::new(-300.0, 42.0)).unwrap(),
        VectorView::new(32.0, VectorPoint::new(120.5, -80.25)).unwrap(),
    ];
    let documents = [
        VectorPoint::new(0.0, 0.0),
        VectorPoint::new(-10.0, 20.0),
        VectorPoint::new(190.0, 120.0),
        VectorPoint::new(55.5, 77.25),
    ];
    for view in views {
        for document in documents {
            let viewport = view
                .document_to_viewport(view_box, layout, document)
                .expect("valid layout and viewBox must map");
            let back = view
                .viewport_to_document(view_box, layout, viewport)
                .expect("inverse must exist");
            assert_close(back, document, "viewport round-trip");

            let view_point = view.document_to_view(document);
            let back_view = view.view_to_document(view_point);
            assert_close(back_view, document, "view scratch round-trip");
        }
    }
    // The same document point maps to the same viewport point through either route.
    let view = VectorView::new(3.0, VectorPoint::new(-4.5, 8.0)).unwrap();
    let document = VectorPoint::new(12.0, 34.0);
    let direct = view
        .document_to_viewport(view_box, layout, document)
        .unwrap();
    let via_view = view.document_to_view(document);
    let expected = VectorPoint::new(
        layout.x + (via_view.x - view_box.min_x) * layout.width / view_box.width,
        layout.y + (via_view.y - view_box.min_y) * layout.height / view_box.height,
    );
    assert_close(direct, expected, "direct mapping");
}

#[test]
fn cursor_anchored_zoom_keeps_document_point_under_cursor() {
    let view_box = VectorViewBox::new(0.0, 0.0, 960.0, 640.0);
    let layout = LayoutBox::new(0.0, 0.0, 960.0, 640.0);
    let pointers = [
        VectorPoint::new(120.0, 80.0),
        VectorPoint::new(480.0, 320.0),
        VectorPoint::new(930.0, 610.0),
    ];
    for pointer in pointers {
        for factor in [1.25, 0.8, 4.0, 0.125] {
            let view = VectorView::new(2.0, VectorPoint::new(-40.0, 15.0)).unwrap();
            let anchor_before = view
                .viewport_to_document(view_box, layout, pointer)
                .unwrap();
            let zoomed = view
                .zoomed_about_viewport_point(view_box, layout, pointer, factor)
                .unwrap();
            let anchor_after = zoomed
                .viewport_to_document(view_box, layout, pointer)
                .unwrap();
            assert_close(anchor_after, anchor_before, "zoom anchor");
            let expected_zoom =
                (view.zoom() * factor).clamp(VectorView::MIN_ZOOM, VectorView::MAX_ZOOM);
            assert_eq!(zoomed.zoom(), expected_zoom);
            // The anchored document point still maps under the same cursor.
            let remapped = zoomed
                .document_to_viewport(view_box, layout, anchor_before)
                .unwrap();
            assert_close(remapped, pointer, "cursor remains over anchor");
        }
    }
    // Zoom in past the limit: the anchor stays fixed and the zoom clamps.
    let pointer = VectorPoint::new(400.0, 300.0);
    let mut view = VectorView::identity();
    let anchor = view
        .viewport_to_document(view_box, layout, pointer)
        .unwrap();
    for _ in 0..40 {
        view = view
            .zoomed_about_viewport_point(view_box, layout, pointer, 2.0)
            .unwrap();
        let current = view
            .viewport_to_document(view_box, layout, pointer)
            .unwrap();
        assert_close(current, anchor, "anchor at maximum zoom");
    }
    assert_eq!(view.zoom(), VectorView::MAX_ZOOM);
    for _ in 0..40 {
        view = view
            .zoomed_about_viewport_point(view_box, layout, pointer, 0.5)
            .unwrap();
        let current = view
            .viewport_to_document(view_box, layout, pointer)
            .unwrap();
        assert_close(current, anchor, "anchor at minimum zoom");
    }
    assert_eq!(view.zoom(), VectorView::MIN_ZOOM);
}

#[test]
fn invalid_and_degenerate_view_inputs_are_rejected() {
    for zoom in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.0, -1.0] {
        assert!(
            VectorView::new(zoom, VectorPoint::new(0.0, 0.0)).is_err(),
            "zoom {zoom} must be rejected"
        );
        assert!(VectorView::from_zoom(zoom).is_err());
    }
    for offset in [
        VectorPoint::new(f32::NAN, 0.0),
        VectorPoint::new(0.0, f32::INFINITY),
    ] {
        assert!(VectorView::new(1.0, offset).is_err());
    }
    // Finite zooms outside the practical limits are clamped, never singular.
    assert_eq!(
        VectorView::from_zoom(1.0e9).unwrap().zoom(),
        VectorView::MAX_ZOOM
    );
    assert_eq!(
        VectorView::from_zoom(1.0e-9).unwrap().zoom(),
        VectorView::MIN_ZOOM
    );
    let view = VectorView::identity();
    for factor in [0.0, -2.0, f32::NAN, f32::INFINITY] {
        assert!(
            view.zoomed_about(VectorPoint::new(1.0, 2.0), factor)
                .is_err()
        );
    }
    assert!(
        view.panned_by_viewport_delta(
            VectorViewBox::new(0.0, 0.0, 10.0, 10.0),
            LayoutBox::new(0.0, 0.0, 10.0, 10.0),
            f32::NAN,
            0.0,
        )
        .is_err()
    );
    // Degenerate viewports cannot map, but valid views never become singular.
    for layout in [
        LayoutBox::new(0.0, 0.0, 0.0, 10.0),
        LayoutBox::new(0.0, 0.0, 10.0, -1.0),
        LayoutBox::new(f32::NAN, 0.0, 10.0, 10.0),
    ] {
        assert!(
            view.document_to_viewport(
                VectorViewBox::new(0.0, 0.0, 10.0, 10.0),
                layout,
                VectorPoint::new(1.0, 1.0),
            )
            .is_none()
        );
        assert!(
            view.viewport_to_document(
                VectorViewBox::new(0.0, 0.0, 10.0, 10.0),
                layout,
                VectorPoint::new(1.0, 1.0),
            )
            .is_none()
        );
    }
    assert!(
        view.document_to_viewport(
            VectorViewBox::new(0.0, 0.0, 0.0, 10.0),
            LayoutBox::new(0.0, 0.0, 10.0, 10.0),
            VectorPoint::new(1.0, 1.0),
        )
        .is_none()
    );
    assert!(
        view.zoomed_about_viewport_point(
            VectorViewBox::new(0.0, 0.0, 10.0, 10.0),
            LayoutBox::new(0.0, 0.0, 0.0, 10.0),
            VectorPoint::new(1.0, 1.0),
            2.0,
        )
        .is_err()
    );
    // A finite but extreme document coordinate that overflows the f32 mapping
    // is rejected rather than producing an infinite pointer position.
    assert!(
        VectorView::from_zoom(VectorView::MAX_ZOOM)
            .unwrap()
            .document_to_viewport(
                VectorViewBox::new(0.0, 0.0, 10.0, 10.0),
                LayoutBox::new(0.0, 0.0, 10.0, 10.0),
                VectorPoint::new(f32::MAX, 1.0),
            )
            .is_none()
    );
    assert!(VectorView::default().is_identity());
}

#[test]
fn zero_allocation_view_updates_do_not_copy_geometry() {
    let geometry_commands: Vec<PathCommand> = (0..2048)
        .map(|index| {
            if index == 0 {
                PathCommand::MoveTo(VectorPoint::new(0.0, 0.0))
            } else {
                PathCommand::LineTo(VectorPoint::new(index as f32, (index % 97) as f32))
            }
        })
        .collect();
    let geometry = VectorGeometry::new(geometry_commands).unwrap();
    let path = VectorPath::from_geometry(geometry.clone(), VectorPaint::default()).unwrap();
    let mut scene =
        VectorScene::new(VectorViewBox::new(0.0, 0.0, 2048.0, 97.0), vec![path]).unwrap();
    let identity = geometry.identity();
    let commands_ptr = scene.paths()[0].commands().as_ptr();
    let commands_len = scene.paths()[0].commands().len();
    let scene_size = std::mem::size_of::<VectorScene>();
    let view_size = std::mem::size_of::<VectorView>();
    let option_view_size = std::mem::size_of::<Option<VectorView>>();
    assert!(view_size <= 16, "view state must stay tiny: {view_size}");
    assert!(
        option_view_size <= 16,
        "optional view state must stay tiny: {option_view_size}"
    );
    assert!(
        scene_size <= 128,
        "scene state must stay small: {scene_size}"
    );

    let before = start_recording();
    let view_box = scene.view_box();
    let layout = LayoutBox::new(0.0, 0.0, 800.0, 600.0);
    let mut view = VectorView::identity();
    for step in 0..1000u32 {
        view = view
            .panned_by_viewport_delta(
                view_box,
                layout,
                (step % 7) as f32 - 3.0,
                (step % 5) as f32 - 2.0,
            )
            .unwrap();
        let factor = if step % 3 == 0 { 1.01 } else { 0.995 };
        view = view
            .zoomed_about_viewport_point(view_box, layout, VectorPoint::new(400.0, 300.0), factor)
            .unwrap();
        scene.set_view(Some(view));
        assert!(scene.view().is_some());
        assert_eq!(scene.paths().len(), 1);
        assert_eq!(scene.paths()[0].geometry().identity(), identity);
        assert!(scene.paths()[0].geometry().shares_storage_with(&geometry));
    }
    let (allocations, bytes) = stop_recording(before);
    assert_eq!(
        allocations, 0,
        "view-only updates must not allocate (bytes: {bytes})"
    );
    assert_eq!(bytes, 0);
    assert_eq!(scene.paths()[0].commands().as_ptr(), commands_ptr);
    assert_eq!(scene.paths()[0].commands().len(), commands_len);

    // A per-frame scene clone only bumps the geometry handle; a distinct view
    // never forces a command copy.
    let mut rebuilt = scene.clone();
    rebuilt.set_view(Some(
        VectorView::new(8.0, VectorPoint::new(10.0, 20.0)).unwrap(),
    ));
    assert!(rebuilt.paths()[0].geometry().shares_storage_with(&geometry));
    assert_eq!(rebuilt.paths()[0].geometry().identity(), identity);
    assert_ne!(rebuilt.view(), scene.view());
}

#[test]
fn view_only_changes_keep_curve_controls_and_svg_export() {
    let geometry = VectorGeometry::new(vec![
        PathCommand::MoveTo(VectorPoint::new(1.5, 2.25)),
        PathCommand::CubicTo {
            control1: VectorPoint::new(3.0, 4.0),
            control2: VectorPoint::new(5.5, 6.75),
            end: VectorPoint::new(8.0, 9.0),
        },
        PathCommand::QuadTo {
            control: VectorPoint::new(10.0, 11.0),
            end: VectorPoint::new(12.5, 13.5),
        },
        PathCommand::Close,
    ])
    .unwrap();
    let path = VectorPath::from_geometry(geometry.clone(), VectorPaint::default()).unwrap();
    let mut scene = VectorScene::new(VectorViewBox::new(0.0, 0.0, 20.0, 20.0), vec![path]).unwrap();
    let exported = scene.to_svg();
    let path_data = scene.paths()[0].to_svg_path_data();
    let commands_ptr = scene.paths()[0].commands().as_ptr();
    let identity = geometry.identity();
    assert!(exported.contains("C 3 4 5.5 6.75 8 9"));
    assert!(exported.contains("Q 10 11 12.5 13.5"));
    assert!(!exported.contains("transform"));

    for step in 0..32 {
        let view = VectorView::new(1.0 + step as f32, VectorPoint::new(step as f32, -0.5)).unwrap();
        scene.set_view(Some(view));
        let mut rebuilt = scene.clone();
        rebuilt.set_view(Some(
            VectorView::new(1.0 / (step as f32 + 1.0), VectorPoint::new(2.0, 3.0)).unwrap(),
        ));
        assert_eq!(scene.to_svg(), exported, "export must stay authored");
        assert_eq!(rebuilt.to_svg(), exported);
        assert_eq!(scene.paths()[0].to_svg_path_data(), path_data);
        assert_eq!(scene.paths()[0].commands().as_ptr(), commands_ptr);
        assert_eq!(scene.paths()[0].geometry().identity(), identity);
        assert!(rebuilt.paths()[0].geometry().shares_storage_with(&geometry));
    }

    // A geometry edit still produces new authored data, a new identity, and a
    // different export.
    let replacement_geometry = VectorGeometry::new(vec![
        PathCommand::MoveTo(VectorPoint::new(0.0, 0.0)),
        PathCommand::LineTo(VectorPoint::new(4.0, 4.0)),
    ])
    .unwrap();
    let replacement =
        VectorPath::from_geometry(replacement_geometry.clone(), VectorPaint::default()).unwrap();
    let rewritten =
        VectorScene::new(VectorViewBox::new(0.0, 0.0, 20.0, 20.0), vec![replacement]).unwrap();
    assert_ne!(replacement_geometry.identity(), identity);
    assert_ne!(
        rewritten.paths()[0].to_svg_path_data(),
        scene.paths()[0].to_svg_path_data()
    );
}

#[test]
fn viewport_view_clips_to_the_layout_box() {
    let view_box = VectorViewBox::new(0.0, 0.0, 100.0, 100.0);
    let layout_width = 100usize;
    let layout_height = 100usize;
    let layout = LayoutBox::new(20.0, 20.0, 40.0, 40.0);
    // This path intentionally extends far beyond the viewBox and layout box.
    let path = rect(-50.0, -50.0, 200.0, 200.0, BLUE);

    let legacy = VectorScene::new(view_box, vec![path.clone()]).unwrap();
    let legacy_pixels = render_node_rgba(
        RenderNode::vector(layout, legacy),
        layout_width,
        layout_height,
    );
    assert_pixel(
        &legacy_pixels,
        layout_width,
        70,
        30,
        [BLUE.r, BLUE.g, BLUE.b],
        "without a view, legacy vector scenes may paint outside their layout box",
    );

    let clipped = VectorScene::new(view_box, vec![path])
        .unwrap()
        .with_view(VectorView::identity());
    let clipped_pixels = render_node_rgba(
        RenderNode::vector(layout, clipped),
        layout_width,
        layout_height,
    );
    assert_pixel(
        &clipped_pixels,
        layout_width,
        40,
        40,
        [BLUE.r, BLUE.g, BLUE.b],
        "the viewport interior still paints",
    );
    assert_white(
        &clipped_pixels,
        layout_width,
        70,
        30,
        "a viewport view clips content to the element's layout box",
    );
    assert_white(
        &clipped_pixels,
        layout_width,
        10,
        10,
        "outside the viewport",
    );
}

#[test]
fn scene_view_composes_with_css_transform_and_layout() {
    let stylesheet =
        parse_stylesheet("#draw { width: 40px; height: 40px; transform: translate(20px, 20px); }")
            .unwrap();
    let view_box = VectorViewBox::new(0.0, 0.0, 100.0, 100.0);
    let marker = rect(10.0, 10.0, 10.0, 10.0, BLUE);
    let build = |view: Option<VectorView>| {
        let mut scene = VectorScene::new(view_box, vec![marker.clone()]).unwrap();
        scene.set_view(view);
        let node: Node = Node::vector(scene).with_id("draw").into();
        build_render_tree_in_viewport(&node, &stylesheet, 100, 100)
    };

    let identity_tree = build(Some(VectorView::identity()));
    assert_eq!(identity_tree.layout, LayoutBox::new(0.0, 0.0, 40.0, 40.0));
    assert!(
        !identity_tree.style.transform.is_identity(),
        "CSS transform must still be applied"
    );
    let RenderKind::Svg(identity_scene) = &identity_tree.kind else {
        panic!("expected native vector leaf");
    };
    assert_eq!(identity_scene.view, Some(VectorView::identity()));

    // doc 10..20 -> layout 4..8 -> CSS translate 20 -> screen 24..28.
    let expected_layout = identity_tree.layout;
    let expected_transform = identity_tree.style.transform.clone();
    let identity_pixels = render_node_rgba(identity_tree, 100, 100);
    assert_pixel(
        &identity_pixels,
        100,
        26,
        26,
        [BLUE.r, BLUE.g, BLUE.b],
        "CSS translation positions the visible square",
    );
    assert_white(
        &identity_pixels,
        100,
        30,
        30,
        "outside the translated square",
    );

    // Zooming by 2 maps doc 10..20 -> layout 8..16 -> screen 28..36.
    let zoomed_tree = build(Some(
        VectorView::new(2.0, VectorPoint::new(0.0, 0.0)).unwrap(),
    ));
    assert_eq!(zoomed_tree.layout, expected_layout);
    assert_eq!(zoomed_tree.style.transform, expected_transform);
    let zoomed_pixels = render_node_rgba(zoomed_tree, 100, 100);
    assert_white(&zoomed_pixels, 100, 26, 26, "left of the zoomed square");
    assert_pixel(
        &zoomed_pixels,
        100,
        30,
        30,
        [BLUE.r, BLUE.g, BLUE.b],
        "zoom and CSS transform compose",
    );
}

#[test]
fn view_mapping_composes_with_device_scale() {
    let view_box = VectorViewBox::new(0.0, 0.0, 100.0, 100.0);
    let marker = rect(10.0, 10.0, 10.0, 10.0, BLUE);
    let scene = VectorScene::new(view_box, vec![marker])
        .unwrap()
        .with_view(VectorView::new(2.0, VectorPoint::new(0.0, 0.0)).unwrap());
    let node = RenderNode::vector(LayoutBox::new(0.0, 0.0, 100.0, 100.0), scene);
    let mut rgba = vec![0u8; 200 * 200 * 4];
    render_to_rgba8_at_scale(&[node], &mut rgba, 200, 200, Color::WHITE, 2.0);
    // Zoom 2 maps doc 10..20 to logical 20..40, i.e. physical 40..80.
    assert_pixel(
        &rgba,
        200,
        60,
        60,
        [BLUE.r, BLUE.g, BLUE.b],
        "inside physical zoom",
    );
    assert_white(&rgba, 200, 34, 34, "left of physical zoom");
    assert_white(&rgba, 200, 84, 84, "right of physical zoom");
}

#[test]
fn view_only_updates_match_full_paint_in_dirty_regions() {
    let view_box = VectorViewBox::new(0.0, 0.0, 100.0, 100.0);
    let layout = LayoutBox::new(0.0, 0.0, 100.0, 100.0);
    let path = rect(10.0, 10.0, 20.0, 20.0, BLUE);
    let previous = RenderNode::vector(
        layout,
        VectorScene::new(view_box, vec![path.clone()])
            .unwrap()
            .with_view(VectorView::identity()),
    );
    let next_scene = VectorScene::new(view_box, vec![path]).unwrap().with_view(
        VectorView::identity()
            .panned_by_viewport_delta(view_box, layout, 18.0, 7.0)
            .unwrap(),
    );
    let next = RenderNode::vector(layout, next_scene);

    let mut incremental = vec![0u32; 100 * 100];
    let mut full = vec![0u32; 100 * 100];
    render_to_buffer(
        &[previous.clone()],
        &mut incremental,
        100,
        100,
        Color::WHITE,
    );
    render_scene_update(
        &[previous],
        &[next.clone()],
        &mut incremental,
        100,
        100,
        Color::WHITE,
    );
    render_to_buffer(&[next], &mut full, 100, 100, Color::WHITE);
    assert_eq!(
        incremental, full,
        "a view-only update must repaint the same pixels as a full paint"
    );
}

#[test]
fn raster_cache_invalidates_on_view_changes_but_not_on_repaint() {
    static CACHE_LOCK: Mutex<()> = Mutex::new(());
    let _guard = CACHE_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    set_layer_cache_limits(LayerCacheLimits {
        total_bytes: 64 * 1024 * 1024,
        layer_bytes: 16 * 1024 * 1024,
    });
    clear_layer_cache();

    let view_box = VectorViewBox::new(0.0, 0.0, 100.0, 100.0);
    let layout = LayoutBox::new(0.0, 0.0, 80.0, 60.0);
    let path = curve(BLUE);
    let node = |view: VectorView| {
        RenderNode::vector(
            layout,
            VectorScene::new(view_box, vec![path.clone()])
                .unwrap()
                .with_view(view),
        )
        .with_cached_layer(true)
    };
    let width = 120;
    let height = 100;
    let mut pixels = vec![0u32; width * height];
    let base_view = VectorView::identity();
    let first = node(base_view);
    let builds_before = layer_cache_stats().builds;
    render_to_buffer(&[first.clone()], &mut pixels, width, height, Color::WHITE);
    let after_first = layer_cache_stats().builds;
    assert!(
        after_first > builds_before,
        "the first cached paint should build a raster"
    );
    render_to_buffer(&[first], &mut pixels, width, height, Color::WHITE);
    assert_eq!(
        layer_cache_stats().builds,
        after_first,
        "an unchanged scene must reuse its raster"
    );

    // View-only change: the cache key includes the view, so the raster rebuilds
    // at the new magnification instead of stretching the previous bitmap.
    let zoomed = node(VectorView::new(4.0, VectorPoint::new(0.0, 0.0)).unwrap());
    render_to_buffer(&[zoomed], &mut pixels, width, height, Color::WHITE);
    let after_zoom = layer_cache_stats().builds;
    assert!(
        after_zoom > after_first,
        "a view change must invalidate the raster cache entry"
    );
    // Paint-only re-render of the new view reuses the rebuilt raster.
    let zoomed_again = node(VectorView::new(4.0, VectorPoint::new(0.0, 0.0)).unwrap());
    render_to_buffer(&[zoomed_again], &mut pixels, width, height, Color::WHITE);
    assert_eq!(layer_cache_stats().builds, after_zoom);

    // A geometry edit is still a new authored revision and must invalidate the
    // derived raster even when the view is unchanged.
    let edited = RenderNode::vector(
        layout,
        VectorScene::new(view_box, vec![rect(6.0, 6.0, 30.0, 30.0, BLUE)])
            .unwrap()
            .with_view(VectorView::new(4.0, VectorPoint::new(0.0, 0.0)).unwrap()),
    )
    .with_cached_layer(true);
    render_to_buffer(&[edited], &mut pixels, width, height, Color::WHITE);
    assert!(
        layer_cache_stats().builds > after_zoom,
        "a geometry edit must invalidate the cached raster"
    );

    clear_layer_cache();
    set_layer_cache_limits(LayerCacheLimits::default());
}
