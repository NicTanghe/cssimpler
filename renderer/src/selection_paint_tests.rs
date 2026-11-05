use super::*;
use cssimpler_core::TextSelectionStyle;
use cssimpler_core::fonts::{FontFamily, TextTransform, WhiteSpace};

// With unchanged selection colours, every pixel must be identical. This catches
// displaced glyphs, changed tracking, rewrapping, and duplicated text at edges.
#[test]
fn selection_preserves_original_text_pixels() {
    for (text, size, tracking, width) in [
        ("CASHIN", 15.0, 3.0, 220.0),
        ("***", 30.0, 8.0, 220.0),
        ("AVATAR Toffee", 24.0, 1.5, 220.0),
        ("one two three four", 20.0, 0.0, 100.0),
        ("aßz", 24.0, 2.0, 220.0),
        ("नमस्ते संसार", 24.0, 1.0, 220.0),
    ] {
        for align in [TextAlign::Left, TextAlign::Center, TextAlign::Right] {
            for fallback in [false, true] {
                let mut style = VisualStyle::default();
                style.foreground = Color::BLACK;
                style.text.size_px = size;
                style.text.letter_spacing_px = tracking;
                style.text.text_align = align;
                if text == "aßz" {
                    style.text.text_transform = TextTransform::Uppercase;
                }
                if fallback {
                    style.text.families =
                        vec![FontFamily::Named("missing-selection-paint-test".into())];
                }
                let node = RenderNode::text(LayoutBox::new(20.25, 20.5, width, 120.0), text)
                    .with_style(style);
                let mut baseline = vec![0; 280 * 160];
                render_to_buffer(&[node.clone()], &mut baseline, 280, 160, Color::WHITE);
                let boundaries: Vec<_> = text
                    .char_indices()
                    .map(|(i, _)| i)
                    .chain(std::iter::once(text.len()))
                    .collect();
                for pair in boundaries
                    .windows(2)
                    .chain(std::iter::once([0, text.len()].as_slice()))
                {
                    let selected = node.clone().with_text_edit(TextEditDecoration {
                        caret: None,
                        selection: Some(TextSelectionRange {
                            start: pair[0],
                            end: pair[1],
                            style: TextSelectionStyle {
                                foreground: Color::BLACK,
                                background: Color::WHITE,
                                ..Default::default()
                            },
                        }),
                    });
                    let mut pixels = vec![0; baseline.len()];
                    render_to_buffer(&[selected], &mut pixels, 280, 160, Color::WHITE);
                    let different = baseline.iter().zip(&pixels).filter(|(a, b)| a != b).count();
                    assert_eq!(
                        different, 0,
                        "selection moved or repainted {text:?} {pair:?}, align={align:?}, fallback={fallback}"
                    );
                }
            }
        }
    }
}

#[test]
fn transformed_selection_preserves_original_glyph_positions() {
    let mut style = VisualStyle::default();
    style.foreground = Color::BLACK;
    style.text.size_px = 30.0;
    style.text.letter_spacing_px = 8.0;
    style.text.white_space = WhiteSpace::NoWrap;
    style.text.text_align = TextAlign::Center;
    let layout = LayoutBox::new(20.25, 20.5, 220.0, 50.0);
    let selection = TextSelectionRange {
        start: 1,
        end: 2,
        style: TextSelectionStyle {
            foreground: Color::BLACK,
            background: Color::WHITE,
            ..Default::default()
        },
    };
    for matrix in [
        AffineTransform::translate(0.25, 0.5),
        AffineTransform {
            a: 1.25,
            d: 1.25,
            ..AffineTransform::IDENTITY
        },
        AffineTransform {
            a: 0.98,
            b: 0.15,
            c: -0.15,
            d: 0.98,
            ..AffineTransform::IDENTITY
        },
    ] {
        let clip = ClipState::new(ClipRect::full(360.0, 200.0));
        let mut baseline = vec![pack_rgb(Color::WHITE); 360 * 200];
        fonts::draw_text_transformed(
            &mut baseline,
            360,
            200,
            layout,
            "***",
            None,
            &style,
            matrix,
            &clip,
        );
        let mut selected = vec![pack_rgb(Color::WHITE); baseline.len()];
        draw_text_selection_transformed(
            &mut selected,
            360,
            200,
            layout,
            "***",
            None,
            &style,
            &selection,
            matrix,
            &clip,
        );
        let different = baseline
            .iter()
            .zip(&selected)
            .filter(|(a, b)| a != b)
            .count();
        assert_eq!(
            different, 0,
            "transformed selection changed {different} pixels: {matrix:?}"
        );
    }
}
