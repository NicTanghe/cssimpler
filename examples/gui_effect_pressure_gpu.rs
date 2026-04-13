#[allow(dead_code)]
#[path = "gui_effect_pressure.rs"]
mod shared;

use anyhow::Result;
use cssimpler::renderer::{RendererBackendKind, WindowConfig};

fn main() -> Result<()> {
    shared::run(
        WindowConfig::new("cssimpler / gui effect pressure / gpu", 1440, 960)
            .with_backend(RendererBackendKind::Gpu),
    )
}
