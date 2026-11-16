use cssimpler_core::Node;

use crate::ui;

fn increment() {}
fn capture_input(_value: &str) {}

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
            click
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
fn ui_macro_supports_input_event_binding() {
    let tree = ui! {
        <input type="text" id="q" value="Bob" oninput={capture_input}>
    };

    match tree {
        Node::Element(input) => {
            assert!(input.handlers.input.is_some());
            assert_eq!(input.attribute("value"), Some("Bob"));
        }
        Node::Text(_) => panic!("expected input element"),
    }
}

#[test]
fn ui_macro_supports_plain_text_children_without_braces() {
    let tree = ui! {
        <h1>This is a Heading</h1>
    };

    match tree {
        Node::Element(heading) => {
            assert_eq!(heading.children.len(), 1);
            let Node::Text(text) = &heading.children[0] else {
                panic!("expected heading text node");
            };
            assert_eq!(text, "This is a Heading");
        }
        Node::Text(_) => panic!("expected heading element"),
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
            click
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
            click
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

#[test]
fn ui_macro_accepts_html_void_input_without_self_closing_slash() {
    let tree = ui! {
        <form>
            <label for="fname">First Name</label>
            <input type="text" id="fname" name="fname">
            <label for="lname">Last Name</label>
        </form>
    };

    match tree {
        Node::Element(form) => {
            assert_eq!(form.children.len(), 3);
            let Node::Element(input) = &form.children[1] else {
                panic!("expected input element");
            };
            assert_eq!(input.tag, "input");
            assert_eq!(input.attribute("type"), Some("text"));
            assert!(input.children.is_empty());
        }
        Node::Text(_) => panic!("expected form element"),
    }
}

#[test]
fn ui_macro_dynamic_class_lists_match_literal_lists() {
    let classes = String::from("card\tselected\nactive");
    let empty = "  ";
    let dynamic =
        ui! { <button class="base" class={classes} class={empty} onclick={increment}></button> };
    let literal = ui! { <button class="base card selected active" onclick={increment}></button> };
    let (Node::Element(dynamic), Node::Element(literal)) = (dynamic, literal) else {
        panic!("expected buttons");
    };
    assert_eq!(dynamic.classes, literal.classes);
    assert_eq!(dynamic.attribute("class"), literal.attribute("class"));
    assert!(dynamic.handlers.click.is_some());
}

#[test]
fn dynamic_lists_and_optional_children_are_flattened_without_wrapper_nodes() {
    let rows = vec![crate::ui! {<p>One</p>}, crate::ui! {<p>Two</p>}];
    let tree = crate::ui! {<section>{rows}{Some(crate::ui!{<button>Save</button>})}{None::<crate::core::Node>}</section>};
    let crate::core::Node::Element(root) = tree else {
        panic!("section expected")
    };
    assert_eq!(root.children.len(), 3);
    for (child, tag) in root.children.iter().zip(["p", "p", "button"]) {
        let crate::core::Node::Element(element) = child else {
            panic!("element expected")
        };
        assert_eq!(element.tag, tag);
    }
}

#[test]
fn captured_handlers_keep_instance_state_separate() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    fn button(count: &Arc<AtomicUsize>) -> Node {
        let count = Arc::clone(count);
        ui! { <button onclick_shared={move || { count.fetch_add(1, Ordering::Relaxed); }}>click</button> }
    }

    let first = Arc::new(AtomicUsize::new(0));
    let second = Arc::new(AtomicUsize::new(0));
    let Node::Element(first_button) = button(&first) else {
        panic!("expected button")
    };
    let _second_button = button(&second);
    first_button.handlers.click().unwrap()();
    assert_eq!(first.load(Ordering::Relaxed), 1);
    assert_eq!(second.load(Ordering::Relaxed), 0);
}

#[test]
fn changing_handler_kind_replaces_the_previous_handler() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let count = Arc::new(AtomicUsize::new(0));
    let captured = count.clone();
    let node = cssimpler_core::Node::element("button").on_click_shared(move || {
        captured.fetch_add(1, Ordering::Relaxed);
    });
    let cloned = node.clone();
    let replaced = node.on_click(increment);
    replaced.handlers.click().unwrap()();
    assert_eq!(count.load(Ordering::Relaxed), 0);
    cloned.handlers.click().unwrap()();
    assert_eq!(count.load(Ordering::Relaxed), 1);
    let captured = count.clone();
    let replaced = replaced.on_click_shared(move || {
        captured.fetch_add(1, Ordering::Relaxed);
    });
    replaced.handlers.click().unwrap()();
    assert_eq!(count.load(Ordering::Relaxed), 2);
}

#[test]
fn cached_layer_request_reaches_the_render_tree_without_inheriting_to_children() {
    let tree = Node::element("div")
        .with_cached_layer(true)
        .with_child(Node::element("div").with_child(Node::text("board")).into())
        .into();
    let rendered =
        cssimpler_style::build_render_tree(&tree, &cssimpler_style::Stylesheet::default());
    assert!(rendered.style.cache_layer);
    assert!(cssimpler_core::establishes_stacking_context(&rendered));
    assert!(!rendered.children[0].style.cache_layer);
}

fn native_curve_scene(end_x: f32) -> cssimpler_core::VectorScene {
    use cssimpler_core::{
        PathCommand::*, VectorPaint, VectorPath, VectorPoint as P, VectorScene, VectorViewBox,
    };
    VectorScene::new(
        VectorViewBox::new(0., 0., 40., 40.),
        vec![
            VectorPath::new(
                vec![
                    MoveTo(P::new(4., 30.)),
                    CubicTo {
                        control1: P::new(4., 4.),
                        control2: P::new(end_x, 4.),
                        end: P::new(end_x, 30.),
                    },
                    Close,
                ],
                VectorPaint::default(),
            )
            .unwrap(),
        ],
    )
    .unwrap()
}

#[test]
fn native_vector_survives_runtime_and_paint_refresh_with_curves_intact() {
    use cssimpler_core::{ElementPath, RenderKind, RuntimeSyncPolicy, RuntimeWorld};
    let tree: Node = Node::vector(native_curve_scene(36.))
        .with_id("native")
        .into();
    let mut world = RuntimeWorld::default();
    world.sync_root(
        0,
        &tree,
        RuntimeSyncPolicy::ForceRebuild,
        cssimpler_core::RuntimeDirtyClass::Structure,
    );
    let changed: Node = Node::vector(native_curve_scene(24.))
        .with_id("native")
        .into();
    world.sync_root(
        0,
        &changed,
        RuntimeSyncPolicy::PreferPatch,
        cssimpler_core::RuntimeDirtyClass::Paint,
    );
    let restored = world.root_as_node(0).unwrap();
    let rendered =
        cssimpler_style::build_render_tree(&restored, &cssimpler_style::Stylesheet::default());
    assert_eq!(rendered.layout.width, 40.);
    assert_eq!(rendered.element_path, Some(ElementPath::root(0)));
    let RenderKind::Svg(svg) = rendered.kind else {
        panic!("expected shared vector rendering leaf")
    };
    assert_eq!(svg.paths[0].geometry.commands.len(), 3);
    assert_eq!(
        svg.paths[0].geometry.commands[1],
        native_curve_scene(24.).paths()[0].commands()[1]
    );
    assert!(svg.paths[0].geometry.contours.is_empty());
}

#[test]
fn native_curves_paint_and_incremental_updates_match_full_redraw() {
    use cssimpler_core::{Color, LayoutBox, RenderNode};
    use cssimpler_renderer::render_to_buffer;
    let previous = vec![RenderNode::vector(
        LayoutBox::new(0., 0., 40., 40.),
        native_curve_scene(36.),
    )];
    let next = vec![RenderNode::vector(
        LayoutBox::new(0., 0., 40., 40.),
        native_curve_scene(24.),
    )];
    let mut old = vec![0; 1600];
    let mut new = old.clone();
    render_to_buffer(&previous, &mut old, 40, 40, Color::WHITE);
    render_to_buffer(&next, &mut new, 40, 40, Color::WHITE);
    assert_ne!(old, new);
    let mut incremental = old.clone();
    cssimpler_renderer::render_scene_update(
        &previous,
        &next,
        &mut incremental,
        40,
        40,
        Color::WHITE,
    );
    assert!(
        incremental == new,
        "native curve damage should match full rendering"
    );
    // This point is inside the curve, whereas the endpoint-only chord is at y=30.
    assert_ne!(old[15 * 40 + 20], old[0]);
    assert_eq!(old[2 * 40 + 20], old[0]);
}
