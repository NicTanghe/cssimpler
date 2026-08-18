use std::num::NonZeroU32;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

use cssimpler_core::{
    Color, NativeMaterial, RenderNode, establishes_stacking_context, stacking_context_level,
};
#[cfg(test)]
use softbuffer::Rect;
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
use winit::window::{Window, WindowAttributes, WindowId};

use crate::input::{
    ButtonState, EngineEvent, KeyIdentity, KeyLocation, KeyboardEvent, KeyboardModifiers,
    PointerButton, PointerPosition, ScrollDelta, TextInputEvent, ViewportEvent,
};

use super::{
    ClipRect, ElementInteractionState, ElementPath, FrameInfo, FrameTimingStats, GlassRenderMode,
    MouseEventKind, RedrawSchedule, RendererError, Result, SceneProvider, WindowConfig,
    clip_pixel_bounds, dispatch_hover_transition_events, dispatch_mouse_event,
    drawable_viewport_size, duration_to_us, is_transparent, native_glass, pack_softbuffer_rgb,
    record_frame_timing_stats, redraw_auto_scroll_indicator_regions,
    render_scene_update_internal_from_roots_with_glass, render_to_buffer_internal_from_roots,
    render_to_buffer_internal_from_roots_with_alpha, resize_buffer, scrollbar,
    settle_element_interaction, should_present_frame_from_roots, should_suspend_updates,
    to_softbuffer_rgb_blue_noise, to_softbuffer_rgb_blue_noise_with_alpha,
};

const DEFAULT_NATIVE_GLASS_TINT: Color = Color::rgba(245, 250, 255, 128);
static SYSTEM_DRAG_OVERRIDE: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct NativeGlassSummary {
    required: bool,
    preferred_tint: Option<Color>,
}

#[derive(Clone, Copy)]
struct DeferredGlassStackingContext<'a> {
    node: &'a RenderNode,
    level: i32,
    order: u64,
}

fn summarize_native_glass(roots: &[RenderNode]) -> NativeGlassSummary {
    if !roots.iter().any(subtree_has_native_glass) {
        return NativeGlassSummary::default();
    }

    let mut summary = NativeGlassSummary::default();
    for root in roots {
        summarize_glass_stacking_context(root, &mut summary);
        if summary.preferred_tint.is_some() {
            break;
        }
    }
    summary
}

fn subtree_has_native_glass(node: &RenderNode) -> bool {
    node.style.native_material == NativeMaterial::Glass
        || node.children.iter().any(subtree_has_native_glass)
}

fn summarize_glass_stacking_context(node: &RenderNode, summary: &mut NativeGlassSummary) {
    summarize_glass_node(node, summary);
    if summary.preferred_tint.is_some() {
        return;
    }

    let mut deferred = Vec::new();
    let mut order = 0;
    collect_deferred_glass_stacking_contexts(node, &mut order, &mut deferred);
    deferred.sort_by(|left, right| {
        left.level
            .cmp(&right.level)
            .then(left.order.cmp(&right.order))
    });

    for entry in deferred.iter().filter(|entry| entry.level < 0) {
        summarize_glass_stacking_context(entry.node, summary);
        if summary.preferred_tint.is_some() {
            return;
        }
    }

    summarize_glass_normal_children(node, summary);
    if summary.preferred_tint.is_some() {
        return;
    }

    for entry in deferred.iter().filter(|entry| entry.level >= 0) {
        summarize_glass_stacking_context(entry.node, summary);
        if summary.preferred_tint.is_some() {
            return;
        }
    }
}

fn summarize_glass_normal_node(node: &RenderNode, summary: &mut NativeGlassSummary) {
    summarize_glass_node(node, summary);
    if summary.preferred_tint.is_none() {
        summarize_glass_normal_children(node, summary);
    }
}

fn summarize_glass_normal_children(node: &RenderNode, summary: &mut NativeGlassSummary) {
    for child in &node.children {
        if establishes_stacking_context(child) {
            continue;
        }
        summarize_glass_normal_node(child, summary);
        if summary.preferred_tint.is_some() {
            return;
        }
    }
}

fn collect_deferred_glass_stacking_contexts<'a>(
    node: &'a RenderNode,
    order: &mut u64,
    deferred: &mut Vec<DeferredGlassStackingContext<'a>>,
) {
    for child in &node.children {
        if establishes_stacking_context(child) {
            deferred.push(DeferredGlassStackingContext {
                node: child,
                level: stacking_context_level(child),
                order: *order,
            });
            *order = order.wrapping_add(1);
        } else {
            collect_deferred_glass_stacking_contexts(child, order, deferred);
        }
    }
}

fn summarize_glass_node(node: &RenderNode, summary: &mut NativeGlassSummary) {
    if node.style.native_material != NativeMaterial::Glass {
        return;
    }

    summary.required = true;
    if summary.preferred_tint.is_none() {
        summary.preferred_tint = node.style.glass_tint;
    }
}

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

pub(super) fn set_system_drag_override(active: bool) {
    SYSTEM_DRAG_OVERRIDE.store(active, Ordering::Relaxed);
}

fn system_drag_override_active() -> bool {
    SYSTEM_DRAG_OVERRIDE.load(Ordering::Relaxed)
}

struct RuntimeApp<P> {
    config: WindowConfig,
    scene_provider: P,
    context: Context<OwnedDisplayHandle>,
    surface: Option<Surface<OwnedDisplayHandle, Arc<Window>>>,
    window: Option<Arc<Window>>,
    window_id: Option<WindowId>,
    fatal_error: Option<RendererError>,
    buffer: Vec<u32>,
    alpha_buffer: Vec<u8>,
    buffer_width: usize,
    buffer_height: usize,
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
    previous_presented_scene: Option<Vec<RenderNode>>,
    previous_presented_indicator: Option<scrollbar::AutoScrollIndicator>,
    scrollbar_controller: scrollbar::ScrollbarController,
    native_glass_active: bool,
    native_glass_tint: Option<Color>,
    native_glass_diagnostic: Option<String>,
}

impl<P> RuntimeApp<P>
where
    P: SceneProvider,
{
    fn new(config: WindowConfig, scene_provider: P, context: Context<OwnedDisplayHandle>) -> Self {
        Self {
            config,
            scene_provider,
            context,
            surface: None,
            window: None,
            window_id: None,
            fatal_error: None,
            buffer: Vec::new(),
            alpha_buffer: Vec::new(),
            buffer_width: 0,
            buffer_height: 0,
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
            native_glass_active: false,
            native_glass_tint: None,
            native_glass_diagnostic: None,
        }
    }

    fn finish(mut self) -> Result<()> {
        self.clear_native_glass();
        self.surface = None;
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
        size.width > 0 && size.height > 0 && self.surface.is_some()
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
        match Surface::new(&self.context, Arc::clone(window)) {
            Ok(surface) => {
                self.surface = Some(surface);
                self.resize_surface(event_loop);
            }
            Err(error) => self.fail(event_loop, error),
        }
    }

    fn resize_surface(&mut self, event_loop: &ActiveEventLoop) {
        let Some(surface) = self.surface.as_mut() else {
            return;
        };
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let size = window.inner_size();
        let (Some(width), Some(height)) =
            (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
        else {
            return;
        };
        if let Err(error) = surface.resize(width, height) {
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

        let attributes = window_attributes_for_config(&self.config);
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                self.fail(event_loop, error);
                return;
            }
        };
        window.set_ime_allowed(true);
        finish_window_setup(&window, &self.config);
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
        self.clear_native_glass();
        let _ = self.scrollbar_controller.cancel_middle_button_auto_scroll();
        self.surface = None;
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

        let suppress_pointer_for_system_drag = should_suspend_updates(
            self.left_down,
            self.modifiers.super_key,
            system_drag_override_active(),
        );
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
        let resized = self.buffer_width != viewport.width || self.buffer_height != viewport.height;
        resize_buffer(
            &mut self.buffer,
            &mut self.buffer_width,
            &mut self.buffer_height,
            viewport.width,
            viewport.height,
            self.config.clear_color,
        );

        let native_glass = summarize_native_glass(&scene);
        self.sync_native_glass(native_glass);
        let glass_mode = self.glass_render_mode();
        let use_alpha_buffer = self.native_glass_active;
        if use_alpha_buffer {
            self.alpha_buffer.resize(
                self.buffer_width.saturating_mul(self.buffer_height),
                u8::MAX,
            );
        }

        if should_present_frame_from_roots(
            self.previous_presented_scene.as_deref(),
            &scene,
            self.previous_presented_indicator,
            auto_scroll_indicator,
            resized,
        ) {
            let paint_start = Instant::now();
            let paint_stats = if use_alpha_buffer {
                render_to_buffer_internal_from_roots_with_alpha(
                    &scene,
                    &mut self.buffer,
                    self.buffer_width,
                    self.buffer_height,
                    self.config.clear_color,
                    glass_mode,
                    Some(&mut self.alpha_buffer),
                )
            } else if resized {
                render_to_buffer_internal_from_roots(
                    &scene,
                    &mut self.buffer,
                    self.buffer_width,
                    self.buffer_height,
                    self.config.clear_color,
                    glass_mode,
                )
            } else if let Some(previous_scene) = self.previous_presented_scene.as_deref() {
                render_scene_update_internal_from_roots_with_glass(
                    previous_scene,
                    &scene,
                    &mut self.buffer,
                    self.buffer_width,
                    self.buffer_height,
                    self.config.clear_color,
                    glass_mode,
                )
            } else {
                render_to_buffer_internal_from_roots(
                    &scene,
                    &mut self.buffer,
                    self.buffer_width,
                    self.buffer_height,
                    self.config.clear_color,
                    glass_mode,
                )
            };
            frame_stats.paint_us = duration_to_us(paint_start.elapsed());
            frame_stats.render_workers = paint_stats.workers;
            frame_stats.dirty_regions = paint_stats.dirty_regions;
            frame_stats.dirty_jobs = paint_stats.dirty_jobs;
            frame_stats.damage_pixels = paint_stats.damage_pixels;
            frame_stats.painted_pixels = paint_stats.painted_pixels;
            frame_stats.scene_passes = paint_stats.scene_passes;
            frame_stats.paint_mode = paint_stats.mode;
            frame_stats.paint_reason = paint_stats.reason;

            let present_start = Instant::now();
            redraw_auto_scroll_indicator_regions(
                self.previous_presented_indicator,
                auto_scroll_indicator,
                &scene,
                &mut self.buffer,
                self.buffer_width,
                self.buffer_height,
                self.config.clear_color,
                glass_mode,
                if use_alpha_buffer {
                    Some(&mut self.alpha_buffer)
                } else {
                    None
                },
            );
            if self.native_glass_active && native_glass::uses_custom_presenter() {
                let Some(window) = self.window.as_ref() else {
                    return;
                };
                match native_glass::present(
                    window,
                    &self.buffer,
                    Some(self.alpha_buffer.as_slice()),
                    self.buffer_width,
                    self.buffer_height,
                    self.scale_factor,
                ) {
                    Ok(true) => {}
                    Ok(false) => {}
                    Err(error) => {
                        self.fail(event_loop, RendererError::Surface(error));
                        return;
                    }
                }
            } else {
                let surface_result = if let Some(surface) = self.surface.as_mut() {
                    match surface.buffer_mut() {
                        Ok(mut target) => {
                            let target_width = target.width().get() as usize;
                            let target_height = target.height().get() as usize;
                            copy_render_buffer_into_surface(
                                &mut target,
                                target_width,
                                target_height,
                                &self.buffer,
                                self.buffer_width,
                                self.buffer_height,
                                if self.native_glass_active {
                                    Some(self.alpha_buffer.as_slice())
                                } else {
                                    None
                                },
                                pack_softbuffer_rgb(self.config.clear_color),
                                self.native_glass_active,
                                paint_stats.damage_rects.as_deref(),
                            );
                            // Transparent pixels reveal native glass on Windows. Present the full
                            // surface so cleared transparent regions also replace stale pixels
                            // during resize and layout changes.
                            target.present().map_err(RendererError::from)
                        }
                        Err(error) => Err(RendererError::from(error)),
                    }
                } else {
                    Ok(())
                };
                if let Err(error) = surface_result {
                    self.fail(event_loop, error);
                    return;
                }
            }
            frame_stats.present_us = duration_to_us(present_start.elapsed());
            self.previous_presented_scene = Some(scene);
            self.previous_presented_indicator = auto_scroll_indicator;
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

    fn sync_native_glass(&mut self, summary: NativeGlassSummary) {
        if !summary.required {
            self.clear_native_glass();
            self.clear_native_glass_diagnostic();
            return;
        }

        if !self.config.glass_capable {
            self.native_glass_active = false;
            self.native_glass_tint = None;
            self.note_native_glass_diagnostic(
                "native glass requested, but WindowConfig is not glass-capable; call WindowConfig::with_glass_capable(true). Using renderer fallback.",
            );
            return;
        }

        let tint = summary.preferred_tint.unwrap_or(DEFAULT_NATIVE_GLASS_TINT);
        if self.native_glass_active && self.native_glass_tint == Some(tint) {
            return;
        }

        let Some(window) = self.window.as_ref() else {
            return;
        };

        match native_glass::apply(window, tint) {
            Ok(true) => {
                window.set_transparent(true);
                self.native_glass_active = true;
                self.native_glass_tint = Some(tint);
                self.clear_native_glass_diagnostic();
            }
            Ok(false) => {
                window.set_transparent(false);
                self.native_glass_active = false;
                self.native_glass_tint = None;
                self.note_native_glass_diagnostic(
                    "native glass is unavailable on this platform. Using renderer fallback.",
                );
            }
            Err(error) => {
                window.set_transparent(false);
                self.native_glass_active = false;
                self.native_glass_tint = None;
                self.note_native_glass_diagnostic(format!(
                    "native glass failed: {error}. Using renderer fallback."
                ));
            }
        }
    }

    fn clear_native_glass(&mut self) {
        if !self.native_glass_active && self.native_glass_tint.is_none() {
            return;
        }

        if let Some(window) = self.window.as_ref() {
            let _ = native_glass::clear(window);
            window.set_transparent(window_uses_native_glass(&self.config));
        }
        self.native_glass_active = false;
        self.native_glass_tint = None;
    }

    fn glass_render_mode(&self) -> GlassRenderMode {
        if self.native_glass_active {
            if native_glass::uses_custom_presenter() {
                GlassRenderMode::NativeWithTint
            } else {
                GlassRenderMode::Native
            }
        } else {
            GlassRenderMode::Fallback
        }
    }

    fn note_native_glass_diagnostic(&mut self, message: impl Into<String>) {
        let message = message.into();
        if self.native_glass_diagnostic.as_deref() == Some(message.as_str()) {
            return;
        }

        eprintln!("cssimpler: {message}");
        self.native_glass_diagnostic = Some(message);
    }

    fn clear_native_glass_diagnostic(&mut self) {
        self.native_glass_diagnostic = None;
    }
}

fn window_attributes_for_config(config: &WindowConfig) -> WindowAttributes {
    #[allow(unused_mut)]
    let mut attributes = Window::default_attributes()
        .with_title(config.title.clone())
        .with_inner_size(LogicalSize::new(config.width as f64, config.height as f64))
        .with_resizable(true)
        .with_transparent(window_uses_native_glass(config))
        .with_decorations(config.decorations);

    #[cfg(target_os = "windows")]
    if config.glass_capable && !config.decorations {
        use winit::dpi::PhysicalSize;
        use winit::platform::windows::WindowAttributesExtWindows;

        attributes = attributes
            .with_undecorated_shadow(false)
            .with_inner_size(PhysicalSize::new(0, 0));
    }

    attributes
}

fn window_uses_native_glass(config: &WindowConfig) -> bool {
    config.glass_capable && native_glass::requires_initial_transparency()
}

fn finish_window_setup(window: &Window, config: &WindowConfig) {
    #[cfg(target_os = "windows")]
    if config.glass_capable && !config.decorations {
        use winit::dpi::PhysicalSize;
        use winit::platform::windows::WindowExtWindows;

        window.set_undecorated_shadow(true);
        let _ = window.request_inner_size(PhysicalSize::new(
            config.width.max(1) as u32,
            config.height.max(1) as u32,
        ));
    }

    #[cfg(not(target_os = "windows"))]
    let _ = (window, config);
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
        self.surface = None;
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

fn copy_render_buffer_into_surface(
    target: &mut [u32],
    target_width: usize,
    target_height: usize,
    source: &[u32],
    source_width: usize,
    source_height: usize,
    source_alpha: Option<&[u8]>,
    clear: u32,
    preserve_transparency: bool,
    damage_rects: Option<&[ClipRect]>,
) {
    debug_assert_eq!(target.len(), target_width.saturating_mul(target_height));
    debug_assert_eq!(source.len(), source_width.saturating_mul(source_height));
    debug_assert!(source_alpha.is_none_or(|alpha| alpha.len() == source.len()));

    if target_width == source_width && target_height == source_height {
        if let Some(rects) = damage_rects {
            for &rect in rects {
                let Some((x0, y0, x1, y1)) = clip_pixel_bounds(rect, source_width, source_height)
                else {
                    continue;
                };
                for row in y0 as usize..y1 as usize {
                    let row_start = row * source_width;
                    for column in x0 as usize..x1 as usize {
                        let index = row_start + column;
                        target[index] = surface_pixel(
                            source[index],
                            source_alpha.map(|alpha| alpha[index]),
                            column,
                            row,
                            clear,
                            preserve_transparency,
                        );
                    }
                }
            }
            return;
        }

        for row in 0..source_height {
            let row_start = row * source_width;
            for column in 0..source_width {
                let index = row_start + column;
                target[index] = surface_pixel(
                    source[index],
                    source_alpha.map(|alpha| alpha[index]),
                    column,
                    row,
                    clear,
                    preserve_transparency,
                );
            }
        }
        return;
    }

    target.fill(clear);
    let copy_width = source_width.min(target_width);
    let copy_height = source_height.min(target_height);
    for row in 0..copy_height {
        let src_row = row * source_width;
        let dst_row = row * target_width;
        for column in 0..copy_width {
            let source_index = src_row + column;
            target[dst_row + column] = surface_pixel(
                source[source_index],
                source_alpha.map(|alpha| alpha[source_index]),
                column,
                row,
                clear,
                preserve_transparency,
            );
        }
    }
}

fn surface_pixel(
    pixel: u32,
    alpha: Option<u8>,
    column: usize,
    row: usize,
    clear: u32,
    preserve_transparency: bool,
) -> u32 {
    if let Some(alpha) = alpha {
        if alpha == 0 {
            return if preserve_transparency { 0 } else { clear };
        }
        return to_softbuffer_rgb_blue_noise_with_alpha(pixel, alpha, column, row);
    }

    if is_transparent(pixel) && !preserve_transparency {
        clear
    } else {
        to_softbuffer_rgb_blue_noise(pixel, column, row)
    }
}

#[cfg(test)]
fn non_transparent_damage_rects(
    source: &[u32],
    source_width: usize,
    source_height: usize,
    target_width: usize,
    target_height: usize,
) -> Vec<Rect> {
    let copy_width = source_width.min(target_width);
    let copy_height = source_height.min(target_height);
    if copy_width == 0 || copy_height == 0 {
        return Vec::new();
    }

    let mut runs = Vec::<DamageRun>::new();
    for row in 0..copy_height {
        let mut column = 0;
        while column < copy_width {
            while column < copy_width && is_transparent(source[row * source_width + column]) {
                column += 1;
            }
            if column >= copy_width {
                break;
            }
            let x0 = column;
            while column < copy_width && !is_transparent(source[row * source_width + column]) {
                column += 1;
            }
            let x1 = column;
            if let Some(previous) = runs.last_mut()
                && previous.x0 == x0
                && previous.x1 == x1
                && previous.y1 == row
            {
                previous.y1 += 1;
                continue;
            }
            runs.push(DamageRun {
                x0,
                x1,
                y0: row,
                y1: row + 1,
            });
        }
    }

    runs.into_iter().filter_map(|run| run.into_rect()).collect()
}

#[derive(Clone, Copy, Debug)]
#[cfg(test)]
struct DamageRun {
    x0: usize,
    x1: usize,
    y0: usize,
    y1: usize,
}

#[cfg(test)]
impl DamageRun {
    fn into_rect(self) -> Option<Rect> {
        Some(Rect {
            x: u32::try_from(self.x0).ok()?,
            y: u32::try_from(self.y0).ok()?,
            width: NonZeroU32::new(u32::try_from(self.x1.checked_sub(self.x0)?).ok()?)?,
            height: NonZeroU32::new(u32::try_from(self.y1.checked_sub(self.y0)?).ok()?)?,
        })
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
    use cssimpler_core::{
        Color, ExtractedScene, LayoutBox, NativeMaterial, RenderNode, VisualStyle, ZIndex,
    };
    use winit::dpi::PhysicalPosition;
    use winit::event::{ElementState, MouseScrollDelta};
    use winit::keyboard::{
        Key, KeyCode, KeyLocation as WinitKeyLocation, ModifiersState, NamedKey, PhysicalKey,
    };

    use crate::input::{
        ButtonState, KeyIdentity, KeyLocation, KeyboardModifiers, PointerButton, ScrollDelta,
    };
    use crate::{
        ClipRect, pack_rgb, pack_transparent, to_softbuffer_rgb_blue_noise,
        to_softbuffer_rgb_blue_noise_with_alpha,
    };

    use super::{
        copy_render_buffer_into_surface, non_transparent_damage_rects, normalize_button_state,
        normalize_key_location, normalize_logical_key, normalize_modifiers, normalize_physical_key,
        normalize_pointer_button, normalize_scroll_delta, summarize_native_glass,
        window_uses_native_glass,
    };
    use crate::WindowConfig;

    fn glass_node(tint: Option<Color>) -> RenderNode {
        RenderNode::container(LayoutBox::new(0.0, 0.0, 20.0, 20.0)).with_style(VisualStyle {
            native_material: NativeMaterial::Glass,
            glass_tint: tint,
            ..VisualStyle::default()
        })
    }

    fn stacking_glass_node(level: i32, tint: Option<Color>) -> RenderNode {
        glass_node(tint).with_style(VisualStyle {
            native_material: NativeMaterial::Glass,
            glass_tint: tint,
            positioned: true,
            z_index: ZIndex::Integer(level),
            ..VisualStyle::default()
        })
    }

    fn assert_native_glass_summary_matches_extraction(label: &str, roots: &[RenderNode]) {
        let summary = summarize_native_glass(roots);
        let extracted = ExtractedScene::from_render_roots(roots);
        assert_eq!(
            summary.required,
            extracted.requires_native_glass(),
            "native-glass requirement differs for {label}"
        );
        assert_eq!(
            summary.preferred_tint,
            extracted.preferred_glass_tint(),
            "preferred native-glass tint differs for {label}"
        );
    }

    #[test]
    fn lightweight_native_glass_summary_matches_extracted_scene() {
        let first = Color::rgba(20, 40, 60, 80);
        let second = Color::rgba(90, 110, 130, 150);

        let cases = [
            ("empty", Vec::new()),
            (
                "plain nested tree",
                vec![
                    RenderNode::container(LayoutBox::new(0.0, 0.0, 80.0, 80.0))
                        .with_child(RenderNode::container(LayoutBox::new(1.0, 1.0, 10.0, 10.0)))
                        .with_child(
                            RenderNode::container(LayoutBox::new(2.0, 2.0, 10.0, 10.0)).with_style(
                                VisualStyle {
                                    positioned: true,
                                    z_index: ZIndex::Integer(7),
                                    ..VisualStyle::default()
                                },
                            ),
                        ),
                ],
            ),
            ("untinted glass", vec![glass_node(None)]),
            (
                "untinted ancestor then tinted descendant",
                vec![glass_node(None).with_child(glass_node(Some(first)))],
            ),
            (
                "multiple roots",
                vec![glass_node(None), glass_node(Some(second))],
            ),
            (
                "equal-level deferred contexts",
                vec![
                    RenderNode::container(LayoutBox::new(0.0, 0.0, 80.0, 80.0))
                        .with_child(
                            RenderNode::container(LayoutBox::new(0.0, 0.0, 40.0, 40.0))
                                .with_child(stacking_glass_node(2, Some(first))),
                        )
                        .with_child(stacking_glass_node(2, Some(second))),
                ],
            ),
        ];

        for (label, roots) in cases {
            assert_native_glass_summary_matches_extraction(label, &roots);
        }
    }

    #[test]
    fn lightweight_native_glass_summary_preserves_stacking_order_tint_preference() {
        let normal_tint = Color::rgba(10, 20, 30, 40);
        let negative_tint = Color::rgba(50, 60, 70, 80);
        let positive_tint = Color::rgba(90, 100, 110, 120);

        let negative_before_normal = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 80.0, 80.0))
                .with_child(glass_node(Some(normal_tint)))
                .with_child(stacking_glass_node(-3, Some(negative_tint)))
                .with_child(stacking_glass_node(4, Some(positive_tint))),
        ];
        assert_native_glass_summary_matches_extraction(
            "negative stacking context before normal flow",
            &negative_before_normal,
        );
        assert_eq!(
            summarize_native_glass(&negative_before_normal).preferred_tint,
            Some(negative_tint)
        );

        let positive_after_normal = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 80.0, 80.0))
                .with_child(
                    RenderNode::container(LayoutBox::new(0.0, 0.0, 40.0, 40.0))
                        .with_child(stacking_glass_node(4, Some(positive_tint))),
                )
                .with_child(glass_node(Some(normal_tint))),
        ];
        assert_native_glass_summary_matches_extraction(
            "promoted positive context after normal flow",
            &positive_after_normal,
        );
        assert_eq!(
            summarize_native_glass(&positive_after_normal).preferred_tint,
            Some(normal_tint)
        );
    }

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

    #[test]
    fn blit_to_surface_copies_rows_when_target_is_wider() {
        let source = vec![
            pack_rgb(Color::rgb(1, 2, 3)),
            pack_rgb(Color::rgb(4, 5, 6)),
            pack_rgb(Color::rgb(7, 8, 9)),
            pack_rgb(Color::rgb(10, 11, 12)),
            pack_rgb(Color::rgb(13, 14, 15)),
            pack_rgb(Color::rgb(16, 17, 18)),
        ];
        let mut target = vec![9; 10];
        copy_render_buffer_into_surface(&mut target, 5, 2, &source, 3, 2, None, 0, false, None);
        assert_eq!(
            target,
            vec![
                to_softbuffer_rgb_blue_noise(source[0], 0, 0),
                to_softbuffer_rgb_blue_noise(source[1], 1, 0),
                to_softbuffer_rgb_blue_noise(source[2], 2, 0),
                0,
                0,
                to_softbuffer_rgb_blue_noise(source[3], 0, 1),
                to_softbuffer_rgb_blue_noise(source[4], 1, 1),
                to_softbuffer_rgb_blue_noise(source[5], 2, 1),
                0,
                0,
            ]
        );
    }

    #[test]
    fn blit_to_surface_copies_rows_when_target_is_narrower() {
        let source = vec![
            pack_rgb(Color::rgb(1, 2, 3)),
            pack_rgb(Color::rgb(4, 5, 6)),
            pack_rgb(Color::rgb(7, 8, 9)),
            pack_rgb(Color::rgb(10, 11, 12)),
            pack_rgb(Color::rgb(13, 14, 15)),
            pack_rgb(Color::rgb(16, 17, 18)),
            pack_rgb(Color::rgb(19, 20, 21)),
            pack_rgb(Color::rgb(22, 23, 24)),
        ];
        let mut target = vec![9; 6];
        copy_render_buffer_into_surface(&mut target, 3, 2, &source, 4, 2, None, 0, false, None);
        assert_eq!(
            target,
            vec![
                to_softbuffer_rgb_blue_noise(source[0], 0, 0),
                to_softbuffer_rgb_blue_noise(source[1], 1, 0),
                to_softbuffer_rgb_blue_noise(source[2], 2, 0),
                to_softbuffer_rgb_blue_noise(source[4], 0, 1),
                to_softbuffer_rgb_blue_noise(source[5], 1, 1),
                to_softbuffer_rgb_blue_noise(source[6], 2, 1),
            ]
        );
    }

    #[test]
    fn blit_to_surface_can_preserve_transparent_glass_pixels() {
        let clear = 0x00ff00;
        let source = vec![
            pack_rgb(Color::rgb(1, 2, 3)),
            pack_transparent(),
            pack_rgb(Color::rgb(7, 8, 9)),
        ];
        let mut target = vec![clear; 3];

        copy_render_buffer_into_surface(&mut target, 3, 1, &source, 3, 1, None, clear, true, None);

        assert_eq!(
            target,
            vec![
                to_softbuffer_rgb_blue_noise(source[0], 0, 0),
                0,
                to_softbuffer_rgb_blue_noise(source[2], 2, 0)
            ]
        );

        copy_render_buffer_into_surface(&mut target, 3, 1, &source, 3, 1, None, clear, false, None);

        assert_eq!(target[1], clear);
    }

    #[test]
    fn blit_to_surface_uses_external_eight_bit_alpha() {
        let source = vec![
            pack_rgb(Color::rgb(100, 200, 50)),
            pack_rgb(Color::rgb(20, 40, 80)),
            pack_rgb(Color::rgb(1, 2, 3)),
        ];
        let alpha = [128, 0, 255];
        let clear = 0x00ff00;
        let mut target = vec![clear; 3];

        copy_render_buffer_into_surface(
            &mut target,
            3,
            1,
            &source,
            3,
            1,
            Some(&alpha),
            clear,
            true,
            None,
        );

        assert_eq!(
            target,
            vec![
                to_softbuffer_rgb_blue_noise_with_alpha(source[0], alpha[0], 0, 0),
                0,
                to_softbuffer_rgb_blue_noise_with_alpha(source[2], alpha[2], 2, 0),
            ]
        );
    }

    #[test]
    fn blit_to_surface_respects_damage_rects() {
        let source = vec![
            pack_rgb(Color::rgb(10, 20, 30)),
            pack_rgb(Color::rgb(40, 50, 60)),
            pack_rgb(Color::rgb(70, 80, 90)),
            pack_rgb(Color::rgb(100, 110, 120)),
        ];
        let clear = 0x000000;
        let mut target = vec![clear; 4];
        let damage = [ClipRect {
            x0: 1.0,
            y0: 0.0,
            x1: 2.0,
            y1: 2.0,
        }];

        copy_render_buffer_into_surface(
            &mut target,
            2,
            2,
            &source,
            2,
            2,
            None,
            clear,
            false,
            Some(&damage),
        );

        assert_eq!(
            target,
            vec![
                clear,
                to_softbuffer_rgb_blue_noise(source[1], 1, 0),
                clear,
                to_softbuffer_rgb_blue_noise(source[3], 1, 1),
            ]
        );
    }

    #[test]
    fn glass_capable_windows_and_macos_start_transparent_when_required_by_native_backend() {
        let config = WindowConfig::new("glass", 320, 180).with_glass_capable(true);

        #[cfg(any(target_os = "windows", target_os = "macos"))]
        assert!(window_uses_native_glass(&config));

        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        assert!(!window_uses_native_glass(&config));
    }

    #[test]
    fn transparent_glass_damage_skips_reveal_holes() {
        let source = vec![
            pack_rgb(Color::rgb(1, 2, 3)),
            pack_rgb(Color::rgb(4, 5, 6)),
            pack_transparent(),
            pack_rgb(Color::rgb(7, 8, 9)),
            pack_rgb(Color::rgb(10, 11, 12)),
            pack_transparent(),
        ];

        let damage = non_transparent_damage_rects(&source, 3, 2, 3, 2);
        let simplified = damage
            .iter()
            .map(|rect| (rect.x, rect.y, rect.width.get(), rect.height.get()))
            .collect::<Vec<_>>();

        assert_eq!(simplified, vec![(0, 0, 2, 2)]);
    }
}
