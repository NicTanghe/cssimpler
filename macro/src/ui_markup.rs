use proc_macro::TokenStream;

use proc_macro2::{Delimiter, TokenStream as TokenStream2, TokenTree};
use quote::quote;
use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};
use syn::{Error, Expr, Ident, LitStr, Result, Token, braced, parse_macro_input};

pub fn expand_ui(input: TokenStream) -> TokenStream {
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
    Text(String),
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

        let mut text_tokens = Vec::new();
        while !input.is_empty() && !input.peek(Token![<]) && !input.peek(syn::token::Brace) {
            text_tokens.push(input.parse::<TokenTree>()?);
        }

        if text_tokens.is_empty() {
            return Err(input.error("expected a child element, text, or a braced Rust expression"));
        }

        Ok(Self::Text(tokens_to_text(&text_tokens)))
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

fn tokens_to_text(tokens: &[TokenTree]) -> String {
    let mut text = String::new();

    for token in tokens {
        let segment = token_to_text(token);
        if segment.is_empty() {
            continue;
        }

        if let (Some(prev), Some(next)) = (text.chars().last(), segment.chars().next()) {
            if needs_space_between(prev, next) {
                text.push(' ');
            }
        }

        text.push_str(&segment);
    }

    text
}

fn token_to_text(token: &TokenTree) -> String {
    match token {
        TokenTree::Ident(ident) => ident.to_string(),
        TokenTree::Literal(literal) => {
            let raw = literal.to_string();
            match syn::parse_str::<LitStr>(&raw) {
                Ok(string_literal) => string_literal.value(),
                Err(_) => raw,
            }
        }
        TokenTree::Punct(punctuation) => punctuation.as_char().to_string(),
        TokenTree::Group(group) => {
            let inner_tokens: Vec<_> = group.stream().into_iter().collect();
            let inner = tokens_to_text(&inner_tokens);

            match group.delimiter() {
                Delimiter::Parenthesis => format!("({inner})"),
                Delimiter::Bracket => format!("[{inner}]"),
                Delimiter::Brace => format!("{{{inner}}}"),
                Delimiter::None => inner,
            }
        }
    }
}

fn needs_space_between(previous: char, next: char) -> bool {
    !(is_no_space_after(previous) || is_no_space_before(next))
}

fn is_no_space_before(ch: char) -> bool {
    matches!(
        ch,
        ',' | '.' | ';' | ':' | '!' | '?' | '%' | ')' | ']' | '}' | '>' | '/' | '-'
    )
}

fn is_no_space_after(ch: char) -> bool {
    matches!(ch, '(' | '[' | '{' | '<' | '/' | '-')
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
        Child::Text(text) => Ok(quote!(::cssimpler_core::Node::text(#text))),
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

#[cfg(test)]
mod tests {
    use quote::quote;
    use syn::{parse_quote, parse_str};

    use super::{
        Attribute, AttributeName, AttributeValue, Child, UiRoot, expand_attribute, expand_element,
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
    fn parser_accepts_plain_text_children_without_braces() {
        let root: UiRoot =
            parse_str(r#"<h1>This is a Heading</h1>"#).expect("ui markup should parse");
        assert_eq!(root.element.children.len(), 1);

        match &root.element.children[0] {
            Child::Text(text) => assert_eq!(text, "This is a Heading"),
            _ => panic!("expected plain text child"),
        }
    }

    #[test]
    fn parser_normalizes_plain_text_punctuation_without_extra_spacing() {
        let root: UiRoot = parse_str(r#"<p>end-to-end, really.</p>"#).expect("ui markup parse");
        let Child::Text(text) = &root.element.children[0] else {
            panic!("expected text child");
        };

        assert_eq!(text, "end-to-end, really.");
    }

    #[test]
    fn parser_supports_plain_string_literal_children() {
        let root: UiRoot = parse_str(r#"<p>"Hello"</p>"#).expect("ui markup parse");
        let Child::Text(text) = &root.element.children[0] else {
            panic!("expected text child");
        };

        assert_eq!(text, "Hello");
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
