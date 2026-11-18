use cssimpler::{
    core::{
        Color, LayoutBox, PathCommand as Cmd, VectorPaint, VectorPath, VectorPoint as P,
        VectorScene, VectorView, VectorViewBox,
    },
    renderer::hit_test_vector_scene,
};

fn rectangle(left: f32, top: f32, right: f32, bottom: f32) -> [Cmd; 5] {
    [
        Cmd::MoveTo(P::new(left, top)),
        Cmd::LineTo(P::new(right, top)),
        Cmd::LineTo(P::new(right, bottom)),
        Cmd::LineTo(P::new(left, bottom)),
        Cmd::Close,
    ]
}

fn scene(paths: Vec<VectorPath>) -> VectorScene {
    VectorScene::new(VectorViewBox::new(0., 0., 100., 100.), paths).unwrap()
}

#[test]
fn picks_topmost_painted_path_and_respects_winding_holes() {
    let base = VectorPath::new(rectangle(5., 5., 95., 95.).into(), VectorPaint::default()).unwrap();
    let mut commands = rectangle(10., 10., 90., 90.).to_vec();
    commands.extend(rectangle(70., 30., 30., 70.));
    let overlay = VectorPath::new(commands, VectorPaint::default()).unwrap();
    let transparent = VectorPath::from_geometry(
        base.geometry().clone(),
        VectorPaint {
            fill: Some(Color::BLACK.with_alpha(0)),
            ..VectorPaint::default()
        },
    )
    .unwrap();
    let unpainted = VectorPath::from_geometry(
        base.geometry().clone(),
        VectorPaint {
            fill: None,
            ..VectorPaint::default()
        },
    )
    .unwrap();
    let scene = scene(vec![base, overlay, transparent, unpainted]);
    let layout = LayoutBox::new(0., 0., 100., 100.);
    let pick = |x, y| hit_test_vector_scene(&scene, layout, P::new(x, y), 0.);
    assert_eq!(pick(20., 20.), Some(1));
    assert_eq!(pick(50., 50.), Some(0));
    assert_eq!(pick(2., 2.), None);
}

#[test]
fn maps_view_box_and_pan_zoom_and_clips_to_layout() {
    let path =
        VectorPath::new(rectangle(10., 15., 40., 45.).into(), VectorPaint::default()).unwrap();
    let scene = VectorScene::new(VectorViewBox::new(10., 15., 50., 80.), vec![path])
        .unwrap()
        .with_view(VectorView::new(2., P::new(-30., -35.)).unwrap());
    let layout = LayoutBox::new(20., 30., 100., 80.);
    let view = scene.view().unwrap();
    let pointer = |point| {
        view.document_to_viewport(scene.view_box(), layout, point)
            .unwrap()
    };
    assert_eq!(
        hit_test_vector_scene(&scene, layout, pointer(P::new(30., 35.)), 0.),
        Some(0)
    );
    assert_eq!(
        hit_test_vector_scene(&scene, layout, pointer(P::new(15., 20.)), 10.),
        None
    );
    assert_eq!(
        hit_test_vector_scene(&scene, layout, P::new(120., 40.), 0.),
        None
    );
}

#[test]
fn tolerance_is_in_viewport_pixels_for_curved_strokes_at_multiple_zooms() {
    let path = VectorPath::new(
        vec![
            Cmd::MoveTo(P::new(20., 40.)),
            Cmd::QuadTo {
                control: P::new(40., 60.),
                end: P::new(60., 40.),
            },
        ],
        VectorPaint {
            fill: None,
            stroke: Some(Color::BLACK),
            stroke_width: 1.,
        },
    )
    .unwrap();
    let mut scene = scene(vec![path]);
    let layout = LayoutBox::new(5., 5., 200., 100.);
    let center = P::new(40., 50.);
    for zoom in [0.5, 1., 2.] {
        let offset = P::new(40. - zoom * center.x, 50. - zoom * center.y);
        let view = VectorView::new(zoom, offset).unwrap();
        scene.set_view(Some(view));
        let point = view
            .document_to_viewport(scene.view_box(), layout, center)
            .unwrap();
        let close = P::new(point.x, point.y + 2.);
        let far = P::new(point.x, point.y + 5.);
        assert_eq!(hit_test_vector_scene(&scene, layout, point, 0.), Some(0));
        assert_eq!(hit_test_vector_scene(&scene, layout, close, 0.), None);
        assert_eq!(hit_test_vector_scene(&scene, layout, close, 3.), Some(0));
        assert_eq!(hit_test_vector_scene(&scene, layout, far, 3.), None);
    }
}

#[test]
fn fill_closes_open_subpaths_and_invalid_queries_miss() {
    let path = VectorPath::new(
        vec![
            Cmd::MoveTo(P::new(10., 10.)),
            Cmd::LineTo(P::new(90., 10.)),
            Cmd::LineTo(P::new(50., 90.)),
        ],
        VectorPaint::default(),
    )
    .unwrap();
    let scene = scene(vec![path]);
    let layout = LayoutBox::new(0., 0., 100., 100.);
    let point = P::new(40., 30.);
    assert_eq!(hit_test_vector_scene(&scene, layout, point, 0.), Some(0));
    assert_eq!(hit_test_vector_scene(&scene, layout, point, -1.), None);
    assert_eq!(hit_test_vector_scene(&scene, layout, point, f32::NAN), None);
    assert_eq!(
        hit_test_vector_scene(&scene, layout, P::new(0., f32::NAN), 0.),
        None
    );
    assert_eq!(
        hit_test_vector_scene(&scene, LayoutBox::new(0., 0., 0., 100.), point, 0.),
        None
    );
}
