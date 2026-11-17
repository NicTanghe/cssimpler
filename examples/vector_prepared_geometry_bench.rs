//! Stage-separated benchmark for prepared native vector geometry (issue #22).
//!
//! The existing vector benchmarks report one whole "paint" number. This harness
//! splits that number so the decision about a prepared-geometry cache rests on
//! measured shares instead of intuition:
//!
//! - path construction: authored commands -> tiny-skia `Path`
//! - stroke preparation: tiny-skia stroker outline
//! - projection: source -> device space (`Path::transform` or the perspective
//!   flattener)
//! - coverage rasterization: mask clear + `fill_path` per device-anchored tile
//! - coverage blending: the per-pixel mask/clip/color blend loop
//! - scene traversal: painting the same node tree with empty vector scenes
//! - finalize (headless presentation proxy): 10-bit buffer -> RGBA8
//!
//! The first five come from the renderer's opt-in probe
//! (`cssimpler::renderer::vector_preparation_stats`); scene traversal is
//! measured as the fast-paint difference against empty scenes; the RGBA8
//! conversion stands in for presentation because this environment is headless
//! (no softbuffer/WGPU swap). Presentation to a real window is not measured
//! here.
//!
//! Pixel-layer caching is off: every vector node keeps `cache_layer = false`
//! and the trees are painted through the plain `render_to_buffer` path.
//!
//! ```sh
//! cargo run --release --example vector_prepared_geometry_bench -- --scenario small-icon
//! cargo run --release --example vector_prepared_geometry_bench -- --scenario curves --json
//! ```
//!
//! One JSON line per invocation; see `scripts/vector_prepared_geometry_ab.py`
//! for the multi-process driver that prints medians and spreads.

use cssimpler::{
    core::{
        Color, LengthPercentageValue, Node, PathCommand::*, RenderNode, TransformOperation,
        VectorPaint, VectorPath, VectorPoint as P, VectorScene, VectorViewBox,
    },
    renderer::{
        VectorPreparationStats, render_to_buffer, render_to_rgba8, reset_vector_preparation_stats,
        set_vector_preparation_probe_enabled, vector_preparation_stats,
    },
    style::{Stylesheet, build_render_tree_in_viewport},
};
use std::{hint::black_box, time::Instant};

fn icon_path(x: f32, y: f32) -> VectorPath {
    VectorPath::new(
        vec![
            MoveTo(P::new(x + 4., y + 4.)),
            LineTo(P::new(x + 28., y + 4.)),
            LineTo(P::new(x + 28., y + 28.)),
            LineTo(P::new(x + 4., y + 28.)),
            LineTo(P::new(x + 4., y + 10.)),
            LineTo(P::new(x + 12., y + 10.)),
            LineTo(P::new(x + 12., y + 20.)),
            LineTo(P::new(x + 20., y + 20.)),
            Close,
        ],
        VectorPaint {
            fill: Some(Color::rgb(40, 90, 160)),
            stroke: Some(Color::rgb(10, 20, 40)),
            stroke_width: 1.5,
        },
    )
    .unwrap()
}

/// 128 small stroked icons on a 512x512 canvas (below the parallel paint
/// threshold, so stage sums can be compared with frame time directly).
fn icon_paths() -> Vec<VectorPath> {
    (0..128)
        .map(|index| {
            let column = (index % 16) as f32;
            let row = (index / 16) as f32;
            icon_path(column * 32.0, row * 32.0)
        })
        .collect()
}

fn curve_path(x: f32, y: f32) -> VectorPath {
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
            fill: Some(Color::rgb(200, 220, 240)),
            stroke: Some(Color::rgb(30, 60, 90)),
            stroke_width: 2.,
        },
    )
    .unwrap()
}

/// 12 curved, stroked drawings on a 960x640 canvas.
fn curve_paths() -> Vec<VectorPath> {
    (0..12)
        .map(|index| {
            let x = 20. + (index % 4) as f32 * 230.;
            let y = 20. + (index / 4) as f32 * 200.;
            curve_path(x, y)
        })
        .collect()
}

/// A detailed, stroke-only icon stand-in: 128 authored segments per path.
/// This is the most geometry-preparation-heavy shape the harness covers
/// (detailed strokes, little coverage area to blend).
fn dense_stroke_path(x: f32, y: f32) -> VectorPath {
    let mut commands = vec![MoveTo(P::new(x + 4., y + 4.))];
    for index in 1..128 {
        let t = index as f32 / 127.0;
        let zig = if index % 2 == 0 { 1.0 } else { -1.0 };
        commands.push(LineTo(P::new(
            x + 4. + t * 120.,
            y + 62. + zig * 48. * (1.0 - t * 0.5),
        )));
    }
    VectorPath::new(
        commands,
        VectorPaint {
            fill: None,
            stroke: Some(Color::rgb(10, 40, 90)),
            stroke_width: 1.2,
        },
    )
    .unwrap()
}

/// 16 detailed stroke-only motifs on a 512x512 canvas.
fn dense_stroke_paths() -> Vec<VectorPath> {
    let mut paths = Vec::with_capacity(16);
    for index in 0..16 {
        let column = (index % 4) as f32;
        let row = (index / 4) as f32;
        paths.push(dense_stroke_path(column * 128.0, row * 128.0));
    }
    paths
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// Same immutable geometry and paint on every frame.
    Static,
    /// Geometry unchanged; a translation-only transform changes per frame.
    Translate,
    /// Geometry unchanged; a magnification change per frame.
    Zoom,
    /// Geometry unchanged; one path is repainted per frame.
    PaintUpdate,
    /// One path is replaced with new geometry per frame.
    Edit,
}

struct Workload {
    name: &'static str,
    view_box: VectorViewBox,
    width: usize,
    height: usize,
    paths: Vec<VectorPath>,
    kind: Kind,
    iterations: usize,
}

impl Workload {
    fn new(
        name: &'static str,
        view_box: VectorViewBox,
        width: usize,
        height: usize,
        paths: Vec<VectorPath>,
        kind: Kind,
    ) -> Self {
        Self {
            name,
            view_box,
            width,
            height,
            iterations: if paths.len() > 64 { 40 } else { 24 },
            paths,
            kind,
        }
    }

    fn empty_paths(&self) -> Vec<VectorPath> {
        Vec::new()
    }

    fn node(&self, paths: &[VectorPath], step: usize) -> Node {
        let mut node: Node =
            Node::vector(VectorScene::new(self.view_box, paths.to_vec()).unwrap()).into();
        let Node::Element(element) = &mut node else {
            return node;
        };
        // Pixel-layer caching must stay off in both revisions.
        element.style.visual.cache_layer = false;
        match self.kind {
            Kind::Translate => {
                let offset = (step % 8) as f32 * 0.5;
                element.style.visual.transform.operations = vec![TransformOperation::Translate {
                    x: LengthPercentageValue::from_px(offset),
                    y: LengthPercentageValue::from_px(offset * 0.5),
                }];
            }
            Kind::Zoom => {
                // 1.0x .. 1.35x, changing every frame: stroke flattening
                // resolution changes with the scale, so any prepared outline
                // built for a previous frame would be incompatible.
                let scale = 1.0 + (step % 8) as f32 * 0.05;
                element.style.visual.transform.operations =
                    vec![TransformOperation::Scale { x: scale, y: scale }];
            }
            Kind::PaintUpdate => {
                if let Some(scene) = &mut element.vector_scene
                    && let Some(path) = scene.paths.get_mut(step % paths.len().max(1))
                {
                    path.paint.fill = Some(cssimpler::core::SvgPathPaintSource::Color(
                        if step % 2 == 0 {
                            Color::rgb(255, 0, 0)
                        } else {
                            Color::rgb(0, 0, 255)
                        },
                    ));
                }
            }
            Kind::Static | Kind::Edit => {}
        }
        node
    }

    /// Pre-build per-frame trees outside the timed region so the measurement is
    /// paint only. `Edit` builds new replacement geometry for one path per frame.
    fn trees(&self, stylesheet: &Stylesheet, steps: usize) -> Vec<Vec<RenderNode>> {
        let mut paths = self.paths.clone();
        (0..steps)
            .map(|step| {
                if self.kind == Kind::Edit && !paths.is_empty() {
                    let shift = (step % 32) as f32;
                    paths[0] = if paths.len() > 64 {
                        icon_path(shift, 0.)
                    } else {
                        curve_path(shift, 0.)
                    };
                }
                let node = self.node(&paths, step);
                let tree =
                    build_render_tree_in_viewport(&node, stylesheet, self.width, self.height);
                assert!(
                    !tree.style.cache_layer,
                    "pixel-layer caching must be off for this harness"
                );
                vec![tree]
            })
            .collect()
    }

    fn empty_trees(&self, stylesheet: &Stylesheet, steps: usize) -> Vec<Vec<RenderNode>> {
        let empty = self.empty_paths();
        (0..steps)
            .map(|step| {
                let node = self.node(&empty, step);
                vec![build_render_tree_in_viewport(
                    &node,
                    stylesheet,
                    self.width,
                    self.height,
                )]
            })
            .collect()
    }
}

struct FrameTimes {
    median_ms: f64,
    min_ms: f64,
    max_ms: f64,
}

impl FrameTimes {
    fn from(mut samples: Vec<f64>) -> Self {
        samples.sort_by(f64::total_cmp);
        Self {
            median_ms: samples[samples.len() / 2],
            min_ms: samples[0],
            max_ms: samples[samples.len() - 1],
        }
    }
}

/// Paint `trees` once and time every frame. Returns per-frame times in ms.
fn paint_frames(
    trees: &[Vec<RenderNode>],
    pixels: &mut [u32],
    width: usize,
    height: usize,
) -> FrameTimes {
    let mut samples = Vec::with_capacity(trees.len());
    for tree in trees {
        let start = Instant::now();
        render_to_buffer(tree, pixels, width, height, Color::WHITE);
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
        black_box(&pixels[0]);
    }
    FrameTimes::from(samples)
}

/// Fixed, allocation-free ALU workload used to normalize timings against the
/// speed this process actually received from the shared machine. Returns the
/// best wall time in ms of three runs.
fn calibrate() -> f64 {
    let run = || {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let start = Instant::now();
        for _ in 0..2_000_000u64 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
        }
        black_box(seed);
        start.elapsed().as_secs_f64() * 1000.0
    };
    (0..3).map(|_| run()).fold(f64::INFINITY, f64::min)
}

fn workload(name: &str) -> Option<Workload> {
    let icons = VectorViewBox::new(0., 0., 512., 512.);
    let curves = VectorViewBox::new(0., 0., 960., 640.);
    Some(match name {
        "small-icon" => Workload::new("small-icon", icons, 512, 512, icon_paths(), Kind::Static),
        "small-icon-translate" => Workload::new(
            "small-icon-translate",
            icons,
            512,
            512,
            icon_paths(),
            Kind::Translate,
        ),
        "small-icon-edit" => {
            Workload::new("small-icon-edit", icons, 512, 512, icon_paths(), Kind::Edit)
        }
        "dense-strokes" => Workload::new(
            "dense-strokes",
            icons,
            512,
            512,
            dense_stroke_paths(),
            Kind::Static,
        ),
        "curves" => Workload::new("curves", curves, 960, 640, curve_paths(), Kind::Static),
        // Below MIN_PARALLEL_RENDER_PIXELS (640 * 480): serial paint, so the
        // stage sums and the wall frame time are directly comparable.
        "curves-serial" => Workload::new(
            "curves-serial",
            curves,
            560,
            360,
            curve_paths(),
            Kind::Static,
        ),
        "curves-translate" => Workload::new(
            "curves-translate",
            curves,
            960,
            640,
            curve_paths(),
            Kind::Translate,
        ),
        "curves-zoom" => Workload::new("curves-zoom", curves, 960, 640, curve_paths(), Kind::Zoom),
        "curves-paint-update" => Workload::new(
            "curves-paint-update",
            curves,
            960,
            640,
            curve_paths(),
            Kind::PaintUpdate,
        ),
        "curves-edit" => Workload::new("curves-edit", curves, 960, 640, curve_paths(), Kind::Edit),
        _ => return None,
    })
}

fn ns_to_ms_per_frame(ns: u64, frames: usize) -> f64 {
    ns as f64 / 1_000_000.0 / frames.max(1) as f64
}

fn rss_kib() -> Option<u64> {
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        return status.lines().find_map(|line| {
            line.strip_prefix("VmRSS:")
                .and_then(|value| value.split_whitespace().next())
                .and_then(|value| value.parse().ok())
        });
    }
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    String::from_utf8(output.stdout).ok()?.trim().parse().ok()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| {
        args.windows(2)
            .find(|pair| pair[0] == name)
            .map(|pair| pair[1].as_str())
    };
    let name = arg("--scenario").unwrap_or("small-icon");
    let Some(workload) = workload(name) else {
        eprintln!(
            "unknown --scenario {name}; expected one of: small-icon, small-icon-translate, \
             small-icon-edit, dense-strokes, curves, curves-serial, curves-translate, \
             curves-zoom, curves-paint-update, curves-edit"
        );
        std::process::exit(2);
    };
    let label = arg("--label").unwrap_or("unlabeled");
    let probe_frames = arg("--probe-frames")
        .map(|value| {
            value
                .parse::<usize>()
                .expect("--probe-frames must be an integer")
        })
        .unwrap_or(workload.iterations)
        .max(1)
        .min(workload.iterations);

    let stylesheet = Stylesheet::default();
    let (width, height) = (workload.width, workload.height);
    let rss_before_kib = rss_kib();
    let calibration_ms = calibrate();

    let trees = workload.trees(&stylesheet, workload.iterations);
    let empty_trees = workload.empty_trees(&stylesheet, workload.iterations);
    let mut pixels = vec![0u32; width * height];

    // Warm caches and code paths before timing.
    for tree in trees.iter().take(2) {
        render_to_buffer(tree, &mut pixels, width, height, Color::WHITE);
    }

    set_vector_preparation_probe_enabled(false);
    let full = paint_frames(&trees, &mut pixels, width, height);
    let empty = paint_frames(&empty_trees, &mut pixels, width, height);

    // Probe run: stage sums are aggregate CPU work across paint workers, so they
    // are reported per frame and compared with the wall frame time.
    set_vector_preparation_probe_enabled(true);
    reset_vector_preparation_stats();
    for tree in trees.iter().take(probe_frames) {
        render_to_buffer(tree, &mut pixels, width, height, Color::WHITE);
        black_box(&pixels[0]);
    }
    let stats: VectorPreparationStats = vector_preparation_stats();
    set_vector_preparation_probe_enabled(false);

    // Wall time with the probe enabled, to expose clock overhead.
    set_vector_preparation_probe_enabled(true);
    reset_vector_preparation_stats();
    let probed_wall = paint_frames(&trees[..probe_frames], &mut pixels, width, height);
    set_vector_preparation_probe_enabled(false);

    // Headless presentation proxy: the runtime's 10-bit -> RGBA8 finalize.
    let mut rgba = vec![0u8; width * height * 4];
    let mut finalize_ms = Vec::with_capacity(3);
    for _ in 0..3 {
        let start = Instant::now();
        render_to_rgba8(&trees[0], &mut rgba, width, height, Color::WHITE);
        finalize_ms.push(start.elapsed().as_secs_f64() * 1000.0);
        black_box(rgba[0]);
    }
    finalize_ms.sort_by(f64::total_cmp);
    let finalize_ms = finalize_ms[finalize_ms.len() / 2];

    let rss_after_kib = rss_kib();
    let per_frame = |ns: u64| ns_to_ms_per_frame(ns, probe_frames);
    let json_optional =
        |value: Option<u64>| value.map_or("null".to_string(), |value| value.to_string());
    let geometry_prep_ms = per_frame(stats.geometry_prep_ns());
    let attributed_ms = per_frame(stats.attributed_ns());
    let vector_work_ms = (full.median_ms - empty.median_ms).max(0.0);
    // Geometry preparation is CPU work summed across paint workers. Divide by
    // the paint's CPU-to-wall ratio to express it in frame-time terms.
    let geometry_prep_wall_ms = if attributed_ms > 0.0 {
        geometry_prep_ms * full.median_ms / attributed_ms
    } else {
        0.0
    };
    println!(
        "{{\"scenario\":\"{}\",\"label\":\"{}\",\"width\":{width},\"height\":{height},\
\"frames\":{},\"probe_frames\":{probe_frames},\"calibration_ms\":{calibration_ms:.5},\
\"frame_ms\":{:.5},\"frame_min_ms\":{:.5},\"frame_max_ms\":{:.5},\"frame_norm\":{:.6},\
\"empty_scene_ms\":{:.5},\"vector_work_ms\":{:.5},\"vector_work_share\":{:.4},\
\"frame_probe_on_ms\":{:.5},\"probe_overhead_ms\":{:.5},\
\"finalize_ms\":{finalize_ms:.5},\
\"paths_per_frame\":{:.2},\"stroke_outlines_per_frame\":{:.2},\"coverage_tiles_per_frame\":{:.2},\
\"path_build_ms\":{:.5},\"stroke_prep_ms\":{:.5},\"projection_ms\":{:.5},\
\"coverage_raster_ms\":{:.5},\"coverage_blend_ms\":{:.5},\
\"geometry_prep_ms\":{geometry_prep_ms:.5},\"attributed_ms\":{attributed_ms:.5},\
\"geometry_prep_share_of_attributed\":{:.4},\
\"geometry_prep_wall_ms\":{geometry_prep_wall_ms:.5},\
\"geometry_prep_wall_share\":{:.4},\
\"finalize_minus_paint_ms\":{:.5},\
\"paths_per_second\":{:.1},\
\"rss_before_kib\":{},\"rss_after_kib\":{}}}",
        workload.name,
        label,
        workload.iterations,
        full.median_ms,
        full.min_ms,
        full.max_ms,
        full.median_ms / calibration_ms,
        empty.median_ms,
        vector_work_ms,
        if full.median_ms > 0.0 {
            vector_work_ms / full.median_ms
        } else {
            0.0
        },
        probed_wall.median_ms,
        probed_wall.median_ms - full.median_ms,
        stats.paths_painted as f64 / probe_frames as f64,
        stats.stroke_outlines as f64 / probe_frames as f64,
        stats.coverage_tiles as f64 / probe_frames as f64,
        per_frame(stats.path_build_ns),
        per_frame(stats.stroke_prep_ns),
        per_frame(stats.projection_ns),
        per_frame(stats.coverage_raster_ns),
        per_frame(stats.coverage_blend_ns),
        if attributed_ms > 0.0 {
            geometry_prep_ms / attributed_ms
        } else {
            0.0
        },
        if full.median_ms > 0.0 {
            geometry_prep_wall_ms / full.median_ms
        } else {
            0.0
        },
        finalize_ms - full.median_ms,
        stats.paths_painted as f64 / probe_frames as f64 / (full.median_ms / 1000.0).max(1e-9),
        json_optional(rss_before_kib),
        json_optional(rss_after_kib),
    );
}
