use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use anyhow::Result;
use cssimpler::app::{App, Invalidation, RenderMode};
use cssimpler::core::{
    Color, CornerRadius, ElementInteractionState, ElementPath, Insets, LengthPercentageValue, Node,
    RenderKind, RenderNode, Style, Transform2D, TransformMatrix3d, TransformOperation,
    TransformStyleMode,
};
use cssimpler::renderer::{FrameInfo, SceneProvider, ViewportSize, WindowConfig, render_to_buffer};
use cssimpler::style::{Stylesheet, build_render_tree_in_viewport, parse_stylesheet};

const SLIDER_STEPS: usize = 9;
const CONTROL_PANEL_INDEX: usize = 1;
const CONTROL_ADJUSTER_INDEX: usize = 1;
const CONTROL_MINUS_INDEX: usize = 0;
const CONTROL_TRACK_INDEX: usize = 1;
const CONTROL_PLUS_INDEX: usize = 2;
const ACTION_NONE: u64 = 0;
const ACTION_SELECT_BASE: u64 = 1;
const ACTION_SELECT_COUNT: u64 = CONTROL_COUNT as u64 * SLIDER_STEPS as u64;
const ACTION_DECREMENT_BASE: u64 = ACTION_SELECT_BASE + ACTION_SELECT_COUNT;
const ACTION_INCREMENT_BASE: u64 = ACTION_DECREMENT_BASE + CONTROL_COUNT as u64;
const ACTION_RESET: u64 = ACTION_INCREMENT_BASE + CONTROL_COUNT as u64;

static PENDING_ACTION: AtomicU64 = AtomicU64::new(ACTION_NONE);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ControlId {
    Perspective,
    TranslateX,
    TranslateY,
    TranslateZ,
    RotateX,
    RotateY,
    RotateZ,
    ScaleX,
    ScaleY,
    ScaleZ,
}

impl ControlId {
    const ALL: [Self; 10] = [
        Self::Perspective,
        Self::TranslateX,
        Self::TranslateY,
        Self::TranslateZ,
        Self::RotateX,
        Self::RotateY,
        Self::RotateZ,
        Self::ScaleX,
        Self::ScaleY,
        Self::ScaleZ,
    ];

    fn index(self) -> usize {
        match self {
            Self::Perspective => 0,
            Self::TranslateX => 1,
            Self::TranslateY => 2,
            Self::TranslateZ => 3,
            Self::RotateX => 4,
            Self::RotateY => 5,
            Self::RotateZ => 6,
            Self::ScaleX => 7,
            Self::ScaleY => 8,
            Self::ScaleZ => 9,
        }
    }

    fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    fn label(self) -> &'static str {
        match self {
            Self::Perspective => "Perspective",
            Self::TranslateX => "Translate X",
            Self::TranslateY => "Translate Y",
            Self::TranslateZ => "Translate Z",
            Self::RotateX => "Rotate X",
            Self::RotateY => "Rotate Y",
            Self::RotateZ => "Rotate Z",
            Self::ScaleX => "Scale X",
            Self::ScaleY => "Scale Y",
            Self::ScaleZ => "Scale Z",
        }
    }

    fn min(self) -> f32 {
        match self {
            Self::Perspective => 300.0,
            Self::TranslateX | Self::TranslateY => -140.0,
            Self::TranslateZ => -220.0,
            Self::RotateX | Self::RotateY | Self::RotateZ => -60.0,
            Self::ScaleX | Self::ScaleY | Self::ScaleZ => 0.5,
        }
    }

    fn max(self) -> f32 {
        match self {
            Self::Perspective => 1600.0,
            Self::TranslateX | Self::TranslateY => 140.0,
            Self::TranslateZ => 220.0,
            Self::RotateX | Self::RotateY | Self::RotateZ => 60.0,
            Self::ScaleX | Self::ScaleY | Self::ScaleZ => 1.5,
        }
    }

    fn default_step(self) -> usize {
        match self {
            Self::Perspective
            | Self::TranslateX
            | Self::TranslateY
            | Self::TranslateZ
            | Self::RotateX
            | Self::RotateY
            | Self::RotateZ
            | Self::ScaleX
            | Self::ScaleY
            | Self::ScaleZ => SLIDER_STEPS / 2,
        }
    }

    fn format_value(self, value: f32) -> String {
        match self {
            Self::Perspective => format!("{value:.0}px"),
            Self::TranslateX | Self::TranslateY | Self::TranslateZ => format!("{value:.0}px"),
            Self::RotateX | Self::RotateY | Self::RotateZ => format!("{value:.0}deg"),
            Self::ScaleX | Self::ScaleY | Self::ScaleZ => format!("{value:.2}x"),
        }
    }
}

const CONTROL_COUNT: usize = ControlId::ALL.len();

#[derive(Clone, Debug, PartialEq)]
struct PlaygroundState {
    steps: [usize; CONTROL_COUNT],
}

impl Default for PlaygroundState {
    fn default() -> Self {
        let mut steps = [0; CONTROL_COUNT];
        for control in ControlId::ALL {
            steps[control.index()] = control.default_step();
        }
        Self { steps }
    }
}

impl PlaygroundState {
    fn step(&self, control: ControlId) -> usize {
        self.steps[control.index()]
    }

    fn set_step(&mut self, control: ControlId, step: usize) {
        self.steps[control.index()] = step.min(SLIDER_STEPS.saturating_sub(1));
    }

    fn step_by_delta(&mut self, control: ControlId, delta: isize) {
        let current = self.step(control) as isize;
        let next = (current + delta).clamp(0, SLIDER_STEPS.saturating_sub(1) as isize) as usize;
        self.set_step(control, next);
    }

    fn value(&self, control: ControlId) -> f32 {
        value_from_step(control, self.step(control))
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

type PlaygroundApp = App<
    'static,
    PlaygroundState,
    fn(&mut PlaygroundState, FrameInfo) -> Invalidation,
    fn(&PlaygroundState) -> Node,
>;

struct PlaygroundProvider {
    app: PlaygroundApp,
    scene: Vec<RenderNode>,
}

impl PlaygroundProvider {
    fn new() -> Self {
        set_hovered_path(None);
        set_active_path(None);
        Self {
            app: App::new(
                PlaygroundState::default(),
                stylesheet(),
                update as fn(&mut PlaygroundState, FrameInfo) -> Invalidation,
                build_ui as fn(&PlaygroundState) -> Node,
            )
            .with_render_mode(RenderMode::OnInvalidation),
            scene: Vec::new(),
        }
    }
}

impl SceneProvider for PlaygroundProvider {
    fn update(&mut self, frame: FrameInfo) {
        <PlaygroundApp as SceneProvider>::update(&mut self.app, frame);
    }

    fn scene(&self) -> &[RenderNode] {
        &self.scene
    }

    fn capture_scene(&mut self) -> Vec<RenderNode> {
        let scene = <PlaygroundApp as SceneProvider>::capture_scene(&mut self.app);
        self.scene = scene.clone();
        scene
    }

    fn set_viewport(&mut self, viewport: ViewportSize) {
        <PlaygroundApp as SceneProvider>::set_viewport(&mut self.app, viewport);
    }

    fn set_element_interaction(&mut self, interaction: ElementInteractionState) -> bool {
        set_hovered_path(interaction.hovered.clone());
        set_active_path(interaction.active.clone());
        <PlaygroundApp as SceneProvider>::set_element_interaction(&mut self.app, interaction)
    }

    fn handle_engine_event(&mut self, event: &cssimpler::renderer::EngineEvent) -> bool {
        <PlaygroundApp as SceneProvider>::handle_engine_event(&mut self.app, event)
    }

    fn redraw_schedule(&self) -> cssimpler::renderer::RedrawSchedule {
        <PlaygroundApp as SceneProvider>::redraw_schedule(&self.app)
    }

    fn needs_redraw(&self) -> bool {
        <PlaygroundApp as SceneProvider>::needs_redraw(&self.app)
    }
}

fn main() -> Result<()> {
    let config = WindowConfig::new("cssimpler / 3d transform playground", 1440, 980);
    cssimpler::renderer::run_with_scene_provider(config, PlaygroundProvider::new())
        .map_err(Into::into)
}

fn update(state: &mut PlaygroundState, _frame: FrameInfo) -> Invalidation {
    match decode_action(PENDING_ACTION.swap(ACTION_NONE, Ordering::Relaxed)) {
        Some(Action::Select { control, step }) => {
            state.set_step(control, step);
            Invalidation::Paint
        }
        Some(Action::Nudge { control, delta }) => {
            state.step_by_delta(control, delta);
            Invalidation::Paint
        }
        Some(Action::Reset) => {
            state.reset();
            Invalidation::Paint
        }
        None => Invalidation::Clean,
    }
}

fn build_ui(state: &PlaygroundState) -> Node {
    Node::element("div")
        .with_id("app")
        .with_children([build_stage_panel(state), build_controls_panel(state)])
        .into()
}

fn build_stage_panel(state: &PlaygroundState) -> Node {
    Node::element("section")
        .with_class("stage-panel")
        .with_children([build_stage_copy(state), build_stage_shell(state)])
        .into()
}

fn build_stage_copy(state: &PlaygroundState) -> Node {
    Node::element("div")
        .with_class("stage-copy")
        .with_children([
            text_node("p", "eyebrow", "Example / 3D transform playground"),
            text_node("h1", "stage-title", "A centered card with a true pivot reference"),
            text_node(
                "p",
                "stage-note",
                "The transformed card now sits on top of a static ghost outline, and the centered depth stack includes an inline SVG layer so we can check 3D transforms on vector content too.",
            ),
            text_node("p", "transform-order", "Order: translate -> rotate -> scale"),
            text_node("p", "transform-readout", transform_readout(state)),
            Node::element("button")
                .with_class("reset-button")
                .with_attribute("type", "button")
                .on_click(reset_transforms)
                .with_child(Node::text("Reset all transforms"))
                .into(),
        ])
        .into()
}

fn build_stage_shell(state: &PlaygroundState) -> Node {
    Node::element("div")
        .with_class("stage-shell")
        .with_child(
            Node::element("div")
                .with_class("stage-frame")
                .with_child(
                    Node::element("div")
                        .with_class("projection-plane")
                        .with_style(perspective_style(state))
                        .with_child(
                            Node::element("div")
                                .with_class("playground-anchor")
                                .with_children([
                                    Node::element("div").with_class("ghost-card").into(),
                                    build_card(state),
                                ])
                                .into(),
                        )
                        .into(),
                )
                .into(),
        )
        .into()
}

fn build_card(state: &PlaygroundState) -> Node {
    Node::element("div")
        .with_class("play-card")
        .with_style(card_style(state))
        .with_children([
            Node::element("div")
                .with_class("card-copy")
                .with_children([
                    text_node("p", "card-kicker", "Centered test card"),
                    text_node("h2", "card-title", "Transform Debug"),
                    text_node(
                        "p",
                        "card-text",
                        "A static ghost outline and a lifted SVG badge make it easier to spot pivot, clipping, and vector transform issues.",
                    ),
                ])
                .into(),
            Node::element("div")
                .with_class("card-guide-frame")
                .into(),
            Node::element("div")
                .with_class("card-guide-horizontal")
                .into(),
            Node::element("div")
                .with_class("card-guide-vertical")
                .into(),
            Node::element("div")
                .with_class("card-guide-center-dot")
                .into(),
            Node::element("div")
                .with_class("card-depth-stack")
                .with_children([
                    Node::element("div")
                        .with_class("depth-chip")
                        .with_class("depth-chip-one")
                        .into(),
                    Node::element("div")
                        .with_class("depth-chip")
                        .with_class("depth-chip-two")
                        .into(),
                    Node::element("div")
                        .with_class("depth-chip")
                        .with_class("depth-chip-three")
                        .into(),
                    build_depth_svg(),
                ])
                .into(),
        ])
        .into()
}

fn build_depth_svg() -> Node {
    Node::element("div")
        .with_class("depth-svg-frame")
        .with_child(
            Node::element("svg")
                .with_class("depth-svg")
                .with_attribute("xmlns", "http://www.w3.org/2000/svg")
                .with_attribute("viewBox", "0 0 100 100")
                .with_children([
                    Node::element("path")
                        .with_class("depth-svg-face")
                        .with_class("depth-svg-face-back")
                        .with_attribute("d", "M50 12 L82 30 L50 48 L18 30 Z")
                        .into(),
                    Node::element("path")
                        .with_class("depth-svg-face")
                        .with_class("depth-svg-face-left")
                        .with_attribute("d", "M18 30 L50 48 L50 86 L18 68 Z")
                        .into(),
                    Node::element("path")
                        .with_class("depth-svg-face")
                        .with_class("depth-svg-face-right")
                        .with_attribute("d", "M82 30 L50 48 L50 86 L82 68 Z")
                        .into(),
                    Node::element("path")
                        .with_class("depth-svg-line")
                        .with_attribute("d", "M50 12 L82 30 L82 68 L50 86 L18 68 L18 30 Z")
                        .into(),
                    Node::element("path")
                        .with_class("depth-svg-line")
                        .with_attribute("d", "M18 30 L50 48 L82 30")
                        .into(),
                    Node::element("path")
                        .with_class("depth-svg-line")
                        .with_attribute("d", "M50 48 L50 86")
                        .into(),
                ])
                .into(),
        )
        .into()
}

fn build_controls_panel(state: &PlaygroundState) -> Node {
    Node::element("section")
        .with_class("controls-panel")
        .with_children(
            ControlId::ALL
                .into_iter()
                .map(|control| build_slider_row(control, state)),
        )
        .into()
}

fn build_slider_row(control: ControlId, state: &PlaygroundState) -> Node {
    Node::element("div")
        .with_class("slider-row")
        .with_children([
            Node::element("div")
                .with_class("slider-meta")
                .with_children([
                    text_node("p", "slider-name", control.label()),
                    text_node(
                        "p",
                        "slider-value",
                        control.format_value(state.value(control)),
                    ),
                ])
                .into(),
            Node::element("div")
                .with_class("slider-adjuster")
                .with_children([
                    build_nudge_button(control, -1),
                    Node::element("div")
                        .with_class("slider-track")
                        .with_children(
                            (0..SLIDER_STEPS)
                                .map(|step| build_slider_stop(control, step, state.step(control))),
                        )
                        .into(),
                    build_nudge_button(control, 1),
                ])
                .into(),
        ])
        .into()
}

fn build_nudge_button(control: ControlId, delta: isize) -> Node {
    let (class_name, label, handler) = if delta < 0 {
        (
            "slider-stepper slider-stepper-down",
            "-",
            step_slider_down as fn(),
        )
    } else {
        (
            "slider-stepper slider-stepper-up",
            "+",
            step_slider_up as fn(),
        )
    };

    Node::element("button")
        .with_class(class_name)
        .with_attribute("type", "button")
        .with_attribute(
            "aria-label",
            format!(
                "{} {}",
                if delta < 0 { "Decrease" } else { "Increase" },
                control.label()
            ),
        )
        .with_attribute("data-control", control.index().to_string())
        .with_attribute("data-delta", delta.to_string())
        .on_click(handler)
        .with_child(Node::text(label))
        .into()
}

fn build_slider_stop(control: ControlId, step: usize, active_step: usize) -> Node {
    let value = control.format_value(value_from_step(control, step));
    let mut stop = Node::element("button")
        .with_class("slider-stop")
        .with_attribute("type", "button")
        .with_attribute(
            "aria-label",
            format!("Set {} to {}", control.label(), value),
        )
        .with_attribute("data-control", control.index().to_string())
        .with_attribute("data-step", step.to_string())
        .on_mousedown(select_slider_stop)
        .on_mouseenter(drag_slider_stop)
        .on_click(select_slider_stop);

    if step == active_step {
        stop = stop.with_class("slider-stop-active");
    }
    if step == control.default_step() {
        stop = stop.with_class("slider-stop-home");
    }

    stop.into()
}

fn text_node(tag: &str, class_name: &str, text: impl Into<String>) -> Node {
    Node::element(tag)
        .with_class(class_name)
        .with_child(Node::text(text.into()))
        .into()
}

fn perspective_style(state: &PlaygroundState) -> Style {
    let mut style = Style::default();
    style.visual.perspective = Some(state.value(ControlId::Perspective));
    style.visual.transform_style = TransformStyleMode::Preserve3d;
    style
}

fn card_style(state: &PlaygroundState) -> Style {
    let mut style = Style::default();
    style.visual.transform_style = TransformStyleMode::Preserve3d;
    style.visual.transform = Transform2D {
        operations: vec![
            TransformOperation::Translate {
                x: LengthPercentageValue::from_px(state.value(ControlId::TranslateX)),
                y: LengthPercentageValue::from_px(state.value(ControlId::TranslateY)),
            },
            TransformOperation::TranslateZ {
                z: state.value(ControlId::TranslateZ),
            },
            TransformOperation::RotateX {
                degrees: state.value(ControlId::RotateX),
            },
            TransformOperation::RotateY {
                degrees: state.value(ControlId::RotateY),
            },
            TransformOperation::RotateZ {
                degrees: state.value(ControlId::RotateZ),
            },
            TransformOperation::Matrix3d {
                matrix: TransformMatrix3d::scale(
                    state.value(ControlId::ScaleX),
                    state.value(ControlId::ScaleY),
                    state.value(ControlId::ScaleZ),
                ),
            },
        ],
        ..Transform2D::default()
    };
    style.visual.background = Some(Color::rgb(16, 185, 129));
    style.visual.corner_radius = CornerRadius::all(28.0);
    style.visual.shadows = vec![cssimpler::core::BoxShadow {
        color: Color::rgba(4, 47, 46, 96),
        offset_x: 0.0,
        offset_y: 26.0,
        blur_radius: 34.0,
        spread: -18.0,
    }];
    style.visual.border.color = Color::rgba(226, 232, 240, 44);
    style.visual.border.widths = Insets::all(1.0);
    style
}

fn transform_readout(state: &PlaygroundState) -> String {
    format!(
        "perspective({:.0}px) | transform: translate3d({:.0}px, {:.0}px, {:.0}px) rotateX({:.0}deg) rotateY({:.0}deg) rotateZ({:.0}deg) scale3d({:.2}, {:.2}, {:.2})",
        state.value(ControlId::Perspective),
        state.value(ControlId::TranslateX),
        state.value(ControlId::TranslateY),
        state.value(ControlId::TranslateZ),
        state.value(ControlId::RotateX),
        state.value(ControlId::RotateY),
        state.value(ControlId::RotateZ),
        state.value(ControlId::ScaleX),
        state.value(ControlId::ScaleY),
        state.value(ControlId::ScaleZ),
    )
}

fn value_from_step(control: ControlId, step: usize) -> f32 {
    let clamped = step.min(SLIDER_STEPS.saturating_sub(1));
    let t = if SLIDER_STEPS <= 1 {
        0.0
    } else {
        clamped as f32 / (SLIDER_STEPS - 1) as f32
    };
    control.min() + (control.max() - control.min()) * t
}

fn select_slider_stop() {
    let Some((control, step)) = hovered_slider_stop() else {
        return;
    };
    PENDING_ACTION.store(encode_select_action(control, step), Ordering::Relaxed);
}

fn drag_slider_stop() {
    let Some((hovered_control, hovered_step)) = hovered_slider_stop() else {
        return;
    };
    let Some((active_control, _)) = active_slider_stop() else {
        return;
    };
    if hovered_control != active_control {
        return;
    }

    PENDING_ACTION.store(
        encode_select_action(hovered_control, hovered_step),
        Ordering::Relaxed,
    );
}

fn step_slider_down() {
    let Some(control) = hovered_nudge_control(CONTROL_MINUS_INDEX) else {
        return;
    };
    PENDING_ACTION.store(encode_nudge_action(control, -1), Ordering::Relaxed);
}

fn step_slider_up() {
    let Some(control) = hovered_nudge_control(CONTROL_PLUS_INDEX) else {
        return;
    };
    PENDING_ACTION.store(encode_nudge_action(control, 1), Ordering::Relaxed);
}

fn reset_transforms() {
    PENDING_ACTION.store(ACTION_RESET, Ordering::Relaxed);
}

fn encode_select_action(control: ControlId, step: usize) -> u64 {
    ACTION_SELECT_BASE + control.index() as u64 * SLIDER_STEPS as u64 + step as u64
}

fn encode_nudge_action(control: ControlId, delta: isize) -> u64 {
    let base = if delta < 0 {
        ACTION_DECREMENT_BASE
    } else {
        ACTION_INCREMENT_BASE
    };
    base + control.index() as u64
}

enum Action {
    Select { control: ControlId, step: usize },
    Nudge { control: ControlId, delta: isize },
    Reset,
}

fn decode_action(action: u64) -> Option<Action> {
    if action == ACTION_NONE {
        return None;
    }
    if action == ACTION_RESET {
        return Some(Action::Reset);
    }

    if (ACTION_SELECT_BASE..ACTION_DECREMENT_BASE).contains(&action) {
        let index = action.checked_sub(ACTION_SELECT_BASE)?;
        let control = ControlId::from_index((index / SLIDER_STEPS as u64) as usize)?;
        let step = (index % SLIDER_STEPS as u64) as usize;
        return Some(Action::Select { control, step });
    }

    if (ACTION_DECREMENT_BASE..ACTION_INCREMENT_BASE).contains(&action) {
        let control = ControlId::from_index((action - ACTION_DECREMENT_BASE) as usize)?;
        return Some(Action::Nudge { control, delta: -1 });
    }

    if (ACTION_INCREMENT_BASE..ACTION_RESET).contains(&action) {
        let control = ControlId::from_index((action - ACTION_INCREMENT_BASE) as usize)?;
        return Some(Action::Nudge { control, delta: 1 });
    }

    None
}

fn decode_slider_stop_path(path: &ElementPath) -> Option<(ControlId, usize)> {
    let [
        panel_index,
        row_index,
        adjuster_index,
        track_index,
        stop_index,
    ] = path.children.as_slice()
    else {
        return None;
    };
    if *panel_index != CONTROL_PANEL_INDEX
        || *adjuster_index != CONTROL_ADJUSTER_INDEX
        || *track_index != CONTROL_TRACK_INDEX
    {
        return None;
    }
    let control = ControlId::from_index(*row_index)?;
    (*stop_index < SLIDER_STEPS).then_some((control, *stop_index))
}

fn decode_nudge_button_path(path: &ElementPath) -> Option<(ControlId, usize)> {
    let [panel_index, row_index, adjuster_index, button_index] = path.children.as_slice() else {
        return None;
    };
    if *panel_index != CONTROL_PANEL_INDEX || *adjuster_index != CONTROL_ADJUSTER_INDEX {
        return None;
    }
    let control = ControlId::from_index(*row_index)?;
    matches!(*button_index, CONTROL_MINUS_INDEX | CONTROL_PLUS_INDEX)
        .then_some((control, *button_index))
}

fn hovered_slider_stop() -> Option<(ControlId, usize)> {
    hovered_path().and_then(|path| decode_slider_stop_path(&path))
}

fn active_slider_stop() -> Option<(ControlId, usize)> {
    active_path().and_then(|path| decode_slider_stop_path(&path))
}

fn hovered_nudge_control(expected_button_index: usize) -> Option<ControlId> {
    let (control, button_index) =
        hovered_path().and_then(|path| decode_nudge_button_path(&path))?;
    (button_index == expected_button_index).then_some(control)
}

fn hovered_path_store() -> &'static Mutex<Option<ElementPath>> {
    static STORE: OnceLock<Mutex<Option<ElementPath>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(None))
}

fn hovered_path() -> Option<ElementPath> {
    hovered_path_store()
        .lock()
        .expect("hovered path store should not be poisoned")
        .clone()
}

fn set_hovered_path(path: Option<ElementPath>) {
    *hovered_path_store()
        .lock()
        .expect("hovered path store should not be poisoned") = path;
}

fn active_path_store() -> &'static Mutex<Option<ElementPath>> {
    static STORE: OnceLock<Mutex<Option<ElementPath>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(None))
}

fn active_path() -> Option<ElementPath> {
    active_path_store()
        .lock()
        .expect("active path store should not be poisoned")
        .clone()
}

fn set_active_path(path: Option<ElementPath>) {
    *active_path_store()
        .lock()
        .expect("active path store should not be poisoned") = path;
}

fn stylesheet() -> &'static Stylesheet {
    static STYLESHEET: OnceLock<Stylesheet> = OnceLock::new();

    STYLESHEET.get_or_init(|| {
        parse_stylesheet(include_str!("transform_3d_playground.css"))
            .expect("transform playground stylesheet should stay valid")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slider_stop_paths_decode_to_the_expected_control_and_step() {
        let path = ElementPath::root(0)
            .with_child(CONTROL_PANEL_INDEX)
            .with_child(ControlId::RotateY.index())
            .with_child(CONTROL_ADJUSTER_INDEX)
            .with_child(CONTROL_TRACK_INDEX)
            .with_child(6);

        assert_eq!(
            decode_slider_stop_path(&path),
            Some((ControlId::RotateY, 6))
        );
    }

    #[test]
    fn update_applies_the_pending_slider_action() {
        let _guard = action_lock();
        let mut state = PlaygroundState::default();

        set_hovered_path(Some(
            ElementPath::root(0)
                .with_child(CONTROL_PANEL_INDEX)
                .with_child(ControlId::TranslateZ.index())
                .with_child(CONTROL_ADJUSTER_INDEX)
                .with_child(CONTROL_TRACK_INDEX)
                .with_child(SLIDER_STEPS - 1),
        ));
        PENDING_ACTION.store(ACTION_NONE, Ordering::Relaxed);
        select_slider_stop();

        let invalidation = update(
            &mut state,
            FrameInfo {
                frame_index: 1,
                delta: Duration::from_millis(16),
            },
        );

        assert_eq!(invalidation, Invalidation::Paint);
        assert_eq!(state.step(ControlId::TranslateZ), SLIDER_STEPS - 1);
        assert!(state.value(ControlId::TranslateZ) > 200.0);
    }

    #[test]
    fn drag_selection_updates_the_hovered_stop_when_the_same_slider_is_active() {
        let _guard = action_lock();
        let mut state = PlaygroundState::default();

        set_hovered_path(Some(
            ElementPath::root(0)
                .with_child(CONTROL_PANEL_INDEX)
                .with_child(ControlId::RotateY.index())
                .with_child(CONTROL_ADJUSTER_INDEX)
                .with_child(CONTROL_TRACK_INDEX)
                .with_child(6),
        ));
        set_active_path(Some(
            ElementPath::root(0)
                .with_child(CONTROL_PANEL_INDEX)
                .with_child(ControlId::RotateY.index())
                .with_child(CONTROL_ADJUSTER_INDEX)
                .with_child(CONTROL_TRACK_INDEX)
                .with_child(3),
        ));
        PENDING_ACTION.store(ACTION_NONE, Ordering::Relaxed);

        drag_slider_stop();
        let invalidation = update(
            &mut state,
            FrameInfo {
                frame_index: 2,
                delta: Duration::from_millis(16),
            },
        );

        assert_eq!(invalidation, Invalidation::Paint);
        assert_eq!(state.step(ControlId::RotateY), 6);
    }

    #[test]
    fn drag_selection_ignores_other_slider_rows() {
        let _guard = action_lock();
        let mut state = PlaygroundState::default();
        let original_step = state.step(ControlId::RotateY);

        set_hovered_path(Some(
            ElementPath::root(0)
                .with_child(CONTROL_PANEL_INDEX)
                .with_child(ControlId::RotateY.index())
                .with_child(CONTROL_ADJUSTER_INDEX)
                .with_child(CONTROL_TRACK_INDEX)
                .with_child(7),
        ));
        set_active_path(Some(
            ElementPath::root(0)
                .with_child(CONTROL_PANEL_INDEX)
                .with_child(ControlId::RotateX.index())
                .with_child(CONTROL_ADJUSTER_INDEX)
                .with_child(CONTROL_TRACK_INDEX)
                .with_child(4),
        ));
        PENDING_ACTION.store(ACTION_NONE, Ordering::Relaxed);

        drag_slider_stop();
        let invalidation = update(
            &mut state,
            FrameInfo {
                frame_index: 3,
                delta: Duration::from_millis(16),
            },
        );

        assert_eq!(invalidation, Invalidation::Clean);
        assert_eq!(state.step(ControlId::RotateY), original_step);
    }

    #[test]
    fn nudge_buttons_step_the_current_control_up_and_down() {
        let _guard = action_lock();
        let mut state = PlaygroundState::default();
        let start = state.step(ControlId::ScaleZ);

        set_hovered_path(Some(
            ElementPath::root(0)
                .with_child(CONTROL_PANEL_INDEX)
                .with_child(ControlId::ScaleZ.index())
                .with_child(CONTROL_ADJUSTER_INDEX)
                .with_child(CONTROL_PLUS_INDEX),
        ));
        step_slider_up();
        assert_eq!(
            update(
                &mut state,
                FrameInfo {
                    frame_index: 4,
                    delta: Duration::from_millis(16),
                },
            ),
            Invalidation::Paint
        );
        assert_eq!(state.step(ControlId::ScaleZ), start + 1);

        set_hovered_path(Some(
            ElementPath::root(0)
                .with_child(CONTROL_PANEL_INDEX)
                .with_child(ControlId::ScaleZ.index())
                .with_child(CONTROL_ADJUSTER_INDEX)
                .with_child(CONTROL_MINUS_INDEX),
        ));
        step_slider_down();
        assert_eq!(
            update(
                &mut state,
                FrameInfo {
                    frame_index: 5,
                    delta: Duration::from_millis(16),
                },
            ),
            Invalidation::Paint
        );
        assert_eq!(state.step(ControlId::ScaleZ), start);
    }

    #[test]
    fn ui_renders_one_clickable_stop_per_control_step_plus_reset() {
        let _guard = action_lock();
        let tree = build_ui(&PlaygroundState::default());
        let scene = build_render_tree_in_viewport(&tree, stylesheet(), 1440, 980);

        assert_eq!(
            count_click_handlers_in_node(&scene),
            CONTROL_COUNT * (SLIDER_STEPS + 2) + 1
        );
        assert!(find_text_in_node(&scene, "Transform Debug"));
    }

    #[test]
    fn rotate_y_in_the_example_keeps_a_broad_visible_face() {
        let _guard = action_lock();
        let mut state = PlaygroundState::default();
        state.set_step(ControlId::RotateY, ControlId::RotateY.default_step() + 1);

        let tree = build_ui(&state);
        let scene = build_render_tree_in_viewport(&tree, stylesheet(), 1440, 980);
        let projection_plane = &scene.children[0].children[1].children[0].children[0];
        let anchor = &projection_plane.children[0];
        let play_card = &anchor.children[1];
        let mut buffer = vec![0_u32; 1440 * 980];
        render_to_buffer(
            std::slice::from_ref(&scene),
            &mut buffer,
            1440,
            980,
            Color::WHITE,
        );

        let accent = ((16_u32) << 16) | ((185_u32) << 8) | 129_u32;
        let mut x0 = usize::MAX;
        let mut x1 = 0usize;
        let mut hit_count = 0usize;

        for y in 40..600 {
            for x in 320..840 {
                let pixel = buffer[y * 1440 + x];
                if pixel != accent {
                    continue;
                }
                x0 = x0.min(x);
                x1 = x1.max(x);
                hit_count += 1;
            }
        }

        assert!(
            hit_count > 6_000,
            "the example should keep a substantial visible card face (hits={hit_count}, x0={x0}, x1={x1})"
        );
        assert!(
            x1 > x0,
            "the example should produce a measurable visible face width (hits={hit_count}, x0={x0}, x1={x1})"
        );
        assert!(
            x1 - x0 > 110,
            "rotateY(15deg) in the playground should still read as a broad card face (hits={hit_count}, x0={x0}, x1={x1}, projection={:?}, anchor={:?}, card={:?})",
            projection_plane.layout,
            anchor.layout,
            play_card.layout,
        );
    }

    fn count_click_handlers_in_node(node: &RenderNode) -> usize {
        let own = usize::from(node.handlers.click.is_some());
        own + node
            .children
            .iter()
            .map(count_click_handlers_in_node)
            .sum::<usize>()
    }

    fn find_text_in_node(node: &RenderNode, needle: &str) -> bool {
        if let RenderKind::Text(content) = &node.kind
            && content == needle
        {
            return true;
        }

        node.children
            .iter()
            .any(|child| find_text_in_node(child, needle))
    }

    fn action_lock() -> MutexGuard<'static, ()> {
        static TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        TEST_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("test action lock should not be poisoned")
    }
}
