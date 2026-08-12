use std::collections::HashMap;

use crate::core::{
    ElementNode, ElementPath, Node, RenderNode, RuntimeWorld, TextEditDecoration,
    TextSelectionRange,
};
use crate::fonts::{TextCaretStop, layout_text_caret_stops};
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
        let Some(focused) = self.focused.as_ref() else {
            return;
        };
        let Some(state) = self.states.get(&focused.key) else {
            return;
        };

        let mut authored_roots = HashMap::new();
        for node in scene.iter() {
            cache_authored_roots(node, runtime_world, &mut authored_roots);
        }

        for node in scene {
            Self::decorate_node(
                node,
                stylesheet,
                runtime_world,
                &authored_roots,
                &focused.key,
                state,
            );
        }
    }

    fn decorate_node(
        node: &mut RenderNode,
        stylesheet: &Stylesheet,
        runtime_world: &RuntimeWorld,
        authored_roots: &HashMap<usize, Option<Node>>,
        focused_key: &str,
        state: &NativeTextInputState,
    ) {
        if let Some(path) = node.element_path.as_ref()
            && let Some(root) = authored_roots.get(&path.root).and_then(Option::as_ref)
            && let Some(element) = element_at_path(root, path)
            && is_native_text_input(element)
            && text_input_key(element, path) == focused_key
        {
            node.text_edit = Some(text_edit_decoration(
                state,
                resolve_selection_style_at_path(
                    root,
                    stylesheet,
                    runtime_world.interaction(),
                    path,
                )
                .unwrap_or_default(),
            ));
        }

        for child in &mut node.children {
            Self::decorate_node(
                child,
                stylesheet,
                runtime_world,
                authored_roots,
                focused_key,
                state,
            );
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

fn cache_authored_roots(
    node: &RenderNode,
    runtime_world: &RuntimeWorld,
    authored_roots: &mut HashMap<usize, Option<Node>>,
) {
    if let Some(path) = node.element_path.as_ref() {
        authored_roots
            .entry(path.root)
            .or_insert_with(|| runtime_world.root_as_node(path.root));
    }

    for child in &node.children {
        cache_authored_roots(child, runtime_world, authored_roots);
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

    let caret_stops = layout_text_caret_stops(value, &node.style.text);
    let full_width = caret_stops.last().map(|stop| stop.offset_px).unwrap_or(0.0);
    if content_x >= full_width {
        return value.len();
    }

    nearest_caret_stop(&caret_stops, content_x)
}

fn nearest_caret_stop(stops: &[TextCaretStop], x: f32) -> usize {
    let offsets_are_ordered = stops
        .windows(2)
        .all(|pair| pair[0].offset_px <= pair[1].offset_px);
    if !offsets_are_ordered {
        return stops
            .iter()
            .fold((0, x.abs()), |(best_index, best_distance), stop| {
                let distance = (stop.offset_px - x).abs();
                if distance < best_distance {
                    (stop.byte_index, distance)
                } else {
                    (best_index, best_distance)
                }
            })
            .0;
    }

    let right = stops.partition_point(|stop| stop.offset_px < x);
    let Some(right_stop) = stops.get(right) else {
        return stops.last().map_or(0, |stop| stop.byte_index);
    };
    if right == 0 {
        return right_stop.byte_index;
    }

    let left_offset = stops[right - 1].offset_px;
    // Collapsed whitespace can give several source positions the same visual offset. Preserve the
    // previous first-match behavior by choosing the earliest position in that equal-offset run.
    let left = stops[..right].partition_point(|stop| stop.offset_px < left_offset);
    let left_stop = &stops[left];
    if (right_stop.offset_px - x).abs() < (left_stop.offset_px - x).abs() {
        right_stop.byte_index
    } else {
        left_stop.byte_index
    }
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

#[cfg(test)]
mod tests {
    use crate::core::{
        Color, LayoutBox, RuntimeDirtyClass, RuntimeSyncPolicy, TextEditDecoration,
        TextSelectionStyle,
    };
    use crate::fonts::{
        FontFamily, TextStyle, TextTransform, WhiteSpace, layout_text_block,
        layout_text_caret_stops,
    };
    use crate::style::parse_stylesheet;

    use super::*;

    #[test]
    fn caret_hit_testing_matches_repeated_prefix_layout_reference() {
        let value = "  A\u{301} Stra\u{df}e \r\n\u{130} \u{1f469}\u{200d}\u{1f680}  ";
        let mut node = RenderNode::text(LayoutBox::new(13.0, 0.0, 400.0, 24.0), value);
        node.content_inset.left = 7.0;
        let origin = node.layout.x + node.content_inset.left;

        for white_space in [
            WhiteSpace::Normal,
            WhiteSpace::NoWrap,
            WhiteSpace::Pre,
            WhiteSpace::PreWrap,
        ] {
            for text_transform in [
                TextTransform::None,
                TextTransform::Uppercase,
                TextTransform::Lowercase,
                TextTransform::Capitalize,
            ] {
                for letter_spacing_px in [0.0, 1.25] {
                    node.style.text = TextStyle {
                        families: vec![FontFamily::Named(
                            "cssimpler-missing-font-for-caret-hit-tests".to_string(),
                        )],
                        letter_spacing_px,
                        text_transform,
                        white_space,
                        ..TextStyle::default()
                    };

                    let mut prefix_widths = vec![0.0];
                    prefix_widths.extend(value.char_indices().map(|(index, character)| {
                        layout_text_block(
                            &value[..index + character.len_utf8()],
                            &node.style.text,
                            None,
                        )
                        .width
                    }));
                    let full_width = *prefix_widths.last().unwrap_or(&0.0);
                    let mut candidates = vec![-1.0, 0.0, 0.25, full_width, full_width + 1.0];
                    candidates.extend(prefix_widths.iter().copied());
                    candidates.extend(
                        prefix_widths
                            .windows(2)
                            .map(|pair| (pair[0] + pair[1]) * 0.5),
                    );

                    for content_x in candidates {
                        let x = origin + content_x;
                        assert_eq!(
                            caret_index_from_pointer(&node, value, x),
                            reference_caret_index_from_pointer(&node, value, x),
                            "caret mismatch at x={content_x} for {text_transform:?}/{white_space:?}/spacing={letter_spacing_px}",
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn long_unicode_caret_hit_keeps_source_utf8_character_index() {
        let unit = "a\u{301}\u{1f469}\u{200d}\u{1f680}\u{df}";
        let value = unit.repeat(2_048);
        let mut node = RenderNode::text(LayoutBox::new(0.0, 0.0, 200_000.0, 24.0), &value);
        node.style.text = TextStyle {
            families: vec![FontFamily::Named(
                "cssimpler-missing-font-for-long-caret-hit-test".to_string(),
            )],
            text_transform: TextTransform::Uppercase,
            white_space: WhiteSpace::Pre,
            ..TextStyle::default()
        };
        let expected_index = unit.len() * 1_337 + 'a'.len_utf8();
        let stops = layout_text_caret_stops(&value, &node.style.text);
        let target = stops
            .iter()
            .find(|stop| stop.byte_index == expected_index)
            .expect("the combining sequence's scalar boundary remains a legal caret position");

        assert_eq!(
            caret_index_from_pointer(&node, &value, target.offset_px),
            expected_index
        );
        assert!(value.is_char_boundary(expected_index));
    }

    #[test]
    fn nearest_caret_stop_preserves_first_match_tie_semantics() {
        let duplicate_offsets = [
            TextCaretStop {
                byte_index: 0,
                offset_px: 0.0,
            },
            TextCaretStop {
                byte_index: 1,
                offset_px: 10.0,
            },
            TextCaretStop {
                byte_index: 2,
                offset_px: 10.0,
            },
            TextCaretStop {
                byte_index: 3,
                offset_px: 20.0,
            },
        ];
        assert_eq!(nearest_caret_stop(&duplicate_offsets, 10.0), 1);
        assert_eq!(nearest_caret_stop(&duplicate_offsets, 12.0), 1);
        assert_eq!(nearest_caret_stop(&duplicate_offsets, 15.0), 1);

        let unordered_offsets = [
            TextCaretStop {
                byte_index: 0,
                offset_px: 0.0,
            },
            TextCaretStop {
                byte_index: 1,
                offset_px: 8.0,
            },
            TextCaretStop {
                byte_index: 2,
                offset_px: 4.0,
            },
            TextCaretStop {
                byte_index: 3,
                offset_px: 12.0,
            },
        ];
        assert_eq!(nearest_caret_stop(&unordered_offsets, 6.0), 1);
    }

    fn reference_caret_index_from_pointer(node: &RenderNode, value: &str, x: f32) -> usize {
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
        for (index, character) in value.char_indices() {
            let boundary = index + character.len_utf8();
            let width = layout_text_block(&value[..boundary], &node.style.text, None).width;
            let distance = (width - content_x).abs();
            if distance < best_distance {
                best_index = boundary;
                best_distance = distance;
            }
        }
        best_index
    }

    #[test]
    fn decorate_scene_preserves_duplicate_key_and_per_path_selection_semantics() {
        let shared_input = || {
            Node::element("input")
                .with_attribute("type", "text")
                .with_attribute("name", "shared")
                .with_attribute("value", "hello")
                .into()
        };
        let other_input: Node = Node::element("input")
            .with_attribute("type", "text")
            .with_attribute("name", "other")
            .with_attribute("value", "other")
            .into();
        let first_root: Node = Node::element("main")
            .with_child(
                Node::element("section")
                    .with_class("left")
                    .with_child(shared_input())
                    .into(),
            )
            .with_child(
                Node::element("section")
                    .with_class("right")
                    .with_child(shared_input())
                    .into(),
            )
            .with_child(
                Node::element("section")
                    .with_class("other")
                    .with_child(other_input)
                    .into(),
            )
            .into();
        let second_root: Node = Node::element("section")
            .with_class("second-root")
            .with_child(shared_input())
            .into();

        let mut runtime_world = RuntimeWorld::default();
        runtime_world.sync_root(
            0,
            &first_root,
            RuntimeSyncPolicy::ForceRebuild,
            RuntimeDirtyClass::Structure,
        );
        runtime_world.sync_root(
            1,
            &second_root,
            RuntimeSyncPolicy::ForceRebuild,
            RuntimeDirtyClass::Structure,
        );

        let left_path = ElementPath::root(0).with_child(0).with_child(0);
        let right_path = ElementPath::root(0).with_child(1).with_child(0);
        let other_path = ElementPath::root(0).with_child(2).with_child(0);
        let second_root_path = ElementPath::root(1).with_child(0);
        let render_input = |path| {
            RenderNode::text(LayoutBox::new(0.0, 0.0, 80.0, 20.0), "hello").with_element_path(path)
        };
        let mut scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 240.0, 80.0))
                .with_element_path(ElementPath::root(0))
                .with_child(
                    RenderNode::container(LayoutBox::new(0.0, 0.0, 80.0, 20.0))
                        .with_element_path(ElementPath::root(0).with_child(0))
                        .with_child(render_input(left_path.clone())),
                )
                .with_child(
                    RenderNode::container(LayoutBox::new(80.0, 0.0, 80.0, 20.0))
                        .with_element_path(ElementPath::root(0).with_child(1))
                        .with_child(render_input(right_path.clone())),
                )
                .with_child(
                    RenderNode::container(LayoutBox::new(160.0, 0.0, 80.0, 20.0))
                        .with_element_path(ElementPath::root(0).with_child(2))
                        .with_child(render_input(other_path.clone())),
                ),
            RenderNode::container(LayoutBox::new(0.0, 30.0, 80.0, 20.0))
                .with_element_path(ElementPath::root(1))
                .with_child(render_input(second_root_path.clone())),
        ];
        let stylesheet = parse_stylesheet(
            ".left input::selection { background: #ff0000; color: #ffffff; }
             .right input::selection { background: #00ff00; color: #000000; }
             .second-root input::selection { background: #0000ff; color: #ffffff; }",
        )
        .expect("selection stylesheet should parse");
        let shared_key = "name:shared".to_string();
        let mut inputs = NativeTextInputs::default();
        inputs.states.insert(
            shared_key.clone(),
            NativeTextInputState {
                value: "hello".to_string(),
                cursor: 4,
                selection_anchor: Some(1),
            },
        );
        inputs.focused = Some(FocusedInput {
            key: shared_key,
            path: left_path.clone(),
        });

        inputs.decorate_scene(&mut scene, &stylesheet, &runtime_world);

        assert_selection(
            find_render_node_at_path(&scene, &left_path).and_then(|node| node.text_edit.as_ref()),
            Color::rgb(255, 0, 0),
            Color::WHITE,
        );
        assert_selection(
            find_render_node_at_path(&scene, &right_path).and_then(|node| node.text_edit.as_ref()),
            Color::rgb(0, 255, 0),
            Color::BLACK,
        );
        assert_selection(
            find_render_node_at_path(&scene, &second_root_path)
                .and_then(|node| node.text_edit.as_ref()),
            Color::rgb(0, 0, 255),
            Color::WHITE,
        );
        assert!(
            find_render_node_at_path(&scene, &other_path)
                .is_some_and(|node| node.text_edit.is_none())
        );
    }

    #[test]
    fn decorate_scene_preserves_focused_caret_semantics() {
        let input: Node = Node::element("input")
            .with_id("field")
            .with_attribute("type", "text")
            .with_attribute("value", "hello")
            .into();
        let mut runtime_world = RuntimeWorld::default();
        runtime_world.sync_root(
            0,
            &input,
            RuntimeSyncPolicy::ForceRebuild,
            RuntimeDirtyClass::Structure,
        );
        let path = ElementPath::root(0);
        let mut scene = vec![
            RenderNode::text(LayoutBox::new(0.0, 0.0, 80.0, 20.0), "hello")
                .with_element_path(path.clone()),
        ];
        let mut inputs = NativeTextInputs::default();
        inputs.states.insert(
            "id:field".to_string(),
            NativeTextInputState {
                value: "hello".to_string(),
                cursor: 3,
                selection_anchor: None,
            },
        );
        inputs.focused = Some(FocusedInput {
            key: "id:field".to_string(),
            path: path.clone(),
        });

        inputs.decorate_scene(&mut scene, &Stylesheet::default(), &runtime_world);

        assert_eq!(
            find_render_node_at_path(&scene, &path).and_then(|node| node.text_edit.as_ref()),
            Some(&TextEditDecoration {
                caret: Some(3),
                selection: None,
            })
        );
    }

    fn assert_selection(
        decoration: Option<&TextEditDecoration>,
        background: Color,
        foreground: Color,
    ) {
        let selection = decoration
            .and_then(|decoration| decoration.selection.as_ref())
            .expect("matching focused input should have a selection decoration");
        assert_eq!((selection.start, selection.end), (1, 4));
        assert_eq!(
            selection.style,
            TextSelectionStyle {
                background,
                foreground,
                text_shadows: Vec::new(),
            }
        );
    }
}
