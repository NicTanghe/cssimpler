use std::error::Error;
use std::fmt::{Display, Formatter};

use cssimpler_core::{
    Color, ElementNode, EventHandler, LayoutBox, LayoutStyle, Node, OverflowMode, RenderNode,
    ScrollbarWidth, Style,
    fonts::{TextStyle, layout_text_block},
};
use lightningcss::declaration::DeclarationBlock;
use lightningcss::properties::Property;
use lightningcss::properties::align::{
    AlignContent as CssAlignContent, AlignItems as CssAlignItems, AlignSelf as CssAlignSelf,
    ContentDistribution, ContentPosition, GapValue, JustifyContent as CssJustifyContent,
    SelfPosition,
};
use lightningcss::properties::display::{Display as CssDisplay, DisplayInside, DisplayKeyword};
use lightningcss::properties::flex::{FlexDirection as CssFlexDirection, FlexWrap as CssFlexWrap};
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
use lightningcss::values::length::{LengthPercentage, LengthPercentageOrAuto};
use taffy::geometry::{Line, Size as TaffySize};
use taffy::prelude::{
    AlignContent as TaffyAlignContent, AlignItems as TaffyAlignItems, AlignSelf as TaffyAlignSelf,
    AvailableSpace, Dimension, Display as TaffyDisplay, FlexDirection, FlexWrap, GridPlacement,
    GridTrackRepetition, JustifyContent as TaffyJustifyContent,
    LengthPercentage as TaffyLengthPercentage, LengthPercentageAuto as TaffyLengthPercentageAuto,
    MaxTrackSizingFunction, MinTrackSizingFunction, NodeId, NonRepeatedTrackSizingFunction,
    Position as TaffyPosition, Style as TaffyStyle, TaffyGridLine, TaffyGridSpan, TaffyTree,
    TrackSizingFunction,
};

mod fonts;
mod visual;

use self::fonts::{
    FontSizeDeclaration, FontWeightDeclaration, LineHeightDeclaration, apply_font_declaration,
    extract_property as extract_font_property,
};

pub use visual::{BackgroundLayerDeclaration, ShadowDeclaration};

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
    BackgroundLayers(Vec<BackgroundLayerDeclaration>),
    Foreground(Color),
    FontFamilies(Vec<cssimpler_core::fonts::FontFamily>),
    FontSize(FontSizeDeclaration),
    FontWeight(FontWeightDeclaration),
    FontStyle(cssimpler_core::fonts::FontStyle),
    LineHeight(LineHeightDeclaration),
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
    OverflowX(OverflowMode),
    OverflowY(OverflowMode),
    ScrollbarWidth(ScrollbarWidth),
    ScrollbarColors(Option<Color>, Option<Color>),
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
    text_style: TextStyle,
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
    resolve_style_with_inherited_text(element, stylesheet, None)
}

fn resolve_style_with_inherited_text(
    element: &ElementNode,
    stylesheet: &Stylesheet,
    inherited_text: Option<&TextStyle>,
) -> Style {
    let mut resolved = element.style.clone();
    if let Some(inherited_text) = inherited_text {
        resolved.visual.text = inherited_text.clone();
    }
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

    let resolved = resolve_element_tree(root_element, stylesheet, None);
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
                    |context| {
                        measure_text(
                            &context.text,
                            &context.text_style,
                            known_dimensions,
                            available_space,
                        )
                    },
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
    if let Some(declarations) = extract_font_property(property) {
        return declarations;
    }

    if let Some(declarations) = visual::extract_property(property) {
        return declarations;
    }

    match property {
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
        Property::Top(value) => Ok(vec![Declaration::InsetTop(
            length_percentage_auto_to_taffy(value)?,
        )]),
        Property::Right(value) => Ok(vec![Declaration::InsetRight(
            length_percentage_auto_to_taffy(value)?,
        )]),
        Property::Bottom(value) => Ok(vec![Declaration::InsetBottom(
            length_percentage_auto_to_taffy(value)?,
        )]),
        Property::Left(value) => Ok(vec![Declaration::InsetLeft(
            length_percentage_auto_to_taffy(value)?,
        )]),
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
        Property::FlexDirection(direction, _) => Ok(vec![Declaration::FlexDirection(
            flex_direction_from_css(direction),
        )]),
        Property::FlexWrap(wrap, _) => Ok(vec![Declaration::FlexWrap(flex_wrap_from_css(wrap))]),
        Property::JustifyContent(content, _) => Ok(vec![Declaration::JustifyContent(
            justify_content_from_css(content)?,
        )]),
        Property::AlignItems(items, _) => {
            Ok(vec![Declaration::AlignItems(align_items_from_css(items)?)])
        }
        Property::AlignSelf(self_alignment, _) => Ok(vec![Declaration::AlignSelf(
            align_self_from_css(self_alignment)?,
        )]),
        Property::AlignContent(content, _) => Ok(vec![Declaration::AlignContent(
            align_content_from_css(content)?,
        )]),
        Property::RowGap(value) => Ok(vec![Declaration::GapRow(gap_value_to_taffy(value)?)]),
        Property::ColumnGap(value) => Ok(vec![Declaration::GapColumn(gap_value_to_taffy(value)?)]),
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
        Property::GridColumn(column) => {
            Ok(vec![Declaration::GridColumn(grid_column_from_css(column)?)])
        }
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
        Property::GridRowEnd(end) => {
            Ok(vec![Declaration::GridRowEnd(grid_placement_from_css(end)?)])
        }
        _ => Ok(Vec::new()),
    }
}

fn overflow_x_declaration(value: CssOverflowKeyword) -> Declaration {
    Declaration::OverflowX(overflow_mode_from_css_keyword(value))
}

fn overflow_y_declaration(value: CssOverflowKeyword) -> Declaration {
    Declaration::OverflowY(overflow_mode_from_css_keyword(value))
}

fn overflow_mode_from_css_keyword(value: CssOverflowKeyword) -> OverflowMode {
    match value {
        CssOverflowKeyword::Visible => OverflowMode::Visible,
        CssOverflowKeyword::Clip => OverflowMode::Clip,
        CssOverflowKeyword::Hidden => OverflowMode::Hidden,
        CssOverflowKeyword::Auto => OverflowMode::Auto,
        CssOverflowKeyword::Scroll => OverflowMode::Scroll,
    }
}

fn display_from_css(display: &CssDisplay) -> Result<TaffyDisplay, StyleError> {
    match display {
        CssDisplay::Keyword(DisplayKeyword::None) => Ok(TaffyDisplay::None),
        CssDisplay::Keyword(keyword) => Err(StyleError::UnsupportedValue(format!("{keyword:?}"))),
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
        CssJustifyContent::ContentPosition { value, .. } => Some(content_position_to_taffy(*value)),
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
        CssAlignContent::ContentPosition { value, .. } => Some(content_position_to_taffy(*value)),
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

fn align_self_from_css(align_self: &CssAlignSelf) -> Result<Option<TaffyAlignSelf>, StyleError> {
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

fn repeat_count_to_taffy(repeat_count: &RepeatCount) -> Result<GridTrackRepetition, StyleError> {
    match repeat_count {
        RepeatCount::Number(value) if *value > 0 => Ok(GridTrackRepetition::Count(*value as u16)),
        RepeatCount::Number(value) => Err(StyleError::UnsupportedValue(format!("{value:?}"))),
        RepeatCount::AutoFill => Ok(GridTrackRepetition::AutoFill),
        RepeatCount::AutoFit => Ok(GridTrackRepetition::AutoFit),
    }
}

fn track_size_to_taffy(track_size: &TrackSize) -> Result<TrackSizingFunction, StyleError> {
    match track_size {
        TrackSize::TrackBreadth(track_breadth) => Ok(TrackSizingFunction::Single(
            track_breadth_to_taffy(track_breadth)?,
        )),
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
    if apply_font_declaration(style, declaration) {
        return;
    }

    if visual::apply_declaration(style, declaration) {
        return;
    }

    match declaration {
        Declaration::OverflowX(mode) => {
            style.layout.taffy.overflow.x = visual::taffy_overflow_from_mode(*mode);
            style.visual.overflow.x = *mode;
            visual::sync_scrollbar_gutter(style);
        }
        Declaration::OverflowY(mode) => {
            style.layout.taffy.overflow.y = visual::taffy_overflow_from_mode(*mode);
            style.visual.overflow.y = *mode;
            visual::sync_scrollbar_gutter(style);
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
        _ => unreachable!("visual declarations are handled before the layout match"),
    }
}

fn resolve_element_tree(
    element: &ElementNode,
    stylesheet: &Stylesheet,
    inherited_text: Option<&TextStyle>,
) -> ResolvedElement {
    let style = resolve_style_with_inherited_text(element, stylesheet, inherited_text);

    ResolvedElement {
        style: style.clone(),
        text: element_text(element),
        on_click: element.on_click,
        children: element
            .children
            .iter()
            .filter_map(|child| match child {
                Node::Element(child) => Some(resolve_element_tree(
                    child,
                    stylesheet,
                    Some(&style.visual.text),
                )),
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
                    text_style: resolved.style.visual.text.clone(),
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
    let scrollbars = visual::scrollbars_from_layout(&tree.style, layout);

    if child_nodes.is_empty() && !tree.text.is_empty() {
        let mut node = RenderNode::text(layout_box, tree.text.clone())
            .with_style(tree.style.visual.clone())
            .with_content_inset(content_inset);
        if let Some(scrollbars) = scrollbars {
            node = node.with_scrollbars(scrollbars);
        }
        if let Some(handler) = tree.on_click {
            node = node.on_click(handler);
        }
        node
    } else {
        let mut node = RenderNode::container(layout_box)
            .with_style(tree.style.visual.clone())
            .with_content_inset(content_inset)
            .with_children(child_nodes);
        if let Some(scrollbars) = scrollbars {
            node = node.with_scrollbars(scrollbars);
        }
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
    text_style: &TextStyle,
    known_dimensions: TaffySize<Option<f32>>,
    available_space: TaffySize<AvailableSpace>,
) -> TaffySize<f32> {
    if text.is_empty() {
        return TaffySize {
            width: 0.0,
            height: 0.0,
        };
    }

    let wrap_width = known_dimensions
        .width
        .or_else(|| match available_space.width {
            AvailableSpace::Definite(width) => Some(width.max(1.0)),
            AvailableSpace::MinContent | AvailableSpace::MaxContent => None,
        });
    let layout = layout_text_block(text, text_style, wrap_width);

    TaffySize {
        width: known_dimensions.width.unwrap_or(layout.width),
        height: known_dimensions.height.unwrap_or(layout.height),
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
    use cssimpler_core::{
        AnglePercentageValue, BackgroundLayer, CircleRadius, Color, ConicGradient,
        GradientDirection, GradientHorizontal, GradientPoint, GradientStop, LengthPercentageValue,
        LinearGradient, Node, RadialShape, ScrollbarWidth, ShapeExtent,
    };
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
        assert!(
            stylesheet.rules[1]
                .declarations
                .contains(&Declaration::InsetLeft(TaffyLengthPercentageAuto::Length(
                    160.0
                ),))
        );
    }

    #[test]
    fn parser_supports_tag_selectors() {
        let stylesheet = parse_stylesheet("button { width: 120px; height: 32px; color: #2563eb; }")
            .expect("tag selector stylesheet should parse");

        assert_eq!(stylesheet.rules.len(), 1);
        assert_eq!(
            stylesheet.rules[0].selector,
            Selector::Tag("button".to_string())
        );
        assert!(
            stylesheet.rules[0]
                .declarations
                .contains(&Declaration::Width(Dimension::Length(120.0)))
        );
    }

    #[test]
    fn parser_supports_scrollbar_custom_properties() {
        let stylesheet = parse_stylesheet(
            ".pane {
                overflow: auto;
                scrollbar-width: thin;
                scrollbar-color: #112233 #ddeeff;
            }",
        )
        .expect("scrollbar stylesheet should parse");

        assert!(
            stylesheet.rules[0]
                .declarations
                .contains(&Declaration::ScrollbarWidth(ScrollbarWidth::Thin))
        );
        assert!(
            stylesheet.rules[0]
                .declarations
                .contains(&Declaration::ScrollbarColors(
                    Some(Color::rgb(17, 34, 51)),
                    Some(Color::rgb(221, 238, 255)),
                ))
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
    fn parser_supports_linear_gradient_background_images() {
        let stylesheet = parse_stylesheet(
            ".card {
                width: 3px;
                height: 1px;
                background-image: linear-gradient(to right, #000000, #ffffff);
            }",
        )
        .expect("linear gradient stylesheet should parse");
        let tree = Node::element("div").with_class("card").into();
        let scene = build_render_tree(&tree, &stylesheet);

        assert_eq!(
            scene.style.background_layers,
            vec![BackgroundLayer::LinearGradient(LinearGradient {
                direction: GradientDirection::Horizontal(GradientHorizontal::Right),
                repeating: false,
                stops: vec![
                    GradientStop {
                        color: Color::rgb(0, 0, 0),
                        position: LengthPercentageValue::from_fraction(0.0),
                    },
                    GradientStop {
                        color: Color::rgb(255, 255, 255),
                        position: LengthPercentageValue::from_fraction(1.0),
                    },
                ],
            })]
        );
    }

    #[test]
    fn parser_supports_layered_radial_and_conic_backgrounds() {
        let stylesheet = parse_stylesheet(
            ".card {
                width: 5px;
                height: 5px;
                background:
                    radial-gradient(circle at 50% 50%, #ff0000 2px, #0000ff 100%),
                    conic-gradient(from 90deg at 25% 75%, #00ff00 0deg, #ffffff 1turn),
                    #112233;
            }",
        )
        .expect("layered gradient background should parse");
        let tree = Node::element("div").with_class("card").into();
        let scene = build_render_tree(&tree, &stylesheet);

        assert_eq!(scene.style.background, Some(Color::rgb(17, 34, 51)));
        assert_eq!(scene.style.background_layers.len(), 2);

        assert_eq!(
            scene.style.background_layers[0],
            BackgroundLayer::RadialGradient(cssimpler_core::RadialGradient {
                shape: RadialShape::Circle(CircleRadius::Extent(ShapeExtent::FarthestCorner)),
                center: GradientPoint::CENTER,
                repeating: false,
                stops: vec![
                    GradientStop {
                        color: Color::rgb(255, 0, 0),
                        position: LengthPercentageValue::from_px(2.0),
                    },
                    GradientStop {
                        color: Color::rgb(0, 0, 255),
                        position: LengthPercentageValue::from_fraction(1.0),
                    },
                ],
            })
        );
        assert_eq!(
            scene.style.background_layers[1],
            BackgroundLayer::ConicGradient(ConicGradient {
                angle: 90.0,
                center: GradientPoint {
                    x: LengthPercentageValue::from_fraction(0.25),
                    y: LengthPercentageValue::from_fraction(0.75),
                },
                repeating: false,
                stops: vec![
                    GradientStop {
                        color: Color::rgb(0, 255, 0),
                        position: AnglePercentageValue::from_degrees(0.0),
                    },
                    GradientStop {
                        color: Color::rgb(255, 255, 255),
                        position: AnglePercentageValue::from_degrees(360.0),
                    },
                ],
            })
        );
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

    #[test]
    fn render_tree_attaches_scrollbar_metrics_from_layout() {
        let stylesheet = parse_stylesheet(
            "#pane {
                width: 100px;
                height: 80px;
                overflow-y: scroll;
                scrollbar-width: 12px;
                background-color: #ffffff;
            }
            .tall {
                width: 88px;
                height: 200px;
                background-color: #0f172a;
            }",
        )
        .expect("scrollbar stylesheet should parse");
        let tree = Node::element("div")
            .with_id("pane")
            .with_child(Node::element("div").with_class("tall").into())
            .into();
        let scene = build_render_tree(&tree, &stylesheet);
        let scrollbars = scene.scrollbars.expect("scrollbars should be attached");

        assert!(scrollbars.shows_vertical());
        assert_eq!(scrollbars.metrics.reserved_width, 12.0);
        assert!(scrollbars.metrics.max_offset_y > 0.0);
    }
}
