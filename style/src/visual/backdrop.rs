use cssimpler_core::Style;
use lightningcss::printer::PrinterOptions;
use lightningcss::properties::effects::{Filter, FilterList};
use lightningcss::traits::ToCss;
use lightningcss::values::length::Length;

use crate::{Declaration, StyleError};

pub(super) fn backdrop_filter_declarations(
    filters: &FilterList<'_>,
) -> Result<Vec<Declaration>, StyleError> {
    match filters {
        FilterList::None => Ok(vec![Declaration::BackdropBlur(0.0)]),
        FilterList::Filters(filters) => {
            let [filter] = filters.as_slice() else {
                return Err(unsupported_backdrop_filter_value(filters));
            };
            let Filter::Blur(radius) = filter else {
                return Err(unsupported_backdrop_filter_value(filters));
            };
            Ok(vec![Declaration::BackdropBlur(length_to_px(radius)?.max(0.0))])
        }
    }
}

pub(super) fn apply_backdrop_blur(style: &mut Style, radius: f32) {
    style.visual.backdrop_blur_radius = radius.max(0.0);
}

fn length_to_px(value: &Length) -> Result<f32, StyleError> {
    value
        .to_px()
        .map(|value| value as f32)
        .ok_or_else(|| StyleError::UnsupportedValue(format!("{value:?}")))
}

fn unsupported_backdrop_filter_value(filters: &[Filter<'_>]) -> StyleError {
    StyleError::UnsupportedValue(format!(
        "unsupported backdrop-filter value: {} (only blur() is supported)",
        filters
            .iter()
            .map(|filter| {
                filter
                    .to_css_string(PrinterOptions::default())
                    .unwrap_or_else(|_| format!("{filter:?}"))
            })
            .collect::<Vec<_>>()
            .join(" ")
    ))
}
