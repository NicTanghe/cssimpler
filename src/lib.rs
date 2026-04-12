pub mod app;

pub use cssimpler_core as core;
pub use cssimpler_core::fonts;
pub use cssimpler_macro::{baked_stylesheet, baked_ui, ui, ui_prefab};
pub use cssimpler_renderer as renderer;
pub use cssimpler_style as style;

#[cfg(test)]
mod baked_macro_tests;
#[cfg(test)]
mod ui_macro_tests;
