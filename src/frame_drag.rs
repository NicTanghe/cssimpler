//! Opt-in frame-aligned drag coalescing.
//!
//! Pointer devices can deliver several motion samples between two displayed
//! frames. Applications that rebuild vector geometry from the pointer position
//! only need the latest sample, once per frame. [`FrameAlignedDrag`] stores that
//! bounded latest state and publishes it through [`FrameAlignedDrag::begin_frame`].
//! Raw [`EngineEvent`] delivery is untouched: applications still receive every
//! sample through their normal event handler, which is what freehand drawing
//! needs.
//!
//! The scheduling half of the contract lives in the renderer: install the drag
//! state in application state, feed every event from the app's event handler,
//! call [`FrameAlignedDrag::begin_frame`] at the top of the update callback, and
//! enable [`crate::app::App::with_frame_aligned_input`]. That opts the app into
//! at most one pointer-driven paint per frame interval.
//!
//! Pending state is fixed-capacity: each pointer keeps only its latest position,
//! its pressed buttons, and its capture target. Press/release/cancel transitions
//! use a bounded ring. Nothing in the hot motion path allocates or grows.

use crate::core::ElementPath;
use crate::renderer::{ButtonState, EngineEvent, PointerButton, PointerPosition, ScrollDelta};

/// Pointer id used by the primary mouse and by [`FrameAlignedDrag::feed`].
pub const MOUSE_POINTER: PointerId = PointerId(0);

/// Maximum number of pointers whose latest drag state is retained.
pub const MAX_ACTIVE_POINTERS: usize = 8;

/// Maximum number of press/release/cancel transitions retained between frames.
pub const MAX_PENDING_TRANSITIONS: usize = 32;

/// Maximum number of simultaneously pressed buttons retained per pointer.
pub const MAX_BUTTONS_PER_POINTER: usize = 8;

/// Stable identifier for an independent pointer (mouse, touch contact, pen).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PointerId(u64);

impl PointerId {
    pub const fn new(id: u64) -> Self {
        Self(id)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Why an active pointer was cancelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragCancelReason {
    /// The pointer left the window without a release.
    PointerLeft,
    /// The window lost focus or the platform suspended it.
    FocusLost,
    /// The platform suspended the application.
    Suspended,
    /// More pointers, buttons, or transitions arrived than the bounded state can
    /// retain. The whole pending batch must be discarded.
    Overflow,
}

/// Press/release/cancel edges published together with a frame snapshot.
///
/// Ordering follows arrival order. Motion is not an edge: the latest position is
/// reported through [`DragPointerState::position`].
#[derive(Clone, Debug, PartialEq)]
pub enum DragTransition {
    Pressed {
        pointer: PointerId,
        button: PointerButton,
        position: PointerPosition,
        capture: Option<ElementPath>,
    },
    Released {
        pointer: PointerId,
        button: PointerButton,
        position: PointerPosition,
        capture: Option<ElementPath>,
    },
    Cancelled {
        pointer: PointerId,
        reason: DragCancelReason,
        position: PointerPosition,
    },
}

impl DragTransition {
    pub fn pointer(&self) -> PointerId {
        match self {
            Self::Pressed { pointer, .. }
            | Self::Released { pointer, .. }
            | Self::Cancelled { pointer, .. } => *pointer,
        }
    }

    pub fn position(&self) -> PointerPosition {
        match self {
            Self::Pressed { position, .. }
            | Self::Released { position, .. }
            | Self::Cancelled { position, .. } => *position,
        }
    }
}

/// Latest state of one pointer during the published frame.
#[derive(Clone, Debug, PartialEq)]
pub struct DragPointerState {
    pub id: PointerId,
    /// Latest pointer position at frame time.
    pub position: PointerPosition,
    /// Buttons still held, in press order.
    pub buttons: Vec<PointerButton>,
    /// Handler that owned the pointer when it was pressed.
    pub capture: Option<ElementPath>,
    /// Whether the pointer is still held at frame time.
    pub active: bool,
    /// Whether the position changed since the previously published frame.
    pub moved: bool,
}

impl DragPointerState {
    pub fn is_active(&self) -> bool {
        self.active
    }
}

/// One coalesced drag snapshot, produced at most once per displayed frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FrameDragUpdate {
    /// Frame index supplied to [`FrameAlignedDrag::begin_frame`].
    pub frame_index: u64,
    /// Latest state per pointer that was active or released at frame time.
    pub pointers: Vec<DragPointerState>,
    /// Ordered press/release/cancel edges since the previous published frame.
    pub transitions: Vec<DragTransition>,
    /// Accumulated line-unit wheel deltas (`x`, `y`) since the previous frame.
    pub wheel_lines: (f32, f32),
    /// Accumulated pixel-unit wheel deltas (`x`, `y`) since the previous frame.
    pub wheel_pixels: (f32, f32),
    /// `true` when the bounded pending state overflowed. Discard this batch and
    /// reset the drag: entries may be missing or already cancelled.
    pub overflowed: bool,
}

impl FrameDragUpdate {
    pub fn pointer(&self, pointer: PointerId) -> Option<&DragPointerState> {
        self.pointers.iter().find(|state| state.id == pointer)
    }

    pub fn has_motion(&self) -> bool {
        self.pointers.iter().any(|pointer| pointer.moved)
    }

    pub fn has_wheel(&self) -> bool {
        self.wheel_lines != (0.0, 0.0) || self.wheel_pixels != (0.0, 0.0)
    }
}

#[derive(Clone, Debug, Default)]
struct PointerSlot {
    id: PointerId,
    position: PointerPosition,
    buttons: [Option<PointerButton>; MAX_BUTTONS_PER_POINTER],
    button_count: usize,
    capture: Option<ElementPath>,
    active: bool,
    moved: bool,
    released: bool,
}

impl PointerSlot {
    fn new(id: PointerId) -> Self {
        Self {
            id,
            position: PointerPosition::default(),
            buttons: [None; MAX_BUTTONS_PER_POINTER],
            button_count: 0,
            capture: None,
            active: false,
            moved: false,
            released: false,
        }
    }

    fn snapshot(&self) -> DragPointerState {
        DragPointerState {
            id: self.id,
            position: self.position,
            buttons: self.buttons[..self.button_count]
                .iter()
                .flatten()
                .copied()
                .collect(),
            capture: self.capture.clone(),
            active: self.active,
            moved: self.moved,
        }
    }

    fn holds(&self, button: PointerButton) -> bool {
        self.buttons[..self.button_count]
            .iter()
            .flatten()
            .any(|pressed| *pressed == button)
    }

    fn press(&mut self, button: PointerButton) -> bool {
        if self.holds(button) || self.button_count == MAX_BUTTONS_PER_POINTER {
            return false;
        }
        self.buttons[self.button_count] = Some(button);
        self.button_count += 1;
        self.active = true;
        self.released = false;
        true
    }

    fn release(&mut self, button: PointerButton) -> bool {
        let Some(index) = self.buttons[..self.button_count]
            .iter()
            .position(|pressed| *pressed == Some(button))
        else {
            return false;
        };
        self.buttons[index] = self.buttons[self.button_count - 1];
        self.buttons[self.button_count - 1] = None;
        self.button_count -= 1;
        if self.button_count == 0 {
            self.active = false;
            self.released = true;
        }
        true
    }
}

/// Bounded, opt-in coalescer for the latest drag state per displayed frame.
///
/// Feed every raw [`EngineEvent`] from an app event handler with [`feed`](Self::feed)
/// (or explicit ids with [`feed_pointer`](Self::feed_pointer)), then call
/// [`begin_frame`](Self::begin_frame) once per update. The returned
/// [`FrameDragUpdate`] contains the latest position per pointer, ordered
/// press/release/cancel transitions, and accumulated wheel deltas. Calling
/// `begin_frame` again for the same frame index returns `None`, so follow-up
/// view rebuilds within one displayed frame cannot consume the same drag twice.
///
/// Memory is bounded by [`MAX_ACTIVE_POINTERS`], [`MAX_PENDING_TRANSITIONS`], and
/// [`MAX_BUTTONS_PER_POINTER`]; motion samples overwrite the latest position and
/// never queue. On overflow the update is marked [`FrameDragUpdate::overflowed`]
/// and all pointers are cancelled; consumers must discard that batch.
#[derive(Clone, Debug)]
pub struct FrameAlignedDrag {
    pointers: [Option<PointerSlot>; MAX_ACTIVE_POINTERS],
    transitions: [Option<DragTransition>; MAX_PENDING_TRANSITIONS],
    transition_head: usize,
    transition_len: usize,
    wheel_lines: (f32, f32),
    wheel_pixels: (f32, f32),
    wheel_pending: bool,
    motion: bool,
    forced: bool,
    overflowed: bool,
    last_frame: Option<u64>,
}

impl Default for FrameAlignedDrag {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameAlignedDrag {
    pub fn new() -> Self {
        Self {
            pointers: std::array::from_fn(|_| None),
            transitions: std::array::from_fn(|_| None),
            transition_head: 0,
            transition_len: 0,
            wheel_lines: (0.0, 0.0),
            wheel_pixels: (0.0, 0.0),
            wheel_pending: false,
            motion: false,
            forced: false,
            overflowed: false,
            last_frame: None,
        }
    }

    /// Feed one event for the primary mouse pointer.
    ///
    /// Returns `true` when the event changed drag state, so a caller that is not
    /// using [`crate::app::App::with_frame_aligned_input`] can still request a
    /// paint. Idle pointer motion only records the position and returns `false`.
    pub fn feed(&mut self, event: &EngineEvent) -> bool {
        self.feed_pointer(MOUSE_POINTER, event)
    }

    /// Feed one event for an explicit independent pointer id.
    ///
    /// The built-in mouse path reports one pointer. Hosts that track multiple
    /// touches or pens pass their stable device id here so each pointer keeps an
    /// independent latest position, pressed buttons, and capture target.
    pub fn feed_pointer(&mut self, pointer: PointerId, event: &EngineEvent) -> bool {
        match event {
            EngineEvent::PointerMoved { position, .. } => self.move_pointer(pointer, *position),
            EngineEvent::PointerButton { button, state, .. } => match state {
                ButtonState::Pressed => self.press(pointer, *button),
                ButtonState::Released => self.release(pointer, *button),
            },
            EngineEvent::Scroll { delta, .. } => {
                self.accumulate_wheel(*delta);
                true
            }
            EngineEvent::PointerLeft => self.cancel_all(DragCancelReason::PointerLeft),
            EngineEvent::FocusChanged(false) => self.cancel_all(DragCancelReason::FocusLost),
            EngineEvent::Suspended => self.cancel_all(DragCancelReason::Suspended),
            _ => false,
        }
    }

    /// Feed an event together with a position the host already knows, such as a
    /// touch contact that reports its press location without a prior move.
    pub fn feed_pointer_at(
        &mut self,
        pointer: PointerId,
        position: PointerPosition,
        event: &EngineEvent,
    ) -> bool {
        self.set_position(pointer, position);
        self.feed_pointer(pointer, event)
    }

    /// Record the latest pointer position without an event.
    pub fn set_position(&mut self, pointer: PointerId, position: PointerPosition) -> bool {
        let Some(index) = self.slot_for(pointer) else {
            return false;
        };
        let slot = self.pointers[index].as_mut().expect("slot exists");
        let changed = slot.position != position;
        slot.position = position;
        if slot.active && changed {
            slot.moved = true;
            self.motion = true;
        }
        changed
    }

    /// Assign the capture owner for a pointer. Presses record the current value;
    /// motion never changes it, which preserves target ownership during a drag.
    pub fn set_capture(&mut self, pointer: PointerId, capture: Option<ElementPath>) -> bool {
        let Some(index) = self.slot_for(pointer) else {
            return false;
        };
        let slot = self.pointers[index].as_mut().expect("slot exists");
        let changed = slot.capture != capture;
        slot.capture = capture;
        if changed && slot.active {
            self.motion = true;
        }
        changed
    }

    pub fn capture(&self, pointer: PointerId) -> Option<&ElementPath> {
        self.slot(pointer).and_then(|slot| slot.capture.as_ref())
    }

    pub fn position(&self, pointer: PointerId) -> Option<PointerPosition> {
        self.slot(pointer).map(|slot| slot.position)
    }

    /// Whether unpublished motion, transitions, wheel input, or cancellation is
    /// waiting for the next frame.
    pub fn has_pending(&self) -> bool {
        self.motion
            || self.forced
            || self.transition_len > 0
            || self.wheel_pending
            || self.overflowed
    }

    /// Whether the pointer for `pointer` is still held.
    pub fn is_pointer_active(&self, pointer: PointerId) -> bool {
        self.slot(pointer).is_some_and(|slot| slot.active)
    }

    pub fn active_pointer_count(&self) -> usize {
        self.pointers
            .iter()
            .flatten()
            .filter(|slot| slot.active)
            .count()
    }

    /// Number of allocated pointer slots, including positions seen while idle.
    pub fn tracked_pointer_count(&self) -> usize {
        self.pointers.iter().flatten().count()
    }

    pub fn pending_transition_count(&self) -> usize {
        self.transition_len
    }

    pub fn last_published_frame(&self) -> Option<u64> {
        self.last_frame
    }

    /// Publish the latest state for `frame_index`.
    ///
    /// Returns `None` when nothing changed, or when this frame index was already
    /// published (for example, a follow-up view rebuild inside one displayed
    /// frame). The drag state is cleared only after the snapshot is produced, so
    /// a move followed by a release before the next paint still reports the
    /// final position together with the release transition.
    pub fn begin_frame(&mut self, frame_index: u64) -> Option<FrameDragUpdate> {
        if self.last_frame == Some(frame_index) {
            return None;
        }
        if !self.has_pending() {
            self.last_frame = Some(frame_index);
            return None;
        }

        let mut transitions = Vec::with_capacity(self.transition_len);
        for _ in 0..self.transition_len {
            let index = self.transition_head;
            if let Some(transition) = self.transitions[index].take() {
                transitions.push(transition);
            }
            self.transition_head = (self.transition_head + 1) % MAX_PENDING_TRANSITIONS;
        }
        self.transition_len = 0;
        self.transition_head = 0;

        let pointers = self
            .pointers
            .iter()
            .flatten()
            .filter(|slot| slot.active || slot.released)
            .map(PointerSlot::snapshot)
            .collect();

        let update = FrameDragUpdate {
            frame_index,
            pointers,
            transitions,
            wheel_lines: std::mem::take(&mut self.wheel_lines),
            wheel_pixels: std::mem::take(&mut self.wheel_pixels),
            overflowed: std::mem::take(&mut self.overflowed),
        };

        for slot in self.pointers.iter_mut().flatten() {
            slot.moved = false;
            slot.released = false;
            if !slot.active {
                slot.capture = None;
                slot.button_count = 0;
            }
        }
        self.wheel_pending = false;
        self.motion = false;
        self.forced = false;
        self.last_frame = Some(frame_index);
        Some(update)
    }

    /// Drop all pending state. Use when the host restarts input tracking (for
    /// example, after a native window drag).
    pub fn reset(&mut self) {
        for slot in self.pointers.iter_mut().flatten() {
            slot.active = false;
            slot.button_count = 0;
            slot.capture = None;
            slot.moved = false;
            slot.released = false;
        }
        self.transitions = std::array::from_fn(|_| None);
        self.transition_head = 0;
        self.transition_len = 0;
        self.wheel_lines = (0.0, 0.0);
        self.wheel_pixels = (0.0, 0.0);
        self.wheel_pending = false;
        self.motion = false;
        self.forced = false;
        self.overflowed = false;
    }

    fn slot(&self, pointer: PointerId) -> Option<&PointerSlot> {
        self.pointers
            .iter()
            .flatten()
            .find(|slot| slot.id == pointer)
    }

    fn slot_index(&self, pointer: PointerId) -> Option<usize> {
        self.pointers
            .iter()
            .position(|slot| slot.as_ref().is_some_and(|slot| slot.id == pointer))
    }

    /// Find the slot for `pointer`, allocating an empty or idle slot. Returns
    /// `None` only when every slot holds an active pointer; that is an overflow.
    fn slot_for(&mut self, pointer: PointerId) -> Option<usize> {
        // After an overflow the pending batch is invalid. Ignore further input
        // until the frame boundary drains it, so the bounded batch cannot be
        // resurrected as a partial drag.
        if self.overflowed {
            return None;
        }
        if let Some(index) = self.slot_index(pointer) {
            return Some(index);
        }
        if let Some(index) = self.pointers.iter().position(Option::is_none) {
            self.pointers[index] = Some(PointerSlot::new(pointer));
            return Some(index);
        }
        if let Some(index) = self
            .pointers
            .iter()
            .position(|slot| slot.as_ref().is_some_and(|slot| !slot.active))
        {
            self.pointers[index] = Some(PointerSlot::new(pointer));
            return Some(index);
        }
        self.overflow();
        None
    }

    fn move_pointer(&mut self, pointer: PointerId, position: PointerPosition) -> bool {
        let Some(index) = self.slot_for(pointer) else {
            return false;
        };
        let slot = self.pointers[index].as_mut().expect("slot exists");
        let changed = slot.position != position;
        slot.position = position;
        if !slot.active {
            return false;
        }
        if changed {
            slot.moved = true;
        }
        self.motion = true;
        true
    }

    fn press(&mut self, pointer: PointerId, button: PointerButton) -> bool {
        let Some(index) = self.slot_for(pointer) else {
            return false;
        };
        let capture = self.pointers[index]
            .as_ref()
            .expect("slot exists")
            .capture
            .clone();
        let slot = self.pointers[index].as_mut().expect("slot exists");
        if !slot.press(button) {
            return false;
        }
        let position = slot.position;
        let transition = DragTransition::Pressed {
            pointer,
            button,
            position,
            capture,
        };
        self.push_transition(transition);
        self.forced = true;
        true
    }

    fn release(&mut self, pointer: PointerId, button: PointerButton) -> bool {
        let Some(index) = self.slot_index(pointer) else {
            return false;
        };
        let capture = self.pointers[index]
            .as_ref()
            .expect("slot exists")
            .capture
            .clone();
        let slot = self.pointers[index].as_mut().expect("slot exists");
        if !slot.release(button) {
            return false;
        }
        let position = slot.position;
        let transition = DragTransition::Released {
            pointer,
            button,
            position,
            capture,
        };
        self.push_transition(transition);
        self.forced = true;
        true
    }

    fn accumulate_wheel(&mut self, delta: ScrollDelta) {
        match delta {
            ScrollDelta::Lines { x, y } => {
                self.wheel_lines.0 += x;
                self.wheel_lines.1 += y;
            }
            ScrollDelta::Pixels { x, y } => {
                self.wheel_pixels.0 += x;
                self.wheel_pixels.1 += y;
            }
        }
        self.wheel_pending = true;
    }

    fn cancel_all(&mut self, reason: DragCancelReason) -> bool {
        let mut cancelled = false;
        // Collect first: `push_transition` mutates the ring while a pointer
        // iterator is borrowed. At most one transition per pointer slot.
        let mut pending: [Option<DragTransition>; MAX_ACTIVE_POINTERS] =
            std::array::from_fn(|_| None);
        let mut pending_len = 0;
        for slot in self.pointers.iter_mut().flatten() {
            if !slot.active {
                continue;
            }
            pending[pending_len] = Some(DragTransition::Cancelled {
                pointer: slot.id,
                reason,
                position: slot.position,
            });
            pending_len += 1;
            slot.active = false;
            slot.released = true;
            slot.button_count = 0;
            slot.buttons = [None; MAX_BUTTONS_PER_POINTER];
            cancelled = true;
        }
        for transition in pending.into_iter().flatten() {
            self.push_transition(transition);
        }
        if cancelled {
            self.forced = true;
        }
        cancelled
    }

    fn push_transition(&mut self, transition: DragTransition) {
        if self.transition_len == MAX_PENDING_TRANSITIONS {
            self.overflow();
            return;
        }
        let index = (self.transition_head + self.transition_len) % MAX_PENDING_TRANSITIONS;
        self.transitions[index] = Some(transition);
        self.transition_len += 1;
    }

    /// Fail safe: never grow the pending state. Already queued edges stay, but
    /// the update is marked `overflowed` and every pointer is cancelled so the
    /// consumer discards the batch instead of acting on a partial sequence.
    fn overflow(&mut self) {
        self.overflowed = true;
        self.forced = true;
        self.motion = false;
        for slot in self.pointers.iter_mut().flatten() {
            slot.active = false;
            slot.released = false;
            slot.button_count = 0;
            slot.buttons = [None; MAX_BUTTONS_PER_POINTER];
            slot.capture = None;
            slot.moved = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn moved(x: f32, y: f32) -> EngineEvent {
        EngineEvent::PointerMoved {
            position: PointerPosition { x, y },
            modifiers: Default::default(),
        }
    }

    fn button(button: PointerButton, state: ButtonState) -> EngineEvent {
        EngineEvent::PointerButton {
            button,
            state,
            modifiers: Default::default(),
        }
    }

    #[test]
    fn motion_is_latest_only_and_published_once_per_frame() {
        let mut drag = FrameAlignedDrag::new();
        drag.feed(&moved(1.0, 1.0));
        drag.feed(&moved(2.0, 2.0));
        drag.feed(&button(PointerButton::Primary, ButtonState::Pressed));
        drag.feed(&moved(3.0, 3.0));
        drag.feed(&moved(4.0, 4.0));

        let frame = drag.begin_frame(7).expect("frame should publish");
        assert_eq!(frame.frame_index, 7);
        assert_eq!(frame.pointers.len(), 1);
        assert_eq!(
            frame.pointers[0].position,
            PointerPosition { x: 4.0, y: 4.0 }
        );
        assert!(frame.pointers[0].active);
        assert!(frame.has_motion());
        assert_eq!(frame.transitions.len(), 1);
        assert_eq!(drag.pending_transition_count(), 0);
        assert!(!drag.has_pending());

        // The same displayed frame cannot consume the drag twice.
        assert!(drag.begin_frame(7).is_none());
        // An idle next frame publishes nothing.
        assert!(drag.begin_frame(8).is_none());
    }

    #[test]
    fn move_and_release_before_a_paint_flushes_the_final_position() {
        let mut drag = FrameAlignedDrag::new();
        drag.feed(&moved(5.0, 6.0));
        drag.feed(&button(PointerButton::Primary, ButtonState::Pressed));
        drag.feed(&moved(20.0, 30.0));
        drag.feed(&moved(40.0, 50.0));
        drag.feed(&button(PointerButton::Primary, ButtonState::Released));

        let frame = drag.begin_frame(1).expect("release must publish");
        let pointer = frame.pointer(MOUSE_POINTER).expect("pointer state");
        assert_eq!(pointer.position, PointerPosition { x: 40.0, y: 50.0 });
        assert!(!pointer.active);
        assert_eq!(frame.transitions.len(), 2);
        assert_eq!(
            frame.transitions[1].position(),
            PointerPosition { x: 40.0, y: 50.0 }
        );
        assert!(matches!(
            frame.transitions[1],
            DragTransition::Released { .. }
        ));
        // Released pointer is not repeated in later frames.
        assert!(drag.begin_frame(2).is_none());
    }

    #[test]
    fn release_without_a_preceding_move_uses_the_last_known_position() {
        let mut drag = FrameAlignedDrag::new();
        drag.feed_pointer_at(
            MOUSE_POINTER,
            PointerPosition { x: 9.0, y: 8.0 },
            &button(PointerButton::Primary, ButtonState::Pressed),
        );
        drag.feed_pointer_at(
            MOUSE_POINTER,
            PointerPosition { x: 11.0, y: 12.0 },
            &button(PointerButton::Primary, ButtonState::Released),
        );
        let frame = drag.begin_frame(0).expect("frame");
        assert_eq!(
            frame.transitions[0].position(),
            PointerPosition { x: 9.0, y: 8.0 }
        );
        assert_eq!(
            frame.transitions[1].position(),
            PointerPosition { x: 11.0, y: 12.0 }
        );
    }

    #[test]
    fn capture_is_retained_during_the_drag_and_flushed_with_the_release() {
        let mut drag = FrameAlignedDrag::new();
        let target = ElementPath::root(1).with_child(2);
        drag.set_capture(MOUSE_POINTER, Some(target.clone()));
        drag.feed(&moved(1.0, 1.0));
        drag.feed(&button(PointerButton::Primary, ButtonState::Pressed));
        drag.feed(&moved(50.0, 50.0));
        let frame = drag.begin_frame(1).expect("frame");
        assert_eq!(frame.pointers[0].capture, Some(target.clone()));
        assert_eq!(drag.capture(MOUSE_POINTER), Some(&target));

        drag.feed(&button(PointerButton::Primary, ButtonState::Released));
        let frame = drag.begin_frame(2).expect("release frame");
        let DragTransition::Released {
            capture, position, ..
        } = &frame.transitions[0]
        else {
            panic!("expected release transition");
        };
        assert_eq!(capture, &Some(target));
        assert_eq!(*position, PointerPosition { x: 50.0, y: 50.0 });
        assert_eq!(drag.capture(MOUSE_POINTER), None);
    }

    #[test]
    fn cancellation_clears_active_drag_with_a_reason() {
        for (event, reason) in [
            (EngineEvent::PointerLeft, DragCancelReason::PointerLeft),
            (
                EngineEvent::FocusChanged(false),
                DragCancelReason::FocusLost,
            ),
            (EngineEvent::Suspended, DragCancelReason::Suspended),
        ] {
            let mut drag = FrameAlignedDrag::new();
            drag.feed(&moved(2.0, 3.0));
            drag.feed(&button(PointerButton::Primary, ButtonState::Pressed));
            drag.feed(&moved(4.0, 5.0));
            assert!(drag.feed(&event));
            let frame = drag.begin_frame(1).expect("cancellation frame");
            assert_eq!(frame.transitions.len(), 2);
            assert!(matches!(
                frame.transitions[1],
                DragTransition::Cancelled {
                    reason: found,
                    position,
                    ..
                } if found == reason && position == PointerPosition { x: 4.0, y: 5.0 }
            ));
            assert_eq!(drag.active_pointer_count(), 0);
            // Focus gained is not a cancellation.
            let mut drag = FrameAlignedDrag::new();
            assert!(!drag.feed(&EngineEvent::FocusChanged(true)));
        }
    }

    #[test]
    fn independent_pointers_keep_their_own_state_and_release() {
        let first = PointerId::new(1);
        let second = PointerId::new(2);
        let mut drag = FrameAlignedDrag::new();
        let first_target = ElementPath::root(0);
        let second_target = ElementPath::root(1).with_child(0);
        drag.set_capture(first, Some(first_target.clone()));
        drag.set_capture(second, Some(second_target.clone()));
        drag.feed_pointer_at(
            first,
            PointerPosition { x: 1.0, y: 1.0 },
            &button(PointerButton::Primary, ButtonState::Pressed),
        );
        drag.feed_pointer_at(
            second,
            PointerPosition { x: 9.0, y: 9.0 },
            &button(PointerButton::Primary, ButtonState::Pressed),
        );
        drag.feed_pointer(first, &moved(4.0, 4.0));
        drag.feed_pointer(second, &moved(40.0, 40.0));
        let frame = drag.begin_frame(1).expect("frame");
        assert_eq!(frame.pointers.len(), 2);
        assert_eq!(
            frame.pointer(first).unwrap().position,
            PointerPosition { x: 4.0, y: 4.0 }
        );
        assert_eq!(
            frame.pointer(second).unwrap().position,
            PointerPosition { x: 40.0, y: 40.0 }
        );
        assert_eq!(frame.pointer(first).unwrap().capture, Some(first_target));
        assert_eq!(frame.pointer(second).unwrap().capture, Some(second_target));

        drag.feed_pointer(
            first,
            &button(PointerButton::Primary, ButtonState::Released),
        );
        let frame = drag.begin_frame(2).expect("release frame");
        assert_eq!(frame.transitions.len(), 1);
        assert!(!frame.pointer(first).unwrap().active);
        assert!(frame.pointer(second).unwrap().active);
        assert!(drag.is_pointer_active(second));
        assert!(!drag.is_pointer_active(first));
    }

    #[test]
    fn wheel_deltas_accumulate_per_frame_instead_of_dropping_samples() {
        let mut drag = FrameAlignedDrag::new();
        drag.feed(&EngineEvent::Scroll {
            delta: ScrollDelta::Lines { x: 1.0, y: 2.0 },
            modifiers: Default::default(),
        });
        drag.feed(&EngineEvent::Scroll {
            delta: ScrollDelta::Lines { x: 0.5, y: -1.0 },
            modifiers: Default::default(),
        });
        drag.feed(&EngineEvent::Scroll {
            delta: ScrollDelta::Pixels { x: 16.0, y: 32.0 },
            modifiers: Default::default(),
        });
        let frame = drag.begin_frame(1).expect("frame");
        assert_eq!(frame.wheel_lines, (1.5, 1.0));
        assert_eq!(frame.wheel_pixels, (16.0, 32.0));
        assert!(drag.begin_frame(1).is_none());
        let frame = drag.begin_frame(2);
        assert!(frame.is_none());
    }

    #[test]
    fn press_release_order_is_preserved() {
        let mut drag = FrameAlignedDrag::new();
        drag.feed_pointer_at(
            MOUSE_POINTER,
            PointerPosition { x: 1.0, y: 1.0 },
            &button(PointerButton::Primary, ButtonState::Pressed),
        );
        drag.feed_pointer_at(
            MOUSE_POINTER,
            PointerPosition { x: 2.0, y: 2.0 },
            &button(PointerButton::Secondary, ButtonState::Pressed),
        );
        drag.feed_pointer_at(
            MOUSE_POINTER,
            PointerPosition { x: 3.0, y: 3.0 },
            &button(PointerButton::Primary, ButtonState::Released),
        );
        let frame = drag.begin_frame(1).expect("frame");
        let order: Vec<_> = frame
            .transitions
            .iter()
            .map(|transition| match transition {
                DragTransition::Pressed { button, .. } => (true, *button),
                DragTransition::Released { button, .. } => (false, *button),
                DragTransition::Cancelled { .. } => panic!("unexpected cancellation"),
            })
            .collect();
        assert_eq!(
            order,
            vec![
                (true, PointerButton::Primary),
                (true, PointerButton::Secondary),
                (false, PointerButton::Primary),
            ]
        );
        assert_eq!(frame.pointers[0].buttons, vec![PointerButton::Secondary]);
    }

    #[test]
    fn pending_state_stays_bounded_under_a_large_burst() {
        let mut drag = FrameAlignedDrag::new();
        // Far more motion than any device can produce between frames.
        let mut x = 0.0;
        for _ in 0..1_000_000 {
            x += 1.0;
            drag.feed(&moved(x, x));
        }
        assert_eq!(drag.tracked_pointer_count(), 1);
        assert_eq!(drag.pending_transition_count(), 0);
        // Idle motion only remembers the latest position; it does not queue a
        // frame request or any sample history.
        assert!(!drag.has_pending());

        drag.feed(&button(PointerButton::Primary, ButtonState::Pressed));
        assert!(drag.has_pending());
        let mut transitions = 0_u32;
        for _ in 0..10_000 {
            drag.feed(&button(PointerButton::Primary, ButtonState::Pressed));
            drag.feed(&button(PointerButton::Primary, ButtonState::Released));
            transitions += 2;
        }
        assert!(transitions > MAX_PENDING_TRANSITIONS as u32);
        // Stored edges never exceed the fixed ring.
        assert!(drag.pending_transition_count() <= MAX_PENDING_TRANSITIONS);
        let frame = drag.begin_frame(1).expect("overflow frame");
        assert!(frame.overflowed);
        assert!(frame.transitions.len() <= MAX_PENDING_TRANSITIONS);
        assert!(drag.tracked_pointer_count() <= MAX_ACTIVE_POINTERS);
        assert!(!drag.has_pending());
    }

    #[test]
    fn many_pointers_overflow_fail_safely() {
        let mut drag = FrameAlignedDrag::new();
        for id in 0..(MAX_ACTIVE_POINTERS as u64 + 4) {
            drag.feed_pointer_at(
                PointerId::new(id),
                PointerPosition {
                    x: id as f32,
                    y: 0.0,
                },
                &button(PointerButton::Primary, ButtonState::Pressed),
            );
        }
        let frame = drag.begin_frame(1).expect("frame");
        assert!(frame.overflowed);
        assert_eq!(drag.active_pointer_count(), 0);
    }

    #[test]
    fn reset_drops_pending_state() {
        let mut drag = FrameAlignedDrag::new();
        drag.feed(&moved(1.0, 1.0));
        drag.feed(&button(PointerButton::Primary, ButtonState::Pressed));
        drag.reset();
        assert!(!drag.has_pending());
        assert!(drag.begin_frame(1).is_none());
    }
}
