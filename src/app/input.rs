use std::collections::HashMap;

use crate::core::{
    ElementNode, ElementPath, Node, RenderNode, RuntimeWorld, TextEditDecoration,
    TextSelectionRange,
};
use crate::fonts::layout_text_block;
use crate::renderer::{
    self, ButtonState, EngineEvent, KeyIdentity, KeyboardEvent, PointerButton, PointerPosition,
    TextInputEvent,
};
use crate::style::{Stylesheet, resolve_selection_style_at_path};

#[derive(Clone, Debug, Default)]
struct NativeTextInputState {
    value: String,
    cursor: usize,
    selection_anchor: Option<usize>,
}

#[derive(Clone, Debug)]
struct ActiveInputDrag {
    key: String,
    path: ElementPath,
    anchor: usize,
}

#[derive(Clone, Debug)]
struct FocusedInput {
    key: String,
    path: ElementPath,
}

#[derive(Clone, Debug, Default)]
pub(super) struct NativeTextInputs {
    states: HashMap<String, NativeTextInputState>,
    focused: Option<FocusedInput>,
    pointer_position: Option<PointerPosition>,
    active_drag: Option<ActiveInputDrag>,
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
        let before = self.focused_value();
        let changed = match event {
            EngineEvent::PointerMoved { position, .. } => {
                self.pointer_position = Some(*position);
                self.update_drag_selection(scene)
            }
            EngineEvent::PointerLeft => {
                self.pointer_position = None;
                self.active_drag = None;
                false
            }
            EngineEvent::PointerButton {
                button: PointerButton::Primary,
                state: ButtonState::Pressed,
                ..
            } => self.focus_input_at_pointer(scene, runtime_world),
            EngineEvent::PointerButton {
                button: PointerButton::Primary,
                state: ButtonState::Released,
                ..
            } => self.finish_drag_selection(),
            EngineEvent::TextInput(TextInputEvent::Commit(text)) => self.insert_focused_text(text),
            EngineEvent::TextInput(TextInputEvent::Preedit { .. }) => false,
            EngineEvent::Key(event) => self.handle_key_event(event),
            _ => false,
        };

        if changed {
            self.dispatch_input_if_value_changed(before, runtime_world);
        }

        changed
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
                    let is_controlled = element.handlers.input.is_some();
                    let state =
                        self.states
                            .entry(key.clone())
                            .or_insert_with(|| NativeTextInputState {
                                cursor: initial_value.len(),
                                value: initial_value,
                                selection_anchor: None,
                            });
                    if is_controlled {
                        let authored_value =
                            element.attribute("value").unwrap_or_default().to_string();
                        if state.value != authored_value {
                            state.value = authored_value;
                        }
                    }
                    state.normalize_selection();
                    if self
                        .focused
                        .as_ref()
                        .is_some_and(|focused| focused.key == key)
                    {
                        self.focused = Some(FocusedInput {
                            key: key.clone(),
                            path: path.clone(),
                        });
                    }

                    element.set_attribute("value", state.value.clone());
                    element.children.clear();
                    if !state.value.is_empty() {
                        element.children.push(Node::Text(state.value.clone()));
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

    pub(super) fn decorate_scene(
        &self,
        scene: &mut [RenderNode],
        stylesheet: &Stylesheet,
        runtime_world: &RuntimeWorld,
    ) {
        if self.focused.is_none() {
            return;
        }

        for node in scene {
            self.decorate_node(node, stylesheet, runtime_world);
        }
    }

    fn decorate_node(
        &self,
        node: &mut RenderNode,
        stylesheet: &Stylesheet,
        runtime_world: &RuntimeWorld,
    ) {
        if let Some(path) = node.element_path.clone()
            && let Some(root) = runtime_world.root_as_node(path.root)
            && let Some(element) = element_at_path(&root, &path)
            && is_native_text_input(element)
        {
            let key = text_input_key(element, &path);
            if self
                .focused
                .as_ref()
                .is_some_and(|focused| focused.key == key)
                && let Some(state) = self.states.get(&key)
            {
                node.text_edit = Some(text_edit_decoration(
                    state,
                    resolve_selection_style_at_path(
                        &root,
                        stylesheet,
                        runtime_world.interaction(),
                        &path,
                    )
                    .unwrap_or_default(),
                ));
            }
        }

        for child in &mut node.children {
            self.decorate_node(child, stylesheet, runtime_world);
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
        let fallback_cursor = initial_value.len();
        let state = self
            .states
            .entry(key.clone())
            .or_insert_with(|| NativeTextInputState {
                cursor: fallback_cursor,
                value: initial_value,
                selection_anchor: None,
            });
        let cursor =
            caret_index_from_pointer_or_value(Some(scene), &path, &state.value, position.x)
                .unwrap_or_else(|| clamp_to_char_boundary(&state.value, fallback_cursor));
        let changed = !self
            .focused
            .as_ref()
            .is_some_and(|focused| focused.key == key && focused.path == path)
            || state.cursor != cursor
            || state.selection_anchor.is_some();

        state.set_cursor(cursor);
        self.focused = Some(FocusedInput {
            key: key.clone(),
            path: path.clone(),
        });
        self.active_drag = Some(ActiveInputDrag {
            key,
            path,
            anchor: cursor,
        });

        changed
    }

    fn update_drag_selection(&mut self, scene: Option<&[RenderNode]>) -> bool {
        let Some(drag) = self.active_drag.clone() else {
            return false;
        };
        let Some(position) = self.pointer_position else {
            return false;
        };
        let Some(state) = self.states.get_mut(&drag.key) else {
            return false;
        };
        let Some(cursor) =
            caret_index_from_pointer_or_value(scene, &drag.path, &state.value, position.x)
        else {
            return false;
        };

        let previous_cursor = state.cursor;
        let previous_anchor = state.selection_anchor;
        state.set_selection(drag.anchor, cursor);

        previous_cursor != state.cursor || previous_anchor != state.selection_anchor
    }

    fn finish_drag_selection(&mut self) -> bool {
        let Some(drag) = self.active_drag.take() else {
            return false;
        };
        let Some(state) = self.states.get_mut(&drag.key) else {
            return false;
        };
        if state.selection_range().is_none() && state.selection_anchor.take().is_some() {
            return true;
        }
        false
    }

    fn clear_focus(&mut self) -> bool {
        self.active_drag = None;
        let Some(focused) = self.focused.take() else {
            return false;
        };
        if let Some(state) = self.states.get_mut(&focused.key) {
            state.selection_anchor = None;
        }
        true
    }

    fn insert_focused_text(&mut self, text: &str) -> bool {
        let Some(focused) = self.focused.clone() else {
            return false;
        };
        let Some(state) = self.states.get_mut(&focused.key) else {
            return false;
        };
        let text = text
            .chars()
            .filter(|character| !character.is_control())
            .collect::<String>();
        if text.is_empty() {
            return false;
        }

        if let Some((start, end)) = state.selection_range() {
            state.value.replace_range(start..end, &text);
            state.cursor = start + text.len();
            state.selection_anchor = None;
        } else {
            state.cursor = clamp_to_char_boundary(&state.value, state.cursor);
            state.value.insert_str(state.cursor, &text);
            state.cursor += text.len();
        }
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
        let Some(focused) = self.focused.clone() else {
            return false;
        };
        let Some(state) = self.states.get_mut(&focused.key) else {
            return false;
        };
        state.cursor = clamp_to_char_boundary(&state.value, state.cursor);
        action(state)
    }

    fn focused_value(&self) -> Option<(String, String)> {
        let focused = self.focused.as_ref()?;
        let value = self.states.get(&focused.key)?.value.clone();
        Some((focused.key.clone(), value))
    }

    fn dispatch_input_if_value_changed(
        &self,
        before: Option<(String, String)>,
        runtime_world: &RuntimeWorld,
    ) {
        let Some(focused) = self.focused.as_ref() else {
            return;
        };
        let Some(state) = self.states.get(&focused.key) else {
            return;
        };
        if before.as_ref().map(|(key, _)| key) != Some(&focused.key) {
            return;
        }
        if before
            .as_ref()
            .is_some_and(|(_, value)| value == &state.value)
        {
            return;
        }
        let Some(root) = runtime_world.root_as_node(focused.path.root) else {
            return;
        };
        let Some(element) = element_at_path(&root, &focused.path) else {
            return;
        };
        if !is_native_text_input(element) || text_input_key(element, &focused.path) != focused.key {
            return;
        }
        if let Some(handler) = element.handlers.input {
            handler(&state.value);
        }
    }

    fn backspace_focused(&mut self) -> bool {
        self.with_focused_state_mut(|state| {
            if let Some((start, end)) = state.selection_range() {
                state.value.drain(start..end);
                state.cursor = start;
                state.selection_anchor = None;
                return true;
            }
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
            if let Some((start, end)) = state.selection_range() {
                state.value.drain(start..end);
                state.cursor = start;
                state.selection_anchor = None;
                return true;
            }
            let Some(next) = next_char_boundary(&state.value, state.cursor) else {
                return false;
            };
            state.value.drain(state.cursor..next);
            true
        })
    }

    fn move_focused_cursor_left(&mut self) -> bool {
        self.with_focused_state_mut(|state| {
            if let Some((start, _)) = state.selection_range() {
                state.cursor = start;
                state.selection_anchor = None;
                return true;
            }
            let Some(previous) = previous_char_boundary(&state.value, state.cursor) else {
                return false;
            };
            state.cursor = previous;
            true
        })
    }

    fn move_focused_cursor_right(&mut self) -> bool {
        self.with_focused_state_mut(|state| {
            if let Some((_, end)) = state.selection_range() {
                state.cursor = end;
                state.selection_anchor = None;
                return true;
            }
            let Some(next) = next_char_boundary(&state.value, state.cursor) else {
                return false;
            };
            state.cursor = next;
            true
        })
    }

    fn move_focused_cursor_home(&mut self) -> bool {
        self.with_focused_state_mut(|state| {
            if state.cursor == 0 && state.selection_anchor.is_none() {
                return false;
            }
            state.cursor = 0;
            state.selection_anchor = None;
            true
        })
    }

    fn move_focused_cursor_end(&mut self) -> bool {
        self.with_focused_state_mut(|state| {
            if state.cursor == state.value.len() && state.selection_anchor.is_none() {
                return false;
            }
            state.cursor = state.value.len();
            state.selection_anchor = None;
            true
        })
    }
}

impl NativeTextInputState {
    fn normalize_selection(&mut self) {
        self.cursor = clamp_to_char_boundary(&self.value, self.cursor);
        self.selection_anchor = self
            .selection_anchor
            .map(|anchor| clamp_to_char_boundary(&self.value, anchor))
            .filter(|anchor| *anchor != self.cursor);
    }

    fn set_cursor(&mut self, cursor: usize) {
        self.cursor = clamp_to_char_boundary(&self.value, cursor);
        self.selection_anchor = None;
    }

    fn set_selection(&mut self, anchor: usize, cursor: usize) {
        let anchor = clamp_to_char_boundary(&self.value, anchor);
        let cursor = clamp_to_char_boundary(&self.value, cursor);
        self.cursor = cursor;
        self.selection_anchor = (anchor != cursor).then_some(anchor);
    }

    fn selection_range(&self) -> Option<(usize, usize)> {
        let anchor = self.selection_anchor?;
        if anchor == self.cursor {
            None
        } else if anchor < self.cursor {
            Some((anchor, self.cursor))
        } else {
            Some((self.cursor, anchor))
        }
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

fn text_edit_decoration(
    state: &NativeTextInputState,
    selection_style: crate::core::TextSelectionStyle,
) -> TextEditDecoration {
    let selection = state
        .selection_range()
        .map(|(start, end)| TextSelectionRange {
            start,
            end,
            style: selection_style,
        });

    TextEditDecoration {
        caret: selection.is_none().then_some(state.cursor),
        selection,
    }
}

fn caret_index_from_pointer_or_value(
    scene: Option<&[RenderNode]>,
    path: &ElementPath,
    value: &str,
    x: f32,
) -> Option<usize> {
    scene
        .and_then(|scene| find_render_node_at_path(scene, path))
        .map(|node| caret_index_from_pointer(node, value, x))
}

fn caret_index_from_pointer(node: &RenderNode, value: &str, x: f32) -> usize {
    if value.is_empty() {
        return 0;
    }

    let content_x = x - node.layout.x - node.content_inset.left;
    if content_x <= 0.0 {
        return 0;
    }

    let full_width = layout_text_block(value, &node.style.text, None).width;
    if content_x >= full_width {
        return value.len();
    }

    let mut best_index = 0;
    let mut best_distance = content_x.abs();
    for boundary in char_boundaries(value).into_iter().skip(1) {
        let width = layout_text_block(&value[..boundary], &node.style.text, None).width;
        let distance = (width - content_x).abs();
        if distance < best_distance {
            best_index = boundary;
            best_distance = distance;
        }
    }

    best_index
}

fn char_boundaries(value: &str) -> Vec<usize> {
    let mut boundaries = Vec::with_capacity(value.chars().count() + 1);
    boundaries.push(0);
    for (index, character) in value.char_indices() {
        boundaries.push(index + character.len_utf8());
    }
    boundaries
}

fn find_render_node_at_path<'a>(
    scene: &'a [RenderNode],
    path: &ElementPath,
) -> Option<&'a RenderNode> {
    scene
        .iter()
        .find_map(|node| find_render_node_at_path_node(node, path))
}

fn find_render_node_at_path_node<'a>(
    node: &'a RenderNode,
    path: &ElementPath,
) -> Option<&'a RenderNode> {
    if node.element_path.as_ref() == Some(path) {
        return Some(node);
    }

    node.children
        .iter()
        .find_map(|child| find_render_node_at_path_node(child, path))
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
