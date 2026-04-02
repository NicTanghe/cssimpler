use std::error::Error;
use std::fmt::{Display, Formatter};
use std::time::{Duration, Instant};

mod color;
mod fonts;
mod scrollbar;

use crate::color::{resolve_angle_stops, resolve_length_stops, sample_gradient};
use cssimpler_core::{
    BackgroundLayer, CircleRadius, Color, ConicGradient, CornerRadius, ElementInteractionState,
    ElementPath, EllipseRadius, EventHandler, GradientDirection, GradientHorizontal, GradientPoint,
    GradientVertical, Insets, LayoutBox, LinearGradient, LinearRgba, RadialGradient, RadialShape,
    RenderKind, RenderNode, ShapeExtent,
};
use minifb::{Key, MouseButton, MouseMode, Window, WindowOptions};

const MAX_INCREMENTAL_DIRTY_REGIONS: usize = 8;
const MAX_INCREMENTAL_DIRTY_AREA_RATIO: f32 = 0.5;
#[derive(Clone, Copy, Debug)]
pub struct FrameInfo {
    pub frame_index: u64,
    pub delta: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewportSize {
    pub width: usize,
    pub height: usize,
}

impl ViewportSize {
    pub const fn new(width: usize, height: usize) -> Self {
        Self {
            width: if width == 0 { 1 } else { width },
            height: if height == 0 { 1 } else { height },
        }
    }
}

impl Default for ViewportSize {
    fn default() -> Self {
        Self::new(1, 1)
    }
}

fn drawable_viewport_size(width: usize, height: usize) -> Option<ViewportSize> {
    if width == 0 || height == 0 {
        None
    } else {
        Some(ViewportSize::new(width, height))
    }
}

#[derive(Clone, Debug)]
pub struct WindowConfig {
    pub title: String,
    pub width: usize,
    pub height: usize,
    pub clear_color: Color,
    pub frame_time: Duration,
    pub middle_button_auto_scroll: bool,
}

impl WindowConfig {
    pub fn new(title: impl Into<String>, width: usize, height: usize) -> Self {
        Self {
            title: title.into(),
            width,
            height,
            clear_color: Color::rgb(248, 250, 252),
            frame_time: Duration::from_millis(16),
            middle_button_auto_scroll: true,
        }
    }
}

#[derive(Debug)]
pub enum RendererError {
    Window(minifb::Error),
}

impl Display for RendererError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Window(source) => write!(f, "renderer backend error: {source}"),
        }
    }
}

impl Error for RendererError {}

impl From<minifb::Error> for RendererError {
    fn from(value: minifb::Error) -> Self {
        Self::Window(value)
    }
}

pub type Result<T> = std::result::Result<T, RendererError>;

pub trait SceneProvider {
    fn update(&mut self, frame: FrameInfo);

    fn scene(&self) -> &[RenderNode];

    fn set_viewport(&mut self, viewport: ViewportSize) {
        let _ = viewport;
    }

    fn capture_scene(&mut self) -> Vec<RenderNode> {
        self.scene().to_vec()
    }

    fn set_element_interaction(&mut self, interaction: ElementInteractionState) -> bool {
        let _ = interaction;
        false
    }
}

struct ClosureSceneProvider<F> {
    render_scene: F,
    scene: Vec<RenderNode>,
}

impl<F> ClosureSceneProvider<F> {
    fn new(render_scene: F) -> Self {
        Self {
            render_scene,
            scene: Vec::new(),
        }
    }
}

struct ViewportClosureSceneProvider<F> {
    render_scene: F,
    scene: Vec<RenderNode>,
    viewport: ViewportSize,
}

impl<F> ViewportClosureSceneProvider<F> {
    fn new(render_scene: F) -> Self {
        Self {
            render_scene,
            scene: Vec::new(),
            viewport: ViewportSize::default(),
        }
    }
}

impl<F> SceneProvider for ClosureSceneProvider<F>
where
    F: FnMut(FrameInfo) -> Vec<RenderNode>,
{
    fn update(&mut self, frame: FrameInfo) {
        self.scene = (self.render_scene)(frame);
    }

    fn scene(&self) -> &[RenderNode] {
        &self.scene
    }

    fn capture_scene(&mut self) -> Vec<RenderNode> {
        std::mem::take(&mut self.scene)
    }
}

impl<F> SceneProvider for ViewportClosureSceneProvider<F>
where
    F: FnMut(FrameInfo, ViewportSize) -> Vec<RenderNode>,
{
    fn update(&mut self, frame: FrameInfo) {
        self.scene = (self.render_scene)(frame, self.viewport);
    }

    fn scene(&self) -> &[RenderNode] {
        &self.scene
    }

    fn set_viewport(&mut self, viewport: ViewportSize) {
        self.viewport = viewport;
    }

    fn capture_scene(&mut self) -> Vec<RenderNode> {
        std::mem::take(&mut self.scene)
    }
}

pub fn run<F>(config: WindowConfig, render_scene: F) -> Result<()>
where
    F: FnMut(FrameInfo) -> Vec<RenderNode>,
{
    run_with_scene_provider(config, ClosureSceneProvider::new(render_scene))
}

pub fn run_with_viewport<F>(config: WindowConfig, render_scene: F) -> Result<()>
where
    F: FnMut(FrameInfo, ViewportSize) -> Vec<RenderNode>,
{
    run_with_scene_provider(config, ViewportClosureSceneProvider::new(render_scene))
}

pub fn run_with_scene_provider<P>(config: WindowConfig, mut scene_provider: P) -> Result<()>
where
    P: SceneProvider,
{
    let initial_viewport = ViewportSize::new(config.width, config.height);
    scene_provider.set_viewport(initial_viewport);
    scene_provider.update(FrameInfo {
        frame_index: 0,
        delta: Duration::ZERO,
    });
    let mut initial_scene = scene_provider.capture_scene();
    let mut scrollbar_controller = scrollbar::ScrollbarController::default();
    scrollbar_controller.apply_to_scene(&mut initial_scene);
    let initial_indicator = scrollbar_controller.auto_scroll_indicator();

    let mut window = Window::new(&config.title, config.width, config.height, window_options())?;
    window.set_target_fps(frame_time_to_fps(config.frame_time));

    let mut buffer_width = config.width.max(1);
    let mut buffer_height = config.height.max(1);
    let mut buffer = vec![pack_rgb(config.clear_color); buffer_width * buffer_height];
    render_to_buffer(
        &initial_scene,
        &mut buffer,
        buffer_width,
        buffer_height,
        config.clear_color,
    );
    if let Some(indicator) = initial_indicator {
        scrollbar::draw_auto_scroll_indicator(indicator, &mut buffer, buffer_width, buffer_height);
    }
    window.update_with_buffer(&buffer, buffer_width, buffer_height)?;

    let mut last_frame = Instant::now();
    let mut frame_index = 1_u64;
    let mut previous_left_down = false;
    let mut previous_middle_down = false;
    let mut suppress_left_pointer_until_release = false;
    let mut element_interaction = ElementInteractionState::default();
    let mut previous_presented_scene: Option<Vec<RenderNode>> = Some(initial_scene);
    let mut previous_presented_indicator: Option<scrollbar::AutoScrollIndicator> = initial_indicator;

    while window.is_open() && !window.is_key_down(Key::Escape) {
        let now = Instant::now();
        let delta = now.saturating_duration_since(last_frame);
        last_frame = now;

        let left_down = window.get_mouse_down(MouseButton::Left);
        let middle_down = window.get_mouse_down(MouseButton::Middle);
        let suppress_pointer_for_system_drag = should_suspend_updates(
            left_down,
            window.is_key_down(Key::LeftSuper),
            window.is_key_down(Key::RightSuper),
        );
        if suppress_pointer_for_system_drag {
            suppress_left_pointer_until_release = true;
        } else if !left_down {
            suppress_left_pointer_until_release = false;
        }

        let interactive_left_down =
            left_down && !suppress_pointer_for_system_drag && !suppress_left_pointer_until_release;

        let (window_width, window_height) = window.get_size();
        let Some(viewport) = drawable_viewport_size(window_width, window_height) else {
            let _ = scrollbar_controller.cancel_middle_button_auto_scroll();
            suppress_left_pointer_until_release = false;
            previous_left_down = false;
            previous_middle_down = false;
            window.update();
            continue;
        };
        scene_provider.set_viewport(viewport);
        let frame = FrameInfo { frame_index, delta };
        scene_provider.update(frame);
        let mut scene = scene_provider.capture_scene();
        scrollbar_controller.apply_to_scene(&mut scene);
        let mouse_position = window.get_mouse_pos(MouseMode::Clamp);
        let click_started = interactive_left_down && !previous_left_down;
        let middle_click_started = middle_down && !previous_middle_down;
        let auto_scroll_canceled_click =
            click_started && scrollbar_controller.cancel_middle_button_auto_scroll();
        if config.middle_button_auto_scroll {
            if middle_click_started {
                let _ =
                    scrollbar_controller.toggle_middle_button_auto_scroll(&scene, mouse_position);
            }
        } else {
            let _ = scrollbar_controller.cancel_middle_button_auto_scroll();
        }
        let _ =
            scrollbar_controller.step_middle_button_auto_scroll(&mut scene, mouse_position, delta);
        scrollbar_controller.handle_wheel(&mut scene, mouse_position, window.get_scroll_wheel());
        let scrollbar_consumed_click = scrollbar_controller.handle_pointer(
            &mut scene,
            mouse_position,
            interactive_left_down,
            click_started,
        );
        let normal_click_started =
            click_started && !auto_scroll_canceled_click && !scrollbar_consumed_click;
        settle_element_interaction(
            &mut scene_provider,
            frame,
            &mut scene,
            &mut scrollbar_controller,
            mouse_position,
            interactive_left_down,
            normal_click_started,
            &mut element_interaction,
        );

        let click_triggered_rerender = if normal_click_started {
            if let Some((mouse_x, mouse_y)) = mouse_position {
                dispatch_click(&scene, mouse_x, mouse_y)
            } else {
                false
            }
        } else {
            false
        };

        if click_triggered_rerender {
            scene_provider.update(frame);
            scene = scene_provider.capture_scene();
            scrollbar_controller.apply_to_scene(&mut scene);
            scrollbar_controller.handle_pointer(
                &mut scene,
                mouse_position,
                interactive_left_down,
                false,
            );
            settle_element_interaction(
                &mut scene_provider,
                frame,
                &mut scene,
                &mut scrollbar_controller,
                mouse_position,
                interactive_left_down,
                false,
                &mut element_interaction,
            );
        }

        let auto_scroll_indicator = scrollbar_controller.auto_scroll_indicator();

        let resized = buffer_width != viewport.width || buffer_height != viewport.height;
        resize_buffer(
            &mut buffer,
            &mut buffer_width,
            &mut buffer_height,
            viewport.width,
            viewport.height,
            config.clear_color,
        );
        if should_present_frame(
            previous_presented_scene.as_deref(),
            &scene,
            previous_presented_indicator,
            auto_scroll_indicator,
            resized,
        ) {
            if resized {
                render_to_buffer(
                    &scene,
                    &mut buffer,
                    buffer_width,
                    buffer_height,
                    config.clear_color,
                );
            } else if let Some(previous_scene) = previous_presented_scene.as_deref() {
                render_scene_update(
                    previous_scene,
                    &scene,
                    &mut buffer,
                    buffer_width,
                    buffer_height,
                    config.clear_color,
                );
            } else {
                render_to_buffer(
                    &scene,
                    &mut buffer,
                    buffer_width,
                    buffer_height,
                    config.clear_color,
                );
            }
            redraw_auto_scroll_indicator_regions(
                previous_presented_indicator,
                auto_scroll_indicator,
                &scene,
                &mut buffer,
                buffer_width,
                buffer_height,
                config.clear_color,
            );
            window.update_with_buffer(&buffer, buffer_width, buffer_height)?;
            previous_presented_scene = Some(scene.clone());
            previous_presented_indicator = auto_scroll_indicator;
        } else {
            window.update();
        }

        previous_left_down = interactive_left_down;
        previous_middle_down = middle_down;
        frame_index += 1;
    }

    Ok(())
}

fn window_options() -> WindowOptions {
    WindowOptions {
        resize: true,
        ..WindowOptions::default()
    }
}

pub fn render_to_buffer(
    scene: &[RenderNode],
    buffer: &mut [u32],
    width: usize,
    height: usize,
    clear_color: Color,
) {
    buffer.fill(pack_rgb(clear_color));
    let clip = ClipRect::full(width as f32, height as f32);

    for node in scene {
        draw_node(node, buffer, width, height, clip, CullMode::Layout);
    }
}

fn render_scene_update(
    previous_scene: &[RenderNode],
    scene: &[RenderNode],
    buffer: &mut [u32],
    width: usize,
    height: usize,
    clear_color: Color,
) {
    let mut dirty_regions = dirty_regions_between_scenes(previous_scene, scene);
    if dirty_regions.is_empty() {
        return;
    }

    coalesce_dirty_regions(&mut dirty_regions);
    if should_full_redraw(&dirty_regions, width, height) {
        render_to_buffer(scene, buffer, width, height, clear_color);
        return;
    }

    let full_clip = ClipRect::full(width as f32, height as f32);
    for dirty_region in dirty_regions {
        let Some(dirty_region) = dirty_region
            .intersect(full_clip)
            .and_then(|clip| snap_clip_to_pixel_grid(clip, width, height))
        else {
            continue;
        };
        clear_clip(buffer, width, height, dirty_region, clear_color);
        for node in scene {
            draw_node(node, buffer, width, height, dirty_region, CullMode::Subtree);
        }
    }
}

fn should_present_scene(
    previous_scene: Option<&[RenderNode]>,
    scene: &[RenderNode],
    resized: bool,
) -> bool {
    if resized {
        return true;
    }

    let Some(previous_scene) = previous_scene else {
        return true;
    };

    !scenes_match_visuals(previous_scene, scene)
}

fn should_present_frame(
    previous_scene: Option<&[RenderNode]>,
    scene: &[RenderNode],
    previous_indicator: Option<scrollbar::AutoScrollIndicator>,
    indicator: Option<scrollbar::AutoScrollIndicator>,
    resized: bool,
) -> bool {
    should_present_scene(previous_scene, scene, resized) || previous_indicator != indicator
}

fn redraw_auto_scroll_indicator_regions(
    previous_indicator: Option<scrollbar::AutoScrollIndicator>,
    indicator: Option<scrollbar::AutoScrollIndicator>,
    scene: &[RenderNode],
    buffer: &mut [u32],
    width: usize,
    height: usize,
    clear_color: Color,
) {
    let mut bounds = Vec::new();
    if let Some(previous_indicator) = previous_indicator {
        bounds.push(scrollbar::auto_scroll_indicator_bounds(previous_indicator));
    }
    if let Some(indicator) = indicator {
        let indicator_bounds = scrollbar::auto_scroll_indicator_bounds(indicator);
        if !bounds.iter().any(|existing| *existing == indicator_bounds) {
            bounds.push(indicator_bounds);
        }
    }

    let full_clip = ClipRect::full(width as f32, height as f32);
    for bounds in bounds {
        let Some(clip) = full_clip
            .intersect(layout_clip(bounds))
            .and_then(|clip| snap_clip_to_pixel_grid(clip, width, height))
        else {
            continue;
        };
        clear_clip(buffer, width, height, clip, clear_color);
        for node in scene {
            draw_node(node, buffer, width, height, clip, CullMode::Subtree);
        }
    }

    if let Some(indicator) = indicator {
        scrollbar::draw_auto_scroll_indicator(indicator, buffer, width, height);
    }
}

fn scenes_match_visuals(left: &[RenderNode], right: &[RenderNode]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right.iter())
            .all(|(left, right)| render_nodes_match_visuals(left, right))
}

fn render_nodes_match_visuals(left: &RenderNode, right: &RenderNode) -> bool {
    render_nodes_match_own_visuals(left, right)
        && scenes_match_visuals(&left.children, &right.children)
}

fn render_nodes_match_own_visuals(left: &RenderNode, right: &RenderNode) -> bool {
    left.kind == right.kind
        && left.layout == right.layout
        && left.style == right.style
        && left.content_inset == right.content_inset
        && left.scrollbars == right.scrollbars
}

fn should_suspend_updates(left_down: bool, left_super_down: bool, right_super_down: bool) -> bool {
    left_down && (left_super_down || right_super_down)
}

#[derive(Clone, Copy)]
enum CullMode {
    Layout,
    Subtree,
}

#[derive(Clone, Copy, Debug)]
struct ClipRect {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
}

impl ClipRect {
    fn full(width: f32, height: f32) -> Self {
        Self {
            x0: 0.0,
            y0: 0.0,
            x1: width,
            y1: height,
        }
    }

    fn unbounded() -> Self {
        Self {
            x0: f32::MIN,
            y0: f32::MIN,
            x1: f32::MAX,
            y1: f32::MAX,
        }
    }

    fn intersect(self, other: Self) -> Option<Self> {
        let clipped = Self {
            x0: self.x0.max(other.x0),
            y0: self.y0.max(other.y0),
            x1: self.x1.min(other.x1),
            y1: self.y1.min(other.y1),
        };

        (!clipped.is_empty()).then_some(clipped)
    }

    fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x0 && y >= self.y0 && x < self.x1 && y < self.y1
    }

    fn union(self, other: Self) -> Self {
        Self {
            x0: self.x0.min(other.x0),
            y0: self.y0.min(other.y0),
            x1: self.x1.max(other.x1),
            y1: self.y1.max(other.y1),
        }
    }

    fn overlaps_or_touches(self, other: Self) -> bool {
        self.x0 <= other.x1 && self.x1 >= other.x0 && self.y0 <= other.y1 && self.y1 >= other.y0
    }

    fn area(self) -> f32 {
        if self.is_empty() {
            0.0
        } else {
            (self.x1 - self.x0) * (self.y1 - self.y0)
        }
    }

    fn is_empty(self) -> bool {
        self.x0 >= self.x1 || self.y0 >= self.y1
    }
}

fn draw_node(
    node: &RenderNode,
    buffer: &mut [u32],
    width: usize,
    height: usize,
    clip: ClipRect,
    cull_mode: CullMode,
) {
    if clip.is_empty() || !node_intersects_clip(node, clip, cull_mode) {
        return;
    }

    for shadow in &node.style.shadows {
        draw_shadow(
            buffer,
            width,
            height,
            node.layout,
            node.style.corner_radius,
            *shadow,
            clip,
        );
    }

    if !matches!(node.kind, RenderKind::Text(_)) {
        for shadow in &node.style.filter_drop_shadows {
            draw_shadow_effect(
                buffer,
                width,
                height,
                node.layout,
                node.style.corner_radius,
                *shadow,
                node.style.foreground,
                clip,
            );
        }
    }

    draw_background_and_border(node, buffer, width, height, clip);

    if let RenderKind::Text(content) = &node.kind {
        let text_layout = scrollbar::text_layout(node);
        let text_clip = scrollbar::text_clip(node, clip);
        fonts::draw_text(
            buffer,
            width,
            height,
            text_layout,
            content,
            &node.style,
            text_clip,
        );
    }

    let child_clip = if node.style.overflow.clips_any_axis() {
        clip.intersect(layout_clip(node.layout))
    } else {
        Some(clip)
    };

    let Some(child_clip) = child_clip else {
        return;
    };

    for child in &node.children {
        draw_node(child, buffer, width, height, child_clip, cull_mode);
    }

    scrollbar::draw_scrollbars(node, buffer, width, height, clip);
}

fn node_intersects_clip(node: &RenderNode, clip: ClipRect, cull_mode: CullMode) -> bool {
    match cull_mode {
        CullMode::Layout => clip.intersect(layout_clip(node.layout)).is_some(),
        CullMode::Subtree => subtree_visual_bounds(node)
            .and_then(|bounds| clip.intersect(bounds))
            .is_some(),
    }
}

fn dispatch_click(scene: &[RenderNode], x: f32, y: f32) -> bool {
    let Some(handler) = hit_test_scene(scene, x, y) else {
        return false;
    };

    handler();
    true
}

fn hit_test_scene(scene: &[RenderNode], x: f32, y: f32) -> Option<EventHandler> {
    scene
        .iter()
        .rev()
        .find_map(|node| hit_test_node(node, x, y, ClipRect::unbounded()))
}

fn hit_test_element_path(scene: &[RenderNode], x: f32, y: f32) -> Option<ElementPath> {
    scene
        .iter()
        .enumerate()
        .rev()
        .find_map(|(root_index, node)| {
            hit_test_element_path_node(
                node,
                x,
                y,
                ClipRect::unbounded(),
                &ElementPath::root(root_index),
            )
        })
}

fn hit_test_node(node: &RenderNode, x: f32, y: f32, clip: ClipRect) -> Option<EventHandler> {
    if !clip.contains(x, y) || !layout_contains(node.layout, x, y) {
        return None;
    }

    let child_clip = if node.style.overflow.clips_any_axis() {
        clip.intersect(layout_clip(node.layout))?
    } else {
        clip
    };

    for child in node.children.iter().rev() {
        if let Some(handler) = hit_test_node(child, x, y, child_clip) {
            return Some(handler);
        }
    }

    node.on_click
}

fn hit_test_element_path_node(
    node: &RenderNode,
    x: f32,
    y: f32,
    clip: ClipRect,
    path: &ElementPath,
) -> Option<ElementPath> {
    if !clip.contains(x, y) || !layout_contains(node.layout, x, y) {
        return None;
    }

    let child_clip = if node.style.overflow.clips_any_axis() {
        clip.intersect(layout_clip(node.layout))?
    } else {
        clip
    };

    for (index, child) in node.children.iter().enumerate().rev() {
        let child_path = path.with_child(index);
        if let Some(hit) = hit_test_element_path_node(child, x, y, child_clip, &child_path) {
            return Some(hit);
        }
    }

    node.element_path.clone().or_else(|| Some(path.clone()))
}

fn settle_element_interaction<P>(
    scene_provider: &mut P,
    frame: FrameInfo,
    scene: &mut Vec<RenderNode>,
    scrollbar_controller: &mut scrollbar::ScrollbarController,
    mouse_position: Option<(f32, f32)>,
    interactive_left_down: bool,
    press_started: bool,
    interaction: &mut ElementInteractionState,
) where
    P: SceneProvider,
{
    let mut press_started = press_started;

    for _ in 0..4 {
        let hovered = mouse_position
            .and_then(|(mouse_x, mouse_y)| hit_test_element_path(scene, mouse_x, mouse_y));
        let next = next_element_interaction_state(
            interaction,
            hovered,
            interactive_left_down,
            press_started,
        );
        press_started = false;

        if next == *interaction {
            break;
        }

        *interaction = next.clone();
        if !scene_provider.set_element_interaction(next) {
            break;
        }

        scene_provider.update(frame);
        *scene = scene_provider.capture_scene();
        scrollbar_controller.apply_to_scene(scene);
        scrollbar_controller.handle_pointer(scene, mouse_position, interactive_left_down, false);
    }
}

fn next_element_interaction_state(
    previous: &ElementInteractionState,
    hovered: Option<ElementPath>,
    left_down: bool,
    press_started: bool,
) -> ElementInteractionState {
    let active = if press_started {
        hovered.clone()
    } else if left_down {
        previous.active.clone()
    } else {
        None
    };

    ElementInteractionState { hovered, active }
}

fn layout_contains(layout: LayoutBox, x: f32, y: f32) -> bool {
    x >= layout.x && y >= layout.y && x < layout.x + layout.width && y < layout.y + layout.height
}

fn draw_background_and_border(
    node: &RenderNode,
    buffer: &mut [u32],
    width: usize,
    height: usize,
    clip: ClipRect,
) {
    if !node.style.border.widths.is_zero() {
        let inner_layout = inset_layout(node.layout, node.style.border.widths);
        let inner_radius = inset_corner_radius(node.style.corner_radius, node.style.border.widths);

        draw_rounded_ring(
            buffer,
            width,
            height,
            node.layout,
            node.style.corner_radius,
            Some((inner_layout, inner_radius)),
            node.style.border.color,
            clip,
        );
    }

    let fill_layout = if node.style.border.widths.is_zero() {
        node.layout
    } else {
        inset_layout(node.layout, node.style.border.widths)
    };
    let fill_radius = if node.style.border.widths.is_zero() {
        node.style.corner_radius
    } else {
        inset_corner_radius(node.style.corner_radius, node.style.border.widths)
    };

    if let Some(background) = node.style.background {
        draw_rounded_rect(
            buffer,
            width,
            height,
            fill_layout,
            fill_radius,
            background,
            clip,
        );
    }

    for layer in node.style.background_layers.iter().rev() {
        draw_background_layer(buffer, width, height, fill_layout, fill_radius, layer, clip);
    }
}

fn draw_shadow(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    layout: LayoutBox,
    radius: CornerRadius,
    shadow: cssimpler_core::BoxShadow,
    clip: ClipRect,
) {
    let base_layout = offset_layout(
        expand_layout(layout, shadow.spread),
        shadow.offset_x,
        shadow.offset_y,
    );
    let base_radius = expand_corner_radius(radius, shadow.spread);
    let blur_radius = shadow.blur_radius.max(0.0);

    if blur_radius <= 0.0 {
        draw_rounded_rect(
            buffer,
            width,
            height,
            base_layout,
            base_radius,
            shadow.color,
            clip,
        );
        return;
    }

    let blurred_bounds = expand_layout(base_layout, blur_radius);
    let Some((x0, y0, x1, y1)) = pixel_bounds(blurred_bounds, clip, width, height) else {
        return;
    };

    for y in y0..y1 {
        for x in x0..x1 {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let alpha = shadow_alpha(
                px,
                py,
                base_layout,
                base_radius,
                blur_radius,
                shadow.color.a,
            );
            if alpha == 0 {
                continue;
            }

            blend_pixel(buffer, width, height, x, y, shadow.color.with_alpha(alpha));
        }
    }
}

fn draw_shadow_effect(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    layout: LayoutBox,
    radius: CornerRadius,
    shadow: cssimpler_core::ShadowEffect,
    fallback_color: Color,
    clip: ClipRect,
) {
    draw_shadow(
        buffer,
        width,
        height,
        layout,
        radius,
        cssimpler_core::BoxShadow {
            color: shadow.color.unwrap_or(fallback_color),
            offset_x: shadow.offset_x,
            offset_y: shadow.offset_y,
            blur_radius: shadow.blur_radius,
            spread: shadow.spread,
        },
        clip,
    );
}

fn draw_rounded_rect(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    layout: LayoutBox,
    radius: CornerRadius,
    color: Color,
    clip: ClipRect,
) {
    let Some((x0, y0, x1, y1)) = pixel_bounds(layout, clip, width, height) else {
        return;
    };

    for y in y0..y1 {
        for x in x0..x1 {
            if point_in_rounded_rect(x as f32 + 0.5, y as f32 + 0.5, layout, radius) {
                blend_pixel(buffer, width, height, x, y, color);
            }
        }
    }
}

#[derive(Clone, Copy)]
struct ResolvedRadialShape {
    radius_x: f32,
    radius_y: f32,
}

fn draw_background_layer(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    layout: LayoutBox,
    radius: CornerRadius,
    layer: &BackgroundLayer,
    clip: ClipRect,
) {
    match layer {
        BackgroundLayer::LinearGradient(gradient) => {
            draw_linear_gradient(buffer, width, height, layout, radius, gradient, clip);
        }
        BackgroundLayer::RadialGradient(gradient) => {
            draw_radial_gradient(buffer, width, height, layout, radius, gradient, clip);
        }
        BackgroundLayer::ConicGradient(gradient) => {
            draw_conic_gradient(buffer, width, height, layout, radius, gradient, clip);
        }
    }
}

fn draw_linear_gradient(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    layout: LayoutBox,
    radius: CornerRadius,
    gradient: &LinearGradient,
    clip: ClipRect,
) {
    let Some((x0, y0, x1, y1)) = pixel_bounds(layout, clip, width, height) else {
        return;
    };

    let Some(first_stop) = gradient.stops.first() else {
        return;
    };
    let direction = gradient_direction_vector(gradient.direction, layout);
    let center_x = layout.x + layout.width * 0.5;
    let center_y = layout.y + layout.height * 0.5;
    let (min_projection, max_projection) =
        gradient_projection_bounds(layout, center_x, center_y, direction);
    let projection_span = max_projection - min_projection;

    if projection_span.abs() <= f32::EPSILON {
        draw_rounded_rect(
            buffer,
            width,
            height,
            layout,
            radius,
            first_stop.color,
            clip,
        );
        return;
    }

    let stops = resolve_length_stops(&gradient.stops, projection_span, min_projection);
    for y in y0..y1 {
        for x in x0..x1 {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            if !point_in_rounded_rect(px, py, layout, radius) {
                continue;
            }

            let projection = ((px - center_x) * direction.0) + ((py - center_y) * direction.1);
            let color = Color::from_linear_rgba(sample_gradient(
                &stops,
                projection,
                gradient.repeating,
                gradient.interpolation,
            ));
            blend_pixel(buffer, width, height, x, y, color);
        }
    }
}

fn draw_radial_gradient(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    layout: LayoutBox,
    radius: CornerRadius,
    gradient: &RadialGradient,
    clip: ClipRect,
) {
    let Some((x0, y0, x1, y1)) = pixel_bounds(layout, clip, width, height) else {
        return;
    };

    let Some(first_stop) = gradient.stops.first() else {
        return;
    };

    let (center_x, center_y) = resolve_gradient_point(gradient.center, layout);
    let resolved_shape = resolve_radial_shape(gradient.shape, layout, center_x, center_y);
    if resolved_shape.radius_x <= f32::EPSILON || resolved_shape.radius_y <= f32::EPSILON {
        draw_rounded_rect(
            buffer,
            width,
            height,
            layout,
            radius,
            first_stop.color,
            clip,
        );
        return;
    }

    for y in y0..y1 {
        for x in x0..x1 {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            if !point_in_rounded_rect(px, py, layout, radius) {
                continue;
            }

            let dx = px - center_x;
            let dy = py - center_y;
            let distance = (dx * dx + dy * dy).sqrt();
            let ray_length = radial_ray_length(dx, dy, resolved_shape);
            let stops = resolve_length_stops(&gradient.stops, ray_length, 0.0);
            let color = Color::from_linear_rgba(sample_gradient(
                &stops,
                distance,
                gradient.repeating,
                gradient.interpolation,
            ));
            blend_pixel(buffer, width, height, x, y, color);
        }
    }
}

fn draw_conic_gradient(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    layout: LayoutBox,
    radius: CornerRadius,
    gradient: &ConicGradient,
    clip: ClipRect,
) {
    let Some((x0, y0, x1, y1)) = pixel_bounds(layout, clip, width, height) else {
        return;
    };

    let Some(_first_stop) = gradient.stops.first() else {
        return;
    };

    let stops = resolve_angle_stops(&gradient.stops);
    let (center_x, center_y) = resolve_gradient_point(gradient.center, layout);

    for y in y0..y1 {
        for x in x0..x1 {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            if !point_in_rounded_rect(px, py, layout, radius) {
                continue;
            }

            let dx = px - center_x;
            let dy = py - center_y;
            let angle = if dx.abs() <= f32::EPSILON && dy.abs() <= f32::EPSILON {
                0.0
            } else {
                dx.atan2(-dy).to_degrees().rem_euclid(360.0)
            };
            let position = (angle - gradient.angle).rem_euclid(360.0);
            let color = Color::from_linear_rgba(sample_gradient(
                &stops,
                position,
                gradient.repeating,
                gradient.interpolation,
            ));
            blend_pixel(buffer, width, height, x, y, color);
        }
    }
}

fn gradient_direction_vector(direction: GradientDirection, layout: LayoutBox) -> (f32, f32) {
    match direction {
        GradientDirection::Angle(degrees) => {
            let radians = degrees.to_radians();
            (radians.sin(), -radians.cos())
        }
        GradientDirection::Horizontal(GradientHorizontal::Left) => (-1.0, 0.0),
        GradientDirection::Horizontal(GradientHorizontal::Right) => (1.0, 0.0),
        GradientDirection::Vertical(GradientVertical::Top) => (0.0, -1.0),
        GradientDirection::Vertical(GradientVertical::Bottom) => (0.0, 1.0),
        GradientDirection::Corner {
            horizontal,
            vertical,
        } => {
            let dx = match horizontal {
                GradientHorizontal::Left => -layout.width.max(1.0),
                GradientHorizontal::Right => layout.width.max(1.0),
            };
            let dy = match vertical {
                GradientVertical::Top => -layout.height.max(1.0),
                GradientVertical::Bottom => layout.height.max(1.0),
            };
            normalize_vector(dx, dy)
        }
    }
}

fn normalize_vector(x: f32, y: f32) -> (f32, f32) {
    let length = (x * x + y * y).sqrt();
    if length <= f32::EPSILON {
        (0.0, 1.0)
    } else {
        (x / length, y / length)
    }
}

fn gradient_projection_bounds(
    layout: LayoutBox,
    center_x: f32,
    center_y: f32,
    direction: (f32, f32),
) -> (f32, f32) {
    let corners = [
        (layout.x, layout.y),
        (layout.x + layout.width, layout.y),
        (layout.x, layout.y + layout.height),
        (layout.x + layout.width, layout.y + layout.height),
    ];
    let mut min_projection = f32::INFINITY;
    let mut max_projection = f32::NEG_INFINITY;

    for (x, y) in corners {
        let projection = ((x - center_x) * direction.0) + ((y - center_y) * direction.1);
        min_projection = min_projection.min(projection);
        max_projection = max_projection.max(projection);
    }

    (min_projection, max_projection)
}

fn resolve_gradient_point(point: GradientPoint, layout: LayoutBox) -> (f32, f32) {
    (
        layout.x + point.x.resolve(layout.width),
        layout.y + point.y.resolve(layout.height),
    )
}

fn resolve_radial_shape(
    shape: RadialShape,
    layout: LayoutBox,
    center_x: f32,
    center_y: f32,
) -> ResolvedRadialShape {
    match shape {
        RadialShape::Circle(radius) => {
            let radius = match radius {
                CircleRadius::Explicit(radius) => radius.max(0.0),
                CircleRadius::Extent(extent) => {
                    resolve_circle_extent(extent, layout, center_x, center_y)
                }
            };
            ResolvedRadialShape {
                radius_x: radius,
                radius_y: radius,
            }
        }
        RadialShape::Ellipse(radius) => match radius {
            EllipseRadius::Explicit { x, y } => ResolvedRadialShape {
                radius_x: x.resolve(layout.width).max(0.0),
                radius_y: y.resolve(layout.height).max(0.0),
            },
            EllipseRadius::Extent(extent) => {
                resolve_ellipse_extent(extent, layout, center_x, center_y)
            }
        },
    }
}

fn resolve_circle_extent(
    extent: ShapeExtent,
    layout: LayoutBox,
    center_x: f32,
    center_y: f32,
) -> f32 {
    let (left, right, top, bottom) = side_distances(layout, center_x, center_y);
    let corners = corner_offsets(left, right, top, bottom);

    match extent {
        ShapeExtent::ClosestSide => left.min(right).min(top).min(bottom),
        ShapeExtent::FarthestSide => left.max(right).max(top).max(bottom),
        ShapeExtent::ClosestCorner => corners
            .iter()
            .map(|(dx, dy)| (dx * dx + dy * dy).sqrt())
            .fold(f32::INFINITY, f32::min),
        ShapeExtent::FarthestCorner => corners
            .iter()
            .map(|(dx, dy)| (dx * dx + dy * dy).sqrt())
            .fold(0.0, f32::max),
    }
}

fn resolve_ellipse_extent(
    extent: ShapeExtent,
    layout: LayoutBox,
    center_x: f32,
    center_y: f32,
) -> ResolvedRadialShape {
    let (left, right, top, bottom) = side_distances(layout, center_x, center_y);
    let corners = corner_offsets(left, right, top, bottom);

    match extent {
        ShapeExtent::ClosestSide => ResolvedRadialShape {
            radius_x: left.min(right),
            radius_y: top.min(bottom),
        },
        ShapeExtent::FarthestSide => ResolvedRadialShape {
            radius_x: left.max(right),
            radius_y: top.max(bottom),
        },
        ShapeExtent::ClosestCorner => {
            scale_ellipse_to_corner(left.min(right), top.min(bottom), &corners, false)
        }
        ShapeExtent::FarthestCorner => {
            scale_ellipse_to_corner(left.max(right), top.max(bottom), &corners, true)
        }
    }
}

fn scale_ellipse_to_corner(
    base_radius_x: f32,
    base_radius_y: f32,
    corners: &[(f32, f32); 4],
    farthest: bool,
) -> ResolvedRadialShape {
    if base_radius_x <= f32::EPSILON || base_radius_y <= f32::EPSILON {
        return ResolvedRadialShape {
            radius_x: 0.0,
            radius_y: 0.0,
        };
    }

    let mut scale = if farthest { 0.0 } else { f32::INFINITY };
    for &(dx, dy) in corners {
        let factor = ((dx / base_radius_x).powi(2) + (dy / base_radius_y).powi(2)).sqrt();
        if farthest {
            scale = scale.max(factor);
        } else {
            scale = scale.min(factor);
        }
    }

    ResolvedRadialShape {
        radius_x: base_radius_x * scale,
        radius_y: base_radius_y * scale,
    }
}

fn side_distances(layout: LayoutBox, center_x: f32, center_y: f32) -> (f32, f32, f32, f32) {
    (
        (center_x - layout.x).abs(),
        (layout.x + layout.width - center_x).abs(),
        (center_y - layout.y).abs(),
        (layout.y + layout.height - center_y).abs(),
    )
}

fn corner_offsets(left: f32, right: f32, top: f32, bottom: f32) -> [(f32, f32); 4] {
    [(left, top), (right, top), (left, bottom), (right, bottom)]
}

fn radial_ray_length(dx: f32, dy: f32, shape: ResolvedRadialShape) -> f32 {
    if dx.abs() <= f32::EPSILON && dy.abs() <= f32::EPSILON {
        return 0.0;
    }

    let radius_x = shape.radius_x.max(f32::EPSILON);
    let radius_y = shape.radius_y.max(f32::EPSILON);
    let denominator =
        ((dx * dx) / (radius_x * radius_x) + (dy * dy) / (radius_y * radius_y)).sqrt();
    if denominator <= f32::EPSILON {
        0.0
    } else {
        (dx * dx + dy * dy).sqrt() / denominator
    }
}

fn draw_rounded_ring(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    outer_layout: LayoutBox,
    outer_radius: CornerRadius,
    inner: Option<(LayoutBox, CornerRadius)>,
    color: Color,
    clip: ClipRect,
) {
    let Some((x0, y0, x1, y1)) = pixel_bounds(outer_layout, clip, width, height) else {
        return;
    };

    for y in y0..y1 {
        for x in x0..x1 {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            if !point_in_rounded_rect(px, py, outer_layout, outer_radius) {
                continue;
            }

            if let Some((inner_layout, inner_radius)) = inner
                && point_in_rounded_rect(px, py, inner_layout, inner_radius)
            {
                continue;
            }

            blend_pixel(buffer, width, height, x, y, color);
        }
    }
}

fn dirty_regions_between_scenes(
    previous_scene: &[RenderNode],
    scene: &[RenderNode],
) -> Vec<ClipRect> {
    let mut dirty_regions = Vec::new();
    collect_scene_dirty_regions(previous_scene, scene, &mut dirty_regions);
    dirty_regions
}

fn collect_scene_dirty_regions(
    previous_scene: &[RenderNode],
    scene: &[RenderNode],
    dirty_regions: &mut Vec<ClipRect>,
) {
    let count = previous_scene.len().max(scene.len());

    for index in 0..count {
        match (previous_scene.get(index), scene.get(index)) {
            (Some(previous), Some(current)) => {
                collect_node_dirty_regions(previous, current, dirty_regions);
            }
            (Some(previous), None) => push_subtree_dirty_region(previous, dirty_regions),
            (None, Some(current)) => push_subtree_dirty_region(current, dirty_regions),
            (None, None) => {}
        }
    }
}

fn collect_node_dirty_regions(
    previous: &RenderNode,
    current: &RenderNode,
    dirty_regions: &mut Vec<ClipRect>,
) {
    if render_nodes_match_visuals(previous, current) {
        return;
    }

    if !render_nodes_match_own_visuals(previous, current) {
        push_dirty_region(
            union_optional_bounds(
                subtree_visual_bounds(previous),
                subtree_visual_bounds(current),
            ),
            dirty_regions,
        );
        return;
    }

    collect_scene_dirty_regions(&previous.children, &current.children, dirty_regions);
}

fn push_subtree_dirty_region(node: &RenderNode, dirty_regions: &mut Vec<ClipRect>) {
    push_dirty_region(subtree_visual_bounds(node), dirty_regions);
}

fn push_dirty_region(region: Option<ClipRect>, dirty_regions: &mut Vec<ClipRect>) {
    if let Some(region) = region
        && !region.is_empty()
    {
        dirty_regions.push(region);
    }
}

fn coalesce_dirty_regions(dirty_regions: &mut Vec<ClipRect>) {
    let mut index = 0;
    while index < dirty_regions.len() {
        let mut merged = false;
        let mut other_index = index + 1;
        while other_index < dirty_regions.len() {
            if dirty_regions[index].overlaps_or_touches(dirty_regions[other_index]) {
                dirty_regions[index] = dirty_regions[index].union(dirty_regions[other_index]);
                dirty_regions.swap_remove(other_index);
                merged = true;
            } else {
                other_index += 1;
            }
        }

        if !merged {
            index += 1;
        }
    }
}

fn should_full_redraw(dirty_regions: &[ClipRect], width: usize, height: usize) -> bool {
    if dirty_regions.len() > MAX_INCREMENTAL_DIRTY_REGIONS {
        return true;
    }

    let full_clip = ClipRect::full(width as f32, height as f32);
    let dirty_area: f32 = dirty_regions
        .iter()
        .filter_map(|region| region.intersect(full_clip))
        .map(ClipRect::area)
        .sum();

    dirty_area > full_clip.area() * MAX_INCREMENTAL_DIRTY_AREA_RATIO
}

fn subtree_visual_bounds(node: &RenderNode) -> Option<ClipRect> {
    let mut bounds = node_visual_bounds(node);
    let parent_clip = non_empty_layout_clip(node.layout);

    for child in &node.children {
        let Some(mut child_bounds) = subtree_visual_bounds(child) else {
            continue;
        };

        if node.style.overflow.clips_any_axis() {
            let Some(clip) = parent_clip else {
                continue;
            };
            let Some(clipped_bounds) = child_bounds.intersect(clip) else {
                continue;
            };
            child_bounds = clipped_bounds;
        }

        bounds = union_optional_bounds(bounds, Some(child_bounds));
    }

    bounds
}

fn node_visual_bounds(node: &RenderNode) -> Option<ClipRect> {
    let mut bounds = non_empty_layout_clip(node.layout);

    if matches!(node.kind, RenderKind::Text(_)) && node.style.text_stroke.width > 0.0 {
        bounds = union_optional_bounds(
            bounds,
            text_stroke_bounds(node.layout, node.style.text_stroke),
        );
    }

    for shadow in &node.style.shadows {
        bounds = union_optional_bounds(bounds, shadow_bounds(node.layout, *shadow));
    }

    match &node.kind {
        RenderKind::Text(_) => {
            for shadow in &node.style.text_shadows {
                bounds = union_optional_bounds(bounds, shadow_effect_bounds(node.layout, *shadow));
            }
            for shadow in &node.style.filter_drop_shadows {
                bounds = union_optional_bounds(bounds, shadow_effect_bounds(node.layout, *shadow));
            }
        }
        RenderKind::Container => {
            for shadow in &node.style.filter_drop_shadows {
                bounds = union_optional_bounds(bounds, shadow_effect_bounds(node.layout, *shadow));
            }
        }
    }

    bounds
}

fn shadow_bounds(layout: LayoutBox, shadow: cssimpler_core::BoxShadow) -> Option<ClipRect> {
    let shadow_layout = offset_layout(
        expand_layout(layout, shadow.spread),
        shadow.offset_x,
        shadow.offset_y,
    );
    non_empty_layout_clip(expand_layout(shadow_layout, shadow.blur_radius.max(0.0)))
}

fn shadow_effect_bounds(
    layout: LayoutBox,
    shadow: cssimpler_core::ShadowEffect,
) -> Option<ClipRect> {
    shadow_bounds(
        layout,
        cssimpler_core::BoxShadow {
            color: shadow.color.unwrap_or(Color::BLACK),
            offset_x: shadow.offset_x,
            offset_y: shadow.offset_y,
            blur_radius: shadow.blur_radius,
            spread: shadow.spread,
        },
    )
}

fn text_stroke_bounds(
    layout: LayoutBox,
    stroke: cssimpler_core::TextStrokeStyle,
) -> Option<ClipRect> {
    if stroke.width <= 0.0 {
        return None;
    }

    non_empty_layout_clip(expand_layout(layout, stroke.width.ceil().max(0.0)))
}

fn non_empty_layout_clip(layout: LayoutBox) -> Option<ClipRect> {
    let clip = layout_clip(layout);
    (!clip.is_empty()).then_some(clip)
}

fn union_optional_bounds(left: Option<ClipRect>, right: Option<ClipRect>) -> Option<ClipRect> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.union(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

fn clear_clip(buffer: &mut [u32], width: usize, height: usize, clip: ClipRect, clear_color: Color) {
    let Some((x0, y0, x1, y1)) = clip_pixel_bounds(clip, width, height) else {
        return;
    };
    let clear = pack_rgb(clear_color);

    for y in y0..y1 {
        let start = y as usize * width + x0 as usize;
        let end = y as usize * width + x1 as usize;
        buffer[start..end].fill(clear);
    }
}

fn resize_buffer(
    buffer: &mut Vec<u32>,
    width: &mut usize,
    height: &mut usize,
    next_width: usize,
    next_height: usize,
    clear_color: Color,
) {
    let next_width = next_width.max(1);
    let next_height = next_height.max(1);

    if *width == next_width && *height == next_height {
        return;
    }

    *width = next_width;
    *height = next_height;
    buffer.resize(next_width * next_height, pack_rgb(clear_color));
}

fn layout_clip(layout: LayoutBox) -> ClipRect {
    ClipRect {
        x0: layout.x,
        y0: layout.y,
        x1: layout.x + layout.width,
        y1: layout.y + layout.height,
    }
}

fn pixel_bounds(
    layout: LayoutBox,
    clip: ClipRect,
    width: usize,
    height: usize,
) -> Option<(i32, i32, i32, i32)> {
    let clip = clip.intersect(ClipRect::full(width as f32, height as f32))?;
    let x0 = layout.x.max(clip.x0).floor().max(0.0) as i32;
    let y0 = layout.y.max(clip.y0).floor().max(0.0) as i32;
    let x1 = (layout.x + layout.width)
        .min(clip.x1)
        .ceil()
        .min(width as f32) as i32;
    let y1 = (layout.y + layout.height)
        .min(clip.y1)
        .ceil()
        .min(height as f32) as i32;
    (x0 < x1 && y0 < y1).then_some((x0, y0, x1, y1))
}

fn clip_pixel_bounds(clip: ClipRect, width: usize, height: usize) -> Option<(i32, i32, i32, i32)> {
    let clip = clip.intersect(ClipRect::full(width as f32, height as f32))?;
    let x0 = clip.x0.floor().max(0.0) as i32;
    let y0 = clip.y0.floor().max(0.0) as i32;
    let x1 = clip.x1.ceil().min(width as f32) as i32;
    let y1 = clip.y1.ceil().min(height as f32) as i32;
    (x0 < x1 && y0 < y1).then_some((x0, y0, x1, y1))
}

fn snap_clip_to_pixel_grid(clip: ClipRect, width: usize, height: usize) -> Option<ClipRect> {
    let (x0, y0, x1, y1) = clip_pixel_bounds(clip, width, height)?;
    Some(ClipRect {
        x0: x0 as f32,
        y0: y0 as f32,
        x1: x1 as f32,
        y1: y1 as f32,
    })
}

fn point_in_rounded_rect(x: f32, y: f32, layout: LayoutBox, radius: CornerRadius) -> bool {
    if !layout_contains(layout, x, y) {
        return false;
    }

    let radius = clamp_corner_radius(radius, layout.width, layout.height);
    if radius.top_left == 0.0
        && radius.top_right == 0.0
        && radius.bottom_right == 0.0
        && radius.bottom_left == 0.0
    {
        return true;
    }

    if x < layout.x + radius.top_left && y < layout.y + radius.top_left {
        return point_in_corner(
            x,
            y,
            layout.x + radius.top_left,
            layout.y + radius.top_left,
            radius.top_left,
        );
    }

    if x > layout.x + layout.width - radius.top_right && y < layout.y + radius.top_right {
        return point_in_corner(
            x,
            y,
            layout.x + layout.width - radius.top_right,
            layout.y + radius.top_right,
            radius.top_right,
        );
    }

    if x > layout.x + layout.width - radius.bottom_right
        && y > layout.y + layout.height - radius.bottom_right
    {
        return point_in_corner(
            x,
            y,
            layout.x + layout.width - radius.bottom_right,
            layout.y + layout.height - radius.bottom_right,
            radius.bottom_right,
        );
    }

    if x < layout.x + radius.bottom_left && y > layout.y + layout.height - radius.bottom_left {
        return point_in_corner(
            x,
            y,
            layout.x + radius.bottom_left,
            layout.y + layout.height - radius.bottom_left,
            radius.bottom_left,
        );
    }

    true
}

fn point_in_corner(x: f32, y: f32, center_x: f32, center_y: f32, radius: f32) -> bool {
    if radius <= 0.0 {
        return true;
    }

    let dx = x - center_x;
    let dy = y - center_y;
    (dx * dx) + (dy * dy) <= radius * radius
}

fn shadow_alpha(
    x: f32,
    y: f32,
    layout: LayoutBox,
    radius: CornerRadius,
    blur_radius: f32,
    max_alpha: u8,
) -> u8 {
    if point_in_rounded_rect(x, y, layout, radius) {
        return max_alpha;
    }

    let distance = distance_to_rounded_rect(x, y, layout, radius);
    if distance >= blur_radius {
        return 0;
    }

    let falloff = 1.0 - (distance / blur_radius);
    ((max_alpha as f32) * falloff * falloff).round() as u8
}

fn distance_to_rounded_rect(x: f32, y: f32, layout: LayoutBox, radius: CornerRadius) -> f32 {
    let radius = clamp_corner_radius(radius, layout.width, layout.height);
    let left = layout.x;
    let top = layout.y;
    let right = layout.x + layout.width;
    let bottom = layout.y + layout.height;

    if x < left + radius.top_left && y < top + radius.top_left {
        return distance_to_corner(
            x,
            y,
            left + radius.top_left,
            top + radius.top_left,
            radius.top_left,
        );
    }

    if x > right - radius.top_right && y < top + radius.top_right {
        return distance_to_corner(
            x,
            y,
            right - radius.top_right,
            top + radius.top_right,
            radius.top_right,
        );
    }

    if x > right - radius.bottom_right && y > bottom - radius.bottom_right {
        return distance_to_corner(
            x,
            y,
            right - radius.bottom_right,
            bottom - radius.bottom_right,
            radius.bottom_right,
        );
    }

    if x < left + radius.bottom_left && y > bottom - radius.bottom_left {
        return distance_to_corner(
            x,
            y,
            left + radius.bottom_left,
            bottom - radius.bottom_left,
            radius.bottom_left,
        );
    }

    let dx = if x < left {
        left - x
    } else if x > right {
        x - right
    } else {
        0.0
    };
    let dy = if y < top {
        top - y
    } else if y > bottom {
        y - bottom
    } else {
        0.0
    };

    if dx > 0.0 || dy > 0.0 {
        (dx * dx + dy * dy).sqrt()
    } else {
        0.0
    }
}

fn distance_to_corner(x: f32, y: f32, center_x: f32, center_y: f32, radius: f32) -> f32 {
    if radius <= 0.0 {
        let dx = x - center_x;
        let dy = y - center_y;
        return (dx * dx + dy * dy).sqrt();
    }

    let dx = x - center_x;
    let dy = y - center_y;
    ((dx * dx + dy * dy).sqrt() - radius).max(0.0)
}

fn clamp_corner_radius(radius: CornerRadius, width: f32, height: f32) -> CornerRadius {
    let max_radius = 0.5 * width.min(height).max(0.0);
    CornerRadius {
        top_left: radius.top_left.min(max_radius).max(0.0),
        top_right: radius.top_right.min(max_radius).max(0.0),
        bottom_right: radius.bottom_right.min(max_radius).max(0.0),
        bottom_left: radius.bottom_left.min(max_radius).max(0.0),
    }
}

fn inset_layout(layout: LayoutBox, insets: Insets) -> LayoutBox {
    let width = (layout.width - insets.left - insets.right).max(0.0);
    let height = (layout.height - insets.top - insets.bottom).max(0.0);
    LayoutBox::new(layout.x + insets.left, layout.y + insets.top, width, height)
}

fn inset_corner_radius(radius: CornerRadius, insets: Insets) -> CornerRadius {
    CornerRadius {
        top_left: (radius.top_left - insets.top.max(insets.left)).max(0.0),
        top_right: (radius.top_right - insets.top.max(insets.right)).max(0.0),
        bottom_right: (radius.bottom_right - insets.bottom.max(insets.right)).max(0.0),
        bottom_left: (radius.bottom_left - insets.bottom.max(insets.left)).max(0.0),
    }
}

fn expand_layout(layout: LayoutBox, amount: f32) -> LayoutBox {
    let width = (layout.width + amount * 2.0).max(0.0);
    let height = (layout.height + amount * 2.0).max(0.0);
    LayoutBox::new(layout.x - amount, layout.y - amount, width, height)
}

fn offset_layout(layout: LayoutBox, x: f32, y: f32) -> LayoutBox {
    LayoutBox::new(layout.x + x, layout.y + y, layout.width, layout.height)
}

fn expand_corner_radius(radius: CornerRadius, amount: f32) -> CornerRadius {
    CornerRadius {
        top_left: (radius.top_left + amount).max(0.0),
        top_right: (radius.top_right + amount).max(0.0),
        bottom_right: (radius.bottom_right + amount).max(0.0),
        bottom_left: (radius.bottom_left + amount).max(0.0),
    }
}

fn blend_pixel(buffer: &mut [u32], width: usize, height: usize, x: i32, y: i32, color: Color) {
    if color.a == 0 {
        return;
    }

    if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
        return;
    }

    let index = y as usize * width + x as usize;
    if color.a == 255 {
        buffer[index] = pack_rgb(color);
        return;
    }

    let source = color.to_linear_rgba();
    let destination = unpack_rgb(buffer[index]).to_linear_rgba();
    let alpha = source.a;
    let inverse_alpha = 1.0 - alpha;
    let blended = Color::from_linear_rgba(LinearRgba {
        r: source.r * alpha + destination.r * inverse_alpha,
        g: source.g * alpha + destination.g * inverse_alpha,
        b: source.b * alpha + destination.b * inverse_alpha,
        a: 1.0,
    });

    buffer[index] = pack_rgb(blended);
}

fn pack_rgb(color: Color) -> u32 {
    ((color.r as u32) << 16) | ((color.g as u32) << 8) | color.b as u32
}

fn unpack_rgb(pixel: u32) -> Color {
    Color::rgb(
        ((pixel >> 16) & 0xFF) as u8,
        ((pixel >> 8) & 0xFF) as u8,
        (pixel & 0xFF) as u8,
    )
}

fn frame_time_to_fps(frame_time: Duration) -> usize {
    if frame_time.is_zero() {
        return 0;
    }

    (1.0 / frame_time.as_secs_f64()).round().max(1.0) as usize
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use cssimpler_core::fonts::{FontFamily, TextStyle, TextTransform, register_font_file};
    use cssimpler_core::{
        AnglePercentageValue, BackgroundLayer, BoxShadow, CircleRadius, Color, ConicGradient,
        CornerRadius, ElementPath, GradientDirection, GradientHorizontal, GradientInterpolation,
        GradientPoint, GradientStop, LayoutBox, LengthPercentageValue, LinearGradient, Overflow,
        RadialGradient, RadialShape, RenderNode, ShadowEffect, TextStrokeStyle, VisualStyle,
    };

    use crate::{
        ViewportSize, WindowConfig, blend_pixel, dispatch_click, drawable_viewport_size,
        hit_test_element_path, pack_rgb, render_scene_update, render_to_buffer, resize_buffer,
        scenes_match_visuals, should_present_frame, should_present_scene, should_suspend_updates,
        window_options,
    };

    static CLICK_COUNT: AtomicUsize = AtomicUsize::new(0);
    static CLICK_TARGET: AtomicUsize = AtomicUsize::new(0);

    fn increment_click_count() {
        CLICK_COUNT.fetch_add(1, Ordering::SeqCst);
    }

    fn mark_parent_clicked() {
        CLICK_TARGET.store(1, Ordering::SeqCst);
    }

    fn mark_child_clicked() {
        CLICK_TARGET.store(2, Ordering::SeqCst);
    }

    fn alternate_click_handler() {
        CLICK_COUNT.fetch_add(10, Ordering::SeqCst);
    }

    fn bundled_font_family() -> String {
        let asset_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples/assets/powerline-demo.ttf");
        let families = register_font_file(&asset_path)
            .expect("bundled powerline demo font should register during renderer tests");
        families
            .into_iter()
            .next()
            .expect("bundled powerline font should expose at least one family name")
    }

    fn text_scene_with_content(style: TextStyle, content: &str) -> Vec<RenderNode> {
        vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 320.0, 120.0))
                .with_style(VisualStyle {
                    background: Some(Color::rgb(245, 247, 250)),
                    ..VisualStyle::default()
                })
                .with_child(
                    RenderNode::text(LayoutBox::new(20.0, 28.0, 280.0, 56.0), content).with_style(
                        VisualStyle {
                            foreground: Color::rgb(17, 37, 61),
                            text: style,
                            ..VisualStyle::default()
                        },
                    ),
                ),
        ]
    }

    fn text_scene(style: TextStyle) -> Vec<RenderNode> {
        text_scene_with_content(style, "WWW iii 0123456789")
    }

    #[test]
    fn offscreen_rendering_marks_the_expected_pixels() {
        let scene = vec![
            RenderNode::container(LayoutBox::new(2.0, 3.0, 6.0, 5.0)).with_style(VisualStyle {
                background: Some(Color::rgb(40, 120, 220)),
                ..VisualStyle::default()
            }),
        ];
        let mut buffer = vec![0_u32; 20 * 20];

        render_to_buffer(&scene, &mut buffer, 20, 20, Color::WHITE);

        assert!(buffer.contains(&pack_rgb(Color::rgb(40, 120, 220))));
    }

    #[test]
    fn rounded_background_respects_corner_radius() {
        let scene = vec![
            RenderNode::container(LayoutBox::new(2.0, 2.0, 8.0, 8.0)).with_style(VisualStyle {
                background: Some(Color::rgb(40, 120, 220)),
                corner_radius: CornerRadius::all(4.0),
                ..VisualStyle::default()
            }),
        ];
        let mut buffer = vec![0_u32; 16 * 16];

        render_to_buffer(&scene, &mut buffer, 16, 16, Color::WHITE);

        assert_eq!(buffer[2 * 16 + 2], pack_rgb(Color::WHITE));
        assert_eq!(buffer[6 * 16 + 6], pack_rgb(Color::rgb(40, 120, 220)));
    }

    #[test]
    fn linear_gradients_interpolate_in_oklab_by_default() {
        let scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 3.0, 1.0)).with_style(VisualStyle {
                background_layers: vec![BackgroundLayer::LinearGradient(LinearGradient {
                    direction: GradientDirection::Horizontal(GradientHorizontal::Right),
                    interpolation: GradientInterpolation::Oklab,
                    repeating: false,
                    stops: vec![
                        GradientStop {
                            color: Color::BLACK,
                            position: LengthPercentageValue::from_fraction(0.0),
                        },
                        GradientStop {
                            color: Color::WHITE,
                            position: LengthPercentageValue::from_fraction(1.0),
                        },
                    ],
                })],
                ..VisualStyle::default()
            }),
        ];
        let mut buffer = vec![0_u32; 3];

        render_to_buffer(&scene, &mut buffer, 3, 1, Color::BLACK);

        assert_eq!(buffer[1], pack_rgb(Color::rgb(99, 99, 99)));
    }

    #[test]
    fn linear_gradients_can_still_interpolate_in_linear_color_space() {
        let scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 3.0, 1.0)).with_style(VisualStyle {
                background_layers: vec![BackgroundLayer::LinearGradient(LinearGradient {
                    direction: GradientDirection::Horizontal(GradientHorizontal::Right),
                    interpolation: GradientInterpolation::LinearSrgb,
                    repeating: false,
                    stops: vec![
                        GradientStop {
                            color: Color::BLACK,
                            position: LengthPercentageValue::from_fraction(0.0),
                        },
                        GradientStop {
                            color: Color::WHITE,
                            position: LengthPercentageValue::from_fraction(1.0),
                        },
                    ],
                })],
                ..VisualStyle::default()
            }),
        ];
        let mut buffer = vec![0_u32; 3];

        render_to_buffer(&scene, &mut buffer, 3, 1, Color::BLACK);

        assert_eq!(buffer[1], pack_rgb(Color::rgb(188, 188, 188)));
    }

    #[test]
    fn layered_backgrounds_draw_the_first_layer_on_top() {
        let scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 3.0, 1.0)).with_style(VisualStyle {
                background_layers: vec![
                    BackgroundLayer::LinearGradient(LinearGradient {
                        direction: GradientDirection::Horizontal(GradientHorizontal::Right),
                        interpolation: GradientInterpolation::Oklab,
                        repeating: false,
                        stops: vec![
                            GradientStop {
                                color: Color::rgb(220, 38, 38),
                                position: LengthPercentageValue::from_fraction(0.0),
                            },
                            GradientStop {
                                color: Color::rgb(220, 38, 38),
                                position: LengthPercentageValue::from_fraction(1.0),
                            },
                        ],
                    }),
                    BackgroundLayer::LinearGradient(LinearGradient {
                        direction: GradientDirection::Horizontal(GradientHorizontal::Right),
                        interpolation: GradientInterpolation::Oklab,
                        repeating: false,
                        stops: vec![
                            GradientStop {
                                color: Color::rgb(37, 99, 235),
                                position: LengthPercentageValue::from_fraction(0.0),
                            },
                            GradientStop {
                                color: Color::rgb(37, 99, 235),
                                position: LengthPercentageValue::from_fraction(1.0),
                            },
                        ],
                    }),
                ],
                ..VisualStyle::default()
            }),
        ];
        let mut buffer = vec![0_u32; 3];

        render_to_buffer(&scene, &mut buffer, 3, 1, Color::BLACK);

        assert_eq!(buffer[1], pack_rgb(Color::rgb(220, 38, 38)));
    }

    #[test]
    fn linear_gradients_support_length_based_stop_positions() {
        let scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 5.0, 1.0)).with_style(VisualStyle {
                background_layers: vec![BackgroundLayer::LinearGradient(LinearGradient {
                    direction: GradientDirection::Horizontal(GradientHorizontal::Right),
                    interpolation: GradientInterpolation::Oklab,
                    repeating: false,
                    stops: vec![
                        GradientStop {
                            color: Color::BLACK,
                            position: LengthPercentageValue::from_px(0.0),
                        },
                        GradientStop {
                            color: Color::WHITE,
                            position: LengthPercentageValue::from_px(2.0),
                        },
                    ],
                })],
                ..VisualStyle::default()
            }),
        ];
        let mut buffer = vec![0_u32; 5];

        render_to_buffer(&scene, &mut buffer, 5, 1, Color::BLACK);

        assert_ne!(buffer[1], pack_rgb(Color::WHITE));
        assert_eq!(buffer[2], pack_rgb(Color::WHITE));
        assert_eq!(buffer[4], pack_rgb(Color::WHITE));
    }

    #[test]
    fn radial_gradients_render_from_the_center_outward() {
        let scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 5.0, 5.0)).with_style(VisualStyle {
                background_layers: vec![BackgroundLayer::RadialGradient(RadialGradient {
                    shape: RadialShape::Circle(CircleRadius::Explicit(2.0)),
                    center: GradientPoint::CENTER,
                    interpolation: GradientInterpolation::Oklab,
                    repeating: false,
                    stops: vec![
                        GradientStop {
                            color: Color::BLACK,
                            position: LengthPercentageValue::from_px(0.0),
                        },
                        GradientStop {
                            color: Color::WHITE,
                            position: LengthPercentageValue::from_px(2.0),
                        },
                    ],
                })],
                ..VisualStyle::default()
            }),
        ];
        let mut buffer = vec![0_u32; 25];

        render_to_buffer(&scene, &mut buffer, 5, 5, Color::BLACK);

        assert_eq!(buffer[2 * 5 + 2], pack_rgb(Color::BLACK));
        assert_eq!(buffer[2 * 5 + 4], pack_rgb(Color::WHITE));
    }

    #[test]
    fn conic_gradients_support_multiple_angle_sectors() {
        let scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 5.0, 5.0)).with_style(VisualStyle {
                background_layers: vec![BackgroundLayer::ConicGradient(ConicGradient {
                    angle: 0.0,
                    center: GradientPoint::CENTER,
                    interpolation: GradientInterpolation::Oklab,
                    repeating: false,
                    stops: vec![
                        GradientStop {
                            color: Color::rgb(255, 0, 0),
                            position: AnglePercentageValue::from_degrees(0.0),
                        },
                        GradientStop {
                            color: Color::rgb(255, 0, 0),
                            position: AnglePercentageValue::from_degrees(90.0),
                        },
                        GradientStop {
                            color: Color::rgb(0, 255, 0),
                            position: AnglePercentageValue::from_degrees(90.0),
                        },
                        GradientStop {
                            color: Color::rgb(0, 255, 0),
                            position: AnglePercentageValue::from_degrees(180.0),
                        },
                        GradientStop {
                            color: Color::rgb(0, 0, 255),
                            position: AnglePercentageValue::from_degrees(180.0),
                        },
                        GradientStop {
                            color: Color::rgb(0, 0, 255),
                            position: AnglePercentageValue::from_degrees(270.0),
                        },
                        GradientStop {
                            color: Color::rgb(255, 255, 255),
                            position: AnglePercentageValue::from_degrees(270.0),
                        },
                        GradientStop {
                            color: Color::rgb(255, 255, 255),
                            position: AnglePercentageValue::from_turns(1.0),
                        },
                    ],
                })],
                ..VisualStyle::default()
            }),
        ];
        let mut buffer = vec![0_u32; 25];

        render_to_buffer(&scene, &mut buffer, 5, 5, Color::BLACK);

        assert_eq!(buffer[4], pack_rgb(Color::rgb(255, 0, 0)));
        assert_eq!(buffer[4 * 5 + 4], pack_rgb(Color::rgb(0, 255, 0)));
        assert_eq!(buffer[4 * 5], pack_rgb(Color::rgb(0, 0, 255)));
        assert_eq!(buffer[0], pack_rgb(Color::rgb(255, 255, 255)));
    }

    #[test]
    fn gallery_style_conic_gradient_changes_when_the_interpolation_mode_changes() {
        fn render_conic(interpolation: GradientInterpolation) -> Vec<u32> {
            let scene = vec![
                RenderNode::container(LayoutBox::new(0.0, 0.0, 96.0, 96.0)).with_style(
                    VisualStyle {
                        background_layers: vec![BackgroundLayer::ConicGradient(ConicGradient {
                            angle: 220.0,
                            center: GradientPoint {
                                x: LengthPercentageValue::from_fraction(0.64),
                                y: LengthPercentageValue::from_fraction(0.56),
                            },
                            interpolation,
                            repeating: false,
                            stops: vec![
                                GradientStop {
                                    color: Color::rgb(56, 189, 248),
                                    position: AnglePercentageValue::from_degrees(0.0),
                                },
                                GradientStop {
                                    color: Color::rgb(45, 212, 191),
                                    position: AnglePercentageValue::from_degrees(100.0),
                                },
                                GradientStop {
                                    color: Color::rgb(245, 158, 11),
                                    position: AnglePercentageValue::from_degrees(220.0),
                                },
                                GradientStop {
                                    color: Color::rgb(244, 114, 182),
                                    position: AnglePercentageValue::from_degrees(300.0),
                                },
                                GradientStop {
                                    color: Color::rgb(56, 189, 248),
                                    position: AnglePercentageValue::from_degrees(360.0),
                                },
                            ],
                        })],
                        ..VisualStyle::default()
                    },
                ),
            ];
            let mut buffer = vec![0_u32; 96 * 96];
            render_to_buffer(&scene, &mut buffer, 96, 96, Color::BLACK);
            buffer
        }

        let oklab = render_conic(GradientInterpolation::Oklab);
        let linear = render_conic(GradientInterpolation::LinearSrgb);
        let different_pixels = oklab
            .iter()
            .zip(&linear)
            .filter(|(left, right)| left != right)
            .count();

        assert!(different_pixels > 0);
    }

    #[test]
    fn overflow_clip_hides_child_pixels_outside_parent_bounds() {
        let scene = vec![
            RenderNode::container(LayoutBox::new(4.0, 4.0, 6.0, 6.0))
                .with_style(VisualStyle {
                    overflow: Overflow::CLIP,
                    ..VisualStyle::default()
                })
                .with_child(
                    RenderNode::container(LayoutBox::new(6.0, 6.0, 10.0, 10.0)).with_style(
                        VisualStyle {
                            background: Some(Color::rgb(220, 38, 38)),
                            ..VisualStyle::default()
                        },
                    ),
                ),
        ];
        let mut buffer = vec![0_u32; 20 * 20];

        render_to_buffer(&scene, &mut buffer, 20, 20, Color::WHITE);

        assert_eq!(buffer[8 * 20 + 8], pack_rgb(Color::rgb(220, 38, 38)));
        assert_eq!(buffer[14 * 20 + 14], pack_rgb(Color::WHITE));
    }

    #[test]
    fn box_shadow_renders_behind_the_element() {
        let scene = vec![
            RenderNode::container(LayoutBox::new(6.0, 6.0, 6.0, 6.0)).with_style(VisualStyle {
                shadows: vec![BoxShadow {
                    color: Color::rgba(15, 23, 42, 160),
                    offset_x: 2.0,
                    offset_y: 2.0,
                    blur_radius: 0.0,
                    spread: 0.0,
                }],
                ..VisualStyle::default()
            }),
        ];
        let mut buffer = vec![0_u32; 20 * 20];

        render_to_buffer(&scene, &mut buffer, 20, 20, Color::WHITE);

        assert_ne!(buffer[13 * 20 + 13], pack_rgb(Color::WHITE));
    }

    #[test]
    fn alpha_blending_uses_linear_color_space() {
        let mut buffer = vec![pack_rgb(Color::BLACK)];

        blend_pixel(&mut buffer, 1, 1, 0, 0, Color::rgba(255, 255, 255, 128));

        assert_eq!(buffer[0], pack_rgb(Color::rgb(188, 188, 188)));
    }

    #[test]
    fn rendered_text_pixels_change_when_font_family_changes() {
        let bundled_family = bundled_font_family();
        let baseline_style = TextStyle {
            size_px: 28.0,
            ..TextStyle::default()
        };
        let bundled_style = TextStyle {
            families: vec![FontFamily::Named(bundled_family)],
            size_px: 28.0,
            ..TextStyle::default()
        };
        let baseline_scene = text_scene(baseline_style);
        let bundled_scene = text_scene(bundled_style);
        let mut baseline_buffer = vec![0_u32; 320 * 120];
        let mut bundled_buffer = vec![0_u32; 320 * 120];

        render_to_buffer(
            &baseline_scene,
            &mut baseline_buffer,
            320,
            120,
            Color::WHITE,
        );
        render_to_buffer(&bundled_scene, &mut bundled_buffer, 320, 120, Color::WHITE);

        let different_pixels = baseline_buffer
            .iter()
            .zip(&bundled_buffer)
            .filter(|(left, right)| left != right)
            .count();

        assert_ne!(baseline_buffer, bundled_buffer);
        assert!(different_pixels > 100);
    }

    #[test]
    fn text_transform_renders_the_same_pixels_as_pretransformed_content() {
        let transformed_scene = text_scene_with_content(
            TextStyle {
                text_transform: TextTransform::Uppercase,
                ..TextStyle::default()
            },
            "Straße",
        );
        let literal_scene = text_scene_with_content(TextStyle::default(), "STRASSE");
        let mut transformed_buffer = vec![0_u32; 320 * 120];
        let mut literal_buffer = vec![0_u32; 320 * 120];

        render_to_buffer(
            &transformed_scene,
            &mut transformed_buffer,
            320,
            120,
            Color::WHITE,
        );
        render_to_buffer(&literal_scene, &mut literal_buffer, 320, 120, Color::WHITE);

        assert_eq!(transformed_buffer, literal_buffer);
    }

    #[test]
    fn text_stroke_renders_outline_pixels_without_extra_nodes() {
        let stroked_scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 320.0, 120.0))
                .with_style(VisualStyle {
                    background: Some(Color::rgb(245, 247, 250)),
                    ..VisualStyle::default()
                })
                .with_child(
                    RenderNode::text(LayoutBox::new(20.0, 28.0, 280.0, 56.0), "Outline")
                        .with_style(VisualStyle {
                            foreground: Color::rgb(17, 37, 61),
                            text: TextStyle::default(),
                            text_stroke: TextStrokeStyle {
                                width: 2.0,
                                color: Some(Color::rgb(245, 158, 11)),
                            },
                            ..VisualStyle::default()
                        }),
                ),
        ];
        let baseline_outline_scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 320.0, 120.0))
                .with_style(VisualStyle {
                    background: Some(Color::rgb(245, 247, 250)),
                    ..VisualStyle::default()
                })
                .with_child(
                    RenderNode::text(LayoutBox::new(20.0, 28.0, 280.0, 56.0), "Outline")
                        .with_style(VisualStyle {
                            foreground: Color::rgb(17, 37, 61),
                            text: TextStyle::default(),
                            ..VisualStyle::default()
                        }),
                ),
        ];
        let mut baseline_buffer = vec![0_u32; 320 * 120];
        let mut stroked_buffer = vec![0_u32; 320 * 120];

        render_to_buffer(
            &baseline_outline_scene,
            &mut baseline_buffer,
            320,
            120,
            Color::WHITE,
        );
        render_to_buffer(&stroked_scene, &mut stroked_buffer, 320, 120, Color::WHITE);

        let different_pixels = baseline_buffer
            .iter()
            .zip(&stroked_buffer)
            .filter(|(left, right)| left != right)
            .count();

        assert!(different_pixels > 0);
        assert!(stroked_buffer.contains(&pack_rgb(Color::rgb(245, 158, 11))));
    }

    #[test]
    fn text_shadow_renders_glow_outside_text_bounds() {
        let scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 320.0, 120.0))
                .with_style(VisualStyle {
                    background: Some(Color::rgb(245, 247, 250)),
                    ..VisualStyle::default()
                })
                .with_child(
                    RenderNode::text(LayoutBox::new(40.0, 36.0, 180.0, 40.0), "Glow").with_style(
                        VisualStyle {
                            foreground: Color::rgb(17, 37, 61),
                            text: TextStyle::default(),
                            text_shadows: vec![ShadowEffect {
                                color: Some(Color::rgba(37, 99, 235, 180)),
                                offset_x: 0.0,
                                offset_y: 0.0,
                                blur_radius: 4.0,
                                spread: 2.0,
                            }],
                            ..VisualStyle::default()
                        },
                    ),
                ),
        ];
        let mut buffer = vec![0_u32; 320 * 120];

        render_to_buffer(&scene, &mut buffer, 320, 120, Color::WHITE);

        assert_ne!(buffer[34 * 320 + 36], pack_rgb(Color::WHITE));
    }

    #[test]
    fn visible_overflow_text_shadow_can_paint_beyond_the_text_layout_box() {
        let background = Color::rgb(245, 247, 250);
        let scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 240.0, 120.0))
                .with_style(VisualStyle {
                    background: Some(background),
                    ..VisualStyle::default()
                })
                .with_child(
                    RenderNode::text(LayoutBox::new(80.0, 40.0, 84.0, 36.0), "Glow").with_style(
                        VisualStyle {
                            foreground: Color::rgb(34, 197, 94),
                            text: TextStyle::default(),
                            text_shadows: vec![ShadowEffect {
                                color: Some(Color::rgba(34, 197, 94, 200)),
                                offset_x: 0.0,
                                offset_y: 0.0,
                                blur_radius: 6.0,
                                spread: 3.0,
                            }],
                            ..VisualStyle::default()
                        },
                    ),
                ),
        ];
        let mut buffer = vec![0_u32; 240 * 120];

        render_to_buffer(&scene, &mut buffer, 240, 120, Color::WHITE);

        assert_ne!(buffer[48 * 240 + 76], pack_rgb(background));
    }

    #[test]
    fn clipped_text_shadow_stays_inside_the_text_layout_box() {
        let background = Color::rgb(245, 247, 250);
        let scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 240.0, 120.0))
                .with_style(VisualStyle {
                    background: Some(background),
                    ..VisualStyle::default()
                })
                .with_child(
                    RenderNode::text(LayoutBox::new(80.0, 40.0, 84.0, 36.0), "Glow").with_style(
                        VisualStyle {
                            foreground: Color::rgb(34, 197, 94),
                            text: TextStyle::default(),
                            text_shadows: vec![ShadowEffect {
                                color: Some(Color::rgba(34, 197, 94, 200)),
                                offset_x: 0.0,
                                offset_y: 0.0,
                                blur_radius: 6.0,
                                spread: 3.0,
                            }],
                            overflow: Overflow::CLIP,
                            ..VisualStyle::default()
                        },
                    ),
                ),
        ];
        let mut buffer = vec![0_u32; 240 * 120];

        render_to_buffer(&scene, &mut buffer, 240, 120, Color::WHITE);

        assert_eq!(buffer[48 * 240 + 76], pack_rgb(background));
    }

    #[test]
    fn filter_drop_shadow_renders_for_supported_container_layers() {
        let scene = vec![
            RenderNode::container(LayoutBox::new(48.0, 32.0, 72.0, 44.0)).with_style(VisualStyle {
                background: Some(Color::rgb(241, 245, 249)),
                filter_drop_shadows: vec![ShadowEffect {
                    color: Some(Color::rgba(15, 23, 42, 180)),
                    offset_x: 8.0,
                    offset_y: 10.0,
                    blur_radius: 6.0,
                    spread: 0.0,
                }],
                ..VisualStyle::default()
            }),
        ];
        let mut buffer = vec![0_u32; 200 * 140];

        render_to_buffer(&scene, &mut buffer, 200, 140, Color::WHITE);

        assert_ne!(buffer[86 * 200 + 126], pack_rgb(Color::WHITE));
    }

    #[test]
    fn dispatch_click_invokes_the_hit_handler() {
        CLICK_COUNT.store(0, Ordering::SeqCst);
        let scene = vec![
            RenderNode::container(LayoutBox::new(4.0, 6.0, 40.0, 24.0))
                .on_click(increment_click_count),
        ];

        assert!(dispatch_click(&scene, 12.0, 12.0));
        assert_eq!(CLICK_COUNT.load(Ordering::SeqCst), 1);
        assert!(!dispatch_click(&scene, 80.0, 80.0));
    }

    #[test]
    fn dispatch_click_prefers_the_topmost_child() {
        CLICK_TARGET.store(0, Ordering::SeqCst);
        let scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 80.0, 60.0))
                .on_click(mark_parent_clicked)
                .with_child(
                    RenderNode::container(LayoutBox::new(12.0, 10.0, 30.0, 20.0))
                        .on_click(mark_child_clicked),
                ),
        ];

        assert!(dispatch_click(&scene, 20.0, 18.0));
        assert_eq!(CLICK_TARGET.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn hit_testing_respects_parent_clipping() {
        CLICK_COUNT.store(0, Ordering::SeqCst);
        let scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 20.0, 20.0))
                .with_style(VisualStyle {
                    overflow: Overflow::CLIP,
                    ..VisualStyle::default()
                })
                .with_child(
                    RenderNode::container(LayoutBox::new(10.0, 10.0, 20.0, 20.0))
                        .on_click(increment_click_count),
                ),
        ];

        assert!(dispatch_click(&scene, 12.0, 12.0));
        assert_eq!(CLICK_COUNT.load(Ordering::SeqCst), 1);
        assert!(!dispatch_click(&scene, 28.0, 28.0));
    }

    #[test]
    fn hit_test_element_path_returns_the_deepest_visible_node() {
        let scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 80.0, 60.0)).with_child(
                RenderNode::container(LayoutBox::new(12.0, 10.0, 30.0, 20.0))
                    .with_child(RenderNode::container(LayoutBox::new(18.0, 14.0, 8.0, 6.0))),
            ),
        ];

        assert_eq!(
            hit_test_element_path(&scene, 20.0, 18.0),
            Some(ElementPath {
                root: 0,
                children: vec![0, 0],
            })
        );
        assert_eq!(
            hit_test_element_path(&scene, 6.0, 6.0),
            Some(ElementPath::root(0))
        );
    }

    #[test]
    fn hit_test_element_path_respects_parent_clipping() {
        let scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 20.0, 20.0))
                .with_style(VisualStyle {
                    overflow: Overflow::CLIP,
                    ..VisualStyle::default()
                })
                .with_child(RenderNode::container(LayoutBox::new(
                    10.0, 10.0, 20.0, 20.0,
                ))),
        ];

        assert_eq!(
            hit_test_element_path(&scene, 12.0, 12.0),
            Some(ElementPath {
                root: 0,
                children: vec![0],
            })
        );
        assert_eq!(hit_test_element_path(&scene, 28.0, 28.0), None);
    }

    #[test]
    fn resize_buffer_tracks_the_latest_window_size_without_scaling() {
        let mut width = 320;
        let mut height = 180;
        let mut buffer = vec![0_u32; width * height];

        resize_buffer(&mut buffer, &mut width, &mut height, 640, 360, Color::WHITE);

        assert_eq!(width, 640);
        assert_eq!(height, 360);
        assert_eq!(buffer.len(), 640 * 360);
    }

    #[test]
    fn window_options_enable_native_resizing() {
        assert!(window_options().resize);
    }

    #[test]
    fn visual_scene_comparison_ignores_click_handlers() {
        let left = vec![
            RenderNode::container(LayoutBox::new(4.0, 6.0, 40.0, 24.0))
                .on_click(increment_click_count),
        ];
        let right = vec![
            RenderNode::container(LayoutBox::new(4.0, 6.0, 40.0, 24.0))
                .on_click(alternate_click_handler),
        ];

        assert!(scenes_match_visuals(&left, &right));
    }

    #[test]
    fn should_present_scene_when_visuals_change() {
        let previous = vec![
            RenderNode::container(LayoutBox::new(4.0, 6.0, 40.0, 24.0)).with_style(VisualStyle {
                background: Some(Color::rgb(40, 120, 220)),
                ..VisualStyle::default()
            }),
        ];
        let next = vec![
            RenderNode::container(LayoutBox::new(4.0, 6.0, 40.0, 24.0)).with_style(VisualStyle {
                background: Some(Color::rgb(220, 38, 38)),
                ..VisualStyle::default()
            }),
        ];

        assert!(should_present_scene(Some(&previous), &next, false));
        assert!(!should_present_scene(Some(&previous), &previous, false));
    }

    #[test]
    fn should_present_scene_when_buffer_is_resized() {
        let scene = vec![RenderNode::container(LayoutBox::new(4.0, 6.0, 40.0, 24.0))];

        assert!(should_present_scene(Some(&scene), &scene, true));
    }

    #[test]
    fn super_dragging_suspends_updates() {
        assert!(should_suspend_updates(true, true, false));
        assert!(should_suspend_updates(true, false, true));
        assert!(should_suspend_updates(true, true, true));
        assert!(!should_suspend_updates(false, true, false));
        assert!(!should_suspend_updates(true, false, false));
    }

    #[test]
    fn drawable_viewport_size_skips_minimized_windows() {
        assert_eq!(drawable_viewport_size(0, 0), None);
        assert_eq!(drawable_viewport_size(0, 720), None);
        assert_eq!(drawable_viewport_size(1280, 0), None);
        assert_eq!(
            drawable_viewport_size(1280, 720),
            Some(ViewportSize::new(1280, 720))
        );
    }

    #[test]
    fn window_config_enables_middle_button_auto_scroll_by_default() {
        let config = WindowConfig::new("cssimpler", 960, 540);

        assert!(config.middle_button_auto_scroll);
    }

    #[test]
    fn auto_scroll_indicator_changes_force_a_present() {
        let scene = vec![RenderNode::container(LayoutBox::new(4.0, 6.0, 40.0, 24.0))];

        assert!(should_present_frame(
            Some(&scene),
            &scene,
            None,
            Some(crate::scrollbar::AutoScrollIndicator { x: 24.0, y: 18.0 }),
            false,
        ));
    }

    #[test]
    fn incremental_render_matches_full_render_when_a_node_moves() {
        let previous = vec![
            RenderNode::container(LayoutBox::new(2.0, 2.0, 4.0, 4.0)).with_style(VisualStyle {
                background: Some(Color::rgb(40, 120, 220)),
                ..VisualStyle::default()
            }),
        ];
        let next = vec![
            RenderNode::container(LayoutBox::new(10.0, 2.0, 4.0, 4.0)).with_style(VisualStyle {
                background: Some(Color::rgb(40, 120, 220)),
                ..VisualStyle::default()
            }),
        ];
        let mut incremental = vec![0_u32; 20 * 20];
        let mut full = vec![0_u32; 20 * 20];

        render_to_buffer(&previous, &mut incremental, 20, 20, Color::WHITE);
        render_scene_update(&previous, &next, &mut incremental, 20, 20, Color::WHITE);
        render_to_buffer(&next, &mut full, 20, 20, Color::WHITE);

        assert_eq!(incremental, full);
    }

    #[test]
    fn incremental_render_clears_shadow_pixels_and_redraws_the_background() {
        let background =
            RenderNode::container(LayoutBox::new(0.0, 0.0, 20.0, 20.0)).with_style(VisualStyle {
                background: Some(Color::rgb(226, 232, 240)),
                ..VisualStyle::default()
            });
        let previous = vec![
            background.clone(),
            RenderNode::container(LayoutBox::new(6.0, 6.0, 4.0, 4.0)).with_style(VisualStyle {
                background: Some(Color::rgb(15, 23, 42)),
                shadows: vec![BoxShadow {
                    color: Color::rgba(15, 23, 42, 140),
                    offset_x: 3.0,
                    offset_y: 3.0,
                    blur_radius: 2.0,
                    spread: 0.0,
                }],
                ..VisualStyle::default()
            }),
        ];
        let next = vec![
            background,
            RenderNode::container(LayoutBox::new(6.0, 6.0, 4.0, 4.0)).with_style(VisualStyle {
                background: Some(Color::rgb(15, 23, 42)),
                ..VisualStyle::default()
            }),
        ];
        let mut incremental = vec![0_u32; 20 * 20];
        let mut full = vec![0_u32; 20 * 20];

        render_to_buffer(&previous, &mut incremental, 20, 20, Color::WHITE);
        render_scene_update(&previous, &next, &mut incremental, 20, 20, Color::WHITE);
        render_to_buffer(&next, &mut full, 20, 20, Color::WHITE);

        assert_eq!(incremental, full);
    }

    #[test]
    fn incremental_render_redraws_pixels_cleared_by_fractional_dirty_regions() {
        let background =
            RenderNode::container(LayoutBox::new(0.0, 0.0, 32.0, 16.0)).with_style(VisualStyle {
                background: Some(Color::rgb(226, 232, 240)),
                ..VisualStyle::default()
            });
        let static_bar =
            RenderNode::container(LayoutBox::new(6.0, 4.0, 20.0, 8.0)).with_style(VisualStyle {
                background: Some(Color::rgb(15, 23, 42)),
                ..VisualStyle::default()
            });
        let previous = vec![
            background.clone(),
            static_bar.clone(),
            RenderNode::container(LayoutBox::new(10.0, 0.0, 7.2, 16.0)),
        ];
        let next = vec![
            background,
            static_bar,
            RenderNode::container(LayoutBox::new(10.0, 0.0, 11.6, 16.0)),
        ];
        let mut incremental = vec![0_u32; 32 * 16];
        let mut full = vec![0_u32; 32 * 16];

        render_to_buffer(&previous, &mut incremental, 32, 16, Color::WHITE);
        render_scene_update(&previous, &next, &mut incremental, 32, 16, Color::WHITE);
        render_to_buffer(&next, &mut full, 32, 16, Color::WHITE);

        assert_eq!(incremental, full);
    }

    #[test]
    fn incremental_render_matches_full_render_for_fractional_dirty_regions_over_text() {
        let background =
            RenderNode::container(LayoutBox::new(0.0, 0.0, 160.0, 80.0)).with_style(VisualStyle {
                background: Some(Color::rgb(245, 247, 250)),
                ..VisualStyle::default()
            });
        let text =
            RenderNode::text(LayoutBox::new(24.0, 20.0, 112.0, 36.0), "UIVERSE").with_style(
                VisualStyle {
                    foreground: Color::rgba(255, 255, 255, 0),
                    text: TextStyle {
                        size_px: 32.0,
                        ..TextStyle::default()
                    },
                    text_stroke: TextStrokeStyle {
                        width: 1.0,
                        color: Some(Color::rgb(15, 23, 42)),
                    },
                    ..VisualStyle::default()
                },
            );
        let previous = vec![
            background.clone(),
            text.clone(),
            RenderNode::container(LayoutBox::new(0.0, 0.0, 53.2, 80.0)),
        ];
        let next = vec![
            background,
            text,
            RenderNode::container(LayoutBox::new(0.0, 0.0, 57.7, 80.0)),
        ];
        let mut incremental = vec![0_u32; 160 * 80];
        let mut full = vec![0_u32; 160 * 80];

        render_to_buffer(&previous, &mut incremental, 160, 80, Color::WHITE);
        render_scene_update(&previous, &next, &mut incremental, 160, 80, Color::WHITE);
        render_to_buffer(&next, &mut full, 160, 80, Color::WHITE);

        assert_eq!(incremental, full);
    }

    #[test]
    fn incremental_render_does_not_leave_stripes_after_a_fractional_reveal_sweeps_over_text() {
        fn scene(reveal_width: f32) -> Vec<RenderNode> {
            vec![
                RenderNode::container(LayoutBox::new(0.0, 0.0, 200.0, 96.0)).with_style(
                    VisualStyle {
                        background: Some(Color::rgb(38, 38, 38)),
                        ..VisualStyle::default()
                    },
                ),
                RenderNode::text(LayoutBox::new(28.0, 28.0, 144.0, 40.0), "UIVERSE").with_style(
                    VisualStyle {
                        foreground: Color::rgba(255, 255, 255, 0),
                        text: TextStyle {
                            size_px: 36.0,
                            ..TextStyle::default()
                        },
                        text_stroke: TextStrokeStyle {
                            width: 1.0,
                            color: Some(Color::rgb(240, 240, 240)),
                        },
                        ..VisualStyle::default()
                    },
                ),
                RenderNode::container(LayoutBox::new(28.0, 20.0, reveal_width, 56.0)).with_style(
                    VisualStyle {
                        overflow: Overflow::CLIP,
                        ..VisualStyle::default()
                    },
                ),
            ]
        }

        let widths = [4.2, 21.4, 39.8, 58.1, 76.6, 95.2, 63.7, 27.3, 4.2];
        let mut incremental = vec![0_u32; 200 * 96];
        let mut previous = scene(widths[0]);
        render_to_buffer(&previous, &mut incremental, 200, 96, Color::WHITE);

        for width in widths.iter().copied().skip(1) {
            let next = scene(width);
            render_scene_update(&previous, &next, &mut incremental, 200, 96, Color::WHITE);
            previous = next;
        }

        let final_scene = scene(*widths.last().expect("sequence should not be empty"));
        let mut full = vec![0_u32; 200 * 96];
        render_to_buffer(&final_scene, &mut full, 200, 96, Color::WHITE);

        assert_eq!(incremental, full);
    }
}
