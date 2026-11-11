//! Metal sublayers share AppKit's composition tree without intercepting input.
use super::*;
use objc2::{
    MainThreadMarker, msg_send,
    rc::Retained,
    runtime::{AnyObject, Bool},
};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_foundation::ns_string;
use objc2_quartz_core::{CALayer, CAMetalLayer, CATransaction};
use std::cell::Cell;
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

#[path = "macos/input.rs"]
mod input;

pub(crate) struct NativeHost {
    layer: Retained<CAMetalLayer>,
    placement: Cell<Option<Placement>>,
    input: input::InputMonitor,
    // Keeps the NSView/NSWindow alive until after the Metal surface is dropped.
    parent: Arc<Window>,
}

#[derive(Clone, Copy, PartialEq)]
struct Placement {
    root: *const CALayer,
    frame: CGRect,
    scale: f64,
    size: [u32; 2],
}

impl NativeHost {
    pub fn new(_: &ActiveEventLoop, parent: Arc<Window>, transparent: bool) -> Result<Self> {
        let _main = main_thread()?;
        let layer = CAMetalLayer::new();
        layer.setName(Some(ns_string!("cssimpler.gpu.viewport")));
        layer.setOpaque(!transparent);
        layer.setHidden(true);
        // Above the native-glass tint and CPU overlay (z = 999999 and 1000000).
        // Geometry validation hides viewports occluded by later-painted CPU UI.
        layer.setZPosition(1_000_001.0);
        let host = Self {
            input: input::InputMonitor::install(&parent)?,
            layer,
            parent,
            placement: Cell::new(None),
        };
        host.content_layer()?.addSublayer(&host.layer);
        Ok(host)
    }

    fn content_layer(&self) -> Result<Retained<CALayer>> {
        let RawWindowHandle::AppKit(handle) = self.parent.window_handle().map_err(error)?.as_raw()
        else {
            return Err(error("expected AppKit parent"));
        };
        // SAFETY: winit supplies a live NSView, retained by parent. All AppKit
        // access happens on the event-loop/main thread.
        let view: &AnyObject = unsafe { handle.ns_view.cast().as_ref() };
        let window: Option<Retained<AnyObject>> = unsafe { msg_send![view, window] };
        let window = window.ok_or_else(|| error("AppKit view has no window"))?;
        let content: Option<Retained<AnyObject>> = unsafe { msg_send![&*window, contentView] };
        let content = content.ok_or_else(|| error("AppKit window has no content view"))?;
        // Share the CPU presenter's root, above its glass tint/content layers.
        // Keep winit's content view and input routing intact.
        let _: () = unsafe { msg_send![&*content, setWantsLayer: Bool::YES] };
        let root: Option<Retained<CALayer>> = unsafe { msg_send![&*content, layer] };
        root.ok_or_else(|| error("AppKit content view has no backing layer"))
    }

    pub fn take_primary_release(&self) -> Option<crate::KeyboardModifiers> {
        self.input.take_primary_release()
    }

    pub fn take_native_motion(&self) -> Option<(crate::PointerPosition, crate::KeyboardModifiers)> {
        self.input.take_motion()
    }

    pub fn take_system_gesture(&self) -> Option<crate::KeyboardModifiers> {
        self.input.take_system_gesture()
    }

    pub fn set_input_redraw(&self, redraw: crate::gpu::GpuRedrawHandle) {
        self.input.set_redraw(redraw);
    }

    pub fn set_primary_capture(&self, captured: bool) {
        self.input.set_captured(captured);
    }

    pub fn target(&self) -> Result<wgpu::SurfaceTargetUnsafe> {
        Ok(wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(
            Retained::as_ptr(&self.layer).cast_mut().cast(),
        ))
    }

    pub fn place(&self, origin: [i32; 2], size: [u32; 2]) -> Result<()> {
        let _main = main_thread()?;
        self.input.set_windowed(self.parent.fullscreen().is_none());
        let root = self.content_layer()?;
        let scale = self.parent.scale_factor();
        let frame = layer_frame(root.bounds(), root.isGeometryFlipped(), origin, size, scale);
        let placement = Placement {
            root: Retained::as_ptr(&root),
            frame,
            scale,
            size,
        };
        if self.placement.get() == Some(placement) && !self.layer.isHidden() {
            return Ok(());
        }
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        // Reparent when native glass is enabled/disabled during the window's life.
        if self.layer.superlayer().as_deref() != Some(&*root) {
            self.layer.removeFromSuperlayer();
            root.addSublayer(&self.layer);
        }
        self.layer.setContentsScale(scale);
        self.layer.setFrame(frame);
        self.layer
            .setDrawableSize(CGSize::new(size[0] as f64, size[1] as f64));
        self.layer.setHidden(false);
        CATransaction::commit();
        self.placement.set(Some(placement));
        Ok(())
    }

    pub fn hide(&self) {
        self.input.set_captured(false);
        if self.layer.isHidden() {
            return;
        }
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        self.layer.setHidden(true);
        CATransaction::commit();
    }

    pub fn commit(&self) -> Result<()> {
        // Publish geometry before wgpu acquires a drawable, including on resize.
        CATransaction::flush();
        Ok(())
    }
}

impl Drop for NativeHost {
    fn drop(&mut self) {
        self.layer.removeFromSuperlayer();
    }
}

fn main_thread() -> Result<MainThreadMarker> {
    MainThreadMarker::new().ok_or_else(|| error("AppKit GPU hosts require the main thread"))
}

fn layer_frame(
    bounds: CGRect,
    flipped: bool,
    origin: [i32; 2],
    size: [u32; 2],
    scale: f64,
) -> CGRect {
    let x = origin[0] as f64 / scale;
    let y = origin[1] as f64 / scale;
    let width = size[0] as f64 / scale;
    let height = size[1] as f64 / scale;
    CGRect {
        origin: CGPoint::new(
            bounds.origin.x + x,
            bounds.origin.y
                + if flipped {
                    y
                } else {
                    bounds.size.height - y - height
                },
        ),
        size: CGSize::new(width, height),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_viewport_maps_to_points_in_both_layer_orientations() {
        let bounds = CGRect::new(CGPoint::new(3.0, 7.0), CGSize::new(800.0, 600.0));
        for scale in [1.0, 1.5, 2.0] {
            let origin = [(20.0 * scale) as i32, (30.0 * scale) as i32];
            let size = [(320.0 * scale) as u32, (180.0 * scale) as u32];
            for (flipped, y) in [(true, 37.0), (false, 397.0)] {
                let frame = layer_frame(bounds, flipped, origin, size, scale);
                assert_eq!(frame.origin, CGPoint::new(23.0, y));
                assert_eq!(frame.size, CGSize::new(320.0, 180.0));
            }
        }
    }
}
