use cssimpler_core::{LinearRgba, MixBlendMode, RenderNode, establishes_stacking_context};

use super::{
    BufferRows, CachedSubtreeBounds, ClipRect, blend_linear_over, clip_pixel_bounds,
    current_alpha_at, current_render_buffer_rows, linear_to_srgb_unit, pack_linear_rgb,
    pack_transparent, set_current_alpha_at, srgb_unit_to_linear, unpack_linear_rgb,
    with_render_alpha_target, with_render_buffer_rows,
};

pub(crate) fn needs_group(node: &RenderNode, bounds: &CachedSubtreeBounds) -> bool {
    bounds.subtree_uses_mix_blend
        // Backdrop filters take a full-window snapshot. They need separate
        // handling when a subtree is painted into a row-local group.
        && !bounds.subtree_uses_backdrop_blur
        && !bounds.subtree_uses_backdrop_occlusion
        && !bounds.subtree_uses_native_glass
        && establishes_stacking_context(node)
}

pub(crate) fn draw_group(
    mode: MixBlendMode,
    target: &mut [u32],
    width: usize,
    height: usize,
    bounds: ClipRect,
    draw: impl FnOnce(&mut [u32]),
) {
    let Some((x0, y0, x1, y1)) = clip_pixel_bounds(bounds, width, height) else {
        return;
    };
    let target_rows = current_render_buffer_rows();
    let group_rows = BufferRows::new(
        (y0 as usize).max(target_rows.start),
        (y1 as usize).min(target_rows.end),
    );
    if group_rows.len() == 0 {
        return;
    }
    let pixel_count = group_rows.pixel_len(width);
    let mut colors = vec![pack_transparent(); pixel_count];
    let mut alpha = vec![0; pixel_count];
    with_render_buffer_rows(group_rows, || {
        with_render_alpha_target(Some(&mut alpha), || draw(&mut colors));
    });
    for y in group_rows.start..group_rows.end {
        let source_row = (y - group_rows.start) * width;
        let target_row = (y - target_rows.start) * width;
        for x in x0 as usize..x1 as usize {
            let source_index = source_row + x;
            let source_alpha = f32::from(alpha[source_index]) / 255.0;
            if source_alpha == 0.0 {
                continue;
            }
            let index = target_row + x;
            let mut source = unpack_linear_rgb(colors[source_index]);
            source.a = source_alpha;
            composite(mode, target, index, source);
        }
    }
}

fn composite(mode: MixBlendMode, target: &mut [u32], index: usize, mut source: LinearRgba) {
    if mode == MixBlendMode::Normal {
        blend_linear_over(target, index, source);
        return;
    }
    let backdrop = unpack_linear_rgb(target[index]);
    let backdrop_alpha = current_alpha_at(index).map_or(1.0, |value| f32::from(value) / 255.0);
    if backdrop_alpha == 0.0 && mode != MixBlendMode::PlusDarker {
        blend_linear_over(target, index, source);
        return;
    }
    let source_rgb = [source.r, source.g, source.b].map(linear_to_srgb_unit);
    let backdrop_rgb = [backdrop.r, backdrop.g, backdrop.b].map(linear_to_srgb_unit);
    if matches!(mode, MixBlendMode::PlusDarker | MixBlendMode::PlusLighter) {
        let add = |source: f32, backdrop: f32| {
            if mode == MixBlendMode::PlusLighter {
                (source + backdrop).min(1.0)
            } else {
                (source + backdrop - 1.0).max(0.0)
            }
        };
        let alpha = add(source.a, backdrop_alpha);
        if alpha == 0.0 {
            target[index] = pack_transparent();
            set_current_alpha_at(index, 0);
            return;
        }
        let [r, g, b] = std::array::from_fn(|i| {
            srgb_unit_to_linear(
                add(source_rgb[i] * source.a, backdrop_rgb[i] * backdrop_alpha) / alpha,
            )
        });
        target[index] = pack_linear_rgb(LinearRgba { r, g, b, a: alpha });
        set_current_alpha_at(index, (alpha * 255.0).round() as u8);
        return;
    }

    let blended = blend(mode, backdrop_rgb, source_rgb).map(srgb_unit_to_linear);
    [source.r, source.g, source.b] = std::array::from_fn(|i| {
        let original = [source.r, source.g, source.b][i];
        original + (blended[i] - original) * backdrop_alpha
    });
    blend_linear_over(target, index, source);
}

fn blend(mode: MixBlendMode, backdrop: [f32; 3], source: [f32; 3]) -> [f32; 3] {
    match mode {
        MixBlendMode::Hue => set_luminosity(
            set_saturation(source, saturation(backdrop)),
            luminosity(backdrop),
        ),
        MixBlendMode::Saturation => set_luminosity(
            set_saturation(backdrop, saturation(source)),
            luminosity(backdrop),
        ),
        MixBlendMode::Color => set_luminosity(source, luminosity(backdrop)),
        MixBlendMode::Luminosity => set_luminosity(backdrop, luminosity(source)),
        _ => std::array::from_fn(|i| blend_channel(mode, backdrop[i], source[i])),
    }
}

fn blend_channel(mode: MixBlendMode, b: f32, s: f32) -> f32 {
    let screen = |x, y| x + y - x * y;
    let hard_light = |backdrop, source| {
        if source <= 0.5 {
            backdrop * (2.0 * source)
        } else {
            screen(backdrop, 2.0 * source - 1.0)
        }
    };
    match mode {
        MixBlendMode::Multiply => b * s,
        MixBlendMode::Screen => screen(b, s),
        MixBlendMode::Overlay => hard_light(s, b),
        MixBlendMode::Darken => b.min(s),
        MixBlendMode::Lighten => b.max(s),
        MixBlendMode::ColorDodge if b == 0.0 => 0.0,
        MixBlendMode::ColorDodge if s == 1.0 => 1.0,
        MixBlendMode::ColorDodge => (b / (1.0 - s)).min(1.0),
        MixBlendMode::ColorBurn if b == 1.0 => 1.0,
        MixBlendMode::ColorBurn if s == 0.0 => 0.0,
        MixBlendMode::ColorBurn => 1.0 - ((1.0 - b) / s).min(1.0),
        MixBlendMode::HardLight => hard_light(b, s),
        MixBlendMode::SoftLight if s <= 0.5 => b - (1.0 - 2.0 * s) * b * (1.0 - b),
        MixBlendMode::SoftLight => {
            let d = if b <= 0.25 {
                ((16.0 * b - 12.0) * b + 4.0) * b
            } else {
                b.sqrt()
            };
            b + (2.0 * s - 1.0) * (d - b)
        }
        MixBlendMode::Difference => (b - s).abs(),
        MixBlendMode::Exclusion => b + s - 2.0 * b * s,
        _ => s,
    }
}

fn luminosity([r, g, b]: [f32; 3]) -> f32 {
    0.3 * r + 0.59 * g + 0.11 * b
}

fn extremes(rgb: [f32; 3]) -> (f32, f32) {
    let [r, g, b] = rgb;
    (r.min(g).min(b), r.max(g).max(b))
}

fn saturation(rgb: [f32; 3]) -> f32 {
    let (min, max) = extremes(rgb);
    max - min
}

fn set_saturation(rgb: [f32; 3], saturation: f32) -> [f32; 3] {
    let (min, max) = extremes(rgb);
    if max > min {
        rgb.map(|component| (component - min) * saturation / (max - min))
    } else {
        [0.0; 3]
    }
}

fn set_luminosity(rgb: [f32; 3], target: f32) -> [f32; 3] {
    let delta = target - luminosity(rgb);
    let rgb = rgb.map(|component| component + delta);
    let (min, _) = extremes(rgb);
    let rgb = if min < 0.0 {
        rgb.map(|component| target + (component - target) * target / (target - min))
    } else {
        rgb
    };
    let (_, max) = extremes(rgb);
    if max > 1.0 {
        rgb.map(|component| target + (component - target) * (1.0 - target) / (max - target))
    } else {
        rgb
    }
}

#[cfg(test)]
mod tests {
    use cssimpler_core::{
        Color, LayoutBox, LengthPercentageValue, MixBlendMode, RenderNode, TransformOperation,
        VisualStyle,
    };

    use super::super::{render_scene_update, render_to_buffer, unpack_rgb};

    fn rect(background: Color, mix_blend_mode: MixBlendMode) -> RenderNode {
        RenderNode::container(LayoutBox::new(0.0, 0.0, 8.0, 8.0)).with_style(VisualStyle {
            background: Some(background),
            mix_blend_mode,
            ..VisualStyle::default()
        })
    }

    #[test]
    fn multiply_blends_the_subtree_as_one_layer() {
        let layer = rect(Color::rgb(0, 0, 255), MixBlendMode::Multiply)
            .with_child(rect(Color::rgb(255, 0, 0), MixBlendMode::Normal));
        let mut pixels = [0; 64];
        render_to_buffer(&[layer], &mut pixels, 8, 8, Color::rgb(0, 255, 255));
        assert_eq!(unpack_rgb(pixels[0]), Color::BLACK);
    }

    #[test]
    fn isolated_descendant_cannot_blend_with_the_surrounding_scene() {
        let isolate = RenderNode::container(LayoutBox::new(0.0, 0.0, 8.0, 8.0))
            .with_style(VisualStyle {
                isolate_blending: true,
                ..VisualStyle::default()
            })
            .with_child(rect(Color::rgb(255, 0, 0), MixBlendMode::Multiply));
        let mut pixels = [0; 64];
        render_to_buffer(&[isolate], &mut pixels, 8, 8, Color::rgb(0, 0, 255));
        assert_eq!(unpack_rgb(pixels[0]), Color::rgb(255, 0, 0));
    }

    #[test]
    fn transformed_layer_blends_in_screen_coordinates() {
        let mut layer = rect(Color::rgb(255, 0, 0), MixBlendMode::Screen);
        layer
            .style
            .transform
            .operations
            .push(TransformOperation::Translate {
                x: LengthPercentageValue::from_px(8.0),
                y: LengthPercentageValue::from_px(0.0),
            });
        let mut pixels = [0; 128];
        render_to_buffer(&[layer], &mut pixels, 16, 8, Color::rgb(0, 0, 255));
        assert_eq!(unpack_rgb(pixels[0]), Color::rgb(0, 0, 255));
        assert_eq!(unpack_rgb(pixels[8]), Color::rgb(255, 0, 255));
    }

    #[test]
    fn transparent_backdrop_keeps_the_original_alpha_and_color() {
        let layer = rect(Color::rgba(255, 30, 0, 128), MixBlendMode::Multiply);
        let mut pixels = [0; 256];
        super::super::render_to_rgba8(&[layer], &mut pixels, 8, 8, Color::rgba(0, 0, 0, 0));
        assert_eq!(&pixels[..4], &[255, 30, 0, 128]);
    }

    #[test]
    fn transformed_container_keeps_descendant_blend_inside_its_group() {
        let child = rect(Color::rgb(255, 0, 0), MixBlendMode::Multiply);
        let mut group = RenderNode::container(LayoutBox::new(0.0, 0.0, 8.0, 8.0)).with_child(child);
        group
            .style
            .transform
            .operations
            .push(TransformOperation::Translate {
                x: LengthPercentageValue::from_px(8.0),
                y: LengthPercentageValue::from_px(8.0),
            });
        let root = RenderNode::container(LayoutBox::new(0.0, 0.0, 24.0, 24.0)).with_child(group);
        let mut pixels = [0; 576];
        render_to_buffer(&[root], &mut pixels, 24, 24, Color::rgb(0, 0, 255));
        assert_eq!(unpack_rgb(pixels[9 * 24 + 9]), Color::rgb(255, 0, 0));
    }

    #[test]
    fn incremental_paint_reblends_when_the_backdrop_changes() {
        let layer = rect(Color::rgb(255, 0, 0), MixBlendMode::Screen);
        let old = [rect(Color::BLACK, MixBlendMode::Normal), layer.clone()];
        let new = [rect(Color::rgb(0, 0, 255), MixBlendMode::Normal), layer];
        let mut incremental = [0; 64];
        let mut full = incremental;
        render_to_buffer(&old, &mut incremental, 8, 8, Color::WHITE);
        render_scene_update(&old, &new, &mut incremental, 8, 8, Color::WHITE);
        render_to_buffer(&new, &mut full, 8, 8, Color::WHITE);
        assert_eq!(incremental, full);
        assert_eq!(unpack_rgb(incremental[0]), Color::rgb(255, 0, 255));
    }
}
