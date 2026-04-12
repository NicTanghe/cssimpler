use proc_macro::TokenStream;
use std::fs;
use std::path::PathBuf;

use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::quote;
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{Error, Expr, ExprLit, ExprMacro, Lit, LitStr, Token, parse_macro_input};
use taffy::prelude::{
    AlignContent as TaffyAlignContent, AlignItems as TaffyAlignItems, AlignSelf as TaffyAlignSelf,
    Dimension, Display as TaffyDisplay, FlexDirection, FlexWrap,
    JustifyContent as TaffyJustifyContent, LengthPercentage as TaffyLengthPercentage,
    LengthPercentageAuto as TaffyLengthPercentageAuto, Position as TaffyPosition,
};

pub fn expand_baked_stylesheet(input: TokenStream) -> TokenStream {
    let source = parse_macro_input!(input as Expr);
    match expand_stylesheet_expr(&source) {
        Ok(expanded) => expanded.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

fn expand_stylesheet_expr(source: &Expr) -> syn::Result<TokenStream2> {
    let source_text = evaluate_stylesheet_expr(source)?;
    let parsed = cssimpler_style::parse_stylesheet(&source_text).map_err(|error| {
        Error::new(
            source.span(),
            format!("baked_stylesheet! failed to parse stylesheet: {error}"),
        )
    })?;
    let rules = parsed
        .rules
        .iter()
        .map(|rule| quote_rule(rule, source.span()))
        .collect::<syn::Result<Vec<_>>>()?;

    Ok(quote!({
        static __CSSIMPLER_BAKED_STYLESHEET: ::cssimpler_style::StaticStylesheetDesc =
            ::cssimpler_style::StaticStylesheetDesc::new(&[#(#rules),*]);
        __CSSIMPLER_BAKED_STYLESHEET.to_stylesheet()
    }))
}

fn evaluate_stylesheet_expr(expr: &Expr) -> syn::Result<String> {
    match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Str(source),
            ..
        }) => Ok(source.value()),
        Expr::Macro(expr_macro) if expr_macro.mac.path.is_ident("include_str") => {
            evaluate_include_str(expr_macro)
        }
        Expr::Macro(expr_macro) if expr_macro.mac.path.is_ident("concat") => {
            evaluate_concat(expr_macro)
        }
        _ => Err(Error::new(
            expr.span(),
            "baked_stylesheet! expects a string literal, include_str!(...), or concat!(...) of those",
        )),
    }
}

fn evaluate_include_str(expr_macro: &ExprMacro) -> syn::Result<String> {
    let path = syn::parse2::<LitStr>(expr_macro.mac.tokens.clone()).map_err(|_| {
        Error::new(
            expr_macro.mac.span(),
            "include_str! inside baked_stylesheet! expects a string literal path",
        )
    })?;
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").map_err(|_| {
        Error::new(
            expr_macro.mac.span(),
            "CARGO_MANIFEST_DIR is unavailable while expanding baked_stylesheet!",
        )
    })?;
    let full_path = PathBuf::from(manifest_dir).join(path.value());
    fs::read_to_string(&full_path).map_err(|error| {
        Error::new(
            path.span(),
            format!(
                "failed to read stylesheet source from {}: {error}",
                full_path.display()
            ),
        )
    })
}

fn evaluate_concat(expr_macro: &ExprMacro) -> syn::Result<String> {
    let parser = Punctuated::<Expr, Token![,]>::parse_terminated;
    let parts = parser.parse2(expr_macro.mac.tokens.clone()).map_err(|_| {
        Error::new(
            expr_macro.mac.span(),
            "concat! inside baked_stylesheet! expects comma-separated string expressions",
        )
    })?;
    let mut source = String::new();
    for part in parts {
        source.push_str(&evaluate_stylesheet_expr(&part)?);
    }
    Ok(source)
}

fn quote_rule(rule: &cssimpler_style::StyleRule, span: Span) -> syn::Result<TokenStream2> {
    let selector = quote_selector(&rule.selector);
    let declarations = rule
        .declarations
        .iter()
        .map(|declaration| quote_declaration(declaration, span))
        .collect::<syn::Result<Vec<_>>>()?;

    Ok(quote!(
        ::cssimpler_style::StaticStyleRuleDesc::new(
            #selector,
            &[#(#declarations),*],
        )
    ))
}

fn quote_selector(selector: &cssimpler_style::Selector) -> TokenStream2 {
    let rightmost = selector
        .rightmost
        .simple_selectors
        .iter()
        .map(quote_simple_selector)
        .collect::<Vec<_>>();
    let ancestors = selector
        .ancestors
        .iter()
        .map(|ancestor| {
            let combinator = quote_combinator(ancestor.combinator);
            let selectors = ancestor
                .compound
                .simple_selectors
                .iter()
                .map(quote_simple_selector)
                .collect::<Vec<_>>();
            quote!(
                ::cssimpler_style::StaticAncestorSelectorDesc::new(
                    #combinator,
                    &[#(#selectors),*],
                )
            )
        })
        .collect::<Vec<_>>();
    let pseudo_element = quote_pseudo_element(selector.pseudo_element);

    quote!(
        ::cssimpler_style::StaticSelectorDesc::new(
            &[#(#rightmost),*],
            &[#(#ancestors),*],
            #pseudo_element,
        )
    )
}

fn quote_combinator(combinator: cssimpler_style::SelectorCombinator) -> TokenStream2 {
    match combinator {
        cssimpler_style::SelectorCombinator::Descendant => {
            quote!(::cssimpler_style::SelectorCombinator::Descendant)
        }
        cssimpler_style::SelectorCombinator::Child => {
            quote!(::cssimpler_style::SelectorCombinator::Child)
        }
    }
}

fn quote_pseudo_element(
    pseudo_element: Option<cssimpler_style::PseudoElementKind>,
) -> TokenStream2 {
    match pseudo_element {
        Some(cssimpler_style::PseudoElementKind::Before) => {
            quote!(Some(::cssimpler_style::PseudoElementKind::Before))
        }
        Some(cssimpler_style::PseudoElementKind::After) => {
            quote!(Some(::cssimpler_style::PseudoElementKind::After))
        }
        None => quote!(None),
    }
}

fn quote_simple_selector(selector: &cssimpler_style::SimpleSelector) -> TokenStream2 {
    match selector {
        cssimpler_style::SimpleSelector::Class(name) => {
            quote!(::cssimpler_style::StaticSimpleSelectorDesc::Class(#name))
        }
        cssimpler_style::SimpleSelector::Id(name) => {
            quote!(::cssimpler_style::StaticSimpleSelectorDesc::Id(#name))
        }
        cssimpler_style::SimpleSelector::Tag(name) => {
            quote!(::cssimpler_style::StaticSimpleSelectorDesc::Tag(#name))
        }
        cssimpler_style::SimpleSelector::AttributeExists(name) => {
            quote!(::cssimpler_style::StaticSimpleSelectorDesc::AttributeExists(#name))
        }
        cssimpler_style::SimpleSelector::AttributeEquals { name, value } => quote!(
            ::cssimpler_style::StaticSimpleSelectorDesc::AttributeEquals {
                name: #name,
                value: #value,
            }
        ),
        cssimpler_style::SimpleSelector::Hover => {
            quote!(::cssimpler_style::StaticSimpleSelectorDesc::Hover)
        }
        cssimpler_style::SimpleSelector::Active => {
            quote!(::cssimpler_style::StaticSimpleSelectorDesc::Active)
        }
    }
}

fn quote_declaration(
    declaration: &cssimpler_style::Declaration,
    span: Span,
) -> syn::Result<TokenStream2> {
    match declaration {
        cssimpler_style::Declaration::CustomProperty { name, value } => Ok(quote!(
            ::cssimpler_style::StaticDeclarationDesc::CustomProperty {
                name: #name,
                value: #value,
            }
        )),
        cssimpler_style::Declaration::VariableDependentProperty {
            property_name,
            value_css,
        } => Ok(quote!(
            ::cssimpler_style::StaticDeclarationDesc::VariableDependentProperty {
                property_name: #property_name,
                value_css: #value_css,
            }
        )),
        cssimpler_style::Declaration::Background(color) => {
            let color = quote_color(*color);
            Ok(quote!(::cssimpler_style::StaticDeclarationDesc::Background(#color)))
        }
        cssimpler_style::Declaration::Foreground(color) => {
            let color = quote_color(*color);
            Ok(quote!(::cssimpler_style::StaticDeclarationDesc::Foreground(#color)))
        }
        cssimpler_style::Declaration::OverflowX(mode) => {
            let mode = quote_overflow_mode(*mode);
            Ok(quote!(::cssimpler_style::StaticDeclarationDesc::OverflowX(#mode)))
        }
        cssimpler_style::Declaration::OverflowY(mode) => {
            let mode = quote_overflow_mode(*mode);
            Ok(quote!(::cssimpler_style::StaticDeclarationDesc::OverflowY(#mode)))
        }
        cssimpler_style::Declaration::Display(display) => {
            let display = quote_display(*display);
            Ok(quote!(::cssimpler_style::StaticDeclarationDesc::Display(#display)))
        }
        cssimpler_style::Declaration::Position(position) => {
            let position = quote_position(*position);
            Ok(quote!(::cssimpler_style::StaticDeclarationDesc::Position(#position)))
        }
        cssimpler_style::Declaration::InsetTop(value) => {
            let value = quote_length_percentage_auto(*value);
            Ok(quote!(::cssimpler_style::StaticDeclarationDesc::InsetTop(#value)))
        }
        cssimpler_style::Declaration::InsetRight(value) => {
            let value = quote_length_percentage_auto(*value);
            Ok(quote!(::cssimpler_style::StaticDeclarationDesc::InsetRight(#value)))
        }
        cssimpler_style::Declaration::InsetBottom(value) => {
            let value = quote_length_percentage_auto(*value);
            Ok(quote!(::cssimpler_style::StaticDeclarationDesc::InsetBottom(#value)))
        }
        cssimpler_style::Declaration::InsetLeft(value) => {
            let value = quote_length_percentage_auto(*value);
            Ok(quote!(::cssimpler_style::StaticDeclarationDesc::InsetLeft(#value)))
        }
        cssimpler_style::Declaration::Width(value) => {
            let value = quote_dimension(*value);
            Ok(quote!(::cssimpler_style::StaticDeclarationDesc::Width(#value)))
        }
        cssimpler_style::Declaration::Height(value) => {
            let value = quote_dimension(*value);
            Ok(quote!(::cssimpler_style::StaticDeclarationDesc::Height(#value)))
        }
        cssimpler_style::Declaration::MarginTop(value) => {
            let value = quote_length_percentage_auto(*value);
            Ok(quote!(::cssimpler_style::StaticDeclarationDesc::MarginTop(#value)))
        }
        cssimpler_style::Declaration::MarginRight(value) => {
            let value = quote_length_percentage_auto(*value);
            Ok(quote!(::cssimpler_style::StaticDeclarationDesc::MarginRight(#value)))
        }
        cssimpler_style::Declaration::MarginBottom(value) => {
            let value = quote_length_percentage_auto(*value);
            Ok(quote!(::cssimpler_style::StaticDeclarationDesc::MarginBottom(#value)))
        }
        cssimpler_style::Declaration::MarginLeft(value) => {
            let value = quote_length_percentage_auto(*value);
            Ok(quote!(::cssimpler_style::StaticDeclarationDesc::MarginLeft(#value)))
        }
        cssimpler_style::Declaration::PaddingTop(value) => {
            let value = quote_length_percentage(*value);
            Ok(quote!(::cssimpler_style::StaticDeclarationDesc::PaddingTop(#value)))
        }
        cssimpler_style::Declaration::PaddingRight(value) => {
            let value = quote_length_percentage(*value);
            Ok(quote!(::cssimpler_style::StaticDeclarationDesc::PaddingRight(#value)))
        }
        cssimpler_style::Declaration::PaddingBottom(value) => {
            let value = quote_length_percentage(*value);
            Ok(quote!(::cssimpler_style::StaticDeclarationDesc::PaddingBottom(#value)))
        }
        cssimpler_style::Declaration::PaddingLeft(value) => {
            let value = quote_length_percentage(*value);
            Ok(quote!(::cssimpler_style::StaticDeclarationDesc::PaddingLeft(#value)))
        }
        cssimpler_style::Declaration::FlexDirection(direction) => {
            let direction = quote_flex_direction(*direction);
            Ok(quote!(::cssimpler_style::StaticDeclarationDesc::FlexDirection(#direction)))
        }
        cssimpler_style::Declaration::FlexWrap(wrap) => {
            let wrap = quote_flex_wrap(*wrap);
            Ok(quote!(::cssimpler_style::StaticDeclarationDesc::FlexWrap(#wrap)))
        }
        cssimpler_style::Declaration::JustifyContent(value) => {
            let value = quote_justify_content(*value);
            Ok(quote!(::cssimpler_style::StaticDeclarationDesc::JustifyContent(#value)))
        }
        cssimpler_style::Declaration::AlignItems(value) => {
            let value = quote_align_items(*value);
            Ok(quote!(::cssimpler_style::StaticDeclarationDesc::AlignItems(#value)))
        }
        cssimpler_style::Declaration::AlignSelf(value) => {
            let value = quote_align_self(*value);
            Ok(quote!(::cssimpler_style::StaticDeclarationDesc::AlignSelf(#value)))
        }
        cssimpler_style::Declaration::AlignContent(value) => {
            let value = quote_align_content(*value);
            Ok(quote!(::cssimpler_style::StaticDeclarationDesc::AlignContent(#value)))
        }
        cssimpler_style::Declaration::GapRow(value) => {
            let value = quote_length_percentage(*value);
            Ok(quote!(::cssimpler_style::StaticDeclarationDesc::GapRow(#value)))
        }
        cssimpler_style::Declaration::GapColumn(value) => {
            let value = quote_length_percentage(*value);
            Ok(quote!(::cssimpler_style::StaticDeclarationDesc::GapColumn(#value)))
        }
        cssimpler_style::Declaration::FlexGrow(value) => Ok(quote!(
            ::cssimpler_style::StaticDeclarationDesc::FlexGrow(#value)
        )),
        cssimpler_style::Declaration::FlexShrink(value) => Ok(quote!(
            ::cssimpler_style::StaticDeclarationDesc::FlexShrink(#value)
        )),
        cssimpler_style::Declaration::FlexBasis(value) => {
            let value = quote_dimension(*value);
            Ok(quote!(::cssimpler_style::StaticDeclarationDesc::FlexBasis(#value)))
        }
        _ => Err(Error::new(
            span,
            format!(
                "baked_stylesheet! does not support declaration {declaration:?}; use parse_stylesheet(...) for dynamic parsing"
            ),
        )),
    }
}

fn quote_color(color: cssimpler_core::Color) -> TokenStream2 {
    let r = color.r;
    let g = color.g;
    let b = color.b;
    let a = color.a;
    quote!(::cssimpler_core::Color::rgba(#r, #g, #b, #a))
}

fn quote_overflow_mode(mode: cssimpler_core::OverflowMode) -> TokenStream2 {
    match mode {
        cssimpler_core::OverflowMode::Visible => {
            quote!(::cssimpler_core::OverflowMode::Visible)
        }
        cssimpler_core::OverflowMode::Clip => quote!(::cssimpler_core::OverflowMode::Clip),
        cssimpler_core::OverflowMode::Hidden => quote!(::cssimpler_core::OverflowMode::Hidden),
        cssimpler_core::OverflowMode::Auto => quote!(::cssimpler_core::OverflowMode::Auto),
        cssimpler_core::OverflowMode::Scroll => quote!(::cssimpler_core::OverflowMode::Scroll),
    }
}

fn quote_display(display: TaffyDisplay) -> TokenStream2 {
    match display {
        TaffyDisplay::None => quote!(::cssimpler_style::StaticDisplay::None),
        TaffyDisplay::Flex => quote!(::cssimpler_style::StaticDisplay::Flex),
        TaffyDisplay::Grid => quote!(::cssimpler_style::StaticDisplay::Grid),
        TaffyDisplay::Block => quote!(::cssimpler_style::StaticDisplay::Block),
    }
}

fn quote_position(position: TaffyPosition) -> TokenStream2 {
    match position {
        TaffyPosition::Relative => quote!(::cssimpler_style::StaticPosition::Relative),
        TaffyPosition::Absolute => quote!(::cssimpler_style::StaticPosition::Absolute),
    }
}

fn quote_dimension(dimension: Dimension) -> TokenStream2 {
    match dimension {
        Dimension::Auto => quote!(::cssimpler_style::StaticDimension::Auto),
        Dimension::Length(value) => {
            quote!(::cssimpler_style::StaticDimension::Length(#value))
        }
        Dimension::Percent(value) => {
            quote!(::cssimpler_style::StaticDimension::Percent(#value))
        }
    }
}

fn quote_length_percentage(value: TaffyLengthPercentage) -> TokenStream2 {
    match value {
        TaffyLengthPercentage::Length(value) => {
            quote!(::cssimpler_style::StaticLengthPercentage::Length(#value))
        }
        TaffyLengthPercentage::Percent(value) => {
            quote!(::cssimpler_style::StaticLengthPercentage::Percent(#value))
        }
    }
}

fn quote_length_percentage_auto(value: TaffyLengthPercentageAuto) -> TokenStream2 {
    match value {
        TaffyLengthPercentageAuto::Auto => {
            quote!(::cssimpler_style::StaticLengthPercentageAuto::Auto)
        }
        TaffyLengthPercentageAuto::Length(value) => {
            quote!(::cssimpler_style::StaticLengthPercentageAuto::Length(#value))
        }
        TaffyLengthPercentageAuto::Percent(value) => {
            quote!(::cssimpler_style::StaticLengthPercentageAuto::Percent(#value))
        }
    }
}

fn quote_flex_direction(direction: FlexDirection) -> TokenStream2 {
    match direction {
        FlexDirection::Row => quote!(::cssimpler_style::StaticFlexDirection::Row),
        FlexDirection::RowReverse => {
            quote!(::cssimpler_style::StaticFlexDirection::RowReverse)
        }
        FlexDirection::Column => quote!(::cssimpler_style::StaticFlexDirection::Column),
        FlexDirection::ColumnReverse => {
            quote!(::cssimpler_style::StaticFlexDirection::ColumnReverse)
        }
    }
}

fn quote_flex_wrap(wrap: FlexWrap) -> TokenStream2 {
    match wrap {
        FlexWrap::NoWrap => quote!(::cssimpler_style::StaticFlexWrap::NoWrap),
        FlexWrap::Wrap => quote!(::cssimpler_style::StaticFlexWrap::Wrap),
        FlexWrap::WrapReverse => quote!(::cssimpler_style::StaticFlexWrap::WrapReverse),
    }
}

fn quote_justify_content(value: Option<TaffyJustifyContent>) -> TokenStream2 {
    match value {
        Some(TaffyJustifyContent::Start) => {
            quote!(Some(::cssimpler_style::StaticJustifyContent::Start))
        }
        Some(TaffyJustifyContent::End) => {
            quote!(Some(::cssimpler_style::StaticJustifyContent::End))
        }
        Some(TaffyJustifyContent::FlexStart) => {
            quote!(Some(::cssimpler_style::StaticJustifyContent::FlexStart))
        }
        Some(TaffyJustifyContent::FlexEnd) => {
            quote!(Some(::cssimpler_style::StaticJustifyContent::FlexEnd))
        }
        Some(TaffyJustifyContent::Center) => {
            quote!(Some(::cssimpler_style::StaticJustifyContent::Center))
        }
        Some(TaffyJustifyContent::SpaceBetween) => {
            quote!(Some(::cssimpler_style::StaticJustifyContent::SpaceBetween))
        }
        Some(TaffyJustifyContent::SpaceAround) => {
            quote!(Some(::cssimpler_style::StaticJustifyContent::SpaceAround))
        }
        Some(TaffyJustifyContent::SpaceEvenly) => {
            quote!(Some(::cssimpler_style::StaticJustifyContent::SpaceEvenly))
        }
        Some(TaffyJustifyContent::Stretch) => {
            quote!(Some(::cssimpler_style::StaticJustifyContent::Stretch))
        }
        None => quote!(None),
    }
}

fn quote_align_content(value: Option<TaffyAlignContent>) -> TokenStream2 {
    match value {
        Some(TaffyAlignContent::Start) => {
            quote!(Some(::cssimpler_style::StaticAlignContent::Start))
        }
        Some(TaffyAlignContent::End) => {
            quote!(Some(::cssimpler_style::StaticAlignContent::End))
        }
        Some(TaffyAlignContent::FlexStart) => {
            quote!(Some(::cssimpler_style::StaticAlignContent::FlexStart))
        }
        Some(TaffyAlignContent::FlexEnd) => {
            quote!(Some(::cssimpler_style::StaticAlignContent::FlexEnd))
        }
        Some(TaffyAlignContent::Center) => {
            quote!(Some(::cssimpler_style::StaticAlignContent::Center))
        }
        Some(TaffyAlignContent::SpaceBetween) => {
            quote!(Some(::cssimpler_style::StaticAlignContent::SpaceBetween))
        }
        Some(TaffyAlignContent::SpaceAround) => {
            quote!(Some(::cssimpler_style::StaticAlignContent::SpaceAround))
        }
        Some(TaffyAlignContent::SpaceEvenly) => {
            quote!(Some(::cssimpler_style::StaticAlignContent::SpaceEvenly))
        }
        Some(TaffyAlignContent::Stretch) => {
            quote!(Some(::cssimpler_style::StaticAlignContent::Stretch))
        }
        None => quote!(None),
    }
}

fn quote_align_items(value: Option<TaffyAlignItems>) -> TokenStream2 {
    match value {
        Some(TaffyAlignItems::Start) => {
            quote!(Some(::cssimpler_style::StaticAlignItems::Start))
        }
        Some(TaffyAlignItems::End) => {
            quote!(Some(::cssimpler_style::StaticAlignItems::End))
        }
        Some(TaffyAlignItems::FlexStart) => {
            quote!(Some(::cssimpler_style::StaticAlignItems::FlexStart))
        }
        Some(TaffyAlignItems::FlexEnd) => {
            quote!(Some(::cssimpler_style::StaticAlignItems::FlexEnd))
        }
        Some(TaffyAlignItems::Center) => {
            quote!(Some(::cssimpler_style::StaticAlignItems::Center))
        }
        Some(TaffyAlignItems::Stretch) => {
            quote!(Some(::cssimpler_style::StaticAlignItems::Stretch))
        }
        Some(TaffyAlignItems::Baseline) => {
            quote!(Some(::cssimpler_style::StaticAlignItems::Baseline))
        }
        None => quote!(None),
    }
}

fn quote_align_self(value: Option<TaffyAlignSelf>) -> TokenStream2 {
    match value {
        Some(TaffyAlignSelf::Start) => {
            quote!(Some(::cssimpler_style::StaticAlignSelf::Start))
        }
        Some(TaffyAlignSelf::End) => {
            quote!(Some(::cssimpler_style::StaticAlignSelf::End))
        }
        Some(TaffyAlignSelf::FlexStart) => {
            quote!(Some(::cssimpler_style::StaticAlignSelf::FlexStart))
        }
        Some(TaffyAlignSelf::FlexEnd) => {
            quote!(Some(::cssimpler_style::StaticAlignSelf::FlexEnd))
        }
        Some(TaffyAlignSelf::Center) => {
            quote!(Some(::cssimpler_style::StaticAlignSelf::Center))
        }
        Some(TaffyAlignSelf::Stretch) => {
            quote!(Some(::cssimpler_style::StaticAlignSelf::Stretch))
        }
        Some(TaffyAlignSelf::Baseline) => {
            quote!(Some(::cssimpler_style::StaticAlignSelf::Baseline))
        }
        None => quote!(None),
    }
}
