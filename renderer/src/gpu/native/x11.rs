//! X11 child windows share the parent's lifecycle and leave input to winit.
use super::*;
use std::{cell::Cell, num::NonZeroU32, ptr::NonNull};
use winit::raw_window_handle::{
    HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle, XcbDisplayHandle,
    XcbWindowHandle,
};
use x11rb::{
    connection::Connection,
    protocol::{
        composite::{ConnectionExt as _, Redirect},
        render::ConnectionExt as _,
        shape::{ConnectionExt as _, SK, SO},
        xproto::{
            ClipOrdering, ColormapAlloc, ConfigureWindowAux, ConnectionExt as _, CreateWindowAux,
            WindowClass,
        },
    },
    xcb_ffi::XCBConnection,
};

pub(crate) struct NativeHost {
    connection: XCBConnection,
    window: u32,
    colormap: Option<u32>,
    screen: i32,
    shown: Cell<bool>,
    placement: Cell<Option<([i32; 2], [u32; 2])>>,
    // The borrowed XCB connection and parent XID must outlive this host.
    _parent: Arc<Window>,
}

impl NativeHost {
    pub fn new(_: &ActiveEventLoop, parent: Arc<Window>, transparent: bool) -> Result<Self> {
        let display = parent.display_handle().map_err(error)?.as_raw();
        let window = parent.window_handle().map_err(error)?.as_raw();
        let (connection, screen, parent_id) = match (display, window) {
            (RawDisplayHandle::Xlib(display), RawWindowHandle::Xlib(window)) => {
                let display_ptr = display
                    .display
                    .ok_or_else(|| error("missing X11 display"))?;
                let xlib_xcb = x11_dl::xlib_xcb::Xlib_xcb::open().map_err(error)?;
                // SAFETY: winit owns this live Xlib display. Do not change its event queue owner.
                let connection =
                    unsafe { (xlib_xcb.XGetXCBConnection)(display_ptr.as_ptr().cast()) };
                (connection, display.screen, window.window as u32)
            }
            (RawDisplayHandle::Xcb(display), RawWindowHandle::Xcb(window)) => (
                display
                    .connection
                    .ok_or_else(|| error("missing XCB connection"))?
                    .as_ptr(),
                display.screen,
                window.window.get(),
            ),
            _ => {
                return Err(error(
                    "Linux GPU viewports currently require X11; unset WAYLAND_DISPLAY and WAYLAND_SOCKET to use XWayland",
                ));
            }
        };
        // SAFETY: parent retains the display. This wrapper neither owns the connection nor polls events.
        let connection =
            unsafe { XCBConnection::from_raw_xcb_connection(connection, false) }.map_err(error)?;
        let shape = connection
            .shape_query_version()
            .map_err(error)?
            .reply()
            .map_err(error)?;
        if (shape.major_version, shape.minor_version) < (1, 1) {
            return Err(error("GPU child input routing requires X Shape 1.1"));
        }
        let mut host = Self {
            connection,
            window: 0,
            colormap: None,
            screen,
            shown: Cell::new(false),
            placement: Cell::new(None),
            _parent: parent,
        };
        let mut attributes = CreateWindowAux::new().background_pixel(0).border_pixel(0);
        let (depth, visual) = if transparent {
            let version = host
                .connection
                .composite_query_version(0, 4)
                .map_err(error)?
                .reply()
                .map_err(error)?;
            if (version.major_version, version.minor_version) < (0, 4) {
                return Err(error("transparent GPU viewports require X Composite 0.4"));
            }
            let formats = host
                .connection
                .render_query_pict_formats()
                .map_err(error)?
                .reply()
                .map_err(error)?;
            let screen_formats = formats
                .screens
                .get(screen as usize)
                .ok_or_else(|| error("invalid X11 screen"))?;
            let visual = screen_formats
                .depths
                .iter()
                .filter(|d| d.depth == 32)
                .flat_map(|d| &d.visuals)
                .find(|v| {
                    formats
                        .formats
                        .iter()
                        .any(|f| f.id == v.format && f.direct.alpha_mask != 0)
                })
                .ok_or_else(|| {
                    error("X11 screen has no ARGB visual for transparent GPU viewports")
                })?
                .visual;
            let colormap = host.connection.generate_id().map_err(error)?;
            host.connection
                .create_colormap(ColormapAlloc::NONE, colormap, parent_id, visual)
                .map_err(error)?
                .check()
                .map_err(error)?;
            host.colormap = Some(colormap);
            attributes = attributes.colormap(colormap);
            (32, visual)
        } else {
            (0, 0) // CopyFromParent
        };
        let child = host.connection.generate_id().map_err(error)?;
        host.connection
            .create_window(
                depth,
                child,
                parent_id,
                0,
                0,
                1,
                1,
                0,
                WindowClass::INPUT_OUTPUT,
                visual,
                &attributes,
            )
            .map_err(error)?
            .check()
            .map_err(error)?;
        host.window = child;
        // An empty input shape lets orbit, wheel, drag and resize reach the parent.
        host.connection
            .shape_rectangles(SO::SET, SK::INPUT, ClipOrdering::UNSORTED, child, 0, 0, &[])
            .map_err(error)?
            .check()
            .map_err(error)?;
        if transparent {
            // Preserve the CPU backing pixels beneath the child and let the server blend ARGB.
            host.connection
                .composite_redirect_window(child, Redirect::AUTOMATIC)
                .map_err(error)?
                .check()
                .map_err(error)?;
        }
        host.commit()?;
        Ok(host)
    }

    pub fn target(&self) -> Result<wgpu::SurfaceTargetUnsafe> {
        let display = XcbDisplayHandle::new(
            NonNull::new(self.connection.get_raw_xcb_connection()),
            self.screen,
        );
        let window = XcbWindowHandle::new(NonZeroU32::new(self.window).unwrap());
        Ok(wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: Some(RawDisplayHandle::Xcb(display)),
            raw_window_handle: RawWindowHandle::Xcb(window),
        })
    }

    pub fn place(&self, origin: [i32; 2], size: [u32; 2]) -> Result<()> {
        if self.placement.get() != Some((origin, size)) {
            self.connection
                .configure_window(
                    self.window,
                    &ConfigureWindowAux::new()
                        .x(origin[0])
                        .y(origin[1])
                        .width(size[0])
                        .height(size[1]),
                )
                .map_err(error)?
                .check()
                .map_err(error)?;
            self.placement.set(Some((origin, size)));
        }
        if !self.shown.get() {
            self.connection
                .map_window(self.window)
                .map_err(error)?
                .check()
                .map_err(error)?;
            self.shown.set(true);
        }
        self.commit()
    }

    pub fn hide(&self) {
        if self.shown.replace(false) {
            let _ = self.connection.unmap_window(self.window);
            let _ = self.connection.flush();
        }
    }

    pub fn commit(&self) -> Result<()> {
        self.connection.flush().map_err(error)
    }
}

impl Drop for NativeHost {
    fn drop(&mut self) {
        if self.window != 0 {
            let _ = self.connection.destroy_window(self.window);
        }
        if let Some(colormap) = self.colormap {
            let _ = self.connection.free_colormap(colormap);
        }
        let _ = self.connection.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::{
        application::ApplicationHandler, event::WindowEvent, event_loop::EventLoop,
        platform::x11::EventLoopBuilderExtX11, window::WindowId,
    };
    use x11rb::protocol::xproto::MapState;

    #[test]
    #[ignore = "requires a live X11 server with Shape, Render and Composite extensions"]
    fn x11_host_lifecycle() {
        struct Check;
        impl ApplicationHandler for Check {
            fn resumed(&mut self, event_loop: &ActiveEventLoop) {
                let parent = Arc::new(
                    event_loop
                        .create_window(Window::default_attributes().with_visible(false))
                        .unwrap(),
                );
                for transparent in [false, true] {
                    let host = NativeHost::new(event_loop, parent.clone(), transparent).unwrap();
                    host.place([17, 23], [320, 180]).unwrap();
                    let geometry = host
                        .connection
                        .get_geometry(host.window)
                        .unwrap()
                        .reply()
                        .unwrap();
                    assert_eq!(
                        (geometry.x, geometry.y, geometry.width, geometry.height),
                        (17, 23, 320, 180)
                    );
                    if transparent {
                        assert_eq!(geometry.depth, 32);
                    }
                    let input = host
                        .connection
                        .shape_get_rectangles(host.window, SK::INPUT)
                        .unwrap()
                        .reply()
                        .unwrap();
                    assert!(input.rectangles.is_empty());
                    host.hide();
                    assert_eq!(
                        host.connection
                            .get_window_attributes(host.window)
                            .unwrap()
                            .reply()
                            .unwrap()
                            .map_state,
                        MapState::UNMAPPED
                    );
                    host.place([31, 41], [640, 360]).unwrap();
                    let geometry = host
                        .connection
                        .get_geometry(host.window)
                        .unwrap()
                        .reply()
                        .unwrap();
                    assert_eq!(
                        (geometry.x, geometry.y, geometry.width, geometry.height),
                        (31, 41, 640, 360)
                    );
                    let child = host.window;
                    drop(host);
                    // A separate wrapper observes that destruction was sent before parent release.
                    let probe = NativeHost::new(event_loop, parent.clone(), transparent).unwrap();
                    assert!(
                        probe
                            .connection
                            .get_geometry(child)
                            .unwrap()
                            .reply()
                            .is_err()
                    );
                }
                event_loop.exit();
            }

            fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
        }
        let event_loop = EventLoop::builder()
            .with_x11()
            .with_any_thread(true)
            .build()
            .unwrap();
        event_loop.run_app(&mut Check).unwrap();
    }
}
