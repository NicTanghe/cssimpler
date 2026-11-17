//! Native vector viewport, with simulated drag/wheel input and uncached measurements.
//! Run: cargo run --release --example native_pan_zoom -- 100
use cssimpler::{
    core::{
        Color, LayoutBox, PathCommand::*, RenderKind, RenderNode, VectorPaint, VectorPath,
        VectorPoint as P, VectorScene, VectorView, VectorViewBox,
    },
    renderer::{LayerCacheLimits, render_to_buffer, set_layer_cache_limits},
};
use std::{hint::black_box, time::Instant};

fn rss() -> Option<u64> {
    std::fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find_map(|line| {
            line.strip_prefix("VmRSS:")?
                .split_whitespace()
                .next()?
                .parse()
                .ok()
        })
}

fn main() {
    const WIDTH: usize = 320;
    const HEIGHT: usize = 240;
    let frames = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(100usize)
        .max(1);
    let path = VectorPath::new(
        vec![
            MoveTo(P::new(20., 180.)),
            CubicTo {
                control1: P::new(0., 10.),
                control2: P::new(300., 0.),
                end: P::new(280., 180.),
            },
            QuadTo {
                control: P::new(160., 240.),
                end: P::new(20., 180.),
            },
            Close,
        ],
        VectorPaint {
            fill: Some(Color::rgb(100, 160, 220)),
            stroke: Some(Color::BLACK),
            stroke_width: 2.,
        },
    )
    .unwrap();
    let authored = VectorScene::new(
        VectorViewBox::new(0., 0., WIDTH as f32, HEIGHT as f32),
        vec![path],
    )
    .unwrap();
    let original_svg = authored.to_svg();
    // Convert once. Keep authored geometry for editing/export, and retain the render node.
    let mut node = RenderNode::vector(
        LayoutBox::new(0., 0., WIDTH as f32, HEIGHT as f32),
        authored.clone(),
    );
    let mut view = VectorView::new(0.1, 16.).unwrap();
    let mut pixels = vec![0; WIDTH * HEIGHT];
    set_layer_cache_limits(LayerCacheLimits {
        total_bytes: 0,
        layer_bytes: 0,
    });
    render_to_buffer(
        std::slice::from_ref(&node),
        &mut pixels,
        WIDTH,
        HEIGHT,
        Color::WHITE,
    );
    let rss_before = rss();
    let mut prepare = 0.;
    let mut paint = 0.;
    for i in 0..frames {
        let start = Instant::now();
        // Pointer positions are local logical pixels after undoing ancestor CSS transforms.
        let pointer = P::new(140., 100.);
        let _document_pointer = view.to_document(pointer).unwrap();
        view.pan(P::new(if i % 2 == 0 { 1. } else { -1. }, 0.))
            .unwrap();
        view.zoom_at(if i % 2 == 0 { 1.01 } else { 1. / 1.01 }, pointer)
            .unwrap();
        let RenderKind::Svg(scene) = &mut node.kind else {
            unreachable!()
        };
        scene.view_box = view.view_box(WIDTH as f32, HEIGHT as f32).unwrap();
        prepare += start.elapsed().as_secs_f64();
        let start = Instant::now();
        render_to_buffer(
            std::slice::from_ref(&node),
            &mut pixels,
            WIDTH,
            HEIGHT,
            Color::WHITE,
        );
        paint += start.elapsed().as_secs_f64();
        black_box(&pixels);
    }
    assert_eq!(authored.to_svg(), original_svg);
    println!(
        "frames={frames} prepare_mean_ms={:.4} paint_mean_ms={:.4} rss_before_kib={rss_before:?} rss_after_kib={:?}",
        prepare * 1000. / frames as f64,
        paint * 1000. / frames as f64,
        rss()
    );
}
