use cssimpler_core::{Color, OverflowMode, ScrollbarData, ScrollbarMetrics, ScrollbarWidth, Style};
use lightningcss::properties::custom::CustomProperty;
use lightningcss::properties::{Property, PropertyId};
use lightningcss::stylesheet::{ParserOptions, PrinterOptions};
use taffy::Layout as TaffyLayout;

use crate::{Declaration, StyleError};

use super::color;

pub(super) fn custom_property_declarations(
    property: &Property<'_>,
) -> Option<Result<Vec<Declaration>, StyleError>> {
    let Property::Custom(custom) = property else {
        return None;
    };

    if custom.name.as_ref().eq_ignore_ascii_case("scrollbar-width") {
        return Some(scrollbar_width_declaration(custom));
    }

    if custom.name.as_ref().eq_ignore_ascii_case("scrollbar-color") {
        return Some(scrollbar_color_declaration(custom));
    }

    None
}

pub(super) fn apply_scrollbar_width(style: &mut Style, width: ScrollbarWidth) {
    style.visual.scrollbar.width = width;
    sync_taffy_scrollbar_width(style);
}

pub(super) fn apply_scrollbar_colors(
    style: &mut Style,
    thumb_color: Option<Color>,
    track_color: Option<Color>,
) {
    style.visual.scrollbar.thumb_color = thumb_color;
    style.visual.scrollbar.track_color = track_color;
}

pub(crate) fn sync_taffy_scrollbar_width(style: &mut Style) {
    style.layout.taffy.scrollbar_width =
        if style.visual.overflow.x.reserves_gutter() || style.visual.overflow.y.reserves_gutter() {
            style.visual.scrollbar.width.resolve_px()
        } else {
            0.0
        };
}

pub(crate) fn scrollbars_from_layout(style: &Style, layout: &TaffyLayout) -> Option<ScrollbarData> {
    if !style.visual.overflow.allows_scrolling() {
        return None;
    }

    Some(ScrollbarData::new(
        style.visual.overflow.x,
        style.visual.overflow.y,
        style.visual.scrollbar,
        ScrollbarMetrics {
            offset_x: 0.0,
            offset_y: 0.0,
            max_offset_x: layout.scroll_width(),
            max_offset_y: layout.scroll_height(),
            reserved_width: layout.scrollbar_size.width,
            reserved_height: layout.scrollbar_size.height,
        },
    ))
}

pub(crate) fn taffy_overflow_from_mode(mode: OverflowMode) -> taffy::Overflow {
    match mode {
        OverflowMode::Visible => taffy::Overflow::Visible,
        OverflowMode::Clip => taffy::Overflow::Clip,
        OverflowMode::Hidden | OverflowMode::Auto => taffy::Overflow::Hidden,
        OverflowMode::Scroll => taffy::Overflow::Scroll,
    }
}

fn scrollbar_width_declaration(
    custom: &CustomProperty<'_>,
) -> Result<Vec<Declaration>, StyleError> {
    let value = custom_value_to_css(custom)?;
    let width = match value.trim() {
        value if value.eq_ignore_ascii_case("auto") => ScrollbarWidth::Auto,
        value if value.eq_ignore_ascii_case("thin") => ScrollbarWidth::Thin,
        value if value.eq_ignore_ascii_case("none") => ScrollbarWidth::None,
        value => {
            let numeric = value
                .strip_suffix("px")
                .ok_or_else(|| StyleError::UnsupportedValue(value.to_string()))?
                .trim();
            let parsed = numeric
                .parse::<f32>()
                .map_err(|_| StyleError::UnsupportedValue(value.to_string()))?;
            ScrollbarWidth::Px(parsed)
        }
    };

    Ok(vec![Declaration::ScrollbarWidth(width)])
}

fn scrollbar_color_declaration(
    custom: &CustomProperty<'_>,
) -> Result<Vec<Declaration>, StyleError> {
    let value = custom_value_to_css(custom)?;
    if value.trim().eq_ignore_ascii_case("auto") {
        return Ok(vec![Declaration::ScrollbarColors(None, None)]);
    }

    let parts = split_top_level_whitespace(&value);
    if parts.is_empty() || parts.len() > 2 {
        return Err(StyleError::UnsupportedValue(value));
    }

    let thumb_color = parse_css_color(parts[0])?;
    let track_color = if parts.len() == 2 {
        Some(parse_css_color(parts[1])?)
    } else {
        None
    };

    Ok(vec![Declaration::ScrollbarColors(
        Some(thumb_color),
        track_color,
    )])
}

fn custom_value_to_css(custom: &CustomProperty<'_>) -> Result<String, StyleError> {
    Property::Custom(custom.clone())
        .value_to_css_string(PrinterOptions::default())
        .map_err(|error| StyleError::UnsupportedValue(error.to_string()))
}

fn parse_css_color(value: &str) -> Result<Color, StyleError> {
    let property =
        Property::parse_string(PropertyId::from("color"), value, ParserOptions::default())
            .map_err(|_| StyleError::UnsupportedValue(value.to_string()))?;

    match property {
        Property::Color(color_value) => color::color_from_css(&color_value),
        _ => Err(StyleError::UnsupportedValue(value.to_string())),
    }
}

fn split_top_level_whitespace(value: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = None;
    let mut depth = 0_i32;

    for (index, ch) in value.char_indices() {
        match ch {
            '(' => {
                depth += 1;
                start.get_or_insert(index);
            }
            ')' => {
                depth = (depth - 1).max(0);
            }
            ch if ch.is_whitespace() && depth == 0 => {
                if let Some(start_index) = start.take() {
                    parts.push(value[start_index..index].trim());
                }
                continue;
            }
            _ => {
                start.get_or_insert(index);
            }
        }
    }

    if let Some(start_index) = start {
        parts.push(value[start_index..].trim());
    }

    parts.retain(|part| !part.is_empty());
    parts
}
