use super::*;
use cssimpler_core::{LayoutBox, RenderNode};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use winit::{event_loop::ActiveEventLoop, window::Window};

struct LiveViewport {
    // Surface must be destroyed before its native host.
    surface: wgpu::Surface<'static>,
    host: native::NativeHost,
    config: wgpu::SurfaceConfiguration,
    info: GpuViewportInfo,
    renderer: Box<dyn GpuViewportRenderer>,
    visible: bool,
    frame_index: u64,
    last_frame: Option<Instant>,
    next_frame: Instant,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PointerCapture {
    index: usize,
    buttons: Vec<crate::PointerButton>,
}

impl PointerCapture {
    fn press(
        current: Option<Self>,
        hit: Option<usize>,
        button: crate::PointerButton,
    ) -> Option<Self> {
        if let Some(mut current) = current {
            if !current.buttons.contains(&button) {
                current.buttons.push(button);
            }
            Some(current)
        } else {
            hit.map(|index| Self {
                index,
                buttons: vec![button],
            })
        }
    }

    fn release(mut self, button: crate::PointerButton) -> Option<Self> {
        self.buttons.retain(|pressed| *pressed != button);
        (!self.buttons.is_empty()).then_some(self)
    }
}

pub(crate) struct GpuRuntime {
    live: Vec<Option<LiveViewport>>,
    registry: GpuViewportRegistry,
    gpu: Option<GpuContext>,
    instance: Option<wgpu::Instance>,
    focused: Option<usize>,
    captured: Option<PointerCapture>,
    pointer: Option<crate::PointerPosition>,
}

impl GpuRuntime {
    pub fn new(registry: GpuViewportRegistry) -> Result<Self> {
        registry.validate()?;
        let live = (0..registry.entries.len()).map(|_| None).collect();
        Ok(Self {
            live,
            registry,
            gpu: None,
            instance: None,
            focused: None,
            captured: None,
            pointer: None,
        })
    }

    pub fn install_wake<F>(&self, wake: F)
    where
        F: Fn() + Send + Sync + Clone + 'static,
    {
        for entry in &self.registry.entries {
            entry.redraw.set_wake(Box::new(wake.clone()));
        }
    }

    pub fn sync(
        &mut self,
        event_loop: &ActiveEventLoop,
        parent: Arc<Window>,
        roots: &[RenderNode],
        scale: f64,
        background: cssimpler_core::Color,
    ) -> Result<()> {
        if self.registry.is_empty() {
            return Ok(());
        }
        let size = parent.inner_size();
        let window = LayoutBox::new(
            0.0,
            0.0,
            (size.width as f64 / scale) as f32,
            (size.height as f64 / scale) as f32,
        );
        // Validate the complete layout before touching native hosts.
        let locations = self
            .registry
            .entries
            .iter()
            .map(|e| geometry::locate(roots, &e.id, window, background))
            .collect::<Result<Vec<_>>>()?;
        for (index, bounds) in locations.into_iter().enumerate() {
            let Some(location) = bounds else {
                if let Some(live) = &mut self.live[index] {
                    live.host.hide();
                    live.visible = false;
                }
                if self.focused == Some(index) {
                    self.focused = None;
                }
                if self
                    .captured
                    .as_ref()
                    .is_some_and(|capture| capture.index == index)
                {
                    if let Some(live) = &mut self.live[index] {
                        live.renderer.event(&EngineEvent::Suspended);
                    }
                    self.captured = None;
                }
                continue;
            };
            let bounds = location.bounds;
            let origin = [
                (f64::from(bounds.x) * scale).round() as i32,
                (f64::from(bounds.y) * scale).round() as i32,
            ];
            let right = (f64::from(bounds.x + bounds.width) * scale).round() as i32;
            let bottom = (f64::from(bounds.y + bounds.height) * scale).round() as i32;
            let physical_size = [
                (right - origin[0]).max(1) as u32,
                (bottom - origin[1]).max(1) as u32,
            ];
            let entry = &mut self.registry.entries[index];
            if self.live[index].is_none() {
                let host =
                    native::NativeHost::new(event_loop, parent.clone(), entry.options.transparent)?;
                // Native swapchains derive their extent from the child window/layer.
                host.place(origin, physical_size)?;
                let instance = self.instance.get_or_insert_with(wgpu::Instance::default);
                // SAFETY: LiveViewport retains the host and drops the surface first.
                let surface =
                    unsafe { instance.create_surface_unsafe(host.target()?) }.map_err(error)?;
                if self.gpu.is_none() {
                    let adapter = pollster::block_on(instance.request_adapter(
                        &wgpu::RequestAdapterOptions {
                            compatible_surface: Some(&surface),
                            ..Default::default()
                        },
                    ))
                    .map_err(error)?;
                    let (device, queue) =
                        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                            label: Some("CSSimpler GPU viewports"),
                            required_features: self.registry.device_options.required_features,
                            required_limits: self.registry.device_options.required_limits.clone(),
                            ..Default::default()
                        }))
                        .map_err(error)?;
                    self.gpu = Some(GpuContext {
                        adapter,
                        device,
                        queue,
                    });
                }
                let gpu = self.gpu.as_ref().unwrap();
                let caps = surface.get_capabilities(&gpu.adapter);
                let mut config = surface
                    .get_default_config(&gpu.adapter, physical_size[0], physical_size[1])
                    .ok_or_else(|| error("surface is incompatible with shared GPU adapter"))?;
                let native_premultiplied = entry.options.transparent;
                config.format = caps
                    .formats
                    .iter()
                    .copied()
                    // Native transparent layers compose premultiplied encoded RGB.
                    // Resolve in that space to avoid halos on partially covered edges.
                    .find(|f| f.is_srgb() != native_premultiplied)
                    .unwrap_or(config.format);
                config.alpha_mode = if entry.options.transparent {
                    [
                        wgpu::CompositeAlphaMode::PreMultiplied,
                        #[cfg(not(target_os = "linux"))]
                        wgpu::CompositeAlphaMode::PostMultiplied,
                        // X11 delegates premultiplied ARGB composition to XRender.
                        #[cfg(target_os = "linux")]
                        wgpu::CompositeAlphaMode::Inherit,
                        // NVIDIA X11 exposes only Opaque. Our redirected ARGB child
                        // retains alpha; X Composite performs the blending itself.
                        #[cfg(target_os = "linux")]
                        wgpu::CompositeAlphaMode::Opaque,
                    ]
                    .into_iter()
                    .find(|m| caps.alpha_modes.contains(m))
                    .ok_or_else(|| error("native surface does not support alpha composition"))?
                } else {
                    wgpu::CompositeAlphaMode::Opaque
                };
                if !caps.alpha_modes.contains(&config.alpha_mode) {
                    return Err(error(
                        "native surface does not support requested alpha mode",
                    ));
                }
                config.present_mode = wgpu::PresentMode::Fifo;
                #[cfg(target_os = "macos")]
                {
                    // Two Metal drawables keep interactive frames from queuing
                    // behind an additional stale camera pose. wgpu clamps support.
                    config.desired_maximum_frame_latency = 1;
                }
                if caps.usages.contains(wgpu::TextureUsages::COPY_DST) {
                    config.usage |= wgpu::TextureUsages::COPY_DST;
                }
                surface.configure(&gpu.device, &config);
                host.commit()?;
                let info = GpuViewportInfo {
                    bounds,
                    physical_size,
                    scale_factor: scale,
                    format: config.format,
                    alpha_mode: if native_premultiplied {
                        // Describe the renderer's pixel encoding, independent of the
                        // WSI mode. Metal's PostMultiplied mode only marks the layer
                        // non-opaque; Core Animation still consumes premultiplied RGB.
                        wgpu::CompositeAlphaMode::PreMultiplied
                    } else {
                        config.alpha_mode
                    },
                    background: if entry.options.transparent && location.glass {
                        None
                    } else {
                        Some(location.background)
                    },
                    texture_usage: config.usage,
                };
                let factory = entry
                    .factory
                    .take()
                    .ok_or_else(|| error("GPU renderer factory already consumed"))?;
                let renderer = factory(GpuViewportInit {
                    gpu,
                    info,
                    redraw: entry.redraw.clone(),
                })?;
                self.live[index] = Some(LiveViewport {
                    surface,
                    host,
                    config,
                    info,
                    renderer,
                    visible: false,
                    frame_index: 0,
                    last_frame: None,
                    next_frame: Instant::now(),
                });
            }
            let live = self.live[index].as_mut().unwrap();
            // Resize the native drawable before reconfiguring its swapchain.
            live.host.place(origin, physical_size)?;
            let info = GpuViewportInfo {
                bounds,
                physical_size,
                scale_factor: scale,
                background: if entry.options.transparent && location.glass {
                    None
                } else {
                    Some(location.background)
                },
                ..live.info
            };
            if info != live.info {
                if info.physical_size != live.info.physical_size {
                    live.config.width = physical_size[0];
                    live.config.height = physical_size[1];
                    live.surface
                        .configure(&self.gpu.as_ref().unwrap().device, &live.config);
                    live.host.commit()?;
                }
                live.info = info;
                live.renderer.resize(self.gpu.as_ref().unwrap(), info)?;
                entry.redraw.0.dirty.store(true, Ordering::Release);
            }
            if !live.visible {
                entry.redraw.0.dirty.store(true, Ordering::Release);
            }
            live.visible = true;
        }
        Ok(())
    }

    pub fn hide(&mut self) {
        for live in self.live.iter_mut().flatten() {
            live.renderer.event(&EngineEvent::Suspended);
            live.host.hide();
            live.visible = false;
        }
        self.focused = None;
        self.captured = None;
    }

    pub fn draw(&mut self, cadence: Duration) -> Result<()> {
        let Some(gpu) = &self.gpu else {
            return Ok(());
        };
        let now = Instant::now();
        for (entry, live) in self.registry.entries.iter().zip(&mut self.live) {
            let Some(live) = live else {
                continue;
            };
            if !live.visible
                || now < live.next_frame
                || (!entry.redraw.dirty() && entry.options.redraw != RedrawSchedule::EveryFrame)
            {
                continue;
            }
            let (texture, reconfigure) = match live.surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(t) => (t, false),
                wgpu::CurrentSurfaceTexture::Suboptimal(t) => (t, true),
                wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                    live.next_frame = now + cadence;
                    continue;
                }
                wgpu::CurrentSurfaceTexture::Outdated => {
                    live.surface.configure(&gpu.device, &live.config);
                    live.host.commit()?;
                    live.next_frame = now + cadence;
                    continue;
                }
                wgpu::CurrentSurfaceTexture::Lost => {
                    // SAFETY: same host lifetime contract as initial creation.
                    live.surface = unsafe {
                        self.instance
                            .as_ref()
                            .unwrap()
                            .create_surface_unsafe(live.host.target()?)
                    }
                    .map_err(error)?;
                    live.surface.configure(&gpu.device, &live.config);
                    live.host.commit()?;
                    live.next_frame = now + cadence;
                    continue;
                }
                wgpu::CurrentSurfaceTexture::Validation => {
                    return Err(error("GPU surface acquisition validation failed"));
                }
            };
            entry.redraw.take_dirty();
            let target = texture.texture.create_view(&Default::default());
            let mut encoder = gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("CSSimpler viewport frame"),
                });
            live.renderer.render(
                gpu,
                GpuViewportFrame {
                    info: live.info,
                    frame: FrameInfo {
                        frame_index: live.frame_index,
                        delta: live
                            .last_frame
                            .map(|at| now.saturating_duration_since(at))
                            .unwrap_or_default(),
                    },
                    target: &target,
                    texture: &texture.texture,
                    encoder: &mut encoder,
                },
            )?;
            gpu.queue.submit([encoder.finish()]);
            gpu.queue.present(texture);
            if reconfigure {
                live.surface.configure(&gpu.device, &live.config);
                live.host.commit()?;
            }
            live.frame_index += 1;
            live.last_frame = Some(now);
            live.next_frame = now + cadence;
        }
        Ok(())
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.registry
            .entries
            .iter()
            .zip(&self.live)
            .filter_map(|(entry, live)| {
                let live = live.as_ref()?;
                if !live.visible {
                    None
                } else if entry.redraw.dirty() {
                    Some(live.next_frame)
                } else if entry.options.redraw == RedrawSchedule::EveryFrame {
                    Some(live.next_frame)
                } else {
                    None
                }
            })
            .min()
    }

    pub fn event(&mut self, event: &EngineEvent) -> bool {
        // Button events and motion share the event queue. Do not compare a
        // queued move with the current OS button state: the physical release
        // can already have happened while these valid drag events were queued.
        if let EngineEvent::PointerMoved { position, .. } = event {
            self.pointer = Some(*position);
        }
        if matches!(event, EngineEvent::PointerLeft) {
            self.pointer = None;
        }
        let hit = self.pointer.and_then(|p| {
            self.live.iter().position(|live| {
                live.as_ref().is_some_and(|l| {
                    l.visible
                        && p.x >= l.info.bounds.x
                        && p.y >= l.info.bounds.y
                        && p.x < l.info.bounds.x + l.info.bounds.width
                        && p.y < l.info.bounds.y + l.info.bounds.height
                })
            })
        });
        if let EngineEvent::PointerButton {
            button,
            state: crate::ButtonState::Pressed,
            ..
        } = event
        {
            self.focused = hit;
            // An additional button must not replace the original drag owner.
            self.captured = PointerCapture::press(self.captured.take(), hit, *button);
        }
        let target = match event {
            EngineEvent::Key(_) | EngineEvent::TextInput(_) => self.focused,
            EngineEvent::FocusChanged(false) | EngineEvent::Suspended => {
                for live in self.live.iter_mut().flatten() {
                    live.renderer.event(event);
                }
                self.focused = None;
                self.captured = None;
                return false;
            }
            _ => self.captured.as_ref().map(|capture| capture.index).or(hit),
        };
        let consumed = target
            .and_then(|i| {
                self.live[i].as_mut().map(|live| {
                    let mut local = event.clone();
                    if let EngineEvent::PointerMoved { position, .. } = &mut local {
                        position.x -= live.info.bounds.x;
                        position.y -= live.info.bounds.y;
                    }
                    let consumed = live.renderer.event(&local);
                    if consumed {
                        self.registry.entries[i].redraw.request_redraw();
                    }
                    consumed
                })
            })
            .unwrap_or(false);
        if let EngineEvent::PointerButton {
            button,
            state: crate::ButtonState::Released,
            ..
        } = event
        {
            self.captured = self
                .captured
                .take()
                .and_then(|capture| capture.release(*button));
        }
        consumed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PointerButton::{Primary, Secondary};

    #[test]
    fn queued_motion_keeps_capture_until_the_queued_release() {
        let mut runtime = GpuRuntime::new(GpuViewportRegistry::default()).unwrap();
        // Input routing can be exercised without creating a native surface.
        runtime.live.push(None);
        runtime.captured = PointerCapture::press(None, Some(0), Primary);
        for x in 0..256 {
            runtime.event(&EngineEvent::PointerMoved {
                position: crate::PointerPosition {
                    x: x as f32,
                    y: 20.0,
                },
                modifiers: Default::default(),
            });
            let capture = runtime
                .captured
                .as_ref()
                .expect("queued motion must not synthesize a release");
            assert_eq!(capture.index, 0);
            assert_eq!(capture.buttons, vec![Primary]);
        }
        runtime.event(&EngineEvent::PointerButton {
            button: Primary,
            state: crate::ButtonState::Released,
            modifiers: Default::default(),
        });
        assert!(runtime.captured.is_none());
        runtime.event(&EngineEvent::PointerMoved {
            position: crate::PointerPosition { x: 300.0, y: 20.0 },
            modifiers: Default::default(),
        });
        assert!(runtime.captured.is_none(), "hover cannot start a drag");
    }

    #[test]
    fn additional_button_does_not_steal_or_release_the_original_drag() {
        let first = PointerCapture::press(None, Some(1), Primary);
        let second = PointerCapture::press(first.clone(), Some(2), Secondary);
        assert_eq!(second.as_ref().unwrap().index, 1);
        let after_secondary_release = second.and_then(|capture| capture.release(Secondary));
        assert_eq!(after_secondary_release, first);
        assert_eq!(
            after_secondary_release.and_then(|capture| capture.release(Primary)),
            None
        );
    }

    #[test]
    fn release_outside_the_viewport_still_goes_to_the_drag_owner() {
        let capture = PointerCapture::press(None, Some(3), Primary);
        let combined = PointerCapture::press(capture, None, Secondary);
        assert_eq!(combined.as_ref().unwrap().index, 3);
        let capture = combined.and_then(|capture| capture.release(Secondary));
        assert_eq!(capture.and_then(|capture| capture.release(Primary)), None);
        assert_eq!(PointerCapture::press(None, None, Primary), None);
    }

    #[test]
    fn capture_survives_until_all_buttons_are_released_in_either_order() {
        for (first, second) in [(Primary, Secondary), (Secondary, Primary)] {
            let capture = PointerCapture::press(None, Some(2), first);
            let both = PointerCapture::press(capture, None, second);
            let one = both.and_then(|capture| capture.release(first)).unwrap();
            assert_eq!(one.index, 2);
            assert_eq!(one.buttons, vec![second]);
            assert_eq!(one.release(second), None);
        }
    }
}
