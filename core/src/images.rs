//! Decoded image resources. Loading and decoding are performed by the host.
use crate::Color;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, OnceLock},
};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum ImageFit {
    #[default]
    Auto,
    Fill,
    Cover,
    Contain,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RasterBackground {
    pub source: Option<String>,
    pub fit: ImageFit,
    pub position: (f32, f32),
    pub repeat: (bool, bool),
    pub opacity: f32,
    pub generation: u64,
}
impl Default for RasterBackground {
    fn default() -> Self {
        Self {
            source: None,
            fit: ImageFit::Auto,
            position: (0.0, 0.0),
            repeat: (true, true),
            opacity: 1.0,
            generation: 0,
        }
    }
}

#[derive(Debug)]
pub struct RasterImage {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<Color>,
    pub generation: u64,
}
type Registry = VecDeque<(String, Arc<RasterImage>)>;
fn registry() -> &'static Mutex<Registry> {
    static CACHE: OnceLock<Mutex<Registry>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(VecDeque::new()))
}

pub fn register_image(source: String, width: usize, height: usize, rgba: &[u8]) -> bool {
    let Some(size) = width.checked_mul(height).and_then(|v| v.checked_mul(4)) else {
        return false;
    };
    if size == 0 || size != rgba.len() || size > 64 * 1024 * 1024 {
        return false;
    }
    let pixels = rgba
        .chunks_exact(4)
        .map(|p| Color::rgba(p[0], p[1], p[2], p[3]))
        .collect();
    let mut cache = registry().lock().unwrap_or_else(|e| e.into_inner());
    cache.retain(|(key, _)| key != &source);
    static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let generation = GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    cache.push_front((
        source,
        Arc::new(RasterImage {
            width,
            height,
            pixels,
            generation,
        }),
    ));
    let mut bytes = 0;
    cache.retain(|(_, image)| {
        bytes += image.pixels.len() * 4;
        bytes <= 128 * 1024 * 1024
    });
    true
}

pub fn image(source: &str) -> Option<Arc<RasterImage>> {
    registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .find(|(key, _)| key == source)
        .map(|(_, image)| Arc::clone(image))
}
