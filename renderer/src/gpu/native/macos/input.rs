//! Coalesce native drag/up events that vibrancy can swallow before winit.
use super::*;
use block2::RcBlock;
use objc2::runtime::AnyClass;
use std::{cell::RefCell, ptr::NonNull, rc::Rc};

pub(super) struct InputMonitor {
    token: Retained<AnyObject>,
    pointer: Rc<NativePointerState>,
    redraw: Rc<RefCell<Option<crate::gpu::GpuRedrawHandle>>>,
}

#[derive(Default)]
struct NativePointerState {
    primary_release: Cell<Option<crate::KeyboardModifiers>>,
    motion: Cell<Option<(crate::PointerPosition, crate::KeyboardModifiers)>>,
    captured: Cell<bool>,
    windowed: Cell<bool>,
    system_gesture: Cell<Option<crate::KeyboardModifiers>>,
}

impl NativePointerState {
    fn yield_to_system(&self, modifiers: crate::KeyboardModifiers) -> bool {
        if !crate::input::system_pointer_gesture(
            self.windowed.get(),
            modifiers,
            crate::runtime::system_drag_override_active(),
        ) {
            return false;
        }
        self.motion.take();
        self.primary_release.take();
        if self.captured.replace(false) {
            self.system_gesture.set(Some(modifiers));
        }
        true
    }
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
        let pointer = Rc::new(NativePointerState::default());
        pointer.windowed.set(parent.fullscreen().is_none());
        let native_pointer = pointer.clone();
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
            // flagsChanged also cancels an existing drag when a window manager
            // takes over, even if winit never receives that modifier change.
            if matches!(kind, 2 | 6 | 12) {
                let flags: usize = unsafe { msg_send![event_ref, modifierFlags] };
                let modifiers = crate::KeyboardModifiers {
                    shift: flags & (1 << 17) != 0,
                    control: flags & (1 << 18) != 0,
                    alt: flags & (1 << 19) != 0,
                    super_key: flags & (1 << 20) != 0,
                };
                let was_captured = native_pointer.captured.get();
                let reserved = native_pointer.yield_to_system(modifiers);
                if !reserved && kind == 2 {
                    native_pointer.primary_release.set(Some(modifiers));
                } else if !reserved && kind == 6 && was_captured {
                    let point: CGPoint = unsafe { msg_send![event_ref, locationInWindow] };
                    let point: CGPoint = unsafe {
                        msg_send![&*view, convertPoint: point, fromView: None::<&AnyObject>]
                    };
                    native_pointer.motion.set(Some((
                        crate::PointerPosition {
                            x: point.x as f32,
                            y: point.y as f32,
                        },
                        modifiers,
                    )));
                }
                if was_captured && (reserved || kind == 2 || kind == 6) {
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
                    let flags: usize = unsafe { msg_send![event_ref, modifierFlags] };
                    eprintln!(
                        "GPU input native: {} t={timestamp:.6} flags={flags:#x}",
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
                            let movable: objc2::runtime::Bool =
                                unsafe { msg_send![window_ref, isMovable] };
                            let style: usize = unsafe { msg_send![window_ref, styleMask] };
                            eprintln!(
                                "GPU input native: background_window_drag={background_drag:?} movable={movable:?} style={style:#x}"
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
            | (1 << 12)
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
            pointer,
            redraw,
        })
    }

    pub fn take_primary_release(&self) -> Option<crate::KeyboardModifiers> {
        self.pointer.primary_release.take()
    }

    pub fn take_motion(&self) -> Option<(crate::PointerPosition, crate::KeyboardModifiers)> {
        self.pointer.motion.take()
    }

    pub fn take_system_gesture(&self) -> Option<crate::KeyboardModifiers> {
        self.pointer.system_gesture.take()
    }

    pub fn set_windowed(&self, windowed: bool) {
        self.pointer.windowed.set(windowed);
    }

    pub fn set_redraw(&self, redraw: crate::gpu::GpuRedrawHandle) {
        *self.redraw.borrow_mut() = Some(redraw);
    }

    pub fn set_captured(&self, captured: bool) {
        self.pointer.captured.set(captured);
        if !captured {
            self.pointer.motion.take();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_takeover_discards_buffered_motion_and_stops_native_capture() {
        let pointer = NativePointerState::default();
        pointer.windowed.set(true);
        pointer.captured.set(true);
        pointer.motion.set(Some((
            crate::PointerPosition { x: 90.0, y: 20.0 },
            Default::default(),
        )));
        pointer.primary_release.set(Some(Default::default()));
        let modifiers = crate::KeyboardModifiers {
            control: true,
            super_key: true,
            ..Default::default()
        };
        assert!(pointer.yield_to_system(modifiers));
        assert!(!pointer.captured.get());
        assert_eq!(pointer.motion.take(), None);
        assert_eq!(pointer.primary_release.take(), None);
        assert_eq!(pointer.system_gesture.take(), Some(modifiers));
        assert!(!pointer.yield_to_system(Default::default()));
        assert!(
            !pointer.captured.get(),
            "modifier release must not restart a drag"
        );
        assert!(pointer.yield_to_system(modifiers));
        assert_eq!(pointer.system_gesture.take(), None, "wake only on takeover");

        pointer.windowed.set(false);
        pointer.captured.set(true);
        assert!(!pointer.yield_to_system(modifiers));
        assert!(
            pointer.captured.get(),
            "fullscreen modifier gestures belong to the app"
        );
    }
}
