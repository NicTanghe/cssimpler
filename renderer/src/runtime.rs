use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::{Duration, Instant};

use cssimpler_core::ExtractedScene;
use softbuffer::{Context, Surface};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{
    ElementState, Ime, MouseButton as WinitMouseButton, MouseScrollDelta, WindowEvent,
};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, OwnedDisplayHandle};
use winit::keyboard::{
    Key as WinitKey, KeyLocation as WinitKeyLocation, ModifiersState, PhysicalKey,
};
use winit::window::{Window, WindowId};

use crate::input::{
    ButtonState, EngineEvent, KeyIdentity, KeyLocation, KeyboardEvent, KeyboardModifiers,
    PointerButton, PointerPosition, ScrollDelta, TextInputEvent, ViewportEvent,
};

use super::{
    ElementInteractionState, ElementPath, FrameInfo, FramePaintMode, FramePaintReason,
    FrameTimingStats, MouseEventKind, PaintStats, RedrawSchedule, RendererBackendKind,
    RendererError, Result, SceneProvider, WindowConfig, dispatch_hover_transition_events,
    dispatch_mouse_event, drawable_viewport_size, duration_to_us,
    gpu::{GpuPresentOutcome, GpuRuntimeBackend},
    record_frame_timing_stats, redraw_auto_scroll_indicator_regions, render_scene_update_internal,
    render_to_buffer_internal, resize_buffer, scrollbar, settle_element_interaction,
    should_present_frame, should_suspend_updates,
};

pub(super) fn run_with_scene_provider<P>(config: WindowConfig, scene_provider: P) -> Result<()>
where
    P: SceneProvider,
{
    let event_loop = EventLoop::new().map_err(RendererError::from)?;
    let context = Context::new(event_loop.owned_display_handle()).map_err(RendererError::from)?;
    let mut app = RuntimeApp::new(config, scene_provider, context);
    event_loop.run_app(&mut app).map_err(RendererError::from)?;
    app.finish()
}

struct CpuRuntimeBackend {
    surface: Option<Surface<OwnedDisplayHandle, Arc<Window>>>,
    buffer: Vec<u32>,
    buffer_width: usize,
    buffer_height: usize,
}

impl CpuRuntimeBackend {
    fn new(context: &Context<OwnedDisplayHandle>, window: &Arc<Window>) -> Result<Self> {
        let mut backend = Self {
            surface: None,
            buffer: Vec::new(),
            buffer_width: 0,
            buffer_height: 0,
        };
        backend.recreate_surface(context, window)?;
        Ok(backend)
    }

    fn recreate_surface(
        &mut self,
        context: &Context<OwnedDisplayHandle>,
        window: &Arc<Window>,
    ) -> Result<()> {
        self.surface =
            Some(Surface::new(context, Arc::clone(window)).map_err(RendererError::from)?);
        Ok(())
    }

    fn resize_surface(
        &mut self,
        width: usize,
        height: usize,
        clear_color: cssimpler_core::Color,
    ) -> Result<bool> {
        let resized = self.buffer_width != width || self.buffer_height != height;
        resize_buffer(
            &mut self.buffer,
            &mut self.buffer_width,
            &mut self.buffer_height,
            width,
            height,
            clear_color,
        );

        let Some(surface) = self.surface.as_mut() else {
            return Ok(resized);
        };
        let (Some(width), Some(height)) = (
            NonZeroU32::new(self.buffer_width as u32),
            NonZeroU32::new(self.buffer_height as u32),
        ) else {
            return Ok(resized);
        };
        surface.resize(width, height).map_err(RendererError::from)?;
        Ok(resized)
    }

    fn has_surface(&self) -> bool {
        self.surface.is_some()
    }

    fn suspend(&mut self) {
        self.surface = None;
    }

    fn present(
        &mut self,
        scene: &[cssimpler_core::RenderNode],
        extracted_scene: &ExtractedScene,
        previous_scene: Option<&ExtractedScene>,
        previous_indicator: Option<scrollbar::AutoScrollIndicator>,
        indicator: Option<scrollbar::AutoScrollIndicator>,
        clear_color: cssimpler_core::Color,
        resized: bool,
    ) -> Result<Option<PresentedFrame>> {
        let paint_start = Instant::now();
        let paint_stats = if resized {
            render_to_buffer_internal(
                extracted_scene,
                &mut self.buffer,
                self.buffer_width,
                self.buffer_height,
                clear_color,
            )
        } else if let Some(previous_scene) = previous_scene {
            render_scene_update_internal(
                previous_scene,
                extracted_scene,
                &mut self.buffer,
                self.buffer_width,
                self.buffer_height,
                clear_color,
            )
        } else {
            render_to_buffer_internal(
                extracted_scene,
                &mut self.buffer,
                self.buffer_width,
                self.buffer_height,
                clear_color,
            )
        };
        let paint_us = duration_to_us(paint_start.elapsed());

        redraw_auto_scroll_indicator_regions(
            previous_indicator,
            indicator,
            scene,
            &mut self.buffer,
            self.buffer_width,
            self.buffer_height,
            clear_color,
        );

        let Some(surface) = self.surface.as_mut() else {
            return Ok(Some(PresentedFrame {
                paint_stats,
                paint_us,
                present_us: 0,
            }));
        };
        let present_start = Instant::now();
        let mut target = surface.buffer_mut().map_err(RendererError::from)?;
        target.copy_from_slice(&self.buffer);
        target.present().map_err(RendererError::from)?;
        Ok(Some(PresentedFrame {
            paint_stats,
            paint_us,
            present_us: duration_to_us(present_start.elapsed()),
        }))
    }
}

struct RuntimeBackendState {
    preferred: RendererBackendKind,
    cpu: Option<CpuRuntimeBackend>,
    gpu: Option<GpuRuntimeBackend>,
    last_fallback_reason: Option<String>,
}

struct PresentedFrame {
    paint_stats: PaintStats,
    paint_us: u64,
    present_us: u64,
}

impl RuntimeBackendState {
    fn new(preferred: RendererBackendKind) -> Self {
        Self {
            preferred,
            cpu: None,
            gpu: None,
            last_fallback_reason: None,
        }
    }

    fn attach_window(
        &mut self,
        context: &Context<OwnedDisplayHandle>,
        window: &Arc<Window>,
    ) -> Result<()> {
        match self.preferred {
            RendererBackendKind::Cpu => {
                let _ = self.ensure_cpu(context, window)?;
            }
            RendererBackendKind::Gpu => {
                if self.ensure_gpu(window).is_err() {
                    self.report_fallback("GPU backend initialization failed; using CPU fallback");
                    let _ = self.ensure_cpu(context, window)?;
                }
            }
        }
        Ok(())
    }

    fn prepare_viewport(
        &mut self,
        context: &Context<OwnedDisplayHandle>,
        window: &Arc<Window>,
        viewport: super::ViewportSize,
        clear_color: cssimpler_core::Color,
    ) -> Result<bool> {
        match self.preferred {
            RendererBackendKind::Cpu => self.ensure_cpu(context, window)?.resize_surface(
                viewport.width,
                viewport.height,
                clear_color,
            ),
            RendererBackendKind::Gpu => {
                if let Ok(gpu) = self.ensure_gpu(window) {
                    gpu.resize_surface(viewport.width as u32, viewport.height as u32)
                } else {
                    self.report_fallback("GPU backend is unavailable; using CPU fallback");
                    self.ensure_cpu(context, window)?.resize_surface(
                        viewport.width,
                        viewport.height,
                        clear_color,
                    )
                }
            }
        }
    }

    fn has_surface(&self) -> bool {
        self.gpu
            .as_ref()
            .is_some_and(GpuRuntimeBackend::has_surface)
            || self
                .cpu
                .as_ref()
                .is_some_and(CpuRuntimeBackend::has_surface)
    }

    fn suspend(&mut self) {
        if let Some(cpu) = self.cpu.as_mut() {
            cpu.suspend();
        }
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.suspend();
        }
    }

    fn present(
        &mut self,
        context: &Context<OwnedDisplayHandle>,
        window: &Arc<Window>,
        scene: &[cssimpler_core::RenderNode],
        extracted_scene: &ExtractedScene,
        previous_scene: Option<&ExtractedScene>,
        previous_indicator: Option<scrollbar::AutoScrollIndicator>,
        indicator: Option<scrollbar::AutoScrollIndicator>,
        clear_color: cssimpler_core::Color,
        resized: bool,
    ) -> Result<Option<PresentedFrame>> {
        match self.preferred {
            RendererBackendKind::Cpu => self.present_cpu(
                context,
                window,
                scene,
                extracted_scene,
                previous_scene,
                previous_indicator,
                indicator,
                clear_color,
                resized,
            ),
            RendererBackendKind::Gpu => {
                if let Ok(gpu) = self.ensure_gpu(window) {
                    match gpu.present(extracted_scene, clear_color, indicator.is_some())? {
                        GpuPresentOutcome::Presented {
                            paint_us,
                            present_us,
                        } => {
                            self.clear_fallback();
                            Ok(Some(PresentedFrame {
                                paint_stats: gpu_paint_stats(extracted_scene),
                                paint_us,
                                present_us,
                            }))
                        }
                        GpuPresentOutcome::Skipped => Ok(None),
                        GpuPresentOutcome::Fallback(reason) => {
                            self.report_fallback(&reason);
                            self.present_cpu(
                                context,
                                window,
                                scene,
                                extracted_scene,
                                previous_scene,
                                previous_indicator,
                                indicator,
                                clear_color,
                                resized,
                            )
                        }
                    }
                } else {
                    self.report_fallback("GPU backend is unavailable; using CPU fallback");
                    self.present_cpu(
                        context,
                        window,
                        scene,
                        extracted_scene,
                        previous_scene,
                        previous_indicator,
                        indicator,
                        clear_color,
                        resized,
                    )
                }
            }
        }
    }

    fn ensure_cpu(
        &mut self,
        context: &Context<OwnedDisplayHandle>,
        window: &Arc<Window>,
    ) -> Result<&mut CpuRuntimeBackend> {
        if self.cpu.is_none() {
            self.cpu = Some(CpuRuntimeBackend::new(context, window)?);
        } else if !self
            .cpu
            .as_ref()
            .is_some_and(CpuRuntimeBackend::has_surface)
        {
            self.cpu
                .as_mut()
                .expect("cpu backend should exist")
                .recreate_surface(context, window)?;
        }
        Ok(self.cpu.as_mut().expect("cpu backend should exist"))
    }

    fn ensure_gpu(&mut self, window: &Arc<Window>) -> Result<&mut GpuRuntimeBackend> {
        if self.gpu.is_none() {
            self.gpu = Some(GpuRuntimeBackend::new(window)?);
        } else if !self
            .gpu
            .as_ref()
            .is_some_and(GpuRuntimeBackend::has_surface)
        {
            self.gpu
                .as_mut()
                .expect("gpu backend should exist")
                .recreate_surface(window)?;
        }
        Ok(self.gpu.as_mut().expect("gpu backend should exist"))
    }

    #[allow(clippy::too_many_arguments)]
    fn present_cpu(
        &mut self,
        context: &Context<OwnedDisplayHandle>,
        window: &Arc<Window>,
        scene: &[cssimpler_core::RenderNode],
        extracted_scene: &ExtractedScene,
        previous_scene: Option<&ExtractedScene>,
        previous_indicator: Option<scrollbar::AutoScrollIndicator>,
        indicator: Option<scrollbar::AutoScrollIndicator>,
        clear_color: cssimpler_core::Color,
        resized: bool,
    ) -> Result<Option<PresentedFrame>> {
        let size = window.inner_size();
        let Some(viewport) = drawable_viewport_size(size.width as usize, size.height as usize)
        else {
            return Ok(None);
        };
        let cpu = self.ensure_cpu(context, window)?;
        let _ = cpu.resize_surface(viewport.width, viewport.height, clear_color)?;
        cpu.present(
            scene,
            extracted_scene,
            previous_scene,
            previous_indicator,
            indicator,
            clear_color,
            resized,
        )
    }

    fn report_fallback(&mut self, reason: &str) {
        if self.last_fallback_reason.as_deref() == Some(reason) {
            return;
        }
        eprintln!("cssimpler renderer: {reason}");
        self.last_fallback_reason = Some(reason.to_string());
    }

    fn clear_fallback(&mut self) {
        self.last_fallback_reason = None;
    }
}

fn gpu_paint_stats(scene: &ExtractedScene) -> PaintStats {
    let painted_pixels = scene
        .items
        .iter()
        .map(|item| {
            (item.layout.width.max(0.0).ceil() as usize)
                .saturating_mul(item.layout.height.max(0.0).ceil() as usize)
        })
        .sum::<usize>();

    PaintStats {
        workers: 1,
        dirty_regions: 0,
        dirty_jobs: 0,
        damage_pixels: painted_pixels,
        painted_pixels,
        scene_passes: 1,
        mode: FramePaintMode::Full,
        reason: FramePaintReason::FullRedraw,
    }
}

struct RuntimeApp<P> {
    config: WindowConfig,
    scene_provider: P,
    context: Context<OwnedDisplayHandle>,
    backend: RuntimeBackendState,
    window: Option<Arc<Window>>,
    window_id: Option<WindowId>,
    fatal_error: Option<RendererError>,
    frame_index: u64,
    last_frame_at: Option<Instant>,
    next_redraw_at: Option<Instant>,
    redraw_pending: bool,
    immediate_redraw: bool,
    suspended: bool,
    occluded: bool,
    scale_factor: f64,
    modifiers: KeyboardModifiers,
    mouse_position: Option<(f32, f32)>,
    pending_wheel: Option<(f32, f32)>,
    left_down: bool,
    right_down: bool,
    middle_down: bool,
    previous_left_down: bool,
    previous_right_down: bool,
    previous_middle_down: bool,
    previous_mouse_position: Option<(f32, f32)>,
    suppress_left_pointer_until_release: bool,
    left_press_target: Option<ElementPath>,
    last_click: Option<(Instant, ElementPath)>,
    element_interaction: ElementInteractionState,
    previous_presented_scene: Option<ExtractedScene>,
    previous_presented_indicator: Option<scrollbar::AutoScrollIndicator>,
    scrollbar_controller: scrollbar::ScrollbarController,
}

impl<P> RuntimeApp<P>
where
    P: SceneProvider,
{
    fn new(config: WindowConfig, scene_provider: P, context: Context<OwnedDisplayHandle>) -> Self {
        let backend_kind = config.backend;
        Self {
            config,
            scene_provider,
            context,
            backend: RuntimeBackendState::new(backend_kind),
            window: None,
            window_id: None,
            fatal_error: None,
            frame_index: 0,
            last_frame_at: None,
            next_redraw_at: None,
            redraw_pending: false,
            immediate_redraw: true,
            suspended: false,
            occluded: false,
            scale_factor: 1.0,
            modifiers: KeyboardModifiers::default(),
            mouse_position: None,
            pending_wheel: None,
            left_down: false,
            right_down: false,
            middle_down: false,
            previous_left_down: false,
            previous_right_down: false,
            previous_middle_down: false,
            previous_mouse_position: None,
            suppress_left_pointer_until_release: false,
            left_press_target: None,
            last_click: None,
            element_interaction: ElementInteractionState::default(),
            previous_presented_scene: None,
            previous_presented_indicator: None,
            scrollbar_controller: scrollbar::ScrollbarController::default(),
        }
    }

    fn finish(mut self) -> Result<()> {
        self.backend.suspend();
        self.window = None;
        self.fatal_error.map_or(Ok(()), Err)
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: impl Into<RendererError>) {
        if self.fatal_error.is_none() {
            self.fatal_error = Some(error.into());
        }
        event_loop.exit();
    }

    fn can_draw(&self) -> bool {
        if self.suspended || self.occluded {
            return false;
        }
        let Some(window) = self.window.as_ref() else {
            return false;
        };
        let size = window.inner_size();
        size.width > 0 && size.height > 0 && self.backend.has_surface()
    }

    fn wants_continuous_redraw(&self) -> bool {
        matches!(
            self.scene_provider.redraw_schedule(),
            RedrawSchedule::EveryFrame
        ) || self.scene_provider.needs_redraw()
    }

    fn request_immediate_redraw(&mut self) {
        self.immediate_redraw = true;
    }

    fn request_redraw_if_possible(&mut self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        if self.redraw_pending || !self.can_draw() {
            return;
        }
        window.request_redraw();
        self.redraw_pending = true;
    }

    fn recreate_surface(&mut self, event_loop: &ActiveEventLoop) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        if let Err(error) = self.backend.attach_window(&self.context, window) {
            self.fail(event_loop, error);
            return;
        }
        self.resize_surface(event_loop);
    }

    fn resize_surface(&mut self, event_loop: &ActiveEventLoop) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let size = window.inner_size();
        let Some(viewport) = drawable_viewport_size(size.width as usize, size.height as usize)
        else {
            return;
        };
        if let Err(error) =
            self.backend
                .prepare_viewport(&self.context, window, viewport, self.config.clear_color)
        {
            self.fail(event_loop, error);
        }
    }

    fn create_window(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            self.recreate_surface(event_loop);
            self.request_immediate_redraw();
            self.request_redraw_if_possible();
            return;
        }

        let attributes = Window::default_attributes()
            .with_title(self.config.title.clone())
            .with_inner_size(LogicalSize::new(
                self.config.width as f64,
                self.config.height as f64,
            ))
            .with_resizable(true);
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                self.fail(event_loop, error);
                return;
            }
        };
        window.set_ime_allowed(true);
        self.scale_factor = window.scale_factor();
        self.window_id = Some(window.id());
        self.window = Some(window);
        self.recreate_surface(event_loop);
        self.handle_engine_event(event_loop, EngineEvent::Resumed);
        self.handle_viewport_change(event_loop);
        self.request_immediate_redraw();
        self.request_redraw_if_possible();
    }

    fn handle_engine_event(&mut self, _event_loop: &ActiveEventLoop, event: EngineEvent) {
        if self.scene_provider.handle_engine_event(&event) {
            self.request_immediate_redraw();
        }
    }

    fn handle_viewport_change(&mut self, event_loop: &ActiveEventLoop) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let size = window.inner_size();
        self.scale_factor = window.scale_factor();
        if let Some(viewport) = drawable_viewport_size(size.width as usize, size.height as usize) {
            self.scene_provider.set_viewport(viewport);
            self.resize_surface(event_loop);
        }
        self.handle_engine_event(
            event_loop,
            EngineEvent::ViewportChanged(ViewportEvent {
                width: size.width as usize,
                height: size.height as usize,
                scale_factor: self.scale_factor,
            }),
        );
        self.request_immediate_redraw();
    }

    fn clear_pointer_state(&mut self) {
        self.mouse_position = None;
        self.pending_wheel = None;
        self.left_down = false;
        self.right_down = false;
        self.middle_down = false;
        self.previous_left_down = false;
        self.previous_right_down = false;
        self.previous_middle_down = false;
        self.previous_mouse_position = None;
        self.suppress_left_pointer_until_release = false;
        self.left_press_target = None;
    }

    fn prepare_suspend(&mut self, event_loop: &ActiveEventLoop) {
        self.suspended = true;
        let _ = self.scrollbar_controller.cancel_middle_button_auto_scroll();
        self.backend.suspend();
        self.clear_pointer_state();
        self.handle_engine_event(event_loop, EngineEvent::Suspended);
    }

    fn prepare_focus_change(&mut self, event_loop: &ActiveEventLoop, focused: bool) {
        self.handle_engine_event(event_loop, EngineEvent::FocusChanged(focused));
        if focused {
            return;
        }
        let _ = self.scrollbar_controller.cancel_middle_button_auto_scroll();
        self.clear_pointer_state();
        self.request_immediate_redraw();
    }

    fn accumulate_wheel(&mut self, delta: ScrollDelta) {
        let normalized = match delta {
            ScrollDelta::Lines { x, y } => (x, y),
            ScrollDelta::Pixels { x, y } => (
                x / scrollbar::WHEEL_SCROLL_STEP,
                y / scrollbar::WHEEL_SCROLL_STEP,
            ),
        };
        if normalized.0.abs() <= f32::EPSILON && normalized.1.abs() <= f32::EPSILON {
            return;
        }
        match &mut self.pending_wheel {
            Some((pending_x, pending_y)) => {
                *pending_x += normalized.0;
                *pending_y += normalized.1;
            }
            None => {
                self.pending_wheel = Some(normalized);
            }
        }
    }

    fn maybe_emit_text_commit(&mut self, event_loop: &ActiveEventLoop, text: &str) {
        if text.is_empty() || text.chars().any(char::is_control) {
            return;
        }
        self.handle_engine_event(
            event_loop,
            EngineEvent::TextInput(TextInputEvent::Commit(text.to_string())),
        );
    }

    fn draw_frame(&mut self, event_loop: &ActiveEventLoop) {
        self.redraw_pending = false;
        if !self.can_draw() {
            return;
        }
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let size = window.inner_size();
        let Some(viewport) = drawable_viewport_size(size.width as usize, size.height as usize)
        else {
            return;
        };
        let resized = match self.backend.prepare_viewport(
            &self.context,
            window,
            viewport,
            self.config.clear_color,
        ) {
            Ok(resized) => resized,
            Err(error) => {
                self.fail(event_loop, error);
                return;
            }
        };

        let frame_begin = Instant::now();
        let now = Instant::now();
        let delta = self
            .last_frame_at
            .map(|previous| now.saturating_duration_since(previous))
            .unwrap_or(Duration::ZERO);
        self.last_frame_at = Some(now);
        let frame = FrameInfo {
            frame_index: self.frame_index,
            delta,
        };
        let mut frame_stats = FrameTimingStats::default();

        let suppress_pointer_for_system_drag =
            should_suspend_updates(self.left_down, self.modifiers.super_key, false);
        if suppress_pointer_for_system_drag {
            self.suppress_left_pointer_until_release = true;
        } else if !self.left_down {
            self.suppress_left_pointer_until_release = false;
        }
        let interactive_left_down = self.left_down
            && !suppress_pointer_for_system_drag
            && !self.suppress_left_pointer_until_release;

        self.scene_provider.set_viewport(viewport);
        let update_start = Instant::now();
        self.scene_provider.update(frame);
        frame_stats.update_us = duration_to_us(update_start.elapsed());

        let scene_prep_start = Instant::now();
        let mut scene = self.scene_provider.capture_scene();
        self.scrollbar_controller.apply_to_scene(&mut scene);
        let mouse_position = self.mouse_position;
        let previous_hovered = self.element_interaction.hovered.clone();
        let click_started = interactive_left_down && !self.previous_left_down;
        let right_press_started = self.right_down && !self.previous_right_down;
        let middle_click_started = self.middle_down && !self.previous_middle_down;
        let auto_scroll_canceled_click =
            click_started && self.scrollbar_controller.cancel_middle_button_auto_scroll();

        if self.config.middle_button_auto_scroll {
            if middle_click_started {
                let _ = self
                    .scrollbar_controller
                    .toggle_middle_button_auto_scroll(&scene, mouse_position);
            }
        } else {
            let _ = self.scrollbar_controller.cancel_middle_button_auto_scroll();
        }

        let _ = self.scrollbar_controller.step_middle_button_auto_scroll(
            &mut scene,
            mouse_position,
            delta,
        );
        let _ = self.scrollbar_controller.handle_wheel(
            &mut scene,
            mouse_position,
            self.pending_wheel.take(),
        );
        let scrollbar_consumed_click = self.scrollbar_controller.handle_pointer(
            &mut scene,
            mouse_position,
            interactive_left_down,
            click_started,
        );
        let normal_click_started =
            click_started && !auto_scroll_canceled_click && !scrollbar_consumed_click;

        settle_element_interaction(
            &mut self.scene_provider,
            frame,
            &mut scene,
            &mut self.scrollbar_controller,
            mouse_position,
            interactive_left_down,
            normal_click_started,
            &mut self.element_interaction,
        );

        let current_hovered = self.element_interaction.hovered.clone();
        let mouse_moved = mouse_position != self.previous_mouse_position;
        let mut event_triggered_rerender = dispatch_hover_transition_events(
            &scene,
            previous_hovered.as_ref(),
            current_hovered.as_ref(),
        );

        if mouse_moved && let Some((mouse_x, mouse_y)) = mouse_position {
            event_triggered_rerender |=
                dispatch_mouse_event(&scene, mouse_x, mouse_y, MouseEventKind::MouseMove);
        }

        if normal_click_started {
            self.left_press_target = current_hovered.clone();
            if let Some((mouse_x, mouse_y)) = mouse_position {
                event_triggered_rerender |=
                    dispatch_mouse_event(&scene, mouse_x, mouse_y, MouseEventKind::MouseDown);
            }
        } else if click_started {
            self.left_press_target = None;
        }

        if self.previous_left_down && !interactive_left_down {
            if let Some((mouse_x, mouse_y)) = mouse_position {
                event_triggered_rerender |=
                    dispatch_mouse_event(&scene, mouse_x, mouse_y, MouseEventKind::MouseUp);
            }

            let release_target = current_hovered.clone();
            if self.left_press_target == release_target
                && let Some((mouse_x, mouse_y)) = mouse_position
            {
                event_triggered_rerender |=
                    dispatch_mouse_event(&scene, mouse_x, mouse_y, MouseEventKind::Click);
                if let Some(target) = release_target {
                    let click_now = Instant::now();
                    let is_double_click =
                        self.last_click
                            .as_ref()
                            .is_some_and(|(last_at, last_target)| {
                                *last_target == target
                                    && click_now.saturating_duration_since(*last_at)
                                        <= super::DOUBLE_CLICK_THRESHOLD
                            });
                    self.last_click = Some((click_now, target.clone()));
                    if is_double_click && let Some((mouse_x, mouse_y)) = mouse_position {
                        event_triggered_rerender |= dispatch_mouse_event(
                            &scene,
                            mouse_x,
                            mouse_y,
                            MouseEventKind::DblClick,
                        );
                    }
                }
            }

            self.left_press_target = None;
        }

        if right_press_started && let Some((mouse_x, mouse_y)) = mouse_position {
            event_triggered_rerender |=
                dispatch_mouse_event(&scene, mouse_x, mouse_y, MouseEventKind::MouseDown);
            event_triggered_rerender |=
                dispatch_mouse_event(&scene, mouse_x, mouse_y, MouseEventKind::ContextMenu);
        }

        if self.previous_right_down
            && !self.right_down
            && let Some((mouse_x, mouse_y)) = mouse_position
        {
            event_triggered_rerender |=
                dispatch_mouse_event(&scene, mouse_x, mouse_y, MouseEventKind::MouseUp);
        }

        if event_triggered_rerender {
            let rerender_start = Instant::now();
            self.scene_provider.update(frame);
            frame_stats.update_us += duration_to_us(rerender_start.elapsed());
            scene = self.scene_provider.capture_scene();
            self.scrollbar_controller.apply_to_scene(&mut scene);
            self.scrollbar_controller.handle_pointer(
                &mut scene,
                mouse_position,
                interactive_left_down,
                false,
            );
            settle_element_interaction(
                &mut self.scene_provider,
                frame,
                &mut scene,
                &mut self.scrollbar_controller,
                mouse_position,
                interactive_left_down,
                false,
                &mut self.element_interaction,
            );
        }

        frame_stats.scene_prep_us = duration_to_us(scene_prep_start.elapsed());
        let auto_scroll_indicator = self.scrollbar_controller.auto_scroll_indicator();
        let extracted_scene = ExtractedScene::from_render_roots(&scene);

        if should_present_frame(
            self.previous_presented_scene.as_ref(),
            &extracted_scene,
            self.previous_presented_indicator,
            auto_scroll_indicator,
            resized,
        ) {
            let presented_frame = match self.backend.present(
                &self.context,
                window,
                &scene,
                &extracted_scene,
                self.previous_presented_scene.as_ref(),
                self.previous_presented_indicator,
                auto_scroll_indicator,
                self.config.clear_color,
                resized,
            ) {
                Ok(paint_stats) => paint_stats,
                Err(error) => {
                    self.fail(event_loop, error);
                    return;
                }
            };
            if let Some(presented_frame) = presented_frame {
                frame_stats.paint_us = presented_frame.paint_us;
                frame_stats.render_workers = presented_frame.paint_stats.workers;
                frame_stats.dirty_regions = presented_frame.paint_stats.dirty_regions;
                frame_stats.dirty_jobs = presented_frame.paint_stats.dirty_jobs;
                frame_stats.damage_pixels = presented_frame.paint_stats.damage_pixels;
                frame_stats.painted_pixels = presented_frame.paint_stats.painted_pixels;
                frame_stats.scene_passes = presented_frame.paint_stats.scene_passes;
                frame_stats.paint_mode = presented_frame.paint_stats.mode;
                frame_stats.paint_reason = presented_frame.paint_stats.reason;
                frame_stats.present_us = presented_frame.present_us;
                self.previous_presented_scene = Some(extracted_scene);
                self.previous_presented_indicator = auto_scroll_indicator;
            }
        }

        self.previous_left_down = interactive_left_down;
        self.previous_right_down = self.right_down;
        self.previous_middle_down = self.middle_down;
        self.previous_mouse_position = mouse_position;
        frame_stats.total_us = duration_to_us(frame_begin.elapsed());
        record_frame_timing_stats(frame_stats);
        self.frame_index = self.frame_index.saturating_add(1);
        self.immediate_redraw = false;
        self.next_redraw_at = self
            .wants_continuous_redraw()
            .then_some(now + self.config.frame_time);
    }
}

impl<P> ApplicationHandler for RuntimeApp<P>
where
    P: SceneProvider,
{
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.suspended = false;
        self.create_window(event_loop);
    }

    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        self.prepare_suspend(event_loop);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            event_loop.set_control_flow(ControlFlow::Wait);
            return;
        }
        if self.immediate_redraw {
            event_loop.set_control_flow(ControlFlow::Wait);
            self.request_redraw_if_possible();
            return;
        }
        if !self.can_draw() {
            self.next_redraw_at = None;
            event_loop.set_control_flow(ControlFlow::Wait);
            return;
        }
        if self.wants_continuous_redraw() {
            let deadline = self
                .next_redraw_at
                .unwrap_or_else(|| Instant::now() + self.config.frame_time);
            event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
            if Instant::now() >= deadline {
                self.request_redraw_if_possible();
            }
        } else {
            self.next_redraw_at = None;
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.backend.suspend();
        self.window = None;
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self.window_id != Some(window_id) {
            return;
        }

        match event {
            WindowEvent::CloseRequested | WindowEvent::Destroyed => {
                event_loop.exit();
            }
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                self.handle_viewport_change(event_loop);
            }
            WindowEvent::Focused(focused) => {
                self.prepare_focus_change(event_loop, focused);
            }
            WindowEvent::Occluded(occluded) => {
                self.occluded = occluded;
                if !occluded {
                    self.request_immediate_redraw();
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = normalize_modifiers(modifiers.state());
                self.handle_engine_event(event_loop, EngineEvent::ModifiersChanged(self.modifiers));
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.mouse_position = Some((position.x as f32, position.y as f32));
                self.handle_engine_event(
                    event_loop,
                    EngineEvent::PointerMoved {
                        position: PointerPosition {
                            x: position.x as f32,
                            y: position.y as f32,
                        },
                        modifiers: self.modifiers,
                    },
                );
                self.request_immediate_redraw();
            }
            WindowEvent::CursorLeft { .. } => {
                self.mouse_position = None;
                self.handle_engine_event(event_loop, EngineEvent::PointerLeft);
                self.request_immediate_redraw();
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let normalized_state = normalize_button_state(state);
                match button {
                    WinitMouseButton::Left => {
                        self.left_down = matches!(state, ElementState::Pressed)
                    }
                    WinitMouseButton::Right => {
                        self.right_down = matches!(state, ElementState::Pressed)
                    }
                    WinitMouseButton::Middle => {
                        self.middle_down = matches!(state, ElementState::Pressed)
                    }
                    _ => {}
                }
                self.handle_engine_event(
                    event_loop,
                    EngineEvent::PointerButton {
                        button: normalize_pointer_button(button),
                        state: normalized_state,
                        modifiers: self.modifiers,
                    },
                );
                self.request_immediate_redraw();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let delta = normalize_scroll_delta(delta);
                self.accumulate_wheel(delta);
                self.handle_engine_event(
                    event_loop,
                    EngineEvent::Scroll {
                        delta,
                        modifiers: self.modifiers,
                    },
                );
                self.request_immediate_redraw();
            }
            WindowEvent::KeyboardInput {
                event,
                is_synthetic: _,
                ..
            } => {
                let normalized = KeyboardEvent {
                    logical_key: normalize_logical_key(&event.logical_key),
                    physical_key: normalize_physical_key(event.physical_key),
                    location: normalize_key_location(event.location),
                    state: normalize_button_state(event.state),
                    repeat: event.repeat,
                    modifiers: self.modifiers,
                };
                if let Some(text) = event.text.as_deref() {
                    self.maybe_emit_text_commit(event_loop, text);
                }
                self.handle_engine_event(event_loop, EngineEvent::Key(normalized));
            }
            WindowEvent::Ime(Ime::Commit(text)) => {
                self.handle_engine_event(
                    event_loop,
                    EngineEvent::TextInput(TextInputEvent::Commit(text)),
                );
            }
            WindowEvent::Ime(Ime::Preedit(text, cursor)) => {
                self.handle_engine_event(
                    event_loop,
                    EngineEvent::TextInput(TextInputEvent::Preedit { text, cursor }),
                );
            }
            WindowEvent::Ime(Ime::Enabled) | WindowEvent::Ime(Ime::Disabled) => {}
            WindowEvent::RedrawRequested => {
                self.draw_frame(event_loop);
            }
            _ => {}
        }
    }
}

fn normalize_modifiers(state: ModifiersState) -> KeyboardModifiers {
    KeyboardModifiers {
        shift: state.shift_key(),
        control: state.control_key(),
        alt: state.alt_key(),
        super_key: state.super_key(),
    }
}

fn normalize_button_state(state: ElementState) -> ButtonState {
    match state {
        ElementState::Pressed => ButtonState::Pressed,
        ElementState::Released => ButtonState::Released,
    }
}

fn normalize_pointer_button(button: WinitMouseButton) -> PointerButton {
    match button {
        WinitMouseButton::Left => PointerButton::Primary,
        WinitMouseButton::Right => PointerButton::Secondary,
        WinitMouseButton::Middle => PointerButton::Middle,
        WinitMouseButton::Back => PointerButton::Back,
        WinitMouseButton::Forward => PointerButton::Forward,
        WinitMouseButton::Other(value) => PointerButton::Other(value),
    }
}

fn normalize_scroll_delta(delta: MouseScrollDelta) -> ScrollDelta {
    match delta {
        MouseScrollDelta::LineDelta(x, y) => ScrollDelta::Lines { x, y },
        MouseScrollDelta::PixelDelta(position) => ScrollDelta::Pixels {
            x: position.x as f32,
            y: position.y as f32,
        },
    }
}

fn normalize_key_location(location: WinitKeyLocation) -> KeyLocation {
    match location {
        WinitKeyLocation::Standard => KeyLocation::Standard,
        WinitKeyLocation::Left => KeyLocation::Left,
        WinitKeyLocation::Right => KeyLocation::Right,
        WinitKeyLocation::Numpad => KeyLocation::Numpad,
    }
}

fn normalize_logical_key(key: &WinitKey) -> KeyIdentity {
    match key {
        WinitKey::Named(named) => KeyIdentity::Named(format!("{named:?}")),
        WinitKey::Character(value) => KeyIdentity::Character(value.to_string()),
        WinitKey::Dead(value) => KeyIdentity::Dead(*value),
        WinitKey::Unidentified(value) => KeyIdentity::Unidentified(format!("{value:?}")),
    }
}

fn normalize_physical_key(key: PhysicalKey) -> Option<String> {
    Some(match key {
        PhysicalKey::Code(code) => format!("{code:?}"),
        PhysicalKey::Unidentified(code) => format!("{code:?}"),
    })
}

#[cfg(test)]
mod tests {
    use winit::dpi::PhysicalPosition;
    use winit::event::{ElementState, MouseScrollDelta};
    use winit::keyboard::{
        Key, KeyCode, KeyLocation as WinitKeyLocation, ModifiersState, NamedKey, PhysicalKey,
    };

    use crate::input::{
        ButtonState, KeyIdentity, KeyLocation, KeyboardModifiers, PointerButton, ScrollDelta,
    };

    use super::{
        normalize_button_state, normalize_key_location, normalize_logical_key, normalize_modifiers,
        normalize_physical_key, normalize_pointer_button, normalize_scroll_delta,
    };

    #[test]
    fn modifiers_are_normalized_without_winit_types() {
        let state = ModifiersState::SHIFT | ModifiersState::CONTROL | ModifiersState::SUPER;
        assert_eq!(
            normalize_modifiers(state),
            KeyboardModifiers {
                shift: true,
                control: true,
                alt: false,
                super_key: true,
            }
        );
    }

    #[test]
    fn mouse_buttons_are_normalized() {
        assert_eq!(
            normalize_pointer_button(winit::event::MouseButton::Left),
            PointerButton::Primary
        );
        assert_eq!(
            normalize_pointer_button(winit::event::MouseButton::Other(7)),
            PointerButton::Other(7)
        );
    }

    #[test]
    fn wheel_delta_preserves_units() {
        assert_eq!(
            normalize_scroll_delta(MouseScrollDelta::LineDelta(1.5, -2.0)),
            ScrollDelta::Lines { x: 1.5, y: -2.0 }
        );
        assert_eq!(
            normalize_scroll_delta(MouseScrollDelta::PixelDelta(PhysicalPosition::new(
                8.0, 12.0
            ))),
            ScrollDelta::Pixels { x: 8.0, y: 12.0 }
        );
    }

    #[test]
    fn keys_are_normalized_into_engine_owned_ids() {
        assert_eq!(
            normalize_logical_key(&Key::Named(NamedKey::Enter)),
            KeyIdentity::Named("Enter".to_string())
        );
        assert_eq!(
            normalize_logical_key(&Key::Character("x".into())),
            KeyIdentity::Character("x".to_string())
        );
        assert_eq!(
            normalize_physical_key(PhysicalKey::Code(KeyCode::KeyA)),
            Some("KeyA".to_string())
        );
    }

    #[test]
    fn locations_and_button_state_are_normalized() {
        assert_eq!(
            normalize_key_location(WinitKeyLocation::Numpad),
            KeyLocation::Numpad
        );
        assert_eq!(
            normalize_button_state(ElementState::Pressed),
            ButtonState::Pressed
        );
        assert_eq!(
            normalize_button_state(ElementState::Released),
            ButtonState::Released
        );
    }
}
