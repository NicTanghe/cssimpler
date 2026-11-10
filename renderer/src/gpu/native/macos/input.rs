//! Coalesce native drag/up events that vibrancy can swallow before winit.
use super::*;
use block2::RcBlock;
use objc2::runtime::AnyClass;
use std::{cell::RefCell, ptr::NonNull, rc::Rc};

pub(super) struct InputMonitor {
    token: Retained<AnyObject>,
    primary_release: Rc<Cell<Option<crate::KeyboardModifiers>>>,
    motion: Rc<Cell<Option<(crate::PointerPosition, crate::KeyboardModifiers)>>>,
    captured: Rc<Cell<bool>>,
    redraw: Rc<RefCell<Option<crate::gpu::GpuRedrawHandle>>>,
}

impl InputMonitor {
    pub fn install(parent: &Window) -> Result<Self> {
        let trace = crate::gpu::input_trace_enabled();
        let _main = main_thread()?;
        let RawWindowHandle::AppKit(handle) = parent.window_handle().map_err(error)?.as_raw()
        else {
            return Err(error("expected AppKit parent for input monitor"));
        };
        // SAFETY: winit owns the live view; this monitor stays on the main thread.
        let view: &AnyObject = unsafe { handle.ns_view.cast().as_ref() };
        let view = unsafe { Retained::retain(view as *const _ as *mut AnyObject) }
            .ok_or_else(|| error("failed to retain AppKit input view"))?;
        let window: *mut AnyObject = unsafe { msg_send![&*view, window] };
        let window_key = window as usize;
        let class = AnyClass::get(c"NSEvent").ok_or_else(|| error("NSEvent unavailable"))?;
        let phase = Rc::new(Cell::new(0usize));
        let primary_release = Rc::new(Cell::new(None));
        let release = primary_release.clone();
        let motion = Rc::new(Cell::new(None));
        let native_motion = motion.clone();
        let captured = Rc::new(Cell::new(false));
        let active = captured.clone();
        let redraw = Rc::new(RefCell::new(None::<crate::gpu::GpuRedrawHandle>));
        let wake = redraw.clone();
        let handler = RcBlock::new(move |event: NonNull<AnyObject>| -> *mut AnyObject {
            // SAFETY: AppKit supplies a live NSEvent and invokes local monitors
            // on the main thread. Do not retain, consume, or modify the event.
            let event_ref = unsafe { event.as_ref() };
            let source_window: *mut AnyObject = unsafe { msg_send![event_ref, window] };
            if source_window as usize != window_key {
                return event.as_ptr();
            }
            let kind: usize = unsafe { msg_send![event_ref, type] };
            if kind == 2 || (kind == 6 && active.get()) {
                let flags: usize = unsafe { msg_send![event_ref, modifierFlags] };
                let modifiers = crate::KeyboardModifiers {
                    shift: flags & (1 << 17) != 0,
                    control: flags & (1 << 18) != 0,
                    alt: flags & (1 << 19) != 0,
                    super_key: flags & (1 << 20) != 0,
                };
                if kind == 2 {
                    release.set(Some(modifiers));
                } else {
                    let point: CGPoint = unsafe { msg_send![event_ref, locationInWindow] };
                    let point: CGPoint = unsafe {
                        msg_send![&*view, convertPoint: point, fromView: None::<&AnyObject>]
                    };
                    native_motion.set(Some((
                        crate::PointerPosition {
                            x: point.x as f32,
                            y: point.y as f32,
                        },
                        modifiers,
                    )));
                }
                if active.get() {
                    if let Some(redraw) = wake.borrow().as_ref() {
                        redraw.request_redraw();
                    }
                }
            }
            if !trace {
                return event.as_ptr();
            }
            let timestamp: f64 = unsafe { msg_send![event_ref, timestamp] };
            match kind {
                1 | 2 => {
                    eprintln!(
                        "GPU input native: {} t={timestamp:.6}",
                        if kind == 1 {
                            "primary-down"
                        } else {
                            "primary-up"
                        }
                    );
                    if kind == 1 {
                        if let Some(window_ref) = unsafe { source_window.as_ref() } {
                            let background_drag: objc2::runtime::Bool =
                                unsafe { msg_send![window_ref, isMovableByWindowBackground] };
                            eprintln!(
                                "GPU input native: background_window_drag={background_drag:?}"
                            );
                        }
                    }
                    phase.set(kind);
                }
                5 | 6 if phase.get() != kind => {
                    eprintln!(
                        "GPU input native: {} t={timestamp:.6}",
                        if kind == 5 { "hover" } else { "drag" }
                    );
                    phase.set(kind);
                }
                22 if phase.get() != kind => {
                    eprintln!("GPU input native: scroll t={timestamp:.6}");
                    phase.set(kind);
                }
                _ => {}
            }
            event.as_ptr()
        });
        let mask: usize = (1 << 2)
            | (1 << 6)
            | if trace {
                (1 << 1) | (1 << 5) | (1 << 6) | (1 << 22)
            } else {
                0
            };
        // SAFETY: NSEvent expects a block NSEvent*(NSEvent*). AppKit copies it;
        // removing the monitor releases that copy before the host window drops.
        let token: Option<Retained<AnyObject>> = unsafe {
            msg_send![class, addLocalMonitorForEventsMatchingMask: mask, handler: &*handler]
        };
        let token = token.ok_or_else(|| error("failed to install local input monitor"))?;
        if trace {
            eprintln!("GPU input trace installed (AppKit source, winit delivery, viewport)");
        }
        Ok(Self {
            token,
            primary_release,
            motion,
            captured,
            redraw,
        })
    }

    pub fn take_primary_release(&self) -> Option<crate::KeyboardModifiers> {
        self.primary_release.take()
    }

    pub fn take_motion(&self) -> Option<(crate::PointerPosition, crate::KeyboardModifiers)> {
        self.motion.take()
    }

    pub fn set_redraw(&self, redraw: crate::gpu::GpuRedrawHandle) {
        *self.redraw.borrow_mut() = Some(redraw);
    }

    pub fn set_captured(&self, captured: bool) {
        self.captured.set(captured);
        if !captured {
            self.motion.take();
        }
    }
}

impl Drop for InputMonitor {
    fn drop(&mut self) {
        if let Some(class) = AnyClass::get(c"NSEvent") {
            // SAFETY: token came from addLocalMonitor; the host drops on main.
            let _: () = unsafe { msg_send![class, removeMonitor: &*self.token] };
        }
    }
}
