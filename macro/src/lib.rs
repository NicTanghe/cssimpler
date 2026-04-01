use proc_macro::TokenStream;

mod ui_markup;

#[proc_macro]
pub fn ui(input: TokenStream) -> TokenStream {
    ui_markup::expand_ui(input)
}
