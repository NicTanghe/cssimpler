//! Native IME lab: cargo run --example ime_composition
//! Headless event replay: cargo run --example ime_composition -- --smoke
use std::{cell::RefCell, collections::VecDeque, sync::OnceLock, time::Duration};

use anyhow::{Result, bail, ensure};
use cssimpler::{
    app::{App, Invalidation},
    core::{Node, RenderKind, RenderNode},
    renderer::{
        ButtonState, EngineEvent, FrameInfo, KeyIdentity, KeyboardEvent, SceneProvider,
        TextInputEvent, ViewportSize, WindowConfig,
    },
    style::{Stylesheet, parse_stylesheet},
    ui,
};

thread_local! {
    // Callbacks and the native event loop share a thread. Tests keep independent queues.
    static ACTIONS: RefCell<Vec<Action>> = const { RefCell::new(Vec::new()) };
}

enum Action {
    Value(usize, String),
    Enter,
    Reset,
    Replace,
    Rebuild,
    ToggleReadonly,
    ToggleVisible,
}

struct State {
    values: [String; 3],
    changes: [usize; 3],
    enters: usize,
    application_enters: usize,
    readonly: bool,
    visible: bool,
    dirty: bool,
    logs: VecDeque<String>,
    rebuild_elapsed: Duration,
}

impl Default for State {
    fn default() -> Self {
        Self {
            values: [
                "prefix suffix".into(),
                "secret".into(),
                "aligned input".into(),
            ],
            changes: [0; 3],
            enters: 0,
            application_enters: 0,
            readonly: false,
            visible: true,
            dirty: false,
            logs: VecDeque::new(),
            rebuild_elapsed: Duration::ZERO,
        }
    }
}

impl State {
    fn log(&mut self, message: String) {
        eprintln!("[ime] {message}");
        self.logs.push_back(message);
        while self.logs.len() > 8 {
            self.logs.pop_front();
        }
        self.dirty = true;
    }
}

type Lab = App<'static, State, fn(&mut State, FrameInfo) -> Invalidation, fn(&State) -> Node>;

fn lab() -> Lab {
    ACTIONS.with_borrow_mut(Vec::clear);
    App::new(State::default(), stylesheet(), update as _, build_ui as _)
        .with_event_handler(observe_event)
        .with_continuous_updates(true)
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [] => lab()
            .run(WindowConfig::new(
                "cssimpler / native IME test lab",
                1100,
                850,
            ))
            .map_err(Into::into),
        [flag] if flag == "--smoke" => smoke(),
        _ => bail!("Usage: cargo run --example ime_composition [-- --smoke]"),
    }
}

fn queue(action: Action) {
    ACTIONS.with_borrow_mut(|actions| actions.push(action));
}
fn plain_changed(value: &str) {
    queue(Action::Value(0, value.into()));
}
fn password_changed(value: &str) {
    queue(Action::Value(1, value.into()));
}
fn aligned_changed(value: &str) {
    queue(Action::Value(2, value.into()));
}
fn enter() {
    queue(Action::Enter);
}
fn reset() {
    queue(Action::Reset);
}
fn replace() {
    queue(Action::Replace);
}
fn rebuild() {
    queue(Action::Rebuild);
}
fn toggle_readonly() {
    queue(Action::ToggleReadonly);
}
fn toggle_visible() {
    queue(Action::ToggleVisible);
}

fn observe_event(state: &mut State, event: &EngineEvent) -> bool {
    // Observe without consuming: native editing must still receive the event.
    // Never print raw preedit, commit, or password values.
    let message = match event {
        EngineEvent::TextInput(TextInputEvent::Preedit { text, cursor }) => Some(format!(
            "Preedit: {} chars / {} bytes; cursor {cursor:?}",
            text.chars().count(),
            text.len()
        )),
        EngineEvent::TextInput(TextInputEvent::Commit(text)) => Some(format!(
            "Commit: {} chars / {} bytes",
            text.chars().count(),
            text.len()
        )),
        EngineEvent::TextInput(TextInputEvent::Disabled) => {
            Some("IME disabled / preview cancelled".into())
        }
        EngineEvent::FocusChanged(focused) => Some(format!("Window focus: {focused}")),
        EngineEvent::Key(key)
            if key.state == ButtonState::Pressed
                && key.logical_key == KeyIdentity::Named("Enter".into()) =>
        {
            state.application_enters += 1;
            Some("Application Enter observed (must stay unchanged during composition)".into())
        }
        _ => None,
    };
    if let Some(message) = message {
        state.log(message);
    }
    false
}

fn update(state: &mut State, frame: FrameInfo) -> Invalidation {
    let actions = ACTIONS.with_borrow_mut(std::mem::take);
    let mut changed = std::mem::take(&mut state.dirty) || !actions.is_empty();
    for action in actions {
        match action {
            Action::Value(index, value) => {
                state.values[index] = value;
                state.changes[index] += 1;
                state.log(format!(
                    "oninput field {}: call {}",
                    index + 1,
                    state.changes[index]
                ));
            }
            Action::Enter => {
                state.enters += 1;
                state.log("onenter callback".into());
            }
            Action::Reset => *state = State::default(),
            Action::Replace => {
                state.values[0] = "external replacement".into();
                state.log("Controlled value replaced".into());
            }
            Action::Rebuild => state.log("Rebuilt with unchanged values".into()),
            Action::ToggleReadonly => {
                state.readonly = !state.readonly;
                state.log(format!("Plain readonly: {}", state.readonly));
            }
            Action::ToggleVisible => {
                state.visible = !state.visible;
                state.log(format!("Plain visible: {}", state.visible));
            }
        }
    }
    state.rebuild_elapsed += frame.delta;
    if state.rebuild_elapsed >= Duration::from_secs(4) {
        state.rebuild_elapsed = Duration::ZERO;
        state.log("Periodic rebuild with unchanged values".into());
        changed = true;
    }
    state.dirty = false;
    if changed {
        Invalidation::Structure
    } else {
        Invalidation::Clean
    }
}

fn field(state: &State, index: usize) -> Node {
    let id = ["plain", "password", "aligned"][index];
    let handlers = [plain_changed as fn(&str), password_changed, aligned_changed];
    let mut input = Node::element("input")
        .with_id(id)
        .with_attribute("type", if index == 1 { "password" } else { "text" })
        .with_attribute("value", &state.values[index])
        .on_input(handlers[index])
        .on_enter(enter);
    if index == 0 {
        input = input.with_attribute("autofocus", "true");
        if state.readonly {
            input = input.with_attribute("readonly", "true");
        }
    }
    input.into()
}

fn build_ui(state: &State) -> Node {
    let plain = state.visible.then(|| field(state, 0));
    let logs = state.logs.iter().cloned().collect::<Vec<_>>().join("\n");
    ui! {
        <main id="lab">
            <h1>Native IME test lab</h1>
            <p class="intro">Compose with your OS Japanese, Chinese or Korean IME. The popup is owned by the OS.</p>
            <div class="columns">
                <section class="panel fields">
                    <h2>Test fields</h2>
                    <label>{"1. Plain text: select a word, then compose"}</label>
                    {plain}
                    <p class="status">{format!("Committed: {:?} | oninput: {}", state.values[0], state.changes[0])}</p>
                    <label>{"2. Password: preview must remain masked"}</label>
                    {field(state, 1)}
                    <p class="status">{format!("Committed: {} characters (hidden) | oninput: {}", state.values[1].chars().count(), state.changes[1])}</p>
                    <label>3. Scrolling, padding, alignment and transform</label>
                    <div class="scroll-case">
                        <div class="scroll-content">
                            <div class="offset-case">{field(state, 2)}</div>
                            <p>Scroll while composing; the candidate popup should follow the caret.</p>
                        </div>
                    </div>
                    <p class="status">{format!("Aligned oninput: {}", state.changes[2])}</p>
                    <div class="buttons">
                        <button onclick={reset}>Reset</button>
                        <button onclick={replace}>Replace plain value</button>
                        <button onclick={rebuild}>Rebuild same values</button>
                        <button onclick={toggle_readonly}>Toggle readonly</button>
                        <button onclick={toggle_visible}>Hide / show plain</button>
                    </div>
                </section>
                <section class="panel diagnostics">
                    <h2>Expected behavior</h2>
                    <p>1. Preedit is underlined; committed text and oninput counts stay unchanged.</p>
                    <p>2. Confirming a candidate changes the value once. Cancelling restores the original selection.</p>
                    <p>3. Enter used by the IME must not submit the field or reach application shortcuts.</p>
                    <p>4. Switch fields/windows, resize, scroll, and change display scaling. Watch the popup position.</p>
                    <p>5. Every 4 seconds, unchanged values rebuild automatically. Composition should survive.</p>
                    <p class="status">{format!("onenter: {} | application Enter: {}", state.enters, state.application_enters)}</p>
                    <h2>Recent events</h2>
                    <p class="event-log">{logs}</p>
                    <p class="note">The log records lengths and byte offsets, never typed text. Clicking a button cancels composition. Counts also include ordinary keyboard input.</p>
                </section>
            </div>
        </main>
    }
}

fn stylesheet() -> &'static Stylesheet {
    static STYLE: OnceLock<Stylesheet> = OnceLock::new();
    STYLE.get_or_init(|| {
        parse_stylesheet(include_str!("ime_composition.css")).expect("IME lab CSS should parse")
    })
}

fn preedit(text: &str, cursor: Option<(usize, usize)>) -> EngineEvent {
    EngineEvent::TextInput(TextInputEvent::Preedit {
        text: text.into(),
        cursor,
    })
}

fn key(name: &str, command: bool) -> EngineEvent {
    let mut modifiers = cssimpler::renderer::KeyboardModifiers::default();
    if cfg!(target_os = "macos") {
        modifiers.super_key = command;
    } else {
        modifiers.control = command;
    }
    EngineEvent::Key(KeyboardEvent {
        logical_key: if name.len() == 1 {
            KeyIdentity::Character(name.into())
        } else {
            KeyIdentity::Named(name.into())
        },
        physical_key: None,
        location: Default::default(),
        state: ButtonState::Pressed,
        repeat: false,
        modifiers,
    })
}

fn node<'a>(scene: &'a [RenderNode], id: &str) -> Option<&'a RenderNode> {
    scene.iter().find_map(|item| {
        if item.id() == Some(id) {
            Some(item)
        } else {
            node(&item.children, id)
        }
    })
}

fn step(app: &mut Lab, index: &mut u64, event: Option<EngineEvent>) -> Vec<RenderNode> {
    if let Some(event) = event {
        app.handle_engine_event(&event);
    }
    *index += 1;
    app.frame(FrameInfo {
        frame_index: *index,
        delta: Duration::from_millis(16),
    })
}

fn smoke() -> Result<()> {
    let mut app = lab();
    app.set_viewport(ViewportSize {
        width: 1100,
        height: 850,
    });
    let mut index = 0;
    step(&mut app, &mut index, None);
    step(&mut app, &mut index, Some(key("a", true)));
    let scene = step(&mut app, &mut index, Some(preedit("にほん", Some((9, 9)))));
    ensure!(
        matches!(&node(&scene, "plain").unwrap().kind, RenderKind::Text(text) if text == "にほん"),
        "plain preedit not displayed"
    );
    ensure!(
        app.state().values[0] == "prefix suffix" && app.state().changes[0] == 0,
        "preedit changed committed state"
    );
    step(&mut app, &mut index, Some(key("Enter", false)));
    ensure!(
        app.state().enters == 0 && app.state().application_enters == 0,
        "composition Enter escaped to application"
    );
    queue(Action::Rebuild);
    step(&mut app, &mut index, None);
    ensure!(
        SceneProvider::ime_composing(&app),
        "unchanged rebuild lost composition"
    );
    step(&mut app, &mut index, Some(preedit("日本", Some((0, 6)))));
    step(&mut app, &mut index, Some(preedit("", None)));
    step(
        &mut app,
        &mut index,
        Some(EngineEvent::TextInput(TextInputEvent::Commit(
            "日本".into(),
        ))),
    );
    ensure!(
        app.state().values[0] == "日本" && app.state().changes[0] == 1,
        "commit was lost or duplicated"
    );
    step(&mut app, &mut index, Some(preedit("取消", None)));
    step(
        &mut app,
        &mut index,
        Some(EngineEvent::TextInput(TextInputEvent::Disabled)),
    );
    ensure!(
        app.state().values[0] == "日本"
            && app.state().changes[0] == 1
            && !SceneProvider::ime_composing(&app),
        "cancel changed value"
    );
    step(&mut app, &mut index, Some(key("Enter", false)));
    ensure!(
        app.state().enters == 1 && app.state().application_enters == 1,
        "ordinary Enter stopped working after composition"
    );
    step(&mut app, &mut index, Some(key("Tab", false)));
    step(&mut app, &mut index, Some(key("a", true)));
    let scene = step(&mut app, &mut index, Some(preedit("秘密", Some((6, 6)))));
    ensure!(
        matches!(&node(&scene, "password").unwrap().kind, RenderKind::Text(text) if text == "**"),
        "password preview was not masked"
    );
    ensure!(
        app.state().changes[1] == 0,
        "password preedit triggered oninput"
    );
    step(
        &mut app,
        &mut index,
        Some(EngineEvent::TextInput(TextInputEvent::Commit(
            "秘密".into(),
        ))),
    );
    ensure!(
        app.state().changes[1] == 1,
        "password commit callback count differs"
    );
    step(&mut app, &mut index, Some(key("Tab", false)));
    let scene = step(&mut app, &mut index, Some(preedit("語", None)));
    ensure!(
        node(&scene, "aligned")
            .unwrap()
            .text_edit
            .as_ref()
            .is_some_and(|edit| edit.composition.is_some() && edit.caret.is_none()),
        "aligned composition or hidden caret missing"
    );
    for scale in [1.0, 1.25, 2.0] {
        let mut pixels = vec![0; 1100 * 850];
        cssimpler::renderer::render_to_buffer_at_scale(
            &scene,
            &mut pixels,
            1100,
            850,
            cssimpler::core::Color::WHITE,
            scale,
        );
    }
    step(&mut app, &mut index, Some(EngineEvent::FocusChanged(false)));
    ensure!(
        !SceneProvider::ime_composing(&app),
        "focus loss kept composition"
    );
    step(&mut app, &mut index, Some(EngineEvent::FocusChanged(true)));
    step(&mut app, &mut index, Some(key("Tab", false)));
    step(&mut app, &mut index, Some(preedit("pending", Some((7, 7)))));
    queue(Action::Replace);
    step(&mut app, &mut index, None);
    ensure!(
        !SceneProvider::ime_composing(&app) && app.state().values[0] == "external replacement",
        "controlled replacement did not cancel composition"
    );
    step(&mut app, &mut index, Some(preedit("pending", None)));
    queue(Action::ToggleReadonly);
    step(&mut app, &mut index, None);
    ensure!(
        SceneProvider::ime_target(&app).is_none(),
        "readonly field retained IME focus"
    );
    queue(Action::ToggleReadonly);
    step(&mut app, &mut index, None);
    step(&mut app, &mut index, Some(preedit("pending", None)));
    queue(Action::ToggleVisible);
    let scene = step(&mut app, &mut index, None);
    ensure!(
        node(&scene, "plain").is_none() && SceneProvider::ime_target(&app).is_none(),
        "removed field retained IME focus"
    );
    ensure!(
        app.state().logs.iter().all(|line| !line.contains("秘密")),
        "event log contains password text"
    );
    println!(
        "PASS: preview, selection replacement, single commit, cancellation, password masking, shortcut isolation, rebuild, focus loss, controlled replacement, readonly/removal and scaled painting."
    );
    println!(
        "This replays engine events. Run without --smoke to test the actual OS IME and candidate popup."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn ime_lab_event_replay() {
        super::smoke().unwrap();
    }

    #[test]
    fn periodic_rebuild_preserves_composition_without_pointer_input() {
        use super::*;
        let mut app = lab();
        let mut index = 0;
        step(&mut app, &mut index, None);
        step(&mut app, &mut index, Some(preedit("にほん", Some((9, 9)))));
        app.frame(FrameInfo {
            frame_index: 3,
            delta: Duration::from_secs(4),
        });
        assert!(SceneProvider::ime_composing(&app));
        assert_eq!(app.state().values[0], "prefix suffix");
        assert_eq!(app.state().changes[0], 0);
        assert!(
            app.state()
                .logs
                .iter()
                .any(|line| line == "Periodic rebuild with unchanged values")
        );
    }
}
