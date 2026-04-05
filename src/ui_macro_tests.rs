use cssimpler_core::Node;

use crate::ui;

fn increment() {}

#[test]
fn ui_macro_builds_nested_nodes_from_html_like_input() {
    let count = 7_u32;
    let tree = ui! {
        <div id="app" class="card shell">
            <p class="label">
                {count}
            </p>
        </div>
    };

    match tree {
        Node::Element(root) => {
            assert_eq!(root.id.as_deref(), Some("app"));
            assert_eq!(root.classes, vec!["card".to_string(), "shell".to_string()]);
            assert_eq!(root.children.len(), 1);
        }
        Node::Text(_) => panic!("expected root element"),
    }
}

#[test]
fn ui_macro_supports_event_binding() {
    let tree = ui! {
        <button onclick={increment}>
            {"click"}
        </button>
    };

    match tree {
        Node::Element(button) => {
            assert!(button.handlers.click.is_some());
            assert_eq!(button.children.len(), 1);
        }
        Node::Text(_) => panic!("expected button element"),
    }
}

#[test]
fn ui_macro_supports_requested_mouse_event_bindings() {
    let tree = ui! {
        <button
            onclick={increment}
            oncontextmenu={increment}
            ondblclick={increment}
            onmousedown={increment}
            onmouseenter={increment}
            onmouseleave={increment}
            onmousemove={increment}
            onmouseout={increment}
            onmouseover={increment}
            onmouseup={increment}
        >
            {"click"}
        </button>
    };

    match tree {
        Node::Element(button) => {
            assert!(button.handlers.click.is_some());
            assert!(button.handlers.contextmenu.is_some());
            assert!(button.handlers.dblclick.is_some());
            assert!(button.handlers.mousedown.is_some());
            assert!(button.handlers.mouseenter.is_some());
            assert!(button.handlers.mouseleave.is_some());
            assert!(button.handlers.mousemove.is_some());
            assert!(button.handlers.mouseout.is_some());
            assert!(button.handlers.mouseover.is_some());
            assert!(button.handlers.mouseup.is_some());
        }
        Node::Text(_) => panic!("expected button element"),
    }
}

#[test]
fn ui_macro_supports_generic_and_dashed_attributes() {
    let tree = ui! {
        <button type="button" data-text="uiverse" aria-hidden="true" onclick={increment}>
            {"click"}
        </button>
    };

    match tree {
        Node::Element(button) => {
            assert_eq!(button.attribute("type"), Some("button"));
            assert_eq!(button.attribute("data-text"), Some("uiverse"));
            assert_eq!(button.attribute("aria-hidden"), Some("true"));
            assert!(button.handlers.click.is_some());
        }
        Node::Text(_) => panic!("expected button element"),
    }
}
