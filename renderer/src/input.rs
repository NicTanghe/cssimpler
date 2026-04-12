#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyboardModifiers {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    pub super_key: bool,
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
