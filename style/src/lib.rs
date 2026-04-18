use std::collections::HashMap;
use std::error::Error;
use std::fmt::{Display, Formatter};

use cssimpler_core::{
    BorderLineStyle, Color, CustomProperties, ElementInteractionState, ElementNode, ElementPath,
    LayoutStyle, OverflowMode, ScrollbarWidth, Style, SvgPaint, TransformOperation, TransformOrigin,
    TransformStyleMode, TransitionPropertyName, TransitionTimingFunction,
    fonts::{TextStyle, TextTransform},
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
use lightningcss::properties::size::{MaxSize as CssMaxSize, Size as CssSize};
use lightningcss::rules::CssRule;
use lightningcss::stylesheet::{ParserOptions, StyleSheet};
use lightningcss::values::length::{LengthPercentage, LengthPercentageOrAuto, LengthValue};
use taffy::geometry::Line;
use taffy::prelude::{
    AlignContent as TaffyAlignContent, AlignItems as TaffyAlignItems, AlignSelf as TaffyAlignSelf,
    Dimension, Display as TaffyDisplay, FlexDirection, FlexWrap, GridPlacement,
    GridTrackRepetition, JustifyContent as TaffyJustifyContent,
    LengthPercentage as TaffyLengthPercentage, LengthPercentageAuto as TaffyLengthPercentageAuto,
    MaxTrackSizingFunction, MinTrackSizingFunction, NonRepeatedTrackSizingFunction,
    Position as TaffyPosition, Style as TaffyStyle, TaffyGridLine, TaffyGridSpan,
    TrackSizingFunction,
};

mod attributes;
mod custom_properties;
mod fonts;
mod invalidation;
mod render_tree;
mod selectors;
mod svg;
mod transitions;
mod variable_resolution;
mod visual;

use self::attributes::{parse_content_text_source, reject_unsupported_attr_usage};
use self::fonts::{
    FontSizeDeclaration, FontWeightDeclaration, LineHeightDeclaration, apply_font_declaration,
    extract_property as extract_font_property,
};
use self::selectors::extract_selector;
#[cfg(test)]
pub(crate) use render_tree::resolve_element_tree;
pub use render_tree::{
    build_render_tree, build_render_tree_in_viewport,
    build_render_tree_in_viewport_with_interaction,
    build_render_tree_in_viewport_with_interaction_at_root, build_render_tree_with_interaction,
    build_render_tree_with_interaction_at_root, rebuild_render_tree_with_cached_layout,
};

pub use attributes::{AttributeTextSource, parse_attribute_text_source};
pub use invalidation::StyleInvalidation;
pub use selectors::{
    AncestorSelector, CompoundSelector, ElementRef, PseudoElementKind, Selector,
    SelectorCombinator, SimpleSelector,
};
use selectors::{InteractionDependencies, SelectorAnchor};
pub use visual::{BackgroundLayerDeclaration, ShadowDeclaration};

#[derive(Clone, Debug, Default)]
pub struct Stylesheet {
    pub rules: Vec<StyleRule>,
    index: StylesheetIndex,
}

impl Stylesheet {
    pub fn push(&mut self, rule: StyleRule) {
        self.index.insert(self.rules.len(), &rule);
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

    pub fn matching_rules_with_context<'a>(
        &'a self,
        element: ElementRef<'a>,
        ancestors: &'a [ElementRef<'a>],
        element_path: &'a ElementPath,
        interaction: &'a ElementInteractionState,
    ) -> impl Iterator<Item = &'a StyleRule> {
        self.matching_rules_with_context_and_pseudo(
            element,
            ancestors,
            element_path,
            interaction,
            None,
        )
    }

    pub fn matching_rules_with_context_and_pseudo<'a>(
        &'a self,
        element: ElementRef<'a>,
        ancestors: &'a [ElementRef<'a>],
        element_path: &'a ElementPath,
        interaction: &'a ElementInteractionState,
        pseudo_element: Option<PseudoElementKind>,
    ) -> impl Iterator<Item = &'a StyleRule> {
        self.rules.iter().filter(move |rule| {
            rule.selector.matches_with_ancestors_interaction_and_pseudo(
                element,
                ancestors,
                element_path,
                interaction,
                pseudo_element,
            )
        })
    }

    fn matching_rule_indices_with_context_and_pseudo<'a>(
        &'a self,
        element: ElementRef<'a>,
        ancestors: &'a [ElementRef<'a>],
        element_path: &'a ElementPath,
        interaction: &'a ElementInteractionState,
        pseudo_element: Option<PseudoElementKind>,
    ) -> Vec<usize> {
        let mut candidates = self
            .index
            .collect_candidate_rule_indices(element, interaction);
        if candidates.is_empty() {
            return candidates;
        }

        candidates.sort_unstable();
        candidates.dedup();
        candidates.retain(|&index| {
            let Some(rule) = self.rules.get(index) else {
                return false;
            };
            rule.may_match_pseudo_and_interaction(pseudo_element, interaction)
                && rule.selector.matches_with_ancestors_interaction_and_pseudo(
                    element,
                    ancestors,
                    element_path,
                    interaction,
                    pseudo_element,
                )
        });
        candidates
    }
}

#[derive(Clone, Debug, Default)]
struct StylesheetIndex {
    id_rules: HashMap<String, Vec<usize>>,
    class_rules: HashMap<String, Vec<usize>>,
    tag_rules: HashMap<String, Vec<usize>>,
    interactive_only_hover_rules: Vec<usize>,
    interactive_only_active_rules: Vec<usize>,
    interactive_only_hover_active_rules: Vec<usize>,
    unanchored_rules: Vec<usize>,
    hover_rules: Vec<usize>,
    active_rules: Vec<usize>,
}

impl StylesheetIndex {
    fn insert(&mut self, index: usize, rule: &StyleRule) {
        match rule.selector.primary_anchor() {
            Some(SelectorAnchor::Id(name)) => {
                self.id_rules.entry(name).or_default().push(index);
            }
            Some(SelectorAnchor::Class(name)) => {
                self.class_rules.entry(name).or_default().push(index);
            }
            Some(SelectorAnchor::Tag(name)) => {
                self.tag_rules.entry(name).or_default().push(index);
            }
            None if rule.interaction_dependencies().hover
                && rule.interaction_dependencies().active =>
            {
                self.interactive_only_hover_active_rules.push(index);
            }
            None if rule.interaction_dependencies().hover => {
                self.interactive_only_hover_rules.push(index);
            }
            None if rule.interaction_dependencies().active => {
                self.interactive_only_active_rules.push(index);
            }
            None => self.unanchored_rules.push(index),
        }

        let dependencies = rule.interaction_dependencies();
        if dependencies.hover {
            self.hover_rules.push(index);
        }
        if dependencies.active {
            self.active_rules.push(index);
        }
    }

    fn collect_candidate_rule_indices(
        &self,
        element: ElementRef<'_>,
        interaction: &ElementInteractionState,
    ) -> Vec<usize> {
        let mut capacity = self.unanchored_rules.len();
        if let Some(id) = element.id {
            capacity = capacity.saturating_add(self.id_rules.get(id).map_or(0, Vec::len));
        }
        for class_name in element.classes {
            capacity =
                capacity.saturating_add(self.class_rules.get(class_name).map_or(0, Vec::len));
        }
        capacity = capacity.saturating_add(self.tag_rules.get(element.tag).map_or(0, Vec::len));
        if interaction.hovered.is_some() {
            capacity = capacity
                .saturating_add(self.interactive_only_hover_rules.len())
                .saturating_add(self.interactive_only_hover_active_rules.len());
        }
        if interaction.active.is_some() {
            capacity = capacity
                .saturating_add(self.interactive_only_active_rules.len())
                .saturating_add(self.interactive_only_hover_active_rules.len());
        }

        let mut candidates = Vec::with_capacity(capacity);
        candidates.extend_from_slice(&self.unanchored_rules);
        if let Some(id) = element.id
            && let Some(indices) = self.id_rules.get(id)
        {
            candidates.extend_from_slice(indices);
        }
        for class_name in element.classes {
            if let Some(indices) = self.class_rules.get(class_name) {
                candidates.extend_from_slice(indices);
            }
        }
        if let Some(indices) = self.tag_rules.get(element.tag) {
            candidates.extend_from_slice(indices);
        }
        if interaction.hovered.is_some() {
            candidates.extend_from_slice(&self.interactive_only_hover_rules);
            candidates.extend_from_slice(&self.interactive_only_hover_active_rules);
        }
        if interaction.active.is_some() {
            candidates.extend_from_slice(&self.interactive_only_active_rules);
            candidates.extend_from_slice(&self.interactive_only_hover_active_rules);
        }
        candidates
    }

    fn collect_interaction_rule_indices(&self, changed: InteractionDependencies) -> Vec<usize> {
        let mut candidates = Vec::new();
        if changed.hover {
            candidates.extend_from_slice(&self.hover_rules);
        }
        if changed.active {
            candidates.extend_from_slice(&self.active_rules);
        }
        candidates.sort_unstable();
        candidates.dedup();
        candidates
    }
}

#[derive(Clone, Debug)]
pub struct StyleRule {
    pub selector: Selector,
    pub declarations: Vec<Declaration>,
    custom_declaration_indices: Vec<usize>,
    other_declaration_indices: Vec<usize>,
    interaction_dependencies: InteractionDependencies,
}

impl StyleRule {
    pub fn new(selector: Selector, declarations: Vec<Declaration>) -> Self {
        let mut custom_declaration_indices = Vec::new();
        let mut other_declaration_indices = Vec::new();
        for (index, declaration) in declarations.iter().enumerate() {
            if matches!(declaration, Declaration::CustomProperty { .. }) {
                custom_declaration_indices.push(index);
            } else {
                other_declaration_indices.push(index);
            }
        }
        Self {
            interaction_dependencies: selector.interaction_dependencies(),
            selector,
            declarations,
            custom_declaration_indices,
            other_declaration_indices,
        }
    }

    fn interaction_dependencies(&self) -> InteractionDependencies {
        self.interaction_dependencies
    }

    fn may_match_pseudo_and_interaction(
        &self,
        pseudo_element: Option<PseudoElementKind>,
        interaction: &ElementInteractionState,
    ) -> bool {
        self.selector.pseudo_element == pseudo_element
            && (!self.interaction_dependencies.hover || interaction.hovered.is_some())
            && (!self.interaction_dependencies.active || interaction.active.is_some())
    }

    fn apply_custom_declarations(&self, style: &mut Style, position_explicit: &mut bool) {
        for &index in &self.custom_declaration_indices {
            apply_declaration(style, position_explicit, &self.declarations[index]);
        }
    }

    fn apply_other_declarations(&self, style: &mut Style, position_explicit: &mut bool) {
        for &index in &self.other_declaration_indices {
            apply_declaration(style, position_explicit, &self.declarations[index]);
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Declaration {
    Content(Option<AttributeTextSource>),
    CustomProperty {
        name: String,
        value: String,
    },
    VariableDependentProperty {
        property_name: String,
        value_css: String,
    },
    TransitionProperties(Vec<TransitionPropertyName>),
    TransitionDurations(Vec<f32>),
    TransitionDelays(Vec<f32>),
    TransitionTimingFunctions(Vec<TransitionTimingFunction>),
    Background(Color),
    BackgroundLayers(Vec<BackgroundLayerDeclaration>),
    Foreground(Color),
    SvgFill(SvgPaint),
    SvgStroke(SvgPaint),
    SvgStrokeWidth(f32),
    FontFamilies(Vec<cssimpler_core::fonts::FontFamily>),
    FontSize(FontSizeDeclaration),
    FontWeight(FontWeightDeclaration),
    FontStyle(cssimpler_core::fonts::FontStyle),
    LineHeight(LineHeightDeclaration),
    LetterSpacing(f32),
    TextTransform(TextTransform),
    CornerTopLeft(f32),
    CornerTopRight(f32),
    CornerBottomRight(f32),
    CornerBottomLeft(f32),
    BorderTopWidth(f32),
    BorderRightWidth(f32),
    BorderBottomWidth(f32),
    BorderLeftWidth(f32),
    BorderLineStyle(BorderLineStyle),
    BorderColor(Option<Color>),
    BoxShadows(Vec<ShadowDeclaration>),
    TextShadows(Vec<ShadowDeclaration>),
    FilterDropShadows(Vec<ShadowDeclaration>),
    BackdropBlur(f32),
    TextStrokeWidth(f32),
    TextStrokeColor(Option<Color>),
    TransformOperations(Vec<TransformOperation>),
    TransformOrigin(TransformOrigin),
    Perspective(Option<f32>),
    TransformStyle(TransformStyleMode),
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
    MinWidth(Dimension),
    MinHeight(Dimension),
    MaxWidth(Dimension),
    MaxHeight(Dimension),
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
    resolve_style_with_interaction(
        element,
        stylesheet,
        &ElementInteractionState::default(),
        &ElementPath::root(0),
    )
}

pub fn resolve_style_with_interaction(
    element: &ElementNode,
    stylesheet: &Stylesheet,
    interaction: &ElementInteractionState,
    element_path: &ElementPath,
) -> Style {
    resolve_style_target(
        element,
        stylesheet,
        element.style.clone(),
        None,
        None,
        None,
        &[],
        interaction,
        element_path,
        None,
    )
}

fn resolve_style_target(
    element: &ElementNode,
    stylesheet: &Stylesheet,
    mut resolved: Style,
    inherited_text: Option<&TextStyle>,
    inherited_foreground: Option<Color>,
    inherited_custom_properties: Option<&CustomProperties>,
    ancestors: &[ElementRef<'_>],
    interaction: &ElementInteractionState,
    element_path: &ElementPath,
    pseudo_element: Option<PseudoElementKind>,
) -> Style {
    if let Some(inherited_text) = inherited_text {
        resolved.visual.text = inherited_text.clone();
    }
    if let Some(inherited_foreground) = inherited_foreground {
        resolved.visual.foreground = inherited_foreground;
    }
    if let Some(inherited_custom_properties) = inherited_custom_properties {
        custom_properties::inherit(&mut resolved, inherited_custom_properties);
    }
    let mut position_explicit = resolved.layout.taffy.position != TaffyPosition::Relative;
    let element_ref = ElementRef::from(element);
    let matching_rule_indices = stylesheet.matching_rule_indices_with_context_and_pseudo(
        element_ref,
        ancestors,
        element_path,
        interaction,
        pseudo_element,
    );

    for &rule_index in &matching_rule_indices {
        if let Some(rule) = stylesheet.rules.get(rule_index) {
            rule.apply_custom_declarations(&mut resolved, &mut position_explicit);
        }
    }

    for &rule_index in &matching_rule_indices {
        if let Some(rule) = stylesheet.rules.get(rule_index) {
            rule.apply_other_declarations(&mut resolved, &mut position_explicit);
        }
    }

    resolved
}

pub fn to_taffy(style: &LayoutStyle) -> TaffyStyle {
    style.taffy.clone()
}

fn extract_declarations(block: &DeclarationBlock<'_>) -> Result<Vec<Declaration>, StyleError> {
    let mut declarations = Vec::new();

    for (property, _important) in block.iter() {
        declarations.extend(extract_property(property)?);
    }

    Ok(declarations)
}

fn extract_property(property: &Property<'_>) -> Result<Vec<Declaration>, StyleError> {
    reject_unsupported_attr_usage(property)?;

    if let Some(declarations) = custom_properties::extract_property(property) {
        return declarations;
    }

    if let Some(declarations) = variable_resolution::extract_property(property) {
        return declarations;
    }

    if let Some(declarations) = extract_font_property(property) {
        return declarations;
    }

    if let Some(declarations) = visual::extract_property(property) {
        return declarations;
    }

    if let Some(declarations) = transitions::extract_property(property) {
        return declarations;
    }

    match property {
        Property::Custom(custom) if custom.name.as_ref() == "content" => {
            Ok(vec![Declaration::Content(parse_content_text_source(
                &custom.value,
            )?)])
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
        Property::Inset(inset) => Ok(vec![
            Declaration::InsetTop(length_percentage_auto_to_taffy(&inset.top)?),
            Declaration::InsetRight(length_percentage_auto_to_taffy(&inset.right)?),
            Declaration::InsetBottom(length_percentage_auto_to_taffy(&inset.bottom)?),
            Declaration::InsetLeft(length_percentage_auto_to_taffy(&inset.left)?),
        ]),
        Property::Width(size) => Ok(vec![Declaration::Width(dimension_from_css_size(size)?)]),
        Property::Height(size) => Ok(vec![Declaration::Height(dimension_from_css_size(size)?)]),
        Property::MinWidth(size) => Ok(vec![Declaration::MinWidth(dimension_from_css_size(size)?)]),
        Property::MinHeight(size) => {
            Ok(vec![Declaration::MinHeight(dimension_from_css_size(size)?)])
        }
        Property::MaxWidth(size) => {
            Ok(vec![Declaration::MaxWidth(dimension_from_css_max_size(size)?)])
        }
        Property::MaxHeight(size) => {
            Ok(vec![Declaration::MaxHeight(dimension_from_css_max_size(size)?)])
        }
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

fn attribute_text_source_to_core(
    source: AttributeTextSource,
) -> cssimpler_core::GeneratedTextSource {
    match source {
        AttributeTextSource::Literal(value) => cssimpler_core::GeneratedTextSource::Literal(value),
        AttributeTextSource::Attribute(name) => {
            cssimpler_core::GeneratedTextSource::Attribute(name)
        }
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
        LengthPercentage::Dimension(length) => {
            if let Some(value) = length.to_px() {
                Ok(TaffyLengthPercentage::Length(value as f32))
            } else if let Some(value) = viewport_length_to_percent(length) {
                Ok(TaffyLengthPercentage::Percent(value))
            } else {
                Err(StyleError::UnsupportedValue(format!("{value:?}")))
            }
        }
        LengthPercentage::Percentage(percentage) => {
            Ok(TaffyLengthPercentage::Percent(percentage.0))
        }
        _ => Err(StyleError::UnsupportedValue(format!("{value:?}"))),
    }
}

fn viewport_length_to_percent(value: &LengthValue) -> Option<f32> {
    let percent = match value {
        LengthValue::Vw(value)
        | LengthValue::Lvw(value)
        | LengthValue::Svw(value)
        | LengthValue::Dvw(value)
        | LengthValue::Vh(value)
        | LengthValue::Lvh(value)
        | LengthValue::Svh(value)
        | LengthValue::Dvh(value)
        | LengthValue::Vi(value)
        | LengthValue::Svi(value)
        | LengthValue::Lvi(value)
        | LengthValue::Dvi(value)
        | LengthValue::Vb(value)
        | LengthValue::Svb(value)
        | LengthValue::Lvb(value)
        | LengthValue::Dvb(value)
        | LengthValue::Vmin(value)
        | LengthValue::Svmin(value)
        | LengthValue::Lvmin(value)
        | LengthValue::Dvmin(value)
        | LengthValue::Vmax(value)
        | LengthValue::Svmax(value)
        | LengthValue::Lvmax(value)
        | LengthValue::Dvmax(value) => *value,
        _ => return None,
    };

    Some(percent / 100.0)
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

fn dimension_from_css_max_size(size: &CssMaxSize) -> Result<Dimension, StyleError> {
    match size {
        CssMaxSize::None => Ok(Dimension::Auto),
        CssMaxSize::LengthPercentage(value) => Ok(length_percentage_to_taffy(value)?.into()),
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
    if custom_properties::apply_declaration(style, declaration) {
        return;
    }

    if let Some(declarations) =
        variable_resolution::resolve_declaration(declaration, &style.custom_properties)
    {
        let declarations = declarations.unwrap_or_else(|error| panic!("{error}"));
        for declaration in declarations {
            apply_declaration(style, position_explicit, &declaration);
        }
        return;
    }

    if apply_font_declaration(style, declaration) {
        return;
    }

    if visual::apply_declaration(style, declaration) {
        return;
    }

    if transitions::apply_declaration(style, declaration) {
        return;
    }

    match declaration {
        Declaration::Content(value) => {
            style.generated_text = value.clone().map(attribute_text_source_to_core);
        }
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
        Declaration::MinWidth(value) => style.layout.taffy.min_size.width = *value,
        Declaration::MinHeight(value) => style.layout.taffy.min_size.height = *value,
        Declaration::MaxWidth(value) => style.layout.taffy.max_size.width = *value,
        Declaration::MaxHeight(value) => style.layout.taffy.max_size.height = *value,
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use cssimpler_core::{
        AnglePercentageValue, BackgroundLayer, CircleRadius, Color, ConicGradient, ElementNode,
        GradientDirection, GradientHorizontal, GradientInterpolation, GradientPoint, GradientStop,
        LengthPercentageValue, LinearGradient, Node, RadialShape, ScrollbarWidth, ShapeExtent,
        TransformMatrix3d, TransformOperation, TransformOrigin, TransformStyleMode,
        TransitionPropertyName, TransitionTimingFunction, fonts::TextTransform,
    };
    use taffy::prelude::{
        AlignItems as TaffyAlignItems, Dimension, Display as TaffyDisplay,
        FlexDirection as TaffyFlexDirection, JustifyContent as TaffyJustifyContent,
        LengthPercentage as TaffyLengthPercentage,
        LengthPercentageAuto as TaffyLengthPercentageAuto, Position as TaffyPosition,
    };

    use super::{
        Declaration, ElementRef, Selector, ShadowDeclaration, StyleError, StyleRule, Stylesheet,
        build_render_tree, build_render_tree_in_viewport, parse_stylesheet,
        rebuild_render_tree_with_cached_layout, resolve_element_tree, resolve_style, to_taffy,
    };

    #[test]
    fn matching_rules_are_returned_in_insertion_order() {
        let classes = vec!["card".to_string()];
        let attributes = BTreeMap::new();
        let mut stylesheet = Stylesheet::default();
        stylesheet.push(StyleRule::new(
            Selector::class("card"),
            vec![Declaration::Width(Dimension::Length(300.0))],
        ));
        stylesheet.push(StyleRule::new(
            Selector::tag("p"),
            vec![Declaration::Height(Dimension::Length(40.0))],
        ));

        let matching: Vec<_> = stylesheet
            .matching_rules(ElementRef {
                tag: "div",
                id: None,
                classes: &classes,
                attributes: &attributes,
            })
            .collect();

        assert_eq!(matching.len(), 1);
    }

    #[test]
    fn indexed_style_resolution_preserves_anchor_and_custom_property_semantics() {
        let stylesheet = parse_stylesheet(
            ".unused { color: #ff0000; }
             [data-role=\"cta\"] { height: 24px; }
             .target { color: var(--accent); }
             button { width: 120px; }
             #hero { --accent: #2563eb; }",
        )
        .expect("stylesheet should parse");
        let element = ElementNode::new("button")
            .with_id("hero")
            .with_class("target")
            .with_attribute("data-role", "cta");

        let resolved = resolve_style(&element, &stylesheet);

        assert_eq!(resolved.layout.taffy.size.width, Dimension::Length(120.0));
        assert_eq!(resolved.layout.taffy.size.height, Dimension::Length(24.0));
        assert_eq!(resolved.visual.foreground, Color::rgb(37, 99, 235));
        assert_eq!(resolved.custom_properties.get("--accent"), Some("#2563eb"));
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
    fn inset_shorthand_expands_through_existing_absolute_position_pipeline() {
        let stylesheet =
            parse_stylesheet(".card { inset: 0; }").expect("inset shorthand should parse");
        let element = Node::element("div").with_class("card");
        let resolved = resolve_style(&element, &stylesheet);

        assert_eq!(resolved.layout.taffy.position, TaffyPosition::Absolute);
        assert_eq!(
            resolved.layout.taffy.inset.top,
            TaffyLengthPercentageAuto::Length(0.0)
        );
        assert_eq!(
            resolved.layout.taffy.inset.right,
            TaffyLengthPercentageAuto::Length(0.0)
        );
        assert_eq!(
            resolved.layout.taffy.inset.bottom,
            TaffyLengthPercentageAuto::Length(0.0)
        );
        assert_eq!(
            resolved.layout.taffy.inset.left,
            TaffyLengthPercentageAuto::Length(0.0)
        );
    }

    #[test]
    fn parser_and_resolution_support_min_and_max_sizes() {
        let stylesheet = parse_stylesheet(
            ".panel {
                min-width: 120px;
                min-height: 50px;
                max-width: 180px;
                max-height: 90px;
            }",
        )
        .expect("min/max size declarations should parse");
        let element = Node::element("section").with_class("panel");
        let resolved = resolve_style(&element, &stylesheet);

        assert_eq!(
            resolved.layout.taffy.min_size.width,
            Dimension::Length(120.0)
        );
        assert_eq!(
            resolved.layout.taffy.min_size.height,
            Dimension::Length(50.0)
        );
        assert_eq!(
            resolved.layout.taffy.max_size.width,
            Dimension::Length(180.0)
        );
        assert_eq!(
            resolved.layout.taffy.max_size.height,
            Dimension::Length(90.0)
        );
    }

    #[test]
    fn parser_and_resolution_support_viewport_size_units() {
        let stylesheet = parse_stylesheet(
            ".panel {
                width: 100vw;
                height: 100vh;
                min-width: 50dvw;
                min-height: 50dvh;
                max-width: 100vw;
                max-height: 100vh;
            }",
        )
        .expect("viewport size declarations should parse");
        let element = Node::element("section").with_class("panel");
        let resolved = resolve_style(&element, &stylesheet);

        assert_eq!(resolved.layout.taffy.size.width, Dimension::Percent(1.0));
        assert_eq!(resolved.layout.taffy.size.height, Dimension::Percent(1.0));
        assert_eq!(resolved.layout.taffy.min_size.width, Dimension::Percent(0.5));
        assert_eq!(resolved.layout.taffy.min_size.height, Dimension::Percent(0.5));
        assert_eq!(resolved.layout.taffy.max_size.width, Dimension::Percent(1.0));
        assert_eq!(resolved.layout.taffy.max_size.height, Dimension::Percent(1.0));
    }

    #[test]
    fn parser_supports_text_presentation_controls() {
        let stylesheet =
            parse_stylesheet(".label { letter-spacing: 2px; text-transform: uppercase; }")
                .expect("text presentation stylesheet should parse");

        assert!(
            stylesheet.rules[0]
                .declarations
                .contains(&Declaration::LetterSpacing(2.0))
        );
        assert!(
            stylesheet.rules[0]
                .declarations
                .contains(&Declaration::TextTransform(TextTransform::Uppercase))
        );
    }

    #[test]
    fn parser_supports_transform_controls() {
        let stylesheet = parse_stylesheet(
            ".card {
                transform: translate(12px, 25%) rotate(45deg) scale(150%, 0.5);
                transform-origin: right 10px bottom 20%;
            }",
        )
        .expect("transform stylesheet should parse");

        assert!(
            stylesheet.rules[0]
                .declarations
                .contains(&Declaration::TransformOperations(vec![
                    TransformOperation::Translate {
                        x: LengthPercentageValue::from_px(12.0),
                        y: LengthPercentageValue::from_fraction(0.25),
                    },
                    TransformOperation::Rotate { degrees: 45.0 },
                    TransformOperation::Scale { x: 1.5, y: 0.5 },
                ]))
        );
        assert!(
            stylesheet.rules[0]
                .declarations
                .contains(&Declaration::TransformOrigin(TransformOrigin {
                    x: LengthPercentageValue {
                        px: -10.0,
                        fraction: 1.0,
                    },
                    y: LengthPercentageValue {
                        px: 0.0,
                        fraction: 0.8,
                    },
                }))
        );
    }

    #[test]
    fn resolve_style_applies_transform_data_without_changing_layout_defaults() {
        let stylesheet = parse_stylesheet(
            ".card {
                transform: translate(10px, 20px) rotate(90deg);
                transform-origin: 25% 75%;
            }",
        )
        .expect("transform stylesheet should parse");
        let element = ElementNode::new("div").with_class("card");

        let resolved = resolve_style(&element, &stylesheet);

        assert_eq!(
            resolved.visual.transform.operations,
            vec![
                TransformOperation::Translate {
                    x: LengthPercentageValue::from_px(10.0),
                    y: LengthPercentageValue::from_px(20.0),
                },
                TransformOperation::Rotate { degrees: 90.0 },
            ]
        );
        assert_eq!(
            resolved.visual.transform.origin,
            TransformOrigin {
                x: LengthPercentageValue::from_fraction(0.25),
                y: LengthPercentageValue::from_fraction(0.75),
            }
        );
        assert_eq!(resolved.layout.taffy.position, TaffyPosition::Relative);
    }

    #[test]
    fn parser_supports_3d_transform_controls() {
        let stylesheet = parse_stylesheet(
            ".card {
                transform: translate3d(12px, 25%, 30px) rotateX(15deg) rotateY(-20deg) rotateZ(45deg);
                perspective: 800px;
                transform-style: preserve-3d;
            }",
        )
        .expect("3d transform stylesheet should parse");

        assert!(
            stylesheet.rules[0]
                .declarations
                .contains(&Declaration::TransformOperations(vec![
                    TransformOperation::Translate {
                        x: LengthPercentageValue::from_px(12.0),
                        y: LengthPercentageValue::from_fraction(0.25),
                    },
                    TransformOperation::TranslateZ { z: 30.0 },
                    TransformOperation::RotateX { degrees: 15.0 },
                    TransformOperation::RotateY { degrees: -20.0 },
                    TransformOperation::RotateZ { degrees: 45.0 },
                ]))
        );
        assert!(
            stylesheet.rules[0]
                .declarations
                .contains(&Declaration::Perspective(Some(800.0)))
        );
        assert!(
            stylesheet.rules[0]
                .declarations
                .contains(&Declaration::TransformStyle(TransformStyleMode::Preserve3d))
        );
    }

    #[test]
    fn resolve_style_applies_perspective_and_transform_style() {
        let stylesheet = parse_stylesheet(
            ".card {
                perspective: 640px;
                transform-style: preserve-3d;
                transform: translateZ(24px) rotateY(30deg);
            }",
        )
        .expect("3d transform stylesheet should parse");
        let element = ElementNode::new("div").with_class("card");

        let resolved = resolve_style(&element, &stylesheet);

        assert_eq!(resolved.visual.perspective, Some(640.0));
        assert_eq!(
            resolved.visual.transform_style,
            TransformStyleMode::Preserve3d
        );
        assert_eq!(
            resolved.visual.transform.operations,
            vec![
                TransformOperation::TranslateZ { z: 24.0 },
                TransformOperation::RotateY { degrees: 30.0 },
            ]
        );
    }

    #[test]
    fn parser_supports_extended_3d_transform_functions() {
        let stylesheet = parse_stylesheet(
            ".card {
                transform:
                    scaleZ(0.75)
                    scale3d(1.02, 1.03, 1.04)
                    rotate3d(1, 2, 3, 30deg)
                    perspective(600px)
                    matrix3d(
                        1, 0, 0, 0,
                        0, 1, 0, 0,
                        0, 0, 1, 0,
                        12, 24, 36, 1
                    );
            }",
        )
        .expect("extended 3d transform stylesheet should parse");

        assert!(
            stylesheet.rules[0]
                .declarations
                .contains(&Declaration::TransformOperations(vec![
                    TransformOperation::Matrix3d {
                        matrix: TransformMatrix3d::scale(1.0, 1.0, 0.75),
                    },
                    TransformOperation::Matrix3d {
                        matrix: TransformMatrix3d::scale(1.02, 1.03, 1.04),
                    },
                    TransformOperation::Matrix3d {
                        matrix: TransformMatrix3d::rotate(1.0, 2.0, 3.0, 30.0),
                    },
                    TransformOperation::Matrix3d {
                        matrix: TransformMatrix3d::perspective(600.0)
                            .expect("perspective matrix should build"),
                    },
                    TransformOperation::Matrix3d {
                        matrix: TransformMatrix3d {
                            m11: 1.0,
                            m12: 0.0,
                            m13: 0.0,
                            m14: 12.0,
                            m21: 0.0,
                            m22: 1.0,
                            m23: 0.0,
                            m24: 24.0,
                            m31: 0.0,
                            m32: 0.0,
                            m33: 1.0,
                            m34: 36.0,
                            m41: 0.0,
                            m42: 0.0,
                            m43: 0.0,
                            m44: 1.0,
                        },
                    },
                ]))
        );
    }

    #[test]
    fn resolve_style_applies_extended_3d_transform_functions() {
        let stylesheet = parse_stylesheet(
            ".card {
                transform:
                    scale3d(1.02, 1.03, 1.04)
                    rotate3d(1, 2, 3, 30deg)
                    perspective(600px);
            }",
        )
        .expect("extended 3d transform stylesheet should parse");
        let element = ElementNode::new("div").with_class("card");

        let resolved = resolve_style(&element, &stylesheet);

        assert_eq!(
            resolved.visual.transform.operations,
            vec![
                TransformOperation::Matrix3d {
                    matrix: TransformMatrix3d::scale(1.02, 1.03, 1.04),
                },
                TransformOperation::Matrix3d {
                    matrix: TransformMatrix3d::rotate(1.0, 2.0, 3.0, 30.0),
                },
                TransformOperation::Matrix3d {
                    matrix: TransformMatrix3d::perspective(600.0)
                        .expect("perspective matrix should build"),
                },
            ]
        );
    }

    #[test]
    fn parser_supports_percentage_corner_radius() {
        let stylesheet = parse_stylesheet(
            ".card {
                border-top-right-radius: 100%;
                border-bottom-left-radius: 80%;
            }",
        )
        .expect("percentage corner radius stylesheet should parse");

        assert!(
            stylesheet.rules[0]
                .declarations
                .contains(&Declaration::CornerTopRight(-1.0))
        );
        assert!(
            stylesheet.rules[0]
                .declarations
                .contains(&Declaration::CornerBottomLeft(-0.8))
        );
    }

    #[test]
    fn text_transform_stylesheet_matches_pretransformed_layout() {
        let stylesheet = parse_stylesheet(".label { text-transform: uppercase; }")
            .expect("text-transform stylesheet should parse");
        let transformed_tree = Node::element("span")
            .with_class("label")
            .with_child(Node::text("Straße"))
            .into();
        let literal_tree = Node::element("span")
            .with_child(Node::text("STRASSE"))
            .into();

        let transformed_scene = build_render_tree(&transformed_tree, &stylesheet);
        let literal_scene = build_render_tree(&literal_tree, &Stylesheet::default());

        assert!((transformed_scene.layout.width - literal_scene.layout.width).abs() < 0.01);
        assert_eq!(transformed_scene.layout.height, literal_scene.layout.height);
    }

    #[test]
    fn letter_spacing_stylesheet_changes_measured_width() {
        let stylesheet = parse_stylesheet(".label { letter-spacing: 2px; }")
            .expect("letter-spacing stylesheet should parse");
        let tree = Node::element("span")
            .with_class("label")
            .with_child(Node::text("ABCD"))
            .into();
        let baseline_scene = build_render_tree(&tree, &Stylesheet::default());
        let spaced_scene = build_render_tree(&tree, &stylesheet);

        assert!((spaced_scene.layout.width - (baseline_scene.layout.width + 6.0)).abs() < 0.01);
    }

    #[test]
    fn parser_supports_text_effect_controls() {
        let stylesheet = parse_stylesheet(
            ".label {
                -webkit-text-stroke: 2px #ff6600;
                text-shadow: 1px 2px 3px rgba(15, 23, 42, 0.5);
                filter: drop-shadow(4px 5px 6px #112233);
            }",
        )
        .expect("text effect stylesheet should parse");

        assert!(
            stylesheet.rules[0]
                .declarations
                .contains(&Declaration::TextStrokeWidth(2.0))
        );
        assert!(
            stylesheet.rules[0]
                .declarations
                .contains(&Declaration::TextStrokeColor(Some(Color::rgb(255, 102, 0))))
        );
        assert!(
            stylesheet.rules[0]
                .declarations
                .contains(&Declaration::TextShadows(vec![ShadowDeclaration {
                    color: Some(Color::rgba(15, 23, 42, 128)),
                    offset_x: 1.0,
                    offset_y: 2.0,
                    blur_radius: 3.0,
                    spread: 0.0,
                }]))
        );
        assert!(
            stylesheet.rules[0]
                .declarations
                .contains(&Declaration::FilterDropShadows(vec![ShadowDeclaration {
                    color: Some(Color::rgb(17, 34, 51)),
                    offset_x: 4.0,
                    offset_y: 5.0,
                    blur_radius: 6.0,
                    spread: 0.0,
                }]))
        );
    }

    #[test]
    fn parser_supports_backdrop_blur_and_vendor_alias() {
        let stylesheet = parse_stylesheet(
            ".glass {
                -webkit-backdrop-filter: blur(8px);
                backdrop-filter: blur(8px);
            }",
        )
        .expect("backdrop blur stylesheet should parse");
        let blur_count = stylesheet.rules[0]
            .declarations
            .iter()
            .filter(|declaration| match declaration {
                Declaration::BackdropBlur(radius) => (*radius - 8.0).abs() < f32::EPSILON,
                _ => false,
            })
            .count();

        assert_eq!(blur_count, 2);

        let tree = Node::element("div").with_class("glass").into();
        let scene = build_render_tree(&tree, &stylesheet);

        assert_eq!(scene.style.backdrop_blur_radius, 8.0);
    }

    #[test]
    fn unsupported_backdrop_filter_values_fail_clearly() {
        let error = parse_stylesheet(".glass { backdrop-filter: saturate(1.2); }")
            .expect_err("unsupported backdrop-filter should fail clearly");

        assert!(matches!(
            error,
            StyleError::UnsupportedValue(message)
                if message.contains("only blur() is supported")
        ));
    }

    #[test]
    fn parser_supports_transition_shorthand_metadata() {
        let stylesheet = parse_stylesheet(
            ".button { transition: color 180ms linear, width 320ms ease-in-out; }",
        )
        .expect("transition stylesheet should parse");

        assert!(
            stylesheet.rules[0]
                .declarations
                .contains(&Declaration::TransitionProperties(vec![
                    TransitionPropertyName::Property("color".to_string()),
                    TransitionPropertyName::Property("width".to_string()),
                ]))
        );
        assert!(
            stylesheet.rules[0]
                .declarations
                .contains(&Declaration::TransitionDurations(vec![0.18, 0.32]))
        );
        assert!(stylesheet.rules[0].declarations.contains(
            &Declaration::TransitionTimingFunctions(vec![
                TransitionTimingFunction::Linear,
                TransitionTimingFunction::EaseInOut,
            ])
        ));
    }

    #[test]
    fn unsupported_filter_functions_fail_clearly() {
        let error = parse_stylesheet(".badge { filter: blur(2px); }")
            .expect_err("unsupported filter should fail clearly");

        assert!(matches!(
            error,
            StyleError::UnsupportedValue(message)
                if message.contains("only drop-shadow() is supported")
        ));
    }

    #[test]
    fn parser_supports_tag_selectors() {
        let stylesheet = parse_stylesheet("button { width: 120px; height: 32px; color: #2563eb; }")
            .expect("tag selector stylesheet should parse");

        assert_eq!(stylesheet.rules.len(), 1);
        assert_eq!(stylesheet.rules[0].selector, Selector::tag("button"));
        assert!(
            stylesheet.rules[0]
                .declarations
                .contains(&Declaration::Width(Dimension::Length(120.0)))
        );
    }

    #[test]
    fn combinator_selectors_participate_in_render_tree_style_resolution() {
        let stylesheet = parse_stylesheet(
            ".button .hover-text { color: #2563eb; }
             .button > .hover-text { background-color: #0f172a; }",
        )
        .expect("combinator selectors should parse");
        let tree = Node::element("button")
            .with_class("button")
            .with_child(Node::element("span").with_class("hover-text").into())
            .with_child(
                Node::element("div")
                    .with_class("wrapper")
                    .with_child(Node::element("span").with_class("hover-text").into())
                    .into(),
            )
            .into();
        let scene = build_render_tree(&tree, &stylesheet);

        assert_eq!(scene.children[0].style.foreground, Color::rgb(37, 99, 235));
        assert_eq!(
            scene.children[0].style.background,
            Some(Color::rgb(15, 23, 42))
        );
        assert_eq!(
            scene.children[1].children[0].style.foreground,
            Color::rgb(37, 99, 235)
        );
        assert_eq!(scene.children[1].children[0].style.background, None);
    }

    #[test]
    fn generated_content_pseudo_elements_render_before_and_after_text() {
        let stylesheet = parse_stylesheet(
            ".badge::before { content: \"[\"; color: #2563eb; }
             .badge::after { content: attr(data-label); color: #f97316; }",
        )
        .expect("generated content stylesheet should parse");
        let tree = Node::element("div")
            .with_class("badge")
            .with_attribute("data-label", "done")
            .into();
        let scene = build_render_tree(&tree, &stylesheet);

        assert_eq!(
            text_nodes(&scene),
            vec!["[".to_string(), "done".to_string()]
        );
        assert_eq!(scene.children[0].style.foreground, Color::rgb(37, 99, 235));
        assert_eq!(scene.children[1].style.foreground, Color::rgb(249, 115, 22));
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
    fn resolved_children_inherit_custom_properties_from_ancestors() {
        let stylesheet = parse_stylesheet(
            ".button { --animation-color: #2563eb; }
             .label { --animation-offset: 12px; }
             .label.active { --animation-color: #22c55e; }",
        )
        .expect("custom property stylesheet should parse");
        let root = Node::element("div")
            .with_class("button")
            .with_child(Node::element("span").with_class("label").into())
            .with_child(
                Node::element("span")
                    .with_class("label")
                    .with_class("active")
                    .into(),
            );
        let resolved = resolve_element_tree(
            &root,
            &stylesheet,
            None,
            None,
            None,
            &[],
            &cssimpler_core::ElementInteractionState::default(),
            &cssimpler_core::ElementPath::root(0),
        );

        assert_eq!(
            resolved.style.custom_properties.get("--animation-color"),
            Some("#2563eb")
        );
        assert_eq!(
            resolved.children[0]
                .style
                .custom_properties
                .get("--animation-color"),
            Some("#2563eb")
        );
        assert_eq!(
            resolved.children[0]
                .style
                .custom_properties
                .get("--animation-offset"),
            Some("12px")
        );
        assert_eq!(
            resolved.children[1]
                .style
                .custom_properties
                .get("--animation-color"),
            Some("#22c55e")
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
                interpolation: GradientInterpolation::Oklab,
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
                interpolation: GradientInterpolation::Oklab,
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
                interpolation: GradientInterpolation::Oklab,
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
    fn render_tree_carries_prepared_text_layout_for_paint() {
        let stylesheet = parse_stylesheet(
            ".button {
                width: 90px;
                height: 40px;
                padding: 0 8px;
                letter-spacing: 1px;
            }",
        )
        .expect("stylesheet should parse");
        let tree = Node::element("button")
            .with_class("button")
            .with_child(Node::text("wrap this text"))
            .into();
        let scene = build_render_tree(&tree, &stylesheet);
        let prepared = scene
            .text_layout
            .as_ref()
            .expect("text nodes should carry prepared paint layout");

        assert_eq!(prepared.wrap_width, Some(74.0));
        assert_eq!(
            prepared.layout,
            cssimpler_core::fonts::layout_text_block(
                "wrap this text",
                &scene.style.text,
                prepared.wrap_width,
            )
        );
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

        assert!(scene.handlers.click.is_some());
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

    #[test]
    fn viewport_layout_supports_percentage_sized_roots() {
        let stylesheet = parse_stylesheet(
            "#app {
                display: flex;
                width: 100%;
                height: 100%;
                padding: 12px;
                background-color: #ffffff;
            }
            .panel {
                flex-grow: 1;
                height: 40px;
                background-color: #0f172a;
            }",
        )
        .expect("viewport stylesheet should parse");
        let tree = Node::element("div")
            .with_id("app")
            .with_child(Node::element("section").with_class("panel").into())
            .into();
        let scene = build_render_tree_in_viewport(&tree, &stylesheet, 640, 360);

        assert_eq!(scene.layout.width, 640.0);
        assert_eq!(scene.layout.height, 360.0);
        assert_eq!(scene.children[0].layout.x, 12.0);
        assert_eq!(scene.children[0].layout.y, 12.0);
        assert_eq!(scene.children[0].layout.width, 616.0);
    }

    #[test]
    fn viewport_layout_auto_stretches_unsized_roots() {
        let stylesheet = parse_stylesheet(
            "#app {
                display: flex;
                padding: 8px;
                background-color: #ffffff;
            }
            .panel {
                width: 120px;
                height: 40px;
                background-color: #0f172a;
            }",
        )
        .expect("viewport stylesheet should parse");
        let tree = Node::element("div")
            .with_id("app")
            .with_child(Node::element("section").with_class("panel").into())
            .into();
        let scene = build_render_tree_in_viewport(&tree, &stylesheet, 640, 360);

        assert_eq!(scene.layout.width, 640.0);
        assert_eq!(scene.layout.height, 360.0);
        assert_eq!(scene.children[0].layout.x, 8.0);
        assert_eq!(scene.children[0].layout.y, 8.0);
    }

    #[test]
    fn viewport_layout_preserves_explicit_root_sizes() {
        let stylesheet = parse_stylesheet(
            "#app {
                width: 320px;
                height: 180px;
                padding: 8px;
                background-color: #ffffff;
            }
            .panel {
                width: 120px;
                height: 40px;
                background-color: #0f172a;
            }",
        )
        .expect("viewport stylesheet should parse");
        let tree = Node::element("div")
            .with_id("app")
            .with_child(Node::element("section").with_class("panel").into())
            .into();
        let scene = build_render_tree_in_viewport(&tree, &stylesheet, 640, 360);

        assert_eq!(scene.layout.width, 320.0);
        assert_eq!(scene.layout.height, 180.0);
        assert_eq!(scene.children[0].layout.x, 8.0);
        assert_eq!(scene.children[0].layout.y, 8.0);
    }

    #[test]
    fn cached_layout_rerender_updates_visuals_without_rebuilding_layout() {
        let stylesheet = parse_stylesheet(
            "#app { width: 120px; height: 40px; }
             #panel { width: 80px; height: 20px; color: #111111; }
             #panel.hot { color: #2563eb; }",
        )
        .expect("stylesheet should parse");
        let base = Node::element("div").with_id("app").with_child(
            Node::element("section")
                .with_id("panel")
                .with_child(Node::text("value"))
                .into(),
        );
        let next = Node::element("section")
            .with_id("panel")
            .with_class("hot")
            .with_child(Node::text("value"))
            .into();
        let scene = build_render_tree(&base.into(), &stylesheet);
        let rerendered = rebuild_render_tree_with_cached_layout(
            &next,
            &stylesheet,
            &cssimpler_core::ElementInteractionState::default(),
            &cssimpler_core::ElementPath::root(0).with_child(0),
            &scene.children[0],
        )
        .expect("cached layout rerender should succeed");

        assert_eq!(rerendered.layout, scene.children[0].layout);
        assert_eq!(rerendered.style.foreground, Color::rgb(37, 99, 235));
        assert_eq!(rerendered.text_layout, scene.children[0].text_layout);
        assert_eq!(text_nodes(&rerendered), vec!["value".to_string()]);
    }

    #[test]
    fn cached_layout_rerender_falls_back_when_structure_changes() {
        let stylesheet = parse_stylesheet(
            "#app { width: 120px; height: 40px; }
             #panel { width: 80px; height: 20px; color: #111111; }",
        )
        .expect("stylesheet should parse");
        let base = Node::element("div").with_id("app").with_child(
            Node::element("section")
                .with_id("panel")
                .with_child(Node::text("value"))
                .into(),
        );
        let next = Node::element("section")
            .with_id("panel")
            .with_child(Node::element("span").with_child(Node::text("value")).into())
            .into();
        let scene = build_render_tree(&base.into(), &stylesheet);

        assert!(
            rebuild_render_tree_with_cached_layout(
                &next,
                &stylesheet,
                &cssimpler_core::ElementInteractionState::default(),
                &cssimpler_core::ElementPath::root(0).with_child(0),
                &scene.children[0],
            )
            .is_none()
        );
    }

    fn text_nodes(node: &cssimpler_core::RenderNode) -> Vec<String> {
        let mut text = Vec::new();
        collect_text_nodes(node, &mut text);
        text
    }

    fn collect_text_nodes(node: &cssimpler_core::RenderNode, text: &mut Vec<String>) {
        if let cssimpler_core::RenderKind::Text(content) = &node.kind {
            text.push(content.clone());
        }

        for child in &node.children {
            collect_text_nodes(child, text);
        }
    }
}
