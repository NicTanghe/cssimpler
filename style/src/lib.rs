use std::error::Error;
use std::fmt::{Display, Formatter};

use cssimpler_core::{
    BoxShadow as CoreBoxShadow, Color, ElementNode, EventHandler, LayoutBox, LayoutStyle, Node,
    RenderNode, Style,
};
use lightningcss::declaration::DeclarationBlock;
use lightningcss::properties::Property;
use lightningcss::properties::align::{
    AlignContent as CssAlignContent, AlignItems as CssAlignItems, AlignSelf as CssAlignSelf,
    ContentDistribution, ContentPosition, GapValue, JustifyContent as CssJustifyContent,
    SelfPosition,
};
use lightningcss::properties::background::Background;
use lightningcss::properties::border::{BorderSideWidth as CssBorderSideWidth, LineStyle as CssLineStyle};
use lightningcss::properties::display::{Display as CssDisplay, DisplayInside, DisplayKeyword};
use lightningcss::properties::flex::{
    FlexDirection as CssFlexDirection, FlexWrap as CssFlexWrap,
};
use lightningcss::properties::grid::{
    GridColumn as CssGridColumn, GridLine as CssGridLine, GridRow as CssGridRow, RepeatCount,
    TrackBreadth, TrackListItem, TrackSize, TrackSizing as CssTrackSizing,
};
use lightningcss::properties::overflow::OverflowKeyword as CssOverflowKeyword;
use lightningcss::properties::position::Position as CssPosition;
use lightningcss::properties::size::Size as CssSize;
use lightningcss::rules::CssRule;
use lightningcss::selector::{Component, Selector as LightningSelector};
use lightningcss::stylesheet::{ParserOptions, StyleSheet};
use lightningcss::values::color::CssColor;
use lightningcss::values::length::{LengthPercentage, LengthPercentageOrAuto};
use lightningcss::values::size::Size2D;
use taffy::Overflow as TaffyOverflow;
use taffy::geometry::{Line, Size as TaffySize};
use taffy::prelude::{
    AlignContent as TaffyAlignContent, AlignItems as TaffyAlignItems,
    AlignSelf as TaffyAlignSelf, AvailableSpace, Dimension, Display as TaffyDisplay,
    FlexDirection, FlexWrap, GridPlacement, GridTrackRepetition,
    JustifyContent as TaffyJustifyContent, LengthPercentage as TaffyLengthPercentage,
    LengthPercentageAuto as TaffyLengthPercentageAuto, MaxTrackSizingFunction,
    MinTrackSizingFunction, NodeId, NonRepeatedTrackSizingFunction, Position as TaffyPosition,
    Style as TaffyStyle, TaffyGridLine, TaffyGridSpan, TaffyTree, TrackSizingFunction,
};

const GLYPH_WIDTH: f32 = 18.0;
const GLYPH_HEIGHT: f32 = 20.0;

#[derive(Clone, Debug, PartialEq)]
pub struct ShadowDeclaration {
    color: Option<Color>,
    offset_x: f32,
    offset_y: f32,
    blur_radius: f32,
    spread: f32,
}

#[derive(Clone, Debug, Default)]
pub struct Stylesheet {
    pub rules: Vec<StyleRule>,
}

impl Stylesheet {
    pub fn push(&mut self, rule: StyleRule) {
        self.rules.push(rule);
    }

    pub fn matching_rules<'a>(
        &'a self,
        element: ElementRef<'a>,
    ) -> impl Iterator<Item = &'a StyleRule> {
        self.rules
            .iter()
            .filter(move |rule| rule.selector.matches(element))
    }
}

#[derive(Clone, Debug)]
pub struct StyleRule {
    pub selector: Selector,
    pub declarations: Vec<Declaration>,
}

impl StyleRule {
    pub fn new(selector: Selector, declarations: Vec<Declaration>) -> Self {
        Self {
            selector,
            declarations,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Declaration {
    Background(Color),
    Foreground(Color),
    CornerTopLeft(f32),
    CornerTopRight(f32),
    CornerBottomRight(f32),
    CornerBottomLeft(f32),
    BorderTopWidth(f32),
    BorderRightWidth(f32),
    BorderBottomWidth(f32),
    BorderLeftWidth(f32),
    BorderColor(Option<Color>),
    BoxShadows(Vec<ShadowDeclaration>),
    OverflowX(TaffyOverflow, bool),
    OverflowY(TaffyOverflow, bool),
    Display(TaffyDisplay),
    Position(TaffyPosition),
    InsetTop(TaffyLengthPercentageAuto),
    InsetRight(TaffyLengthPercentageAuto),
    InsetBottom(TaffyLengthPercentageAuto),
    InsetLeft(TaffyLengthPercentageAuto),
    Width(Dimension),
    Height(Dimension),
    MarginTop(TaffyLengthPercentageAuto),
    MarginRight(TaffyLengthPercentageAuto),
    MarginBottom(TaffyLengthPercentageAuto),
    MarginLeft(TaffyLengthPercentageAuto),
    PaddingTop(TaffyLengthPercentage),
    PaddingRight(TaffyLengthPercentage),
    PaddingBottom(TaffyLengthPercentage),
    PaddingLeft(TaffyLengthPercentage),
    FlexDirection(FlexDirection),
    FlexWrap(FlexWrap),
    JustifyContent(Option<TaffyJustifyContent>),
    AlignItems(Option<TaffyAlignItems>),
    AlignSelf(Option<TaffyAlignSelf>),
    AlignContent(Option<TaffyAlignContent>),
    GapRow(TaffyLengthPercentage),
    GapColumn(TaffyLengthPercentage),
    FlexGrow(f32),
    FlexShrink(f32),
    FlexBasis(Dimension),
    GridTemplateColumns(Vec<TrackSizingFunction>),
    GridTemplateRows(Vec<TrackSizingFunction>),
    GridColumn(Line<GridPlacement>),
    GridRow(Line<GridPlacement>),
    GridColumnStart(GridPlacement),
    GridColumnEnd(GridPlacement),
    GridRowStart(GridPlacement),
    GridRowEnd(GridPlacement),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selector {
    Class(String),
    Id(String),
    Tag(String),
}

impl Selector {
    pub fn matches(&self, element: ElementRef<'_>) -> bool {
        match self {
            Self::Class(expected) => element
                .classes
                .iter()
                .any(|class_name| class_name == expected),
            Self::Id(expected) => element.id.is_some_and(|actual| actual == expected),
            Self::Tag(expected) => element.tag == expected,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ElementRef<'a> {
    pub tag: &'a str,
    pub id: Option<&'a str>,
    pub classes: &'a [String],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StyleError {
    Parse(String),
    UnsupportedRule(String),
    UnsupportedSelector(String),
    UnsupportedValue(String),
}

impl Display for StyleError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(error) => write!(f, "failed to parse stylesheet: {error}"),
            Self::UnsupportedRule(rule) => write!(f, "unsupported rule: {rule}"),
            Self::UnsupportedSelector(selector) => write!(f, "unsupported selector: {selector}"),
            Self::UnsupportedValue(value) => write!(f, "unsupported value: {value}"),
        }
    }
}

impl Error for StyleError {}

#[derive(Clone, Debug)]
struct ResolvedElement {
    style: Style,
    text: String,
    on_click: Option<EventHandler>,
    children: Vec<ResolvedElement>,
}

#[derive(Clone, Debug)]
struct LayoutTree {
    node_id: NodeId,
    style: Style,
    text: String,
    on_click: Option<EventHandler>,
    children: Vec<LayoutTree>,
}

#[derive(Clone, Debug, Default)]
struct LeafMeasureContext {
    text: String,
}

pub fn parse_stylesheet(source: &str) -> Result<Stylesheet, StyleError> {
    let parsed = StyleSheet::parse(source, ParserOptions::default())
        .map_err(|error| StyleError::Parse(error.to_string()))?;
    let mut stylesheet = Stylesheet::default();

    for rule in &parsed.rules.0 {
        let CssRule::Style(style_rule) = rule else {
            return Err(StyleError::UnsupportedRule(
                "only top-level style rules are supported".to_string(),
            ));
        };

        if !style_rule.rules.0.is_empty() {
            return Err(StyleError::UnsupportedRule(
                "nested style rules are not supported".to_string(),
            ));
        }

        let declarations = extract_declarations(&style_rule.declarations)?;
        for selector in style_rule
            .selectors
            .0
            .iter()
            .map(extract_selector)
            .collect::<Result<Vec<_>, _>>()?
        {
            stylesheet.push(StyleRule::new(selector, declarations.clone()));
        }
    }

    Ok(stylesheet)
}

pub fn resolve_style(element: &ElementNode, stylesheet: &Stylesheet) -> Style {
    let mut resolved = element.style.clone();
    let mut position_explicit = resolved.layout.taffy.position != TaffyPosition::Relative;
    let element_ref = ElementRef {
        tag: &element.tag,
        id: element.id.as_deref(),
        classes: &element.classes,
    };

    for rule in stylesheet.matching_rules(element_ref) {
        for declaration in &rule.declarations {
            apply_declaration(&mut resolved, &mut position_explicit, declaration);
        }
    }

    resolved
}

pub fn to_taffy(style: &LayoutStyle) -> TaffyStyle {
    style.taffy.clone()
}

pub fn build_render_tree(root: &Node, stylesheet: &Stylesheet) -> RenderNode {
    let Node::Element(root_element) = root else {
        panic!("render tree roots must be elements");
    };

    let resolved = resolve_element_tree(root_element, stylesheet);
    let mut taffy = TaffyTree::<LeafMeasureContext>::new();
    let layout_tree = build_layout_tree(&resolved, &mut taffy);
    let available_space = available_space_from_root(&layout_tree.style.layout.taffy);
    taffy
        .compute_layout_with_measure(
            layout_tree.node_id,
            available_space,
            |known_dimensions, available_space, _, context, _| {
                context.map_or(
                    TaffySize {
                        width: 0.0,
                        height: 0.0,
                    },
                    |context| measure_text(&context.text, known_dimensions, available_space),
                )
            },
        )
        .expect("resolved layout should be valid for taffy");

    render_node_from_layout(&layout_tree, &taffy, 0.0, 0.0)
}

fn extract_selector(selector: &LightningSelector<'_>) -> Result<Selector, StyleError> {
    let mut resolved = None;

    for component in selector.iter_raw_match_order() {
        let candidate = match component {
            Component::Class(name) => Selector::Class(name.0.to_string()),
            Component::ID(name) => Selector::Id(name.0.to_string()),
            Component::LocalName(name) => Selector::Tag(name.name.0.to_string()),
            Component::ExplicitUniversalType => continue,
            _ => {
                return Err(StyleError::UnsupportedSelector(format!("{selector:?}")));
            }
        };

        if resolved.replace(candidate).is_some() {
            return Err(StyleError::UnsupportedSelector(format!("{selector:?}")));
        }
    }

    resolved.ok_or_else(|| StyleError::UnsupportedSelector(format!("{selector:?}")))
}

fn extract_declarations(block: &DeclarationBlock<'_>) -> Result<Vec<Declaration>, StyleError> {
    let mut declarations = Vec::new();

    for (property, _important) in block.iter() {
        declarations.extend(extract_property(property)?);
    }

    Ok(declarations)
}

fn extract_property(property: &Property<'_>) -> Result<Vec<Declaration>, StyleError> {
    match property {
        Property::BackgroundColor(color) => Ok(vec![Declaration::Background(color_from_css(color)?)]),
        Property::Background(backgrounds) => Ok(
            extract_background_declaration(backgrounds.first())?
                .into_iter()
                .collect(),
        ),
        Property::Color(color) => Ok(vec![Declaration::Foreground(color_from_css(color)?)]),
        Property::BorderRadius(radius, _) => extract_corner_radius_declarations(radius),
        Property::BorderTopLeftRadius(radius, _) => {
            Ok(vec![Declaration::CornerTopLeft(corner_radius_to_px(radius)?)])
        }
        Property::BorderTopRightRadius(radius, _) => {
            Ok(vec![Declaration::CornerTopRight(corner_radius_to_px(radius)?)])
        }
        Property::BorderBottomRightRadius(radius, _) => {
            Ok(vec![Declaration::CornerBottomRight(corner_radius_to_px(radius)?)])
        }
        Property::BorderBottomLeftRadius(radius, _) => {
            Ok(vec![Declaration::CornerBottomLeft(corner_radius_to_px(radius)?)])
        }
        Property::Border(border) => extract_border_shorthand_declarations(border),
        Property::BorderTop(border) => extract_border_side_declarations(border, BorderSide::Top),
        Property::BorderRight(border) => {
            extract_border_side_declarations(border, BorderSide::Right)
        }
        Property::BorderBottom(border) => {
            extract_border_side_declarations(border, BorderSide::Bottom)
        }
        Property::BorderLeft(border) => extract_border_side_declarations(border, BorderSide::Left),
        Property::BorderWidth(widths) => extract_border_width_declarations(widths),
        Property::BorderTopWidth(value) => {
            Ok(vec![Declaration::BorderTopWidth(border_width_to_px(value)?)])
        }
        Property::BorderRightWidth(value) => {
            Ok(vec![Declaration::BorderRightWidth(border_width_to_px(value)?)])
        }
        Property::BorderBottomWidth(value) => {
            Ok(vec![Declaration::BorderBottomWidth(border_width_to_px(value)?)])
        }
        Property::BorderLeftWidth(value) => {
            Ok(vec![Declaration::BorderLeftWidth(border_width_to_px(value)?)])
        }
        Property::BorderColor(colors) => Ok(vec![Declaration::BorderColor(
            color_from_css_optional(&colors.top)?,
        )]),
        Property::BorderTopColor(color) => {
            Ok(vec![Declaration::BorderColor(color_from_css_optional(color)?)])
        }
        Property::BorderRightColor(color) => {
            Ok(vec![Declaration::BorderColor(color_from_css_optional(color)?)])
        }
        Property::BorderBottomColor(color) => {
            Ok(vec![Declaration::BorderColor(color_from_css_optional(color)?)])
        }
        Property::BorderLeftColor(color) => {
            Ok(vec![Declaration::BorderColor(color_from_css_optional(color)?)])
        }
        Property::BoxShadow(shadows, _) => {
            Ok(vec![Declaration::BoxShadows(box_shadow_declarations(shadows.as_slice())?)])
        }
        Property::Overflow(overflow) => Ok(vec![
            overflow_x_declaration(overflow.x),
            overflow_y_declaration(overflow.y),
        ]),
        Property::OverflowX(value) => Ok(vec![overflow_x_declaration(*value)]),
        Property::OverflowY(value) => Ok(vec![overflow_y_declaration(*value)]),
        Property::Display(display) => Ok(vec![Declaration::Display(display_from_css(display)?)]),
        Property::Position(position) => {
            Ok(vec![Declaration::Position(position_from_css(position))])
        }
        Property::Top(value) => Ok(vec![Declaration::InsetTop(length_percentage_auto_to_taffy(
            value,
        )?)]),
        Property::Right(value) => Ok(vec![Declaration::InsetRight(
            length_percentage_auto_to_taffy(value)?,
        )]),
        Property::Bottom(value) => Ok(vec![Declaration::InsetBottom(
            length_percentage_auto_to_taffy(value)?,
        )]),
        Property::Left(value) => Ok(vec![Declaration::InsetLeft(length_percentage_auto_to_taffy(
            value,
        )?)]),
        Property::Width(size) => Ok(vec![Declaration::Width(dimension_from_css_size(size)?)]),
        Property::Height(size) => Ok(vec![Declaration::Height(dimension_from_css_size(size)?)]),
        Property::Margin(margin) => Ok(vec![
            Declaration::MarginTop(length_percentage_auto_to_taffy(&margin.top)?),
            Declaration::MarginRight(length_percentage_auto_to_taffy(&margin.right)?),
            Declaration::MarginBottom(length_percentage_auto_to_taffy(&margin.bottom)?),
            Declaration::MarginLeft(length_percentage_auto_to_taffy(&margin.left)?),
        ]),
        Property::MarginTop(value) => Ok(vec![Declaration::MarginTop(
            length_percentage_auto_to_taffy(value)?,
        )]),
        Property::MarginRight(value) => Ok(vec![Declaration::MarginRight(
            length_percentage_auto_to_taffy(value)?,
        )]),
        Property::MarginBottom(value) => Ok(vec![Declaration::MarginBottom(
            length_percentage_auto_to_taffy(value)?,
        )]),
        Property::MarginLeft(value) => Ok(vec![Declaration::MarginLeft(
            length_percentage_auto_to_taffy(value)?,
        )]),
        Property::Padding(padding) => Ok(vec![
            Declaration::PaddingTop(length_percentage_auto_to_taffy_padding(&padding.top)?),
            Declaration::PaddingRight(length_percentage_auto_to_taffy_padding(&padding.right)?),
            Declaration::PaddingBottom(length_percentage_auto_to_taffy_padding(&padding.bottom)?),
            Declaration::PaddingLeft(length_percentage_auto_to_taffy_padding(&padding.left)?),
        ]),
        Property::PaddingTop(value) => Ok(vec![Declaration::PaddingTop(
            length_percentage_auto_to_taffy_padding(value)?,
        )]),
        Property::PaddingRight(value) => Ok(vec![Declaration::PaddingRight(
            length_percentage_auto_to_taffy_padding(value)?,
        )]),
        Property::PaddingBottom(value) => Ok(vec![Declaration::PaddingBottom(
            length_percentage_auto_to_taffy_padding(value)?,
        )]),
        Property::PaddingLeft(value) => Ok(vec![Declaration::PaddingLeft(
            length_percentage_auto_to_taffy_padding(value)?,
        )]),
        Property::FlexDirection(direction, _) => {
            Ok(vec![Declaration::FlexDirection(flex_direction_from_css(direction))])
        }
        Property::FlexWrap(wrap, _) => Ok(vec![Declaration::FlexWrap(flex_wrap_from_css(wrap))]),
        Property::JustifyContent(content, _) => Ok(vec![Declaration::JustifyContent(
            justify_content_from_css(content)?,
        )]),
        Property::AlignItems(items, _) => Ok(vec![Declaration::AlignItems(
            align_items_from_css(items)?,
        )]),
        Property::AlignSelf(self_alignment, _) => Ok(vec![Declaration::AlignSelf(
            align_self_from_css(self_alignment)?,
        )]),
        Property::AlignContent(content, _) => Ok(vec![Declaration::AlignContent(
            align_content_from_css(content)?,
        )]),
        Property::RowGap(value) => Ok(vec![Declaration::GapRow(gap_value_to_taffy(value)?)]),
        Property::ColumnGap(value) => {
            Ok(vec![Declaration::GapColumn(gap_value_to_taffy(value)?)])
        }
        Property::Gap(gap) => Ok(vec![
            Declaration::GapRow(gap_value_to_taffy(&gap.row)?),
            Declaration::GapColumn(gap_value_to_taffy(&gap.column)?),
        ]),
        Property::FlexGrow(value, _) => Ok(vec![Declaration::FlexGrow(*value)]),
        Property::FlexShrink(value, _) => Ok(vec![Declaration::FlexShrink(*value)]),
        Property::FlexBasis(value, _) => Ok(vec![Declaration::FlexBasis(
            dimension_from_length_percentage_auto(value)?,
        )]),
        Property::GridTemplateColumns(track_sizing) => Ok(vec![Declaration::GridTemplateColumns(
            track_sizing_from_css(track_sizing)?,
        )]),
        Property::GridTemplateRows(track_sizing) => Ok(vec![Declaration::GridTemplateRows(
            track_sizing_from_css(track_sizing)?,
        )]),
        Property::GridColumn(column) => Ok(vec![Declaration::GridColumn(
            grid_column_from_css(column)?,
        )]),
        Property::GridRow(row) => Ok(vec![Declaration::GridRow(grid_row_from_css(row)?)]),
        Property::GridColumnStart(start) => Ok(vec![Declaration::GridColumnStart(
            grid_placement_from_css(start)?,
        )]),
        Property::GridColumnEnd(end) => Ok(vec![Declaration::GridColumnEnd(
            grid_placement_from_css(end)?,
        )]),
        Property::GridRowStart(start) => Ok(vec![Declaration::GridRowStart(
            grid_placement_from_css(start)?,
        )]),
        Property::GridRowEnd(end) => Ok(vec![Declaration::GridRowEnd(
            grid_placement_from_css(end)?,
        )]),
        _ => Ok(Vec::new()),
    }
}

fn extract_background_declaration(
    background: Option<&Background<'_>>,
) -> Result<Option<Declaration>, StyleError> {
    let Some(background) = background else {
        return Ok(None);
    };

    Ok(Some(Declaration::Background(color_from_css(
        &background.color,
    )?)))
}

#[derive(Clone, Copy)]
enum BorderSide {
    Top,
    Right,
    Bottom,
    Left,
}

fn color_from_css(color: &CssColor) -> Result<Color, StyleError> {
    let rgb = color
        .to_rgb()
        .map_err(|_| StyleError::UnsupportedValue(format!("{color:?}")))?;

    match rgb {
        CssColor::RGBA(rgba) => Ok(Color::rgba(rgba.red, rgba.green, rgba.blue, rgba.alpha)),
        _ => Err(StyleError::UnsupportedValue(format!("{color:?}"))),
    }
}

fn color_from_css_optional(color: &CssColor) -> Result<Option<Color>, StyleError> {
    match color {
        CssColor::CurrentColor => Ok(None),
        _ => color_from_css(color).map(Some),
    }
}

fn extract_corner_radius_declarations(
    radius: &lightningcss::properties::border_radius::BorderRadius,
) -> Result<Vec<Declaration>, StyleError> {
    Ok(vec![
        Declaration::CornerTopLeft(corner_radius_to_px(&radius.top_left)?),
        Declaration::CornerTopRight(corner_radius_to_px(&radius.top_right)?),
        Declaration::CornerBottomRight(corner_radius_to_px(&radius.bottom_right)?),
        Declaration::CornerBottomLeft(corner_radius_to_px(&radius.bottom_left)?),
    ])
}

fn corner_radius_to_px(value: &Size2D<LengthPercentage>) -> Result<f32, StyleError> {
    length_percentage_to_px(&value.0)
}

fn length_percentage_to_px(value: &LengthPercentage) -> Result<f32, StyleError> {
    match value {
        LengthPercentage::Dimension(length) => length
            .to_px()
            .map(|value| value as f32)
            .ok_or_else(|| StyleError::UnsupportedValue(format!("{value:?}"))),
        _ => Err(StyleError::UnsupportedValue(format!("{value:?}"))),
    }
}

fn border_width_to_px(value: &CssBorderSideWidth) -> Result<f32, StyleError> {
    match value {
        CssBorderSideWidth::Thin => Ok(1.0),
        CssBorderSideWidth::Medium => Ok(3.0),
        CssBorderSideWidth::Thick => Ok(5.0),
        CssBorderSideWidth::Length(length) => length
            .to_px()
            .map(|value| value as f32)
            .ok_or_else(|| StyleError::UnsupportedValue(format!("{value:?}"))),
    }
}

fn extract_border_width_declarations(
    widths: &lightningcss::properties::border::BorderWidth,
) -> Result<Vec<Declaration>, StyleError> {
    Ok(vec![
        Declaration::BorderTopWidth(border_width_to_px(&widths.top)?),
        Declaration::BorderRightWidth(border_width_to_px(&widths.right)?),
        Declaration::BorderBottomWidth(border_width_to_px(&widths.bottom)?),
        Declaration::BorderLeftWidth(border_width_to_px(&widths.left)?),
    ])
}

fn extract_border_shorthand_declarations(
    border: &lightningcss::properties::border::Border,
) -> Result<Vec<Declaration>, StyleError> {
    if matches!(border.style, CssLineStyle::None | CssLineStyle::Hidden) {
        return Ok(vec![
            Declaration::BorderTopWidth(0.0),
            Declaration::BorderRightWidth(0.0),
            Declaration::BorderBottomWidth(0.0),
            Declaration::BorderLeftWidth(0.0),
        ]);
    }

    let width = border_width_to_px(&border.width)?;
    Ok(vec![
        Declaration::BorderTopWidth(width),
        Declaration::BorderRightWidth(width),
        Declaration::BorderBottomWidth(width),
        Declaration::BorderLeftWidth(width),
        Declaration::BorderColor(color_from_css_optional(&border.color)?),
    ])
}

fn extract_border_side_declarations<T>(
    border: &T,
    side: BorderSide,
) -> Result<Vec<Declaration>, StyleError>
where
    T: BorderSideAccess,
{
    if matches!(border.line_style(), CssLineStyle::None | CssLineStyle::Hidden) {
        return Ok(vec![border_width_declaration(side, 0.0)]);
    }

    Ok(vec![
        border_width_declaration(side, border_width_to_px(border.width())?),
        Declaration::BorderColor(color_from_css_optional(border.color())?),
    ])
}

trait BorderSideAccess {
    fn width(&self) -> &CssBorderSideWidth;
    fn line_style(&self) -> CssLineStyle;
    fn color(&self) -> &CssColor;
}

impl BorderSideAccess for lightningcss::properties::border::BorderTop {
    fn width(&self) -> &CssBorderSideWidth {
        &self.width
    }

    fn line_style(&self) -> CssLineStyle {
        self.style
    }

    fn color(&self) -> &CssColor {
        &self.color
    }
}

impl BorderSideAccess for lightningcss::properties::border::BorderRight {
    fn width(&self) -> &CssBorderSideWidth {
        &self.width
    }

    fn line_style(&self) -> CssLineStyle {
        self.style
    }

    fn color(&self) -> &CssColor {
        &self.color
    }
}

impl BorderSideAccess for lightningcss::properties::border::BorderBottom {
    fn width(&self) -> &CssBorderSideWidth {
        &self.width
    }

    fn line_style(&self) -> CssLineStyle {
        self.style
    }

    fn color(&self) -> &CssColor {
        &self.color
    }
}

impl BorderSideAccess for lightningcss::properties::border::BorderLeft {
    fn width(&self) -> &CssBorderSideWidth {
        &self.width
    }

    fn line_style(&self) -> CssLineStyle {
        self.style
    }

    fn color(&self) -> &CssColor {
        &self.color
    }
}

fn border_width_declaration(side: BorderSide, value: f32) -> Declaration {
    match side {
        BorderSide::Top => Declaration::BorderTopWidth(value),
        BorderSide::Right => Declaration::BorderRightWidth(value),
        BorderSide::Bottom => Declaration::BorderBottomWidth(value),
        BorderSide::Left => Declaration::BorderLeftWidth(value),
    }
}

fn box_shadow_declarations(
    shadows: &[lightningcss::properties::box_shadow::BoxShadow],
) -> Result<Vec<ShadowDeclaration>, StyleError> {
    shadows
        .iter()
        .filter(|shadow| !shadow.inset)
        .map(|shadow| {
            Ok(ShadowDeclaration {
                color: color_from_css_optional(&shadow.color)?,
                offset_x: length_to_px(&shadow.x_offset)?,
                offset_y: length_to_px(&shadow.y_offset)?,
                blur_radius: length_to_px(&shadow.blur)?,
                spread: length_to_px(&shadow.spread)?,
            })
        })
        .collect()
}

fn length_to_px(value: &lightningcss::values::length::Length) -> Result<f32, StyleError> {
    value
        .to_px()
        .map(|value| value as f32)
        .ok_or_else(|| StyleError::UnsupportedValue(format!("{value:?}")))
}

fn overflow_x_declaration(value: CssOverflowKeyword) -> Declaration {
    let (overflow, clip) = overflow_from_css_keyword(value);
    Declaration::OverflowX(overflow, clip)
}

fn overflow_y_declaration(value: CssOverflowKeyword) -> Declaration {
    let (overflow, clip) = overflow_from_css_keyword(value);
    Declaration::OverflowY(overflow, clip)
}

fn overflow_from_css_keyword(value: CssOverflowKeyword) -> (TaffyOverflow, bool) {
    match value {
        CssOverflowKeyword::Visible => (TaffyOverflow::Visible, false),
        CssOverflowKeyword::Clip => (TaffyOverflow::Clip, true),
        CssOverflowKeyword::Hidden => (TaffyOverflow::Hidden, true),
        CssOverflowKeyword::Scroll | CssOverflowKeyword::Auto => (TaffyOverflow::Scroll, true),
    }
}

fn display_from_css(display: &CssDisplay) -> Result<TaffyDisplay, StyleError> {
    match display {
        CssDisplay::Keyword(DisplayKeyword::None) => Ok(TaffyDisplay::None),
        CssDisplay::Keyword(keyword) => {
            Err(StyleError::UnsupportedValue(format!("{keyword:?}")))
        }
        CssDisplay::Pair(pair) => match &pair.inside {
            DisplayInside::Flex(_) | DisplayInside::Box(_) => Ok(TaffyDisplay::Flex),
            DisplayInside::Grid => Ok(TaffyDisplay::Grid),
            DisplayInside::Flow
            | DisplayInside::FlowRoot
            | DisplayInside::Table
            | DisplayInside::Ruby => Ok(TaffyDisplay::Block),
        },
    }
}

fn position_from_css(position: &CssPosition) -> TaffyPosition {
    match position {
        CssPosition::Absolute | CssPosition::Fixed => TaffyPosition::Absolute,
        CssPosition::Static | CssPosition::Relative | CssPosition::Sticky(_) => {
            TaffyPosition::Relative
        }
    }
}

fn length_percentage_to_taffy(
    value: &LengthPercentage,
) -> Result<TaffyLengthPercentage, StyleError> {
    match value {
        LengthPercentage::Dimension(length) => Ok(TaffyLengthPercentage::Length(
            length
                .to_px()
                .map(|value| value as f32)
                .ok_or_else(|| StyleError::UnsupportedValue(format!("{value:?}")))?,
        )),
        LengthPercentage::Percentage(percentage) => {
            Ok(TaffyLengthPercentage::Percent(percentage.0))
        }
        _ => Err(StyleError::UnsupportedValue(format!("{value:?}"))),
    }
}

fn length_percentage_auto_to_taffy(
    value: &LengthPercentageOrAuto,
) -> Result<TaffyLengthPercentageAuto, StyleError> {
    match value {
        LengthPercentageOrAuto::Auto => Ok(TaffyLengthPercentageAuto::Auto),
        LengthPercentageOrAuto::LengthPercentage(value) => {
            Ok(length_percentage_to_taffy(value)?.into())
        }
    }
}

fn length_percentage_auto_to_taffy_padding(
    value: &LengthPercentageOrAuto,
) -> Result<TaffyLengthPercentage, StyleError> {
    match value {
        LengthPercentageOrAuto::LengthPercentage(value) => length_percentage_to_taffy(value),
        LengthPercentageOrAuto::Auto => Err(StyleError::UnsupportedValue("auto".to_string())),
    }
}

fn dimension_from_css_size(size: &CssSize) -> Result<Dimension, StyleError> {
    match size {
        CssSize::Auto => Ok(Dimension::Auto),
        CssSize::LengthPercentage(value) => Ok(length_percentage_to_taffy(value)?.into()),
        _ => Err(StyleError::UnsupportedValue(format!("{size:?}"))),
    }
}

fn dimension_from_length_percentage_auto(
    value: &LengthPercentageOrAuto,
) -> Result<Dimension, StyleError> {
    match value {
        LengthPercentageOrAuto::Auto => Ok(Dimension::Auto),
        LengthPercentageOrAuto::LengthPercentage(value) => {
            Ok(length_percentage_to_taffy(value)?.into())
        }
    }
}

fn flex_direction_from_css(direction: &CssFlexDirection) -> FlexDirection {
    match direction {
        CssFlexDirection::Row => FlexDirection::Row,
        CssFlexDirection::RowReverse => FlexDirection::RowReverse,
        CssFlexDirection::Column => FlexDirection::Column,
        CssFlexDirection::ColumnReverse => FlexDirection::ColumnReverse,
    }
}

fn flex_wrap_from_css(wrap: &CssFlexWrap) -> FlexWrap {
    match wrap {
        CssFlexWrap::NoWrap => FlexWrap::NoWrap,
        CssFlexWrap::Wrap => FlexWrap::Wrap,
        CssFlexWrap::WrapReverse => FlexWrap::WrapReverse,
    }
}

fn justify_content_from_css(
    justify_content: &CssJustifyContent,
) -> Result<Option<TaffyJustifyContent>, StyleError> {
    Ok(match justify_content {
        CssJustifyContent::Normal => None,
        CssJustifyContent::ContentDistribution(distribution) => {
            Some(content_distribution_to_taffy(*distribution))
        }
        CssJustifyContent::ContentPosition { value, .. } => {
            Some(content_position_to_taffy(*value))
        }
        CssJustifyContent::Left { .. } => Some(TaffyJustifyContent::Start),
        CssJustifyContent::Right { .. } => Some(TaffyJustifyContent::End),
    })
}

fn align_content_from_css(
    align_content: &CssAlignContent,
) -> Result<Option<TaffyAlignContent>, StyleError> {
    Ok(match align_content {
        CssAlignContent::Normal => None,
        CssAlignContent::ContentDistribution(distribution) => {
            Some(content_distribution_to_taffy(*distribution))
        }
        CssAlignContent::ContentPosition { value, .. } => {
            Some(content_position_to_taffy(*value))
        }
        CssAlignContent::BaselinePosition(_) => {
            return Err(StyleError::UnsupportedValue(format!("{align_content:?}")));
        }
    })
}

fn align_items_from_css(
    align_items: &CssAlignItems,
) -> Result<Option<TaffyAlignItems>, StyleError> {
    Ok(match align_items {
        CssAlignItems::Normal => None,
        CssAlignItems::Stretch => Some(TaffyAlignItems::Stretch),
        CssAlignItems::BaselinePosition(_) => Some(TaffyAlignItems::Baseline),
        CssAlignItems::SelfPosition { value, .. } => Some(self_position_to_taffy(*value)),
    })
}

fn align_self_from_css(
    align_self: &CssAlignSelf,
) -> Result<Option<TaffyAlignSelf>, StyleError> {
    Ok(match align_self {
        CssAlignSelf::Auto | CssAlignSelf::Normal => None,
        CssAlignSelf::Stretch => Some(TaffyAlignSelf::Stretch),
        CssAlignSelf::BaselinePosition(_) => Some(TaffyAlignSelf::Baseline),
        CssAlignSelf::SelfPosition { value, .. } => Some(self_position_to_taffy(*value)),
    })
}

fn content_distribution_to_taffy(distribution: ContentDistribution) -> TaffyAlignContent {
    match distribution {
        ContentDistribution::SpaceBetween => TaffyAlignContent::SpaceBetween,
        ContentDistribution::SpaceAround => TaffyAlignContent::SpaceAround,
        ContentDistribution::SpaceEvenly => TaffyAlignContent::SpaceEvenly,
        ContentDistribution::Stretch => TaffyAlignContent::Stretch,
    }
}

fn content_position_to_taffy(position: ContentPosition) -> TaffyAlignContent {
    match position {
        ContentPosition::Center => TaffyAlignContent::Center,
        ContentPosition::Start => TaffyAlignContent::Start,
        ContentPosition::End => TaffyAlignContent::End,
        ContentPosition::FlexStart => TaffyAlignContent::FlexStart,
        ContentPosition::FlexEnd => TaffyAlignContent::FlexEnd,
    }
}

fn self_position_to_taffy(position: SelfPosition) -> TaffyAlignItems {
    match position {
        SelfPosition::Center => TaffyAlignItems::Center,
        SelfPosition::Start | SelfPosition::SelfStart => TaffyAlignItems::Start,
        SelfPosition::End | SelfPosition::SelfEnd => TaffyAlignItems::End,
        SelfPosition::FlexStart => TaffyAlignItems::FlexStart,
        SelfPosition::FlexEnd => TaffyAlignItems::FlexEnd,
    }
}

fn gap_value_to_taffy(value: &GapValue) -> Result<TaffyLengthPercentage, StyleError> {
    match value {
        GapValue::Normal => Ok(TaffyLengthPercentage::Length(0.0)),
        GapValue::LengthPercentage(value) => length_percentage_to_taffy(value),
    }
}

fn track_sizing_from_css(
    track_sizing: &CssTrackSizing<'_>,
) -> Result<Vec<TrackSizingFunction>, StyleError> {
    match track_sizing {
        CssTrackSizing::None => Ok(Vec::new()),
        CssTrackSizing::TrackList(track_list) => {
            let mut tracks = Vec::new();

            for item in &track_list.items {
                match item {
                    TrackListItem::TrackSize(track_size) => {
                        tracks.push(track_size_to_taffy(track_size)?);
                    }
                    TrackListItem::TrackRepeat(repeat) => {
                        let repeated_tracks = repeat
                            .track_sizes
                            .iter()
                            .map(non_repeated_track_size_to_taffy)
                            .collect::<Result<Vec<_>, _>>()?;
                        tracks.push(TrackSizingFunction::Repeat(
                            repeat_count_to_taffy(&repeat.count)?,
                            repeated_tracks.into(),
                        ));
                    }
                }
            }

            Ok(tracks)
        }
    }
}

fn repeat_count_to_taffy(
    repeat_count: &RepeatCount,
) -> Result<GridTrackRepetition, StyleError> {
    match repeat_count {
        RepeatCount::Number(value) if *value > 0 => Ok(GridTrackRepetition::Count(*value as u16)),
        RepeatCount::Number(value) => Err(StyleError::UnsupportedValue(format!("{value:?}"))),
        RepeatCount::AutoFill => Ok(GridTrackRepetition::AutoFill),
        RepeatCount::AutoFit => Ok(GridTrackRepetition::AutoFit),
    }
}

fn track_size_to_taffy(track_size: &TrackSize) -> Result<TrackSizingFunction, StyleError> {
    match track_size {
        TrackSize::TrackBreadth(track_breadth) => {
            Ok(TrackSizingFunction::Single(track_breadth_to_taffy(track_breadth)?))
        }
        TrackSize::MinMax { min, max } => Ok(TrackSizingFunction::Single(
            NonRepeatedTrackSizingFunction {
                min: min_track_breadth_to_taffy(min)?,
                max: max_track_breadth_to_taffy(max)?,
            },
        )),
        TrackSize::FitContent(length) => Ok(TrackSizingFunction::Single(
            NonRepeatedTrackSizingFunction {
                min: MinTrackSizingFunction::Auto,
                max: MaxTrackSizingFunction::FitContent(length_percentage_to_taffy(length)?),
            },
        )),
    }
}

fn non_repeated_track_size_to_taffy(
    track_size: &TrackSize,
) -> Result<NonRepeatedTrackSizingFunction, StyleError> {
    match track_size {
        TrackSize::TrackBreadth(track_breadth) => track_breadth_to_taffy(track_breadth),
        TrackSize::MinMax { min, max } => Ok(NonRepeatedTrackSizingFunction {
            min: min_track_breadth_to_taffy(min)?,
            max: max_track_breadth_to_taffy(max)?,
        }),
        TrackSize::FitContent(length) => Ok(NonRepeatedTrackSizingFunction {
            min: MinTrackSizingFunction::Auto,
            max: MaxTrackSizingFunction::FitContent(length_percentage_to_taffy(length)?),
        }),
    }
}

fn track_breadth_to_taffy(
    track_breadth: &TrackBreadth,
) -> Result<NonRepeatedTrackSizingFunction, StyleError> {
    match track_breadth {
        TrackBreadth::Flex(value) => Ok(NonRepeatedTrackSizingFunction {
            min: MinTrackSizingFunction::Auto,
            max: MaxTrackSizingFunction::Fraction(*value),
        }),
        _ => Ok(NonRepeatedTrackSizingFunction {
            min: min_track_breadth_to_taffy(track_breadth)?,
            max: max_track_breadth_to_taffy(track_breadth)?,
        }),
    }
}

fn min_track_breadth_to_taffy(
    track_breadth: &TrackBreadth,
) -> Result<MinTrackSizingFunction, StyleError> {
    match track_breadth {
        TrackBreadth::Length(value) => Ok(MinTrackSizingFunction::Fixed(
            length_percentage_to_taffy(value)?,
        )),
        TrackBreadth::MinContent => Ok(MinTrackSizingFunction::MinContent),
        TrackBreadth::MaxContent => Ok(MinTrackSizingFunction::MaxContent),
        TrackBreadth::Auto => Ok(MinTrackSizingFunction::Auto),
        TrackBreadth::Flex(value) => Err(StyleError::UnsupportedValue(format!("{value:?}"))),
    }
}

fn max_track_breadth_to_taffy(
    track_breadth: &TrackBreadth,
) -> Result<MaxTrackSizingFunction, StyleError> {
    match track_breadth {
        TrackBreadth::Length(value) => Ok(MaxTrackSizingFunction::Fixed(
            length_percentage_to_taffy(value)?,
        )),
        TrackBreadth::Flex(value) => Ok(MaxTrackSizingFunction::Fraction(*value)),
        TrackBreadth::MinContent => Ok(MaxTrackSizingFunction::MinContent),
        TrackBreadth::MaxContent => Ok(MaxTrackSizingFunction::MaxContent),
        TrackBreadth::Auto => Ok(MaxTrackSizingFunction::Auto),
    }
}

fn grid_row_from_css(row: &CssGridRow<'_>) -> Result<Line<GridPlacement>, StyleError> {
    Ok(Line {
        start: grid_placement_from_css(&row.start)?,
        end: grid_placement_from_css(&row.end)?,
    })
}

fn grid_column_from_css(column: &CssGridColumn<'_>) -> Result<Line<GridPlacement>, StyleError> {
    Ok(Line {
        start: grid_placement_from_css(&column.start)?,
        end: grid_placement_from_css(&column.end)?,
    })
}

fn grid_placement_from_css(line: &CssGridLine<'_>) -> Result<GridPlacement, StyleError> {
    match line {
        CssGridLine::Auto => Ok(GridPlacement::Auto),
        CssGridLine::Line { index, name } => {
            if name.is_some() {
                return Err(StyleError::UnsupportedValue(format!("{line:?}")));
            }

            let index = i16::try_from(*index)
                .map_err(|_| StyleError::UnsupportedValue(format!("{line:?}")))?;
            Ok(GridPlacement::from_line_index(index))
        }
        CssGridLine::Span { index, name } => {
            if name.is_some() || *index <= 0 {
                return Err(StyleError::UnsupportedValue(format!("{line:?}")));
            }

            Ok(GridPlacement::from_span(*index as u16))
        }
        CssGridLine::Area { .. } => Err(StyleError::UnsupportedValue(format!("{line:?}"))),
    }
}

fn apply_declaration(style: &mut Style, position_explicit: &mut bool, declaration: &Declaration) {
    match declaration {
        Declaration::Background(color) => style.visual.background = Some(*color),
        Declaration::Foreground(color) => style.visual.foreground = *color,
        Declaration::CornerTopLeft(value) => style.visual.corner_radius.top_left = *value,
        Declaration::CornerTopRight(value) => style.visual.corner_radius.top_right = *value,
        Declaration::CornerBottomRight(value) => style.visual.corner_radius.bottom_right = *value,
        Declaration::CornerBottomLeft(value) => style.visual.corner_radius.bottom_left = *value,
        Declaration::BorderTopWidth(value) => {
            style.visual.border.widths.top = *value;
            style.layout.taffy.border.top = TaffyLengthPercentage::Length(*value);
        }
        Declaration::BorderRightWidth(value) => {
            style.visual.border.widths.right = *value;
            style.layout.taffy.border.right = TaffyLengthPercentage::Length(*value);
        }
        Declaration::BorderBottomWidth(value) => {
            style.visual.border.widths.bottom = *value;
            style.layout.taffy.border.bottom = TaffyLengthPercentage::Length(*value);
        }
        Declaration::BorderLeftWidth(value) => {
            style.visual.border.widths.left = *value;
            style.layout.taffy.border.left = TaffyLengthPercentage::Length(*value);
        }
        Declaration::BorderColor(color) => {
            style.visual.border.color = color.unwrap_or(style.visual.foreground);
        }
        Declaration::BoxShadows(shadows) => {
            style.visual.shadows = shadows
                .iter()
                .map(|shadow| CoreBoxShadow {
                    color: shadow.color.unwrap_or(style.visual.foreground),
                    offset_x: shadow.offset_x,
                    offset_y: shadow.offset_y,
                    blur_radius: shadow.blur_radius,
                    spread: shadow.spread,
                })
                .collect();
        }
        Declaration::OverflowX(value, clip) => {
            style.layout.taffy.overflow.x = *value;
            style.visual.overflow.clip_x = *clip;
        }
        Declaration::OverflowY(value, clip) => {
            style.layout.taffy.overflow.y = *value;
            style.visual.overflow.clip_y = *clip;
        }
        Declaration::Display(display) => style.layout.taffy.display = *display,
        Declaration::Position(position) => {
            style.layout.taffy.position = *position;
            *position_explicit = true;
        }
        Declaration::InsetTop(value) => {
            if !*position_explicit {
                style.layout.taffy.position = TaffyPosition::Absolute;
            }
            style.layout.taffy.inset.top = *value;
        }
        Declaration::InsetRight(value) => {
            if !*position_explicit {
                style.layout.taffy.position = TaffyPosition::Absolute;
            }
            style.layout.taffy.inset.right = *value;
        }
        Declaration::InsetBottom(value) => {
            if !*position_explicit {
                style.layout.taffy.position = TaffyPosition::Absolute;
            }
            style.layout.taffy.inset.bottom = *value;
        }
        Declaration::InsetLeft(value) => {
            if !*position_explicit {
                style.layout.taffy.position = TaffyPosition::Absolute;
            }
            style.layout.taffy.inset.left = *value;
        }
        Declaration::Width(value) => style.layout.taffy.size.width = *value,
        Declaration::Height(value) => style.layout.taffy.size.height = *value,
        Declaration::MarginTop(value) => style.layout.taffy.margin.top = *value,
        Declaration::MarginRight(value) => style.layout.taffy.margin.right = *value,
        Declaration::MarginBottom(value) => style.layout.taffy.margin.bottom = *value,
        Declaration::MarginLeft(value) => style.layout.taffy.margin.left = *value,
        Declaration::PaddingTop(value) => style.layout.taffy.padding.top = *value,
        Declaration::PaddingRight(value) => style.layout.taffy.padding.right = *value,
        Declaration::PaddingBottom(value) => style.layout.taffy.padding.bottom = *value,
        Declaration::PaddingLeft(value) => style.layout.taffy.padding.left = *value,
        Declaration::FlexDirection(value) => style.layout.taffy.flex_direction = *value,
        Declaration::FlexWrap(value) => style.layout.taffy.flex_wrap = *value,
        Declaration::JustifyContent(value) => style.layout.taffy.justify_content = *value,
        Declaration::AlignItems(value) => style.layout.taffy.align_items = *value,
        Declaration::AlignSelf(value) => style.layout.taffy.align_self = *value,
        Declaration::AlignContent(value) => style.layout.taffy.align_content = *value,
        Declaration::GapRow(value) => style.layout.taffy.gap.height = *value,
        Declaration::GapColumn(value) => style.layout.taffy.gap.width = *value,
        Declaration::FlexGrow(value) => style.layout.taffy.flex_grow = *value,
        Declaration::FlexShrink(value) => style.layout.taffy.flex_shrink = *value,
        Declaration::FlexBasis(value) => style.layout.taffy.flex_basis = *value,
        Declaration::GridTemplateColumns(value) => {
            style.layout.taffy.grid_template_columns = value.clone();
        }
        Declaration::GridTemplateRows(value) => {
            style.layout.taffy.grid_template_rows = value.clone();
        }
        Declaration::GridColumn(value) => style.layout.taffy.grid_column = *value,
        Declaration::GridRow(value) => style.layout.taffy.grid_row = *value,
        Declaration::GridColumnStart(value) => style.layout.taffy.grid_column.start = *value,
        Declaration::GridColumnEnd(value) => style.layout.taffy.grid_column.end = *value,
        Declaration::GridRowStart(value) => style.layout.taffy.grid_row.start = *value,
        Declaration::GridRowEnd(value) => style.layout.taffy.grid_row.end = *value,
    }
}

fn resolve_element_tree(element: &ElementNode, stylesheet: &Stylesheet) -> ResolvedElement {
    ResolvedElement {
        style: resolve_style(element, stylesheet),
        text: element_text(element),
        on_click: element.on_click,
        children: element
            .children
            .iter()
            .filter_map(|child| match child {
                Node::Element(child) => Some(resolve_element_tree(child, stylesheet)),
                Node::Text(_) => None,
            })
            .collect(),
    }
}

fn build_layout_tree(
    resolved: &ResolvedElement,
    taffy: &mut TaffyTree<LeafMeasureContext>,
) -> LayoutTree {
    let children: Vec<_> = resolved
        .children
        .iter()
        .map(|child| build_layout_tree(child, taffy))
        .collect();
    let child_ids: Vec<_> = children.iter().map(|child| child.node_id).collect();
    let node_id = if child_ids.is_empty() {
        taffy
            .new_leaf_with_context(
                to_taffy(&resolved.style.layout),
                LeafMeasureContext {
                    text: resolved.text.clone(),
                },
            )
            .expect("leaf style should be accepted by taffy")
    } else {
        taffy
            .new_with_children(to_taffy(&resolved.style.layout), &child_ids)
            .expect("container style should be accepted by taffy")
    };

    LayoutTree {
        node_id,
        style: resolved.style.clone(),
        text: resolved.text.clone(),
        on_click: resolved.on_click,
        children,
    }
}

fn available_space_from_root(style: &TaffyStyle) -> TaffySize<AvailableSpace> {
    TaffySize {
        width: available_space_from_dimension(style.size.width),
        height: available_space_from_dimension(style.size.height),
    }
}

fn available_space_from_dimension(dimension: Dimension) -> AvailableSpace {
    match dimension {
        Dimension::Length(value) => AvailableSpace::Definite(value),
        _ => AvailableSpace::MaxContent,
    }
}

fn render_node_from_layout(
    tree: &LayoutTree,
    taffy: &TaffyTree<LeafMeasureContext>,
    parent_x: f32,
    parent_y: f32,
) -> RenderNode {
    let layout = taffy
        .layout(tree.node_id)
        .expect("computed layouts should be readable");
    let x = parent_x + layout.location.x;
    let y = parent_y + layout.location.y;
    let layout_box = LayoutBox::new(x, y, layout.size.width, layout.size.height);
    let child_nodes: Vec<_> = tree
        .children
        .iter()
        .map(|child| render_node_from_layout(child, taffy, x, y))
        .collect();
    let content_inset = content_inset_from_taffy(&tree.style.layout.taffy);

    if child_nodes.is_empty() && !tree.text.is_empty() {
        let mut node = RenderNode::text(layout_box, tree.text.clone())
            .with_style(tree.style.visual.clone())
            .with_content_inset(content_inset);
        if let Some(handler) = tree.on_click {
            node = node.on_click(handler);
        }
        node
    } else {
        let mut node = RenderNode::container(layout_box)
            .with_style(tree.style.visual.clone())
            .with_content_inset(content_inset)
            .with_children(child_nodes);
        if let Some(handler) = tree.on_click {
            node = node.on_click(handler);
        }
        node
    }
}

fn element_text(element: &ElementNode) -> String {
    let mut content = String::new();

    for child in &element.children {
        collect_text(child, &mut content);
    }

    content
}

fn collect_text(node: &Node, buffer: &mut String) {
    match node {
        Node::Text(content) => buffer.push_str(content),
        Node::Element(element) => {
            for child in &element.children {
                collect_text(child, buffer);
            }
        }
    }
}

fn measure_text(
    text: &str,
    known_dimensions: TaffySize<Option<f32>>,
    available_space: TaffySize<AvailableSpace>,
) -> TaffySize<f32> {
    if text.is_empty() {
        return TaffySize {
            width: 0.0,
            height: 0.0,
        };
    }

    let wrap_width = known_dimensions.width.or_else(|| match available_space.width {
        AvailableSpace::Definite(width) => Some(width.max(GLYPH_WIDTH)),
        AvailableSpace::MinContent | AvailableSpace::MaxContent => None,
    });
    let lines = wrap_text(text, wrap_width);
    let line_count = lines.len();
    let max_columns = lines.iter().map(|line| line.chars().count()).max().unwrap_or(0);

    TaffySize {
        width: known_dimensions
            .width
            .unwrap_or(max_columns as f32 * GLYPH_WIDTH),
        height: known_dimensions
            .height
            .unwrap_or(line_count as f32 * GLYPH_HEIGHT),
    }
}

fn wrap_text(text: &str, wrap_width: Option<f32>) -> Vec<String> {
    let max_columns = wrap_width
        .map(|width| (width / GLYPH_WIDTH).floor().max(1.0) as usize);
    let Some(max_columns) = max_columns else {
        return text.lines().map(|line| line.to_string()).collect();
    };

    let mut wrapped = Vec::new();
    for source_line in text.lines() {
        wrap_line(source_line, max_columns, &mut wrapped);
    }

    if wrapped.is_empty() {
        wrapped.push(String::new());
    }

    wrapped
}

fn wrap_line(line: &str, max_columns: usize, wrapped: &mut Vec<String>) {
    if line.is_empty() {
        wrapped.push(String::new());
        return;
    }

    let mut current = String::new();
    let mut current_len = 0;
    for word in line.split_whitespace() {
        let word_len = word.chars().count();
        let spacing = usize::from(current_len != 0);

        if word_len > max_columns {
            if current_len != 0 {
                wrapped.push(std::mem::take(&mut current));
                current_len = 0;
            }
            push_broken_word(word, max_columns, wrapped);
            continue;
        }

        if current_len + spacing + word_len > max_columns {
            wrapped.push(std::mem::take(&mut current));
            current_len = 0;
        }

        if current_len != 0 {
            current.push(' ');
            current_len += 1;
        }
        current.push_str(word);
        current_len += word_len;
    }

    if current_len != 0 {
        wrapped.push(current);
    }
}

fn push_broken_word(word: &str, max_columns: usize, wrapped: &mut Vec<String>) {
    let mut segment = String::new();
    let mut segment_len = 0;
    for character in word.chars() {
        if segment_len == max_columns {
            wrapped.push(std::mem::take(&mut segment));
            segment_len = 0;
        }
        segment.push(character);
        segment_len += 1;
    }
    if segment_len != 0 {
        wrapped.push(segment);
    }
}

fn content_inset_from_taffy(style: &TaffyStyle) -> cssimpler_core::Insets {
    cssimpler_core::Insets {
        top: resolved_length(style.padding.top) + resolved_length(style.border.top),
        right: resolved_length(style.padding.right) + resolved_length(style.border.right),
        bottom: resolved_length(style.padding.bottom) + resolved_length(style.border.bottom),
        left: resolved_length(style.padding.left) + resolved_length(style.border.left),
    }
}

fn resolved_length(value: TaffyLengthPercentage) -> f32 {
    match value {
        TaffyLengthPercentage::Length(value) => value,
        TaffyLengthPercentage::Percent(_) => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use cssimpler_core::{Color, Node};
    use taffy::prelude::{
        AlignItems as TaffyAlignItems, Dimension, Display as TaffyDisplay,
        FlexDirection as TaffyFlexDirection, JustifyContent as TaffyJustifyContent,
        LengthPercentage as TaffyLengthPercentage,
        LengthPercentageAuto as TaffyLengthPercentageAuto,
    };

    use super::{
        Declaration, ElementRef, Selector, StyleRule, Stylesheet, build_render_tree,
        parse_stylesheet, resolve_style, to_taffy,
    };

    #[test]
    fn selectors_match_supported_primitives() {
        let classes = vec!["card".to_string(), "selected".to_string()];
        let element = ElementRef {
            tag: "div",
            id: Some("hero"),
            classes: &classes,
        };

        assert!(Selector::Class("card".to_string()).matches(element));
        assert!(Selector::Id("hero".to_string()).matches(element));
        assert!(Selector::Tag("div".to_string()).matches(element));
        assert!(!Selector::Class("ghost".to_string()).matches(element));
    }

    #[test]
    fn matching_rules_are_returned_in_insertion_order() {
        let classes = vec!["card".to_string()];
        let mut stylesheet = Stylesheet::default();
        stylesheet.push(StyleRule::new(
            Selector::Class("card".to_string()),
            vec![Declaration::Width(Dimension::Length(300.0))],
        ));
        stylesheet.push(StyleRule::new(
            Selector::Tag("p".to_string()),
            vec![Declaration::Height(Dimension::Length(40.0))],
        ));

        let matching: Vec<_> = stylesheet
            .matching_rules(ElementRef {
                tag: "div",
                id: None,
                classes: &classes,
            })
            .collect();

        assert_eq!(matching.len(), 1);
    }

    #[test]
    fn parser_supports_a_small_bootstrap_stylesheet() {
        let stylesheet = parse_stylesheet(
            "#app { width: 960px; height: 540px; background-color: #e2e8f0; }
             .card { left: 160px; top: 128px; width: 640px; height: 220px; background-color: #0f172a; color: #ffffff; }",
        )
        .expect("bootstrap stylesheet should parse");

        assert_eq!(stylesheet.rules.len(), 2);
        assert!(
            stylesheet.rules[0]
                .declarations
                .contains(&Declaration::Background(Color::rgb(226, 232, 240)))
        );
        assert!(stylesheet.rules[1].declarations.contains(&Declaration::InsetLeft(
            TaffyLengthPercentageAuto::Length(160.0),
        )));
    }

    #[test]
    fn parser_supports_tag_selectors() {
        let stylesheet = parse_stylesheet("button { width: 120px; height: 32px; color: #2563eb; }")
            .expect("tag selector stylesheet should parse");

        assert_eq!(stylesheet.rules.len(), 1);
        assert_eq!(stylesheet.rules[0].selector, Selector::Tag("button".to_string()));
        assert!(
            stylesheet.rules[0]
                .declarations
                .contains(&Declaration::Width(Dimension::Length(120.0)))
        );
    }

    #[test]
    fn parser_supports_fractional_grid_tracks() {
        parse_stylesheet(
            "#app {
                display: grid;
                width: 640px;
                grid-template-columns: 1fr 160px;
                grid-template-rows: 32px 24px;
            }",
        )
        .expect("fractional grid tracks should parse");
    }

    #[test]
    fn resolve_style_maps_flex_layout_properties_to_taffy() {
        let stylesheet = parse_stylesheet(
            "#app {
                display: flex;
                flex-direction: column;
                justify-content: center;
                align-items: flex-start;
                width: 180px;
                height: 90px;
                padding: 12px;
                margin: 4px;
                gap: 6px 8px;
            }",
        )
        .expect("stylesheet should parse");
        let element = Node::element("div").with_id("app");
        let style = resolve_style(&element, &stylesheet);
        let taffy = to_taffy(&style.layout);

        assert_eq!(taffy.display, TaffyDisplay::Flex);
        assert_eq!(taffy.flex_direction, TaffyFlexDirection::Column);
        assert_eq!(taffy.justify_content, Some(TaffyJustifyContent::Center));
        assert_eq!(taffy.align_items, Some(TaffyAlignItems::FlexStart));
        assert_eq!(taffy.size.width, Dimension::Length(180.0));
        assert_eq!(taffy.size.height, Dimension::Length(90.0));
        assert_eq!(taffy.padding.top, TaffyLengthPercentage::Length(12.0));
        assert_eq!(taffy.margin.left, TaffyLengthPercentageAuto::Length(4.0));
        assert_eq!(taffy.gap.height, TaffyLengthPercentage::Length(6.0));
        assert_eq!(taffy.gap.width, TaffyLengthPercentage::Length(8.0));
    }

    #[test]
    fn flex_layout_respects_padding_gap_and_margin() {
        let stylesheet = parse_stylesheet(
            "#app {
                display: flex;
                width: 200px;
                height: 80px;
                padding: 10px;
                gap: 8px;
                align-items: flex-start;
                background-color: #ffffff;
            }
            .first { width: 40px; height: 20px; background-color: #111111; }
            .second { width: 30px; height: 20px; margin-left: 6px; background-color: #222222; }",
        )
        .expect("stylesheet should parse");
        let tree = Node::element("div")
            .with_id("app")
            .with_child(Node::element("div").with_class("first").into())
            .with_child(Node::element("div").with_class("second").into())
            .into();
        let scene = build_render_tree(&tree, &stylesheet);

        assert_eq!(scene.layout.width, 200.0);
        assert_eq!(scene.layout.height, 80.0);
        assert_eq!(scene.children.len(), 2);
        assert_eq!(scene.children[0].layout.x, 10.0);
        assert_eq!(scene.children[0].layout.y, 10.0);
        assert_eq!(scene.children[1].layout.x, 64.0);
        assert_eq!(scene.children[1].layout.y, 10.0);
    }

    #[test]
    fn grid_layout_positions_children_from_template_tracks() {
        let stylesheet = parse_stylesheet(
            "#app {
                display: grid;
                width: 200px;
                height: 100px;
                padding: 10px;
                gap: 12px 6px;
                grid-template-columns: 80px 60px;
                grid-template-rows: 24px 30px;
                background-color: #ffffff;
            }
            .title { grid-column: 1 / 3; grid-row: 1; height: 24px; color: #0f172a; }
            .button {
                grid-column: 2;
                grid-row: 2;
                width: 60px;
                height: 30px;
                background-color: #2563eb;
                color: #ffffff;
            }",
        )
        .expect("stylesheet should parse");
        let tree = Node::element("div")
            .with_id("app")
            .with_child(
                Node::element("h1")
                    .with_class("title")
                    .with_child(Node::text("grid title"))
                    .into(),
            )
            .with_child(
                Node::element("button")
                    .with_class("button")
                    .with_child(Node::text("go"))
                    .into(),
            )
            .into();
        let scene = build_render_tree(&tree, &stylesheet);

        assert_eq!(scene.children.len(), 2);
        assert_eq!(scene.children[0].layout.x, 10.0);
        assert_eq!(scene.children[0].layout.y, 10.0);
        assert_eq!(scene.children[0].layout.width, 146.0);
        assert_eq!(scene.children[1].layout.x, 96.0);
        assert_eq!(scene.children[1].layout.y, 46.0);
        assert_eq!(scene.children[1].layout.width, 60.0);
    }

    #[test]
    fn render_tree_builder_resolves_styles_outside_the_app() {
        let stylesheet = parse_stylesheet(
            "#app { width: 100px; height: 80px; background-color: #ffffff; }
             .title { left: 10px; top: 12px; width: 40px; height: 16px; color: #0f172a; }",
        )
        .expect("stylesheet should parse");
        let tree = Node::element("div")
            .with_id("app")
            .with_child(
                Node::element("h1")
                    .with_class("title")
                    .with_child(Node::text("hello"))
                    .into(),
            )
            .into();
        let scene = build_render_tree(&tree, &stylesheet);

        assert_eq!(scene.layout.width, 100.0);
        assert_eq!(scene.children.len(), 1);
        assert_eq!(scene.children[0].layout.x, 10.0);
        assert_eq!(scene.children[0].layout.y, 12.0);
    }

    #[test]
    fn later_matching_rules_override_earlier_declarations() {
        let stylesheet = parse_stylesheet(
            ".card { width: 120px; height: 24px; color: #0f172a; }
             .card { width: 180px; color: #2563eb; }",
        )
        .expect("stylesheet should parse");
        let tree = Node::element("div")
            .with_class("card")
            .with_child(Node::text("override"))
            .into();
        let scene = build_render_tree(&tree, &stylesheet);

        assert_eq!(scene.layout.width, 180.0);
        assert_eq!(scene.layout.height, 24.0);
        assert_eq!(scene.style.foreground, Color::rgb(37, 99, 235));
    }

    #[test]
    fn layout_results_are_stable_across_repeated_builds() {
        let stylesheet = parse_stylesheet(
            "#app { display: flex; width: 160px; height: 60px; padding: 8px; gap: 4px; }
             .chip { width: 40px; height: 20px; }",
        )
        .expect("stylesheet should parse");
        let tree = Node::element("div")
            .with_id("app")
            .with_child(Node::element("div").with_class("chip").into())
            .with_child(Node::element("div").with_class("chip").into())
            .into();

        let first = build_render_tree(&tree, &stylesheet);
        let second = build_render_tree(&tree, &stylesheet);

        assert_eq!(first.layout, second.layout);
        assert_eq!(first.children[0].layout, second.children[0].layout);
        assert_eq!(first.children[1].layout, second.children[1].layout);
    }

    #[test]
    fn render_tree_preserves_click_handlers() {
        fn increment() {}

        let stylesheet = parse_stylesheet(
            ".button { width: 90px; height: 24px; background-color: #1d4ed8; color: #ffffff; }",
        )
        .expect("stylesheet should parse");
        let tree = Node::element("button")
            .with_class("button")
            .on_click(increment)
            .with_child(Node::text("increment"))
            .into();
        let scene = build_render_tree(&tree, &stylesheet);

        assert!(scene.on_click.is_some());
        assert_eq!(scene.layout.width, 90.0);
        assert!(matches!(scene.kind, cssimpler_core::RenderKind::Text(_)));
    }
}
