use super::*;
use crate::core::{LayoutBox, RuntimeDirtyClass, RuntimeSyncPolicy};

pub(super) fn key(value: &str, command: bool, shift: bool) -> EngineEvent {
    let mut modifiers = crate::renderer::KeyboardModifiers {
        shift,
        ..Default::default()
    };
    if cfg!(target_os = "macos") {
        modifiers.super_key = command;
    } else {
        modifiers.control = command;
    }
    EngineEvent::Key(KeyboardEvent {
        logical_key: if value.len() == 1 {
            KeyIdentity::Character(value.into())
        } else {
            KeyIdentity::Named(value.into())
        },
        physical_key: None,
        location: Default::default(),
        state: ButtonState::Pressed,
        repeat: false,
        modifiers,
    })
}

pub(super) fn fixture(password: bool) -> (NativeTextInputs, RuntimeWorld) {
    let root: Node = Node::element("input")
        .with_id("edit")
        .with_attribute("type", if password { "password" } else { "text" })
        .with_attribute("value", "aéकz")
        .with_attribute("autofocus", "true")
        .into();
    let mut inputs = NativeTextInputs::default();
    inputs.clipboard.memory = Some(Ok(String::new()));
    inputs.materialize_root(0, root.clone());
    let mut world = RuntimeWorld::default();
    world.sync_root(
        0,
        &root,
        RuntimeSyncPolicy::ForceRebuild,
        RuntimeDirtyClass::Structure,
    );
    (inputs, world)
}

#[test]
fn shift_selection_copy_cut_paste_and_failure_preserve_unicode() {
    let (mut inputs, world) = fixture(false);
    assert!(inputs.handle_engine_event(&key("ArrowLeft", false, true), None, &world));
    assert!(inputs.handle_engine_event(&key("ArrowLeft", false, true), None, &world));
    assert!(inputs.handle_engine_event(&key("c", true, false), None, &world));
    assert_eq!(inputs.clipboard.get_text().as_deref(), Some("कz"));
    assert_eq!(inputs.states["id:edit"].value, "aéकz");
    inputs.clipboard.memory = Some(Err(()));
    assert!(!inputs.handle_engine_event(&key("x", true, false), None, &world));
    assert_eq!(inputs.states["id:edit"].value, "aéकz");
    inputs.clipboard.memory = Some(Ok(String::new()));
    assert!(inputs.handle_engine_event(&key("x", true, false), None, &world));
    assert_eq!(inputs.states["id:edit"].value, "aé");
    assert_eq!(inputs.clipboard.get_text().as_deref(), Some("कz"));
    assert!(inputs.handle_engine_event(&key("a", true, false), None, &world));
    inputs.clipboard.memory = Some(Ok("नमस्ते\r\n!\t".into()));
    assert!(inputs.handle_engine_event(&key("v", true, false), None, &world));
    assert_eq!(inputs.states["id:edit"].value, "नमस्ते!");
    assert!(inputs.states["id:edit"].selection_range().is_none());
}

#[test]
fn password_fields_allow_paste_but_never_export_secrets() {
    let (mut inputs, world) = fixture(true);
    inputs.handle_engine_event(&key("a", true, false), None, &world);
    assert!(!inputs.handle_engine_event(&key("c", true, false), None, &world));
    assert!(!inputs.handle_engine_event(&key("x", true, false), None, &world));
    assert_eq!(inputs.clipboard.get_text().as_deref(), Some(""));
    inputs.clipboard.memory = Some(Ok("1234".into()));
    assert!(inputs.handle_engine_event(&key("v", true, false), None, &world));
    assert_eq!(inputs.states["id:edit"].value, "1234");
}

#[test]
fn shift_home_end_extends_and_unmodified_arrows_collapse_selection() {
    let (mut inputs, world) = fixture(false);
    inputs.handle_engine_event(&key("Home", false, true), None, &world);
    assert_eq!(
        inputs.states["id:edit"].selection_range(),
        Some((0, "aéकz".len()))
    );
    inputs.handle_engine_event(&key("ArrowRight", false, false), None, &world);
    assert_eq!(inputs.states["id:edit"].cursor, "aéकz".len());
    assert_eq!(inputs.states["id:edit"].selection_range(), None);
    inputs.handle_engine_event(&key("Home", false, false), None, &world);
    inputs.handle_engine_event(&key("End", false, true), None, &world);
    assert_eq!(
        inputs.states["id:edit"].selection_range(),
        Some((0, "aéकz".len()))
    );
}

#[test]
fn document_drag_selects_and_copies_across_text_nodes_and_clears_stale_text() {
    let root: Node = Node::element("main")
        .with_child(Node::element("p").with_child(Node::text("First")).into())
        .with_child(Node::element("p").with_child(Node::text("éक")).into())
        .into();
    let mut world = RuntimeWorld::default();
    world.sync_root(
        0,
        &root,
        RuntimeSyncPolicy::ForceRebuild,
        RuntimeDirtyClass::Structure,
    );
    let mut scene = vec![
        RenderNode::container(LayoutBox::new(0.0, 0.0, 200.0, 80.0))
            .with_element_path(ElementPath::root(0))
            .with_child(
                RenderNode::text(LayoutBox::new(0.0, 0.0, 200.0, 25.0), "First")
                    .with_element_path(ElementPath::root(0).with_child(0)),
            )
            .with_child(
                RenderNode::text(LayoutBox::new(0.0, 40.0, 200.0, 25.0), "éक")
                    .with_element_path(ElementPath::root(0).with_child(1)),
            ),
    ];
    let mut inputs = NativeTextInputs::default();
    inputs.clipboard.memory = Some(Ok(String::new()));
    let move_to = |x, y| EngineEvent::PointerMoved {
        position: PointerPosition { x, y },
        modifiers: Default::default(),
    };
    inputs.handle_engine_event(&move_to(0.0, 10.0), Some(&scene), &world);
    inputs.handle_engine_event(
        &EngineEvent::PointerButton {
            button: PointerButton::Primary,
            state: ButtonState::Pressed,
            modifiers: Default::default(),
        },
        Some(&scene),
        &world,
    );
    assert!(inputs.handle_engine_event(&move_to(190.0, 50.0), Some(&scene), &world));
    inputs.handle_engine_event(
        &EngineEvent::PointerButton {
            button: PointerButton::Primary,
            state: ButtonState::Released,
            modifiers: Default::default(),
        },
        Some(&scene),
        &world,
    );
    inputs.decorate_scene(&mut scene, &Stylesheet::default(), &world);
    assert!(
        scene[0]
            .children
            .iter()
            .all(|node| node.text_edit.as_ref().unwrap().selection.is_some())
    );
    assert!(inputs.handle_engine_event(&key("c", true, false), Some(&scene), &world));
    assert_eq!(inputs.clipboard.get_text().as_deref(), Some("First\néक"));
    scene[0].children.clear();
    inputs.handle_engine_event(&key("c", true, false), Some(&scene), &world);
    assert!(inputs.document_selection.selected_text().is_none());
}

#[test]
fn mouse_selection_hits_centered_and_right_aligned_input_text() {
    let value = "Café";
    let mut node = RenderNode::text(LayoutBox::new(10.0, 0.0, 300.0, 30.0), value);
    let width = layout_text_caret_stops(value, &node.style.text)
        .last()
        .unwrap()
        .offset_px;
    for alignment in [
        crate::fonts::TextAlign::Center,
        crate::fonts::TextAlign::Right,
    ] {
        node.style.text.text_align = alignment;
        let origin = 10.0
            + if alignment == crate::fonts::TextAlign::Center {
                (300.0 - width) * 0.5
            } else {
                300.0 - width
            };
        assert_eq!(caret_index_from_pointer(&node, value, origin), 0);
        assert_eq!(
            caret_index_from_pointer(&node, value, origin + width),
            value.len()
        );
    }
}
