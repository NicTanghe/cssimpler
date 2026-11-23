use cssimpler_core::{Color, ElementNode};
use cssimpler_style::{parse_stylesheet, resolve_style};

#[test]
fn relative_alpha_is_applied_to_native_paint() {
    let source = "
        label {
            --base: rgb(20 40 60 / 50%);
            color: alpha(from rgb(20 40 60 / 50%) / calc(alpha * 0.5));
            background-color: alpha(from var(--base) / 25%);
        }
    ";
    let sheet = parse_stylesheet(source).unwrap();
    let style = resolve_style(&ElementNode::new("label"), &sheet);
    assert_eq!(style.visual.foreground, Color::rgba(20, 40, 60, 64));
    assert_eq!(style.visual.background, Some(Color::rgba(20, 40, 60, 64)));
}

#[test]
fn relative_alpha_uses_computed_current_color_even_before_color_declarations() {
    let sheet = parse_stylesheet(
        "label { --paint: alpha(from currentColor / 25%); background-color: var(--paint); color: rgb(20 40 60) }",
    ).unwrap();
    let style = resolve_style(&ElementNode::new("label"), &sheet);
    assert_eq!(style.visual.foreground, Color::rgb(20, 40, 60));
    assert_eq!(style.visual.background, Some(Color::rgba(20, 40, 60, 64)));
}

#[test]
fn color_relative_to_current_color_uses_the_inherited_color() {
    use cssimpler_core::Node;
    let sheet = parse_stylesheet(
        "panel { color: rgb(20 40 60) } label { color: alpha(from currentColor / 25%); background-color: alpha(from currentColor / alpha) }",
    ).unwrap();
    let root: Node = ElementNode::new("panel")
        .with_child(
            ElementNode::new("label")
                .with_child(Node::text("caption"))
                .into(),
        )
        .into();
    let tree = cssimpler_style::build_render_tree(&root, &sheet);
    assert_eq!(
        tree.children[0].style.foreground,
        Color::rgba(20, 40, 60, 64)
    );
    assert_eq!(
        tree.children[0].style.background,
        Some(Color::rgba(20, 40, 60, 64))
    );
}
