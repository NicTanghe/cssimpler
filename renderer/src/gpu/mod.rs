//! Optional native GPU viewports. UI rasterization remains on the CPU.
//!
//! Register a renderer against an existing element ID. The window runtime owns
//! the native host and presentation; callbacks only encode GPU work.

mod geometry;
mod native;
pub(crate) mod runtime;
pub mod usd;

pub use openusd;
pub use openusd_schemas;
pub use wgpu;

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use crate::{EngineEvent, FrameInfo, RedrawSchedule, RendererError, Result};
use cssimpler_core::LayoutBox;

/// A viewport is opaque unless transparency is explicitly requested.
#[derive(Clone, Copy, Debug, Default)]
pub struct GpuViewportOptions {
    pub transparent: bool,
    pub redraw: RedrawSchedule,
}

/// Requirements shared by all viewports in one window runtime.
#[derive(Clone, Debug, Default)]
pub struct GpuDeviceOptions {
    pub required_features: wgpu::Features,
    pub required_limits: wgpu::Limits,
}

/// Shared handles. No native window or presentation handles are exposed.
pub struct GpuContext {
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GpuViewportInfo {
    pub bounds: LayoutBox,
    pub physical_size: [u32; 2],
    pub scale_factor: f64,
    pub format: wgpu::TextureFormat,
    /// Alpha encoding expected from the renderer. Native composition may use a
    /// different swapchain mode (for example, redirected X11 ARGB windows).
    pub alpha_mode: wgpu::CompositeAlphaMode,
    /// Known solid backdrop, or None when native glass must show through.
    pub background: Option<cssimpler_core::Color>,
    pub texture_usage: wgpu::TextureUsages,
}

pub struct GpuViewportFrame<'a> {
    pub info: GpuViewportInfo,
    pub frame: FrameInfo,
    pub target: &'a wgpu::TextureView,
    /// Native presentation image; COPY_DST is available when texture_usage includes it.
    pub texture: &'a wgpu::Texture,
    pub encoder: &'a mut wgpu::CommandEncoder,
}

/// Callback objects stay on the event-loop thread; USD stages need not be Send.
pub trait GpuViewportRenderer {
    fn resize(&mut self, _gpu: &GpuContext, _info: GpuViewportInfo) -> Result<()> {
        Ok(())
    }
    /// Return true when the event was consumed. Pointer positions are local CSS pixels.
    fn event(&mut self, _event: &EngineEvent) -> bool {
        false
    }
    fn render(&mut self, gpu: &GpuContext, frame: GpuViewportFrame<'_>) -> Result<()>;
}

type Wake = Box<dyn Fn() + Send + Sync>;
struct RedrawState {
    dirty: AtomicBool,
    wake: Mutex<Option<Wake>>,
}

/// Thread-safe invalidation without forcing a CPU UI redraw.
#[derive(Clone)]
pub struct GpuRedrawHandle(Arc<RedrawState>);

impl Default for GpuRedrawHandle {
    fn default() -> Self {
        Self(Arc::new(RedrawState {
            dirty: AtomicBool::new(true),
            wake: Mutex::new(None),
        }))
    }
}

impl GpuRedrawHandle {
    pub fn request_redraw(&self) {
        // The first invalidation wakes the event loop. Later pointer samples
        // only update the pending frame; they must not flood its user-event queue.
        if self.0.dirty.swap(true, Ordering::AcqRel) {
            return;
        }
        if let Some(wake) = self
            .0
            .wake
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            wake();
        }
    }
    pub(crate) fn set_wake(&self, wake: Wake) {
        *self.0.wake.lock().unwrap_or_else(|e| e.into_inner()) = Some(wake);
    }
    pub(crate) fn dirty(&self) -> bool {
        self.0.dirty.load(Ordering::Acquire)
    }
    pub(crate) fn take_dirty(&self) -> bool {
        self.0.dirty.swap(false, Ordering::AcqRel)
    }
}

pub struct GpuViewportInit<'a> {
    pub gpu: &'a GpuContext,
    pub info: GpuViewportInfo,
    pub redraw: GpuRedrawHandle,
}

type Factory = Box<dyn FnOnce(GpuViewportInit<'_>) -> Result<Box<dyn GpuViewportRenderer>>>;

pub(crate) struct Registration {
    id: String,
    options: GpuViewportOptions,
    factory: Option<Factory>,
    redraw: GpuRedrawHandle,
}

/// Declarations only: constructing a registry does not initialize the GPU.
#[derive(Default)]
pub struct GpuViewportRegistry {
    entries: Vec<Registration>,
    pub device_options: GpuDeviceOptions,
}

impl GpuViewportRegistry {
    pub fn register<F>(
        &mut self,
        element_id: impl Into<String>,
        options: GpuViewportOptions,
        factory: F,
    ) -> GpuRedrawHandle
    where
        F: FnOnce(GpuViewportInit<'_>) -> Result<Box<dyn GpuViewportRenderer>> + 'static,
    {
        let redraw = GpuRedrawHandle::default();
        self.entries.push(Registration {
            id: element_id.into(),
            options,
            factory: Some(Box::new(factory)),
            redraw: redraw.clone(),
        });
        redraw
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Validate before creating a native host, including inactive registrations.
    pub fn validate(&self) -> Result<()> {
        let mut seen = std::collections::HashSet::new();
        for entry in &self.entries {
            if entry.id.is_empty() || !seen.insert(&entry.id) {
                return Err(error(format!(
                    "empty or duplicate GPU viewport ID: {:?}",
                    entry.id
                )));
            }
        }
        Ok(())
    }
}

pub(crate) fn error(message: impl std::fmt::Display) -> RendererError {
    RendererError::Surface(format!("GPU viewport: {message}"))
}

pub(crate) fn input_trace_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var_os("CSSIMPLER_GPU_INPUT_TRACE").is_some()
            || std::env::args_os().any(|arg| arg == "--trace-input")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duplicate_bindings_fail_without_initializing_a_device() {
        let mut registry = GpuViewportRegistry::default();
        registry.register("scene", GpuViewportOptions::default(), |_| {
            panic!("must remain lazy")
        });
        registry.register("scene", GpuViewportOptions::default(), |_| {
            panic!("must remain lazy")
        });
        assert!(registry.validate().is_err());
    }
    #[test]
    fn redraw_requests_coalesce_and_wake() {
        let handle = GpuRedrawHandle::default();
        assert!(handle.take_dirty());
        assert!(!handle.take_dirty());
        let awake = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let out = awake.clone();
        handle.set_wake(Box::new(move || {
            out.fetch_add(1, Ordering::AcqRel);
        }));
        for _ in 0..1000 {
            handle.request_redraw();
        }
        assert_eq!(awake.load(Ordering::Acquire), 1);
        assert!(handle.take_dirty());
        handle.request_redraw();
        assert_eq!(awake.load(Ordering::Acquire), 2);
    }
}
