//! Integration tests for issue #21: fluent native vector path builder.
use cssimpler::core::{
    Color, Node, PathCommand, PathCommand::*, VectorPaint, VectorPath, VectorPathBuilder,
    VectorPoint as P, VectorScene, VectorViewBox,
};

fn authored_commands() -> Vec<PathCommand> {
    vec![
        MoveTo(P::new(0.12345679, 2.0)),
        LineTo(P::new(30.0, 4.0)),
        QuadTo {
            control: P::new(35.5, 6.25),
            end: P::new(40.0, 8.0),
        },
        CubicTo {
            control1: P::new(-10.0, 100.0),
            control2: P::new(80.0, -40.0),
            end: P::new(90.0, 10.0),
        },
        Close,
    ]
}

fn authored_paint() -> VectorPaint {
    VectorPaint {
        fill: Some(Color::rgba(10, 20, 30, 128)),
        stroke: Some(Color::rgba(200, 100, 50, 255)),
        stroke_width: 2.5,
    }
}

fn scene(path: VectorPath) -> VectorScene {
    VectorScene::new(VectorViewBox::new(0.0, 0.0, 100.0, 100.0), vec![path]).unwrap()
}

#[test]
fn builder_covers_every_command_and_multiple_contours() {
    let path = VectorPath::builder()
        .move_to(P::new(0.12345679, 2.0))
        .line_to(P::new(30.0, 4.0))
        .quad_to(P::new(35.5, 6.25), P::new(40.0, 8.0))
        .cubic_to(
            P::new(-10.0, 100.0),
            P::new(80.0, -40.0),
            P::new(90.0, 10.0),
        )
        .close()
        // A second contour begins with another MoveTo.
        .move_to(P::new(5.0, 5.0))
        .line_to(P::new(6.0, 7.0))
        .close()
        .build()
        .unwrap();

    let expected = [
        MoveTo(P::new(0.12345679, 2.0)),
        LineTo(P::new(30.0, 4.0)),
        QuadTo {
            control: P::new(35.5, 6.25),
            end: P::new(40.0, 8.0),
        },
        CubicTo {
            control1: P::new(-10.0, 100.0),
            control2: P::new(80.0, -40.0),
            end: P::new(90.0, 10.0),
        },
        Close,
        MoveTo(P::new(5.0, 5.0)),
        LineTo(P::new(6.0, 7.0)),
        Close,
    ];
    assert_eq!(path.commands(), expected);
    assert_eq!(
        path.to_svg_path_data(),
        "M 0.12345679 2 L 30 4 Q 35.5 6.25 40 8 C -10 100 80 -40 90 10 Z \
         M 5 5 L 6 7 Z"
    );
}

#[test]
fn open_contours_keep_order_and_f32_precision() {
    let path = VectorPath::builder()
        .move_to(P::new(0.1, 0.2))
        .line_to(P::new(0.3, 0.4))
        .quad_to(P::new(0.5, 0.6), P::new(0.7, 0.8))
        .cubic_to(P::new(0.9, 1.1), P::new(1.2, 1.3), P::new(1.4, 1.5))
        .build()
        .unwrap();

    assert_eq!(
        path.to_svg_path_data(),
        "M 0.1 0.2 L 0.3 0.4 Q 0.5 0.6 0.7 0.8 C 0.9 1.1 1.2 1.3 1.4 1.5"
    );
    // f32 is preserved exactly: the values round-trip through the SVG writer.
    assert_eq!(
        path.commands()[3],
        CubicTo {
            control1: P::new(0.9_f32, 1.1_f32),
            control2: P::new(1.2_f32, 1.3_f32),
            end: P::new(1.4_f32, 1.5_f32),
        }
    );
}

#[test]
fn defaults_match_direct_construction() {
    let path = VectorPath::builder()
        .move_to(P::new(0.0, 0.0))
        .line_to(P::new(10.0, 10.0))
        .build()
        .unwrap();
    let direct = VectorPath::new(
        vec![MoveTo(P::new(0.0, 0.0)), LineTo(P::new(10.0, 10.0))],
        VectorPaint::default(),
    )
    .unwrap();

    assert_eq!(path, direct);
    assert_eq!(path.paint(), VectorPaint::default());
    assert_eq!(path.paint().fill, Some(Color::BLACK));
    assert_eq!(path.paint().stroke, None);
    assert_eq!(path.paint().stroke_width, 1.0);
    assert_eq!(VectorPath::builder().paint(), VectorPaint::default());
}

#[test]
fn paint_settings_are_applied_and_can_be_disabled() {
    let painted = scene(
        VectorPath::builder()
            .move_to(P::new(0.0, 0.0))
            .line_to(P::new(10.0, 10.0))
            .close()
            .fill(Color::rgba(10, 20, 30, 128))
            .stroke(Color::rgba(200, 100, 50, 255))
            .stroke_width(2.5)
            .build()
            .unwrap(),
    );
    let svg = painted.to_svg();
    assert!(svg.contains("fill=\"#0a141e\" fill-opacity=\"0.5019608\""));
    assert!(svg.contains("stroke=\"#c86432\" stroke-opacity=\"1\""));
    assert!(svg.contains("stroke-width=\"2.5\""));

    let disabled = VectorPath::builder()
        .move_to(P::new(0.0, 0.0))
        .line_to(P::new(10.0, 10.0))
        .no_fill()
        .no_stroke()
        .stroke_width(0.0)
        .build()
        .unwrap();
    assert_eq!(disabled.paint().fill, None);
    assert_eq!(disabled.paint().stroke, None);
    let disabled_svg = scene(disabled).to_svg();
    assert!(disabled_svg.contains("fill=\"none\" stroke=\"none\""));
    assert!(disabled_svg.contains("stroke-width=\"0\""));

    // Disabling fill does not change the default stroke, and vice versa.
    let stroke_only = VectorPath::builder().no_fill().build().unwrap();
    assert_eq!(stroke_only.paint().fill, None);
    assert_eq!(stroke_only.paint().stroke, None);
    let fill_only = VectorPath::builder().no_stroke().build().unwrap();
    assert_eq!(fill_only.paint().fill, Some(Color::BLACK));
}

#[test]
fn with_paint_can_reuse_an_existing_paint() {
    let paint = authored_paint();
    let path = VectorPath::builder()
        .with_paint(paint)
        .move_to(P::new(0.0, 0.0))
        .line_to(P::new(1.0, 1.0))
        .build()
        .unwrap();
    assert_eq!(path.paint(), paint);
}

#[test]
fn invalid_input_returns_errors_instead_of_panicking() {
    let p = P::new(1.0, 2.0);
    let ordering_cases = [
        VectorPath::builder().line_to(p),
        VectorPath::builder().quad_to(p, p),
        VectorPath::builder().cubic_to(p, p, p),
        VectorPath::builder().close(),
    ];
    for builder in ordering_cases {
        let error = builder.build().unwrap_err();
        assert_eq!(error.to_string(), "a contour must start with MoveTo");
    }

    let nonfinite_cases = [
        VectorPath::builder()
            .move_to(P::new(f32::NAN, 0.0))
            .line_to(p),
        VectorPath::builder()
            .move_to(p)
            .line_to(P::new(0.0, f32::INFINITY)),
        VectorPath::builder()
            .move_to(p)
            .quad_to(P::new(f32::NEG_INFINITY, 0.0), p),
        VectorPath::builder()
            .move_to(p)
            .cubic_to(p, P::new(0.0, f32::NAN), p),
    ];
    for builder in nonfinite_cases {
        let error = builder.build().unwrap_err();
        assert_eq!(error.to_string(), "path coordinates must be finite");
    }

    for width in [-1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let builder = VectorPath::builder().move_to(p).stroke_width(width);
        let error = builder.build().unwrap_err();
        assert_eq!(
            error.to_string(),
            "stroke width must be finite and nonnegative"
        );
    }

    // Zero-width strokes are valid, and an empty path is a valid no-op path.
    assert!(VectorPath::builder().stroke_width(0.0).build().is_ok());
    assert!(VectorPath::builder().build().is_ok());

    // The builder applies exactly the existing ordering rule: later contours are
    // introduced by MoveTo, while a command after Close continues from the
    // closed contour's initial point, as direct construction and SVG allow.
    let after_close = VectorPath::builder()
        .move_to(p)
        .close()
        .line_to(p)
        .build()
        .unwrap();
    let direct =
        VectorPath::new(vec![MoveTo(p), Close, LineTo(p)], VectorPaint::default()).unwrap();
    assert_eq!(after_close, direct);
}

#[test]
fn command_buffer_is_reserved_and_moved_without_copying() {
    let reserved = VectorPathBuilder::with_capacity(4);
    assert!(reserved.capacity() >= 4);
    assert_eq!(reserved.paint(), VectorPaint::default());
    assert_eq!(VectorPathBuilder::default(), VectorPathBuilder::new());

    let mut builder = VectorPath::builder().reserve(8);
    assert!(builder.capacity() >= 8);
    assert!(builder.is_empty());
    let capacity = builder.capacity();
    let pointer = builder.commands().as_ptr();

    builder = builder
        .move_to(P::new(0.0, 0.0))
        .line_to(P::new(1.0, 1.0))
        .quad_to(P::new(2.0, 2.0), P::new(3.0, 3.0))
        .cubic_to(P::new(4.0, 4.0), P::new(5.0, 5.0), P::new(6.0, 6.0))
        .close()
        .move_to(P::new(7.0, 7.0))
        .line_to(P::new(8.0, 8.0))
        .line_to(P::new(9.0, 9.0));

    assert_eq!(builder.len(), 8);
    assert_eq!(
        builder.capacity(),
        capacity,
        "reserved capacity was not reallocated"
    );
    assert_eq!(
        builder.commands().as_ptr(),
        pointer,
        "chained calls moved the buffer instead of growing or copying it"
    );

    let pointer = builder.commands().as_ptr();
    let path = builder.build().unwrap();
    assert_eq!(path.commands().len(), 8);
    assert_eq!(
        path.commands().as_ptr(),
        pointer,
        "build moved the same command buffer into the path"
    );
}

#[test]
fn built_path_is_identical_to_directly_authored_path_and_svg() {
    let commands = authored_commands();
    let paint = authored_paint();
    let direct = VectorPath::new(commands.clone(), paint).unwrap();
    let built = VectorPath::builder()
        .reserve(commands.len())
        .move_to(P::new(0.12345679, 2.0))
        .line_to(P::new(30.0, 4.0))
        .quad_to(P::new(35.5, 6.25), P::new(40.0, 8.0))
        .cubic_to(
            P::new(-10.0, 100.0),
            P::new(80.0, -40.0),
            P::new(90.0, 10.0),
        )
        .close()
        .with_paint(paint)
        .build()
        .unwrap();

    assert_eq!(built, direct);
    assert_eq!(built.commands(), commands.as_slice());
    assert_eq!(built.to_svg_path_data(), direct.to_svg_path_data());

    let built_scene = scene(built);
    let direct_scene = scene(direct);
    assert_eq!(built_scene.to_svg(), direct_scene.to_svg());
    assert_eq!(
        built_scene.clone().into_render_scene(),
        direct_scene.clone().into_render_scene()
    );

    let bounds = built_scene.into_render_scene().bounds.unwrap();
    assert!(bounds.min_x <= -10.0);
    assert!(bounds.max_y >= 100.0);
}

#[test]
fn node_vector_accepts_a_builder_scene() {
    let scene = scene(
        VectorPath::builder()
            .move_to(P::new(0.0, 0.0))
            .cubic_to(P::new(10.0, 0.0), P::new(10.0, 10.0), P::new(0.0, 10.0))
            .close()
            .stroke(Color::rgb(30, 60, 90))
            .build()
            .unwrap(),
    );
    let expected = scene.clone().into_render_scene();
    let element = Node::vector(scene).with_id("builder-artwork");
    assert_eq!(element.vector_scene, Some(expected));
    let node: Node = element.into();
    assert!(matches!(node, Node::Element(_)));
}
