//! Run with `cargo run --example native_vector_paths -- --export work/curves.svg`.
//! Pass `--diagnostics` to print the last completed frame after the window closes.
//! Add `--frames N` to exit automatically after N repaints, for scripted runs.
use std::{
    fs::File,
    io::{BufWriter, Write},
    sync::atomic::{AtomicU32, Ordering},
};

use cssimpler::{
    app::{App, Invalidation},
    core::{
        Color, Node, PathCommand::*, VectorPaint, VectorPath, VectorPoint as P, VectorScene,
        VectorViewBox,
    },
    renderer::{
        FrameInfo, FramePaintMode, WindowConfig, latest_frame_timing_stats, layer_cache_stats,
        set_vector_timing_enabled,
    },
    style::parse_stylesheet,
};

fn artwork(offset_x: f32) -> VectorScene {
    let point = |x: f32, y: f32| P::new(x + offset_x, y);
    VectorScene::new(
        VectorViewBox::new(0., 0., 400., 300.),
        vec![
            VectorPath::new(
                vec![
                    MoveTo(point(40., 240.)),
                    CubicTo {
                        control1: point(20., 30.),
                        control2: point(260., 10.),
                        end: point(350., 100.),
                    },
                    QuadTo {
                        control: point(390., 260.),
                        end: point(250., 250.),
                    },
                    LineTo(point(40., 240.)),
                    Close,
                ],
                VectorPaint {
                    fill: Some(Color::rgb(200, 220, 240)),
                    stroke: Some(Color::rgb(30, 60, 90)),
                    stroke_width: 2.,
                },
            )
            .unwrap(),
        ],
    )
    .unwrap()
}
static FRAMES_PAINTED: AtomicU32 = AtomicU32::new(0);
static EXIT_AFTER_FRAMES: AtomicU32 = AtomicU32::new(0);

fn update(scene: &mut VectorScene, frame: FrameInfo) -> Invalidation {
    if EXIT_AFTER_FRAMES.load(Ordering::Relaxed) == 0 {
        return Invalidation::Clean;
    }
    FRAMES_PAINTED.fetch_add(1, Ordering::Relaxed);
    // Scripted runs shift the artwork so every counted frame actually paints.
    *scene = artwork((frame.frame_index % 2) as f32);
    Invalidation::Paint
}

fn exit_after_requested_frames(_: &VectorScene) -> bool {
    let limit = EXIT_AFTER_FRAMES.load(Ordering::Relaxed);
    limit > 0 && FRAMES_PAINTED.load(Ordering::Relaxed) >= limit
}
fn view(scene: &VectorScene) -> Node {
    Node::vector(scene.clone()).with_id("artwork").into()
}
fn main() -> anyhow::Result<()> {
    let scene = artwork(0.0);
    let args = std::env::args().collect::<Vec<_>>();
    if let Some(pair) = args.windows(2).find(|pair| pair[0] == "--export") {
        let mut file = BufWriter::new(File::create(&pair[1])?);
        scene.write_svg(&mut file)?;
        file.flush()?;
        return Ok(());
    }
    let diagnostics = args.iter().any(|arg| arg == "--diagnostics");
    if let Some(pair) = args.windows(2).find(|pair| pair[0] == "--frames") {
        EXIT_AFTER_FRAMES.store(pair[1].parse().unwrap_or(0), Ordering::Relaxed);
    }
    set_vector_timing_enabled(diagnostics);
    let css = parse_stylesheet("#artwork { width: 400px; height: 300px; }")?;
    App::new(scene, &css, update, view)
        .with_exit_when(exit_after_requested_frames)
        .run(WindowConfig::new("Native vector curves", 440, 340))?;
    if diagnostics {
        let frame = latest_frame_timing_stats();
        let layers = layer_cache_stats();
        eprintln!(
            "last frame: mode={:?} reason={:?} paint={}us present={}us workers={}",
            frame.paint_mode,
            frame.paint_reason,
            frame.paint_us,
            frame.present_us,
            frame.render_workers
        );
        eprintln!(
            "vector: paths_painted={} build={}us stroke={}us projection={}us coverage={}us timers={}",
            frame.vector.paths_painted,
            frame.vector.path_build_work_us,
            frame.vector.stroke_work_us,
            frame.vector.projection_work_us,
            frame.vector.coverage_work_us,
            frame.vector.timings_enabled
        );
        eprintln!(
            "layer cache: hits/misses={}/{} this frame; retained {} entries / {} bytes; cumulative builds={}",
            frame.layer_cache_hits,
            frame.layer_cache_misses,
            layers.entries,
            layers.bytes,
            layers.builds
        );
        if frame.paint_mode == FramePaintMode::Idle {
            eprintln!(
                "note: the last frame was idle, so its vector counts are zero; interact with the window and close it after a repaint."
            );
        } else if frame.layer_cache_misses > 0 {
            eprintln!(
                "note: layer misses with a rising build count on unchanged artwork point at invalidation or cache limits; see docs/vector-diagnostics.md."
            );
        }
    }
    Ok(())
}
