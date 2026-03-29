use proc_macro::TokenStream;

#[proc_macro]
pub fn ui(_input: TokenStream) -> TokenStream {
    "::cssimpler_core::Node::text(\"\")"
        .parse()
        .expect("ui! bootstrap expansion should stay valid Rust")
}
