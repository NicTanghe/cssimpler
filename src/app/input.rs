use std::collections::HashMap;

use crate::core::{ElementNode, ElementPath, Node, RenderNode, RuntimeWorld};
use crate::renderer::{
    self, ButtonState, EngineEvent, KeyIdentity, KeyboardEvent, PointerButton, PointerPosition,
    TextInputEvent,
};

#[derive(Clone, Debug, Default)]
struct NativeTextInputState {
    value: String,
    cursor: usize,
}

#[derive(Clone, Debug, Default)]
pub(super) struct NativeTextInputs {
    states: HashMap<String, NativeTextInputState>,
    focused: Option<String>,
    pointer_position: Option<PointerPosition>,
}

impl NativeTextInputs {
    pub(super) fn materialize_root(&mut self, root_index: usize, root: Node) -> Node {
        self.materialize_node(root, Some(ElementPath::root(root_index)))
    }

    pub(super) fn handle_engine_event(
        &mut self,
        event: &EngineEvent,
        scene: Option<&[RenderNode]>,
        runtime_world: &RuntimeWorld,
    ) -> bool {
        match event {
            EngineEvent::PointerMoved { position, .. } => {
                self.pointer_position = Some(*position);
                false
            }
            EngineEvent::PointerLeft => {
                self.pointer_position = None;
                false
            }
            EngineEvent::PointerButton {
                button: PointerButton::Primary,
                state: ButtonState::Pressed,
                ..
            } => self.focus_input_at_pointer(scene, runtime_world),
            EngineEvent::TextInput(TextInputEvent::Commit(text)) => self.insert_focused_text(text),
            EngineEvent::TextInput(TextInputEvent::Preedit { .. }) => false,
            EngineEvent::Key(event) => self.handle_key_event(event),
            _ => false,
        }
    }

    fn materialize_node(&mut self, node: Node, element_path: Option<ElementPath>) -> Node {
        match node {
            Node::Text(_) => node,
            Node::Element(mut element) => {
                if let Some(path) = &element_path
                    && is_native_text_input(&element)
                {
                    let key = text_input_key(&element, path);
                    let initial_value = element.attribute("value").unwrap_or_default().to_string();
                    let state =
                        self.states
                            .entry(key.clone())
                            .or_insert_with(|| NativeTextInputState {
                                cursor: initial_value.len(),
                                value: initial_value,
                            });
                    state.cursor = clamp_to_char_boundary(&state.value, state.cursor);

                    let display_value = text_input_display_value(
                        &state.value,
                        state.cursor,
                        self.focused.as_deref() == Some(key.as_str()),
                    );
                    element.set_attribute("value", state.value.clone());
                    element.children.clear();
                    if !display_value.is_empty() {
                        element.children.push(Node::Text(display_value));
                    }
                    return Node::Element(element);
                }

                let mut child_element_index = 0;
                element.children = element
                    .children
                    .into_iter()
                    .map(|child| {
                        let child_path = if matches!(child, Node::Element(_)) {
                            let path = element_path
                                .as_ref()
                                .map(|path| path.with_child(child_element_index));
                            child_element_index += 1;
                            path
                        } else {
                            None
                        };
                        self.materialize_node(child, child_path)
                    })
                    .collect();
                Node::Element(element)
            }
        }
    }

    fn focus_input_at_pointer(
        &mut self,
        scene: Option<&[RenderNode]>,
        runtime_world: &RuntimeWorld,
    ) -> bool {
        let Some(scene) = scene else {
            return self.clear_focus();
        };
        let Some(position) = self.pointer_position else {
            return self.clear_focus();
        };
        let Some(path) = renderer::hit_test_element_path(scene, position.x, position.y) else {
            return self.clear_focus();
        };
        let Some(root) = runtime_world.root_as_node(path.root) else {
            return self.clear_focus();
        };
        let Some(element) = element_at_path(&root, &path) else {
            return self.clear_focus();
        };
        if !is_native_text_input(element) {
            return self.clear_focus();
        }

        let key = text_input_key(element, &path);
        let initial_value = element.attribute("value").unwrap_or_default().to_string();
        let state = self
            .states
            .entry(key.clone())
            .or_insert_with(|| NativeTextInputState {
                cursor: initial_value.len(),
                value: initial_value,
            });
        if self.focused.as_deref() != Some(key.as_str()) {
            state.cursor = state.value.len();
        }

        if self.focused.as_deref() == Some(key.as_str()) {
            false
        } else {
            self.focused = Some(key);
            true
        }
    }

    fn clear_focus(&mut self) -> bool {
        self.focused.take().is_some()
    }

    fn insert_focused_text(&mut self, text: &str) -> bool {
        let Some(key) = self.focused.clone() else {
            return false;
        };
        let Some(state) = self.states.get_mut(&key) else {
            return false;
        };
        let text = text
            .chars()
            .filter(|character| !character.is_control())
            .collect::<String>();
        if text.is_empty() {
            return false;
        }

        state.cursor = clamp_to_char_boundary(&state.value, state.cursor);
        state.value.insert_str(state.cursor, &text);
        state.cursor += text.len();
        true
    }

    fn handle_key_event(&mut self, event: &KeyboardEvent) -> bool {
        if event.state != ButtonState::Pressed {
            return false;
        }
        let KeyIdentity::Named(name) = &event.logical_key else {
            return false;
        };

        match name.as_str() {
            "Backspace" => self.backspace_focused(),
            "Delete" => self.delete_focused(),
            "ArrowLeft" => self.move_focused_cursor_left(),
            "ArrowRight" => self.move_focused_cursor_right(),
            "Home" => self.move_focused_cursor_home(),
            "End" => self.move_focused_cursor_end(),
            "Escape" => self.clear_focus(),
            _ => false,
        }
    }

    fn with_focused_state_mut(
        &mut self,
        action: impl FnOnce(&mut NativeTextInputState) -> bool,
    ) -> bool {
        let Some(key) = self.focused.clone() else {
            return false;
        };
        let Some(state) = self.states.get_mut(&key) else {
            return false;
        };
        state.cursor = clamp_to_char_boundary(&state.value, state.cursor);
        action(state)
    }

    fn backspace_focused(&mut self) -> bool {
        self.with_focused_state_mut(|state| {
            let Some(previous) = previous_char_boundary(&state.value, state.cursor) else {
                return false;
            };
            state.value.drain(previous..state.cursor);
            state.cursor = previous;
            true
        })
    }

    fn delete_focused(&mut self) -> bool {
        self.with_focused_state_mut(|state| {
            let Some(next) = next_char_boundary(&state.value, state.cursor) else {
                return false;
            };
            state.value.drain(state.cursor..next);
            true
        })
    }

    fn move_focused_cursor_left(&mut self) -> bool {
        self.with_focused_state_mut(|state| {
            let Some(previous) = previous_char_boundary(&state.value, state.cursor) else {
                return false;
            };
            state.cursor = previous;
            true
        })
    }

    fn move_focused_cursor_right(&mut self) -> bool {
        self.with_focused_state_mut(|state| {
            let Some(next) = next_char_boundary(&state.value, state.cursor) else {
                return false;
            };
            state.cursor = next;
            true
        })
    }

    fn move_focused_cursor_home(&mut self) -> bool {
        self.with_focused_state_mut(|state| {
            if state.cursor == 0 {
                return false;
            }
            state.cursor = 0;
            true
        })
    }

    fn move_focused_cursor_end(&mut self) -> bool {
        self.with_focused_state_mut(|state| {
            if state.cursor == state.value.len() {
                return false;
            }
            state.cursor = state.value.len();
            true
        })
    }
}

fn is_native_text_input(element: &ElementNode) -> bool {
    element.tag.eq_ignore_ascii_case("input")
        && element
            .attribute("type")
            .is_none_or(|input_type| input_type.eq_ignore_ascii_case("text"))
}

fn text_input_key(element: &ElementNode, path: &ElementPath) -> String {
    if let Some(id) = element.id.as_deref().filter(|id| !id.is_empty()) {
        return format!("id:{id}");
    }
    if let Some(name) = element.attribute("name").filter(|name| !name.is_empty()) {
        return format!("name:{name}");
    }
    format!("path:{}", element_path_key(path))
}

fn element_path_key(path: &ElementPath) -> String {
    let mut key = path.root.to_string();
    for child in &path.children {
        key.push('/');
        key.push_str(&child.to_string());
    }
    key
}

fn text_input_display_value(value: &str, cursor: usize, focused: bool) -> String {
    if !focused {
        return value.to_string();
    }

    let cursor = clamp_to_char_boundary(value, cursor);
    let mut display = String::with_capacity(value.len() + 1);
    display.push_str(&value[..cursor]);
    display.push('|');
    display.push_str(&value[cursor..]);
    display
}

fn element_at_path<'a>(root: &'a Node, path: &ElementPath) -> Option<&'a ElementNode> {
    let mut element = match root {
        Node::Element(element) => element,
        Node::Text(_) => return None,
    };

    for &target_child_index in &path.children {
        let mut child_element_index = 0;
        let mut matched = None;
        for child in &element.children {
            let Node::Element(child_element) = child else {
                continue;
            };
            if child_element_index == target_child_index {
                matched = Some(child_element);
                break;
            }
            child_element_index += 1;
        }
        element = matched?;
    }

    Some(element)
}

fn clamp_to_char_boundary(value: &str, cursor: usize) -> usize {
    if cursor >= value.len() {
        return value.len();
    }
    if value.is_char_boundary(cursor) {
        return cursor;
    }

    let mut clamped = cursor;
    while clamped > 0 && !value.is_char_boundary(clamped) {
        clamped -= 1;
    }
    clamped
}

fn previous_char_boundary(value: &str, cursor: usize) -> Option<usize> {
    let cursor = clamp_to_char_boundary(value, cursor);
    if cursor == 0 {
        None
    } else {
        value[..cursor]
            .char_indices()
            .last()
            .map(|(index, _)| index)
    }
}

fn next_char_boundary(value: &str, cursor: usize) -> Option<usize> {
    let cursor = clamp_to_char_boundary(value, cursor);
    if cursor >= value.len() {
        None
    } else {
        value[cursor..]
            .chars()
            .next()
            .map(|character| cursor + character.len_utf8())
    }
}
