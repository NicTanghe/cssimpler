mod border;
mod color;
mod gradient;
mod scrollbar;
mod shadow;
mod transform;

pub use gradient::BackgroundLayerDeclaration;
pub use shadow::ShadowDeclaration;

use cssimpler_core::Style;
use lightningcss::properties::Property;
use taffy::Layout as TaffyLayout;

use crate::{Declaration, StyleError};

// Keep visual style handling in its own folder so borders, shadows, blur, and
// future color-pipeline work can grow without bloating the main style module.
pub(crate) fn extract_property(
    property: &Property<'_>,
) -> Option<Result<Vec<Declaration>, StyleError>> {
    if let Some(declarations) = scrollbar::custom_property_declarations(property) {
        return Some(declarations);
    }

    if let Some(declarations) = shadow::extract_unparsed_property(property) {
        return Some(declarations);
    }

    if let Property::Unparsed(value) = property
        && matches!(
            value.property_id,
            lightningcss::properties::PropertyId::TransformStyle(_)
        )
    {
        return Some(transform::unparsed_transform_style_declarations(value));
    }

    match property {
        Property::BackgroundColor(color) => Some(color::background_color_declaration(color)),
        Property::Background(backgrounds) => Some(gradient::background_declarations(backgrounds)),
        Property::BackgroundImage(images) => {
            Some(gradient::background_image_declarations(images.as_slice()))
        }
        Property::Color(color) => Some(color::foreground_color_declaration(color)),
        Property::BorderRadius(radius, _) => Some(border::border_radius_declarations(radius)),
        Property::BorderTopLeftRadius(radius, _) => {
            Some(border::border_top_left_radius_declaration(radius))
        }
        Property::BorderTopRightRadius(radius, _) => {
            Some(border::border_top_right_radius_declaration(radius))
        }
        Property::BorderBottomRightRadius(radius, _) => {
            Some(border::border_bottom_right_radius_declaration(radius))
        }
        Property::BorderBottomLeftRadius(radius, _) => {
            Some(border::border_bottom_left_radius_declaration(radius))
        }
        Property::Border(border_value) => Some(border::border_shorthand_declarations(border_value)),
        Property::BorderTop(border_value) => Some(border::border_top_declarations(border_value)),
        Property::BorderRight(border_value) => {
            Some(border::border_right_declarations(border_value))
        }
        Property::BorderBottom(border_value) => {
            Some(border::border_bottom_declarations(border_value))
        }
        Property::BorderLeft(border_value) => Some(border::border_left_declarations(border_value)),
        Property::BorderWidth(widths) => Some(border::border_width_declarations(widths)),
        Property::BorderTopWidth(value) => Some(border::border_top_width_declaration(value)),
        Property::BorderRightWidth(value) => Some(border::border_right_width_declaration(value)),
        Property::BorderBottomWidth(value) => Some(border::border_bottom_width_declaration(value)),
        Property::BorderLeftWidth(value) => Some(border::border_left_width_declaration(value)),
        Property::BorderColor(colors) => Some(border::border_color_declaration(&colors.top)),
        Property::BorderTopColor(color) => Some(border::border_color_declaration(color)),
        Property::BorderRightColor(color) => Some(border::border_color_declaration(color)),
        Property::BorderBottomColor(color) => Some(border::border_color_declaration(color)),
        Property::BorderLeftColor(color) => Some(border::border_color_declaration(color)),
        Property::BoxShadow(shadows, _) => {
            Some(shadow::box_shadow_declarations(shadows.as_slice()))
        }
        Property::TextShadow(shadows) => Some(shadow::text_shadow_declarations(shadows.as_slice())),
        Property::Filter(filters, _) => Some(shadow::filter_drop_shadow_declarations(filters)),
        Property::Transform(value, _) => Some(transform::transform_declarations(value)),
        Property::TransformOrigin(value, _) => {
            Some(transform::transform_origin_declarations(value))
        }
        Property::Perspective(value, _) => Some(transform::perspective_declarations(value)),
        Property::TransformStyle(value, _) => Some(transform::transform_style_declarations(value)),
        Property::Translate(value) => Some(transform::translate_declarations(value)),
        Property::Rotate(value) => Some(transform::rotate_declarations(value)),
        Property::Scale(value) => Some(transform::scale_declarations(value)),
        _ => None,
    }
}

pub(crate) fn apply_declaration(style: &mut Style, declaration: &Declaration) -> bool {
    match declaration {
        Declaration::Background(color) => {
            color::apply_background(style, *color);
            true
        }
        Declaration::BackgroundLayers(layers) => {
            gradient::apply_background_layers(style, layers);
            true
        }
        Declaration::Foreground(color) => {
            color::apply_foreground(style, *color);
            true
        }
        Declaration::CornerTopLeft(value) => {
            border::apply_corner_top_left(style, *value);
            true
        }
        Declaration::CornerTopRight(value) => {
            border::apply_corner_top_right(style, *value);
            true
        }
        Declaration::CornerBottomRight(value) => {
            border::apply_corner_bottom_right(style, *value);
            true
        }
        Declaration::CornerBottomLeft(value) => {
            border::apply_corner_bottom_left(style, *value);
            true
        }
        Declaration::BorderTopWidth(value) => {
            border::apply_border_top_width(style, *value);
            true
        }
        Declaration::BorderRightWidth(value) => {
            border::apply_border_right_width(style, *value);
            true
        }
        Declaration::BorderBottomWidth(value) => {
            border::apply_border_bottom_width(style, *value);
            true
        }
        Declaration::BorderLeftWidth(value) => {
            border::apply_border_left_width(style, *value);
            true
        }
        Declaration::BorderColor(color) => {
            border::apply_border_color(style, *color);
            true
        }
        Declaration::BoxShadows(shadows) => {
            shadow::apply_box_shadows(style, shadows);
            true
        }
        Declaration::TextShadows(shadows) => {
            shadow::apply_text_shadows(style, shadows);
            true
        }
        Declaration::FilterDropShadows(shadows) => {
            shadow::apply_filter_drop_shadows(style, shadows);
            true
        }
        Declaration::TextStrokeWidth(width) => {
            shadow::apply_text_stroke_width(style, *width);
            true
        }
        Declaration::TextStrokeColor(color) => {
            shadow::apply_text_stroke_color(style, *color);
            true
        }
        Declaration::TransformOperations(operations) => {
            transform::apply_transform_operations(style, operations);
            true
        }
        Declaration::TransformOrigin(origin) => {
            transform::apply_transform_origin(style, *origin);
            true
        }
        Declaration::Perspective(perspective) => {
            transform::apply_perspective(style, *perspective);
            true
        }
        Declaration::TransformStyle(mode) => {
            transform::apply_transform_style(style, *mode);
            true
        }
        Declaration::ScrollbarWidth(width) => {
            scrollbar::apply_scrollbar_width(style, *width);
            true
        }
        Declaration::ScrollbarColors(thumb_color, track_color) => {
            scrollbar::apply_scrollbar_colors(style, *thumb_color, *track_color);
            true
        }
        _ => false,
    }
}

pub(crate) fn scrollbars_from_layout(
    style: &Style,
    layout: &TaffyLayout,
) -> Option<cssimpler_core::ScrollbarData> {
    scrollbar::scrollbars_from_layout(style, layout)
}

pub(crate) fn sync_scrollbar_gutter(style: &mut Style) {
    scrollbar::sync_taffy_scrollbar_width(style);
}

pub(crate) fn taffy_overflow_from_mode(mode: cssimpler_core::OverflowMode) -> taffy::Overflow {
    scrollbar::taffy_overflow_from_mode(mode)
}
