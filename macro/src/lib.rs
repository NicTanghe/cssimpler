use proc_macro::TokenStream;

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{Error, Expr, Ident, LitStr, Result, Token, braced, parse_macro_input};

#[proc_macro]
pub fn ui(input: TokenStream) -> TokenStream {
    let root = parse_macro_input!(input as UiRoot);
    match expand_element(&root.element) {
        Ok(expanded) => expanded.into(),
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
        let tag = input.parse::<Ident>()?;
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
        let closing_tag = input.parse::<Ident>()?;
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
    name: Ident,
    value: AttributeValue,
}

enum AttributeValue {
    String(LitStr),
    Expression(Expr),
}

fn parse_attributes(input: ParseStream<'_>) -> Result<Vec<Attribute>> {
    let mut attributes = Vec::new();

    while !input.peek(Token![>]) && !(input.peek(Token![/]) && input.peek2(Token![>])) {
        let name = input.parse::<Ident>()?;
        input.parse::<Token![=]>()?;
        let value = if input.peek(LitStr) {
            AttributeValue::String(input.parse()?)
        } else if input.peek(syn::token::Brace) {
            let content;
            braced!(content in input);
            AttributeValue::Expression(content.parse()?)
        } else {
            return Err(input.error("expected a string literal or a braced Rust expression"));
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

    for child in &element.children {
        let child = expand_child(child)?;
        builder = quote!(#builder.with_child(#child));
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
    let name = attribute.name.to_string();

    match (name.as_str(), &attribute.value) {
        ("id", AttributeValue::String(value)) => Ok(quote!(#builder.with_id(#value))),
        ("id", AttributeValue::Expression(value)) => Ok(quote!(#builder.with_id(#value))),
        ("class", AttributeValue::String(value)) => {
            let classes: Vec<_> = value
                .value()
                .split_whitespace()
                .map(str::to_string)
                .collect();
            let mut builder = builder;

            for class_name in classes {
                builder = quote!(#builder.with_class(#class_name));
            }

            Ok(builder)
        }
        ("class", AttributeValue::Expression(value)) => Ok(quote!(#builder.with_class(#value))),
        ("onclick", AttributeValue::Expression(value)) => Ok(quote!(#builder.on_click(#value))),
        ("onclick", AttributeValue::String(value)) => Err(Error::new(
            value.span(),
            "onclick expects a Rust expression like onclick={handler}",
        )),
        _ => Err(Error::new(
            attribute.name.span(),
            format!("unsupported attribute `{name}`"),
        )),
    }
}
