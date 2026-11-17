//! Issue #17: shared immutable native vector geometry.
//!
//! Covers independent instances (paint/viewBox), runtime paint patches and
//! geometry replacement, render-tree sharing and bounds, SVG export curve
//! preservation, and the cost/identity properties of the retained fingerprint.

use cssimpler::{
    core::{
        Color, Node, PathCommand, RenderKind, RuntimeDirtyClass, RuntimeSyncPolicy, RuntimeWorld,
        SvgPathPaintSource, VectorGeometry, VectorPaint, VectorPath, VectorPoint, VectorScene,
        VectorViewBox,
    },
    renderer::render_to_rgba8,
    style::{Stylesheet, build_render_tree, build_render_tree_in_viewport},
};

fn rectangle(x: f32, y: f32, size: f32) -> VectorGeometry {
    VectorGeometry::new(vec![
        PathCommand::MoveTo(VectorPoint::new(x, y)),
        PathCommand::LineTo(VectorPoint::new(x + size, y)),
        PathCommand::LineTo(VectorPoint::new(x + size, y + size)),
        PathCommand::LineTo(VectorPoint::new(x, y + size)),
        PathCommand::Close,
    ])
    .unwrap()
}

fn curve() -> VectorGeometry {
    VectorGeometry::new(vec![
        PathCommand::MoveTo(VectorPoint::new(10., 10.)),
        PathCommand::CubicTo {
            control1: VectorPoint::new(20., 80.),
            control2: VectorPoint::new(90., -20.),
            end: VectorPoint::new(100., 40.),
        },
        PathCommand::QuadTo {
            control: VectorPoint::new(110., 70.),
            end: VectorPoint::new(60., 90.),
        },
        PathCommand::Close,
    ])
    .unwrap()
}

fn red() -> VectorPaint {
    VectorPaint {
        fill: Some(Color::rgb(255, 0, 0)),
        stroke: None,
        stroke_width: 1.,
    }
}

fn blue() -> VectorPaint {
    VectorPaint {
        fill: Some(Color::rgb(0, 0, 255)),
        stroke: None,
        stroke_width: 1.,
    }
}

#[test]
fn independent_instances_share_geometry_but_not_paint_or_view_box() {
    let geometry = curve();
    let filled = VectorPath::from_geometry(geometry.clone(), red()).unwrap();
    let stroked = VectorPath::from_geometry(
        geometry.clone(),
        VectorPaint {
            fill: None,
            stroke: Some(Color::BLACK),
            stroke_width: 6.,
        },
    )
    .unwrap();
    assert!(filled.geometry().shares_storage_with(stroked.geometry()));
    assert_ne!(filled.paint(), stroked.paint());

    // Two scenes reusing one drawing with different view boxes and paints.
    let scene_a =
        VectorScene::new(VectorViewBox::new(0., 0., 100., 100.), vec![filled.clone()]).unwrap();
    let scene_b =
        VectorScene::new(VectorViewBox::new(8., 8., 50., 50.), vec![stroked.clone()]).unwrap();
    let mut render_a = scene_a.into_render_scene();
    let render_b = scene_b.into_render_scene();
    assert!(
        render_a.paths[0]
            .geometry
            .commands
            .shares_storage_with(&geometry)
    );
    assert!(
        render_b.paths[0]
            .geometry
            .commands
            .shares_storage_with(&geometry)
    );
    assert_ne!(render_a.view_box, render_b.view_box);
    assert_ne!(render_a.paths[0].paint, render_b.paths[0].paint);
    // Geometry bounds are shared and identical; paint-dependent path bounds differ.
    assert_eq!(
        render_a.paths[0].geometry.bounds,
        render_b.paths[0].geometry.bounds
    );
    let geometry_bounds = render_a.paths[0].geometry.bounds.unwrap();
    assert_eq!(
        (
            geometry_bounds.min_x,
            geometry_bounds.min_y,
            geometry_bounds.max_x,
            geometry_bounds.max_y
        ),
        (10., -20., 110., 90.)
    );
    let filled_bounds = render_a.paths[0].bounds().unwrap();
    let stroked_bounds = render_b.paths[0].bounds().unwrap();
    // A 6px stroke widens only the stroked instance by 3px per side.
    assert_eq!(filled_bounds.min_x, 10.);
    assert_eq!(stroked_bounds.min_x, 7.);
    assert_eq!(filled_bounds.min_y, -20.);
    assert_eq!(stroked_bounds.min_y, -23.);

    // Mutating one instance's paint must not touch the other (paint is owned
    // per path; only command storage is shared).
    render_a.paths[0].paint.fill = Some(SvgPathPaintSource::Color(Color::rgb(0, 255, 0)));
    assert_ne!(
        render_a.paths[0].paint, render_b.paths[0].paint,
        "paint updates must stay per instance"
    );
    assert!(
        render_a.paths[0]
            .geometry
            .commands
            .shares_storage_with(&geometry)
    );
}

#[test]
fn runtime_paint_patch_and_geometry_replacement_track_identity() {
    let original = curve();
    let first = VectorPath::from_geometry(original.clone(), red()).unwrap();
    let mut node: Node = Node::vector(
        VectorScene::new(VectorViewBox::new(0., 0., 120., 120.), vec![first]).unwrap(),
    )
    .into();
    let mut world = RuntimeWorld::default();
    world.sync_root(
        0,
        &node,
        RuntimeSyncPolicy::ForceRebuild,
        RuntimeDirtyClass::Structure,
    );
    let rendered = build_render_tree(&world.root_as_node(0).unwrap(), &Stylesheet::default());
    let RenderKind::Svg(scene) = &rendered.kind else {
        panic!("expected native vector leaf")
    };
    assert!(
        scene.paths[0]
            .geometry
            .commands
            .shares_storage_with(&original)
    );

    // Paint-only patch: geometry storage is retained, paint changes.
    let Node::Element(element) = &mut node else {
        panic!("expected element")
    };
    element.vector_scene.as_mut().unwrap().paths[0].paint.fill =
        Some(SvgPathPaintSource::Color(Color::rgb(0, 0, 255)));
    let result = world.sync_root(
        0,
        &node,
        RuntimeSyncPolicy::PreferPatch,
        RuntimeDirtyClass::Paint,
    );
    assert_eq!(result.action, cssimpler::core::RuntimeSyncAction::Patched);
    let roundtrip = world.root_as_node(0).unwrap();
    let rendered = build_render_tree(&roundtrip, &Stylesheet::default());
    let RenderKind::Svg(scene) = &rendered.kind else {
        panic!("expected native vector leaf")
    };
    assert!(
        scene.paths[0]
            .geometry
            .commands
            .shares_storage_with(&original)
    );
    assert_eq!(
        scene.paths[0].paint.fill,
        Some(SvgPathPaintSource::Color(Color::rgb(0, 0, 255)))
    );

    // Gear geometry replacement: new immutable data, new identity, new bounds.
    let replacement = rectangle(40., 40., 20.);
    assert!(!replacement.shares_storage_with(&original));
    assert_ne!(replacement.identity(), original.identity());
    let Node::Element(element) = &mut node else {
        panic!("expected element")
    };
    let scene = element.vector_scene.as_mut().unwrap();
    scene.paths[0].geometry.commands = replacement.clone();
    scene.paths[0].geometry.bounds = replacement.bounds();
    let result = world.sync_root(
        0,
        &node,
        RuntimeSyncPolicy::PreferPatch,
        RuntimeDirtyClass::Structure,
    );
    assert_eq!(result.action, cssimpler::core::RuntimeSyncAction::Patched);
    let roundtrip = world.root_as_node(0).unwrap();
    let rendered = build_render_tree(&roundtrip, &Stylesheet::default());
    let RenderKind::Svg(scene) = &rendered.kind else {
        panic!("expected native vector leaf")
    };
    assert!(
        scene.paths[0]
            .geometry
            .commands
            .shares_storage_with(&replacement)
    );
    assert!(
        !scene.paths[0]
            .geometry
            .commands
            .shares_storage_with(&original)
    );
    let bounds = scene.paths[0].bounds().unwrap();
    assert_eq!((bounds.min_x, bounds.min_y), (40., 40.));
    assert_eq!((bounds.max_x, bounds.max_y), (60., 60.));
}

#[test]
fn render_tree_shares_geometry_and_keeps_paint_dependent_bounds() {
    let geometry = rectangle(8., 8., 48.);
    let filled = VectorPath::from_geometry(geometry.clone(), red()).unwrap();
    let stroked = VectorPath::from_geometry(
        geometry.clone(),
        VectorPaint {
            fill: None,
            stroke: Some(Color::BLACK),
            stroke_width: 10.,
        },
    )
    .unwrap();
    let node: Node = Node::vector(
        VectorScene::new(VectorViewBox::new(0., 0., 64., 64.), vec![filled, stroked]).unwrap(),
    )
    .into();
    let render = build_render_tree_in_viewport(&node, &Stylesheet::default(), 64, 64);
    let RenderKind::Svg(scene) = &render.kind else {
        panic!("expected native vector leaf")
    };
    assert!(
        scene.paths[0]
            .geometry
            .commands
            .shares_storage_with(&geometry)
    );
    assert!(
        scene.paths[1]
            .geometry
            .commands
            .shares_storage_with(&geometry)
    );
    assert_eq!(scene.paths[0].bounds().unwrap().min_x, 8.);
    // A 10px stroke expands the painted bounds by 5px on every side.
    assert_eq!(scene.paths[1].bounds().unwrap().min_x, 3.);
    assert_eq!(scene.paths[1].bounds().unwrap().max_x, 61.);
}

#[test]
fn style_and_layout_rebuild_reuses_geometry() {
    let geometry = rectangle(0., 0., 16.);
    let path = VectorPath::from_geometry(geometry.clone(), red()).unwrap();
    let scene = VectorScene::new(VectorViewBox::new(0., 0., 16., 16.), vec![path]).unwrap();
    // Outer style, layout, transform and background changes happen during
    // style/layout resolution; none of them may copy the command storage.
    let stylesheet = cssimpler::style::parse_stylesheet(
        "#art { width: 32px; height: 32px; transform: translate(4px, 2px); background: #102030; }",
    )
    .unwrap();
    let node: Node = Node::vector(scene.clone()).with_id("art").into();
    let render = build_render_tree_in_viewport(&node, &stylesheet, 64, 64);
    let RenderKind::Svg(rendered_scene) = &render.kind else {
        panic!("expected native vector leaf")
    };
    assert!(
        rendered_scene.paths[0]
            .geometry
            .commands
            .shares_storage_with(&geometry)
    );
    assert_eq!(
        (render.layout.width, render.layout.height),
        (32., 32.),
        "CSS layout must still size the vector leaf"
    );
    assert!(!render.style.transform.is_identity());
    assert_eq!(render.style.background, Some(Color::rgb(0x10, 0x20, 0x30)));
}

#[test]
fn shared_geometry_renders_independent_paint_without_stale_pixels() {
    let geometry = rectangle(8., 8., 48.);
    let render = |paint: VectorPaint| {
        let node: Node = Node::vector(
            VectorScene::new(
                VectorViewBox::new(0., 0., 64., 64.),
                vec![VectorPath::from_geometry(geometry.clone(), paint).unwrap()],
            )
            .unwrap(),
        )
        .into();
        let tree = build_render_tree_in_viewport(&node, &Stylesheet::default(), 64, 64);
        let mut rgba = vec![0u8; 64 * 64 * 4];
        render_to_rgba8(&[tree], &mut rgba, 64, 64, Color::WHITE);
        rgba
    };
    let red_pixels = render(red());
    let blue_pixels = render(blue());
    let center = (32 * 64 + 32) * 4;
    assert_eq!(
        &red_pixels[center..center + 3],
        &[255, 0, 0],
        "shared geometry with a red paint must render red"
    );
    assert_eq!(
        &blue_pixels[center..center + 3],
        &[0, 0, 255],
        "the same shared geometry with a blue paint must render blue"
    );
    // Outside the rectangle remains the cleared background.
    let outside = (2 * 64 + 2) * 4;
    assert_eq!(&red_pixels[outside..outside + 3], &[255, 255, 255]);
}

#[test]
fn svg_export_preserves_original_curves_and_precision() {
    let geometry = VectorGeometry::new(vec![
        PathCommand::MoveTo(VectorPoint::new(1.5, 2.25)),
        PathCommand::CubicTo {
            control1: VectorPoint::new(3., 4.),
            control2: VectorPoint::new(5.5, 6.75),
            end: VectorPoint::new(8., 9.),
        },
        PathCommand::QuadTo {
            control: VectorPoint::new(10., 11.),
            end: VectorPoint::new(12.5, 13.5),
        },
        PathCommand::Close,
    ])
    .unwrap();
    let expected = "M 1.5 2.25 C 3 4 5.5 6.75 8 9 Q 10 11 12.5 13.5 Z";
    let red_path = VectorPath::from_geometry(geometry.clone(), red()).unwrap();
    let blue_path = VectorPath::from_geometry(geometry.clone(), blue()).unwrap();
    assert_eq!(red_path.to_svg_path_data(), expected);
    let scene = VectorScene::new(
        VectorViewBox::new(0., 0., 20., 20.),
        vec![red_path, blue_path],
    )
    .unwrap();
    let svg = scene.to_svg();
    assert_eq!(svg.matches(&format!("d=\"{expected}\"")).count(), 2);
    assert!(svg.contains("fill=\"#ff0000\""));
    assert!(svg.contains("fill=\"#0000ff\""));

    // A content-identical replacement is a new identity but exports identically.
    let replacement = VectorGeometry::new(geometry.commands().to_vec()).unwrap();
    assert_ne!(replacement.identity(), geometry.identity());
    let replacement_path = VectorPath::from_geometry(replacement, red()).unwrap();
    assert_eq!(replacement_path.to_svg_path_data(), expected);
}

#[test]
fn identity_debug_is_constant_cost_and_clone_stable() {
    let commands: Vec<PathCommand> = (0..4096)
        .map(|index| {
            if index == 0 {
                PathCommand::MoveTo(VectorPoint::new(0., 0.))
            } else {
                PathCommand::LineTo(VectorPoint::new(index as f32, index as f32))
            }
        })
        .collect();
    let geometry = VectorGeometry::new(commands).unwrap();
    let debug = format!("{geometry:?}");
    // The fingerprint is retained; Debug must not re-serialize 4096 commands.
    assert!(debug.len() < 80, "unexpected debug length: {debug}");
    assert!(!debug.contains("MoveTo"));
    assert!(!debug.contains("LineTo"));
    assert!(debug.contains(&geometry.identity().to_string()));

    let clone = geometry.clone();
    assert_eq!(clone.identity(), geometry.identity());
    assert_eq!(format!("{clone:?}"), debug);
    assert!(clone.shares_storage_with(&geometry));

    let replacement = VectorGeometry::new(geometry.commands().to_vec()).unwrap();
    assert_ne!(replacement.identity(), geometry.identity());
    assert!(!replacement.shares_storage_with(&geometry));
    assert_ne!(format!("{replacement:?}"), debug);

    // Path Debug (used indirectly by cache keys) also stays bounded.
    let identity = geometry.identity();
    let path = VectorPath::from_geometry(geometry, red()).unwrap();
    let path_debug = format!("{path:?}");
    assert!(
        path_debug.len() < 300,
        "unexpected path debug length: {path_debug}"
    );
    assert!(path_debug.contains(&identity.to_string()));
    assert!(!path_debug.contains("LineTo"));
}

#[test]
fn identities_are_unique_across_many_constructions() {
    let mut identities = std::collections::BTreeSet::new();
    for index in 0..1000 {
        let geometry = VectorGeometry::new(vec![PathCommand::MoveTo(VectorPoint::new(
            index as f32,
            0.,
        ))])
        .unwrap();
        assert!(identities.insert(geometry.identity()));
    }
}

#[test]
fn geometry_handle_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<VectorGeometry>();
    assert_send_sync::<VectorPath>();
}
