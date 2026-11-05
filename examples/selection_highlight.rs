//! Render connected selection highlights without opening a native window.
use cssimpler::core::{
    Color, LayoutBox, RenderNode, TextEditDecoration, TextSelectionRange, TextSelectionStyle,
    VisualStyle,
};
use cssimpler::fonts::{FontFamily, GenericFontFamily, LineHeight, TextStyle, WhiteSpace};
use cssimpler::renderer::{prepare_text_selection_backgrounds, render_to_buffer};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "selection-highlight.ppm".into());
    let rows = [
        (20.0, "Public key for administrator enrollment", false),
        (50.0, "-----BEGIN PUBLIC KEY-----", true),
        (
            70.0,
            "MCowBQYDK2VwAyEAgB+buwtEEtyLZMgvdFVl7Akb0gVP2efk0VMuE6JMo=",
            true,
        ),
        (90.0, "-----END PUBLIC KEY-----", true),
        (
            130.0,
            "C:\\Users\\example\\AppData\\Local\\Cashin\\cashin.register-public.pem",
            false,
        ),
    ];
    let children = rows
        .into_iter()
        .map(|(y, text, monospace)| {
            RenderNode::text(LayoutBox::new(20.0, y, 580.0, 20.0), text)
                .with_style(VisualStyle {
                    foreground: Color::rgb(244, 241, 236),
                    text: TextStyle {
                        families: vec![FontFamily::Generic(if monospace {
                            GenericFontFamily::Monospace
                        } else {
                            GenericFontFamily::SansSerif
                        })],
                        size_px: 13.0,
                        line_height: LineHeight::Px(20.0),
                        white_space: WhiteSpace::Pre,
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .with_text_edit(TextEditDecoration {
                    caret: None,
                    backgrounds: None,
                    selection: Some(TextSelectionRange {
                        start: 0,
                        end: text.len(),
                        style: TextSelectionStyle {
                            background: Color::rgb(179, 215, 255),
                            foreground: Color::rgb(30, 40, 50),
                            ..Default::default()
                        },
                    }),
                })
        })
        .collect::<Vec<_>>();
    let mut scene = vec![
        RenderNode::container(LayoutBox::new(0.0, 0.0, 620.0, 180.0))
            .with_style(VisualStyle {
                background: Some(Color::rgb(36, 36, 39)),
                ..Default::default()
            })
            .with_children(children),
    ];
    prepare_text_selection_backgrounds(&mut scene);
    let mut pixels = vec![0; 620 * 180];
    render_to_buffer(&scene, &mut pixels, 620, 180, Color::BLACK);
    let mut bytes = b"P6\n620 180\n255\n".to_vec();
    for pixel in pixels {
        let channel = |shift: u32| ((((pixel >> shift) & 1023) * 255 + 511) / 1023) as u8;
        bytes.extend([channel(22), channel(12), channel(2)]);
    }
    std::fs::write(output, bytes)?;
    Ok(())
}
