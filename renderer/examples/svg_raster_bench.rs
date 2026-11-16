//! Headless normal-SVG comparison: --case icons|panels --frames 12.
//! Reports per-frame render time and Linux process RSS, including one warmup.
use std::{hint::black_box, time::Instant};

use cssimpler_core::{
    Color, LayoutBox, RenderNode, SvgContour, SvgPathGeometry, SvgPathInstance, SvgPathPaint,
    SvgPathPaintSource, SvgPoint, SvgScene, SvgViewBox,
};
use cssimpler_renderer::render_to_buffer;

const WIDTH: usize = 960;
const HEIGHT: usize = 640;

fn contour(points: &[(f32, f32)], closed: bool) -> SvgContour {
    SvgContour {
        points: points.iter().map(|&(x, y)| SvgPoint::new(x, y)).collect(),
        closed,
    }
}

fn ellipse(cx: f32, cy: f32, rx: f32, ry: f32, segments: usize, reverse: bool) -> SvgContour {
    SvgContour {
        points: (0..segments)
            .map(|i| {
                let angle = i as f32 * std::f32::consts::TAU / segments as f32
                    * if reverse { -1.0 } else { 1.0 };
                SvgPoint::new(cx + rx * angle.cos(), cy + ry * angle.sin())
            })
            .collect(),
        closed: true,
    }
}

fn path(
    contours: Vec<SvgContour>,
    fill: Option<Color>,
    stroke: Option<Color>,
    stroke_width: f32,
) -> SvgPathInstance {
    SvgPathInstance {
        geometry: SvgPathGeometry::new(contours),
        paint: SvgPathPaint {
            fill: fill.map(SvgPathPaintSource::Color),
            stroke: stroke.map(SvgPathPaintSource::Color),
            stroke_width,
        },
    }
}

fn icons() -> Vec<RenderNode> {
    let ink = Color::rgb(35, 55, 80);
    let blue = Color::rgb(70, 140, 220);
    (0..192)
        .map(|index| {
            let paths = match index % 4 {
                0 => vec![
                    path(
                        vec![ellipse(12.0, 12.0, 9.0, 9.0, 32, false)],
                        Some(blue),
                        None,
                        0.0,
                    ),
                    path(
                        vec![contour(&[(7.0, 12.0), (10.0, 15.0), (17.0, 8.0)], false)],
                        None,
                        Some(Color::WHITE),
                        1.8,
                    ),
                ],
                1 => vec![
                    path(
                        vec![contour(
                            &[
                                (2.0, 6.0),
                                (9.0, 6.0),
                                (11.0, 9.0),
                                (22.0, 9.0),
                                (20.0, 20.0),
                                (2.0, 20.0),
                            ],
                            true,
                        )],
                        Some(Color::rgb(240, 195, 70)),
                        Some(ink),
                        1.2,
                    ),
                    path(
                        vec![contour(&[(4.0, 11.0), (19.0, 11.0)], false)],
                        None,
                        Some(ink),
                        1.0,
                    ),
                ],
                2 => vec![
                    path(
                        vec![
                            ellipse(10.0, 10.0, 7.0, 7.0, 32, false),
                            ellipse(10.0, 10.0, 5.0, 5.0, 32, true),
                        ],
                        Some(ink),
                        None,
                        0.0,
                    ),
                    path(
                        vec![contour(&[(15.0, 15.0), (21.0, 21.0)], false)],
                        None,
                        Some(ink),
                        2.5,
                    ),
                ],
                _ => vec![
                    path(
                        vec![contour(
                            &[(4.0, 19.0), (4.0, 11.0), (7.0, 11.0), (7.0, 19.0)],
                            true,
                        )],
                        Some(blue),
                        None,
                        0.0,
                    ),
                    path(
                        vec![contour(
                            &[(10.0, 19.0), (10.0, 5.0), (13.0, 5.0), (13.0, 19.0)],
                            true,
                        )],
                        Some(blue),
                        None,
                        0.0,
                    ),
                    path(
                        vec![contour(
                            &[(16.0, 19.0), (16.0, 8.0), (19.0, 8.0), (19.0, 19.0)],
                            true,
                        )],
                        Some(blue),
                        None,
                        0.0,
                    ),
                    path(
                        vec![contour(&[(2.0, 3.0), (2.0, 21.0), (22.0, 21.0)], false)],
                        None,
                        Some(ink),
                        1.0,
                    ),
                ],
            };
            RenderNode::svg(
                LayoutBox::new(
                    48.0 + (index % 16) as f32 * 54.0,
                    36.0 + (index / 16) as f32 * 48.0,
                    24.0,
                    24.0,
                ),
                SvgScene::new(SvgViewBox::new(0.0, 0.0, 24.0, 24.0), paths),
            )
        })
        .collect()
}

fn panels() -> Vec<RenderNode> {
    (0..12)
        .map(|index| {
            let paths = vec![
                path(
                    vec![
                        ellipse(100.0, 75.0, 86.0, 60.0, 64, false),
                        ellipse(100.0, 75.0, 14.0, 10.0, 24, true),
                    ],
                    Some(Color::rgb(200, 220, 240)),
                    Some(Color::rgb(30, 60, 90)),
                    2.0,
                ),
                path(
                    vec![contour(
                        &[(35.0, 75.0), (75.0, 60.0), (125.0, 65.0), (165.0, 75.0)],
                        false,
                    )],
                    None,
                    Some(Color::rgba(90, 110, 130, 180)),
                    1.0,
                ),
            ];
            RenderNode::svg(
                LayoutBox::new(
                    24.0 + (index % 4) as f32 * 232.0,
                    28.0 + (index / 4) as f32 * 200.0,
                    210.0,
                    170.0,
                ),
                SvgScene::new(SvgViewBox::new(0.0, 0.0, 200.0, 150.0), paths),
            )
        })
        .collect()
}

fn memory_kib(field: &str) -> Option<u64> {
    // Process resident memory, not allocator bytes. None on non-Linux hosts.
    std::fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find_map(|line| {
            line.strip_prefix(field)?
                .split_whitespace()
                .next()?
                .parse()
                .ok()
        })
}

fn json_memory(value: Option<u64>) -> String {
    value.map_or_else(|| "null".to_owned(), |value| value.to_string())
}

fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    let value = |name| {
        args.windows(2)
            .find(|pair| pair[0] == name)
            .map(|pair| pair[1].as_str())
    };
    let case = value("--case").unwrap_or("icons");
    let frames = value("--frames")
        .unwrap_or("12")
        .parse::<usize>()
        .expect("--frames must be an integer")
        .max(1);
    let children = match case {
        "icons" => icons(),
        "panels" => panels(),
        _ => panic!("--case must be icons or panels"),
    };
    let svg_nodes = children.len();
    let paths = children
        .iter()
        .map(|node| {
            if let cssimpler_core::RenderKind::Svg(svg) = &node.kind {
                svg.paths.len()
            } else {
                0
            }
        })
        .sum::<usize>();
    let root = RenderNode::container(LayoutBox::new(0.0, 0.0, WIDTH as f32, HEIGHT as f32))
        .with_children(children);
    let scene = vec![root];
    let mut buffer = vec![0; WIDTH * HEIGHT];
    buffer.fill(black_box(0)); // Touch framebuffer pages before measuring baseline RSS.
    let before_kib = memory_kib("VmRSS:");
    let warmup_start = Instant::now();
    render_to_buffer(&scene, &mut buffer, WIDTH, HEIGHT, Color::WHITE);
    let warmup_ms = warmup_start.elapsed().as_secs_f64() * 1000.0;
    let warm_kib = memory_kib("VmRSS:");
    let mut samples = Vec::with_capacity(frames);
    for _ in 0..frames {
        let start = Instant::now();
        render_to_buffer(black_box(&scene), &mut buffer, WIDTH, HEIGHT, Color::WHITE);
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
        black_box(&buffer);
    }
    samples.sort_by(f64::total_cmp);
    let median_ms = if frames % 2 == 0 {
        (samples[frames / 2 - 1] + samples[frames / 2]) / 2.0
    } else {
        samples[frames / 2]
    };
    let p95_ms = samples[(frames * 95).div_ceil(100).saturating_sub(1)];
    let after_kib = memory_kib("VmRSS:");
    let peak_kib = memory_kib("VmHWM:");
    println!(
        "{{\"case\":\"{case}\",\"width\":{WIDTH},\"height\":{HEIGHT},\"svg_nodes\":{svg_nodes},\"paths\":{paths},\"frames\":{frames},\"warmup_ms\":{warmup_ms:.3},\"median_ms\":{median_ms:.3},\"p95_ms\":{p95_ms:.3},\"rss_before_kib\":{},\"rss_warm_kib\":{},\"rss_after_kib\":{},\"peak_rss_kib\":{}}}",
        json_memory(before_kib),
        json_memory(warm_kib),
        json_memory(after_kib),
        json_memory(peak_kib)
    );
}
