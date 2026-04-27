use std::sync::OnceLock;

use anyhow::Result;
use cssimpler::app::{App, Invalidation};
use cssimpler::core::Node;
use cssimpler::renderer::{FrameInfo, WindowConfig};
use cssimpler::style::{Stylesheet, parse_stylesheet};
use cssimpler::ui;

const BUTTON_TEXT: &str = "uiverse";

fn main() -> Result<()> {
    let config =
        WindowConfig::new("cssimpler / uiverse hover button", 1280, 720).with_decorations(false);

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
                    Uiverse-inspired hover reveal
                </p>
                {build_button()}
                <p class="note">
                    The outlined label stays centered while the neon fill sweeps across on hover.
                </p>
                <p class="kicker">
                    Uiverse-inspired glass card
                </p>
                {build_card()}
                <p class="note">
                    A frosted panel, floating badge stack, and compact social actions sit on top of a mint neon base.
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

fn build_card() -> Node {
    ui! {
        <div class="uiverse-card-demo">
            <div class="parent">
                <div class="card">
                    <div class="logo">
                        <span class="circle circle1"></span>
                        <span class="circle circle2"></span>
                        <span class="circle circle3"></span>
                        <span class="circle circle4"></span>
                        <span class="circle circle5">
                            <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 29.667 31.69" class="svg">
                                <path d="M12.827,1.628A1.561,1.561,0,0,1,14.31,0h2.964a1.561,1.561,0,0,1,1.483,1.628v11.9a9.252,9.252,0,0,1-2.432,6.852q-2.432,2.409-6.963,2.409T2.4,20.452Q0,18.094,0,13.669V1.628A1.561,1.561,0,0,1,1.483,0h2.98A1.561,1.561,0,0,1,5.947,1.628V13.191a5.635,5.635,0,0,0,.85,3.451,3.153,3.153,0,0,0,2.632,1.094,3.032,3.032,0,0,0,2.582-1.076,5.836,5.836,0,0,0,.816-3.486Z" transform="translate(0 0)"></path>
                                <path d="M75.207,20.857a1.561,1.561,0,0,1-1.483,1.628h-2.98a1.561,1.561,0,0,1-1.483-1.628V1.628A1.561,1.561,0,0,1,70.743,0h2.98a1.561,1.561,0,0,1,1.483,1.628Z" transform="translate(-45.91 0)"></path>
                                <path d="M0,80.018A1.561,1.561,0,0,1,1.483,78.39h26.7a1.561,1.561,0,0,1,1.483,1.628v2.006a1.561,1.561,0,0,1-1.483,1.628H1.483A1.561,1.561,0,0,1,0,82.025Z" transform="translate(0 -51.963)"></path>
                            </svg>
                        </span>
                    </div>
                    <div class="glass"></div>
                    <div class="content">
                        <span class="title">UIVERSE (3D UI)</span>
                        <span class="text">
                            Create, share, and use beautiful custom elements made with CSS
                        </span>
                    </div>
                    <div class="bottom">
                        <div class="social-buttons-container">
                            <button class="social-button social-button1" type="button">
                                <svg viewBox="0 0 30 30" xmlns="http://www.w3.org/2000/svg" class="svg">
                                    <path d="M 9.9980469 3 C 6.1390469 3 3 6.1419531 3 10.001953 L 3 20.001953 C 3 23.860953 6.1419531 27 10.001953 27 L 20.001953 27 C 23.860953 27 27 23.858047 27 19.998047 L 27 9.9980469 C 27 6.1390469 23.858047 3 19.998047 3 L 9.9980469 3 z M 22 7 C 22.552 7 23 7.448 23 8 C 23 8.552 22.552 9 22 9 C 21.448 9 21 8.552 21 8 C 21 7.448 21.448 7 22 7 z M 15 9 C 18.309 9 21 11.691 21 15 C 21 18.309 18.309 21 15 21 C 11.691 21 9 18.309 9 15 C 9 11.691 11.691 9 15 9 z M 15 11 A 4 4 0 0 0 11 15 A 4 4 0 0 0 15 19 A 4 4 0 0 0 19 15 A 4 4 0 0 0 15 11 z"></path>
                                </svg>
                            </button>
                            <button class="social-button social-button2" type="button">
                                <svg viewBox="0 0 512 512" xmlns="http://www.w3.org/2000/svg" class="svg">
                                    <path d="M459.37 151.716c.325 4.548.325 9.097.325 13.645 0 138.72-105.583 298.558-298.558 298.558-59.452 0-114.68-17.219-161.137-47.106 8.447.974 16.568 1.299 25.34 1.299 49.055 0 94.213-16.568 130.274-44.832-46.132-.975-84.792-31.188-98.112-72.772 6.498.974 12.995 1.624 19.818 1.624 9.421 0 18.843-1.3 27.614-3.573-48.081-9.747-84.143-51.98-84.143-102.985v-1.299c13.969 7.797 30.214 12.67 47.431 13.319-28.264-18.843-46.781-51.005-46.781-87.391 0-19.492 5.197-37.36 14.294-52.954 51.655 63.675 129.3 105.258 216.365 109.807-1.624-7.797-2.599-15.918-2.599-24.04 0-57.828 46.782-104.934 104.934-104.934 30.213 0 57.502 12.67 76.67 33.137 23.715-4.548 46.456-13.32 66.599-25.34-7.798 24.366-24.366 44.833-46.132 57.827 21.117-2.273 41.584-8.122 60.426-16.243-14.292 20.791-32.161 39.308-52.628 54.253z"></path>
                                </svg>
                            </button>
                            <button class="social-button social-button3" type="button">
                                <svg viewBox="0 0 640 512" xmlns="http://www.w3.org/2000/svg" class="svg">
                                    <path d="M524.531,69.836a1.5,1.5,0,0,0-.764-.7A485.065,485.065,0,0,0,404.081,32.03a1.816,1.816,0,0,0-1.923.91,337.461,337.461,0,0,0-14.9,30.6,447.848,447.848,0,0,0-134.426,0,309.541,309.541,0,0,0-15.135-30.6,1.89,1.89,0,0,0-1.924-.91A483.689,483.689,0,0,0,116.085,69.137a1.712,1.712,0,0,0-.788.676C39.068,183.651,18.186,294.69,28.43,404.354a2.016,2.016,0,0,0,.765,1.375A487.666,487.666,0,0,0,176.02,479.918a1.9,1.9,0,0,0,2.063-.676A348.2,348.2,0,0,0,208.12,430.4a1.86,1.86,0,0,0-1.019-2.588,321.173,321.173,0,0,1-45.868-21.853,1.885,1.885,0,0,1-.185-3.126c3.082-2.309,6.166-4.711,9.109-7.137a1.819,1.819,0,0,1,1.9-.256c96.229,43.917,200.41,43.917,295.5,0a1.812,1.812,0,0,1,1.924.233c2.944,2.426,6.027,4.851,9.132,7.16a1.884,1.884,0,0,1-.162,3.126,301.407,301.407,0,0,1-45.89,21.83,1.875,1.875,0,0,0-1,2.611,391.055,391.055,0,0,0,30.014,48.815,1.864,1.864,0,0,0,2.063.7A486.048,486.048,0,0,0,610.7,405.729a1.882,1.882,0,0,0,.765-1.352C623.729,277.594,590.933,167.465,524.531,69.836ZM222.491,337.58c-28.972,0-52.844-26.587-52.844-59.239S193.056,219.1,222.491,219.1c29.665,0,53.306,26.82,52.843,59.239C275.334,310.993,251.924,337.58,222.491,337.58Zm195.38,0c-28.971,0-52.843-26.587-52.843-59.239S388.437,219.1,417.871,219.1c29.667,0,53.307,26.82,52.844,59.239C470.715,310.993,447.538,337.58,417.871,337.58Z"></path>
                                </svg>
                            </button>
                        </div>
                        <div class="view-more">
                            <button class="view-more-button" type="button">View more</button>
                            <svg class="svg" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" stroke-linecap="round" stroke-linejoin="round">
                                <path d="m6 9 6 6 6-6"></path>
                            </svg>
                        </div>
                    </div>
                </div>
            </div>
        </div>
    }
}

fn stylesheet() -> &'static Stylesheet {
    static STYLESHEET: OnceLock<Stylesheet> = OnceLock::new();

    STYLESHEET.get_or_init(|| {
        let source = format!(
            "{}\n{}",
            include_str!("uiverse_hover_button.css"),
            include_str!("uiverse_card.css")
        );
        parse_stylesheet(&source).expect("uiverse hover button stylesheet should stay valid")
    })
}

#[cfg(test)]
mod tests {
    use super::{BUTTON_TEXT, build_card, build_ui, stylesheet};
    use cssimpler::app::{App, Invalidation};
    use cssimpler::core::fonts::layout_text_block;
    use cssimpler::core::{ElementInteractionState, ElementPath, Node, RenderKind, RenderNode};
    use cssimpler::renderer::{
        FrameInfo, SceneProvider, ViewportSize, render_scene_update, render_to_buffer,
    };
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

    fn actual_text_node(root: &RenderNode) -> &RenderNode {
        &button(root).children[0].children[0].children[0]
    }

    fn hover_fill_text_node(root: &RenderNode) -> &RenderNode {
        &hover_mask(root).children[0].children[0].children[0]
    }

    #[test]
    fn hover_reveal_keeps_the_text_node_layouts_stable() {
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

        assert_eq!(
            actual_text_node(&idle).layout,
            actual_text_node(&hovered).layout
        );
        assert_eq!(
            hover_fill_text_node(&idle).layout,
            hover_fill_text_node(&hovered).layout
        );
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

    #[test]
    fn hovered_card_full_render_spans_a_large_visible_area() {
        let tree = ui! {
            <div>
                {build_card()}
            </div>
        };
        let hovered = build_render_tree_in_viewport_with_interaction(
            &tree,
            stylesheet(),
            480,
            480,
            &ElementInteractionState {
                hovered: Some(ElementPath::root(0).with_child(0).with_child(0)),
                active: None,
            },
        );
        let clear = cssimpler::core::Color::rgb(255, 0, 255);
        let clear_packed = ((clear.r as u32) << 16) | ((clear.g as u32) << 8) | clear.b as u32;
        let mut buffer = vec![0_u32; 480 * 480];

        render_to_buffer(&[hovered], &mut buffer, 480, 480, clear);

        let mut x0 = 480_i32;
        let mut y0 = 480_i32;
        let mut x1 = 0_i32;
        let mut y1 = 0_i32;
        for y in 0..480_i32 {
            for x in 0..480_i32 {
                if buffer[y as usize * 480 + x as usize] == clear_packed {
                    continue;
                }
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }

        assert!(x1 - x0 > 180, "hovered card should cover a wide area");
        assert!(y1 - y0 > 180, "hovered card should cover a tall area");
    }

    #[test]
    fn hovered_card_incremental_render_matches_a_full_redraw() {
        let tree = ui! {
            <div>
                {build_card()}
            </div>
        };
        let idle = build_render_tree_in_viewport_with_interaction(
            &tree,
            stylesheet(),
            480,
            480,
            &ElementInteractionState::default(),
        );
        let hovered = build_render_tree_in_viewport_with_interaction(
            &tree,
            stylesheet(),
            480,
            480,
            &ElementInteractionState {
                hovered: Some(ElementPath::root(0).with_child(0).with_child(0)),
                active: None,
            },
        );
        let clear = cssimpler::core::Color::rgb(255, 0, 255);
        let mut incremental = vec![0_u32; 480 * 480];
        let mut full = vec![0_u32; 480 * 480];

        render_to_buffer(
            std::slice::from_ref(&idle),
            &mut incremental,
            480,
            480,
            clear,
        );
        render_scene_update(
            std::slice::from_ref(&idle),
            std::slice::from_ref(&hovered),
            &mut incremental,
            480,
            480,
            clear,
        );
        render_to_buffer(std::slice::from_ref(&hovered), &mut full, 480, 480, clear);

        assert_eq!(incremental, full);
    }

    #[test]
    fn hovered_card_stays_near_its_idle_center() {
        let tree = ui! {
            <div>
                {build_card()}
            </div>
        };
        let idle = build_render_tree_in_viewport_with_interaction(
            &tree,
            stylesheet(),
            480,
            480,
            &ElementInteractionState::default(),
        );
        let hovered = build_render_tree_in_viewport_with_interaction(
            &tree,
            stylesheet(),
            480,
            480,
            &ElementInteractionState {
                hovered: Some(ElementPath::root(0).with_child(0).with_child(0)),
                active: None,
            },
        );
        let clear = cssimpler::core::Color::rgb(255, 0, 255);
        let clear_packed = ((clear.r as u32) << 16) | ((clear.g as u32) << 8) | clear.b as u32;
        let mut idle_buffer = vec![0_u32; 480 * 480];
        let mut hovered_buffer = vec![0_u32; 480 * 480];

        render_to_buffer(
            std::slice::from_ref(&idle),
            &mut idle_buffer,
            480,
            480,
            clear,
        );
        render_to_buffer(
            std::slice::from_ref(&hovered),
            &mut hovered_buffer,
            480,
            480,
            clear,
        );

        let idle_bounds = visible_bounds(&idle_buffer, 480, 480, clear_packed)
            .expect("idle card should render visible pixels");
        let hovered_bounds = visible_bounds(&hovered_buffer, 480, 480, clear_packed)
            .expect("hovered card should render visible pixels");
        let idle_center_x = (idle_bounds.0 + idle_bounds.2) as f32 * 0.5;
        let idle_center_y = (idle_bounds.1 + idle_bounds.3) as f32 * 0.5;
        let hovered_center_x = (hovered_bounds.0 + hovered_bounds.2) as f32 * 0.5;
        let hovered_center_y = (hovered_bounds.1 + hovered_bounds.3) as f32 * 0.5;

        assert!((hovered_center_x - idle_center_x).abs() < 40.0);
        assert!((hovered_center_y - idle_center_y).abs() < 40.0);
    }

    #[test]
    fn hover_transition_midpoint_keeps_the_real_card_near_its_idle_center() {
        let mut app = App::new(
            (),
            stylesheet(),
            |_state, _frame| Invalidation::Clean,
            |_state| {
                ui! {
                    <div>
                        {build_card()}
                    </div>
                }
            },
        );
        app.set_viewport(ViewportSize {
            width: 480,
            height: 480,
        });

        let idle = app.frame(frame(0));
        assert!(SceneProvider::set_element_interaction(
            &mut app,
            ElementInteractionState {
                hovered: Some(ElementPath::root(0).with_child(0).with_child(0)),
                active: None,
            },
        ));
        let mid = app.frame(FrameInfo {
            frame_index: 1,
            delta: Duration::from_millis(250),
        });
        let final_scene = app.frame(FrameInfo {
            frame_index: 2,
            delta: Duration::from_millis(250),
        });

        let clear = cssimpler::core::Color::rgb(255, 0, 255);
        let clear_packed = ((clear.r as u32) << 16) | ((clear.g as u32) << 8) | clear.b as u32;
        let mut idle_buffer = vec![0_u32; 480 * 480];
        let mut mid_buffer = vec![0_u32; 480 * 480];
        let mut final_buffer = vec![0_u32; 480 * 480];

        render_to_buffer(&idle, &mut idle_buffer, 480, 480, clear);
        render_to_buffer(&mid, &mut mid_buffer, 480, 480, clear);
        render_to_buffer(&final_scene, &mut final_buffer, 480, 480, clear);

        let idle_bounds =
            visible_bounds(&idle_buffer, 480, 480, clear_packed).expect("idle card should render");
        let mid_bounds = visible_bounds(&mid_buffer, 480, 480, clear_packed)
            .expect("mid-transition card should render");
        let final_bounds = visible_bounds(&final_buffer, 480, 480, clear_packed)
            .expect("final card should render");

        let idle_center_x = (idle_bounds.0 + idle_bounds.2) as f32 * 0.5;
        let idle_center_y = (idle_bounds.1 + idle_bounds.3) as f32 * 0.5;
        let mid_center_x = (mid_bounds.0 + mid_bounds.2) as f32 * 0.5;
        let mid_center_y = (mid_bounds.1 + mid_bounds.3) as f32 * 0.5;
        let final_center_x = (final_bounds.0 + final_bounds.2) as f32 * 0.5;
        let final_center_y = (final_bounds.1 + final_bounds.3) as f32 * 0.5;
        assert!(
            (mid_center_x - idle_center_x).abs() < 14.0,
            "mid x drift too large: idle={idle_center_x}, mid={mid_center_x}, final={final_center_x}"
        );
        assert!(
            (mid_center_y - idle_center_y).abs() < 18.0,
            "mid y drift too large: idle={idle_center_y}, mid={mid_center_y}, final={final_center_y}"
        );
        assert!(
            (final_center_x - idle_center_x).abs() < 24.0,
            "final x drift too large: idle={idle_center_x}, mid={mid_center_x}, final={final_center_x}"
        );
        assert!(
            (final_center_y - idle_center_y).abs() < 32.0,
            "final y drift too large: idle={idle_center_y}, mid={mid_center_y}, final={final_center_y}"
        );
    }

    #[test]
    fn hover_transition_midpoint_incremental_render_matches_full_redraw() {
        let mut app = App::new(
            (),
            stylesheet(),
            |_state, _frame| Invalidation::Clean,
            |_state| {
                ui! {
                    <div>
                        {build_card()}
                    </div>
                }
            },
        );
        app.set_viewport(ViewportSize {
            width: 480,
            height: 480,
        });

        let idle = app.frame(frame(0));
        assert!(SceneProvider::set_element_interaction(
            &mut app,
            ElementInteractionState {
                hovered: Some(ElementPath::root(0).with_child(0).with_child(0)),
                active: None,
            },
        ));
        let mid = app.frame(FrameInfo {
            frame_index: 1,
            delta: Duration::from_millis(250),
        });

        let clear = cssimpler::core::Color::rgb(255, 0, 255);
        let mut incremental = vec![0_u32; 480 * 480];
        let mut full = vec![0_u32; 480 * 480];

        render_to_buffer(&idle, &mut incremental, 480, 480, clear);
        render_scene_update(&idle, &mid, &mut incremental, 480, 480, clear);
        render_to_buffer(&mid, &mut full, 480, 480, clear);

        assert_eq!(incremental, full);
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

    fn visible_bounds(
        buffer: &[u32],
        width: usize,
        height: usize,
        clear_packed: u32,
    ) -> Option<(i32, i32, i32, i32)> {
        let mut x0 = width as i32;
        let mut y0 = height as i32;
        let mut x1 = 0_i32;
        let mut y1 = 0_i32;

        for y in 0..height as i32 {
            for x in 0..width as i32 {
                if buffer[y as usize * width + x as usize] == clear_packed {
                    continue;
                }
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }

        (x1 > x0 && y1 > y0).then_some((x0, y0, x1, y1))
    }
}
