# Frame-aligned drag updates

Pointer devices can report several motion samples between two displayed frames.
Rebuilding vector geometry for every intermediate sample spends CPU on frames that
are never shown. The frame-aligned drag API is an **opt-in** way to consume only
the latest drag state once per displayed frame, while every raw input sample keeps
flowing through the existing event path.

Nothing changes for existing applications. Without the opt-in builder, pointer
input keeps requesting a paint per event, `on_mousemove` handlers keep their
existing dispatch, and freehand/gesture consumers keep receiving every sample.

## Quick start

```rust
use cssimpler::{
    app::{App, Invalidation},
    frame_drag::{FrameAlignedDrag, FrameDragUpdate},
    renderer::{EngineEvent, FrameInfo},
};

struct State {
    drag: FrameAlignedDrag,
    preview: Option<FrameDragUpdate>,
    // application geometry...
}

fn on_event(state: &mut State, event: &EngineEvent) -> bool {
    // Raw path: every sample arrives here. The coalescer only keeps the latest
    // position, pressed buttons, capture target, and ordered press/release edges.
    state.drag.feed(event);
    false // do not consume motion; the frame update owns the rebuild
}

fn update(state: &mut State, frame: FrameInfo) -> Invalidation {
    let Some(drag) = state.drag.begin_frame(frame.frame_index) else {
        return Invalidation::Clean;
    };
    if drag.overflowed {
        // The bounded batch overflowed: drop the preview and reset the drag.
        state.preview = None;
        return Invalidation::Paint;
    }
    // `drag.pointers` holds the latest position per pointer; `drag.transitions`
    // holds ordered press/release/cancel edges; `drag.wheel_*` holds sums.
    state.preview = Some(drag);
    Invalidation::Paint
}

fn view(state: &State) -> Node {
    // Build the active path/overlay from state.preview.
    // ...
}

App::new(state, &stylesheet, update, view)
    .with_event_handler(on_event)
    .with_frame_aligned_input(true);
```

`FrameAlignedDrag::feed` always returns whether the event changed drag state. The
return value is only needed by applications that schedule paints themselves; with
`with_frame_aligned_input(true)` the runtime schedules the coalesced paint.

## Scheduling contract

### Raw delivery

`EngineEvent`s are still delivered to the application event handler and to native
text input immediately, in arrival order. The frame-aligned opt-in changes **when
pointer-driven paints happen**, never what the application receives.

### Coalesced paints

`App::with_frame_aligned_input(true)` makes the app report
`InputRedrawPolicy::FrameAligned` to the renderer. For pointer motion and wheel
input, the runtime:

1. delivers the event immediately;
2. if the app is idle (no displayed frame within the configured `frame_time`),
   requests a paint immediately, so input after an idle period is responsive;
3. otherwise defers the paint to the next frame boundary, at most one per frame
   interval;
4. wakes the event loop with `ControlFlow::WaitUntil(deadline)` when deferred, so
   an otherwise idle app still repaints the latest state.

Press, release, click, focus, keyboard, text/IME, and window events keep their
immediate paint behavior, so edges are not delayed to a later frame.

`SceneProvider::input_redraw_policy` is the provider-facing contract; implement it
directly for custom scene providers. `FrameInputCadence` contains the pure timing
decision and is unit tested without a window.

### One update per displayed frame

Call `FrameAlignedDrag::begin_frame(frame.frame_index)` once per update. The first
call for a frame index publishes the pending state; repeating the same index (for
example a follow-up rebuild inside one displayed frame) returns `None`. Pending
motion, transitions, and wheel sums are cleared only after the snapshot is built.
An idle frame with no pending input returns `None` and the update can return
`Invalidation::Clean`, leaving the view function unrun.

## API summary

| Item | Contract |
| --- | --- |
| `FrameAlignedDrag::feed(event)` | Feed an event for the primary mouse pointer. |
| `feed_pointer(id, event)` | Feed an independent pointer id (touch/pen host integration). |
| `feed_pointer_at(id, position, event)` | Feed an event plus the position the host already knows (touch press). |
| `set_position(id, position)` | Update the latest position without an event. |
| `set_capture(id, path)` | Record the handler that owns the pointer. Presses snapshot it; motion never changes it. |
| `begin_frame(index)` | Publish `Option<FrameDragUpdate>` for a displayed frame. |
| `has_pending()` | Whether unpublished input is waiting for the next frame. |
| `reset()` | Drop all pending state (for example after a native system drag). |
| `FrameDragUpdate::pointer(id)` | Latest `DragPointerState` (position, buttons, capture, active, moved). |
| `wheel_lines` / `wheel_pixels` | Scroll deltas accumulated since the previous frame. |

`FrameDragUpdate::transitions` is a `Vec<DragTransition>` in arrival order:
`Pressed { pointer, button, position, capture }`,
`Released { pointer, button, position, capture }`, and
`Cancelled { pointer, reason, position }`. Motion is never an edge: samples
overwrite the latest position.

## Capture, release flush, cancellation, independent pointers

- **Capture/target ownership.** The coalescer records the `set_capture` target at
  press time and keeps it in every frame snapshot until release. Motion and
  pointer drift do not change the owner. The renderer's existing press-target,
  click, and `:active` handling is untouched.
- **Final position before release.** A move followed by a release before the next
  paint publishes both the release transition and the final pointer position in
  the same update. The runtime also paints press/release edges immediately, so the
  final position is never skipped.
- **Cancellation.** `PointerLeft`, `FocusChanged(false)`, and `Suspended` cancel
  every active pointer with `PointerLeft`, `FocusLost`, or `Suspended` reasons and
  keep the last known position in the cancellation transition. Focus gain and
  modifier changes are not cancellations.
- **Independent pointers.** `feed_pointer` keyed by `PointerId` keeps a separate
  latest position, pressed-button list, and capture target per pointer. Releasing
  one pointer does not disturb another. The built-in desktop mouse path reports one
  pointer; see [Limitations](#limitations).

## Bounded pending state

| Bound | Value | Behavior |
| --- | --- | --- |
| `MAX_ACTIVE_POINTERS` | 8 | Idle slots are reused; over 8 active pointers fails safe. |
| `MAX_PENDING_TRANSITIONS` | 32 | Press/release/cancel ring capacity. |
| `MAX_BUTTONS_PER_POINTER` | 8 | Simultaneous buttons retained per pointer. |

Motion is latest-wins and never queues. There is no per-sample history, pixel
buffer, or automatic layer cache. If the bounded batch overflows (more than eight
simultaneous active pointers, more than thirty-two pending edges, or more than
eight buttons on one pointer), the update sets `overflowed: true` and cancels all
pointers instead of growing memory or delivering a partial sequence. Consumers
must discard an overflowed batch. After the frame drains it, input is accepted
again.

The example prints `size_of::<FrameAlignedDrag>()` (about 2.6 KiB) and feeds one
million motion samples with a fixed retained-state size.

## Wheel and pan deltas

`ScrollDelta::Lines` and `ScrollDelta::Pixels` are summed into two separate frame
accumulators, so a burst of wheel events reaches the frame with the total effect
instead of only the last sample. The coalescer does not convert units; apply your
own pixels-per-line factor if the two must be combined. The runtime's scrollbar
path already accumulates wheel input before painting.

## What is preserved

- Raw movement events, pointer positions, and the existing event-handler API.
- Per-sample consumers: if an event handler returns `true` for motion, the
  historical structure invalidation and per-sample view rebuild behavior remain.
- Press/release ordering, primary/secondary/middle button state, click and
  double-click detection, press-target ownership.
- Keyboard input, text/IME commit and preedit events, focus changes, modifiers.
- Semantic hover/enter/leave/over/out dispatch, which runs from the displayed
  scene once per frame as before.

The only behavior change is that an opted-in app's runtime pointer-motion and
wheel paints are delayed to the frame boundary.

## Refresh behavior and caching

- **Caching does not schedule frames.** `with_cached_layer(true)` only retains
  pixels for an unchanged subtree; it never requests a refresh. Committing an
  edited path must still return `Invalidation::Paint` (or a scoped refresh) from
  update.
- **A paint refresh still runs the view function.** `Invalidation::Paint` reruns
  `view`; only `Invalidation::Clean` skips it. Keep the view cheap: build stable
  vector scenes once in application state and clone them into the view, and put
  the moving active path in a separate uncached overlay leaf.
- **Return `Clean` when there is no drag update.** `begin_frame` returning `None`
  means the frame has nothing new; that is the common idle case.
- **Release flushes the final position.** The release transition and final pointer
  position are published in the same update, so a commit-on-release handler sees
  the last position.

The example keeps its grid/artwork scene in state, rebuilds only the overlay, and
marks the background leaf as an explicit cached layer. Its headless probe paints
three drag frames and observes the background layer build count unchanged
(`[1, 1, 1]` for the whole sequence), so stable content keeps its retained raster
while the overlay is repainted.

## Example and tests

```sh
# Headless replay (default): view/geometry rebuild counts, bounded-state probe,
# cached-background paint probe, process RSS.
cargo run -j 2 --example vector_frame_drag

# Interactive native vector drag window (needs a real display).
cargo run -j 2 --example vector_frame_drag -- --window
```

```sh
cargo test -j 2 --test issue_20_frame_drag
cargo test -j 2 -p cssimpler --lib frame_drag
cargo test -j 2 -p cssimpler-renderer --lib frame_input_cadence
```

The integration tests cover bursts of moves, move-plus-release before a paint,
capture changes, focus-loss and pointer-left cancellation, independent pointers,
wheel accumulation, keyboard/text preservation, idle-to-active scheduling, and a
200,000-sample bounded-memory burst. The example's replay is the measurement
harness for rebuild counts and latency.

## Measured replay

Measured on 2026-11-17 on a 16 GB Apple M1 Pro (8 cores) macOS 27.0.1 host,
rustc 1.101.0-nightly, release profile (thin LTO, `codegen-units = 1`). Another
agent was compiling concurrently, so the machine was shared: 1-minute load
averages were 8.56-9.31 during the five measured runs (earlier samples reached
14-17). Numbers are medians of five runs; the first run includes cold-cache
warmup. No other build or benchmark from this worktree was running.

The fixed replay feeds 241 pointer samples (press, 240 moves, release) grouped
into 61 simulated display frames at `MOVES_PER_FRAME = 4`. The raw baseline paints
once per delivered sample, matching the default immediate policy. Both columns use
the same application update/view code and native vector scenes.

| Metric | Raw per-sample | Frame-aligned |
| --- | ---: | ---: |
| Pointer samples fed | 241 | 241 |
| Displayed frames (simulated) | 242 | 61 |
| View rebuilds | 242 | 61 |
| Geometry rebuilds | 242 | 61 |
| View + geometry rebuilds per move | 1.00 | 0.25 |
| Update + view + scene time, median avg / max | 7.9 µs / 22.6 µs | 7.4 µs / 8.7 µs |
| Replay wall time, median | 1.94 ms | 0.46 ms |

The per-frame update+view cost is the same order in both modes; the win is that
the frame-aligned route performs it 61 times instead of 242 times for the same
input. Interaction latency is bounded by the scheduling contract rather than
measured as a display latency here: the first input after an idle period paints
immediately, later samples inside a frame window are consumed at the next frame
boundary (never later), and `FrameInputCadence` unit tests assert the boundary
wake. Press/release edges still paint immediately, so a release is not deferred.

Bounded state in the same release run:

| Probe | Result |
| --- | --- |
| `size_of::<FrameAlignedDrag>()` | 2616 bytes |
| 1,000,000 motion samples while pressed | 1 tracked pointer, 1 pending transition, latest position published |
| 12 concurrent pointer presses | `overflowed = true`, 7 transitions retained (≤ 32), all pointers cancelled |
| Process RSS before/after the 1M-sample probe | about +16-32 KiB (allocator noise) |
| Cached background builds over 3 drag paints | `[1, 1, 1]`, 1 cache entry / 7,884,800 retained bytes |

The RSS probe uses `ps -o rss= -p <pid>` on macOS; it includes allocator noise and
is not a precise heap measurement. The structural bound is the fixed-capacity
state plus the assertions in `tests/issue_20_frame_drag.rs`.

## Limitations

- `EngineEvent::PointerMoved` does not carry a pointer id, so the built-in winit
  mouse path is a single pointer. `feed_pointer` supports independent pointers for
  hosts that already track contact ids; wiring winit touch ids through the engine
  event is a separate change. The interactive window therefore demonstrates one
  mouse pointer.
- Frame-aligned paint coalescing currently covers pointer motion and wheel input.
  Keyboard/text/focus events keep immediate paints by design.
- The interactive window demo needs a real windowing session and is not part of
  the automated headless tests. Everything measured here is driven headlessly.
