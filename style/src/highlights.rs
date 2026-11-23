use std::ops::Range;

use cssimpler_core::{
    Color, ElementInteractionState, Node, RenderKind, RenderNode, Style, TextEditDecoration,
    TextSelectionRange, TextSelectionStyle,
};

use crate::{PseudoElementKind, Stylesheet, element_and_ancestors_at_path, resolve_style_target};

/// Source of an application-defined highlight range.
pub enum TextHighlightKind<'a> {
    /// Name of a custom CSS highlight registered by the host application.
    Named(&'a str),
    /// A result from application search, optionally the active result.
    Search { current: bool },
}

/// Style and paint a UTF-8 byte range on a prepared read-only text run.
///
/// The application supplies the range after layout, using the bytes of this
/// render node's text. This first API handles one range per run. Existing
/// text-input and document-selection decorations keep precedence.
pub fn apply_text_highlight(
    root: &Node,
    sheet: &Stylesheet,
    interaction: &ElementInteractionState,
    run: &mut RenderNode,
    range: Range<usize>,
    kind: TextHighlightKind<'_>,
) -> bool {
    let RenderKind::Text(text) = &run.kind else {
        return false;
    };
    if range.is_empty() || text.get(range.clone()).is_none() || run.text_edit.is_some() {
        return false;
    }
    let Some(path) = &run.element_path else {
        return false;
    };
    let Some((element, ancestors)) = element_and_ancestors_at_path(root, path) else {
        return false;
    };

    let base = resolve_style_target(
        element,
        sheet,
        crate::svg::seed_element_style(element),
        None,
        None,
        None,
        &ancestors,
        interaction,
        path,
        None,
    );
    let mut seed = Style::default();
    seed.visual.foreground = run.style.foreground;
    let (pseudo, current) = match kind {
        TextHighlightKind::Named(name) => (PseudoElementKind::Highlight(name.to_owned()), false),
        TextHighlightKind::Search { current } => {
            seed.visual.foreground = Color::BLACK;
            seed.visual.background = Some(if current {
                Color::rgb(255, 165, 0)
            } else {
                Color::rgb(255, 255, 0)
            });
            (PseudoElementKind::SearchText { current: false }, current)
        }
    };
    let mut highlight = resolve_style_target(
        element,
        sheet,
        seed,
        Some(&base.visual.text),
        None,
        Some(&base.custom_properties),
        &ancestors,
        interaction,
        path,
        Some(pseudo),
    );
    if current {
        highlight = resolve_style_target(
            element,
            sheet,
            highlight,
            None,
            None,
            None,
            &ancestors,
            interaction,
            path,
            Some(PseudoElementKind::SearchText { current: true }),
        );
    }

    run.text_edit = Some(TextEditDecoration {
        selection: Some(TextSelectionRange {
            start: range.start,
            end: range.end,
            style: TextSelectionStyle {
                background: highlight
                    .visual
                    .background
                    .unwrap_or(Color::rgba(0, 0, 0, 0)),
                foreground: highlight.visual.foreground,
                text_shadows: highlight.visual.text_shadows,
            },
        }),
        ..TextEditDecoration::default()
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use cssimpler_core::ElementNode;

    #[test]
    fn named_and_current_search_highlights_resolve_native_paint() {
        let sheet = crate::parse_stylesheet(
            "label::highlight(note) { background-color: red; }
             label::search-text { color: blue; background-color: yellow; }
             label:current::search-text { background-color: green; }",
        )
        .unwrap();
        let root: Node = ElementNode::new("label")
            .with_child(Node::text("café"))
            .into();
        let layout = crate::build_render_tree(&root, &sheet);
        let run = (if matches!(layout.kind, RenderKind::Text(_)) {
            &layout
        } else {
            &layout.children[0]
        })
        .clone();
        let interaction = ElementInteractionState::default();

        let mut named = run.clone();
        assert!(apply_text_highlight(
            &root,
            &sheet,
            &interaction,
            &mut named,
            0..3,
            TextHighlightKind::Named("note")
        ));
        assert_eq!(
            named.text_edit.unwrap().selection.unwrap().style.background,
            Color::rgb(255, 0, 0)
        );

        let mut current = run;
        assert!(!apply_text_highlight(
            &root,
            &sheet,
            &interaction,
            &mut current,
            3..4,
            TextHighlightKind::Search { current: true }
        ));
        assert!(apply_text_highlight(
            &root,
            &sheet,
            &interaction,
            &mut current,
            3..5,
            TextHighlightKind::Search { current: true }
        ));
        let style = current.text_edit.unwrap().selection.unwrap().style;
        assert_eq!(style.foreground, Color::rgb(0, 0, 255));
        assert_eq!(style.background, Color::rgb(0, 128, 0));
    }
}
