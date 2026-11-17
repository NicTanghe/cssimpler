//! Fluent builder for native vector paths, rendered through `Node::vector`.
//!
//! Headless (the default) builds the scene, prints a summary, and writes
//! `work/vector_builder.svg`:
//!
//! ```sh
//! cargo run --example vector_builder
//! cargo run --example vector_builder -- --export work/curves.svg
//! ```
//!
//! Pass `--show` to open the interactive window:
//!
//! ```sh
//! cargo run --example vector_builder -- --show
//! ```
use cssimpler::{
    app::{App, Invalidation},
    core::{Color, Node, VectorPath, VectorPoint as P, VectorScene, VectorViewBox},
    renderer::{FrameInfo, WindowConfig},
    style::parse_stylesheet,
};

fn artwork() -> anyhow::Result<VectorScene> {
    // One closed contour with a fill and a stroke.
    let body = VectorPath::builder()
        .reserve(6)
        .move_to(P::new(40.0, 240.0))
        .cubic_to(
            P::new(20.0, 30.0),
            P::new(260.0, 10.0),
            P::new(350.0, 100.0),
        )
        .quad_to(P::new(390.0, 260.0), P::new(250.0, 250.0))
        .line_to(P::new(40.0, 240.0))
        .close()
        .fill(Color::rgb(200, 220, 240))
        .stroke(Color::rgb(30, 60, 90))
        .stroke_width(2.0)
        .build()?;

    // A second, deliberately open contour: stroke only.
    let accent = VectorPath::builder()
        .move_to(P::new(80.0, 220.0))
        .line_to(P::new(140.0, 120.0))
        .quad_to(P::new(200.0, 60.0), P::new(260.0, 120.0))
        .cubic_to(
            P::new(300.0, 160.0),
            P::new(320.0, 200.0),
            P::new(340.0, 230.0),
        )
        .no_fill()
        .stroke(Color::rgba(220, 120, 40, 200))
        .stroke_width(3.0)
        .build()?;

    Ok(VectorScene::new(
        VectorViewBox::new(0.0, 0.0, 400.0, 300.0),
        vec![body, accent],
    )?)
}

fn update(_: &mut VectorScene, _: FrameInfo) -> Invalidation {
    Invalidation::Clean
}

fn view(scene: &VectorScene) -> Node {
    Node::vector(scene.clone())
        .with_id("builder-artwork")
        .into()
}

fn main() -> anyhow::Result<()> {
    let scene = artwork()?;

    let mut show = false;
    let mut export = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--show" => show = true,
            "--export" => {
                export = Some(
                    args.next()
                        .ok_or_else(|| anyhow::anyhow!("--export requires an output path"))?,
                )
            }
            other => anyhow::bail!("unknown argument {other:?}; use --show or --export <path>"),
        }
    }

    if show {
        let css = parse_stylesheet("#builder-artwork { width: 400px; height: 300px; }")?;
        App::new(scene, &css, update, view)
            .run(WindowConfig::new("Fluent vector builder", 440, 340))
            .map_err(Into::into)
    } else {
        // Exercise the same `Node::vector` view used by the interactive app
        // without requiring a window.
        if let Node::Element(element) = view(&scene) {
            println!(
                "built Node::vector <{}> with id {:?}",
                element.tag, element.id
            );
        }
        let target = export.unwrap_or_else(|| "work/vector_builder.svg".to_owned());
        if let Some(parent) = std::path::Path::new(&target).parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&target, scene.to_svg())?;
        println!(
            "wrote {target} with {} paths; pass --show to open the UI",
            scene.paths().len()
        );
        Ok(())
    }
}
