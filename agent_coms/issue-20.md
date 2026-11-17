## 2026-11-17T21:05Z - issue-20 agent

Branch `agent/issue-20-frame-aligned-drag`, base `origin/main` 6d49b79.

### Scope implemented

- New additive module `src/frame_drag.rs` (`cssimpler::frame_drag`) with
  `FrameAlignedDrag`: bounded latest-latest drag state (latest position per
  pointer, pressed buttons, capture target, ordered press/release/cancel
  transitions, accumulated wheel line/pixel deltas). Fixed capacities:
  `MAX_ACTIVE_POINTERS=8`, `MAX_PENDING_TRANSITIONS=32`,
  `MAX_BUTTONS_PER_POINTER=8`; motion is latest-wins and never queued. Overflow is
  fail-safe (`FrameDragUpdate::overflowed`, all pointers cancelled). `size_of` on
  the 64-bit macOS debug host: 2616 bytes.
- `FrameAlignedDrag::begin_frame(frame_index)` publishes once per displayed frame;
  repeating a frame index returns `None` (follow-up rebuilds cannot double-consume).
  A move plus release before a paint publishes the release transition with the
  final position and the press-time capture target.
- Renderer scheduling opt-in: `SceneProvider::input_redraw_policy` (default
  `Immediate`) and `renderer::InputRedrawPolicy::FrameAligned`. New pure
  `FrameInputCadence` decides immediate vs `WaitUntil(last frame + frame_time)`;
  the runtime wakes an idle event loop at the deferred deadline, and records every
  displayed frame. Runtime coalesces only pointer-motion/wheel paints; raw events
  are still delivered immediately by `handle_engine_event`. Press/release edges
  keep immediate paints. Touch synthesized pointers use the same policy.
- App/FragmentApp builder `with_frame_aligned_input(bool)`.
- Example `examples/vector_frame_drag.rs`: native vector drag with cached static
  background + uncached overlay/handle, headless replay (default) and `--window`.
- Docs `docs/frame-aligned-drag.md` + one link in `docs/README.md`.
- Tests `tests/issue_20_frame_drag.rs` (11) and unit tests in `src/frame_drag.rs`
  (11) and `renderer/src/input.rs` (3 cadence tests).

### Validation evidence (debug, shared host)

```
cargo check --workspace -j 2                      -> ok (pre-existing unused-anyhow warning)
cargo test -j 2 -p cssimpler --lib                -> 92 passed, 0 failed
cargo test -j 2 --test issue_20_frame_drag        -> 11 passed, 0 failed
cargo test -j 2 -p cssimpler-renderer --lib -- --test-threads=1 \
  input::tests tests::dispatch_mouse_event_supports_requested_handlers \
  tests::hover_transition_dispatches_enter_leave_and_over_out_handlers \
  tests::dispatch_click_* touch::tests ...         -> 15 passed, 0 failed
```

Note: renderer tests `tests::dispatch_mouse_event_supports_requested_handlers`
and `tests::hover_transition_dispatches_enter_leave_and_over_out_handlers` share a
process-global `EVENT_FLAGS` counter and fail when run concurrently with each
other (pre-existing, reproduced only in parallel); they pass individually and
serially. This change does not touch that dispatch path.

Headless replay (`cargo run -j 2 --example vector_frame_drag`, debug, load avg
~14 on a 16 GB macOS host shared with another building agent):

```
raw-per-sample:  moves=241 display_frames=242 view_builds=242 geometry_builds=242 replay=15.76 ms update+view avg=63.4 us max=244.9 us
frame-aligned:   moves=241 display_frames=61  drag_frames=61  view_builds=61  geometry_builds=61  replay=3.75 ms update+view avg=60.1 us max=133.0 us
bounded state: 1000000 motion samples -> tracked_pointers=1 (max 8), pending_transitions=1 (max 32), FrameAlignedDrag=2616 bytes
bounded state: 12 pointers -> overflowed=true transitions=7 (max 32)
process RSS before/after 1M-sample probe: 12352 KiB / 12384 KiB
cached background layer builds across 3 drag paints: [1, 1, 1] (entries=1, bytes=7884800)
```

Release run (5 runs, median; load avg 8.56-9.31 during the runs, shared host,
Apple M1 Pro 16 GB, macOS 27.0.1, rustc 1.101.0-nightly, thin LTO):

| Metric | Raw per-sample | Frame-aligned |
| --- | ---: | ---: |
| moves fed / display frames | 241 / 242 | 241 / 61 |
| view rebuilds | 242 | 61 |
| geometry rebuilds | 242 | 61 |
| update+view+scene avg | 7.9 us | 7.4 us |
| update+view+scene max | 22.6 us | 8.7 us |
| replay wall time (median) | 1.94 ms | 0.46 ms |

1M motion samples while pressed: 1 tracked pointer, 1 pending transition,
`FrameAlignedDrag` = 2616 bytes, RSS +16-32 KiB (allocator noise).
12 simultaneous pointers: overflowed=true, 7 transitions retained (ring max 32).
3 drag paints of the cached background: layer builds `[1, 1, 1]`, 1 entry,
7,884,800 retained bytes.

Raw run log: `target/issue20-release-runs.txt` (git-ignored build dir; 5 runs plus load lines).

### Limitations / open evidence

- `EngineEvent::PointerMoved` has no pointer id, so the built-in winit mouse path
  is one pointer. Independent-pointer handling is implemented and tested through
  `feed_pointer` / `feed_pointer_at`; forwarding winit touch ids through
  `EngineEvent` is left as a follow-up (scope kept minimal).
- Interactive window validation needs a real display and was not automated.
- The renderer's hover/enter/leave, click and press-target dispatch code is
  untouched; their existing tests are the regression evidence.

### Anticipated conflicts

- `docs/README.md` is also edited by issue #19 and #21 (additional link lines);
  trivial one-line conflict to resolve at integration.
- No overlap with #19/#21/#22 in `renderer/src/runtime.rs`,
  `renderer/src/input.rs`, or `src/app.rs` according to their current branch file
  lists.
