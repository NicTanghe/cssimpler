use cssimpler::{
    core::{
        Color, Node, PathCommand, RenderKind, RuntimeDirtyClass, RuntimeSyncPolicy, RuntimeWorld,
        VectorPaint, VectorPath, VectorPoint, VectorScene, VectorViewBox,
    },
    style::{Stylesheet, build_render_tree},
};

#[test]
fn runtime_round_trip_and_paint_patch_keep_geometry_shared() {
    let path = VectorPath::new(
        vec![
            PathCommand::MoveTo(VectorPoint::new(1., 2.)),
            PathCommand::LineTo(VectorPoint::new(8., 9.)),
        ],
        VectorPaint::default(),
    )
    .unwrap();
    let geometry = path.geometry().clone();
    let scene = VectorScene::new(VectorViewBox::new(0., 0., 10., 10.), vec![path]).unwrap();
    let mut node: Node = Node::vector(scene).into();
    let mut world = RuntimeWorld::default();
    world.sync_root(
        0,
        &node,
        RuntimeSyncPolicy::ForceRebuild,
        RuntimeDirtyClass::Structure,
    );
    if let Node::Element(element) = &mut node {
        element.vector_scene.as_mut().unwrap().paths[0].paint.fill = Some(
            cssimpler::core::SvgPathPaintSource::Color(Color::rgb(255, 0, 0)),
        );
    }
    world.sync_root(
        0,
        &node,
        RuntimeSyncPolicy::PreferPatch,
        RuntimeDirtyClass::Paint,
    );
    let roundtrip = world.root_as_node(0).unwrap();
    let render = build_render_tree(&roundtrip, &Stylesheet::default());
    let RenderKind::Svg(scene) = render.kind else {
        panic!("expected vector leaf")
    };
    assert!(
        scene.paths[0]
            .geometry
            .commands
            .shares_storage_with(&geometry)
    );
    assert_eq!(
        scene.paths[0].paint.fill,
        Some(cssimpler::core::SvgPathPaintSource::Color(Color::rgb(
            255, 0, 0
        )))
    );
}
