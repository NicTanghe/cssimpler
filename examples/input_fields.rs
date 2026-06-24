use std::sync::OnceLock;

use anyhow::Result;
use cssimpler::app::{App, Invalidation};
use cssimpler::core::Node;
use cssimpler::renderer::{FrameInfo, WindowConfig};
use cssimpler::style::{Stylesheet, parse_stylesheet};
use cssimpler::ui;

#[derive(Debug, Default)]
struct InputFieldsState;

fn main() -> Result<()> {
    let config = WindowConfig::new("cssimpler / input fields", 640, 420);

    App::new(InputFieldsState, stylesheet(), update, build_ui)
        .run(config)
        .map_err(Into::into)
}

fn update(_state: &mut InputFieldsState, _frame: FrameInfo) -> Invalidation {
    Invalidation::Clean
}

fn build_ui(_state: &InputFieldsState) -> Node {
    ui! {
        <div id="app">
            <form class="panel">
                <h2>Input Fields with Padding</h2>

                <label for="fname">First Name</label>
                <input type="text" id="fname" name="fname" value="Ada">

                <label for="lname">Last Name</label>
                <input type="text" id="lname" name="lname" value="Lovelace">
            </form>
        </div>
    }
}

fn stylesheet() -> &'static Stylesheet {
    static STYLESHEET: OnceLock<Stylesheet> = OnceLock::new();

    STYLESHEET.get_or_init(|| {
        parse_stylesheet(include_str!("input_fields.css"))
            .expect("input fields example stylesheet should stay valid")
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use cssimpler::core::{Color, RenderKind, RenderNode, TextEditDecoration};
    use cssimpler::fonts::layout_text_block;
    use cssimpler::renderer::{
        ButtonState, EngineEvent, FrameInfo, PointerButton, PointerPosition, SceneProvider,
        TextInputEvent,
    };

    use super::{InputFieldsState, build_ui, stylesheet, update};
    use cssimpler::app::App;

    #[test]
    fn input_fields_example_builds_the_expected_markup() {
        let _ = stylesheet();
        let tree = build_ui(&InputFieldsState);

        let cssimpler::core::Node::Element(root) = tree else {
            panic!("root should be an element");
        };
        let cssimpler::core::Node::Element(form) = &root.children[0] else {
            panic!("form should be the first root child");
        };

        assert_eq!(form.children.len(), 5);
        let cssimpler::core::Node::Element(first_input) = &form.children[2] else {
            panic!("expected first input");
        };

        assert_eq!(first_input.tag, "input");
        assert_eq!(first_input.attribute("type"), Some("text"));
        assert_eq!(first_input.attribute("id"), Some("fname"));
        assert!(first_input.children.is_empty());
    }

    #[test]
    fn input_fields_example_accepts_text_in_each_field() {
        let mut app = App::new(InputFieldsState, stylesheet(), update, build_ui);

        let initial = app.frame(frame(0));
        assert_text_present(&initial, "Ada");
        assert_text_present(&initial, "Lovelace");

        focus_input(&mut app, &initial, "fname");
        let first_focused = app.frame(frame(1));
        assert_text_present(&first_focused, "Ada");
        assert_input_caret(&first_focused, "fname", 3);

        type_text(&mut app, " Marie");
        let first_typed = app.frame(frame(2));
        assert_text_present(&first_typed, "Ada Marie");
        assert_input_caret(&first_typed, "fname", 9);

        focus_input(&mut app, &first_typed, "lname");
        let second_focused = app.frame(frame(3));
        assert_text_present(&second_focused, "Ada Marie");
        assert_input_not_decorated(&second_focused, "fname");
        assert_text_present(&second_focused, "Lovelace");
        assert_input_caret(&second_focused, "lname", 8);

        type_text(&mut app, " Byron");
        let second_typed = app.frame(frame(4));
        assert_text_present(&second_typed, "Ada Marie");
        assert_text_present(&second_typed, "Lovelace Byron");
        assert_input_caret(&second_typed, "lname", 14);
    }

    #[test]
    fn input_fields_example_places_caret_and_replaces_drag_selection() {
        let mut app = App::new(InputFieldsState, stylesheet(), update, build_ui);

        let initial = app.frame(frame(0));
        press_input_after_prefix(&mut app, &initial, "fname", "A");
        release_pointer(&mut app);
        let caret_placed = app.frame(frame(1));
        assert_text_present(&caret_placed, "Ada");
        assert_input_caret(&caret_placed, "fname", 1);

        type_text(&mut app, "X");
        let inserted = app.frame(frame(2));
        assert_text_present(&inserted, "AXda");
        assert_input_caret(&inserted, "fname", 2);

        press_input_after_prefix(&mut app, &inserted, "fname", "A");
        move_pointer_after_prefix(&mut app, &inserted, "fname", "AXda");
        release_pointer(&mut app);
        let selected = app.frame(frame(3));
        assert_text_present(&selected, "AXda");
        assert_input_selection(&selected, "fname", 1, 4);

        type_text(&mut app, "lice");
        let replaced = app.frame(frame(4));
        assert_text_present(&replaced, "Alice");
        assert_input_caret(&replaced, "fname", 5);
    }

    fn focus_input(app: &mut impl SceneProvider, scene: &[RenderNode], id: &str) {
        let node = find_node_by_id(scene, id).expect("input render node should exist");
        let position = PointerPosition {
            x: node.layout.x + node.layout.width * 0.5,
            y: node.layout.y + node.layout.height * 0.5,
        };

        move_pointer(app, position, false);
        press_pointer(app, true);
        release_pointer(app);
    }

    fn press_input_after_prefix(
        app: &mut impl SceneProvider,
        scene: &[RenderNode],
        id: &str,
        prefix: &str,
    ) {
        let position = pointer_position_after_prefix(scene, id, prefix);
        move_pointer(app, position, false);
        press_pointer(app, true);
    }

    fn move_pointer_after_prefix(
        app: &mut impl SceneProvider,
        scene: &[RenderNode],
        id: &str,
        prefix: &str,
    ) {
        let position = pointer_position_after_prefix(scene, id, prefix);
        move_pointer(app, position, true);
    }

    fn pointer_position_after_prefix(
        scene: &[RenderNode],
        id: &str,
        prefix: &str,
    ) -> PointerPosition {
        let node = find_node_by_id(scene, id).expect("input render node should exist");
        let prefix_width = layout_text_block(prefix, &node.style.text, None).width;
        PointerPosition {
            x: node.layout.x + node.content_inset.left + prefix_width + 0.25,
            y: node.layout.y + node.layout.height * 0.5,
        }
    }

    fn move_pointer(app: &mut impl SceneProvider, position: PointerPosition, expect_change: bool) {
        let changed = SceneProvider::handle_engine_event(
            app,
            &EngineEvent::PointerMoved {
                position,
                modifiers: Default::default(),
            },
        );
        assert_eq!(changed, expect_change);
    }

    fn press_pointer(app: &mut impl SceneProvider, expect_change: bool) {
        let changed = SceneProvider::handle_engine_event(
            app,
            &EngineEvent::PointerButton {
                button: PointerButton::Primary,
                state: ButtonState::Pressed,
                modifiers: Default::default(),
            },
        );
        assert_eq!(changed, expect_change);
    }

    fn release_pointer(app: &mut impl SceneProvider) {
        let _ = SceneProvider::handle_engine_event(
            app,
            &EngineEvent::PointerButton {
                button: PointerButton::Primary,
                state: ButtonState::Released,
                modifiers: Default::default(),
            },
        );
    }

    fn type_text(app: &mut impl SceneProvider, text: &str) {
        assert!(SceneProvider::handle_engine_event(
            app,
            &EngineEvent::TextInput(TextInputEvent::Commit(text.to_string())),
        ));
    }

    fn frame(frame_index: u64) -> FrameInfo {
        FrameInfo {
            frame_index,
            delta: Duration::from_millis(16),
        }
    }

    fn assert_text_present(scene: &[RenderNode], expected: &str) {
        let text = text_nodes(scene);
        assert!(
            text.iter().any(|actual| actual == expected),
            "expected text node `{expected}`, got {text:?}"
        );
    }

    fn text_nodes(scene: &[RenderNode]) -> Vec<String> {
        let mut text = Vec::new();
        for node in scene {
            collect_text(node, &mut text);
        }
        text
    }

    fn collect_text(node: &RenderNode, text: &mut Vec<String>) {
        if let RenderKind::Text(content) = &node.kind {
            text.push(content.clone());
        }
        for child in &node.children {
            collect_text(child, text);
        }
    }

    fn assert_input_caret(scene: &[RenderNode], id: &str, expected: usize) {
        let edit = input_edit(scene, id);
        assert_eq!(edit.caret, Some(expected));
        assert!(edit.selection.is_none());
    }

    fn assert_input_selection(scene: &[RenderNode], id: &str, start: usize, end: usize) {
        let edit = input_edit(scene, id);
        assert!(edit.caret.is_none());
        let selection = edit
            .selection
            .as_ref()
            .expect("input should have an active selection");
        assert_eq!((selection.start, selection.end), (start, end));
        assert_eq!(selection.style.background, Color::rgb(255, 0, 153));
        assert_eq!(selection.style.foreground, Color::BLACK);
        assert!(selection.style.text_shadows.is_empty());
    }

    fn assert_input_not_decorated(scene: &[RenderNode], id: &str) {
        let node = find_node_by_id(scene, id).expect("input render node should exist");
        assert!(
            node.text_edit.is_none(),
            "input {id} should not have caret or selection decoration"
        );
    }

    fn input_edit<'a>(scene: &'a [RenderNode], id: &str) -> &'a TextEditDecoration {
        find_node_by_id(scene, id)
            .and_then(|node| node.text_edit.as_ref())
            .unwrap_or_else(|| panic!("input {id} should have text edit decoration"))
    }

    fn find_node_by_id<'a>(scene: &'a [RenderNode], id: &str) -> Option<&'a RenderNode> {
        scene
            .iter()
            .find_map(|node| find_node_by_id_in_node(node, id))
    }

    fn find_node_by_id_in_node<'a>(node: &'a RenderNode, id: &str) -> Option<&'a RenderNode> {
        if node.element_id.as_deref() == Some(id) {
            return Some(node);
        }
        node.children
            .iter()
            .find_map(|child| find_node_by_id_in_node(child, id))
    }
}
