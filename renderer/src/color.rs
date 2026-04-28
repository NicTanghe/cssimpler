use cssimpler_core::{Color, LinearRgba};

const TEN_BIT_MAX: u32 = 1023;
const TWO_BIT_MAX: u32 = 3;

const RED_SHIFT: u32 = 22;
const GREEN_SHIFT: u32 = 12;
const BLUE_SHIFT: u32 = 2;
const ALPHA_SHIFT: u32 = 0;

const TEN_BIT_MASK: u32 = TEN_BIT_MAX;
const OPAQUE_ALPHA_2BIT: u32 = TWO_BIT_MAX;

pub(crate) fn pack_rgb(color: Color) -> u32 {
    pack_internal_rgb_u10(
        u8_to_u10_channel(color.r),
        u8_to_u10_channel(color.g),
        u8_to_u10_channel(color.b),
        OPAQUE_ALPHA_2BIT as u16,
    )
}

pub(crate) fn pack_linear_rgb(color: LinearRgba) -> u32 {
    pack_internal_rgb_u10(
        srgb_unit_to_u10(linear_to_srgb_unit(color.r)),
        srgb_unit_to_u10(linear_to_srgb_unit(color.g)),
        srgb_unit_to_u10(linear_to_srgb_unit(color.b)),
        OPAQUE_ALPHA_2BIT as u16,
    )
}

pub(crate) fn pack_rgba(color: Color) -> u32 {
    pack_internal_rgb_u10(
        u8_to_u10_channel(color.r),
        u8_to_u10_channel(color.g),
        u8_to_u10_channel(color.b),
        u8_to_u2_alpha(color.a),
    )
}

pub(crate) fn pack_transparent() -> u32 {
    pack_internal_rgb_u10(0, 0, 0, 0)
}

pub(crate) fn is_transparent(pixel: u32) -> bool {
    extract_alpha(pixel) == 0
}

pub(crate) fn unpack_rgb(pixel: u32) -> Color {
    Color::rgb(
        u10_to_u8_channel(extract_red(pixel)),
        u10_to_u8_channel(extract_green(pixel)),
        u10_to_u8_channel(extract_blue(pixel)),
    )
}

pub(crate) fn unpack_linear_rgb(pixel: u32) -> LinearRgba {
    LinearRgba {
        r: srgb_unit_to_linear(extract_red(pixel) as f32 / TEN_BIT_MAX as f32),
        g: srgb_unit_to_linear(extract_green(pixel) as f32 / TEN_BIT_MAX as f32),
        b: srgb_unit_to_linear(extract_blue(pixel) as f32 / TEN_BIT_MAX as f32),
        a: extract_alpha(pixel) as f32 / TWO_BIT_MAX as f32,
    }
}

pub(crate) fn unpack_rgb10(pixel: u32) -> (u16, u16, u16) {
    (
        extract_red(pixel),
        extract_green(pixel),
        extract_blue(pixel),
    )
}

pub(crate) fn unpack_alpha8(pixel: u32) -> u8 {
    ((u32::from(extract_alpha(pixel)) * 255 + (TWO_BIT_MAX / 2)) / TWO_BIT_MAX) as u8
}

pub(crate) fn pack_softbuffer_rgb(color: Color) -> u32 {
    pack_softbuffer_channels(color.r, color.g, color.b)
}

pub(crate) fn pack_softbuffer_channels(red: u8, green: u8, blue: u8) -> u32 {
    (u32::from(red) << 16) | (u32::from(green) << 8) | u32::from(blue)
}

pub(crate) fn u8_to_u10_channel(channel: u8) -> u16 {
    ((u32::from(channel) * TEN_BIT_MAX + 127) / 255) as u16
}

pub(crate) fn u10_to_u8_channel(channel: u16) -> u8 {
    ((u32::from(channel).min(TEN_BIT_MAX) * 255 + (TEN_BIT_MAX / 2)) / TEN_BIT_MAX) as u8
}

fn u8_to_u2_alpha(alpha: u8) -> u16 {
    ((u32::from(alpha) * TWO_BIT_MAX + 127) / 255) as u16
}

fn pack_internal_rgb_u10(red: u16, green: u16, blue: u16, alpha: u16) -> u32 {
    ((u32::from(red) & TEN_BIT_MASK) << RED_SHIFT)
        | ((u32::from(green) & TEN_BIT_MASK) << GREEN_SHIFT)
        | ((u32::from(blue) & TEN_BIT_MASK) << BLUE_SHIFT)
        | ((u32::from(alpha) & TWO_BIT_MAX) << ALPHA_SHIFT)
}

fn extract_red(pixel: u32) -> u16 {
    ((pixel >> RED_SHIFT) & TEN_BIT_MASK) as u16
}

fn extract_green(pixel: u32) -> u16 {
    ((pixel >> GREEN_SHIFT) & TEN_BIT_MASK) as u16
}

fn extract_blue(pixel: u32) -> u16 {
    ((pixel >> BLUE_SHIFT) & TEN_BIT_MASK) as u16
}

fn extract_alpha(pixel: u32) -> u16 {
    ((pixel >> ALPHA_SHIFT) & TWO_BIT_MAX) as u16
}

fn srgb_unit_to_u10(value: f32) -> u16 {
    (value.clamp(0.0, 1.0) * TEN_BIT_MAX as f32).round() as u16
}

fn srgb_unit_to_linear(value: f32) -> f32 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb_unit(value: f32) -> f32 {
    let value = value.clamp(0.0, 1.0);
    if value <= 0.003_130_8 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}

#[cfg(test)]
mod tests {
    use cssimpler_core::Color;

    use super::{
        is_transparent, pack_rgb, pack_rgba, pack_transparent, u8_to_u10_channel, unpack_alpha8,
        unpack_rgb,
    };

    #[test]
    fn pack_and_unpack_rgb_round_trips_8bit_inputs() {
        let samples = [
            Color::rgb(0, 0, 0),
            Color::rgb(255, 255, 255),
            Color::rgb(15, 23, 42),
            Color::rgb(37, 99, 235),
            Color::rgb(244, 114, 182),
        ];

        for sample in samples {
            let packed = pack_rgb(sample);
            assert_eq!(unpack_rgb(packed), sample);
        }
    }

    #[test]
    fn u8_to_u10_is_monotonic() {
        let mut previous = 0_u16;
        for value in 0_u16..=255 {
            let current = u8_to_u10_channel(value as u8);
            assert!(current >= previous);
            previous = current;
        }
    }

    #[test]
    fn transparent_pixel_preserves_alpha_state() {
        assert!(is_transparent(pack_transparent()));
        assert!(!is_transparent(pack_rgb(Color::BLACK)));
    }

    #[test]
    fn pack_rgba_preserves_quantized_alpha_state() {
        let transparent = pack_rgba(Color::rgba(10, 20, 30, 0));
        let translucent = pack_rgba(Color::rgba(10, 20, 30, 128));
        let opaque = pack_rgba(Color::rgba(10, 20, 30, 255));

        assert!(is_transparent(transparent));
        assert!(!is_transparent(translucent));
        assert_eq!(unpack_alpha8(opaque), 255);
        assert!(unpack_alpha8(translucent) < 255);
    }
}
