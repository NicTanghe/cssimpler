//! Focused headless tests for issue #20: opt-in frame-aligned drag updates.
//!
//! Everything here runs without a window. The scheduling contract is exercised
//! through the renderer's `FrameInputCadence`; the application side is driven by
//! synthetic `EngineEvent`s and explicit `App::frame` calls, one per simulated
//! displayed frame.

use std::cell::Cell;
use std::time::{Duration, Instant};

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
        ButtonState, EngineEvent, FrameInfo, FrameInputCadence, InputRedrawDecision,
        InputRedrawPolicy, KeyIdentity, KeyboardEvent, KeyboardModifiers, PointerButton,
        PointerPosition, SceneProvider, ScrollDelta, TextInputEvent,
    },
    style::{Stylesheet, parse_stylesheet},
};

const FRAME_TIME: Duration = Duration::from_millis(16);

fn frame(index: u64) -> FrameInfo {
    FrameInfo {
        frame_index: index,
        delta: FRAME_TIME,
    }
}

fn moved(x: f32, y: f32) -> EngineEvent {
    EngineEvent::PointerMoved {
        position: PointerPosition { x, y },
        modifiers: KeyboardModifiers::default(),
    }
}

fn primary(state: ButtonState) -> EngineEvent {
    EngineEvent::PointerButton {
        button: PointerButton::Primary,
        state,
        modifiers: KeyboardModifiers::default(),
    }
}

fn scroll(delta: ScrollDelta) -> EngineEvent {
    EngineEvent::Scroll {
        delta,
        modifiers: KeyboardModifiers::default(),
    }
}

struct TestState {
    drag: FrameAlignedDrag,
    latest: Option<FrameDragUpdate>,
    raw_events: usize,
    raw_moves: usize,
    updated_frames: usize,
    cancelled: usize,
    endpoint: (f32, f32),
    view_builds: Cell<usize>,
    geometry_builds: Cell<usize>,
}

impl TestState {
    fn new() -> Self {
        Self {
            drag: FrameAlignedDrag::new(),
            latest: None,
            raw_events: 0,
            raw_moves: 0,
            updated_frames: 0,
            cancelled: 0,
            endpoint: (0.0, 0.0),
            view_builds: Cell::new(0),
            geometry_builds: Cell::new(0),
        }
    }

    fn reset_counters(&self) {
        self.view_builds.set(0);
        self.geometry_builds.set(0);
    }
}

/// Raw path preserved: every sample reaches this handler, which does not consume
/// motion. The frame update owns the rebuild.
fn on_event(state: &mut TestState, event: &EngineEvent) -> bool {
    state.raw_events += 1;
    if matches!(event, EngineEvent::PointerMoved { .. }) {
        state.raw_moves += 1;
    }
    state.drag.feed(event);
    false
}

/// Existing per-sample consumer: returning `true` keeps the historical
/// structure-invalidation behavior for every move.
fn on_event_consuming_moves(state: &mut TestState, event: &EngineEvent) -> bool {
    state.raw_events += 1;
    if matches!(event, EngineEvent::PointerMoved { .. }) {
        state.raw_moves += 1;
    }
    state.drag.feed(event);
    matches!(event, EngineEvent::PointerMoved { .. })
}

fn update(state: &mut TestState, frame: FrameInfo) -> Invalidation {
    let Some(drag_update) = state.drag.begin_frame(frame.frame_index) else {
        return Invalidation::Clean;
    };
    state.updated_frames += 1;
    for transition in &drag_update.transitions {
        match transition {
            DragTransition::Released { position, .. } => {
                state.endpoint = (position.x, position.y);
            }
            DragTransition::Cancelled { .. } => state.cancelled += 1,
            DragTransition::Pressed { position, .. } => {
                state.endpoint = (position.x, position.y);
            }
        }
    }
    if let Some(pointer) = drag_update.pointers.iter().find(|pointer| pointer.active) {
        state.endpoint = (pointer.position.x, pointer.position.y);
    }
    state.latest = Some(drag_update);
    Invalidation::Paint
}

fn scene(endpoint: (f32, f32)) -> VectorScene {
    let path = VectorPath::new(
        vec![
            MoveTo(P::new(10.0, 10.0)),
            LineTo(P::new(endpoint.0, endpoint.1)),
        ],
        VectorPaint {
            fill: None,
            stroke: Some(Color::rgb(37, 99, 235)),
            stroke_width: 2.0,
        },
    )
    .expect("path");
    VectorScene::new(VectorViewBox::new(0.0, 0.0, 400.0, 300.0), vec![path]).expect("scene")
}

fn view(state: &TestState) -> Node {
    state.view_builds.set(state.view_builds.get() + 1);
    state.geometry_builds.set(state.geometry_builds.get() + 1);
    Node::element("div")
        .with_id("board")
        .with_children([Node::vector(scene(state.endpoint))
            .with_id("drawing")
            .into()])
        .into()
}

const STYLESHEET: &str = "#board { position: relative; width: 400px; height: 300px; } #drawing { width: 400px; height: 300px; }";

type TestApp<'a> =
    App<'a, TestState, fn(&mut TestState, FrameInfo) -> Invalidation, fn(&TestState) -> Node>;

fn build_app<'a>(
    stylesheet: &'a Stylesheet,
    frame_aligned: bool,
    handler: fn(&mut TestState, &EngineEvent) -> bool,
) -> TestApp<'a> {
    App::new(
        TestState::new(),
        stylesheet,
        update as fn(&mut TestState, FrameInfo) -> Invalidation,
        view as fn(&TestState) -> Node,
    )
    .with_event_handler(handler)
    .with_frame_aligned_input(frame_aligned)
}

#[test]
fn frame_aligned_app_rebuilds_geometry_once_per_frame_not_per_sample() {
    let stylesheet = parse_stylesheet(STYLESHEET).expect("stylesheet");
    let mut app = build_app(&stylesheet, true, on_event);
    // Initial build is not part of the measured drag.
    app.frame(frame(0));
    app.state().reset_counters();

    app.handle_engine_event(&moved(10.0, 10.0));
    app.handle_engine_event(&primary(ButtonState::Pressed));
    let mut last = (0.0, 0.0);
    for index in 1..=10_u64 {
        for sample in 0..5 {
            let x = 20.0 + sample as f32 + index as f32 * 5.0;
            let y = 30.0 + index as f32 * 2.0;
            last = (x, y);
            app.handle_engine_event(&moved(x, y));
        }
        app.frame(frame(index));
    }
    app.handle_engine_event(&primary(ButtonState::Released));
    app.frame(frame(11));

    let state = app.state();
    assert_eq!(state.raw_moves, 51, "every raw sample must reach the app");
    assert_eq!(
        state.updated_frames, 11,
        "one drag update per displayed frame"
    );
    assert_eq!(
        state.geometry_builds.get(),
        11,
        "geometry rebuilds once per frame, never per sample"
    );
    assert_eq!(state.view_builds.get(), 11);
    assert_eq!(
        state.endpoint, last,
        "the latest sample is the one consumed"
    );
    // Follow-up rebuilds inside one displayed frame cannot consume it again.
    let before = state.view_builds.get();
    app.frame(frame(11));
    assert_eq!(app.state().view_builds.get(), before);
}

#[test]
fn consuming_move_handlers_keep_per_sample_invalidation() {
    let stylesheet = parse_stylesheet(STYLESHEET).expect("stylesheet");
    let mut app = build_app(&stylesheet, true, on_event_consuming_moves);
    app.frame(frame(0));
    app.state().reset_counters();

    app.handle_engine_event(&moved(10.0, 10.0));
    app.handle_engine_event(&primary(ButtonState::Pressed));
    for index in 1..=12_u64 {
        let (x, y) = (index as f32 * 4.0, index as f32);
        app.handle_engine_event(&moved(x, y));
        // `handle_engine_event` returned true, so the next frame rebuilds.
        assert!(app.handle_engine_event(&moved(x + 1.0, y)));
        app.frame(frame(index));
    }
    let state = app.state();
    // Per-sample invalidation is preserved: the view rebuild count tracks the
    // consuming events, not the number of displayed frames.
    assert_eq!(state.view_builds.get(), 12);
    assert!(state.raw_moves >= 24);
}

#[test]
fn move_plus_release_before_a_paint_is_flushed_through_the_app() {
    let stylesheet = parse_stylesheet(STYLESHEET).expect("stylesheet");
    let mut app = build_app(&stylesheet, true, on_event);
    app.frame(frame(0));
    app.state().reset_counters();

    app.handle_engine_event(&moved(10.0, 10.0));
    app.handle_engine_event(&primary(ButtonState::Pressed));
    app.handle_engine_event(&moved(80.0, 90.0));
    app.handle_engine_event(&moved(120.0, 140.0));
    app.handle_engine_event(&primary(ButtonState::Released));
    app.frame(frame(1));

    let update = app.state().latest.as_ref().expect("frame update");
    assert_eq!(update.pointers.len(), 1);
    assert!(!update.pointers[0].active);
    assert_eq!(
        update.pointers[0].position,
        PointerPosition { x: 120.0, y: 140.0 }
    );
    assert_eq!(update.transitions.len(), 2);
    assert!(matches!(
        update.transitions[0],
        DragTransition::Pressed { .. }
    ));
    assert!(matches!(
        update.transitions[1],
        DragTransition::Released {
            position: PointerPosition { x: 120.0, y: 140.0 },
            ..
        }
    ));
    assert_eq!(app.state().endpoint, (120.0, 140.0));
}

#[test]
fn focus_loss_cancels_the_pending_drag_in_the_next_frame() {
    let stylesheet = parse_stylesheet(STYLESHEET).expect("stylesheet");
    let mut app = build_app(&stylesheet, true, on_event);
    app.frame(frame(0));

    app.handle_engine_event(&moved(30.0, 30.0));
    app.handle_engine_event(&primary(ButtonState::Pressed));
    app.handle_engine_event(&moved(60.0, 60.0));
    // The app is otherwise idle; focus loss must still produce a frame because
    // the cancellation is pending.
    assert!(app.state().drag.has_pending());
    app.handle_engine_event(&EngineEvent::FocusChanged(false));
    app.frame(frame(1));

    let update = app.state().latest.as_ref().expect("cancellation frame");
    assert!(update.transitions.iter().any(|transition| matches!(
        transition,
        DragTransition::Cancelled {
            reason: DragCancelReason::FocusLost,
            position: PointerPosition { x: 60.0, y: 60.0 },
            ..
        }
    )));
    assert_eq!(app.state().cancelled, 1);
    assert_eq!(app.state().drag.active_pointer_count(), 0);
    assert!(app.frame(frame(2)).len() > 0);
    assert!(!app.state().drag.has_pending());
}

#[test]
fn wheel_deltas_reach_the_frame_update_accumulated() {
    let stylesheet = parse_stylesheet(STYLESHEET).expect("stylesheet");
    let mut app = build_app(&stylesheet, true, on_event);
    app.frame(frame(0));
    app.state().reset_counters();

    app.handle_engine_event(&scroll(ScrollDelta::Lines { x: 1.0, y: 2.0 }));
    app.handle_engine_event(&scroll(ScrollDelta::Lines { x: 2.0, y: 3.0 }));
    app.handle_engine_event(&scroll(ScrollDelta::Pixels { x: 10.0, y: 20.0 }));
    app.frame(frame(1));

    let update = app.state().latest.as_ref().expect("wheel frame");
    assert_eq!(update.wheel_lines, (3.0, 5.0));
    assert_eq!(update.wheel_pixels, (10.0, 20.0));
    assert_eq!(app.state().geometry_builds.get(), 1);
    // Repeating the same frame index cannot consume wheel effects twice.
    app.frame(frame(1));
    assert_eq!(app.state().geometry_builds.get(), 1);
}

#[test]
fn keyboard_and_text_events_do_not_disturb_the_pending_drag() {
    let stylesheet = parse_stylesheet(STYLESHEET).expect("stylesheet");
    let mut app = build_app(&stylesheet, true, on_event);
    app.frame(frame(0));

    app.handle_engine_event(&moved(15.0, 25.0));
    app.handle_engine_event(&primary(ButtonState::Pressed));
    app.handle_engine_event(&EngineEvent::Key(KeyboardEvent {
        logical_key: KeyIdentity::Character("a".into()),
        physical_key: None,
        location: Default::default(),
        state: ButtonState::Pressed,
        repeat: false,
        modifiers: KeyboardModifiers::default(),
    }));
    app.handle_engine_event(&EngineEvent::TextInput(TextInputEvent::Commit(
        "typed".into(),
    )));
    app.handle_engine_event(&EngineEvent::ModifiersChanged(KeyboardModifiers {
        shift: true,
        ..KeyboardModifiers::default()
    }));
    app.frame(frame(1));

    let update = app.state().latest.as_ref().expect("frame");
    let pointer = update.pointer(MOUSE_POINTER).expect("pointer");
    assert!(pointer.active);
    assert_eq!(pointer.position, PointerPosition { x: 15.0, y: 25.0 });
    assert_eq!(update.transitions.len(), 1);
    assert_eq!(
        app.state().raw_events,
        5,
        "raw keyboard/text path is intact"
    );
}

#[test]
fn capture_ownership_survives_motion_until_release() {
    let mut drag = FrameAlignedDrag::new();
    let board = ElementPath::root(0);
    drag.set_capture(MOUSE_POINTER, Some(board.clone()));
    drag.feed(&moved(5.0, 5.0));
    drag.feed(&primary(ButtonState::Pressed));
    drag.feed(&moved(40.0, 40.0));
    let first = drag.begin_frame(1).expect("frame");
    assert_eq!(first.pointers[0].capture, Some(board.clone()));
    drag.feed(&moved(70.0, 70.0));
    let second = drag.begin_frame(2).expect("frame");
    assert_eq!(second.pointers[0].capture, Some(board.clone()));
    drag.feed(&primary(ButtonState::Released));
    let release = drag.begin_frame(3).expect("release frame");
    assert!(matches!(
        &release.transitions[0],
        DragTransition::Released { capture: Some(path), .. } if path == &board
    ));
    assert_eq!(drag.capture(MOUSE_POINTER), None);
}

#[test]
fn independent_pointers_publish_independent_latest_positions() {
    let mut drag = FrameAlignedDrag::new();
    let first = PointerId::new(11);
    let second = PointerId::new(12);
    drag.feed_pointer_at(
        first,
        PointerPosition { x: 1.0, y: 2.0 },
        &primary(ButtonState::Pressed),
    );
    drag.feed_pointer_at(
        second,
        PointerPosition { x: 30.0, y: 40.0 },
        &primary(ButtonState::Pressed),
    );
    drag.feed_pointer(first, &moved(5.0, 6.0));
    drag.feed_pointer(second, &moved(70.0, 80.0));
    let update = drag.begin_frame(1).expect("frame");
    assert_eq!(update.pointers.len(), 2);
    assert_eq!(
        update.pointer(first).expect("first").position,
        PointerPosition { x: 5.0, y: 6.0 }
    );
    assert_eq!(
        update.pointer(second).expect("second").position,
        PointerPosition { x: 70.0, y: 80.0 }
    );
    drag.feed_pointer(first, &primary(ButtonState::Released));
    let update = drag.begin_frame(2).expect("release frame");
    assert!(!update.pointer(first).expect("first").active);
    assert!(update.pointer(second).expect("second").active);
}

#[test]
fn idle_to_active_scheduling_wakes_exactly_once_per_frame() {
    let start = Instant::now();
    let mut cadence = FrameInputCadence::new();

    // Idle: the first input after a quiet period paints immediately.
    assert_eq!(
        cadence.schedule(start, FRAME_TIME),
        InputRedrawDecision::Immediate
    );

    // Display a frame, then two samples inside the next frame window.
    cadence.observe_frame(start);
    let first_sample = start + Duration::from_millis(3);
    let second_sample = start + Duration::from_millis(11);
    assert_eq!(
        cadence.schedule(first_sample, FRAME_TIME),
        InputRedrawDecision::WaitUntil(start + FRAME_TIME)
    );
    assert_eq!(
        cadence.schedule(second_sample, FRAME_TIME),
        InputRedrawDecision::WaitUntil(start + FRAME_TIME)
    );
    assert!(!cadence.take_due(start + Duration::from_millis(15)));
    // Exactly one deferred wake, at the frame boundary.
    assert!(cadence.take_due(start + FRAME_TIME));
    assert!(!cadence.take_due(start + FRAME_TIME));
    // After that paint the app is idle again, so the next input is immediate.
    assert_eq!(
        cadence.schedule(start + FRAME_TIME + Duration::from_millis(1), FRAME_TIME),
        InputRedrawDecision::Immediate
    );
}

#[test]
fn app_opt_in_controls_the_input_redraw_policy() {
    let stylesheet = parse_stylesheet(STYLESHEET).expect("stylesheet");
    let default_app = build_app(&stylesheet, false, on_event);
    assert_eq!(
        default_app.input_redraw_policy(),
        InputRedrawPolicy::Immediate
    );
    let aligned = build_app(&stylesheet, true, on_event);
    assert_eq!(
        aligned.input_redraw_policy(),
        InputRedrawPolicy::FrameAligned
    );
}

#[test]
fn huge_motion_burst_keeps_pending_state_bounded() {
    let mut drag = FrameAlignedDrag::new();
    drag.feed(&moved(0.0, 0.0));
    drag.feed(&primary(ButtonState::Pressed));
    assert_eq!(drag.active_pointer_count(), 1);
    for index in 0..200_000_u32 {
        let value = index as f32;
        drag.feed(&moved(value, value));
        assert!(drag.tracked_pointer_count() <= MAX_ACTIVE_POINTERS);
        assert!(drag.pending_transition_count() <= MAX_PENDING_TRANSITIONS);
    }
    let update = drag.begin_frame(1).expect("frame");
    assert_eq!(update.pointers.len(), 1);
    assert_eq!(
        update.pointers[0].position,
        PointerPosition {
            x: 199_999.0,
            y: 199_999.0
        }
    );
    assert!(!update.overflowed);
    // No stale samples: the next idle publish is empty.
    assert!(drag.begin_frame(2).is_none());
}
