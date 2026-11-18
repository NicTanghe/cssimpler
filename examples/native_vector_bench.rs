//! Uncached static comparison. --mode native|svg --scene opaque|mixed --frames 20
use cssimpler::{
    core::{
        Color, Node, PathCommand::*, VectorPaint, VectorPath, VectorPoint as P, VectorScene,
        VectorViewBox,
    },
    renderer::render_to_buffer,
    style::{Stylesheet, build_render_tree},
};
use std::{
    hash::{DefaultHasher, Hash, Hasher},
    hint::black_box,
    time::Instant,
};
fn paths(mixed: bool) -> Vec<VectorPath> {
    (0..12)
        .map(|i| {
            let x = 20. + (i % 4) as f32 * 230.;
            let y = 20. + (i / 4) as f32 * 200.;
            VectorPath::new(
                vec![
                    MoveTo(P::new(x, y + 140.)),
                    CubicTo {
                        control1: P::new(x - 10., y + 20.),
                        control2: P::new(x + 120., y - 10.),
                        end: P::new(x + 190., y + 40.),
                    },
                    QuadTo {
                        control: P::new(x + 215., y + 170.),
                        end: P::new(x + 130., y + 155.),
                    },
                    LineTo(P::new(x, y + 140.)),
                    Close,
                ],
                VectorPaint {
                    fill: Some(if mixed && i % 2 == 0 {
                        Color::rgba(200, 220, 240, 128)
                    } else {
                        Color::rgb(200, 220, 240)
                    }),
                    stroke: Some(Color::rgb(30, 60, 90)),
                    stroke_width: 2.,
                },
            )
            .unwrap()
        })
        .collect()
}
fn build(paths: &[VectorPath], native: bool) -> Node {
    if native {
        Node::vector(
            VectorScene::new(VectorViewBox::new(0., 0., 960., 640.), paths.to_vec()).unwrap(),
        )
        .into()
    } else {
        Node::element("svg")
            .with_attribute("viewBox", "0 0 960 640")
            .with_children(paths.iter().map(|p| {
                let fill = p.paint().fill.unwrap();
                Node::element("path")
                    .with_attribute("d", p.to_svg_path_data())
                    .with_attribute(
                        "fill",
                        format!("#{:02x}{:02x}{:02x}{:02x}", fill.r, fill.g, fill.b, fill.a),
                    )
                    .with_attribute("stroke", "#1e3c5a")
                    .with_attribute("stroke-width", "2")
                    .into()
            }))
            .into()
    }
}
fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    (v[(v.len() - 1) / 2] + v[v.len() / 2]) * 0.5
}
fn measure(n: usize, mut f: impl FnMut()) -> f64 {
    let mut times = vec![];
    for _ in 0..n {
        let start = Instant::now();
        f();
        times.push(start.elapsed().as_secs_f64() * 1000.);
    }
    median(times)
}
fn mem(field: &str) -> Option<u64> {
    std::fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find_map(|l| {
            l.strip_prefix(field)?
                .split_whitespace()
                .next()?
                .parse()
                .ok()
        })
}
fn json(v: Option<u64>) -> String {
    v.map_or("null".into(), |v| v.to_string())
}
fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    let arg = |name| {
        args.windows(2)
            .find(|a| a[0] == name)
            .map(|a| a[1].as_str())
    };
    let mode = arg("--mode").unwrap_or("native");
    assert!(matches!(mode, "native" | "svg"));
    let frames = arg("--frames")
        .unwrap_or("20")
        .parse::<usize>()
        .unwrap()
        .max(1);
    let workload = arg("--scene").unwrap_or("opaque");
    assert!(matches!(workload, "opaque" | "mixed"));
    let source = paths(workload == "mixed");
    let native = mode == "native";
    let construct_ms = measure(frames, || {
        black_box(build(&source, native));
    });
    let node = build(&source, native);
    let css = Stylesheet::default();
    let prepare_ms = measure(frames, || {
        black_box(build_render_tree(&node, &css));
    });
    let scene = vec![build_render_tree(&node, &css)];
    let mut pixels = vec![0u32; 960 * 640];
    pixels.fill(black_box(0));
    let rss_before = mem("VmRSS:");
    render_to_buffer(&scene, &mut pixels, 960, 640, Color::WHITE);
    let paint_ms = measure(frames, || {
        render_to_buffer(black_box(&scene), &mut pixels, 960, 640, Color::WHITE);
        black_box(&pixels);
    });
    let mut checksum = DefaultHasher::new();
    pixels.hash(&mut checksum);
    println!(
        "{{\"mode\":\"{mode}\",\"scene\":\"{workload}\",\"checksum\":\"{:016x}\",\"frames\":{frames},\"construct_ms\":{construct_ms:.4},\"prepare_ms\":{prepare_ms:.4},\"paint_ms\":{paint_ms:.4},\"rss_before_paint_kib\":{},\"rss_after_paint_kib\":{},\"peak_rss_kib\":{}}}",
        checksum.finish(),
        json(rss_before),
        json(mem("VmRSS:")),
        json(mem("VmHWM:"))
    );
}
