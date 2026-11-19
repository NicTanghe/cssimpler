//! Wayland subsurfaces share winit's connection and leave input to the parent.
use super::*;
use std::cell::{Cell, RefCell};
use wayland_backend::client::{Backend, ObjectId};
use wayland_client::{
    Connection, EventQueue, Proxy, QueueHandle, delegate_noop,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{
        wl_compositor::WlCompositor, wl_region::WlRegion, wl_registry::WlRegistry,
        wl_subcompositor::WlSubcompositor, wl_subsurface::WlSubsurface, wl_surface::WlSurface,
    },
};
use wayland_protocols::wp::viewporter::client::{
    wp_viewport::WpViewport, wp_viewporter::WpViewporter,
};
use winit::raw_window_handle::{
    HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle,
    WaylandWindowHandle,
};

/// Dispatch target for the guest queue. The host consumes no events.
struct Guest;

impl wayland_client::Dispatch<WlRegistry, GlobalListContents> for Guest {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
delegate_noop!(Guest: ignore WlCompositor);
delegate_noop!(Guest: ignore WlSubcompositor);
delegate_noop!(Guest: ignore WlSubsurface);
delegate_noop!(Guest: ignore WlRegion);
delegate_noop!(Guest: ignore WlSurface);
delegate_noop!(Guest: ignore WpViewporter);
delegate_noop!(Guest: ignore WpViewport);

pub(crate) struct NativeHost {
    connection: Connection,
    // Owns the guest objects' wl_event_queue; winit's queue stays untouched.
    queue: RefCell<EventQueue<Guest>>,
    queue_handle: QueueHandle<Guest>,
    subcompositor: WlSubcompositor,
    parent_surface: WlSurface,
    surface: WlSurface,
    viewport: Option<WpViewport>,
    subsurface: RefCell<Option<WlSubsurface>>,
    placement: Cell<Option<Placement>>,
    // The borrowed wl_display and parent wl_surface must outlive this host.
    parent: Arc<Window>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Placement {
    position: [i32; 2],
    destination: [i32; 2],
}

impl NativeHost {
    pub fn new(_: &ActiveEventLoop, parent: Arc<Window>, _transparent: bool) -> Result<Self> {
        // Wayland composes premultiplied buffer alpha natively, so transparent and
        // opaque views share one setup. Opaque swapchains present alpha-less formats.
        let (display, window) = wayland_handles(&parent)?;
        // SAFETY: winit owns the live wl_display. The parent Arc keeps it alive, and a
        // foreign backend neither closes the display nor reads winit's queue.
        let backend = unsafe { Backend::from_foreign_display(display.display.as_ptr().cast()) };
        let connection = Connection::from_backend(backend);
        let (globals, queue) = registry_queue_init::<Guest>(&connection).map_err(error)?;
        let queue_handle = queue.handle();
        let compositor: WlCompositor = globals.bind(&queue_handle, 1..=4, ()).map_err(error)?;
        let subcompositor: WlSubcompositor =
            globals.bind(&queue_handle, 1..=1, ()).map_err(error)?;
        // Required for HiDPI: subsurface positions and sizes are logical units.
        let viewporter: Option<WpViewporter> = globals.bind(&queue_handle, 1..=1, ()).ok();
        // SAFETY: the pointer is winit's live wl_surface proxy for the parent window.
        let parent_id =
            unsafe { ObjectId::from_ptr(WlSurface::interface(), window.surface.as_ptr().cast()) }
                .map_err(error)?;
        let parent_surface = WlSurface::from_id(&connection, parent_id).map_err(error)?;
        let surface = compositor.create_surface(&queue_handle, ());
        // An empty input region lets orbit, wheel, drag and resize reach the parent.
        let region = compositor.create_region(&queue_handle, ());
        surface.set_input_region(Some(&region));
        region.destroy();
        let viewport = viewporter
            .as_ref()
            .map(|viewporter| viewporter.get_viewport(&surface, &queue_handle, ()));
        if let Some(viewporter) = viewporter {
            viewporter.destroy();
        }
        let host = Self {
            connection,
            queue: RefCell::new(queue),
            queue_handle,
            subcompositor,
            parent_surface,
            surface,
            viewport,
            subsurface: RefCell::new(None),
            placement: Cell::new(None),
            parent,
        };
        host.commit()?;
        Ok(host)
    }

    pub fn target(&self) -> Result<wgpu::SurfaceTargetUnsafe> {
        let (display, _) = wayland_handles(&self.parent)?;
        let surface = std::ptr::NonNull::new(self.surface.id().as_ptr().cast())
            .ok_or_else(|| error("missing Wayland viewport surface"))?;
        Ok(wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: Some(RawDisplayHandle::Wayland(display)),
            raw_window_handle: RawWindowHandle::Wayland(WaylandWindowHandle::new(surface)),
        })
    }

    pub fn place(&self, origin: [i32; 2], size: [u32; 2]) -> Result<()> {
        let placement = logical_placement(origin, size, self.parent.scale_factor());
        if self.viewport.is_none() && placement.destination != size.map(|v| v as i32) {
            return Err(error(
                "scaled Wayland GPU viewports require the wp_viewporter protocol",
            ));
        }
        let mut subsurface = self.subsurface.borrow_mut();
        if subsurface.is_none() {
            // The child stays a subsurface while shown. Recreating the role after
            // hide() maps it again; the next GPU commit supplies its buffer.
            let child = self.subcompositor.get_subsurface(
                &self.surface,
                &self.parent_surface,
                &self.queue_handle,
                (),
            );
            // GPU frames present independently of the parent's CPU commits.
            child.set_desync();
            *subsurface = Some(child);
            self.placement.set(None);
        }
        if self.placement.get() != Some(placement) {
            let child = subsurface.as_ref().unwrap();
            // Applied with the parent's next commit, which follows this sync in the
            // same frame when the CPU UI presents its layout.
            child.set_position(placement.position[0], placement.position[1]);
            if let Some(viewport) = &self.viewport {
                // Applied with the next GPU present, together with the resized buffer.
                viewport.set_destination(placement.destination[0], placement.destination[1]);
            }
            self.placement.set(Some(placement));
        }
        drop(subsurface);
        self.commit()
    }

    pub fn hide(&self) {
        // Destroying the role unmaps the surface immediately, without touching the
        // buffers wgpu owns. Hidden viewports do not present.
        if let Some(child) = self.subsurface.borrow_mut().take() {
            child.destroy();
            self.placement.set(None);
            let _ = self.connection.flush();
        }
    }

    pub fn commit(&self) -> Result<()> {
        // Drain enter/leave events that winit's reads queued for the guest surface.
        self.queue
            .borrow_mut()
            .dispatch_pending(&mut Guest)
            .map_err(error)?;
        self.connection.flush().map_err(error)
    }
}

impl Drop for NativeHost {
    fn drop(&mut self) {
        if let Some(child) = self.subsurface.get_mut().take() {
            child.destroy();
        }
        if let Some(viewport) = self.viewport.take() {
            viewport.destroy();
        }
        self.surface.destroy();
        let _ = self.connection.flush();
    }
}

fn wayland_handles(parent: &Window) -> Result<(WaylandDisplayHandle, WaylandWindowHandle)> {
    match (
        parent.display_handle().map_err(error)?.as_raw(),
        parent.window_handle().map_err(error)?.as_raw(),
    ) {
        (RawDisplayHandle::Wayland(display), RawWindowHandle::Wayland(window)) => {
            Ok((display, window))
        }
        _ => Err(error("expected a Wayland parent window")),
    }
}

/// Maps the runtime's physical viewport rectangle to surface-local logical units.
/// Edges are rounded independently so adjacent views share boundaries.
fn logical_placement(origin: [i32; 2], size: [u32; 2], scale: f64) -> Placement {
    let edge = |physical: i64| (physical as f64 / scale).round() as i32;
    let position = [edge(origin[0].into()), edge(origin[1].into())];
    let far = [
        edge(i64::from(origin[0]) + i64::from(size[0])),
        edge(i64::from(origin[1]) + i64::from(size[1])),
    ];
    Placement {
        position,
        destination: [(far[0] - position[0]).max(1), (far[1] - position[1]).max(1)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_viewport_maps_to_logical_subsurface_geometry() {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let origin = [(20.0 * scale) as i32, (30.0 * scale) as i32];
            let size = [(320.0 * scale) as u32, (180.0 * scale) as u32];
            assert_eq!(
                logical_placement(origin, size, scale),
                Placement {
                    position: [20, 30],
                    destination: [320, 180],
                },
                "scale {scale}"
            );
        }
        // Tiny views keep a valid (positive) viewport destination.
        assert_eq!(logical_placement([0, 0], [1, 1], 3.0).destination, [1, 1]);
    }

    #[test]
    fn adjacent_views_share_logical_edges_at_fractional_scale() {
        let scale = 1.25;
        let left = logical_placement([0, 0], [333, 100], scale);
        let right = logical_placement([333, 0], [333, 100], scale);
        assert_eq!(
            left.position[0] + left.destination[0],
            right.position[0],
            "no gap or overlap between neighbours"
        );
    }

    #[test]
    #[ignore = "requires a live Wayland compositor with wl_subcompositor and wp_viewporter"]
    fn wayland_host_lifecycle() {
        use winit::{
            application::ApplicationHandler, event::WindowEvent, event_loop::EventLoop,
            platform::wayland::EventLoopBuilderExtWayland, window::WindowId,
        };

        // A protocol error kills the connection, so a clean roundtrip proves the
        // compositor accepted every request sent so far.
        fn roundtrip(host: &NativeHost) {
            host.queue.borrow_mut().roundtrip(&mut Guest).unwrap();
        }

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
                    assert!(host.viewport.is_some());
                    assert!(matches!(
                        host.target().unwrap(),
                        wgpu::SurfaceTargetUnsafe::RawHandle {
                            raw_window_handle: RawWindowHandle::Wayland(_),
                            ..
                        }
                    ));
                    host.place([17, 23], [320, 180]).unwrap();
                    assert!(host.subsurface.borrow().is_some());
                    roundtrip(&host);
                    host.hide();
                    assert!(host.subsurface.borrow().is_none());
                    roundtrip(&host);
                    // Re-assigning the subsurface role after hide() must be legal.
                    host.place([31, 41], [640, 360]).unwrap();
                    host.place([31, 41], [640, 360]).unwrap();
                    roundtrip(&host);
                    let probe = NativeHost::new(event_loop, parent.clone(), transparent).unwrap();
                    drop(host);
                    // A second host on the same parent observes a healthy connection
                    // after the first one tore down its objects.
                    probe.place([0, 0], [1, 1]).unwrap();
                    roundtrip(&probe);
                }
                event_loop.exit();
            }

            fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
        }
        let event_loop = EventLoop::builder()
            .with_wayland()
            .with_any_thread(true)
            .build()
            .unwrap();
        event_loop.run_app(&mut Check).unwrap();
    }
}
