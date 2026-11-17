//! Native vector pan/zoom without rebuilding authored paths (issue #19).
//!
//! ```sh
//! cargo run --example vector_pan_zoom            # headless demo (no display)
//! cargo run --example vector_pan_zoom -- --interactive
//! cargo run --example vector_pan_zoom -- --export work/pan-zoom.svg
//! cargo run --release --example vector_pan_zoom -- --bench
//! ```
//!
//! Interactive controls: drag with the primary button to pan, scroll to zoom
//! around the pointer. The status line reports the pointer mapped back into
//! drawing coordinates through `VectorView::viewport_to_document`.
//!
//! The default mode is headless on purpose: it exercises pan, cursor-anchored
//! zoom, inverse pointer mapping, repeated view changes, and export stability on
//! machines without a window server. `--bench` measures view-only updates,
//! preparation, and uncached painting; pixel-layer caching is disabled.

use cssimpler::{
    app::{App, Invalidation},
    core::{
        Color, LayoutBox, Node, PathCommand::*, VectorPaint, VectorPath, VectorPoint as P,
        VectorScene, VectorView, VectorViewBox,
    },
    renderer::{
        ButtonState, EngineEvent, FrameInfo, LayerCacheLimits, PointerButton, ScrollDelta,
        WindowConfig, render_to_buffer, set_layer_cache_limits,
    },
    style::parse_stylesheet,
};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    hint::black_box,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

/// Authored drawing size. The example keeps the CSS layout fixed so pointer
/// mapping is deterministic; a real editor would query the element's layout box.
const DRAWING_WIDTH: f32 = 800.0;
const DRAWING_HEIGHT: f32 = 600.0;

fn drawing_layout() -> LayoutBox {
    LayoutBox::new(0.0, 0.0, DRAWING_WIDTH, DRAWING_HEIGHT)
}

fn artwork(view_box: VectorViewBox, path_count: usize) -> VectorScene {
    let colors = [
        Color::rgb(200, 220, 240),
        Color::rgb(220, 235, 210),
        Color::rgb(245, 220, 220),
    ];
    let mut paths = Vec::with_capacity(path_count);
    for index in 0..path_count {
        let x = 20.0 + (index % 4) as f32 * (view_box.width / 4.0);
        let y = 20.0 + (index / 4) as f32 * (view_box.height / 3.0);
        let color = colors[index % colors.len()];
        paths.push(
            VectorPath::new(
                vec![
                    MoveTo(P::new(x, y + 120.0)),
                    CubicTo {
                        control1: P::new(x - 10.0, y + 20.0),
                        control2: P::new(x + 120.0, y - 10.0),
                        end: P::new(x + 170.0, y + 40.0),
                    },
                    QuadTo {
                        control: P::new(x + 190.0, y + 150.0),
                        end: P::new(x + 110.0, y + 135.0),
                    },
                    LineTo(P::new(x, y + 120.0)),
                    Close,
                ],
                VectorPaint {
                    fill: Some(color),
                    stroke: Some(Color::rgb(30, 60, 90)),
                    stroke_width: 2.0,
                },
            )
            .unwrap(),
        );
    }
    VectorScene::new(view_box, paths).unwrap()
}

fn document_artwork() -> VectorScene {
    artwork(
        VectorViewBox::new(0.0, 0.0, DRAWING_WIDTH, DRAWING_HEIGHT),
        6,
    )
}

#[derive(Clone, Copy)]
struct ViewStep {
    view: VectorView,
    document_anchor: P,
}

/// A bounded, allocation-free sequence of pan/zoom states. It never retains one
/// geometry copy (or raster) per step; each step is just a `VectorView`.
fn view_step(base: VectorView, step: usize) -> ViewStep {
    let view_box = VectorViewBox::new(0.0, 0.0, DRAWING_WIDTH, DRAWING_HEIGHT);
    let layout = drawing_layout();
    let pointer = P::new(
        120.0 + (step % 53) as f32 * 12.0,
        90.0 + (step % 41) as f32 * 11.0,
    );
    let mut view = base;
    if step % 3 == 0 {
        view = view
            .panned_by_viewport_delta(
                view_box,
                layout,
                if step % 2 == 0 { 3.0 } else { -2.0 },
                if step % 5 == 0 { 1.0 } else { -1.5 },
            )
            .expect("valid layout");
    } else if step % 3 == 1 {
        view = view
            .zoomed_about_viewport_point(
                view_box,
                layout,
                pointer,
                if step % 2 == 0 { 1.01 } else { 0.99 },
            )
            .expect("valid cursor zoom");
    } else {
        view = view
            .zoomed_about_viewport_point(view_box, layout, pointer, 1.04)
            .expect("valid cursor zoom");
    }
    let document_anchor = view
        .viewport_to_document(view_box, layout, pointer)
        .expect("valid layout");
    ViewStep {
        view,
        document_anchor,
    }
}

struct State {
    drawing: VectorScene,
    view: VectorView,
    pointer: P,
    drag_origin: Option<P>,
    frames: u64,
}

fn pointer_document(state: &State) -> Option<P> {
    state
        .view
        .viewport_to_document(state.drawing.view_box(), drawing_layout(), state.pointer)
}

fn update(state: &mut State, _: FrameInfo) -> Invalidation {
    state.frames = state.frames.wrapping_add(1);
    Invalidation::Clean
}

fn view(state: &State) -> Node {
    let document = pointer_document(state);
    let status = match document {
        Some(point) => format!(
            "zoom {:.3}  pointer ({:.1}, {:.1}) -> drawing ({:.2}, {:.2})",
            state.view.zoom(),
            state.pointer.x,
            state.pointer.y,
            point.x,
            point.y
        ),
        None => format!(
            "zoom {:.3}  pointer outside a mappable viewport",
            state.view.zoom()
        ),
    };
    let mut drawing = state.drawing.clone();
    drawing.set_view(Some(state.view));
    Node::element("div")
        .with_class("app")
        .with_children([
            Node::element("div")
                .with_class("viewport")
                .with_child(Node::vector(drawing).with_class("drawing").into())
                .into(),
            Node::element("div")
                .with_class("status")
                .with_child(Node::text(status))
                .into(),
        ])
        .into()
}

fn on_event(state: &mut State, event: &EngineEvent) -> bool {
    match event {
        EngineEvent::PointerMoved { position, .. } => {
            let previous = state.pointer;
            state.pointer = P::new(position.x, position.y);
            if let Some(origin) = state.drag_origin {
                let delta_x = state.pointer.x - previous.x;
                let delta_y = state.pointer.y - previous.y;
                if let Ok(view) = state.view.panned_by_viewport_delta(
                    state.drawing.view_box(),
                    drawing_layout(),
                    delta_x,
                    delta_y,
                ) {
                    state.view = view;
                }
                state.drag_origin = Some(origin);
            }
            true
        }
        EngineEvent::PointerButton {
            button: PointerButton::Primary,
            state: ButtonState::Pressed,
            ..
        } => {
            state.drag_origin = Some(state.pointer);
            true
        }
        EngineEvent::PointerButton {
            button: PointerButton::Primary,
            state: ButtonState::Released,
            ..
        } => {
            state.drag_origin = None;
            if let Some(document) = pointer_document(state) {
                println!(
                    "released at viewport ({:.1}, {:.1}) -> drawing ({:.2}, {:.2})",
                    state.pointer.x, state.pointer.y, document.x, document.y
                );
            }
            true
        }
        EngineEvent::PointerLeft => {
            state.drag_origin = None;
            true
        }
        EngineEvent::Scroll { delta, .. } => {
            let units = match delta {
                ScrollDelta::Lines { y, .. } => *y,
                ScrollDelta::Pixels { y, .. } => *y / 40.0,
            };
            let factor = 1.1_f32.powf(units);
            if let Ok(view) = state.view.zoomed_about_viewport_point(
                state.drawing.view_box(),
                drawing_layout(),
                state.pointer,
                factor,
            ) {
                state.view = view;
                true
            } else {
                false
            }
        }
        _ => false,
    }
}

fn run_interactive() -> anyhow::Result<()> {
    let state = State {
        drawing: document_artwork(),
        view: VectorView::identity(),
        pointer: P::new(DRAWING_WIDTH * 0.5, DRAWING_HEIGHT * 0.5),
        drag_origin: None,
        frames: 0,
    };
    let css = parse_stylesheet(
        "
        .app { background: #f8fafc; }
        .viewport {
            position: absolute;
            left: 0;
            top: 0;
            width: 800px;
            height: 600px;
            overflow: hidden;
            background: #ffffff;
        }
        .drawing { width: 800px; height: 600px; }
        .status {
            position: absolute;
            left: 12px;
            bottom: 12px;
            font-size: 13px;
            color: #334155;
        }
        ",
    )?;
    App::new(state, &css, update, view)
        .with_event_handler(on_event)
        .run(WindowConfig::new("Native vector pan/zoom", 800, 600))
        .map_err(Into::into)
}

fn export() -> anyhow::Result<()> {
    let scene = document_artwork();
    let view = VectorView::new(4.0, P::new(-120.0, -80.0)).unwrap();
    let mut viewed = scene.clone();
    viewed.set_view(Some(view));
    assert_eq!(
        viewed.to_svg(),
        scene.to_svg(),
        "export must ignore the viewport view"
    );
    let svg = scene.to_svg();
    let args = std::env::args().collect::<Vec<_>>();
    let path = args
        .windows(2)
        .find(|pair| pair[0] == "--export")
        .map(|pair| pair[1].clone())
        .unwrap_or_else(|| "pan-zoom.svg".to_string());
    std::fs::write(&path, &svg)?;
    println!(
        "wrote {path} ({} paths, view {:?} not baked into the export)",
        scene.paths().len(),
        view
    );
    Ok(())
}

/// Headless walkthrough of the focused pattern: authored geometry, a tiny view
/// state, pointer-anchored zoom, inverse mapping, and export stability.
fn run_demo() -> anyhow::Result<()> {
    let drawing = document_artwork();
    let view_box = drawing.view_box();
    let layout = drawing_layout();
    let geometry = drawing.paths()[0].geometry().clone();
    let commands_pointer = drawing.paths()[0].commands().as_ptr();
    let exported = drawing.to_svg();

    let pointer = P::new(420.0, 310.0);
    let mut view = VectorView::identity();
    let document_before = view
        .viewport_to_document(view_box, layout, pointer)
        .expect("valid viewport");
    println!(
        "pointer ({:.1}, {:.1}) starts over drawing ({:.2}, {:.2})",
        pointer.x, pointer.y, document_before.x, document_before.y
    );

    view = view
        .panned_by_viewport_delta(view_box, layout, 120.0, 45.0)
        .expect("valid pan");
    let document_after_pan = view
        .viewport_to_document(view_box, layout, pointer)
        .expect("valid viewport");
    println!(
        "after panning right/down by (120, 45), the same pointer is over drawing ({:.2}, {:.2})",
        document_after_pan.x, document_after_pan.y
    );

    view = view
        .zoomed_about_viewport_point(view_box, layout, pointer, 2.0)
        .expect("valid cursor zoom");
    let document_after_zoom = view
        .viewport_to_document(view_box, layout, pointer)
        .expect("valid viewport");
    let anchor_error = (document_after_zoom.x - document_after_pan.x)
        .abs()
        .max((document_after_zoom.y - document_after_pan.y).abs());
    println!(
        "zoomed to {:.3} around the pointer; anchored drawing point moved by {:.6}",
        view.zoom(),
        anchor_error
    );
    assert!(
        anchor_error < 1e-2,
        "cursor-anchored zoom must keep the drawing point under the pointer"
    );

    let mut scene = drawing.clone();
    let mut step = 0usize;
    for _ in 0..400 {
        let next = view_step(view, step);
        view = next.view;
        let _ = next.document_anchor;
        scene.set_view(Some(view));
        assert!(
            scene.paths()[0].geometry().shares_storage_with(&geometry),
            "view-only updates must keep the authored command buffer"
        );
        step += 1;
    }
    assert_eq!(scene.paths()[0].commands().as_ptr(), commands_pointer);
    assert_eq!(
        scene.to_svg(),
        exported,
        "view changes must not alter export"
    );
    println!(
        "400 pan/zoom steps: geometry identity {:?}, commands pointer unchanged, export {} bytes stable",
        geometry.identity(),
        exported.len()
    );
    println!(
        "view state is {} bytes ({} with the scene's Option); no display was needed for this demo",
        std::mem::size_of::<VectorView>(),
        std::mem::size_of::<Option<VectorView>>()
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Benchmark mode (release build; pixel-layer caching off).
// ---------------------------------------------------------------------------

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
    Sample {
        median_ms: times[times.len() / 2],
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
    // Allocate the timing vector before resetting counters so the harness's own
    // bookkeeping is not reported as per-operation work.
    let mut times = Vec::with_capacity(iterations);
    reset_counters();
    for _ in 0..iterations {
        let start = Instant::now();
        operation();
        times.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    let (allocs, bytes) = counter_values();
    sample(times, allocs, bytes, iterations)
}

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

fn run_bench() -> anyhow::Result<()> {
    // Measurement prerequisite: no pixel layers, explicit or promoted.
    set_layer_cache_limits(LayerCacheLimits {
        total_bytes: 0,
        layer_bytes: 0,
    });
    let (width, height) = (960usize, 640usize);
    let view_box = VectorViewBox::new(0.0, 0.0, width as f32, height as f32);
    let layout = LayoutBox::new(0.0, 0.0, width as f32, height as f32);
    let mut base = artwork(view_box, 12);
    let stylesheet = cssimpler::style::Stylesheet::default();
    let calibration_ms = calibrate();

    // Every step produces a fresh Copy view state from the previous one; no
    // geometry or raster is retained per step.
    let mut step = 0usize;
    let mut next_view = |view: &mut VectorView| {
        step += 1;
        if step % 2 == 0 {
            *view = view
                .panned_by_viewport_delta(view_box, layout, 2.0, -1.0)
                .unwrap();
        } else {
            *view = view
                .zoomed_about_viewport_point(
                    view_box,
                    layout,
                    P::new(480.0, 320.0),
                    if step % 4 == 1 { 1.01 } else { 0.99 },
                )
                .unwrap();
        }
        *view
    };

    // Warm the RSS reader's own allocations so later deltas reflect the
    // measured loop rather than the first `ps` invocation.
    let _ = rss_kib();
    let _ = rss_kib();
    let rss_start = rss_kib();

    // Pure view-state change: must be allocation-free and O(1). RSS is sampled
    // after two different operation counts to show a plateau.
    let mut view_state = VectorView::identity();
    for _ in 0..1000 {
        let current = next_view(&mut view_state);
        base.set_view(Some(current));
        black_box(base.view());
    }
    let rss_after_1000_view_updates = rss_kib();
    let view_only = measure(1000, 5000, || {
        let current = next_view(&mut view_state);
        base.set_view(Some(current));
        black_box(base.view());
    });
    let rss_after_6000_view_updates = rss_kib();
    for _ in 0..5000 {
        let current = next_view(&mut view_state);
        base.set_view(Some(current));
        black_box(base.view());
    }
    let rss_after_11000_view_updates = rss_kib();

    // The per-frame rebuild path: clone the scene (geometry handles only), set
    // the new view, and rebuild the DOM node.
    let mut rebuild_view = VectorView::identity();
    let rebuild = measure(5, 400, || {
        let current = next_view(&mut rebuild_view);
        let mut scene = base.clone();
        scene.set_view(Some(current));
        black_box::<Node>(Node::vector(scene).into());
    });

    // Preparation: the same rebuild plus style/layout resolution.
    let mut prepare_view = VectorView::identity();
    let prepare = measure(5, 200, || {
        let current = next_view(&mut prepare_view);
        let mut scene = base.clone();
        scene.set_view(Some(current));
        let node: Node = Node::vector(scene).into();
        black_box(cssimpler::style::build_render_tree_in_viewport(
            &node,
            &stylesheet,
            width,
            height,
        ));
    });

    // Uncached full paint of a prepared scene (fixed view; paint work does not
    // depend on how many earlier view changes happened).
    let paint_scene = base.clone().with_view(VectorView::identity());
    let node: Node = Node::vector(paint_scene).into();
    let tree = cssimpler::style::build_render_tree_in_viewport(&node, &stylesheet, width, height);
    assert!(
        !tree.style.cache_layer,
        "pixel-layer caching must be off for this harness"
    );
    let render_scene = vec![tree];
    let mut pixels = vec![0u32; width * height];
    let paint = measure(2, 8, || {
        render_to_buffer(&render_scene, &mut pixels, width, height, Color::WHITE);
        black_box(&pixels);
    });

    // Repeated full pipeline with a fresh view on every frame. RSS is sampled
    // after 100 and after 1000 frames to show retained memory plateaus instead
    // of growing with the number of pan/zoom operations.
    let mut pipeline_scene = base.clone();
    let mut pipeline_view = VectorView::identity();
    let mut pipeline_pixels = vec![0u32; width * height];
    let mut run_frame = |scene: &mut VectorScene, view: &mut VectorView, pixels: &mut [u32]| {
        let current = next_view(view);
        scene.set_view(Some(current));
        let node: Node = Node::vector(scene.clone()).into();
        let tree =
            cssimpler::style::build_render_tree_in_viewport(&node, &stylesheet, width, height);
        render_to_buffer(&[tree], pixels, width, height, Color::WHITE);
    };
    for _ in 0..20 {
        run_frame(
            &mut pipeline_scene,
            &mut pipeline_view,
            &mut pipeline_pixels,
        );
    }
    for _ in 0..80 {
        run_frame(
            &mut pipeline_scene,
            &mut pipeline_view,
            &mut pipeline_pixels,
        );
    }
    let rss_after_100 = rss_kib();
    for _ in 0..900 {
        run_frame(
            &mut pipeline_scene,
            &mut pipeline_view,
            &mut pipeline_pixels,
        );
    }
    let rss_after_1000 = rss_kib();
    for _ in 0..1000 {
        run_frame(
            &mut pipeline_scene,
            &mut pipeline_view,
            &mut pipeline_pixels,
        );
    }
    let rss_after_2000 = rss_kib();
    let peak = peak_rss_kib();
    let optional = |value: Option<u64>| value.map_or("null".to_string(), |value| value.to_string());
    let rss_delta = |a: Option<u64>, b: Option<u64>| match (a, b) {
        (Some(a), Some(b)) => (b as i64 - a as i64).to_string(),
        _ => "null".to_string(),
    };
    let normalized = |sample: &Sample| sample.median_ms / calibration_ms;
    let export_stable = {
        let mut viewed = base.clone();
        viewed.set_view(Some(VectorView::new(3.5, P::new(-200.0, -100.0)).unwrap()));
        viewed.to_svg() == base.to_svg()
            && viewed.paths()[0]
                .geometry()
                .shares_storage_with(base.paths()[0].geometry())
    };
    println!(
        "{{\"bench\":\"vector-pan-zoom\",\"cached_layers\":false,\"paths\":12,\"width\":{},\"height\":{},\
\"calibration_ms\":{:.5},\"view_state_bytes\":{},\"scene_state_bytes\":{},\
\"view_only_ms\":{:.5},\"view_only_min_ms\":{:.5},\"view_only_max_ms\":{:.5},\"view_only_norm\":{:.6},\"view_only_allocs\":{:.2},\"view_only_bytes\":{:.0},\
\"rebuild_ms\":{:.5},\"rebuild_min_ms\":{:.5},\"rebuild_max_ms\":{:.5},\"rebuild_norm\":{:.6},\"rebuild_allocs\":{:.2},\"rebuild_bytes\":{:.0},\
\"prepare_ms\":{:.5},\"prepare_min_ms\":{:.5},\"prepare_max_ms\":{:.5},\"prepare_norm\":{:.6},\"prepare_allocs\":{:.2},\"prepare_bytes\":{:.0},\
\"paint_ms\":{:.5},\"paint_min_ms\":{:.5},\"paint_max_ms\":{:.5},\"paint_norm\":{:.6},\"paint_allocs\":{:.2},\"paint_bytes\":{:.0},\
\"rss_start_kib\":{},\"rss_after_1000_view_updates_kib\":{},\"rss_after_6000_view_updates_kib\":{},\"rss_after_11000_view_updates_kib\":{},\"rss_after_100_pipeline_frames_kib\":{},\"rss_after_1000_pipeline_frames_kib\":{},\"rss_after_2000_pipeline_frames_kib\":{},\"peak_rss_kib\":{},\
\"retained_view_update_1000_to_6000_kib\":{},\"retained_view_update_6000_to_11000_kib\":{},\"retained_pipeline_100_to_1000_kib\":{},\"retained_pipeline_1000_to_2000_kib\":{},\"export_stable\":{}}}",
        width,
        height,
        calibration_ms,
        std::mem::size_of::<VectorView>(),
        std::mem::size_of::<VectorScene>(),
        view_only.median_ms,
        view_only.min_ms,
        view_only.max_ms,
        normalized(&view_only),
        view_only.allocs_per_op,
        view_only.bytes_per_op,
        rebuild.median_ms,
        rebuild.min_ms,
        rebuild.max_ms,
        normalized(&rebuild),
        rebuild.allocs_per_op,
        rebuild.bytes_per_op,
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
        optional(rss_start),
        optional(rss_after_1000_view_updates),
        optional(rss_after_6000_view_updates),
        optional(rss_after_11000_view_updates),
        optional(rss_after_100),
        optional(rss_after_1000),
        optional(rss_after_2000),
        optional(peak),
        rss_delta(rss_after_1000_view_updates, rss_after_6000_view_updates),
        rss_delta(rss_after_6000_view_updates, rss_after_11000_view_updates),
        rss_delta(rss_after_100, rss_after_1000),
        rss_delta(rss_after_1000, rss_after_2000),
        export_stable,
    );
    set_layer_cache_limits(LayerCacheLimits::default());
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| arg == "--bench") {
        return run_bench();
    }
    if args.iter().any(|arg| arg == "--interactive") {
        return run_interactive();
    }
    if args.iter().any(|arg| arg == "--export") {
        return export();
    }
    run_demo()
}
