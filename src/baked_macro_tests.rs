use cssimpler_core::{
    Color, ElementInteractionState, ElementPath, Node, RuntimeDirtyClass, RuntimeNodeKind,
    RuntimeSyncAction, RuntimeSyncPolicy, RuntimeWorld,
};
use cssimpler_style::{build_render_tree_with_interaction_at_root, parse_stylesheet};

use crate::{baked_stylesheet, baked_ui, ui_prefab};

fn increment() {}

#[test]
fn baked_ui_builds_runtime_nodes_from_static_descriptors() {
    let tree = baked_ui! {
        <button id="cta" class="primary ghost" data-kind="action" onclick={increment}>
            <span>
                {"Press"}
            </span>
        </button>
    };

    let Node::Element(button) = tree else {
        panic!("expected button element");
    };
    let Node::Element(label) = &button.children[0] else {
        panic!("expected nested label element");
    };
    let Node::Text(text) = &label.children[0] else {
        panic!("expected nested text node");
    };

    assert_eq!(button.id.as_deref(), Some("cta"));
    assert_eq!(
        button.classes,
        vec!["primary".to_string(), "ghost".to_string()]
    );
    assert_eq!(button.attribute("data-kind"), Some("action"));
    assert!(button.handlers.click.is_some());
    assert_eq!(text, "Press");
}

#[test]
fn baked_stylesheet_matches_runtime_parser_for_supported_rules() {
    let source = r#"
        .card {
            display: flex;
            width: 180px;
            height: 90px;
            gap: 6px 8px;
            color: #111111;
        }

        .card:hover > .label {
            color: #2563eb;
        }
    "#;
    let tree = Node::element("div")
        .with_class("card")
        .with_child(
            Node::element("span")
                .with_class("label")
                .with_child(Node::text("value"))
                .into(),
        )
        .into();
    let interaction = ElementInteractionState {
        hovered: Some(ElementPath::root(0)),
        active: None,
    };

    let baked = baked_stylesheet!(
        r#"
        .card {
            display: flex;
            width: 180px;
            height: 90px;
            gap: 6px 8px;
            color: #111111;
        }

        .card:hover > .label {
            color: #2563eb;
        }
    "#
    );
    let runtime = parse_stylesheet(source).expect("runtime stylesheet should parse");

    let baked_render = build_render_tree_with_interaction_at_root(&tree, &baked, &interaction, 0);
    let runtime_render =
        build_render_tree_with_interaction_at_root(&tree, &runtime, &interaction, 0);

    assert_eq!(baked_render.layout.width, runtime_render.layout.width);
    assert_eq!(baked_render.layout.height, runtime_render.layout.height);
    assert_eq!(baked_render.children.len(), 1);
    assert_eq!(runtime_render.children.len(), 1);
    assert_eq!(
        baked_render.children[0].style.foreground,
        runtime_render.children[0].style.foreground
    );
    assert_eq!(
        baked_render.children[0].style.foreground,
        Color::rgb(0x25, 0x63, 0xeb)
    );
}

#[test]
fn ui_prefab_spawns_directly_into_runtime_world() {
    let prefab = ui_prefab! {
        <button id="cta" class="primary">
            <span>
                {"Prefab"}
            </span>
        </button>
    };
    let mut world = RuntimeWorld::default();

    let result = world.sync_static_root(
        0,
        prefab,
        RuntimeSyncPolicy::ForceRebuild,
        RuntimeDirtyClass::Structure,
    );

    assert_eq!(result.action, RuntimeSyncAction::Rebuilt);
    let root = world.root_entity(0).expect("root entity should exist");
    assert!(matches!(
        world.entity(root).expect("root data should exist").authored,
        RuntimeNodeKind::StaticElement(_)
    ));

    let roundtrip = world.root_as_node(0).expect("root should roundtrip");
    let Node::Element(button) = roundtrip else {
        panic!("prefab root should stay an element");
    };
    assert_eq!(button.id.as_deref(), Some("cta"));
}
