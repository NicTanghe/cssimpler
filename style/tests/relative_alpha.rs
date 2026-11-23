use cssimpler_core::{Color, ElementNode};
use cssimpler_style::{parse_stylesheet, resolve_style};

#[test]
fn relative_alpha_is_applied_to_native_paint() {
    let source = "
        label {
            --base: rgb(20 40 60 / 50%);
            color: alpha(from rgb(20 40 60 / 50%) / calc(alpha * 0.5));
            background-color: alpha(from var(--base) / 25%);
        }
    ";
    let sheet = parse_stylesheet(source).unwrap();
    let style = resolve_style(&ElementNode::new("label"), &sheet);
    assert_eq!(style.visual.foreground, Color::rgba(20, 40, 60, 64));
    assert_eq!(style.visual.background, Some(Color::rgba(20, 40, 60, 64)));
}
