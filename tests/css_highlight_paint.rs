use cssimpler::core::{Color, ElementInteractionState, ElementNode, Node, RenderKind};
use cssimpler::style::{
    TextHighlightKind, apply_text_highlight, build_render_tree, parse_stylesheet,
};

#[test]
fn a_named_highlight_paints_its_authored_background() {
    let source: Node = ElementNode::new("label")
        .with_child(Node::text("find me"))
        .into();
    let sheet = parse_stylesheet("label::highlight(hit) { background-color: red; }").unwrap();
    let mut scene = build_render_tree(&source, &sheet);
    let run = if matches!(scene.kind, RenderKind::Text(_)) {
        &mut scene
    } else {
        &mut scene.children[0]
    };
    let interaction = ElementInteractionState::default();
    assert!(apply_text_highlight(
        &source,
        &sheet,
        &interaction,
        run,
        0..4,
        TextHighlightKind::Named("hit")
    ));
    assert!(!apply_text_highlight(
        &source,
        &sheet,
        &interaction,
        run,
        0..1,
        TextHighlightKind::Search { current: true }
    ));

    let mut pixels = vec![0; 128 * 48 * 4];
    cssimpler::renderer::render_to_rgba8(&[scene], &mut pixels, 128, 48, Color::WHITE);
    assert!(
        pixels
            .chunks_exact(4)
            .any(|pixel| pixel == [255, 0, 0, 255])
    );
}
