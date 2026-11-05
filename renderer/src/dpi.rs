use cssimpler_core::{LayoutBox, RenderNode, TransformOperation};

/// Apply density after layout and input handling, before rasterization and
/// damage tracking. A wrapper preserves CSS transforms and stacking order.
pub(crate) fn device_scene(scene: Vec<RenderNode>, scale: f32) -> Vec<RenderNode> {
    assert!(
        scale.is_finite() && scale > 0.0,
        "display scale must be positive and finite"
    );
    if scale == 1.0 {
        return scene;
    }
    let mut root = RenderNode::container(LayoutBox::default());
    root.style
        .transform
        .operations
        .push(TransformOperation::Scale { x: scale, y: scale });
    root.children = scene;
    vec![root]
}

pub(crate) fn scaled_radius(
    radius: cssimpler_core::CornerRadius,
    scale: f32,
) -> cssimpler_core::CornerRadius {
    // Negative values encode percentages and must retain their relative size.
    let scaled = |r: f32| if r < 0.0 { r } else { r * scale };
    cssimpler_core::CornerRadius {
        top_left: scaled(radius.top_left),
        top_right: scaled(radius.top_right),
        bottom_left: scaled(radius.bottom_left),
        bottom_right: scaled(radius.bottom_right),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{render_to_buffer, render_to_buffer_at_scale};
    use cssimpler_core::{
        Color, CornerRadius, VisualStyle,
        fonts::{FontFamily, TextStyle, register_font_file},
    };

    #[test]
    fn scaled_text_uses_device_resolution_outlines() {
        let family = register_font_file(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples/assets/powerline-demo.ttf"),
        )
        .unwrap()
        .remove(0);
        for scale in [1.0, 1.2, 1.25, 1.5, 2.0, 3.0] {
            let style = VisualStyle {
                foreground: Color::WHITE,
                text: TextStyle {
                    families: vec![FontFamily::Named(family.clone())],
                    size_px: 15.0,
                    ..TextStyle::default()
                },
                ..VisualStyle::default()
            };
            let logical = LayoutBox::new(7.25, 3.5, 150.0, 40.0);
            let scene =
                vec![RenderNode::text(logical, "Choose your account").with_style(style.clone())];
            let mut device_style = style;
            device_style.text.size_px *= scale;
            let reference = vec![
                RenderNode::text(
                    LayoutBox::new(
                        logical.x * scale,
                        logical.y * scale,
                        logical.width * scale,
                        logical.height * scale,
                    ),
                    "Choose your account",
                )
                .with_style(device_style),
            ];
            let mut actual = vec![0; 512 * 160];
            let mut expected = actual.clone();
            render_to_buffer_at_scale(&scene, &mut actual, 512, 160, Color::BLACK, scale);
            render_to_buffer(&reference, &mut expected, 512, 160, Color::BLACK);
            assert!(actual.iter().any(|&p| p != crate::pack_rgb(Color::BLACK)));
            let mismatch = actual.iter().zip(&expected).position(|(a, b)| a != b);
            assert!(
                mismatch.is_none(),
                "text differs from native-size outlines at {scale}x, first pixel {mismatch:?}"
            );
        }
    }

    #[test]
    fn rounded_corners_are_sampled_in_physical_pixels() {
        for scale in [1.2, 1.25, 1.5, 2.0, 3.0] {
            let layout = LayoutBox::new(10.0, 10.0, 50.0, 30.0);
            let style = VisualStyle {
                background: Some(Color::rgb(245, 42, 53)),
                corner_radius: CornerRadius::all(5.0),
                ..VisualStyle::default()
            };
            let scene = vec![RenderNode::container(layout).with_style(style.clone())];
            let mut pixels = vec![0; 240 * 160];
            render_to_buffer_at_scale(&scene, &mut pixels, 240, 160, Color::BLACK, scale);
            let left = (layout.x * scale) as usize;
            let right = ((layout.x + layout.width) * scale) as usize;
            let top = (layout.y * scale) as usize;
            let bottom = ((layout.y + layout.height) * scale) as usize;
            // Integer physical bounds and symmetric radii must produce matching
            // opposite corners, including partially covered edge pixels.
            if [layout.x, layout.y, layout.width, layout.height]
                .iter()
                .all(|v| (v * scale).fract() == 0.0)
            {
                for y in top..bottom {
                    for x in left..right {
                        assert_eq!(
                            pixels[y * 240 + x],
                            pixels[y * 240 + right - 1 - (x - left)],
                            "horizontal symmetry at {scale}x"
                        );
                        assert_eq!(
                            pixels[y * 240 + x],
                            pixels[(bottom - 1 - (y - top)) * 240 + x],
                            "vertical symmetry at {scale}x"
                        );
                    }
                }
            }
            let fill = crate::pack_rgb(style.background.unwrap());
            assert!(
                pixels
                    .iter()
                    .any(|&p| p != fill && p != crate::pack_rgb(Color::BLACK)),
                "corners need antialiasing at {scale}x"
            );
        }
    }

    use std::path::PathBuf;
}
