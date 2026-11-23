use cssimpler::app::{App, Invalidation};
use cssimpler::core::{Color, ElementNode, ElementPath};
use cssimpler::renderer::{FrameInfo, SceneProvider, ScrollDirections, ScrollState};
use cssimpler::style::parse_stylesheet;

#[test]
fn css_scrollable_queries_restyle_after_scroll_boundary_changes() {
    let sheet = parse_stylesheet(
        "panel { container: feed / scroll-state; }
         #status { width: 40px; height: 20px; background-color: red; }
         @container feed scroll-state(scrollable: bottom) {
             #status { background-color: blue; }
         }",
    )
    .unwrap();
    let mut app = App::new(
        (),
        &sheet,
        |_, _| Invalidation::Clean,
        |_| {
            ElementNode::new("panel")
                .with_child(ElementNode::new("label").with_id("status").into())
                .into()
        },
    );
    let frame = |frame_index| FrameInfo {
        frame_index,
        delta: std::time::Duration::ZERO,
    };
    assert!(app.tracks_scroll_state());
    let mut snapshot = ScrollState {
        element: ElementPath::root(0),
        scrollable: ScrollDirections {
            down: true,
            ..ScrollDirections::default()
        },
        query_container: true,
        names: vec!["feed".into()],
    };
    assert_eq!(
        app.frame(frame(0))[0].children[0].style.background,
        Some(Color::rgb(255, 0, 0))
    );
    assert!(app.set_scroll_state(&[snapshot.clone()]));
    assert_eq!(
        app.frame(frame(1))[0].children[0].style.background,
        Some(Color::rgb(0, 0, 255))
    );
    snapshot.scrollable = ScrollDirections {
        up: true,
        ..ScrollDirections::default()
    };
    assert!(app.set_scroll_state(&[snapshot]));
    assert_eq!(
        app.frame(frame(2))[0].children[0].style.background,
        Some(Color::rgb(255, 0, 0))
    );
}
