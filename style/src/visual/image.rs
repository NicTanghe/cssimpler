use crate::{Declaration, StyleError};
use cssimpler_core::images::ImageFit;
use lightningcss::{
    properties::{Property, background::Background},
    traits::ToCss,
    values::image::Image,
};

pub(super) fn source(images: &[Image<'_>]) -> Option<String> {
    images.iter().find_map(|image| {
        if let Image::Url(url) = image {
            Some(url.url.to_string())
        } else {
            None
        }
    })
}
fn fit(value: &str) -> ImageFit {
    match value {
        "cover" => ImageFit::Cover,
        "contain" => ImageFit::Contain,
        "fill" | "100% 100%" => ImageFit::Fill,
        _ => ImageFit::Auto,
    }
}
fn fraction(value: &str) -> f32 {
    match value {
        "center" => 0.5,
        "right" | "bottom" => 1.0,
        _ => value
            .strip_suffix('%')
            .and_then(|v| v.parse::<f32>().ok())
            .map_or(0.0, |v| v / 100.0),
    }
}
fn position(value: &str) -> (f32, f32) {
    let parts: Vec<_> = value.split_whitespace().collect();
    match parts.as_slice() {
        ["top"] => (0.5, 0.0),
        ["bottom"] => (0.5, 1.0),
        [x] => (fraction(x), 0.5),
        [x, y] => (fraction(x), fraction(y)),
        _ => (0.0, 0.0),
    }
}
fn repeat(value: &str) -> (bool, bool) {
    match value {
        "no-repeat" => (false, false),
        "repeat-x" => (true, false),
        "repeat-y" => (false, true),
        _ => (true, true),
    }
}
fn css(value: &impl ToCss) -> String {
    value.to_css_string(Default::default()).unwrap_or_default()
}

pub(super) fn backgrounds(backgrounds: &[Background<'_>]) -> Vec<Declaration> {
    let chosen = backgrounds
        .iter()
        .find(|b| matches!(b.image, Image::Url(_)))
        .or_else(|| backgrounds.first());
    if let Some(b) = chosen {
        vec![
            Declaration::RasterSource(source(std::slice::from_ref(&b.image))),
            Declaration::RasterSize(fit(&css(&b.size))),
            Declaration::RasterPosition(position(&css(&b.position))),
            Declaration::RasterRepeat(repeat(&css(&b.repeat))),
        ]
    } else {
        vec![Declaration::RasterSource(None)]
    }
}
pub(super) fn extract(property: &Property<'_>) -> Option<Result<Vec<Declaration>, StyleError>> {
    let declaration = match property {
        Property::BackgroundSize(values) => Declaration::RasterSize(fit(&css(values.first()?))),
        Property::BackgroundPosition(values) => {
            Declaration::RasterPosition(position(&css(values.first()?)))
        }
        Property::BackgroundRepeat(values) => {
            Declaration::RasterRepeat(repeat(&css(values.first()?)))
        }
        Property::Custom(custom) if custom.name.as_ref() == "object-fit" => {
            Declaration::RasterSize(fit(&property
                .value_to_css_string(Default::default())
                .ok()?))
        }
        Property::Opacity(value) => {
            Declaration::RasterOpacity(css(value).parse::<f32>().ok()?.clamp(0.0, 1.0))
        }
        _ => return None,
    };
    Some(Ok(vec![declaration]))
}
