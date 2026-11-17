//! Run with `cargo run --example native_vector_paths -- --export work/curves.svg`.
//! Pass `--diagnostics` to print the last completed frame after the window closes.
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

fn artwork() -> VectorScene {
    VectorScene::new(
        VectorViewBox::new(0., 0., 400., 300.),
        vec![
            VectorPath::new(
                vec![
                    MoveTo(P::new(40., 240.)),
                    CubicTo {
                        control1: P::new(20., 30.),
                        control2: P::new(260., 10.),
                        end: P::new(350., 100.),
                    },
                    QuadTo {
                        control: P::new(390., 260.),
                        end: P::new(250., 250.),
                    },
                    LineTo(P::new(40., 240.)),
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
fn update(_: &mut VectorScene, _: FrameInfo) -> Invalidation {
    Invalidation::Clean
}
fn view(scene: &VectorScene) -> Node {
    Node::vector(scene.clone()).with_id("artwork").into()
}
fn main() -> anyhow::Result<()> {
    let scene = artwork();
    let args = std::env::args().collect::<Vec<_>>();
    if let Some(pair) = args.windows(2).find(|pair| pair[0] == "--export") {
        std::fs::write(&pair[1], scene.to_svg())?;
        return Ok(());
    }
    let diagnostics = args.iter().any(|arg| arg == "--diagnostics");
    set_vector_timing_enabled(diagnostics);
    let css = parse_stylesheet("#artwork { width: 400px; height: 300px; }")?;
    App::new(scene, &css, update, view).run(WindowConfig::new("Native vector curves", 440, 340))?;
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
