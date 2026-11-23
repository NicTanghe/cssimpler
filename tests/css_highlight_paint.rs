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

#[test]
fn app_updates_apply_and_clear_application_highlights() {
    use cssimpler::app::{App, HighlightRequest, HighlightSource, Invalidation};
    use cssimpler::core::ElementPath;
    use cssimpler::renderer::FrameInfo;
    use std::time::Duration;

    fn has_painted_highlight(scene: &[cssimpler::core::RenderNode]) -> bool {
        scene.iter().any(|node| {
            node.text_edit
                .as_ref()
                .is_some_and(|edit| edit.selection.is_some())
                || has_painted_highlight(&node.children)
        })
    }

    let sheet = parse_stylesheet("label::highlight(hit) { background: red }").unwrap();
    let mut app = App::new(
        vec![HighlightRequest {
            element: ElementPath::root(0),
            run_index: 0,
            range: 0..4,
            source: HighlightSource::Named("hit".into()),
        }],
        &sheet,
        |_, _| Invalidation::Paint,
        |_| {
            ElementNode::new("label")
                .with_child(Node::text("find me"))
                .into()
        },
    )
    .with_text_highlights(Vec::as_slice);
    let run = |frame_index| FrameInfo {
        frame_index,
        delta: Duration::from_millis(16),
    };
    assert!(has_painted_highlight(&app.frame(run(0))));
    app.state_mut().clear();
    assert!(!has_painted_highlight(&app.frame(run(1))));
}
