use super::{ElementNode, NativeTextInputState};

/// One step per four logical pixels, after a small click/drag threshold.
#[derive(Clone, Debug)]
pub(super) struct NumberDrag {
    origin_y: f32,
    last_y: f32,
    active: bool,
    value: f64,
    step: f64,
    min: Option<f64>,
    max: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::core::{LayoutBox, RuntimeDirtyClass, RuntimeSyncPolicy};
    use std::sync::{Arc, Mutex};

    fn moved(y: f32, shift: bool) -> EngineEvent {
        EngineEvent::PointerMoved {
            position: PointerPosition { x: 20.0, y },
            modifiers: renderer::KeyboardModifiers {
                shift,
                ..Default::default()
            },
        }
    }

    fn button(state: ButtonState) -> EngineEvent {
        EngineEvent::PointerButton {
            button: PointerButton::Primary,
            state,
            modifiers: Default::default(),
        }
    }

    #[test]
    fn number_click_types_and_drag_dispatches_bounded_fractional_values() {
        let changes = Arc::new(Mutex::new(Vec::new()));
        let captured = changes.clone();
        let root: Node = Node::element("input")
            .with_id("number")
            .with_attribute("type", "number")
            .with_attribute("value", "1")
            .with_attribute("step", "0.1")
            .with_attribute("min", "0")
            .with_attribute("max", "2")
            .on_input_shared(move |value: &str| captured.lock().unwrap().push(value.to_string()))
            .into();
        let mut inputs = NativeTextInputs::default();
        inputs.materialize_root(0, root.clone());
        let mut world = RuntimeWorld::default();
        world.sync_root(
            0,
            &root,
            RuntimeSyncPolicy::ForceRebuild,
            RuntimeDirtyClass::Structure,
        );
        let scene = vec![
            RenderNode::text(LayoutBox::new(0.0, 0.0, 100.0, 40.0), "1")
                .with_element_path(ElementPath::root(0)),
        ];
        inputs.handle_engine_event(&moved(20.0, false), Some(&scene), &world);
        inputs.handle_engine_event(&button(ButtonState::Pressed), Some(&scene), &world);
        inputs.handle_engine_event(&moved(18.0, false), Some(&scene), &world);
        assert_eq!(inputs.states["id:number"].value, "1");
        inputs.handle_engine_event(&button(ButtonState::Released), Some(&scene), &world);
        inputs.handle_engine_event(
            &EngineEvent::TextInput(TextInputEvent::Commit("2".into())),
            Some(&scene),
            &world,
        );
        assert_eq!(inputs.states["id:number"].value, "12");
        assert_eq!(changes.lock().unwrap().last().unwrap(), "12");

        // A fresh press snapshots the current edited value and clamps to the bounds.
        inputs.handle_engine_event(&button(ButtonState::Pressed), Some(&scene), &world);
        inputs.handle_engine_event(&moved(14.0, false), Some(&scene), &world);
        assert_eq!(inputs.states["id:number"].value, "2");
        inputs.handle_engine_event(&moved(18.0, false), Some(&scene), &world);
        assert_eq!(inputs.states["id:number"].value, "1.9");
        inputs.handle_engine_event(&moved(22.0, true), Some(&scene), &world);
        assert_eq!(inputs.states["id:number"].value, "0.9");
        inputs.handle_engine_event(&moved(30.0, true), Some(&scene), &world);
        assert_eq!(inputs.states["id:number"].value, "0");
        assert_eq!(changes.lock().unwrap().last().unwrap(), "0");
        inputs.handle_engine_event(&button(ButtonState::Released), Some(&scene), &world);
        inputs.handle_engine_event(&moved(0.0, false), Some(&scene), &world);
        assert_eq!(inputs.states["id:number"].value, "0");
        inputs.handle_engine_event(&moved(20.0, false), Some(&scene), &world);
        inputs.handle_engine_event(&button(ButtonState::Pressed), Some(&scene), &world);
        inputs.handle_engine_event(&moved(16.0, false), Some(&scene), &world);
        assert_eq!(inputs.states["id:number"].value, "0.1");
        inputs.handle_engine_event(&EngineEvent::FocusChanged(false), Some(&scene), &world);
        inputs.handle_engine_event(&moved(0.0, false), Some(&scene), &world);
        assert_eq!(inputs.states["id:number"].value, "0.1");
    }

    #[test]
    fn locked_number_fields_cannot_focus_or_drag() {
        for attribute in ["disabled", "readonly"] {
            let root: Node = Node::element("input")
                .with_id("locked")
                .with_attribute("type", "number")
                .with_attribute("value", "5")
                .with_attribute(attribute, "true")
                .into();
            let mut inputs = NativeTextInputs::default();
            inputs.materialize_root(0, root.clone());
            let mut world = RuntimeWorld::default();
            world.sync_root(
                0,
                &root,
                RuntimeSyncPolicy::ForceRebuild,
                RuntimeDirtyClass::Structure,
            );
            let scene = vec![
                RenderNode::text(LayoutBox::new(0.0, 0.0, 100.0, 40.0), "5")
                    .with_element_path(ElementPath::root(0)),
            ];
            inputs.handle_engine_event(&moved(20.0, false), Some(&scene), &world);
            inputs.handle_engine_event(&button(ButtonState::Pressed), Some(&scene), &world);
            inputs.handle_engine_event(&moved(0.0, true), Some(&scene), &world);
            assert!(inputs.focused.is_none());
            assert!(inputs.active_drag.is_none());
            assert_eq!(inputs.states["id:locked"].value, "5");
        }
    }
}

fn finite(value: &str) -> Option<f64> {
    value.parse::<f64>().ok().filter(|v| v.is_finite())
}

impl NumberDrag {
    pub(super) fn new(element: &ElementNode, y: f32, value: &str) -> Option<Self> {
        if !element
            .attribute("type")
            .is_some_and(|t| t.eq_ignore_ascii_case("number"))
        {
            return None;
        }
        let min = element.attribute("min").and_then(finite);
        let max = element.attribute("max").and_then(finite);
        Some(Self {
            origin_y: y,
            last_y: y,
            active: false,
            value: finite(value).unwrap_or(0.0),
            step: element
                .attribute("step")
                .and_then(finite)
                .filter(|s| *s > 0.0)
                .unwrap_or(1.0),
            min,
            // Ignore an inconsistent upper bound rather than panic while clamping.
            max: max.filter(|max| min.is_none_or(|min| *max >= min)),
        })
    }

    pub(super) fn update(&mut self, y: f32, shift: bool, state: &mut NativeTextInputState) -> bool {
        if !y.is_finite() {
            return false;
        }
        if !self.active {
            if (y - self.origin_y).abs() < 4.0 {
                return false;
            }
            self.active = true;
        }
        let steps = ((self.last_y - y) / 4.0).trunc();
        if steps == 0.0 {
            return false;
        }
        self.last_y -= steps * 4.0;
        let value = self.value + f64::from(steps) * self.step * if shift { 10.0 } else { 1.0 };
        if !value.is_finite() {
            return false;
        }
        self.value = value
            .max(self.min.unwrap_or(f64::NEG_INFINITY))
            .min(self.max.unwrap_or(f64::INFINITY));
        // Remove floating point arithmetic noise while retaining fractional steps.
        let rounded = format!("{:.14e}", self.value)
            .parse::<f64>()
            .unwrap_or(self.value);
        let value = if rounded == 0.0 {
            "0".to_string()
        } else {
            rounded.to_string()
        };
        if state.value == value {
            return false;
        }
        state.value = value;
        state.set_cursor(state.value.len());
        true
    }
}
