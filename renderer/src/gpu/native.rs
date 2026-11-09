//! Native hosts do not own a second input or event loop.
use super::{Result, error};
use std::sync::Arc;
use winit::{event_loop::ActiveEventLoop, window::Window};

#[cfg(windows)]
pub(super) use win32::NativeHost;

#[cfg(windows)]
mod win32 {
    use super::*;
    use std::{
        cell::{Cell, RefCell},
        collections::HashMap,
        rc::{Rc, Weak},
    };
    use windows::{
        Win32::{Foundation::HWND, Graphics::DirectComposition::*},
        core::Interface,
    };
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    // One target per parent HWND, with independently presented view surfaces.
    thread_local! { static ROOTS: RefCell<HashMap<isize, Weak<CompositionRoot>>> = RefCell::new(HashMap::new()); }
    struct CompositionRoot {
        device: IDCompositionDevice,
        _target: IDCompositionTarget,
        visual: IDCompositionVisual,
        _parent: Arc<Window>,
    }

    pub(crate) struct NativeHost {
        visual: IDCompositionVisual,
        root: Rc<CompositionRoot>,
        shown: Cell<bool>,
    }

    impl NativeHost {
        pub fn new(_: &ActiveEventLoop, parent: Arc<Window>, _: bool) -> Result<Self> {
            let RawWindowHandle::Win32(raw) = parent.window_handle().map_err(error)?.as_raw()
            else {
                return Err(error("expected Win32 parent"));
            };
            let key = raw.hwnd.get();
            let root = ROOTS.with(|roots| -> Result<Rc<CompositionRoot>> {
                let mut roots = roots.borrow_mut();
                roots.retain(|_, weak| weak.strong_count() > 0);
                if let Some(root) = roots.get(&key).and_then(Weak::upgrade) {
                    return Ok(root);
                }
                // SAFETY: parent remains live; DComp objects stay on this thread.
                let root = unsafe {
                    let device: IDCompositionDevice =
                        DCompositionCreateDevice(None).map_err(error)?;
                    let target = device
                        .CreateTargetForHwnd(HWND(key as *mut _), true)
                        .map_err(error)?;
                    let visual = device.CreateVisual().map_err(error)?;
                    target.SetRoot(&visual).map_err(error)?;
                    device.Commit().map_err(error)?;
                    Rc::new(CompositionRoot {
                        device,
                        _target: target,
                        visual,
                        _parent: parent,
                    })
                };
                roots.insert(key, Rc::downgrade(&root));
                Ok(root)
            })?;
            let visual = unsafe { root.device.CreateVisual() }.map_err(error)?;
            Ok(Self {
                visual,
                root,
                shown: Cell::new(false),
            })
        }
        pub fn target(&self) -> Result<wgpu::SurfaceTargetUnsafe> {
            // wgpu retains the COM visual. LiveViewport drops the surface first.
            Ok(wgpu::SurfaceTargetUnsafe::CompositionVisual(
                self.visual.as_raw(),
            ))
        }
        pub fn place(&self, origin: [i32; 2], _: [u32; 2]) -> Result<()> {
            unsafe {
                self.visual.SetOffsetX2(origin[0] as f32).map_err(error)?;
                self.visual.SetOffsetY2(origin[1] as f32).map_err(error)?;
                if !self.shown.get() {
                    self.root
                        .visual
                        .AddVisual(&self.visual, true, None)
                        .map_err(error)?;
                }
                // Includes swapchain SetContent performed by wgpu configuration.
                self.root.device.Commit().map_err(error)?;
            }
            self.shown.set(true);
            Ok(())
        }
        pub fn hide(&self) {
            if self.shown.replace(false) {
                unsafe {
                    let _ = self.root.visual.RemoveVisual(&self.visual);
                    let _ = self.root.device.Commit();
                }
            }
        }
        pub fn commit(&self) -> Result<()> {
            unsafe { self.root.device.Commit() }.map_err(error)
        }
    }
    impl Drop for NativeHost {
        fn drop(&mut self) {
            self.hide();
        }
    }
}

#[cfg(not(windows))]
pub(super) struct NativeHost;
#[cfg(not(windows))]
impl NativeHost {
    pub fn new(_: &ActiveEventLoop, _: Arc<Window>, _: bool) -> Result<Self> {
        Err(error(
            "native GPU embedding is not implemented on this platform yet",
        ))
    }
    pub fn target(&self) -> Result<wgpu::SurfaceTargetUnsafe> {
        Err(error("native GPU host unavailable"))
    }
    pub fn place(&self, _: [i32; 2], _: [u32; 2]) -> Result<()> {
        Ok(())
    }
    pub fn hide(&self) {}
    pub fn commit(&self) -> Result<()> {
        Ok(())
    }
}
