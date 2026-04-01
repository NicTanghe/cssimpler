#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GradientInterpolation {
    LinearSrgb,
    #[default]
    Oklab,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const WHITE: Self = Self::rgb(255, 255, 255);
    pub const BLACK: Self = Self::rgb(0, 0, 0);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub const fn with_alpha(self, a: u8) -> Self {
        Self { a, ..self }
    }

    pub fn to_linear_rgba(self) -> LinearRgba {
        LinearRgba {
            r: srgb_channel_to_linear(self.r),
            g: srgb_channel_to_linear(self.g),
            b: srgb_channel_to_linear(self.b),
            a: self.a as f32 / 255.0,
        }
    }

    pub fn from_linear_rgba(color: LinearRgba) -> Self {
        Self {
            r: linear_channel_to_srgb(color.r),
            g: linear_channel_to_srgb(color.g),
            b: linear_channel_to_srgb(color.b),
            a: (color.a.clamp(0.0, 1.0) * 255.0).round() as u8,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinearRgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl LinearRgba {
    pub const TRANSPARENT: Self = Self {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    };

    pub fn lerp(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        Self {
            r: mix(self.r, other.r, t),
            g: mix(self.g, other.g, t),
            b: mix(self.b, other.b, t),
            a: mix(self.a, other.a, t),
        }
    }

    pub fn interpolate(self, other: Self, t: f32, interpolation: GradientInterpolation) -> Self {
        let t = t.clamp(0.0, 1.0);
        if matches!(interpolation, GradientInterpolation::LinearSrgb) {
            return self.lerp(other, t);
        }

        let start = self.to_oklab();
        let end = other.to_oklab();
        Self::from_oklab(start.lerp(end, t), mix(self.a, other.a, t))
    }

    fn to_oklab(self) -> Oklab {
        let l = 0.412_221_470_8 * self.r + 0.536_332_536_3 * self.g + 0.051_445_992_9 * self.b;
        let m = 0.211_903_498_2 * self.r + 0.680_699_545_1 * self.g + 0.107_396_956_6 * self.b;
        let s = 0.088_302_461_9 * self.r + 0.281_718_837_6 * self.g + 0.629_978_700_5 * self.b;

        let l_prime = l.cbrt();
        let m_prime = m.cbrt();
        let s_prime = s.cbrt();

        Oklab {
            l: 0.210_454_255_3 * l_prime + 0.793_617_785 * m_prime - 0.004_072_046_8 * s_prime,
            a: 1.977_998_495_1 * l_prime - 2.428_592_205 * m_prime + 0.450_593_709_9 * s_prime,
            b: 0.025_904_037_1 * l_prime + 0.782_771_766_2 * m_prime - 0.808_675_766 * s_prime,
        }
    }

    fn from_oklab(color: Oklab, alpha: f32) -> Self {
        let l_prime = color.l + 0.396_337_777_4 * color.a + 0.215_803_757_3 * color.b;
        let m_prime = color.l - 0.105_561_345_8 * color.a - 0.063_854_172_8 * color.b;
        let s_prime = color.l - 0.089_484_177_5 * color.a - 1.291_485_548 * color.b;

        let l = l_prime * l_prime * l_prime;
        let m = m_prime * m_prime * m_prime;
        let s = s_prime * s_prime * s_prime;

        Self {
            r: 4.076_741_662_1 * l - 3.307_711_591_3 * m + 0.230_969_929_2 * s,
            g: -1.268_438_004_6 * l + 2.609_757_401_1 * m - 0.341_319_396_5 * s,
            b: -0.004_196_086_3 * l - 0.703_418_614_7 * m + 1.707_614_701 * s,
            a: alpha.clamp(0.0, 1.0),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Oklab {
    l: f32,
    a: f32,
    b: f32,
}

impl Oklab {
    fn lerp(self, other: Self, t: f32) -> Self {
        Self {
            l: mix(self.l, other.l, t),
            a: mix(self.a, other.a, t),
            b: mix(self.b, other.b, t),
        }
    }
}

fn mix(start: f32, end: f32, t: f32) -> f32 {
    start + (end - start) * t
}

fn srgb_channel_to_linear(channel: u8) -> f32 {
    let value = channel as f32 / 255.0;
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_channel_to_srgb(value: f32) -> u8 {
    let value = value.clamp(0.0, 1.0);
    let srgb = if value <= 0.003_130_8 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    };
    (srgb * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::{Color, GradientInterpolation, LinearRgba};

    #[test]
    fn linear_rgba_interpolates_in_oklab_by_default() {
        let start = Color::BLACK.to_linear_rgba();
        let end = Color::WHITE.to_linear_rgba();
        let midpoint = start.interpolate(end, 0.5, GradientInterpolation::Oklab);

        assert_eq!(Color::from_linear_rgba(midpoint), Color::rgb(99, 99, 99));
    }

    #[test]
    fn linear_rgba_can_still_interpolate_in_linear_srgb() {
        let start = Color::BLACK.to_linear_rgba();
        let end = Color::WHITE.to_linear_rgba();
        let midpoint = start.interpolate(end, 0.5, GradientInterpolation::LinearSrgb);

        assert_eq!(Color::from_linear_rgba(midpoint), Color::rgb(188, 188, 188));
    }

    #[test]
    fn oklab_round_trip_preserves_srgb_inputs() {
        let original = Color::rgb(29, 78, 216).to_linear_rgba();
        let round_trip = LinearRgba::from_oklab(original.to_oklab(), original.a);

        assert_eq!(Color::from_linear_rgba(round_trip), Color::rgb(29, 78, 216));
    }
}
