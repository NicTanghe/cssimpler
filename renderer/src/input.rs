use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyboardModifiers {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    pub super_key: bool,
}

/// Leave window-management pointer chords to the OS. An explicit host override
/// remains effective in fullscreen, where ordinary modifier chords belong to UI.
pub(crate) fn system_pointer_gesture(
    windowed: bool,
    modifiers: KeyboardModifiers,
    override_active: bool,
) -> bool {
    let window_chord = if cfg!(target_os = "macos") {
        modifiers.control && modifiers.super_key
    } else {
        modifiers.super_key
    };
    override_active || (windowed && window_chord)
}

pub(crate) fn system_pointer_event(
    event: &EngineEvent,
    windowed: bool,
    override_active: bool,
) -> Option<KeyboardModifiers> {
    let modifiers = match event {
        EngineEvent::ModifiersChanged(modifiers)
        | EngineEvent::PointerMoved { modifiers, .. }
        | EngineEvent::PointerButton { modifiers, .. }
        | EngineEvent::Scroll { modifiers, .. } => *modifiers,
        _ => return None,
    };
    system_pointer_gesture(windowed, modifiers, override_active).then_some(modifiers)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonState {
    Pressed,
    Released,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerButton {
    Primary,
    Secondary,
    Middle,
    Back,
    Forward,
    Other(u16),
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PointerPosition {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScrollDelta {
    Lines { x: f32, y: f32 },
    Pixels { x: f32, y: f32 },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeyLocation {
    #[default]
    Standard,
    Left,
    Right,
    Numpad,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyIdentity {
    Named(String),
    Character(String),
    Dead(Option<char>),
    Unidentified(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyboardEvent {
    pub logical_key: KeyIdentity,
    pub physical_key: Option<String>,
    pub location: KeyLocation,
    pub state: ButtonState,
    pub repeat: bool,
    pub modifiers: KeyboardModifiers,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextInputEvent {
    Commit(String),
    Preedit {
        text: String,
        cursor: Option<(usize, usize)>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportEvent {
    pub width: usize,
    pub height: usize,
    pub scale_factor: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum EngineEvent {
    ViewportChanged(ViewportEvent),
    FocusChanged(bool),
    ModifiersChanged(KeyboardModifiers),
    PointerMoved {
        position: PointerPosition,
        modifiers: KeyboardModifiers,
    },
    PointerLeft,
    PointerButton {
        button: PointerButton,
        state: ButtonState,
        modifiers: KeyboardModifiers,
    },
    Scroll {
        delta: ScrollDelta,
        modifiers: KeyboardModifiers,
    },
    Key(KeyboardEvent),
    TextInput(TextInputEvent),
    Suspended,
    Resumed,
}

/// How a [`crate::SceneProvider`] wants pointer-driven redraws scheduled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InputRedrawPolicy {
    /// Every consumed or unconsumed pointer/work event may request a paint.
    /// This is the historical behavior and remains the default.
    #[default]
    Immediate,
    /// Keep delivering raw pointer events immediately, but coalesce the paints
    /// they request to at most one per frame interval. Providers that consume
    /// the latest drag state once per frame opt in with this policy.
    FrameAligned,
}

/// Result of [`FrameInputCadence::schedule`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputRedrawDecision {
    /// Paint as soon as the event loop can service a redraw.
    Immediate,
    /// The first paint after this instant will observe the latest input.
    WaitUntil(Instant),
}

/// Small, allocation-free cadence helper for frame-aligned pointer input.
///
/// The runtime records each displayed frame with [`observe_frame`](Self::observe_frame).
/// Pointer motion and wheel input call [`schedule`](Self::schedule): input that
/// arrives while the app is idle (no frame within `frame_time`) paints
/// immediately, input inside the current frame window defers to the frame
/// boundary. [`take_due`](Self::take_due) lets an idle event loop wake exactly
/// when the deferred paint is due.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameInputCadence {
    last_frame_at: Option<Instant>,
    deadline: Option<Instant>,
}

impl FrameInputCadence {
    pub const fn new() -> Self {
        Self {
            last_frame_at: None,
            deadline: None,
        }
    }

    /// Record that a frame was displayed at `now`. A displayed frame already
    /// observed the latest input, so any pending coalesced wake is no longer due.
    pub fn observe_frame(&mut self, now: Instant) {
        self.last_frame_at = Some(now);
        self.deadline = None;
    }

    /// Decide when input that arrived at `now` needs a paint.
    pub fn schedule(&mut self, now: Instant, frame_time: Duration) -> InputRedrawDecision {
        let boundary = self
            .last_frame_at
            .and_then(|last| last.checked_add(frame_time));
        match boundary {
            Some(deadline) if now < deadline => {
                self.deadline = Some(
                    self.deadline
                        .map_or(deadline, |pending| pending.min(deadline)),
                );
                InputRedrawDecision::WaitUntil(deadline)
            }
            _ => {
                self.deadline = None;
                InputRedrawDecision::Immediate
            }
        }
    }

    /// The instant of the pending deferred paint, if input arrived mid-frame.
    pub fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    /// Consume the pending wake when `now` has reached it.
    pub fn take_due(&mut self, now: Instant) -> bool {
        if self.deadline.is_some_and(|deadline| now >= deadline) {
            self.deadline = None;
            true
        } else {
            false
        }
    }

    /// Drop a pending wake without painting.
    pub fn clear_deadline(&mut self) {
        self.deadline = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_input_cadence_immediate_then_deferred_then_due() {
        let start = Instant::now();
        let frame_time = Duration::from_millis(16);
        let mut cadence = FrameInputCadence::new();

        // Nothing displayed yet: first input paints immediately.
        assert_eq!(
            cadence.schedule(start, frame_time),
            InputRedrawDecision::Immediate
        );
        assert_eq!(cadence.deadline(), None);

        cadence.observe_frame(start);
        assert_eq!(cadence.deadline(), None);

        // Input 4 ms into a 16 ms frame waits for the boundary.
        let mid = start + Duration::from_millis(4);
        assert_eq!(
            cadence.schedule(mid, frame_time),
            InputRedrawDecision::WaitUntil(start + frame_time)
        );
        // A second sample in the same frame keeps the same deadline (one paint).
        assert_eq!(
            cadence.schedule(start + Duration::from_millis(12), frame_time),
            InputRedrawDecision::WaitUntil(start + frame_time)
        );

        assert!(!cadence.take_due(start + Duration::from_millis(15)));
        assert!(cadence.take_due(start + frame_time));
        assert_eq!(cadence.deadline(), None);
        assert!(!cadence.take_due(start + frame_time));

        // A new displayed frame starts a new window: input inside it defers to
        // the next boundary, and input after the window paints immediately.
        cadence.observe_frame(start + frame_time);
        assert_eq!(
            cadence.schedule(start + frame_time + Duration::from_millis(1), frame_time),
            InputRedrawDecision::WaitUntil(start + frame_time * 2)
        );
        assert_eq!(
            cadence.schedule(start + frame_time * 2, frame_time),
            InputRedrawDecision::Immediate
        );
    }

    #[test]
    fn frame_input_cadence_clears_pending_wake_when_a_frame_is_displayed() {
        let start = Instant::now();
        let frame_time = Duration::from_millis(16);
        let mut cadence = FrameInputCadence::new();
        cadence.observe_frame(start);
        assert!(matches!(
            cadence.schedule(start + Duration::from_millis(2), frame_time),
            InputRedrawDecision::WaitUntil(_)
        ));
        assert!(cadence.deadline().is_some());
        cadence.observe_frame(start + Duration::from_millis(8));
        assert_eq!(cadence.deadline(), None);
        assert!(!cadence.take_due(start + frame_time));
    }

    #[test]
    fn zero_frame_time_never_defers() {
        let start = Instant::now();
        let mut cadence = FrameInputCadence::new();
        cadence.observe_frame(start);
        assert_eq!(
            cadence.schedule(start, Duration::ZERO),
            InputRedrawDecision::Immediate
        );
    }

    #[test]
    fn window_management_modifiers_yield_only_windowed_pointer_input() {
        for modifiers in [KeyboardModifiers {
            super_key: true,
            control: cfg!(target_os = "macos"),
            ..Default::default()
        }] {
            assert!(system_pointer_gesture(true, modifiers, false));
            assert!(!system_pointer_gesture(false, modifiers, false));
        }
        for modifiers in [
            KeyboardModifiers::default(),
            KeyboardModifiers {
                shift: true,
                alt: true,
                ..Default::default()
            },
        ] {
            assert!(!system_pointer_gesture(true, modifiers, false));
            assert!(system_pointer_gesture(false, modifiers, true));
        }
        #[cfg(target_os = "macos")]
        for modifiers in [
            KeyboardModifiers {
                control: true,
                ..Default::default()
            },
            KeyboardModifiers {
                super_key: true,
                ..Default::default()
            },
        ] {
            assert!(
                !system_pointer_gesture(true, modifiers, false),
                "individual modifiers must not disable ordinary app input"
            );
        }
        let key = EngineEvent::Key(KeyboardEvent {
            logical_key: KeyIdentity::Character("c".into()),
            physical_key: None,
            location: KeyLocation::Standard,
            state: ButtonState::Pressed,
            repeat: false,
            modifiers: KeyboardModifiers {
                control: true,
                super_key: true,
                ..Default::default()
            },
        });
        assert_eq!(system_pointer_event(&key, true, false), None);
    }
}
