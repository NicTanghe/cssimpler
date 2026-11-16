//! Run with `cargo run --example native_vector_paths -- --export work/curves.svg`.
use cssimpler::{
    app::{App, Invalidation},
    core::{
        Color, Node, PathCommand::*, VectorPaint, VectorPath, VectorPoint as P, VectorScene,
        VectorViewBox,
    },
    renderer::{FrameInfo, WindowConfig},
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
    let css = parse_stylesheet("#artwork { width: 400px; height: 300px; }")?;
    App::new(scene, &css, update, view)
        .run(WindowConfig::new("Native vector curves", 440, 340))
        .map_err(Into::into)
}
