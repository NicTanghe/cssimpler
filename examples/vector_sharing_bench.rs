//! A/B harness for native vector geometry sharing (issue #17).
//!
//! This example is deliberately restricted to the native-vector API that exists
//! both before and after shared geometry, so the same source file can be built
//! at the baseline revision and on the shared-geometry branch. Run it in a
//! release build; see `scripts/vector_sharing_ab.py` for the alternating A/B
//! driver.
//!
//! `cargo run --release --example vector_sharing_bench -- --scenario small-icon --label head`
//!
//! Pixel-layer caching is off: every node leaves `cache_layer` at its default
//! (false), so `render_to_buffer` paints directly. The harness reports
//! allocation calls and requested bytes for each phase, using a wrapping system
//! allocator. Allocation counting is enabled for both revisions, so its overhead
//! is symmetric; it does not measure resident memory (see
//! `core/examples/vector_clone_measure.rs` for retained-storage RSS).

use cssimpler::{
    core::{
        Color, LengthPercentageValue, Node, PathCommand::*, SvgPathPaintSource, TransformOperation,
        VectorPaint, VectorPath, VectorPoint as P, VectorScene, VectorViewBox,
    },
    renderer::render_to_buffer,
    style::{Stylesheet, build_render_tree_in_viewport},
};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    hint::black_box,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

struct CountingAllocator;
static ALLOC_CALLS: AtomicU64 = AtomicU64::new(0);
static ALLOC_BYTES: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOC_CALLS.fetch_add(1, Ordering::Relaxed);
        ALLOC_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        // SAFETY: forwarded unchanged to the system allocator.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: forwarded unchanged to the system allocator.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator;

fn reset_counters() {
    ALLOC_CALLS.store(0, Ordering::Relaxed);
    ALLOC_BYTES.store(0, Ordering::Relaxed);
}

fn counter_values() -> (u64, u64) {
    (
        ALLOC_CALLS.load(Ordering::Relaxed),
        ALLOC_BYTES.load(Ordering::Relaxed),
    )
}

fn rss_kib() -> Option<u64> {
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        return status.lines().find_map(|line| {
            line.strip_prefix("VmRSS:")
                .and_then(|value| value.split_whitespace().next())
                .and_then(|value| value.parse().ok())
        });
    }
    let pid = std::process::id().to_string();
    let output = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid])
        .output()
        .ok()?;
    String::from_utf8(output.stdout).ok()?.trim().parse().ok()
}

fn peak_rss_kib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status.lines().find_map(|line| {
        line.strip_prefix("VmHWM:")
            .and_then(|value| value.split_whitespace().next())
            .and_then(|value| value.parse().ok())
    })
}

struct Sample {
    median_ms: f64,
    min_ms: f64,
    max_ms: f64,
    allocs_per_op: f64,
    bytes_per_op: f64,
}

fn sample(mut times: Vec<f64>, allocs: u64, bytes: u64, ops: usize) -> Sample {
    times.sort_by(f64::total_cmp);
    let median_ms = times[times.len() / 2];
    Sample {
        median_ms,
        min_ms: times[0],
        max_ms: times[times.len() - 1],
        allocs_per_op: allocs as f64 / ops as f64,
        bytes_per_op: bytes as f64 / ops as f64,
    }
}

fn measure(warmup: usize, iterations: usize, mut operation: impl FnMut()) -> Sample {
    for _ in 0..warmup {
        operation();
    }
    reset_counters();
    let mut times = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let start = Instant::now();
        operation();
        times.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    let (allocs, bytes) = counter_values();
    sample(times, allocs, bytes, iterations)
}

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

/// 128 small icon instances on a 512x512 canvas.
fn icon_paths() -> Vec<VectorPath> {
    let mut paths = Vec::with_capacity(128);
    for index in 0..128 {
        let column = (index % 16) as f32;
        let row = (index / 16) as f32;
        paths.push(icon_path(column * 32.0, row * 32.0));
    }
    paths
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

/// Twelve curved, stroked drawings on a 960x640 canvas.
fn curve_paths() -> Vec<VectorPath> {
    (0..12)
        .map(|index| {
            let x = 20. + (index % 4) as f32 * 230.;
            let y = 20. + (index / 4) as f32 * 200.;
            curve_path(x, y)
        })
        .collect()
}

#[derive(Clone, Copy)]
enum Scenario {
    SmallIcon,
    Curves,
    TransformUpdate,
    PaintUpdate,
    RepeatedEdits,
}

impl Scenario {
    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "small-icon" => Self::SmallIcon,
            "curves" => Self::Curves,
            "transform-update" => Self::TransformUpdate,
            "paint-update" => Self::PaintUpdate,
            "repeated-edits" => Self::RepeatedEdits,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            Self::SmallIcon => "small-icon",
            Self::Curves => "curves",
            Self::TransformUpdate => "transform-update",
            Self::PaintUpdate => "paint-update",
            Self::RepeatedEdits => "repeated-edits",
        }
    }

    fn view_box(self) -> VectorViewBox {
        match self {
            Self::SmallIcon => VectorViewBox::new(0., 0., 512., 512.),
            _ => VectorViewBox::new(0., 0., 960., 640.),
        }
    }

    fn buffer(self) -> (usize, usize) {
        match self {
            Self::SmallIcon => (512, 512),
            _ => (960, 640),
        }
    }

    fn paint_iterations(self) -> usize {
        match self {
            Self::SmallIcon => 40,
            _ => 8,
        }
    }

    fn source_paths(self) -> Vec<VectorPath> {
        match self {
            Self::SmallIcon => icon_paths(),
            _ => curve_paths(),
        }
    }

    /// Build a node with a freshly cloned scene (the per-view rebuild path).
    fn base_node(self, paths: &[VectorPath]) -> Node {
        Node::vector(VectorScene::new(self.view_box(), paths.to_vec()).unwrap()).into()
    }

    /// One frame where only the outer transform changes; geometry is reused.
    fn transform_node(self, scene: &VectorScene, step: usize) -> Node {
        let mut node: Node = Node::vector(scene.clone()).into();
        let offset = (step % 8) as f32 * 0.5;
        if let Node::Element(element) = &mut node {
            element.style.visual.transform.operations = vec![TransformOperation::Translate {
                x: LengthPercentageValue::from_px(offset),
                y: LengthPercentageValue::from_px(offset * 0.5),
            }];
        }
        node
    }

    /// One frame where only one path's paint changes; geometry is reused.
    fn paint_node(self, scene: &VectorScene, step: usize) -> Node {
        let mut node: Node = Node::vector(scene.clone()).into();
        if let Node::Element(element) = &mut node
            && let Some(scene) = &mut element.vector_scene
            && let Some(path) = scene.paths.get_mut(step % 12)
        {
            path.paint.fill = Some(SvgPathPaintSource::Color(if step % 2 == 0 {
                Color::rgb(255, 0, 0)
            } else {
                Color::rgb(0, 0, 255)
            }));
        }
        node
    }

    /// One frame that replaces the first path's geometry (an edit).
    fn edit_node(self, paths: &mut Vec<VectorPath>, step: usize) -> Node {
        let shift = (step % 32) as f32;
        let replacement = match self {
            Self::SmallIcon => icon_path(shift, 0.),
            _ => curve_path(shift, 0.),
        };
        paths[0] = replacement;
        self.base_node(paths)
    }
}

fn styled_node(mut node: Node) -> Node {
    // Keep pixel-layer caching off explicitly; the default is already false.
    if let Node::Element(element) = &mut node {
        element.style.visual.cache_layer = false;
    }
    node
}

fn json_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Fixed, allocation-free ALU workload used to normalize timings against the
/// speed this process actually received from the (possibly shared, thermally
/// throttled or E-core-scheduled) machine. Returns the best wall time in ms of
/// three calibration runs; callers divide measured phases by it.
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
    let mut best = f64::INFINITY;
    for _ in 0..3 {
        best = best.min(run());
    }
    best
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| {
        args.windows(2)
            .find(|pair| pair[0] == name)
            .map(|pair| pair[1].as_str())
    };
    let scenario = Scenario::parse(arg("--scenario").unwrap_or("small-icon"))
        .expect("scenario must be one of small-icon, curves, transform-update, paint-update, repeated-edits");
    let label = arg("--label").unwrap_or("unlabeled");
    let frames_scale = arg("--scale")
        .map(|value| value.parse::<f64>().expect("--scale must be a number"))
        .unwrap_or(1.0)
        .max(0.01);

    let paths = scenario.source_paths();
    let scene = VectorScene::new(scenario.view_box(), paths.clone()).unwrap();
    let stylesheet = Stylesheet::default();
    let (width, height) = scenario.buffer();
    let iterations = ((match scenario {
        Scenario::SmallIcon => 400,
        Scenario::Curves | Scenario::TransformUpdate | Scenario::PaintUpdate => 200,
        Scenario::RepeatedEdits => 120,
    }) as f64
        * frames_scale)
        .round()
        .max(1.0) as usize;
    let paint_iterations = ((scenario.paint_iterations() as f64) * frames_scale)
        .round()
        .max(1.0) as usize;

    let rss_before = rss_kib();
    let calibration_ms = calibrate();
    let normalized = |sample: &Sample| sample.median_ms / calibration_ms;

    let construct = measure(3, iterations, || {
        black_box(scenario.base_node(&paths));
    });

    let node = styled_node(scenario.base_node(&paths));
    let prepare = measure(3, iterations, || {
        black_box(build_render_tree_in_viewport(
            &node,
            &stylesheet,
            width,
            height,
        ));
    });

    let tree = build_render_tree_in_viewport(&node, &stylesheet, width, height);
    let render_scene = vec![tree];
    assert!(
        !render_scene[0].style.cache_layer,
        "pixel-layer caching must be off for this harness"
    );
    let mut pixels = vec![0u32; width * height];
    let paint = measure(1, paint_iterations, || {
        render_to_buffer(&render_scene, &mut pixels, width, height, Color::WHITE);
        black_box(&pixels);
    });

    // Per-frame update: scene rebuild + style/layout resolution, with the
    // scenario-specific mutation applied first.
    let mut mutable_paths = paths.clone();
    let mut step = 0usize;
    let update = measure(3, iterations, || {
        step = step.wrapping_add(1);
        let step = black_box(step);
        let node = match scenario {
            Scenario::SmallIcon | Scenario::Curves => {
                styled_node(scenario.base_node(&mutable_paths))
            }
            Scenario::TransformUpdate => styled_node(scenario.transform_node(&scene, step)),
            Scenario::PaintUpdate => styled_node(scenario.paint_node(&scene, step)),
            Scenario::RepeatedEdits => styled_node(scenario.edit_node(&mut mutable_paths, step)),
        };
        black_box(build_render_tree_in_viewport(
            &node,
            &stylesheet,
            width,
            height,
        ));
    });

    let rss_after = rss_kib();
    let peak = peak_rss_kib();
    let optional = |value: Option<u64>| value.map_or("null".to_string(), |value| value.to_string());
    println!(
        "{{\"scenario\":\"{}\",\"label\":\"{}\",\"cached_layers\":false,\"frames\":{},\"width\":{},\"height\":{},\"calibration_ms\":{:.5},\
\"construct_ms\":{:.5},\"construct_min_ms\":{:.5},\"construct_max_ms\":{:.5},\"construct_norm\":{:.6},\"construct_allocs\":{:.2},\"construct_bytes\":{:.0},\
\"prepare_ms\":{:.5},\"prepare_min_ms\":{:.5},\"prepare_max_ms\":{:.5},\"prepare_norm\":{:.6},\"prepare_allocs\":{:.2},\"prepare_bytes\":{:.0},\
\"paint_ms\":{:.5},\"paint_min_ms\":{:.5},\"paint_max_ms\":{:.5},\"paint_norm\":{:.6},\"paint_allocs\":{:.2},\"paint_bytes\":{:.0},\
\"update_ms\":{:.5},\"update_min_ms\":{:.5},\"update_max_ms\":{:.5},\"update_norm\":{:.6},\"update_allocs\":{:.2},\"update_bytes\":{:.0},\
\"rss_before_kib\":{},\"rss_after_kib\":{},\"peak_rss_kib\":{}}}",
        scenario.name(),
        json_escape(label),
        iterations,
        width,
        height,
        calibration_ms,
        construct.median_ms,
        construct.min_ms,
        construct.max_ms,
        normalized(&construct),
        construct.allocs_per_op,
        construct.bytes_per_op,
        prepare.median_ms,
        prepare.min_ms,
        prepare.max_ms,
        normalized(&prepare),
        prepare.allocs_per_op,
        prepare.bytes_per_op,
        paint.median_ms,
        paint.min_ms,
        paint.max_ms,
        normalized(&paint),
        paint.allocs_per_op,
        paint.bytes_per_op,
        update.median_ms,
        update.min_ms,
        update.max_ms,
        normalized(&update),
        update.allocs_per_op,
        update.bytes_per_op,
        optional(rss_before),
        optional(rss_after),
        optional(peak),
    );
}
