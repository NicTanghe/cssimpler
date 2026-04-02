use std::sync::OnceLock;

use anyhow::Result;
use cssimpler::app::{App, Invalidation};
use cssimpler::core::Node;
use cssimpler::renderer::{FrameInfo, WindowConfig};
use cssimpler::style::{Stylesheet, parse_stylesheet};
use cssimpler::ui;

const BUTTON_TEXT: &str = "uiverse";

fn main() -> Result<()> {
    let config = WindowConfig::new("cssimpler / uiverse hover button", 1280, 720);

    App::new((), stylesheet(), update, build_ui)
        .run(config)
        .map_err(Into::into)
}

fn update(_state: &mut (), _frame: FrameInfo) -> Invalidation {
    Invalidation::Clean
}

fn build_ui(_state: &()) -> Node {
    ui! {
        <div id="app">
            <section class="spotlight">
                <p class="kicker">
                    {"Uiverse-inspired hover reveal"}
                </p>
                {build_button()}
                <p class="note">
                    {"The outlined label stays centered while the neon fill sweeps across on hover."}
                </p>
            </section>
        </div>
    }
}

fn build_button() -> Node {
    ui! {
        <button class="button" type="button">
            <span class="actual-text">
                <span class="actual-label">
                    <span class="actual-label-text">
                        {BUTTON_TEXT}
                    </span>
                </span>
            </span>
            <span class="hover-text">
                <span class="hover-fill" aria-hidden="true">
                    <span class="hover-label">
                        <span class="hover-label-text">
                            {BUTTON_TEXT}
                        </span>
                    </span>
                </span>
            </span>
        </button>
    }
}

fn stylesheet() -> &'static Stylesheet {
    static STYLESHEET: OnceLock<Stylesheet> = OnceLock::new();

    STYLESHEET.get_or_init(|| {
        parse_stylesheet(include_str!("uiverse_hover_button.css"))
            .expect("uiverse hover button stylesheet should stay valid")
    })
}

#[cfg(test)]
mod tests {
    use super::{BUTTON_TEXT, build_ui, stylesheet};
    use cssimpler::app::{App, Invalidation};
    use cssimpler::core::fonts::layout_text_block;
    use cssimpler::core::{ElementInteractionState, ElementPath, Node, RenderKind, RenderNode};
    use cssimpler::renderer::FrameInfo;
    use cssimpler::style::{build_render_tree_in_viewport_with_interaction, parse_stylesheet};
    use cssimpler::ui;
    use std::time::Duration;

    #[test]
    fn hover_mask_expands_to_cover_the_button() {
        let tree = build_ui(&());
        let idle = build_render_tree_in_viewport_with_interaction(
            &tree,
            stylesheet(),
            1280,
            720,
            &ElementInteractionState::default(),
        );
        let hovered = build_render_tree_in_viewport_with_interaction(
            &tree,
            stylesheet(),
            1280,
            720,
            &ElementInteractionState {
                hovered: Some(ElementPath::root(0).with_child(0).with_child(1)),
                active: None,
            },
        );

        let idle_mask = hover_mask(&idle);
        let hovered_mask = hover_mask(&hovered);
        let hovered_button = button(&hovered);

        assert_eq!(idle_mask.layout.width, idle_mask.style.border.widths.right);
        assert!((hovered_mask.layout.width - hovered_button.layout.width).abs() < 0.01);
        assert_eq!(hovered_mask.style.border.widths.right, 4.0);
        assert_eq!(hovered_mask.children.len(), 1);
        assert!((hovered_mask.children[0].layout.width - hovered_button.layout.width).abs() < 0.01);
        assert!(matches!(
            &hovered_mask.children[0].children[0].children[0].kind,
            RenderKind::Text(content) if content == BUTTON_TEXT
        ));
    }

    fn button(root: &RenderNode) -> &RenderNode {
        &root.children[0].children[1]
    }

    fn hover_mask(root: &RenderNode) -> &RenderNode {
        &button(root).children[1]
    }

    #[test]
    fn reveal_transition_keeps_the_hover_label_on_one_line() {
        let stylesheet = parse_stylesheet(
            r#"
            .button {
              width: 320px;
              height: 88px;
              display: flex;
              justify-content: center;
              align-items: center;
              position: relative;
              font-size: 44px;
              font-weight: 700;
              line-height: 1;
              letter-spacing: 2px;
              text-transform: uppercase;
            }

            .actual-text {
              display: flex;
              width: 320px;
              height: 88px;
              justify-content: center;
              align-items: center;
            }

            .actual-label {
              display: flex;
              width: 320px;
              height: 88px;
              justify-content: center;
              align-items: center;
              flex-shrink: 0;
            }

            .actual-label-text {
              display: block;
              width: 252px;
              flex-shrink: 0;
            }

            .hover-text {
              width: 0px;
              height: 88px;
              position: absolute;
              inset: 0;
              overflow: hidden;
              border-right: 4px solid #37ff8b;
              transition: width 32ms linear;
            }

            .button.hot .hover-text {
              width: 100%;
            }

            .hover-fill {
              display: flex;
              width: 320px;
              height: 88px;
              justify-content: center;
              align-items: center;
            }

            .hover-label {
              display: flex;
              width: 320px;
              height: 88px;
              justify-content: center;
              align-items: center;
              flex-shrink: 0;
            }

            .hover-label-text {
              display: block;
              width: 252px;
              flex-shrink: 0;
            }
            "#,
        )
        .expect("stylesheet should parse");

        let mut app = App::new(
            false,
            &stylesheet,
            |state, frame| {
                if frame.frame_index == 1 {
                    *state = true;
                    Invalidation::Layout
                } else {
                    Invalidation::Clean
                }
            },
            |state| {
                if *state {
                    ui! {
                        <div>
                            {build_test_button(true)}
                        </div>
                    }
                } else {
                    ui! {
                        <div>
                            {build_test_button(false)}
                        </div>
                    }
                }
            },
        );

        let _first = app.frame(frame(0));
        let second = app.frame(frame(1));
        let _third = app.frame(frame(2));

        let mid_button = &second[0].children[0];
        let mid_mask = &mid_button.children[1];
        let hover_fill = &mid_mask.children[0];
        let hover_label = &hover_fill.children[0];
        let hover_label_text = &hover_label.children[0];

        assert!(mid_mask.layout.width > 4.0);
        assert!(mid_mask.layout.width < mid_button.layout.width);
        assert!((hover_fill.layout.width - mid_button.layout.width).abs() < 0.01);
        assert!((hover_label.layout.width - mid_button.layout.width).abs() < 0.01);
        assert!(matches!(
            &hover_label_text.kind,
            RenderKind::Text(content) if content == BUTTON_TEXT
        ));

        let text_layout = layout_text_block(
            BUTTON_TEXT,
            &hover_label_text.style.text,
            Some(hover_label_text.layout.width.max(1.0)),
        );
        assert_eq!(text_layout.lines.len(), 1);
    }

    fn build_test_button(is_hot: bool) -> Node {
        if is_hot {
            ui! {
                <button class="button hot">
                    <span class="actual-text">
                        <span class="actual-label">
                            <span class="actual-label-text">
                                {BUTTON_TEXT}
                            </span>
                        </span>
                    </span>
                    <span class="hover-text">
                        <span class="hover-fill">
                            <span class="hover-label">
                                <span class="hover-label-text">
                                    {BUTTON_TEXT}
                                </span>
                            </span>
                        </span>
                    </span>
                </button>
            }
        } else {
            ui! {
                <button class="button">
                    <span class="actual-text">
                        <span class="actual-label">
                            <span class="actual-label-text">
                                {BUTTON_TEXT}
                            </span>
                        </span>
                    </span>
                    <span class="hover-text">
                        <span class="hover-fill">
                            <span class="hover-label">
                                <span class="hover-label-text">
                                    {BUTTON_TEXT}
                                </span>
                            </span>
                        </span>
                    </span>
                </button>
            }
        }
    }

    fn frame(frame_index: u64) -> FrameInfo {
        FrameInfo {
            frame_index,
            delta: Duration::from_millis(16),
        }
    }
}
