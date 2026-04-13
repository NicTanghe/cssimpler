#[allow(dead_code)]
#[path = "uiverse_hover_button.rs"]
mod shared;

use anyhow::Result;
use cssimpler::renderer::{RendererBackendKind, WindowConfig};

fn main() -> Result<()> {
    shared::run(
        WindowConfig::new("cssimpler / uiverse hover button / gpu", 1280, 720)
            .with_backend(RendererBackendKind::Gpu),
    )
}
