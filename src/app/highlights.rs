use std::collections::HashMap;
use std::ops::Range;

use cssimpler_core::{ElementPath, RenderKind, RenderNode, RuntimeWorld};
use cssimpler_style::{Stylesheet, TextHighlightKind, apply_text_highlight};

/// The application-defined source of a highlight.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HighlightSource {
    Named(String),
    Search { current: bool },
}

/// A UTF-8 byte range in a prepared text run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HighlightRequest {
    pub element: ElementPath,
    /// Zero-based text run at this element path in document order.
    pub run_index: usize,
    pub range: Range<usize>,
    pub source: HighlightSource,
}

pub(super) fn decorate(
    scene: &mut [RenderNode],
    sheet: &Stylesheet,
    world: &RuntimeWorld,
    requests: &[HighlightRequest],
) {
    let mut roots = HashMap::new();
    for request in requests {
        let mut remaining = request.run_index;
        let Some(run) = find_run(scene, &request.element, &mut remaining) else {
            continue;
        };
        let Some(root) = roots
            .entry(request.element.root)
            .or_insert_with(|| world.root_as_node(request.element.root))
            .as_ref()
        else {
            continue;
        };
        let kind = match &request.source {
            HighlightSource::Named(name) => TextHighlightKind::Named(name),
            HighlightSource::Search { current } => TextHighlightKind::Search { current: *current },
        };
        apply_text_highlight(
            root,
            sheet,
            world.interaction(),
            run,
            request.range.clone(),
            kind,
        );
    }
}

fn find_run<'a>(
    scene: &'a mut [RenderNode],
    path: &ElementPath,
    remaining: &mut usize,
) -> Option<&'a mut RenderNode> {
    for node in scene {
        if node.element_path.as_ref() == Some(path) && matches!(node.kind, RenderKind::Text(_)) {
            if *remaining == 0 {
                return Some(node);
            }
            *remaining -= 1;
        }
        if let Some(run) = find_run(&mut node.children, path, remaining) {
            return Some(run);
        }
    }
    None
}
