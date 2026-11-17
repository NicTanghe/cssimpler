//! Issue #22: prepared native vector geometry and stroke outlines.
//!
//! The investigation concluded from the stage-separated measurements that a
//! prepared-geometry cache does not pay for its memory and complexity (see
//! `docs/prepared-geometry-cache.md`), so the renderer keeps the direct
//! preparation path. These tests pin the properties the decision relies on:
//!
//! - the stage probe is pure instrumentation: identical pixels with it on or off
//! - every paint really does rebuild paths and stroke outlines (no hidden cache,
//!   no retained history, no paint-keyed reuse)
//! - worker bands aggregate their stage work without losing counts
//!
//! If a cache is ever implemented behind a budget, replace the second test with
//! cache-on/cache-off pixel equality plus invalidation/cap/zero-budget tests.

use cssimpler::{
    core::{
        Color, Node, PathCommand, RenderNode, VectorPaint, VectorPath, VectorPoint, VectorScene,
        VectorViewBox,
    },
    renderer::{
        render_to_buffer, reset_vector_preparation_stats, set_vector_preparation_probe_enabled,
        vector_preparation_probe_enabled, vector_preparation_stats,
    },
    style::{Stylesheet, build_render_tree_in_viewport},
};
use std::sync::Mutex;

/// The probe and its counters are process-wide; serialize the tests that use
/// them so assertions describe the work of one test.
static PROBE_LOCK: Mutex<()> = Mutex::new(());

fn stroked_scene(view_box: f32, width: usize, height: usize) -> Vec<RenderNode> {
    let paths = (0..8)
        .map(|index| {
            let x = 6. + index as f32 * (view_box - 16.) / 8.;
            VectorPath::new(
                vec![
                    PathCommand::MoveTo(VectorPoint::new(x, 8.)),
                    PathCommand::CubicTo {
                        control1: VectorPoint::new(x - 4., view_box * 0.3),
                        control2: VectorPoint::new(x + 24., view_box * 0.6),
                        end: VectorPoint::new(x + 4., view_box - 8.),
                    },
                    PathCommand::QuadTo {
                        control: VectorPoint::new(x + 18., view_box * 0.5),
                        end: VectorPoint::new(x, view_box * 0.5),
                    },
                ],
                VectorPaint {
                    fill: Some(Color::rgb(40, 90, 160)),
                    stroke: Some(Color::rgb(10, 20, 40)),
                    stroke_width: 2.0,
                },
            )
            .unwrap()
        })
        .collect();
    let node: Node = Node::vector(
        VectorScene::new(VectorViewBox::new(0., 0., view_box, view_box), paths).unwrap(),
    )
    .into();
    let tree = build_render_tree_in_viewport(&node, &Stylesheet::default(), width, height);
    assert!(!tree.style.cache_layer, "pixel-layer caching must stay off");
    vec![tree]
}

fn paint(scene: &[RenderNode], pixels: &mut [u32], width: usize, height: usize) {
    render_to_buffer(scene, pixels, width, height, Color::WHITE);
}

#[test]
fn probe_preserves_pixels_and_reports_every_stage() {
    let _guard = PROBE_LOCK.lock().unwrap();
    let (width, height) = (160, 160);
    let scene = stroked_scene(160., width, height);
    let mut off = vec![0u32; width * height];
    let mut on = vec![0u32; width * height];

    set_vector_preparation_probe_enabled(false);
    reset_vector_preparation_stats();
    paint(&scene, &mut off, width, height);
    assert!(
        vector_preparation_stats().paths_painted == 0,
        "disabled probe must not count work"
    );

    set_vector_preparation_probe_enabled(true);
    assert!(vector_preparation_probe_enabled());
    reset_vector_preparation_stats();
    // Two frames: the probe is cumulative, so counts double.
    paint(&scene, &mut on, width, height);
    let first = vector_preparation_stats();
    paint(&scene, &mut on, width, height);
    let both = vector_preparation_stats();
    set_vector_preparation_probe_enabled(false);

    assert_eq!(off, on, "instrumentation changed the painted pixels");
    assert_eq!(first.paths_painted, 8, "one build per path per frame");
    assert_eq!(both.paths_painted, 16);
    assert_eq!(first.stroke_outlines, 8, "every path has a stroke");
    assert!(
        first.coverage_tiles > 0,
        "filled/stroked paths must rasterize coverage tiles"
    );
    assert!(both.coverage_tiles >= first.coverage_tiles * 2);
    assert!(both.path_build_ns > 0, "path construction was not timed");
    assert!(both.stroke_prep_ns > 0, "stroke preparation was not timed");
    assert!(
        both.projection_ns > 0,
        "projection into device space was not timed"
    );
    assert!(both.coverage_raster_ns > 0, "coverage raster was not timed");
    assert!(
        both.coverage_blend_ns > 0,
        "coverage blending was not timed"
    );
    assert!(both.geometry_prep_ns() <= both.attributed_ns());
}

#[test]
fn every_paint_rebuilds_paths_and_outlines_without_retained_state() {
    let _guard = PROBE_LOCK.lock().unwrap();
    let (width, height) = (120, 120);
    let scene = stroked_scene(120., width, height);
    let mut pixels = vec![0u32; width * height];

    set_vector_preparation_probe_enabled(true);
    reset_vector_preparation_stats();
    let mut builds = Vec::new();
    for frame in 0..5 {
        paint(&scene, &mut pixels, width, height);
        builds.push(vector_preparation_stats().paths_painted);
        assert_eq!(
            builds[frame],
            8 * (frame + 1),
            "each paint must construct every path again: no prepared-geometry \
             cache or retained history is active"
        );
    }
    // Repeating the identical geometry also re-strokes it: there is no cache
    // keyed by geometry identity, so the zero-budget/direct-preparation path is
    // the only path in the shipped renderer.
    assert_eq!(vector_preparation_stats().stroke_outlines, 8 * 5);
    set_vector_preparation_probe_enabled(false);
    reset_vector_preparation_stats();
}

#[test]
fn paint_only_changes_still_rebuild_geometry_but_keep_new_colors() {
    let _guard = PROBE_LOCK.lock().unwrap();
    let (width, height) = (96, 96);
    let geometry = cssimpler::core::VectorGeometry::new(vec![
        PathCommand::MoveTo(VectorPoint::new(8., 8.)),
        PathCommand::LineTo(VectorPoint::new(88., 8.)),
        PathCommand::LineTo(VectorPoint::new(88., 88.)),
        PathCommand::Close,
    ])
    .unwrap();
    let make = |fill: Color| {
        let path = VectorPath::from_geometry(
            geometry.clone(),
            VectorPaint {
                fill: Some(fill),
                stroke: None,
                stroke_width: 0.0,
            },
        )
        .unwrap();
        let node: Node = Node::vector(
            VectorScene::new(VectorViewBox::new(0., 0., 96., 96.), vec![path]).unwrap(),
        )
        .into();
        vec![build_render_tree_in_viewport(
            &node,
            &Stylesheet::default(),
            width,
            height,
        )]
    };
    let red = make(Color::rgb(255, 0, 0));
    let blue = make(Color::rgb(0, 0, 255));
    let mut red_pixels = vec![0u32; width * height];
    let mut blue_pixels = vec![0u32; width * height];

    set_vector_preparation_probe_enabled(true);
    reset_vector_preparation_stats();
    paint(&red, &mut red_pixels, width, height);
    paint(&blue, &mut blue_pixels, width, height);
    let stats = vector_preparation_stats();
    set_vector_preparation_probe_enabled(false);

    assert_eq!(
        stats.paths_painted, 2,
        "a paint-only change does not reuse the other instance's prepared path"
    );
    assert_ne!(
        red_pixels, blue_pixels,
        "paint-only changes must still reach the pixels"
    );
    // The two instances share one immutable geometry handle, so the rebuilt
    // paths differ only in paint.
    assert!(geometry.bounds().is_some());
}

#[test]
fn worker_bands_aggregate_stage_work_without_losing_counts() {
    let _guard = PROBE_LOCK.lock().unwrap();
    // 640 x 480 is the smallest buffer the renderer paints in parallel.
    let (width, height) = (640, 480);
    let scene = stroked_scene(240., width, height);
    let mut pixels = vec![0u32; width * height];

    set_vector_preparation_probe_enabled(true);
    reset_vector_preparation_stats();
    paint(&scene, &mut pixels, width, height);
    let one = vector_preparation_stats();
    paint(&scene, &mut pixels, width, height);
    let two = vector_preparation_stats();
    set_vector_preparation_probe_enabled(false);
    reset_vector_preparation_stats();

    assert!(
        one.paths_painted >= 8,
        "worker bands build at least one path per visible path"
    );
    // Band splitting is deterministic, so two identical frames double every
    // aggregated counter. This fails if a worker's local totals are dropped.
    assert_eq!(two.paths_painted, one.paths_painted * 2);
    assert_eq!(two.stroke_outlines, one.stroke_outlines * 2);
    assert_eq!(two.coverage_tiles, one.coverage_tiles * 2);
    assert!(two.coverage_blend_ns > 0);
}
