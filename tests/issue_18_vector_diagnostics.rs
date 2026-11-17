//! Public-API checks for issue #18: vector frame diagnostics must be fixed-size,
//! off by default, and must not disturb rendering output.

use std::mem::size_of;

use cssimpler::core::{
    Color, LayoutBox, RenderNode, SvgContour, SvgPathGeometry, SvgPathInstance, SvgPathPaint,
    SvgPathPaintSource, SvgPoint, SvgScene, SvgViewBox,
};
use cssimpler::renderer::{
    FrameTimingStats, VectorFrameStats, latest_frame_timing_stats, render_to_buffer,
    set_vector_timing_enabled,
};

fn sample_scene() -> Vec<RenderNode> {
    let mut paths = Vec::new();
    for index in 0..4 {
        let x = 8.0 + index as f32 * 12.0;
        let y = 8.0 + index as f32 * 9.0;
        paths.push(SvgPathInstance {
            geometry: SvgPathGeometry::new(vec![SvgContour {
                points: vec![
                    SvgPoint::new(x + 0.3, y + 0.7),
                    SvgPoint::new(x + 18.0, y + 1.1),
                    SvgPoint::new(x + 14.0, y + 16.0),
                    SvgPoint::new(x + 1.0, y + 12.0),
                ],
                closed: true,
            }]),
            paint: SvgPathPaint {
                fill: Some(SvgPathPaintSource::Color(Color::rgb(37, 99, 235))),
                stroke: Some(SvgPathPaintSource::Color(Color::rgb(15, 23, 42))),
                stroke_width: 1.5,
            },
        });
    }
    vec![RenderNode::svg(
        LayoutBox::new(0.0, 0.0, 64.0, 64.0),
        SvgScene::new(SvgViewBox::new(0.0, 0.0, 64.0, 64.0), paths),
    )]
}

#[test]
fn enabling_detailed_timers_preserves_pixel_output() {
    let scene = sample_scene();
    let (width, height) = (64, 64);

    set_vector_timing_enabled(false);
    let mut without_timers = vec![0_u32; width * height];
    render_to_buffer(&scene, &mut without_timers, width, height, Color::WHITE);

    set_vector_timing_enabled(true);
    let mut with_timers = vec![0_u32; width * height];
    render_to_buffer(&scene, &mut with_timers, width, height, Color::WHITE);
    set_vector_timing_enabled(false);

    assert_eq!(without_timers, with_timers);
}

#[test]
fn frame_diagnostics_snapshot_is_fixed_size_and_off_by_default() {
    fn assert_copy<T: Copy>() {}

    assert_copy::<FrameTimingStats>();
    assert_copy::<VectorFrameStats>();
    // A fixed-size value snapshot: no history, names, or per-path storage.
    assert!(size_of::<FrameTimingStats>() <= 512);

    assert!(!VectorFrameStats::default().timings_enabled);
    let stats = latest_frame_timing_stats();
    assert_eq!(stats.vector, VectorFrameStats::default());
    assert_eq!(stats.layer_cache_hits, 0);
    assert_eq!(stats.layer_cache_misses, 0);
}
