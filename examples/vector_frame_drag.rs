//! Native vector drag with opt-in frame-aligned updates.
//!
//! The default headless replay compares frame-aligned drag updates with the
//! raw per-sample refresh path and prints bounded-state measurements:
//!
//! ```sh
//! cargo run --example vector_frame_drag
//! ```
//!
//! Pass `--window` to open the interactive native vector demo. In the window,
//! press and drag anywhere on the board: the rubber-band overlay and handle are
//! rebuilt once per displayed frame, while the cached background drawing stays
//! unchanged. The example also prints a summary when the window closes.

use std::cell::Cell;
use std::time::Instant;

use cssimpler::{
    app::{App, Invalidation},
    core::{
        Color, ElementPath, Node, PathCommand::*, VectorPaint, VectorPath, VectorPoint as P,
        VectorScene, VectorViewBox,
    },
    frame_drag::{
        DragCancelReason, DragTransition, FrameAlignedDrag, FrameDragUpdate, MAX_ACTIVE_POINTERS,
        MAX_PENDING_TRANSITIONS, MOUSE_POINTER, PointerId,
    },
    renderer::{
        ButtonState, EngineEvent, FrameInfo, LayerCacheLimits, PointerButton, PointerPosition,
        SceneProvider, WindowConfig, layer_cache_stats, render_to_buffer, set_layer_cache_limits,
    },
    style::parse_stylesheet,
};

const WIDTH: f32 = 880.0;
const HEIGHT: f32 = 560.0;
const FRAME_MS: u64 = 16;
const MOVES: usize = 240;
const MOVES_PER_FRAME: usize = 4;

fn view_box() -> VectorViewBox {
    VectorViewBox::new(0.0, 0.0, WIDTH, HEIGHT)
}

fn line_paint() -> VectorPaint {
    VectorPaint {
        fill: None,
        stroke: Some(Color::rgb(37, 99, 235)),
        stroke_width: 3.0,
    }
}

fn background_scene() -> VectorScene {
    let mut commands = Vec::new();
    let mut x = 0.0;
    while x <= WIDTH {
        commands.push(MoveTo(P::new(x, 0.0)));
        commands.push(LineTo(P::new(x, HEIGHT)));
        x += 40.0;
    }
    let mut y = 0.0;
    while y <= HEIGHT {
        commands.push(MoveTo(P::new(0.0, y)));
        commands.push(LineTo(P::new(WIDTH, y)));
        y += 40.0;
    }
    let grid = VectorPath::new(
        commands,
        VectorPaint {
            fill: None,
            stroke: Some(Color::rgba(148, 163, 184, 70)),
            stroke_width: 1.0,
        },
    )
    .expect("grid path");

    let art = VectorPath::new(
        vec![
            MoveTo(P::new(80.0, HEIGHT - 80.0)),
            CubicTo {
                control1: P::new(140.0, 60.0),
                control2: P::new(420.0, 80.0),
                end: P::new(620.0, 240.0),
            },
            QuadTo {
                control: P::new(760.0, 360.0),
                end: P::new(800.0, HEIGHT - 120.0),
            },
            LineTo(P::new(80.0, HEIGHT - 80.0)),
            Close,
        ],
        VectorPaint {
            fill: Some(Color::rgba(191, 219, 254, 170)),
            stroke: Some(Color::rgb(30, 64, 175)),
            stroke_width: 2.0,
        },
    )
    .expect("art path");

    VectorScene::new(view_box(), vec![grid, art]).expect("background scene")
}

fn overlay_scene(segments: &[[P; 2]], start: Option<P>, current: P) -> VectorScene {
    let mut paths = Vec::new();
    if !segments.is_empty() {
        let mut commands = Vec::with_capacity(segments.len() * 4);
        for segment in segments {
            commands.push(MoveTo(segment[0]));
            commands.push(LineTo(segment[1]));
        }
        paths.push(VectorPath::new(commands, line_paint()).expect("segment path"));
    }
    if let Some(start) = start {
        paths.push(
            VectorPath::new(vec![MoveTo(start), LineTo(current)], line_paint())
                .expect("active path"),
        );
    }
    let half = 5.0;
    paths.push(
        VectorPath::new(
            vec![
                MoveTo(P::new(current.x - half, current.y - half)),
                LineTo(P::new(current.x + half, current.y - half)),
                LineTo(P::new(current.x + half, current.y + half)),
                LineTo(P::new(current.x - half, current.y + half)),
                Close,
            ],
            VectorPaint {
                fill: Some(Color::rgb(239, 68, 68)),
                stroke: Some(Color::rgb(127, 29, 29)),
                stroke_width: 1.0,
            },
        )
        .expect("handle path"),
    );
    VectorScene::new(view_box(), paths).expect("overlay scene")
}

struct DragState {
    drag: FrameAlignedDrag,
    background: VectorScene,
    latest: Option<FrameDragUpdate>,
    segments: Vec<[P; 2]>,
    start: Option<P>,
    current: P,
    raw_samples: usize,
    drag_frames: usize,
    view_builds: Cell<usize>,
    geometry_builds: Cell<usize>,
    cancelled: usize,
}

impl DragState {
    fn new() -> Self {
        Self {
            drag: FrameAlignedDrag::new(),
            background: background_scene(),
            latest: None,
            segments: Vec::new(),
            start: None,
            current: P::new(WIDTH * 0.5, HEIGHT * 0.5),
            raw_samples: 0,
            drag_frames: 0,
            view_builds: Cell::new(0),
            geometry_builds: Cell::new(0),
            cancelled: 0,
        }
    }
}

fn raw_sample_count(event: &EngineEvent) -> usize {
    usize::from(matches!(event, EngineEvent::PointerMoved { .. }))
}

/// Raw input path: every sample still reaches the application. The drag state
/// only coalesces what the *next frame* sees.
fn on_event(state: &mut DragState, event: &EngineEvent) -> bool {
    state.raw_samples += raw_sample_count(event);
    if let EngineEvent::PointerButton {
        button: PointerButton::Primary,
        state: ButtonState::Pressed,
        ..
    } = event
    {
        // Keep target ownership explicitly for the demo board.
        state
            .drag
            .set_capture(MOUSE_POINTER, Some(ElementPath::root(0)));
    }
    state.drag.feed(event);
    // Motion is not consumed here: the frame-aligned update below owns the
    // rebuild, so returning `true` (structure invalidation) would defeat it.
    false
}

fn apply_update(state: &mut DragState, update: FrameDragUpdate) {
    for transition in &update.transitions {
        match transition {
            DragTransition::Pressed { position, .. } => {
                state.start = Some(P::new(position.x, position.y));
                state.current = P::new(position.x, position.y);
            }
            DragTransition::Released { position, .. } => {
                state.current = P::new(position.x, position.y);
                if let Some(start) = state.start.take() {
                    state.segments.push([start, state.current]);
                }
            }
            DragTransition::Cancelled {
                reason, position, ..
            } => {
                if matches!(reason, DragCancelReason::Overflow) {
                    state.segments.clear();
                }
                state.current = P::new(position.x, position.y);
                state.start = None;
                state.cancelled += 1;
            }
        }
    }
    if let Some(pointer) = update.pointers.iter().find(|pointer| pointer.active) {
        state.current = P::new(pointer.position.x, pointer.position.y);
    }
    if update.overflowed {
        state.segments.clear();
        state.start = None;
    }
    state.latest = Some(update);
}

fn update(state: &mut DragState, frame: FrameInfo) -> Invalidation {
    let Some(drag_update) = state.drag.begin_frame(frame.frame_index) else {
        return Invalidation::Clean;
    };
    state.drag_frames += 1;
    apply_update(state, drag_update);
    Invalidation::Paint
}

fn view(state: &DragState) -> Node {
    state.view_builds.set(state.view_builds.get() + 1);
    state.geometry_builds.set(state.geometry_builds.get() + 1);
    Node::element("div")
        .with_id("board")
        .with_children([
            Node::vector(state.background.clone())
                .with_class("background")
                .with_cached_layer(true)
                .into(),
            Node::vector(overlay_scene(&state.segments, state.start, state.current))
                .with_class("overlay")
                .into(),
        ])
        .into()
}

const STYLESHEET: &str = r#"
#board { position: relative; width: 880px; height: 560px; }
.background, .overlay {
    position: absolute;
    left: 0;
    top: 0;
    width: 880px;
    height: 560px;
}
.overlay { z-index: 1; }
"#;

fn frame(index: u64) -> FrameInfo {
    FrameInfo {
        frame_index: index,
        delta: std::time::Duration::from_millis(FRAME_MS),
    }
}

fn moved(x: f32, y: f32) -> EngineEvent {
    EngineEvent::PointerMoved {
        position: PointerPosition { x, y },
        modifiers: Default::default(),
    }
}

fn primary(state: ButtonState) -> EngineEvent {
    EngineEvent::PointerButton {
        button: PointerButton::Primary,
        state,
        modifiers: Default::default(),
    }
}

struct ReplayResult {
    display_frames: usize,
    raw_moves: usize,
    drag_frames: usize,
    view_builds: usize,
    geometry_builds: usize,
    elapsed_ms: f64,
    frame_avg_us: f64,
    frame_max_us: f64,
    final_endpoint: (f32, f32),
}

/// Run one displayed frame and record update+view+scene time. This is the
/// application-side latency from "input is available" to "geometry is rebuilt".
fn paint<P: SceneProvider>(provider: &mut P, info: FrameInfo, timing: &mut (f64, f64, u64)) {
    let start = Instant::now();
    provider.update(info);
    let _ = provider.scene();
    let micros = start.elapsed().as_secs_f64() * 1_000_000.0;
    timing.0 += micros;
    timing.1 = timing.1.max(micros);
    timing.2 += 1;
}

/// Fixed input replay with the same 243 pointer samples in both modes.
///
/// Frame-aligned mode paints once per group of `MOVES_PER_FRAME` samples plus
/// the release. Raw mode paints once after every sample, which is what the
/// existing immediate pointer-redraw policy does.
fn replay(frame_aligned: bool) -> ReplayResult {
    let stylesheet = parse_stylesheet(STYLESHEET).expect("stylesheet");
    let mut app = App::new(DragState::new(), &stylesheet, update, view)
        .with_event_handler(on_event)
        .with_frame_aligned_input(frame_aligned);
    // Initial build, excluded from the measured deltas.
    app.frame(frame(0));
    let mut timing = (0.0_f64, 0.0_f64, 0_u64);
    let views_before = app.state().view_builds.get();
    let geometry_before = app.state().geometry_builds.get();
    let mut raw_moves = 0;
    let mut display_frames = 0;
    let started = Instant::now();

    let mut next_frame = 1_u64;

    app.handle_engine_event(&moved(10.0, 10.0));
    raw_moves += 1;
    app.handle_engine_event(&primary(ButtonState::Pressed));
    if !frame_aligned {
        display_frames += 1;
        paint(&mut app, frame(next_frame), &mut timing);
        next_frame += 1;
    }

    for index in 0..MOVES {
        let x = 13.0 + index as f32 * 3.0;
        let y = 10.0 + (index as f32 * 1.7).sin() * 120.0;
        app.handle_engine_event(&moved(x, y));
        raw_moves += 1;
        if !frame_aligned {
            display_frames += 1;
            paint(&mut app, frame(next_frame), &mut timing);
            next_frame += 1;
        } else if (index + 1) % MOVES_PER_FRAME == 0 {
            display_frames += 1;
            paint(&mut app, frame(next_frame), &mut timing);
            next_frame += 1;
        }
    }
    app.handle_engine_event(&primary(ButtonState::Released));
    display_frames += 1;
    paint(&mut app, frame(next_frame), &mut timing);

    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    let state = app.state();
    ReplayResult {
        display_frames,
        raw_moves,
        drag_frames: state.drag_frames,
        view_builds: state.view_builds.get() - views_before,
        geometry_builds: state.geometry_builds.get() - geometry_before,
        elapsed_ms,
        frame_avg_us: timing.0 / timing.2.max(1) as f64,
        frame_max_us: timing.1,
        final_endpoint: (state.current.x, state.current.y),
    }
}

/// Boundary evidence: feed far more motion samples than any real device can
/// produce between frames and show that retained state stays fixed.
fn bounded_state_probe() {
    let mut drag = FrameAlignedDrag::new();
    drag.feed(&moved(1.0, 1.0));
    drag.feed(&primary(ButtonState::Pressed));
    let samples = 1_000_000;
    for index in 0..samples {
        let value = (index % 10_000) as f32;
        drag.feed(&moved(value, value));
    }
    let stored = drag.tracked_pointer_count();
    let pending = drag.pending_transition_count();
    let frame = drag
        .begin_frame(1)
        .expect("latest sample is still published");
    let pointer = frame.pointer(MOUSE_POINTER).expect("pointer");
    println!(
        "bounded state: {samples} motion samples -> tracked_pointers={stored} (max {MAX_ACTIVE_POINTERS}), \
pending_transitions={pending} (max {MAX_PENDING_TRANSITIONS}), published_position=({:.1}, {:.1}), \
FrameAlignedDrag={} bytes",
        pointer.position.x,
        pointer.position.y,
        std::mem::size_of::<FrameAlignedDrag>(),
    );
    // Multiple independent pointers are bounded too.
    for id in 0..(MAX_ACTIVE_POINTERS as u64 + 4) {
        drag.feed_pointer_at(
            PointerId::new(id),
            PointerPosition {
                x: id as f32,
                y: 0.0,
            },
            &primary(ButtonState::Pressed),
        );
    }
    let overflow = drag.begin_frame(2).expect("overflow frame");
    println!(
        "bounded state: {} pointers -> overflowed={} transitions={} (max {MAX_PENDING_TRANSITIONS})",
        MAX_ACTIVE_POINTERS as u64 + 4,
        overflow.overflowed,
        overflow.transitions.len(),
    );
}

fn process_rss_kib() -> Option<u64> {
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    text.trim().parse().ok()
}

fn headless_report() {
    println!(
        "cssimpler frame-aligned drag replay ({MOVES} moves, {MOVES_PER_FRAME} samples/frame)"
    );
    let raw = replay(false);
    let aligned = replay(true);
    println!(
        "raw-per-sample:  moves={} display_frames={} view_builds={} geometry_builds={} replay={:.2} ms update+view avg={:.1} us max={:.1} us",
        raw.raw_moves,
        raw.display_frames,
        raw.view_builds,
        raw.geometry_builds,
        raw.elapsed_ms,
        raw.frame_avg_us,
        raw.frame_max_us
    );
    println!(
        "frame-aligned:   moves={} display_frames={} drag_frames={} view_builds={} geometry_builds={} replay={:.2} ms update+view avg={:.1} us max={:.1} us",
        aligned.raw_moves,
        aligned.display_frames,
        aligned.drag_frames,
        aligned.view_builds,
        aligned.geometry_builds,
        aligned.elapsed_ms,
        aligned.frame_avg_us,
        aligned.frame_max_us
    );
    println!(
        "view rebuilds per move: raw={:.2} frame-aligned={:.2}; final endpoint = ({:.1}, {:.1})",
        raw.view_builds as f64 / raw.raw_moves as f64,
        aligned.view_builds as f64 / aligned.raw_moves as f64,
        aligned.final_endpoint.0,
        aligned.final_endpoint.1,
    );
    assert_eq!(aligned.drag_frames, aligned.display_frames);
    assert!(aligned.view_builds <= aligned.display_frames + 1);
    assert!(aligned.geometry_builds <= aligned.display_frames + 1);
    let rss_before = process_rss_kib();
    bounded_state_probe();
    let rss_after = process_rss_kib();
    if let (Some(before), Some(after)) = (rss_before, rss_after) {
        println!("process RSS before/after 1M-sample probe: {before} KiB / {after} KiB");
    }
    cached_background_probe();
    println!(
        "note: scheduling cadence is covered by renderer FrameInputCadence unit tests; \
this replay calls one paint per simulated frame."
    );
}

/// Paint the cached background under a moving overlay three times. The explicit
/// layer must be rasterized once; later drag frames reuse it because the
/// background subtree did not change.
fn cached_background_probe() {
    // The default explicit-layer budget is 4 MiB; this 880 x 560 board retains
    // 16 bytes per pixel, so raise the budget and show the layer being reused.
    set_layer_cache_limits(LayerCacheLimits {
        total_bytes: 32 * 1024 * 1024,
        layer_bytes: 16 * 1024 * 1024,
    });
    let stylesheet = parse_stylesheet(STYLESHEET).expect("stylesheet");
    let mut app = App::new(DragState::new(), &stylesheet, update, view)
        .with_event_handler(on_event)
        .with_frame_aligned_input(true);
    app.frame(frame(0));
    let mut buffer = vec![0_u32; WIDTH as usize * HEIGHT as usize];
    app.handle_engine_event(&moved(40.0, 40.0));
    app.handle_engine_event(&primary(ButtonState::Pressed));
    let mut builds = Vec::new();
    for step in 0..3_u64 {
        app.handle_engine_event(&moved(60.0 + step as f32 * 80.0, 80.0 + step as f32 * 40.0));
        let scene = app.frame(frame(step + 1));
        render_to_buffer(
            &scene,
            &mut buffer,
            WIDTH as usize,
            HEIGHT as usize,
            Color::rgb(255, 255, 255),
        );
        builds.push(layer_cache_stats().builds);
    }
    let stats = layer_cache_stats();
    println!(
        "cached background layer builds across 3 drag paints: {builds:?} (cache entries={}, bytes={})",
        stats.entries, stats.bytes
    );
    assert!(
        builds[2] == builds[0],
        "unchanged background must not rebuild its cached layer for every drag frame"
    );
}

fn run_window() -> anyhow::Result<()> {
    // The 880 x 560 background retains 16 bytes per pixel; raise the explicit
    // layer budget so the demo actually reuses the cached background raster.
    set_layer_cache_limits(LayerCacheLimits {
        total_bytes: 32 * 1024 * 1024,
        layer_bytes: 16 * 1024 * 1024,
    });
    let stylesheet = parse_stylesheet(STYLESHEET)?;
    App::new(DragState::new(), &stylesheet, update, view)
        .with_event_handler(on_event)
        .with_frame_aligned_input(true)
        .run(WindowConfig::new(
            "Frame-aligned vector drag",
            WIDTH as usize,
            HEIGHT as usize,
        ))
        .map_err(Into::into)
}

fn main() -> anyhow::Result<()> {
    if std::env::args().any(|arg| arg == "--window") {
        return run_window();
    }
    headless_report();
    Ok(())
}
