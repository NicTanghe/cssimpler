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

#[cfg(test)]
mod tests {
    use super::*;

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
