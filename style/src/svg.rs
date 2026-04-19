use cssimpler_core::{
    Color, CustomProperties, ElementInteractionState, ElementNode, ElementPath, Node, Style,
    SvgContour, SvgPaint, SvgPathGeometry, SvgPathInstance, SvgPathPaint, SvgPoint, SvgScene,
    SvgStyle, SvgViewBox, fonts::TextStyle,
};
use lightningcss::properties::{Property, PropertyId};
use lightningcss::stylesheet::ParserOptions;
use taffy::prelude::Dimension;

use crate::{ElementRef, Stylesheet};

#[derive(Clone, Debug)]
pub(crate) struct ResolvedSvgRoot {
    pub(crate) style: Style,
    pub(crate) scene: SvgScene,
}

#[derive(Clone, Copy, Debug)]
struct SvgCascadeState {
    fill: SvgPaint,
    stroke: SvgPaint,
    stroke_width: f32,
    translate_x: f32,
    translate_y: f32,
}

impl Default for SvgCascadeState {
    fn default() -> Self {
        Self {
            fill: SvgPaint::Color(Color::BLACK),
            stroke: SvgPaint::None,
            stroke_width: 1.0,
            translate_x: 0.0,
            translate_y: 0.0,
        }
    }
}

pub(crate) fn is_supported_svg_tag(tag: &str) -> bool {
    matches!(tag, "svg" | "g" | "path" | "circle")
}

pub(crate) fn seed_element_style(element: &ElementNode) -> Style {
    let mut style = element.style.clone();
    if !is_supported_svg_tag(&element.tag) {
        return style;
    }

    for (name, value) in element.attributes() {
        if name == "id"
            || name == "class"
            || name == "viewBox"
            || name == "d"
            || name == "cx"
            || name == "cy"
            || name == "r"
            || name == "transform"
            || name == "stroke-linecap"
            || name == "stroke-linejoin"
            || name.starts_with("xmlns")
        {
            continue;
        }

        match (element.tag.as_str(), name.as_str()) {
            (_, "fill" | "stroke") => {
                apply_svg_attribute_property(&mut style, element, name, value);
            }
            (_, "stroke-width") => {
                apply_svg_attribute_property(
                    &mut style,
                    element,
                    name,
                    &normalize_svg_length_attribute(value),
                );
            }
            ("svg", "width" | "height") => {
                apply_svg_attribute_property(
                    &mut style,
                    element,
                    name,
                    &normalize_svg_length_attribute(value),
                );
            }
            _ => panic!("unsupported SVG attribute `{}` on <{}>", name, element.tag),
        }
    }

    style
}

pub(crate) fn resolve_svg_root(
    element: &ElementNode,
    stylesheet: &Stylesheet,
    mut style: Style,
    ancestors: &[ElementRef<'_>],
    interaction: &ElementInteractionState,
    element_path: &ElementPath,
) -> ResolvedSvgRoot {
    let metadata = parse_svg_root_metadata(element);
    apply_svg_root_intrinsic_size(&mut style, metadata.view_box);

    let mut child_ancestors = Vec::with_capacity(ancestors.len() + 1);
    child_ancestors.push(ElementRef::from(element));
    child_ancestors.extend_from_slice(ancestors);

    let inherited_svg = svg_cascade_from_style(SvgCascadeState::default(), &style.visual.svg);
    let mut paths = Vec::new();
    let mut child_index = 0;
    for child in &element.children {
        match child {
            Node::Text(text) if text.trim().is_empty() => {}
            Node::Text(_) => panic!("text nodes are not supported inside <svg>"),
            Node::Element(child) => {
                let child_path = element_path.with_child(child_index);
                child_index += 1;
                collect_svg_paths(
                    child,
                    stylesheet,
                    Some(&style.visual.text),
                    style.visual.foreground,
                    Some(&style.custom_properties),
                    &child_ancestors,
                    interaction,
                    &child_path,
                    inherited_svg,
                    &mut paths,
                );
            }
        }
    }

    ResolvedSvgRoot {
        style,
        scene: SvgScene::new(metadata.view_box, paths),
    }
}

fn apply_svg_attribute_property(
    style: &mut Style,
    element: &ElementNode,
    property_name: &str,
    property_value: &str,
) {
    let property = Property::parse_string(
        PropertyId::from(property_name),
        property_value,
        ParserOptions::default(),
    )
    .unwrap_or_else(|error| {
        panic!(
            "unsupported SVG attribute `{}` on <{}>: {}",
            property_name, element.tag, error
        )
    });
    let declarations = crate::extract_property(&property).unwrap_or_else(|error| {
        panic!(
            "unsupported SVG attribute `{}` on <{}>: {}",
            property_name, element.tag, error
        )
    });
    let mut position_explicit = style.layout.taffy.position != taffy::prelude::Position::Relative;
    let mut font_state = crate::fonts::FontDeclarationState::default();
    for declaration in declarations {
        crate::apply_declaration(style, &mut position_explicit, &mut font_state, &declaration);
    }
}

fn normalize_svg_length_attribute(value: &str) -> String {
    let value = value.trim();
    if value.parse::<f32>().is_ok() {
        format!("{value}px")
    } else {
        value.to_string()
    }
}

fn parse_svg_root_metadata(element: &ElementNode) -> SvgRootMetadata {
    let view_box = element
        .attribute("viewBox")
        .map(parse_view_box)
        .transpose()
        .unwrap_or_else(|error| panic!("unsupported SVG `viewBox` on <svg>: {error}"));
    let fallback_width = numeric_svg_length_attribute(element.attribute("width"))
        .or_else(|| view_box.map(|view_box| view_box.width))
        .unwrap_or(300.0);
    let fallback_height = numeric_svg_length_attribute(element.attribute("height"))
        .or_else(|| view_box.map(|view_box| view_box.height))
        .unwrap_or(150.0);

    SvgRootMetadata {
        view_box: view_box
            .unwrap_or_else(|| SvgViewBox::new(0.0, 0.0, fallback_width, fallback_height)),
    }
}

fn numeric_svg_length_attribute(value: Option<&str>) -> Option<f32> {
    let value = value?.trim();
    if let Ok(parsed) = value.parse::<f32>() {
        return Some(parsed);
    }

    let stripped = value.strip_suffix("px")?.trim();
    stripped.parse::<f32>().ok()
}

fn parse_view_box(value: &str) -> Result<SvgViewBox, String> {
    let numbers = value
        .replace(',', " ")
        .split_whitespace()
        .map(|part| {
            part.parse::<f32>()
                .map_err(|_| format!("invalid viewBox component `{part}`"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let [min_x, min_y, width, height] = numbers.as_slice() else {
        return Err("viewBox must contain four numbers".to_string());
    };
    if *width <= f32::EPSILON || *height <= f32::EPSILON {
        return Err("viewBox width and height must be positive".to_string());
    }

    Ok(SvgViewBox::new(*min_x, *min_y, *width, *height))
}

fn apply_svg_root_intrinsic_size(style: &mut Style, view_box: SvgViewBox) {
    if matches!(style.layout.taffy.size.width, Dimension::Auto) {
        style.layout.taffy.size.width = Dimension::Length(view_box.width.max(0.0));
    }
    if matches!(style.layout.taffy.size.height, Dimension::Auto) {
        style.layout.taffy.size.height = Dimension::Length(view_box.height.max(0.0));
    }
}

fn collect_svg_paths(
    element: &ElementNode,
    stylesheet: &Stylesheet,
    inherited_text: Option<&TextStyle>,
    inherited_foreground: Color,
    inherited_custom_properties: Option<&CustomProperties>,
    ancestors: &[ElementRef<'_>],
    interaction: &ElementInteractionState,
    element_path: &ElementPath,
    inherited_svg: SvgCascadeState,
    paths: &mut Vec<SvgPathInstance>,
) {
    match element.tag.as_str() {
        "g" | "path" | "circle" => {}
        "svg" => panic!("nested <svg> elements are not supported inside SVG subtrees"),
        _ => panic!("unsupported SVG tag <{}>", element.tag),
    }

    let style = crate::resolve_style_target(
        element,
        stylesheet,
        seed_element_style(element),
        inherited_text,
        Some(inherited_foreground),
        inherited_custom_properties,
        ancestors,
        interaction,
        element_path,
        None,
    );
    let svg_state = svg_cascade_with_transform(
        svg_cascade_from_style(inherited_svg, &style.visual.svg),
        element,
    );

    match element.tag.as_str() {
        "g" => {
            let mut child_ancestors = Vec::with_capacity(ancestors.len() + 1);
            child_ancestors.push(ElementRef::from(element));
            child_ancestors.extend_from_slice(ancestors);

            let mut child_index = 0;
            for child in &element.children {
                match child {
                    Node::Text(text) if text.trim().is_empty() => {}
                    Node::Text(_) => panic!("text nodes are not supported inside <g>"),
                    Node::Element(child) => {
                        let child_path = element_path.with_child(child_index);
                        child_index += 1;
                        collect_svg_paths(
                            child,
                            stylesheet,
                            Some(&style.visual.text),
                            style.visual.foreground,
                            Some(&style.custom_properties),
                            &child_ancestors,
                            interaction,
                            &child_path,
                            svg_state,
                            paths,
                        );
                    }
                }
            }
        }
        "path" => {
            for child in &element.children {
                match child {
                    Node::Text(text) if text.trim().is_empty() => {}
                    _ => panic!("<path> does not support child nodes"),
                }
            }

            let data = element
                .attribute("d")
                .unwrap_or_else(|| panic!("supported <path> elements require a `d` attribute"));
            let geometry = parse_svg_path_data(data)
                .unwrap_or_else(|error| panic!("unsupported SVG path data on <path>: {error}"));
            paths.push(SvgPathInstance {
                geometry: translate_svg_geometry(
                    geometry,
                    svg_state.translate_x,
                    svg_state.translate_y,
                ),
                paint: svg_path_paint(svg_state, style.visual.foreground),
            });
        }
        "circle" => {
            for child in &element.children {
                match child {
                    Node::Text(text) if text.trim().is_empty() => {}
                    _ => panic!("<circle> does not support child nodes"),
                }
            }

            let geometry = parse_svg_circle_geometry(element).unwrap_or_else(|error| {
                panic!("unsupported SVG circle geometry on <circle>: {error}")
            });
            let Some(geometry) = geometry else {
                return;
            };
            paths.push(SvgPathInstance {
                geometry: translate_svg_geometry(
                    geometry,
                    svg_state.translate_x,
                    svg_state.translate_y,
                ),
                paint: svg_path_paint(svg_state, style.visual.foreground),
            });
        }
        _ => unreachable!("unsupported SVG tags are rejected before matching"),
    }
}

fn svg_cascade_from_style(parent: SvgCascadeState, style: &SvgStyle) -> SvgCascadeState {
    SvgCascadeState {
        fill: style.fill.unwrap_or(parent.fill),
        stroke: style.stroke.unwrap_or(parent.stroke),
        stroke_width: style.stroke_width.unwrap_or(parent.stroke_width).max(0.0),
        translate_x: parent.translate_x,
        translate_y: parent.translate_y,
    }
}

fn svg_cascade_with_transform(
    mut state: SvgCascadeState,
    element: &ElementNode,
) -> SvgCascadeState {
    let Some(transform) = element.attribute("transform") else {
        return state;
    };

    let (translate_x, translate_y) =
        parse_svg_translate_transform(transform).unwrap_or_else(|error| {
            panic!(
                "unsupported SVG transform attribute on <{}>: {}",
                element.tag, error
            )
        });
    state.translate_x += translate_x;
    state.translate_y += translate_y;
    state
}

fn parse_svg_translate_transform(value: &str) -> Result<(f32, f32), String> {
    let mut rest = value.trim();
    let mut translate_x = 0.0;
    let mut translate_y = 0.0;

    while !rest.is_empty() {
        let Some(arguments_start) = rest.find('(') else {
            return Err(format!("expected transform function in `{value}`"));
        };
        let function = rest[..arguments_start].trim();
        let remaining = &rest[arguments_start + 1..];
        let Some(arguments_end) = remaining.find(')') else {
            return Err(format!("missing `)` in `{value}`"));
        };
        let arguments = &remaining[..arguments_end];
        let numbers = arguments
            .replace(',', " ")
            .split_whitespace()
            .map(|part| {
                part.parse::<f32>()
                    .map_err(|_| format!("invalid transform component `{part}`"))
            })
            .collect::<Result<Vec<_>, _>>()?;

        match function {
            "translate" => match numbers.as_slice() {
                [x] => {
                    translate_x += *x;
                }
                [x, y] => {
                    translate_x += *x;
                    translate_y += *y;
                }
                _ => {
                    return Err(format!(
                        "`translate()` expects one or two numeric arguments, got `{arguments}`"
                    ));
                }
            },
            _ => {
                return Err(format!(
                    "only translate(...) is supported, got `{function}`"
                ));
            }
        }

        rest = remaining[arguments_end + 1..].trim_start();
    }

    Ok((translate_x, translate_y))
}

fn parse_svg_circle_geometry(element: &ElementNode) -> Result<Option<SvgPathGeometry>, String> {
    let center_x = parse_svg_shape_length_attribute(element, "cx")?.unwrap_or(0.0);
    let center_y = parse_svg_shape_length_attribute(element, "cy")?.unwrap_or(0.0);
    let radius = parse_svg_shape_length_attribute(element, "r")?.unwrap_or(0.0);
    if radius <= f32::EPSILON {
        return Ok(None);
    }

    const CIRCLE_SEGMENTS: usize = 48;
    let mut points = Vec::with_capacity(CIRCLE_SEGMENTS);
    for segment in 0..CIRCLE_SEGMENTS {
        let angle = std::f32::consts::TAU * segment as f32 / CIRCLE_SEGMENTS as f32;
        points.push(SvgPoint::new(
            center_x + radius * angle.cos(),
            center_y + radius * angle.sin(),
        ));
    }

    Ok(Some(SvgPathGeometry::new(vec![SvgContour {
        points,
        closed: true,
    }])))
}

fn parse_svg_shape_length_attribute(
    element: &ElementNode,
    attribute_name: &str,
) -> Result<Option<f32>, String> {
    let Some(value) = element.attribute(attribute_name) else {
        return Ok(None);
    };

    numeric_svg_length_attribute(Some(value))
        .ok_or_else(|| {
            format!("`{attribute_name}` expects a numeric value or `px` length, got `{value}`")
        })
        .map(Some)
}

fn translate_svg_geometry(
    mut geometry: SvgPathGeometry,
    translate_x: f32,
    translate_y: f32,
) -> SvgPathGeometry {
    if translate_x.abs() <= f32::EPSILON && translate_y.abs() <= f32::EPSILON {
        return geometry;
    }

    for contour in &mut geometry.contours {
        for point in &mut contour.points {
            point.x += translate_x;
            point.y += translate_y;
        }
    }
    geometry.bounds = geometry.bounds.map(|bounds| cssimpler_core::SvgBounds {
        min_x: bounds.min_x + translate_x,
        min_y: bounds.min_y + translate_y,
        max_x: bounds.max_x + translate_x,
        max_y: bounds.max_y + translate_y,
    });
    geometry
}

fn svg_path_paint(style: SvgCascadeState, current_color: Color) -> SvgPathPaint {
    let stroke = if style.stroke_width <= f32::EPSILON {
        None
    } else {
        resolve_svg_paint(style.stroke, current_color)
    };

    SvgPathPaint {
        fill: resolve_svg_paint(style.fill, current_color),
        stroke,
        stroke_width: style.stroke_width.max(0.0),
    }
}

fn resolve_svg_paint(paint: SvgPaint, current_color: Color) -> Option<Color> {
    match paint {
        SvgPaint::Color(color) => Some(color),
        SvgPaint::CurrentColor => Some(current_color),
        SvgPaint::None => None,
    }
}

fn parse_svg_path_data(data: &str) -> Result<SvgPathGeometry, String> {
    let mut parser = PathDataParser::new(data);
    let mut builder = SvgPathBuilder::default();
    let mut active_command = None;

    while !parser.is_eof() {
        let command = if let Some(command) = parser.consume_command() {
            active_command = Some(command);
            command
        } else {
            active_command.ok_or_else(|| "path data must start with a command".to_string())?
        };

        match command {
            'M' => {
                builder.move_to(parser.parse_absolute_point()?)?;
                while parser.has_number() {
                    builder.line_to(parser.parse_absolute_point()?)?;
                }
                active_command = Some('L');
            }
            'm' => {
                let current = builder.current_point_or_origin();
                builder.move_to(parser.parse_relative_point(current)?)?;
                while parser.has_number() {
                    builder.line_to(parser.parse_relative_point(builder.current_point()?)?)?;
                }
                active_command = Some('l');
            }
            'L' => parse_repeated_command(&mut parser, "L", |parser| {
                builder.line_to(parser.parse_absolute_point()?)
            })?,
            'l' => parse_repeated_command(&mut parser, "l", |parser| {
                builder.line_to(parser.parse_relative_point(builder.current_point()?)?)
            })?,
            'H' => parse_repeated_command(&mut parser, "H", |parser| {
                builder.horizontal_to(parser.parse_number()?)
            })?,
            'h' => parse_repeated_command(&mut parser, "h", |parser| {
                builder.horizontal_to_relative(parser.parse_number()?)
            })?,
            'V' => parse_repeated_command(&mut parser, "V", |parser| {
                builder.vertical_to(parser.parse_number()?)
            })?,
            'v' => parse_repeated_command(&mut parser, "v", |parser| {
                builder.vertical_to_relative(parser.parse_number()?)
            })?,
            'C' => parse_repeated_command(&mut parser, "C", |parser| {
                builder.cubic_to(
                    parser.parse_absolute_point()?,
                    parser.parse_absolute_point()?,
                    parser.parse_absolute_point()?,
                )
            })?,
            'c' => parse_repeated_command(&mut parser, "c", |parser| {
                let current = builder.current_point()?;
                let control_1 = parser.parse_relative_point(current)?;
                let current = builder.current_point()?;
                let control_2 = parser.parse_relative_point(current)?;
                let current = builder.current_point()?;
                let end = parser.parse_relative_point(current)?;
                builder.cubic_to(control_1, control_2, end)
            })?,
            'S' => parse_repeated_command(&mut parser, "S", |parser| {
                let current = builder.current_point()?;
                let control_1 = reflect_control_point(current, builder.last_cubic_control());
                builder.cubic_to(
                    control_1,
                    parser.parse_absolute_point()?,
                    parser.parse_absolute_point()?,
                )
            })?,
            's' => parse_repeated_command(&mut parser, "s", |parser| {
                let current = builder.current_point()?;
                let control_1 = reflect_control_point(current, builder.last_cubic_control());
                let current = builder.current_point()?;
                let control_2 = parser.parse_relative_point(current)?;
                let current = builder.current_point()?;
                let end = parser.parse_relative_point(current)?;
                builder.cubic_to(control_1, control_2, end)
            })?,
            'Q' => parse_repeated_command(&mut parser, "Q", |parser| {
                builder.quadratic_to(
                    parser.parse_absolute_point()?,
                    parser.parse_absolute_point()?,
                )
            })?,
            'q' => parse_repeated_command(&mut parser, "q", |parser| {
                let current = builder.current_point()?;
                let control = parser.parse_relative_point(current)?;
                let current = builder.current_point()?;
                let end = parser.parse_relative_point(current)?;
                builder.quadratic_to(control, end)
            })?,
            'T' => parse_repeated_command(&mut parser, "T", |parser| {
                let current = builder.current_point()?;
                let control = reflect_control_point(current, builder.last_quadratic_control());
                builder.quadratic_to(control, parser.parse_absolute_point()?)
            })?,
            't' => parse_repeated_command(&mut parser, "t", |parser| {
                let current = builder.current_point()?;
                let control = reflect_control_point(current, builder.last_quadratic_control());
                let current = builder.current_point()?;
                let end = parser.parse_relative_point(current)?;
                builder.quadratic_to(control, end)
            })?,
            'A' => parse_repeated_command(&mut parser, "A", |parser| {
                builder.arc_to(
                    parser.parse_number()?,
                    parser.parse_number()?,
                    parser.parse_number()?,
                    parser.parse_arc_flag()?,
                    parser.parse_arc_flag()?,
                    parser.parse_absolute_point()?,
                )
            })?,
            'a' => parse_repeated_command(&mut parser, "a", |parser| {
                let radius_x = parser.parse_number()?;
                let radius_y = parser.parse_number()?;
                let x_axis_rotation = parser.parse_number()?;
                let large_arc = parser.parse_arc_flag()?;
                let sweep = parser.parse_arc_flag()?;
                let current = builder.current_point()?;
                let end = parser.parse_relative_point(current)?;
                builder.arc_to(radius_x, radius_y, x_axis_rotation, large_arc, sweep, end)
            })?,
            'Z' | 'z' => {
                builder.close_path()?;
                active_command = None;
            }
            _ => return Err(format!("unsupported SVG path command `{command}`")),
        }
    }

    Ok(builder.finish())
}

fn parse_repeated_command(
    parser: &mut PathDataParser<'_>,
    name: &str,
    mut parse_item: impl FnMut(&mut PathDataParser<'_>) -> Result<(), String>,
) -> Result<(), String> {
    let mut parsed_any = false;
    while parser.has_number() {
        parsed_any = true;
        parse_item(parser)?;
    }
    if !parsed_any {
        return Err(format!("`{name}` requires coordinate data"));
    }
    Ok(())
}

fn reflect_control_point(current: SvgPoint, previous: Option<SvgPoint>) -> SvgPoint {
    previous.map_or(current, |previous| {
        SvgPoint::new(2.0 * current.x - previous.x, 2.0 * current.y - previous.y)
    })
}

#[derive(Default)]
struct SvgPathBuilder {
    contours: Vec<SvgContour>,
    current_points: Vec<SvgPoint>,
    current_point: Option<SvgPoint>,
    subpath_start: Option<SvgPoint>,
    last_cubic_control: Option<SvgPoint>,
    last_quadratic_control: Option<SvgPoint>,
}

impl SvgPathBuilder {
    fn current_point_or_origin(&self) -> SvgPoint {
        self.current_point.unwrap_or(SvgPoint::new(0.0, 0.0))
    }

    fn current_point(&self) -> Result<SvgPoint, String> {
        self.current_point
            .ok_or_else(|| "path data references a point before the first move command".to_string())
    }

    fn last_cubic_control(&self) -> Option<SvgPoint> {
        self.last_cubic_control
    }

    fn last_quadratic_control(&self) -> Option<SvgPoint> {
        self.last_quadratic_control
    }

    fn move_to(&mut self, point: SvgPoint) -> Result<(), String> {
        self.flush_open_contour();
        self.current_points.push(point);
        self.current_point = Some(point);
        self.subpath_start = Some(point);
        self.last_cubic_control = None;
        self.last_quadratic_control = None;
        Ok(())
    }

    fn line_to(&mut self, point: SvgPoint) -> Result<(), String> {
        self.ensure_segment_start()?;
        self.current_points.push(point);
        self.current_point = Some(point);
        self.last_cubic_control = None;
        self.last_quadratic_control = None;
        Ok(())
    }

    fn horizontal_to(&mut self, x: f32) -> Result<(), String> {
        let current = self.current_point()?;
        self.line_to(SvgPoint::new(x, current.y))
    }

    fn horizontal_to_relative(&mut self, dx: f32) -> Result<(), String> {
        let current = self.current_point()?;
        self.line_to(SvgPoint::new(current.x + dx, current.y))
    }

    fn vertical_to(&mut self, y: f32) -> Result<(), String> {
        let current = self.current_point()?;
        self.line_to(SvgPoint::new(current.x, y))
    }

    fn vertical_to_relative(&mut self, dy: f32) -> Result<(), String> {
        let current = self.current_point()?;
        self.line_to(SvgPoint::new(current.x, current.y + dy))
    }

    fn cubic_to(
        &mut self,
        control_1: SvgPoint,
        control_2: SvgPoint,
        end: SvgPoint,
    ) -> Result<(), String> {
        let start = self.ensure_segment_start()?;
        flatten_cubic(
            start,
            control_1,
            control_2,
            end,
            &mut self.current_points,
            0,
        );
        self.current_point = Some(end);
        self.last_cubic_control = Some(control_2);
        self.last_quadratic_control = None;
        Ok(())
    }

    fn quadratic_to(&mut self, control: SvgPoint, end: SvgPoint) -> Result<(), String> {
        let start = self.ensure_segment_start()?;
        flatten_quadratic(start, control, end, &mut self.current_points, 0);
        self.current_point = Some(end);
        self.last_cubic_control = None;
        self.last_quadratic_control = Some(control);
        Ok(())
    }

    fn arc_to(
        &mut self,
        radius_x: f32,
        radius_y: f32,
        x_axis_rotation_degrees: f32,
        large_arc: bool,
        sweep: bool,
        end: SvgPoint,
    ) -> Result<(), String> {
        let start = self.ensure_segment_start()?;
        flatten_arc(
            start,
            radius_x,
            radius_y,
            x_axis_rotation_degrees,
            large_arc,
            sweep,
            end,
            &mut self.current_points,
        );
        self.current_point = Some(end);
        self.last_cubic_control = None;
        self.last_quadratic_control = None;
        Ok(())
    }

    fn close_path(&mut self) -> Result<(), String> {
        let start = self
            .subpath_start
            .ok_or_else(|| "close-path used before the first move command".to_string())?;
        if self.current_points.is_empty() {
            self.current_points.push(start);
        }
        if self.current_points.len() > 1 {
            self.contours.push(SvgContour {
                points: std::mem::take(&mut self.current_points),
                closed: true,
            });
        } else {
            self.current_points.clear();
        }
        self.current_point = Some(start);
        self.last_cubic_control = None;
        self.last_quadratic_control = None;
        Ok(())
    }

    fn finish(mut self) -> SvgPathGeometry {
        self.flush_open_contour();
        SvgPathGeometry::new(self.contours)
    }

    fn ensure_segment_start(&mut self) -> Result<SvgPoint, String> {
        let current = self.current_point()?;
        if self.current_points.is_empty() {
            self.current_points.push(current);
        }
        Ok(current)
    }

    fn flush_open_contour(&mut self) {
        if self.current_points.len() > 1 {
            self.contours.push(SvgContour {
                points: std::mem::take(&mut self.current_points),
                closed: false,
            });
        } else {
            self.current_points.clear();
        }
    }
}

struct PathDataParser<'a> {
    input: &'a str,
    index: usize,
}

impl<'a> PathDataParser<'a> {
    fn new(input: &'a str) -> Self {
        Self { input, index: 0 }
    }

    fn is_eof(&mut self) -> bool {
        self.skip_separators();
        self.index >= self.input.len()
    }

    fn consume_command(&mut self) -> Option<char> {
        self.skip_separators();
        let command = self.input[self.index..].chars().next()?;
        if !command.is_ascii_alphabetic() {
            return None;
        }
        self.index += command.len_utf8();
        Some(command)
    }

    fn has_number(&mut self) -> bool {
        self.skip_separators();
        self.input
            .as_bytes()
            .get(self.index)
            .is_some_and(|byte| matches!(byte, b'+' | b'-' | b'.' | b'0'..=b'9'))
    }

    fn parse_number(&mut self) -> Result<f32, String> {
        self.skip_separators();
        let bytes = self.input.as_bytes();
        let start = self.index;
        if self.index < bytes.len() && matches!(bytes[self.index], b'+' | b'-') {
            self.index += 1;
        }

        let mut digits = 0;
        while self.index < bytes.len() && bytes[self.index].is_ascii_digit() {
            self.index += 1;
            digits += 1;
        }

        if self.index < bytes.len() && bytes[self.index] == b'.' {
            self.index += 1;
            while self.index < bytes.len() && bytes[self.index].is_ascii_digit() {
                self.index += 1;
                digits += 1;
            }
        }

        if digits == 0 {
            return Err("expected a numeric SVG path component".to_string());
        }

        if self.index < bytes.len() && matches!(bytes[self.index], b'e' | b'E') {
            let exponent_start = self.index;
            self.index += 1;
            if self.index < bytes.len() && matches!(bytes[self.index], b'+' | b'-') {
                self.index += 1;
            }

            let exponent_digits_start = self.index;
            while self.index < bytes.len() && bytes[self.index].is_ascii_digit() {
                self.index += 1;
            }
            if self.index == exponent_digits_start {
                self.index = exponent_start;
            }
        }

        self.input[start..self.index]
            .parse::<f32>()
            .map_err(|_| "invalid numeric SVG path component".to_string())
    }

    fn parse_absolute_point(&mut self) -> Result<SvgPoint, String> {
        Ok(SvgPoint::new(self.parse_number()?, self.parse_number()?))
    }

    fn parse_relative_point(&mut self, current: SvgPoint) -> Result<SvgPoint, String> {
        Ok(SvgPoint::new(
            current.x + self.parse_number()?,
            current.y + self.parse_number()?,
        ))
    }

    fn parse_arc_flag(&mut self) -> Result<bool, String> {
        let value = self.parse_number()?;
        if (value - 0.0).abs() <= f32::EPSILON {
            Ok(false)
        } else if (value - 1.0).abs() <= f32::EPSILON {
            Ok(true)
        } else {
            Err(format!("arc flags must be 0 or 1, got `{value}`"))
        }
    }

    fn skip_separators(&mut self) {
        while let Some(byte) = self.input.as_bytes().get(self.index) {
            if byte.is_ascii_whitespace() || *byte == b',' {
                self.index += 1;
            } else {
                break;
            }
        }
    }
}

fn flatten_cubic(
    start: SvgPoint,
    control_1: SvgPoint,
    control_2: SvgPoint,
    end: SvgPoint,
    output: &mut Vec<SvgPoint>,
    depth: u8,
) {
    if depth >= 10
        || (point_to_line_distance_sq(control_1, start, end) <= 0.0625
            && point_to_line_distance_sq(control_2, start, end) <= 0.0625)
    {
        output.push(end);
        return;
    }

    let start_control = midpoint(start, control_1);
    let control_mid = midpoint(control_1, control_2);
    let control_end = midpoint(control_2, end);
    let left_control = midpoint(start_control, control_mid);
    let right_control = midpoint(control_mid, control_end);
    let split = midpoint(left_control, right_control);

    flatten_cubic(start, start_control, left_control, split, output, depth + 1);
    flatten_cubic(split, right_control, control_end, end, output, depth + 1);
}

fn flatten_quadratic(
    start: SvgPoint,
    control: SvgPoint,
    end: SvgPoint,
    output: &mut Vec<SvgPoint>,
    depth: u8,
) {
    if depth >= 10 || point_to_line_distance_sq(control, start, end) <= 0.0625 {
        output.push(end);
        return;
    }

    let start_control = midpoint(start, control);
    let control_end = midpoint(control, end);
    let split = midpoint(start_control, control_end);

    flatten_quadratic(start, start_control, split, output, depth + 1);
    flatten_quadratic(split, control_end, end, output, depth + 1);
}

fn flatten_arc(
    start: SvgPoint,
    radius_x: f32,
    radius_y: f32,
    x_axis_rotation_degrees: f32,
    large_arc: bool,
    sweep: bool,
    end: SvgPoint,
    output: &mut Vec<SvgPoint>,
) {
    let radius_x = radius_x.abs();
    let radius_y = radius_y.abs();
    if radius_x <= f32::EPSILON
        || radius_y <= f32::EPSILON
        || (start.x - end.x).abs() <= f32::EPSILON && (start.y - end.y).abs() <= f32::EPSILON
    {
        output.push(end);
        return;
    }

    let phi = x_axis_rotation_degrees.to_radians();
    let cos_phi = phi.cos();
    let sin_phi = phi.sin();
    let dx = (start.x - end.x) * 0.5;
    let dy = (start.y - end.y) * 0.5;
    let x1_prime = cos_phi * dx + sin_phi * dy;
    let y1_prime = -sin_phi * dx + cos_phi * dy;

    let mut radius_x_sq = radius_x * radius_x;
    let mut radius_y_sq = radius_y * radius_y;
    let x1_prime_sq = x1_prime * x1_prime;
    let y1_prime_sq = y1_prime * y1_prime;

    let radii_scale = x1_prime_sq / radius_x_sq + y1_prime_sq / radius_y_sq;
    let (radius_x, radius_y, radius_x_sq_scaled, radius_y_sq_scaled) = if radii_scale > 1.0 {
        let scale = radii_scale.sqrt();
        let radius_x = radius_x * scale;
        let radius_y = radius_y * scale;
        (radius_x, radius_y, radius_x * radius_x, radius_y * radius_y)
    } else {
        (radius_x, radius_y, radius_x_sq, radius_y_sq)
    };
    radius_x_sq = radius_x_sq_scaled;
    radius_y_sq = radius_y_sq_scaled;

    let numerator =
        radius_x_sq * radius_y_sq - radius_x_sq * y1_prime_sq - radius_y_sq * x1_prime_sq;
    let denominator = radius_x_sq * y1_prime_sq + radius_y_sq * x1_prime_sq;
    let center_scale = if denominator <= f32::EPSILON {
        0.0
    } else {
        let sign = if large_arc == sweep { -1.0 } else { 1.0 };
        sign * (numerator.max(0.0) / denominator).sqrt()
    };
    let cx_prime = center_scale * (radius_x * y1_prime / radius_y);
    let cy_prime = center_scale * (-radius_y * x1_prime / radius_x);

    let center_x = cos_phi * cx_prime - sin_phi * cy_prime + (start.x + end.x) * 0.5;
    let center_y = sin_phi * cx_prime + cos_phi * cy_prime + (start.y + end.y) * 0.5;

    let start_vector = SvgPoint::new(
        (x1_prime - cx_prime) / radius_x,
        (y1_prime - cy_prime) / radius_y,
    );
    let end_vector = SvgPoint::new(
        (-x1_prime - cx_prime) / radius_x,
        (-y1_prime - cy_prime) / radius_y,
    );
    let start_angle = vector_angle(SvgPoint::new(1.0, 0.0), start_vector);
    let mut delta_angle = vector_angle(start_vector, end_vector);
    if !sweep && delta_angle > 0.0 {
        delta_angle -= std::f32::consts::TAU;
    } else if sweep && delta_angle < 0.0 {
        delta_angle += std::f32::consts::TAU;
    }

    let segment_count =
        ((delta_angle.abs() / (std::f32::consts::PI / 12.0)).ceil() as usize).max(1);
    for step in 1..=segment_count {
        let t = start_angle + delta_angle * (step as f32 / segment_count as f32);
        let cos_t = t.cos();
        let sin_t = t.sin();
        output.push(SvgPoint::new(
            center_x + cos_phi * radius_x * cos_t - sin_phi * radius_y * sin_t,
            center_y + sin_phi * radius_x * cos_t + cos_phi * radius_y * sin_t,
        ));
    }
}

fn midpoint(left: SvgPoint, right: SvgPoint) -> SvgPoint {
    SvgPoint::new((left.x + right.x) * 0.5, (left.y + right.y) * 0.5)
}

fn vector_angle(from: SvgPoint, to: SvgPoint) -> f32 {
    let cross = from.x * to.y - from.y * to.x;
    let dot = from.x * to.x + from.y * to.y;
    cross.atan2(dot)
}

fn point_to_line_distance_sq(point: SvgPoint, start: SvgPoint, end: SvgPoint) -> f32 {
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    let length_sq = dx * dx + dy * dy;
    if length_sq <= f32::EPSILON {
        let px = point.x - start.x;
        let py = point.y - start.y;
        return px * px + py * py;
    }

    let numerator = dy * point.x - dx * point.y + end.x * start.y - end.y * start.x;
    (numerator * numerator) / length_sq
}

struct SvgRootMetadata {
    view_box: SvgViewBox,
}

#[cfg(test)]
mod tests {
    use cssimpler_core::{Color, Node, RenderKind};

    use crate::{Stylesheet, build_render_tree, parse_stylesheet};

    #[test]
    fn svg_render_tree_builds_a_vector_leaf_and_inherits_current_color() {
        let stylesheet = parse_stylesheet(".icon { color: #2563eb; fill: currentColor; }")
            .expect("stylesheet should parse");
        let tree = Node::element("svg")
            .with_class("icon")
            .with_attribute("viewBox", "0 0 24 24")
            .with_child(
                Node::element("path")
                    .with_attribute("d", "M2 2 L22 2 L22 22 L2 22 Z")
                    .into(),
            )
            .into();

        let scene = build_render_tree(&tree, &stylesheet);

        assert_eq!(scene.layout.width, 24.0);
        assert_eq!(scene.layout.height, 24.0);
        let RenderKind::Svg(svg) = scene.kind else {
            panic!("expected an SVG render node");
        };
        assert_eq!(svg.paths.len(), 1);
        assert_eq!(svg.paths[0].paint.fill, Some(Color::rgb(37, 99, 235)));
        assert_eq!(svg.paths[0].paint.stroke, None);
    }

    #[test]
    fn svg_selectors_can_style_path_strokes() {
        let stylesheet =
            parse_stylesheet(".accent { fill: none; stroke: #f97316; stroke-width: 2px; }")
                .expect("stylesheet should parse");
        let tree = Node::element("svg")
            .with_attribute("viewBox", "0 0 24 24")
            .with_child(
                Node::element("path")
                    .with_class("accent")
                    .with_attribute("d", "M4 12 L20 12")
                    .into(),
            )
            .into();

        let scene = build_render_tree(&tree, &stylesheet);
        let RenderKind::Svg(svg) = scene.kind else {
            panic!("expected an SVG render node");
        };
        assert_eq!(svg.paths.len(), 1);
        assert_eq!(svg.paths[0].paint.fill, None);
        assert_eq!(svg.paths[0].paint.stroke, Some(Color::rgb(249, 115, 22)));
        assert_eq!(svg.paths[0].paint.stroke_width, 2.0);
    }

    #[test]
    fn svg_allows_xmlns_and_path_translate_attributes() {
        let tree = Node::element("svg")
            .with_attribute("xmlns", "http://www.w3.org/2000/svg")
            .with_attribute("viewBox", "0 0 24 24")
            .with_child(
                Node::element("path")
                    .with_attribute("d", "M0 0 L4 0 L4 4 L0 4 Z")
                    .with_attribute("transform", "translate(10 6)")
                    .into(),
            )
            .into();

        let scene = build_render_tree(&tree, &Stylesheet::default());
        let RenderKind::Svg(svg) = scene.kind else {
            panic!("expected an SVG render node");
        };
        let bounds = svg.paths[0]
            .geometry
            .bounds
            .expect("translated geometry should keep bounds");
        assert_eq!(bounds.min_x, 10.0);
        assert_eq!(bounds.min_y, 6.0);
        assert_eq!(bounds.max_x, 14.0);
        assert_eq!(bounds.max_y, 10.0);
    }

    #[test]
    fn svg_circle_elements_render_as_closed_paths() {
        let tree = Node::element("svg")
            .with_attribute("viewBox", "0 0 24 24")
            .with_child(
                Node::element("circle")
                    .with_attribute("cx", "12")
                    .with_attribute("cy", "12")
                    .with_attribute("r", "6")
                    .into(),
            )
            .into();

        let scene = build_render_tree(&tree, &Stylesheet::default());
        let RenderKind::Svg(svg) = scene.kind else {
            panic!("expected an SVG render node");
        };
        assert_eq!(svg.paths.len(), 1);

        let contour = &svg.paths[0].geometry.contours[0];
        assert!(contour.closed);
        assert_eq!(contour.points.len(), 48);

        let bounds = svg.paths[0]
            .geometry
            .bounds
            .expect("circle geometry should have bounds");
        assert!((bounds.min_x - 6.0).abs() < 0.01);
        assert!((bounds.min_y - 6.0).abs() < 0.01);
        assert!((bounds.max_x - 18.0).abs() < 0.01);
        assert!((bounds.max_y - 18.0).abs() < 0.01);
    }

    #[test]
    fn unsupported_svg_transform_values_fail_clearly() {
        let tree = Node::element("svg")
            .with_attribute("viewBox", "0 0 24 24")
            .with_child(
                Node::element("path")
                    .with_attribute("d", "M4 12 L20 12")
                    .with_attribute("transform", "scale(2)")
                    .into(),
            )
            .into();

        let error = std::panic::catch_unwind(|| build_render_tree(&tree, &Stylesheet::default()))
            .expect_err("unsupported SVG attributes should panic");
        let message = if let Some(message) = error.downcast_ref::<String>() {
            message.clone()
        } else if let Some(message) = error.downcast_ref::<&str>() {
            (*message).to_string()
        } else {
            String::new()
        };

        assert!(message.contains("unsupported SVG transform attribute"));
    }

    #[test]
    fn svg_path_parser_supports_absolute_and_relative_arc_commands() {
        let tree = Node::element("svg")
            .with_attribute("viewBox", "0 0 24 24")
            .with_child(
                Node::element("path")
                    .with_attribute("d", "M4 12 A 4 4 0 0 0 12 12 a 4 4 0 0 0 8 0")
                    .into(),
            )
            .into();

        let scene = build_render_tree(&tree, &Stylesheet::default());
        let RenderKind::Svg(svg) = scene.kind else {
            panic!("expected an SVG render node");
        };
        let contour = &svg.paths[0].geometry.contours[0];
        assert!(contour.points.len() > 4);
        let bounds = svg.paths[0]
            .geometry
            .bounds
            .expect("arc geometry should have bounds");
        assert!(bounds.max_x >= 20.0);
    }
}
