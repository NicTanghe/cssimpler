use super::editing_tests::{fixture, key};
use super::*;
use crate::app::{App, FrameInfo, Invalidation};
use crate::renderer::SceneProvider;
use std::cell::RefCell;

fn preedit(text: &str, cursor: Option<(usize, usize)>) -> EngineEvent {
    EngineEvent::TextInput(TextInputEvent::Preedit {
        text: text.into(),
        cursor,
    })
}

#[test]
fn ime_preview_replaces_selection_but_commit_changes_value_only_once() {
    let (mut inputs, world) = fixture(false);
    inputs.handle_engine_event(&key("ArrowLeft", false, true), None, &world);
    inputs.handle_engine_event(&key("ArrowLeft", false, true), None, &world);
    assert!(inputs.handle_engine_event(&preedit("にほん", Some((9, 9))), None, &world));
    assert_eq!(inputs.states["id:edit"].value, "aéकz");
    assert_eq!(inputs.states["id:edit"].preview_value(), "aéにほん");
    inputs.handle_engine_event(&preedit("日本", Some((0, 6))), None, &world);
    let edit = text_edit_decoration(&inputs.states["id:edit"], false, Default::default());
    assert_eq!(edit.composition, Some(3..9));
    assert_eq!(edit.caret, Some(9));
    assert_eq!(edit.selection.map(|s| (s.start, s.end)), Some((3, 9)));
    // Winit clears preedit immediately before committing. Keep the original selection.
    inputs.handle_engine_event(&preedit("", None), None, &world);
    inputs.handle_engine_event(
        &EngineEvent::TextInput(TextInputEvent::Commit("日本".into())),
        None,
        &world,
    );
    assert_eq!(inputs.states["id:edit"].value, "aé日本");
    assert_eq!(inputs.states["id:edit"].cursor, 9);
    assert!(!inputs.is_composing());
}

#[test]
fn ime_owns_editing_keys_and_cancel_restores_committed_selection() {
    for cancel in [
        preedit("", None),
        EngineEvent::TextInput(TextInputEvent::Disabled),
        EngineEvent::FocusChanged(false),
        EngineEvent::Suspended,
    ] {
        let (mut inputs, world) = fixture(false);
        inputs.handle_engine_event(&key("ArrowLeft", false, true), None, &world);
        let original = inputs.states["id:edit"].clone();
        inputs.handle_engine_event(&preedit("候補", Some((6, 6))), None, &world);
        for name in [
            "ArrowLeft",
            "Backspace",
            "Delete",
            "Home",
            "End",
            "Tab",
            "Enter",
            "Escape",
        ] {
            assert!(inputs.handle_engine_event(&key(name, false, false), None, &world));
        }
        inputs.handle_engine_event(&key("x", true, false), None, &world);
        assert_eq!(inputs.clipboard.get_text().as_deref(), Some(""));
        assert!(inputs.handle_engine_event(&cancel, None, &world));
        let state = &inputs.states["id:edit"];
        assert_eq!(state.preview_value(), original.value);
        assert_eq!(
            (state.cursor, state.selection_anchor),
            (original.cursor, original.selection_anchor)
        );
        assert!(!inputs.is_composing());
    }
}

#[test]
fn ime_password_preview_and_ranges_are_masked_and_utf8_safe() {
    let (mut inputs, world) = fixture(true);
    inputs.handle_engine_event(&preedit("日本", Some((1, usize::MAX))), None, &world);
    let state = &inputs.states["id:edit"];
    let edit = text_edit_decoration(state, true, Default::default());
    assert_eq!(display_value(&state.preview_value(), true), "******");
    assert_eq!(edit.composition, Some(4..6));
    assert_eq!(edit.caret, Some(6));
    assert_eq!(edit.selection.map(|s| (s.start, s.end)), Some((4, 6)));
    inputs.handle_engine_event(&preedit("日本", None), None, &world);
    assert_eq!(
        text_edit_decoration(&inputs.states["id:edit"], true, Default::default()).caret,
        None
    );
    inputs.clear_focus();
    assert!(!inputs.handle_engine_event(&preedit("ignored", None), None, &world));
    assert!(!inputs.handle_engine_event(
        &EngineEvent::TextInput(TextInputEvent::Commit("ignored".into())),
        None,
        &world
    ));
}

thread_local! {
    static INPUTS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn capture(value: &str) {
    INPUTS.with_borrow_mut(|values| values.push(value.into()));
}

fn rendered_text(scene: &[RenderNode]) -> String {
    scene
        .iter()
        .map(|node| {
            let own = match &node.kind {
                crate::core::RenderKind::Text(text) => text.as_str(),
                _ => "",
            };
            format!("{own}{}", rendered_text(&node.children))
        })
        .collect()
}

#[test]
fn ime_app_preview_survives_rebuild_without_callbacks_and_cancels_on_controlled_change() {
    INPUTS.with_borrow_mut(Vec::clear);
    let stylesheet =
        crate::style::parse_stylesheet("input { width: 240px; height: 30px; }").unwrap();
    let mut app = App::new(
        "aé".to_owned(),
        &stylesheet,
        |_: &mut String, _| Invalidation::Clean,
        |value: &String| {
            Node::element("input")
                .with_id("edit")
                .with_attribute("autofocus", "true")
                .with_attribute("value", value)
                .on_input(capture)
                .into()
        },
    )
    .with_event_handler(|_, event| {
        assert!(
            !matches!(event, EngineEvent::Key(_)),
            "composition key reached application shortcut handler"
        );
        false
    });
    let frame = |n| FrameInfo {
        frame_index: n,
        delta: Default::default(),
    };
    app.frame(frame(0));
    app.handle_engine_event(&preedit("日本", Some((6, 6))));
    assert_eq!(rendered_text(&app.frame(frame(1))), "aé日本");
    app.invalidate(Invalidation::Structure);
    assert_eq!(rendered_text(&app.frame(frame(2))), "aé日本");
    app.handle_engine_event(&key("Enter", false, false));
    #[cfg(feature = "gpu-viewport")]
    assert!(SceneProvider::handle_application_event(
        &mut app,
        &key("Enter", false, false)
    ));
    INPUTS.with_borrow(|values| assert!(values.is_empty()));
    app.handle_engine_event(&preedit("", None));
    app.handle_engine_event(&EngineEvent::TextInput(TextInputEvent::Commit(
        "日本".into(),
    )));
    INPUTS.with_borrow(|values| assert_eq!(values.as_slice(), ["aé日本"]));
    // The application accepts the committed value through its controlled state.
    *app.state_mut() = "aé日本".into();
    app.invalidate(Invalidation::Structure);
    app.frame(frame(3));
    app.handle_engine_event(&preedit("語", Some((3, 3))));
    *app.state_mut() = "external".into();
    app.invalidate(Invalidation::Structure);
    assert_eq!(rendered_text(&app.frame(frame(4))), "external");
    assert_eq!(SceneProvider::ime_target(&app), Some(ElementPath::root(0)));
    assert!(!app.text_inputs.is_composing());
}

#[test]
fn ime_fragment_app_commits_preview_and_protects_shortcuts() {
    use crate::app::{Fragment, FragmentApp};
    let stylesheet = Stylesheet::default();
    let mut app = FragmentApp::new(
        (),
        &stylesheet,
        |_, _| Invalidation::Clean,
        [Fragment::new("editor", |_: &()| {
            Node::element("input")
                .with_id("edit")
                .with_attribute("autofocus", "true")
                .with_attribute("value", "start")
                .into()
        })],
    )
    .with_event_handler(|_, event| {
        assert!(!matches!(event, EngineEvent::Key(_)));
        false
    });
    let frame = |frame_index| FrameInfo {
        frame_index,
        delta: Default::default(),
    };
    app.frame(frame(0));
    app.handle_engine_event(&preedit("語", Some((3, 3))));
    assert_eq!(rendered_text(&app.frame(frame(1))), "start語");
    app.handle_engine_event(&key("Enter", false, false));
    #[cfg(feature = "gpu-viewport")]
    assert!(SceneProvider::handle_application_event(
        &mut app,
        &key("Enter", false, false)
    ));
    assert!(SceneProvider::ime_composing(&app));
    // Some embedding clients send Commit without the preceding synthetic clear.
    app.handle_engine_event(&EngineEvent::TextInput(TextInputEvent::Commit("語".into())));
    assert_eq!(rendered_text(&app.frame(frame(2))), "start語");
    assert!(!SceneProvider::ime_composing(&app));
}

#[test]
fn ime_focus_removal_and_readonly_cancel_pending_text() {
    for attribute in ["readonly", "disabled"] {
        let (mut inputs, world) = fixture(false);
        inputs.handle_engine_event(&preedit("temporary", Some((9, 9))), None, &world);
        let root = Node::element("input")
            .with_id("edit")
            .with_attribute("value", "aéकz")
            .with_attribute(attribute, "true")
            .into();
        inputs.materialize_root(0, root);
        assert!(inputs.ime_target().is_none());
        assert!(inputs.states["id:edit"].composition.is_none());
    }
    let (mut inputs, world) = fixture(false);
    inputs.handle_engine_event(&preedit("temporary", None), None, &world);
    inputs.materialize_root(0, Node::element("div").into());
    assert!(inputs.ime_target().is_none());
    assert!(inputs.states["id:edit"].composition.is_none());
}
