use proc_macro::TokenStream;

mod baked_stylesheet;
mod ui_markup;

#[proc_macro]
pub fn ui(input: TokenStream) -> TokenStream {
    ui_markup::expand_ui(input)
}

#[proc_macro]
pub fn baked_ui(input: TokenStream) -> TokenStream {
    ui_markup::expand_baked_ui(input)
}

#[proc_macro]
pub fn ui_prefab(input: TokenStream) -> TokenStream {
    ui_markup::expand_ui_prefab(input)
}

#[proc_macro]
pub fn baked_stylesheet(input: TokenStream) -> TokenStream {
    baked_stylesheet::expand_baked_stylesheet(input)
}
