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
        target_relation: None,
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

#[test]
fn marker_pseudo_classes_follow_the_selected_target() {
    use cssimpler::core::ScrollTargetRelation::{After, Before, Current};

    let sheet = parse_stylesheet(
        "nav { scroll-target-group: auto; }
         a { width: 30px; height: 10px; }
         a:target-before { background-color: red; }
         a:target-current { background-color: blue; }
         a:target-after { background-color: green; }",
    )
    .unwrap();
    let mut app = App::new(
        (),
        &sheet,
        |_, _| Invalidation::Clean,
        |_| {
            ElementNode::new("nav")
                .with_children(
                    ["#a", "#b", "#c"]
                        .map(|href| ElementNode::new("a").with_attribute("href", href).into()),
                )
                .into()
        },
    );
    let frame = |frame_index| FrameInfo {
        frame_index,
        delta: std::time::Duration::ZERO,
    };
    assert!(app.tracks_scroll_state());
    let first = app.frame(frame(0));
    assert!(first[0].style.scroll_target_group);
    assert_eq!(
        first[0].children[1].style.scroll_target_href.as_deref(),
        Some("b")
    );
    let snapshot = [Before, Current, After]
        .into_iter()
        .enumerate()
        .map(|(index, target_relation)| ScrollState {
            element: ElementPath::root(0).with_child(index),
            scrollable: ScrollDirections::default(),
            query_container: false,
            names: Vec::new(),
            target_relation: Some(target_relation),
        })
        .collect::<Vec<_>>();
    assert!(app.set_scroll_state(&snapshot));
    let updated = app.frame(frame(1));
    let backgrounds = updated[0]
        .children
        .iter()
        .map(|node| node.style.background)
        .collect::<Vec<_>>();
    assert_eq!(
        backgrounds,
        [
            Some(Color::rgb(255, 0, 0)),
            Some(Color::rgb(0, 0, 255)),
            Some(Color::rgb(0, 128, 0))
        ]
    );
}
