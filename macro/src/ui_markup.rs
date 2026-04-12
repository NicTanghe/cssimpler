use proc_macro::TokenStream;

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};
use syn::spanned::Spanned;
use syn::{Error, Expr, Ident, Lit, LitStr, Result, Token, braced, parse_macro_input};

pub fn expand_ui(input: TokenStream) -> TokenStream {
    let root = parse_macro_input!(input as UiRoot);
    match expand_element(&root.element) {
        Ok(expanded) => expanded.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

pub fn expand_baked_ui(input: TokenStream) -> TokenStream {
    let root = parse_macro_input!(input as UiRoot);
    match expand_baked_element(&root.element) {
        Ok(expanded) => quote!({
            static __CSSIMPLER_BAKED_UI: ::cssimpler_core::StaticNodeDesc = #expanded;
            ::cssimpler_core::into_node(&__CSSIMPLER_BAKED_UI)
        })
        .into(),
        Err(error) => error.to_compile_error().into(),
    }
}

pub fn expand_ui_prefab(input: TokenStream) -> TokenStream {
    let root = parse_macro_input!(input as UiRoot);
    match expand_baked_element(&root.element) {
        Ok(expanded) => quote!({
            static __CSSIMPLER_UI_PREFAB: ::cssimpler_core::StaticNodeDesc = #expanded;
            &__CSSIMPLER_UI_PREFAB
        })
        .into(),
        Err(error) => error.to_compile_error().into(),
    }
}

struct UiRoot {
    element: Element,
}

impl Parse for UiRoot {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let element = input.parse::<Element>()?;
        if !input.is_empty() {
            return Err(input.error("expected a single root element"));
        }

        Ok(Self { element })
    }
}

struct Element {
    tag: Ident,
    attributes: Vec<Attribute>,
    children: Vec<Child>,
}

impl Parse for Element {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        input.parse::<Token![<]>()?;
        let tag = input.call(Ident::parse_any)?;
        let attributes = parse_attributes(input)?;

        if input.peek(Token![/]) {
            input.parse::<Token![/]>()?;
            input.parse::<Token![>]>()?;
            return Ok(Self {
                tag,
                attributes,
                children: Vec::new(),
            });
        }

        input.parse::<Token![>]>()?;

        let mut children = Vec::new();
        while !is_closing_tag(input) {
            if input.is_empty() {
                return Err(Error::new(tag.span(), "missing closing tag"));
            }

            children.push(input.parse::<Child>()?);
        }

        input.parse::<Token![<]>()?;
        input.parse::<Token![/]>()?;
        let closing_tag = input.call(Ident::parse_any)?;
        if closing_tag != tag {
            return Err(Error::new(
                closing_tag.span(),
                format!("expected closing tag </{}>", tag),
            ));
        }
        input.parse::<Token![>]>()?;

        Ok(Self {
            tag,
            attributes,
            children,
        })
    }
}

enum Child {
    Element(Element),
    Expression(Expr),
}

impl Parse for Child {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        if input.peek(Token![<]) {
            return Ok(Self::Element(input.parse()?));
        }

        if input.peek(syn::token::Brace) {
            let content;
            braced!(content in input);
            return Ok(Self::Expression(content.parse()?));
        }

        Err(input.error("expected a child element or a braced Rust expression"))
    }
}

struct Attribute {
    name: AttributeName,
    value: AttributeValue,
}

enum AttributeValue {
    String(LitStr),
    Expression(Expr),
}

struct AttributeName {
    segments: Vec<Ident>,
}

impl AttributeName {
    fn as_string(&self) -> String {
        self.segments
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("-")
    }
}

impl Parse for AttributeName {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let mut segments = vec![input.call(Ident::parse_any)?];

        while input.peek(Token![-]) {
            let fork = input.fork();
            fork.parse::<Token![-]>()?;
            if fork.call(Ident::parse_any).is_err() {
                break;
            }

            input.parse::<Token![-]>()?;
            segments.push(input.call(Ident::parse_any)?);
        }

        Ok(Self { segments })
    }
}

fn parse_attributes(input: ParseStream<'_>) -> Result<Vec<Attribute>> {
    let mut attributes = Vec::with_capacity(8);

    loop {
        if input.peek(Token![>]) || (input.peek(Token![/]) && input.peek2(Token![>])) {
            break;
        }

        let name = input.parse::<AttributeName>()?;
        input.parse::<Token![=]>()?;

        let value = if input.peek(LitStr) {
            AttributeValue::String(input.parse()?)
        } else if input.peek(syn::token::Brace) {
            let content;
            braced!(content in input);
            AttributeValue::Expression(content.parse()?)
        } else {
            return Err(input.error("expected string literal or {expression}"));
        };

        attributes.push(Attribute { name, value });
    }

    Ok(attributes)
}

fn is_closing_tag(input: ParseStream<'_>) -> bool {
    input.peek(Token![<]) && input.peek2(Token![/])
}

fn expand_element(element: &Element) -> Result<TokenStream2> {
    let tag = element.tag.to_string();
    let mut builder = quote!(::cssimpler_core::Node::element(#tag));

    for attribute in &element.attributes {
        builder = expand_attribute(builder, attribute)?;
    }

    if !element.children.is_empty() {
        let children = element
            .children
            .iter()
            .map(expand_child)
            .collect::<Result<Vec<_>>>()?;
        builder = quote! {
            #builder
            #( .with_child(#children) )*
        };
    }

    Ok(quote!(::cssimpler_core::Node::from(#builder)))
}

fn expand_child(child: &Child) -> Result<TokenStream2> {
    match child {
        Child::Element(element) => expand_element(element),
        Child::Expression(expression) => Ok(quote!(::cssimpler_core::into_node(#expression))),
    }
}

fn expand_attribute(builder: TokenStream2, attribute: &Attribute) -> Result<TokenStream2> {
    let name = attribute.name.as_string();
    let name_str = name.as_str();

    match name_str {
        "id" => match &attribute.value {
            AttributeValue::String(v) => Ok(quote!(#builder.with_id(#v))),
            AttributeValue::Expression(v) => Ok(quote!(#builder.with_id(#v))),
        },

        "class" => match &attribute.value {
            AttributeValue::String(v) => {
                let val = v.value();
                if !val.contains(' ') {
                    Ok(quote!(#builder.with_class(#val)))
                } else {
                    let classes = val.split_whitespace();
                    Ok(quote!(#builder #( .with_class(#classes) )* ))
                }
            }
            AttributeValue::Expression(v) => Ok(quote!(#builder.with_class(#v))),
        },

        _ if name_str.starts_with("on") => match &attribute.value {
            AttributeValue::Expression(v) => {
                let snake_name = format!("on_{}", &name_str[2..]);
                let method = quote::format_ident!("{}", snake_name);
                Ok(quote!(#builder.#method(#v)))
            }
            AttributeValue::String(v) => Err(Error::new(
                v.span(),
                format!("{name_str} expects a {{expression}}"),
            )),
        },

        _ => match &attribute.value {
            AttributeValue::String(v) => Ok(quote!(#builder.with_attribute(#name_str, #v))),
            AttributeValue::Expression(v) => Ok(quote!(#builder.with_attribute(#name_str, #v))),
        },
    }
}

fn expand_baked_element(element: &Element) -> Result<TokenStream2> {
    let tag = element.tag.to_string();
    let mut id = quote!(None);
    let mut classes = Vec::new();
    let mut attributes = Vec::new();
    let mut handlers = StaticHandlers::default();

    for attribute in &element.attributes {
        let name = attribute.name.as_string();
        match name.as_str() {
            "id" => {
                let value = static_attribute_text(&attribute.value)?;
                id = quote!(Some(#value));
            }
            "class" => {
                let value = static_attribute_text(&attribute.value)?;
                classes.extend(
                    value
                        .split_whitespace()
                        .map(|class_name| quote!(#class_name))
                        .collect::<Vec<_>>(),
                );
            }
            _ if name.starts_with("on") => {
                handlers.assign(name.as_str(), &attribute.value)?;
            }
            _ => {
                let value = static_attribute_text(&attribute.value)?;
                attributes.push(quote!(
                    ::cssimpler_core::StaticAttribute::new(#name, #value)
                ));
            }
        }
    }

    let children = element
        .children
        .iter()
        .map(expand_baked_child)
        .collect::<Result<Vec<_>>>()?;
    let handler_tokens = handlers.to_tokens();

    Ok(quote!(
        ::cssimpler_core::StaticNodeDesc::element(::cssimpler_core::StaticElementNodeDesc {
            tag: #tag,
            id: #id,
            classes: &[#(#classes),*],
            attributes: &[#(#attributes),*],
            children: &[#(#children),*],
            handlers: #handler_tokens,
        })
    ))
}

fn expand_baked_child(child: &Child) -> Result<TokenStream2> {
    match child {
        Child::Element(element) => expand_baked_element(element),
        Child::Expression(expression) => {
            let text = static_text_from_expr(expression).ok_or_else(|| {
                Error::new(
                    expression.span(),
                    "baked_ui! only supports literal text children; use ui! for dynamic expressions",
                )
            })?;
            Ok(quote!(::cssimpler_core::StaticNodeDesc::text(#text)))
        }
    }
}

fn static_attribute_text(value: &AttributeValue) -> Result<String> {
    match value {
        AttributeValue::String(value) => Ok(value.value()),
        AttributeValue::Expression(expression) => {
            static_text_from_expr(expression).ok_or_else(|| {
                Error::new(
                    expression.span(),
                    "baked_ui! attribute values must be string, char, bool, or numeric literals",
                )
            })
        }
    }
}

fn static_text_from_expr(expression: &Expr) -> Option<String> {
    match expression {
        Expr::Lit(expr_lit) => match &expr_lit.lit {
            Lit::Str(value) => Some(value.value()),
            Lit::Char(value) => Some(value.value().to_string()),
            Lit::Bool(value) => Some(value.value.to_string()),
            Lit::Int(value) => Some(value.base10_digits().to_string()),
            Lit::Float(value) => Some(value.base10_digits().to_string()),
            _ => None,
        },
        _ => None,
    }
}

#[derive(Default)]
struct StaticHandlers {
    click: Option<TokenStream2>,
    contextmenu: Option<TokenStream2>,
    dblclick: Option<TokenStream2>,
    mousedown: Option<TokenStream2>,
    mouseenter: Option<TokenStream2>,
    mouseleave: Option<TokenStream2>,
    mousemove: Option<TokenStream2>,
    mouseout: Option<TokenStream2>,
    mouseover: Option<TokenStream2>,
    mouseup: Option<TokenStream2>,
}

impl StaticHandlers {
    fn assign(&mut self, name: &str, value: &AttributeValue) -> Result<()> {
        let AttributeValue::Expression(expression) = value else {
            return Err(Error::new(
                match value {
                    AttributeValue::String(value) => value.span(),
                    AttributeValue::Expression(expression) => expression.span(),
                },
                format!("{name} expects a {{expression}}"),
            ));
        };

        let slot = match name {
            "onclick" => &mut self.click,
            "oncontextmenu" => &mut self.contextmenu,
            "ondblclick" => &mut self.dblclick,
            "onmousedown" => &mut self.mousedown,
            "onmouseenter" => &mut self.mouseenter,
            "onmouseleave" => &mut self.mouseleave,
            "onmousemove" => &mut self.mousemove,
            "onmouseout" => &mut self.mouseout,
            "onmouseover" => &mut self.mouseover,
            "onmouseup" => &mut self.mouseup,
            _ => {
                return Err(Error::new(
                    expression.span(),
                    format!("unsupported event handler attribute {name}"),
                ));
            }
        };
        *slot = Some(quote!(#expression));
        Ok(())
    }

    fn to_tokens(&self) -> TokenStream2 {
        let click = option_tokens(&self.click);
        let contextmenu = option_tokens(&self.contextmenu);
        let dblclick = option_tokens(&self.dblclick);
        let mousedown = option_tokens(&self.mousedown);
        let mouseenter = option_tokens(&self.mouseenter);
        let mouseleave = option_tokens(&self.mouseleave);
        let mousemove = option_tokens(&self.mousemove);
        let mouseout = option_tokens(&self.mouseout);
        let mouseover = option_tokens(&self.mouseover);
        let mouseup = option_tokens(&self.mouseup);

        quote!(
            ::cssimpler_core::EventHandlers {
                click: #click,
                contextmenu: #contextmenu,
                dblclick: #dblclick,
                mousedown: #mousedown,
                mouseenter: #mouseenter,
                mouseleave: #mouseleave,
                mousemove: #mousemove,
                mouseout: #mouseout,
                mouseover: #mouseover,
                mouseup: #mouseup,
            }
        )
    }
}

fn option_tokens(value: &Option<TokenStream2>) -> TokenStream2 {
    match value {
        Some(value) => quote!(Some(#value)),
        None => quote!(None),
    }
}

#[cfg(test)]
mod tests {
    use quote::quote;
    use syn::{parse_quote, parse_str};

    use super::{
        Attribute, AttributeName, AttributeValue, UiRoot, expand_attribute, expand_element,
    };

    #[test]
    fn parser_accepts_dashed_and_keyword_attribute_names() {
        let root: UiRoot =
            parse_str(r#"<button data-text="uiverse" aria-hidden="true" type="button"></button>"#)
                .expect("ui markup should parse");
        let names: Vec<_> = root
            .element
            .attributes
            .iter()
            .map(|attribute| attribute.name.as_string())
            .collect();

        assert_eq!(names, vec!["data-text", "aria-hidden", "type"]);
    }

    #[test]
    fn generic_string_attributes_expand_to_with_attribute_calls() {
        let root: UiRoot = parse_str(r#"<button data-text="uiverse" type="button"></button>"#)
            .expect("ui markup should parse");
        let expanded = expand_element(&root.element)
            .expect("supported attributes should expand")
            .to_string();

        assert!(expanded.contains(". with_attribute (\"data-text\" , \"uiverse\")"));
        assert!(expanded.contains(". with_attribute (\"type\" , \"button\")"));
    }

    #[test]
    fn generic_attributes_accept_expression_values() {
        let attribute = Attribute {
            name: parse_str::<AttributeName>("data-text").expect("attribute name should parse"),
            value: AttributeValue::Expression(parse_quote!(dynamic_value)),
        };
        let expanded = expand_attribute(quote!(builder), &attribute)
            .expect("generic expression attributes should expand")
            .to_string();

        assert!(expanded.contains(". with_attribute (\"data-text\" , dynamic_value)"));
    }
}
