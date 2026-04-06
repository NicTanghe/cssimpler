use std::collections::{HashMap, HashSet};
use std::time::Duration;

use cssimpler_core::{Color, CornerRadius, LayoutBox, LinearRgba, RenderNode};

use crate::{
    ClipRect, PreparedBlendColor, blend_prepared_pixel, draw_rounded_rect, inset_layout,
    layout_clip, layout_contains, offset_layout,
};

const MIN_THUMB_LENGTH: f32 = 18.0;
const WHEEL_SCROLL_STEP: f32 = 40.0;
const AUTO_SCROLL_DEADZONE: f32 = 8.0;
const AUTO_SCROLL_SPEED_PER_PIXEL: f32 = 10.0;
const AUTO_SCROLL_MAX_FRAME_SECONDS: f32 = 0.05;
const AUTO_SCROLL_INDICATOR_RADIUS: f32 = 15.0;

#[derive(Default)]
pub(crate) struct ScrollbarController {
    offsets: HashMap<Vec<usize>, CachedOffsets>,
    drag: Option<ScrollbarDrag>,
    auto_scroll: Option<AutoScrollSession>,
}

#[derive(Clone, Copy, Default)]
struct CachedOffsets {
    x: f32,
    y: f32,
}

#[derive(Clone)]
struct ScrollbarDrag {
    path: Vec<usize>,
    axis: ScrollbarAxis,
    start_mouse: f32,
    start_offset: f32,
}

#[derive(Clone)]
struct AutoScrollSession {
    path: Vec<usize>,
    anchor_x: f32,
    anchor_y: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct AutoScrollIndicator {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScrollbarAxis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScrollbarPart {
    Track,
    Thumb,
}

#[derive(Clone)]
struct ScrollbarHit {
    path: Vec<usize>,
    axis: ScrollbarAxis,
    part: ScrollbarPart,
}

#[derive(Clone, Copy)]
struct ScrollbarRects {
    track: LayoutBox,
    thumb: LayoutBox,
}

#[derive(Clone, Copy)]
enum AutoScrollDirection {
    Up,
    Down,
    Left,
    Right,
}

impl ScrollbarController {
    pub(crate) fn apply_to_scene(&mut self, scene: &mut [RenderNode]) {
        let mut live_paths = HashSet::new();
        for (index, node) in scene.iter_mut().enumerate() {
            let mut path = vec![index];
            self.apply_to_node(node, &mut path, &mut live_paths);
        }

        self.offsets.retain(|path, _| live_paths.contains(path));
        if self
            .drag
            .as_ref()
            .is_some_and(|drag| !live_paths.contains(&drag.path))
        {
            self.drag = None;
        }
        if self
            .auto_scroll
            .as_ref()
            .is_some_and(|auto_scroll| !live_paths.contains(&auto_scroll.path))
        {
            self.auto_scroll = None;
        }
    }

    pub(crate) fn toggle_middle_button_auto_scroll(
        &mut self,
        scene: &[RenderNode],
        mouse_position: Option<(f32, f32)>,
    ) -> bool {
        if self.auto_scroll.take().is_some() {
            return true;
        }

        let Some((mouse_x, mouse_y)) = mouse_position else {
            return false;
        };
        let Some(path) = find_auto_scroll_target(scene, mouse_x, mouse_y) else {
            return false;
        };
        self.auto_scroll = Some(AutoScrollSession {
            path,
            anchor_x: mouse_x,
            anchor_y: mouse_y,
        });
        true
    }

    pub(crate) fn cancel_middle_button_auto_scroll(&mut self) -> bool {
        self.auto_scroll.take().is_some()
    }

    pub(crate) fn auto_scroll_indicator(&self) -> Option<AutoScrollIndicator> {
        self.auto_scroll
            .as_ref()
            .map(|auto_scroll| AutoScrollIndicator {
                x: auto_scroll.anchor_x,
                y: auto_scroll.anchor_y,
            })
    }

    pub(crate) fn step_middle_button_auto_scroll(
        &mut self,
        scene: &mut [RenderNode],
        mouse_position: Option<(f32, f32)>,
        frame_delta: Duration,
    ) -> bool {
        let Some(auto_scroll) = self.auto_scroll.clone() else {
            return false;
        };
        let Some((mouse_x, mouse_y)) = mouse_position else {
            return false;
        };

        let frame_seconds = frame_delta
            .as_secs_f32()
            .clamp(0.0, AUTO_SCROLL_MAX_FRAME_SECONDS);
        if frame_seconds <= f32::EPSILON {
            return false;
        }

        let delta_x = auto_scroll_delta(mouse_x - auto_scroll.anchor_x, frame_seconds);
        let delta_y = auto_scroll_delta(mouse_y - auto_scroll.anchor_y, frame_seconds);
        if delta_x.abs() <= f32::EPSILON && delta_y.abs() <= f32::EPSILON {
            return false;
        }

        let Some(node) = node_mut_at_path(scene, &auto_scroll.path) else {
            self.auto_scroll = None;
            return false;
        };
        if !node.scrollbars.is_some_and(scrollbars_can_auto_scroll) {
            self.auto_scroll = None;
            return false;
        }

        self.apply_scroll_delta(node, &auto_scroll.path, delta_x, delta_y)
    }

    pub(crate) fn handle_wheel(
        &mut self,
        scene: &mut [RenderNode],
        mouse_position: Option<(f32, f32)>,
        wheel: Option<(f32, f32)>,
    ) -> bool {
        let Some((mouse_x, mouse_y)) = mouse_position else {
            return false;
        };
        let Some((wheel_x, wheel_y)) = wheel else {
            return false;
        };

        let delta_x = if wheel_x.abs() <= f32::EPSILON {
            0.0
        } else {
            -wheel_x * WHEEL_SCROLL_STEP
        };
        let delta_y = if wheel_y.abs() <= f32::EPSILON {
            0.0
        } else {
            -wheel_y * WHEEL_SCROLL_STEP
        };

        if delta_x.abs() <= f32::EPSILON && delta_y.abs() <= f32::EPSILON {
            return false;
        }

        for (index, node) in scene.iter_mut().enumerate().rev() {
            let mut path = vec![index];
            if self.handle_wheel_on_node(
                node,
                &mut path,
                mouse_x,
                mouse_y,
                ClipRect::unbounded(),
                delta_x,
                delta_y,
            ) {
                return true;
            }
        }

        false
    }

    pub(crate) fn handle_pointer(
        &mut self,
        scene: &mut [RenderNode],
        mouse_position: Option<(f32, f32)>,
        left_down: bool,
        click_started: bool,
    ) -> bool {
        if !left_down {
            self.drag = None;
        }

        let Some((mouse_x, mouse_y)) = mouse_position else {
            return false;
        };

        let mut consumed = false;

        if let Some(drag) = self.drag.clone() {
            if let Some(node) = node_mut_at_path(scene, &drag.path) {
                consumed |= self.update_drag(node, &drag, mouse_x, mouse_y);
            } else {
                self.drag = None;
            }
        } else if click_started
            && let Some(hit) = find_scrollbar_hit(scene, mouse_x, mouse_y)
            && let Some(node) = node_mut_at_path(scene, &hit.path)
        {
            consumed = true;
            match hit.part {
                ScrollbarPart::Thumb => {
                    self.start_drag(node, &hit, mouse_x, mouse_y);
                }
                ScrollbarPart::Track => {
                    page_scroll(node, &hit, mouse_x, mouse_y);
                }
            }
            self.persist_node_offsets(node, &hit.path);
        }

        if let Some(hit) = find_scrollbar_hit(scene, mouse_x, mouse_y)
            && let Some(node) = node_mut_at_path(scene, &hit.path)
        {
            mark_hover(node, hit.axis, hit.part);
        }

        if let Some(drag) = &self.drag
            && let Some(node) = node_mut_at_path(scene, &drag.path)
        {
            mark_active(node, drag.axis);
        }

        consumed
    }

    fn apply_to_node(
        &mut self,
        node: &mut RenderNode,
        path: &mut Vec<usize>,
        live_paths: &mut HashSet<Vec<usize>>,
    ) {
        if let Some(scrollbars) = node.scrollbars.as_mut() {
            live_paths.insert(path.clone());
            scrollbars.interaction = Default::default();

            let cached = self.offsets.get(path).copied().unwrap_or(CachedOffsets {
                x: scrollbars.metrics.offset_x,
                y: scrollbars.metrics.offset_y,
            });
            let previous_x = scrollbars.metrics.offset_x;
            let previous_y = scrollbars.metrics.offset_y;
            scrollbars.metrics.offset_x = cached.x;
            scrollbars.metrics.offset_y = cached.y;
            scrollbars.clamp_offsets();

            let delta_x = previous_x - scrollbars.metrics.offset_x;
            let delta_y = previous_y - scrollbars.metrics.offset_y;
            if delta_x.abs() > f32::EPSILON || delta_y.abs() > f32::EPSILON {
                for child in &mut node.children {
                    translate_subtree(child, delta_x, delta_y);
                }
            }

            self.persist_node_offsets(node, path);
        }

        for (index, child) in node.children.iter_mut().enumerate() {
            path.push(index);
            self.apply_to_node(child, path, live_paths);
            path.pop();
        }
    }

    fn handle_wheel_on_node(
        &mut self,
        node: &mut RenderNode,
        path: &mut Vec<usize>,
        x: f32,
        y: f32,
        clip: ClipRect,
        delta_x: f32,
        delta_y: f32,
    ) -> bool {
        if !clip.contains(x, y) || !layout_contains(node.layout, x, y) {
            return false;
        }

        let child_clip = if node.style.overflow.clips_any_axis() {
            let Some(child_clip) = clip.intersect(layout_clip(node.layout)) else {
                return false;
            };
            child_clip
        } else {
            clip
        };

        for index in (0..node.children.len()).rev() {
            path.push(index);
            if self.handle_wheel_on_node(
                &mut node.children[index],
                path,
                x,
                y,
                child_clip,
                delta_x,
                delta_y,
            ) {
                path.pop();
                return true;
            }
            path.pop();
        }

        let Some(scrollbars) = node.scrollbars else {
            return false;
        };
        if !scrollbars.overflow_x.allows_scrolling() && !scrollbars.overflow_y.allows_scrolling() {
            return false;
        }

        self.apply_scroll_delta(node, path, delta_x, delta_y)
    }

    fn apply_scroll_delta(
        &mut self,
        node: &mut RenderNode,
        path: &[usize],
        delta_x: f32,
        delta_y: f32,
    ) -> bool {
        let Some(scrollbars) = node.scrollbars.as_mut() else {
            return false;
        };

        let previous_x = scrollbars.metrics.offset_x;
        let previous_y = scrollbars.metrics.offset_y;

        if scrollbars.overflow_x.allows_scrolling() && scrollbars.metrics.max_offset_x > 0.0 {
            scrollbars.metrics.offset_x += delta_x;
        }
        if scrollbars.overflow_y.allows_scrolling() && scrollbars.metrics.max_offset_y > 0.0 {
            scrollbars.metrics.offset_y += delta_y;
        }
        scrollbars.clamp_offsets();

        let applied_delta_x = previous_x - scrollbars.metrics.offset_x;
        let applied_delta_y = previous_y - scrollbars.metrics.offset_y;
        if applied_delta_x.abs() <= f32::EPSILON && applied_delta_y.abs() <= f32::EPSILON {
            return false;
        }

        for child in &mut node.children {
            translate_subtree(child, applied_delta_x, applied_delta_y);
        }

        self.persist_node_offsets(node, path);
        true
    }

    fn update_drag(
        &mut self,
        node: &mut RenderNode,
        drag: &ScrollbarDrag,
        mouse_x: f32,
        mouse_y: f32,
    ) -> bool {
        let Some(scrollbars) = node.scrollbars else {
            self.drag = None;
            return false;
        };
        let Some(rects) = scrollbar_rects(node, drag.axis) else {
            self.drag = None;
            return false;
        };

        let travel = axis_length(rects.track, drag.axis) - axis_length(rects.thumb, drag.axis);
        let max_offset = axis_max_offset(scrollbars, drag.axis);
        if travel <= f32::EPSILON || max_offset <= f32::EPSILON {
            return false;
        }

        let mouse = axis_coordinate(mouse_x, mouse_y, drag.axis);
        let delta = mouse - drag.start_mouse;
        let next_offset = drag.start_offset + delta * (max_offset / travel);
        self.apply_scroll_offset(node, &drag.path, drag.axis, next_offset)
    }

    fn start_drag(
        &mut self,
        node: &mut RenderNode,
        hit: &ScrollbarHit,
        mouse_x: f32,
        mouse_y: f32,
    ) {
        let Some(scrollbars) = node.scrollbars else {
            return;
        };
        self.drag = Some(ScrollbarDrag {
            path: hit.path.clone(),
            axis: hit.axis,
            start_mouse: axis_coordinate(mouse_x, mouse_y, hit.axis),
            start_offset: axis_offset(scrollbars, hit.axis),
        });
        mark_active(node, hit.axis);
    }

    fn apply_scroll_offset(
        &mut self,
        node: &mut RenderNode,
        path: &[usize],
        axis: ScrollbarAxis,
        next_offset: f32,
    ) -> bool {
        let Some(scrollbars) = node.scrollbars.as_mut() else {
            return false;
        };

        let previous_x = scrollbars.metrics.offset_x;
        let previous_y = scrollbars.metrics.offset_y;

        match axis {
            ScrollbarAxis::Horizontal => scrollbars.metrics.offset_x = next_offset,
            ScrollbarAxis::Vertical => scrollbars.metrics.offset_y = next_offset,
        }
        scrollbars.clamp_offsets();

        let delta_x = previous_x - scrollbars.metrics.offset_x;
        let delta_y = previous_y - scrollbars.metrics.offset_y;
        if delta_x.abs() <= f32::EPSILON && delta_y.abs() <= f32::EPSILON {
            return false;
        }

        for child in &mut node.children {
            translate_subtree(child, delta_x, delta_y);
        }

        self.persist_node_offsets(node, path);
        true
    }

    fn persist_node_offsets(&mut self, node: &RenderNode, path: &[usize]) {
        if let Some(scrollbars) = node.scrollbars {
            self.offsets.insert(
                path.to_vec(),
                CachedOffsets {
                    x: scrollbars.metrics.offset_x,
                    y: scrollbars.metrics.offset_y,
                },
            );
        }
    }
}

pub(crate) fn text_layout(node: &RenderNode) -> LayoutBox {
    let mut layout = inset_layout(node.layout, node.content_inset);

    if let Some(scrollbars) = node.scrollbars {
        layout.width = (layout.width + scrollbars.metrics.max_offset_x).max(0.0);
        layout.height = (layout.height + scrollbars.metrics.max_offset_y).max(0.0);
        layout = offset_layout(
            layout,
            -scrollbars.metrics.offset_x,
            -scrollbars.metrics.offset_y,
        );
    }

    layout
}

pub(crate) fn text_viewport(node: &RenderNode) -> LayoutBox {
    let mut viewport = inset_layout(node.layout, node.content_inset);
    if let Some(scrollbars) = node.scrollbars {
        viewport.width = (viewport.width - scrollbars.metrics.reserved_width).max(0.0);
        viewport.height = (viewport.height - scrollbars.metrics.reserved_height).max(0.0);
    }
    viewport
}

pub(crate) fn text_clip(node: &RenderNode, clip: ClipRect) -> ClipRect {
    let viewport = text_viewport(node);

    if node.style.overflow.clips_any_axis() || node.scrollbars.is_some() {
        clip.intersect(layout_clip(viewport))
            .unwrap_or(ClipRect::full(0.0, 0.0))
    } else {
        clip
    }
}

pub(crate) fn draw_scrollbars(
    node: &RenderNode,
    buffer: &mut [u32],
    width: usize,
    height: usize,
    clip: ClipRect,
) {
    let Some(scrollbars) = node.scrollbars else {
        return;
    };

    let resolved = resolved_scrollbar_colors(node, scrollbars);
    let radius = CornerRadius::all((scrollbars.resolved_width() * 0.5).min(12.0));

    if let Some(rects) = scrollbar_rects(node, ScrollbarAxis::Vertical) {
        let track_color = if scrollbars.interaction.vertical.track_hovered {
            mix_color(resolved.track, Color::WHITE, 0.08)
        } else {
            resolved.track
        };
        let thumb_color = axis_thumb_color(
            resolved.thumb,
            scrollbars.interaction.vertical.thumb_hovered,
            scrollbars.interaction.vertical.thumb_active,
        );
        draw_rounded_rect(
            buffer,
            width,
            height,
            rects.track,
            radius,
            track_color,
            clip,
        );
        draw_rounded_rect(
            buffer,
            width,
            height,
            rects.thumb,
            radius,
            thumb_color,
            clip,
        );
    }

    if let Some(rects) = scrollbar_rects(node, ScrollbarAxis::Horizontal) {
        let track_color = if scrollbars.interaction.horizontal.track_hovered {
            mix_color(resolved.track, Color::WHITE, 0.08)
        } else {
            resolved.track
        };
        let thumb_color = axis_thumb_color(
            resolved.thumb,
            scrollbars.interaction.horizontal.thumb_hovered,
            scrollbars.interaction.horizontal.thumb_active,
        );
        draw_rounded_rect(
            buffer,
            width,
            height,
            rects.track,
            radius,
            track_color,
            clip,
        );
        draw_rounded_rect(
            buffer,
            width,
            height,
            rects.thumb,
            radius,
            thumb_color,
            clip,
        );
    }

    if scrollbars.shows_horizontal()
        && scrollbars.shows_vertical()
        && let Some(corner) = scrollbar_corner(node, scrollbars.resolved_width())
    {
        draw_rounded_rect(buffer, width, height, corner, radius, resolved.track, clip);
    }
}

pub(crate) fn draw_auto_scroll_indicator(
    indicator: AutoScrollIndicator,
    buffer: &mut [u32],
    width: usize,
    height: usize,
) {
    let clip = ClipRect::full(width as f32, height as f32);
    let outer_layout = centered_layout(indicator.x, indicator.y, AUTO_SCROLL_INDICATOR_RADIUS);
    let inner_layout =
        centered_layout(indicator.x, indicator.y, AUTO_SCROLL_INDICATOR_RADIUS - 1.5);
    let center_dot = centered_layout(indicator.x, indicator.y, 2.5);
    let radius = CornerRadius::all(AUTO_SCROLL_INDICATOR_RADIUS);
    let inner_radius = CornerRadius::all((AUTO_SCROLL_INDICATOR_RADIUS - 1.5).max(0.0));

    draw_rounded_rect(
        buffer,
        width,
        height,
        offset_layout(outer_layout, 0.0, 1.0),
        radius,
        Color::rgba(15, 23, 42, 56),
        clip,
    );
    draw_rounded_rect(
        buffer,
        width,
        height,
        outer_layout,
        radius,
        Color::rgba(255, 255, 255, 232),
        clip,
    );
    draw_rounded_rect(
        buffer,
        width,
        height,
        inner_layout,
        inner_radius,
        Color::rgba(226, 232, 240, 248),
        clip,
    );
    draw_rounded_rect(
        buffer,
        width,
        height,
        center_dot,
        CornerRadius::all(2.5),
        Color::rgba(30, 41, 59, 216),
        clip,
    );

    let icon_color = Color::rgba(30, 41, 59, 212);
    draw_indicator_arrow(
        buffer,
        width,
        height,
        indicator.x,
        indicator.y - 8.5,
        AutoScrollDirection::Up,
        icon_color,
    );
    draw_indicator_arrow(
        buffer,
        width,
        height,
        indicator.x,
        indicator.y + 8.5,
        AutoScrollDirection::Down,
        icon_color,
    );
    draw_indicator_arrow(
        buffer,
        width,
        height,
        indicator.x - 8.5,
        indicator.y,
        AutoScrollDirection::Left,
        icon_color,
    );
    draw_indicator_arrow(
        buffer,
        width,
        height,
        indicator.x + 8.5,
        indicator.y,
        AutoScrollDirection::Right,
        icon_color,
    );
}

pub(crate) fn auto_scroll_indicator_bounds(indicator: AutoScrollIndicator) -> LayoutBox {
    centered_layout(indicator.x, indicator.y, AUTO_SCROLL_INDICATOR_RADIUS + 6.0)
}

fn find_scrollbar_hit(scene: &[RenderNode], x: f32, y: f32) -> Option<ScrollbarHit> {
    scene.iter().enumerate().rev().find_map(|(index, node)| {
        find_scrollbar_hit_node(node, x, y, ClipRect::unbounded(), &[index])
    })
}

fn centered_layout(x: f32, y: f32, radius: f32) -> LayoutBox {
    LayoutBox::new(x - radius, y - radius, radius * 2.0, radius * 2.0)
}

fn find_auto_scroll_target(scene: &[RenderNode], x: f32, y: f32) -> Option<Vec<usize>> {
    scene.iter().enumerate().rev().find_map(|(index, node)| {
        find_auto_scroll_target_node(node, x, y, ClipRect::unbounded(), &[index])
    })
}

fn find_auto_scroll_target_node(
    node: &RenderNode,
    x: f32,
    y: f32,
    clip: ClipRect,
    path: &[usize],
) -> Option<Vec<usize>> {
    if !clip.contains(x, y) || !layout_contains(node.layout, x, y) {
        return None;
    }

    let child_clip = if node.style.overflow.clips_any_axis() {
        clip.intersect(layout_clip(node.layout))?
    } else {
        clip
    };

    for (index, child) in node.children.iter().enumerate().rev() {
        let mut next_path = path.to_vec();
        next_path.push(index);
        if let Some(path) = find_auto_scroll_target_node(child, x, y, child_clip, &next_path) {
            return Some(path);
        }
    }

    node.scrollbars
        .filter(|scrollbars| scrollbars_can_auto_scroll(*scrollbars))
        .map(|_| path.to_vec())
}

fn find_scrollbar_hit_node(
    node: &RenderNode,
    x: f32,
    y: f32,
    clip: ClipRect,
    path: &[usize],
) -> Option<ScrollbarHit> {
    if !clip.contains(x, y) || !layout_contains(node.layout, x, y) {
        return None;
    }

    if let Some(hit) = scrollbar_hit_in_node(node, x, y, path) {
        return Some(hit);
    }

    let child_clip = if node.style.overflow.clips_any_axis() {
        clip.intersect(layout_clip(node.layout))?
    } else {
        clip
    };

    node.children
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, child)| {
            let mut next_path = path.to_vec();
            next_path.push(index);
            find_scrollbar_hit_node(child, x, y, child_clip, &next_path)
        })
}

fn scrollbar_hit_in_node(
    node: &RenderNode,
    x: f32,
    y: f32,
    path: &[usize],
) -> Option<ScrollbarHit> {
    let scrollbars = node.scrollbars?;

    if let Some(corner) = scrollbar_corner(node, scrollbars.resolved_width())
        && layout_contains(corner, x, y)
    {
        return None;
    }

    for axis in [ScrollbarAxis::Vertical, ScrollbarAxis::Horizontal] {
        let Some(rects) = scrollbar_rects(node, axis) else {
            continue;
        };

        if layout_contains(rects.thumb, x, y) {
            return Some(ScrollbarHit {
                path: path.to_vec(),
                axis,
                part: ScrollbarPart::Thumb,
            });
        }

        if layout_contains(rects.track, x, y) {
            return Some(ScrollbarHit {
                path: path.to_vec(),
                axis,
                part: ScrollbarPart::Track,
            });
        }
    }

    None
}

fn scrollbar_rects(node: &RenderNode, axis: ScrollbarAxis) -> Option<ScrollbarRects> {
    let scrollbars = node.scrollbars?;
    let thickness = scrollbars.resolved_width();
    if thickness <= f32::EPSILON {
        return None;
    }

    let shows_axis = match axis {
        ScrollbarAxis::Horizontal => scrollbars.shows_horizontal(),
        ScrollbarAxis::Vertical => scrollbars.shows_vertical(),
    };
    if !shows_axis {
        return None;
    }

    let inner = inset_layout(node.layout, node.style.border.widths);
    let overlap = if axis == ScrollbarAxis::Vertical && scrollbars.shows_horizontal()
        || axis == ScrollbarAxis::Horizontal && scrollbars.shows_vertical()
    {
        thickness
    } else {
        0.0
    };

    let track = match axis {
        ScrollbarAxis::Vertical => LayoutBox::new(
            inner.x + (inner.width - thickness).max(0.0),
            inner.y,
            thickness.min(inner.width),
            (inner.height - overlap).max(0.0),
        ),
        ScrollbarAxis::Horizontal => LayoutBox::new(
            inner.x,
            inner.y + (inner.height - thickness).max(0.0),
            (inner.width - overlap).max(0.0),
            thickness.min(inner.height),
        ),
    };

    if track.width <= f32::EPSILON || track.height <= f32::EPSILON {
        return None;
    }

    let track_length = axis_length(track, axis);
    let max_offset = axis_max_offset(scrollbars, axis);
    let offset = axis_offset(scrollbars, axis);
    let viewport = track_length.max(1.0);
    let total = viewport + max_offset.max(0.0);
    let thumb_length = if max_offset <= f32::EPSILON {
        track_length
    } else {
        (track_length * (viewport / total)).clamp(MIN_THUMB_LENGTH.min(track_length), track_length)
    };
    let thumb_travel = (track_length - thumb_length).max(0.0);
    let thumb_offset = if max_offset <= f32::EPSILON {
        0.0
    } else {
        thumb_travel * (offset / max_offset)
    };

    let thumb = match axis {
        ScrollbarAxis::Vertical => {
            LayoutBox::new(track.x, track.y + thumb_offset, track.width, thumb_length)
        }
        ScrollbarAxis::Horizontal => {
            LayoutBox::new(track.x + thumb_offset, track.y, thumb_length, track.height)
        }
    };

    Some(ScrollbarRects { track, thumb })
}

fn scrollbar_corner(node: &RenderNode, thickness: f32) -> Option<LayoutBox> {
    let scrollbars = node.scrollbars?;
    if !scrollbars.shows_horizontal() || !scrollbars.shows_vertical() || thickness <= f32::EPSILON {
        return None;
    }

    let inner = inset_layout(node.layout, node.style.border.widths);
    Some(LayoutBox::new(
        inner.x + (inner.width - thickness).max(0.0),
        inner.y + (inner.height - thickness).max(0.0),
        thickness.min(inner.width),
        thickness.min(inner.height),
    ))
}

fn page_scroll(node: &mut RenderNode, hit: &ScrollbarHit, mouse_x: f32, mouse_y: f32) {
    let Some(rects) = scrollbar_rects(node, hit.axis) else {
        return;
    };
    let Some(scrollbars) = node.scrollbars else {
        return;
    };

    let mouse = axis_coordinate(mouse_x, mouse_y, hit.axis);
    let thumb_start = axis_start(rects.thumb, hit.axis);
    let thumb_end = thumb_start + axis_length(rects.thumb, hit.axis);
    let direction = if mouse < thumb_start {
        -1.0
    } else if mouse > thumb_end {
        1.0
    } else {
        0.0
    };
    if direction == 0.0 {
        return;
    }

    let page = axis_length(rects.track, hit.axis) * 0.9;
    let next_offset = axis_offset(scrollbars, hit.axis) + page * direction;
    match hit.axis {
        ScrollbarAxis::Horizontal => {
            let _ = apply_track_offset(node, hit.axis, next_offset);
        }
        ScrollbarAxis::Vertical => {
            let _ = apply_track_offset(node, hit.axis, next_offset);
        }
    }
}

fn apply_track_offset(node: &mut RenderNode, axis: ScrollbarAxis, next_offset: f32) -> bool {
    let Some(scrollbars) = node.scrollbars.as_mut() else {
        return false;
    };
    let previous_x = scrollbars.metrics.offset_x;
    let previous_y = scrollbars.metrics.offset_y;
    match axis {
        ScrollbarAxis::Horizontal => scrollbars.metrics.offset_x = next_offset,
        ScrollbarAxis::Vertical => scrollbars.metrics.offset_y = next_offset,
    }
    scrollbars.clamp_offsets();
    let delta_x = previous_x - scrollbars.metrics.offset_x;
    let delta_y = previous_y - scrollbars.metrics.offset_y;
    if delta_x.abs() <= f32::EPSILON && delta_y.abs() <= f32::EPSILON {
        return false;
    }

    for child in &mut node.children {
        translate_subtree(child, delta_x, delta_y);
    }
    true
}

fn translate_subtree(node: &mut RenderNode, dx: f32, dy: f32) {
    node.layout.x += dx;
    node.layout.y += dy;
    for child in &mut node.children {
        translate_subtree(child, dx, dy);
    }
}

fn mark_hover(node: &mut RenderNode, axis: ScrollbarAxis, part: ScrollbarPart) {
    let Some(scrollbars) = node.scrollbars.as_mut() else {
        return;
    };

    let state = match axis {
        ScrollbarAxis::Horizontal => &mut scrollbars.interaction.horizontal,
        ScrollbarAxis::Vertical => &mut scrollbars.interaction.vertical,
    };
    match part {
        ScrollbarPart::Track => state.track_hovered = true,
        ScrollbarPart::Thumb => state.thumb_hovered = true,
    }
}

fn mark_active(node: &mut RenderNode, axis: ScrollbarAxis) {
    let Some(scrollbars) = node.scrollbars.as_mut() else {
        return;
    };

    let state = match axis {
        ScrollbarAxis::Horizontal => &mut scrollbars.interaction.horizontal,
        ScrollbarAxis::Vertical => &mut scrollbars.interaction.vertical,
    };
    state.thumb_hovered = true;
    state.thumb_active = true;
}

fn node_mut_at_path<'a>(nodes: &'a mut [RenderNode], path: &[usize]) -> Option<&'a mut RenderNode> {
    let (&index, rest) = path.split_first()?;
    let node = nodes.get_mut(index)?;
    if rest.is_empty() {
        Some(node)
    } else {
        node_mut_at_path(&mut node.children, rest)
    }
}

fn axis_coordinate(x: f32, y: f32, axis: ScrollbarAxis) -> f32 {
    match axis {
        ScrollbarAxis::Horizontal => x,
        ScrollbarAxis::Vertical => y,
    }
}

fn axis_length(layout: LayoutBox, axis: ScrollbarAxis) -> f32 {
    match axis {
        ScrollbarAxis::Horizontal => layout.width,
        ScrollbarAxis::Vertical => layout.height,
    }
}

fn axis_start(layout: LayoutBox, axis: ScrollbarAxis) -> f32 {
    match axis {
        ScrollbarAxis::Horizontal => layout.x,
        ScrollbarAxis::Vertical => layout.y,
    }
}

fn axis_offset(scrollbars: cssimpler_core::ScrollbarData, axis: ScrollbarAxis) -> f32 {
    match axis {
        ScrollbarAxis::Horizontal => scrollbars.metrics.offset_x,
        ScrollbarAxis::Vertical => scrollbars.metrics.offset_y,
    }
}

fn axis_max_offset(scrollbars: cssimpler_core::ScrollbarData, axis: ScrollbarAxis) -> f32 {
    match axis {
        ScrollbarAxis::Horizontal => scrollbars.metrics.max_offset_x,
        ScrollbarAxis::Vertical => scrollbars.metrics.max_offset_y,
    }
}

fn scrollbars_can_auto_scroll(scrollbars: cssimpler_core::ScrollbarData) -> bool {
    (scrollbars.overflow_x.allows_scrolling() && scrollbars.metrics.max_offset_x > 0.0)
        || (scrollbars.overflow_y.allows_scrolling() && scrollbars.metrics.max_offset_y > 0.0)
}

fn auto_scroll_delta(displacement: f32, frame_seconds: f32) -> f32 {
    let magnitude = displacement.abs();
    if magnitude <= AUTO_SCROLL_DEADZONE || frame_seconds <= f32::EPSILON {
        return 0.0;
    }

    let direction = displacement.signum();
    let speed = (magnitude - AUTO_SCROLL_DEADZONE) * AUTO_SCROLL_SPEED_PER_PIXEL;
    direction * speed * frame_seconds
}

fn draw_indicator_arrow(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    center_x: f32,
    center_y: f32,
    direction: AutoScrollDirection,
    color: Color,
) {
    let ((tip_x, tip_y), (left_x, left_y), (right_x, right_y)) = match direction {
        AutoScrollDirection::Up => (
            (center_x, center_y - 2.5),
            (center_x - 3.5, center_y + 1.0),
            (center_x + 3.5, center_y + 1.0),
        ),
        AutoScrollDirection::Down => (
            (center_x, center_y + 2.5),
            (center_x - 3.5, center_y - 1.0),
            (center_x + 3.5, center_y - 1.0),
        ),
        AutoScrollDirection::Left => (
            (center_x - 2.5, center_y),
            (center_x + 1.0, center_y - 3.5),
            (center_x + 1.0, center_y + 3.5),
        ),
        AutoScrollDirection::Right => (
            (center_x + 2.5, center_y),
            (center_x - 1.0, center_y - 3.5),
            (center_x - 1.0, center_y + 3.5),
        ),
    };

    draw_line_segment(buffer, width, height, left_x, left_y, tip_x, tip_y, color);
    draw_line_segment(buffer, width, height, tip_x, tip_y, right_x, right_y, color);
}

fn draw_line_segment(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    start_x: f32,
    start_y: f32,
    end_x: f32,
    end_y: f32,
    color: Color,
) {
    let delta_x = end_x - start_x;
    let delta_y = end_y - start_y;
    let steps = delta_x.abs().max(delta_y.abs()).ceil() as i32;
    let prepared_color = PreparedBlendColor::new(color);
    if steps <= 0 {
        blend_prepared_pixel(
            buffer,
            width,
            height,
            start_x.round() as i32,
            start_y.round() as i32,
            prepared_color,
        );
        return;
    }

    for step in 0..=steps {
        let t = step as f32 / steps as f32;
        let x = start_x + delta_x * t;
        let y = start_y + delta_y * t;
        for offset_y in -1_i32..=1 {
            for offset_x in -1_i32..=1 {
                if offset_x.abs() + offset_y.abs() > 1 {
                    continue;
                }
                blend_prepared_pixel(
                    buffer,
                    width,
                    height,
                    x.round() as i32 + offset_x,
                    y.round() as i32 + offset_y,
                    prepared_color,
                );
            }
        }
    }
}

#[derive(Clone, Copy)]
struct ResolvedScrollbarColors {
    track: Color,
    thumb: Color,
}

fn resolved_scrollbar_colors(
    node: &RenderNode,
    scrollbars: cssimpler_core::ScrollbarData,
) -> ResolvedScrollbarColors {
    let base_background = node.style.background.unwrap_or(Color::rgb(241, 245, 249));
    let track = scrollbars
        .style
        .track_color
        .unwrap_or_else(|| mix_color(base_background, node.style.foreground, 0.12).with_alpha(208));
    let thumb = scrollbars
        .style
        .thumb_color
        .unwrap_or_else(|| mix_color(base_background, node.style.foreground, 0.36).with_alpha(232));

    ResolvedScrollbarColors { track, thumb }
}

fn axis_thumb_color(base: Color, hovered: bool, active: bool) -> Color {
    if active {
        mix_color(base, Color::BLACK, 0.12)
    } else if hovered {
        mix_color(base, Color::WHITE, 0.08)
    } else {
        base
    }
}

fn mix_color(base: Color, accent: Color, amount: f32) -> Color {
    let mixed = base
        .to_linear_rgba()
        .lerp(accent.to_linear_rgba(), amount.clamp(0.0, 1.0));
    Color::from_linear_rgba(LinearRgba {
        a: base.a as f32 / 255.0,
        ..mixed
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use cssimpler_core::{
        LayoutBox, OverflowMode, ScrollbarData, ScrollbarMetrics, ScrollbarStyle,
    };

    use super::{
        AutoScrollIndicator, ScrollbarAxis, ScrollbarController, auto_scroll_delta,
        scrollbar_rects, text_layout,
    };

    #[test]
    fn text_layout_expands_by_scroll_extent_and_applies_offset() {
        let node =
            cssimpler_core::RenderNode::text(LayoutBox::new(10.0, 12.0, 100.0, 40.0), "demo")
                .with_content_inset(cssimpler_core::Insets::all(4.0))
                .with_scrollbars(ScrollbarData::new(
                    OverflowMode::Auto,
                    OverflowMode::Auto,
                    ScrollbarStyle::default(),
                    ScrollbarMetrics {
                        offset_x: 12.0,
                        offset_y: 6.0,
                        max_offset_x: 40.0,
                        max_offset_y: 10.0,
                        reserved_width: 0.0,
                        reserved_height: 0.0,
                    },
                ));

        let layout = text_layout(&node);

        assert_eq!(layout.x, 2.0);
        assert_eq!(layout.y, 10.0);
        assert_eq!(layout.width, 132.0);
        assert_eq!(layout.height, 42.0);
    }

    #[test]
    fn scrollbar_thumb_shrinks_as_content_grows() {
        let node = cssimpler_core::RenderNode::container(LayoutBox::new(0.0, 0.0, 120.0, 80.0))
            .with_scrollbars(ScrollbarData::new(
                OverflowMode::Hidden,
                OverflowMode::Scroll,
                ScrollbarStyle {
                    width: cssimpler_core::ScrollbarWidth::Px(12.0),
                    ..ScrollbarStyle::default()
                },
                ScrollbarMetrics {
                    max_offset_x: 0.0,
                    max_offset_y: 120.0,
                    reserved_width: 12.0,
                    reserved_height: 0.0,
                    ..ScrollbarMetrics::default()
                },
            ));

        let rects = scrollbar_rects(&node, ScrollbarAxis::Vertical).expect("vertical scrollbar");

        assert!(rects.thumb.height < rects.track.height);
        assert_eq!(rects.track.width, 12.0);
    }

    #[test]
    fn controller_applies_cached_offsets_to_children() {
        let mut controller = ScrollbarController::default();
        controller
            .offsets
            .insert(vec![0], super::CachedOffsets { x: 0.0, y: 20.0 });
        let mut scene = vec![
            cssimpler_core::RenderNode::container(LayoutBox::new(0.0, 0.0, 120.0, 80.0))
                .with_scrollbars(ScrollbarData::new(
                    OverflowMode::Hidden,
                    OverflowMode::Auto,
                    ScrollbarStyle::default(),
                    ScrollbarMetrics {
                        offset_x: 0.0,
                        offset_y: 0.0,
                        max_offset_x: 0.0,
                        max_offset_y: 60.0,
                        reserved_width: 0.0,
                        reserved_height: 0.0,
                    },
                ))
                .with_child(cssimpler_core::RenderNode::container(LayoutBox::new(
                    0.0, 0.0, 60.0, 24.0,
                ))),
        ];

        controller.apply_to_scene(&mut scene);

        assert_eq!(scene[0].children[0].layout.y, -20.0);
    }

    #[test]
    fn middle_button_auto_scroll_moves_the_targeted_scroll_container() {
        let mut controller = ScrollbarController::default();
        let mut scene = vec![
            cssimpler_core::RenderNode::container(LayoutBox::new(0.0, 0.0, 120.0, 80.0))
                .with_scrollbars(ScrollbarData::new(
                    OverflowMode::Hidden,
                    OverflowMode::Auto,
                    ScrollbarStyle::default(),
                    ScrollbarMetrics {
                        offset_x: 0.0,
                        offset_y: 0.0,
                        max_offset_x: 0.0,
                        max_offset_y: 200.0,
                        reserved_width: 0.0,
                        reserved_height: 0.0,
                    },
                ))
                .with_child(cssimpler_core::RenderNode::container(LayoutBox::new(
                    0.0, 0.0, 60.0, 24.0,
                ))),
        ];

        assert!(controller.toggle_middle_button_auto_scroll(&scene, Some((40.0, 20.0))));
        assert!(controller.step_middle_button_auto_scroll(
            &mut scene,
            Some((40.0, 40.0)),
            Duration::from_millis(16),
        ));
        assert!(
            scene[0]
                .scrollbars
                .is_some_and(|scrollbars| scrollbars.metrics.offset_y > 0.0)
        );
        assert!(scene[0].children[0].layout.y < 0.0);
    }

    #[test]
    fn middle_button_auto_scroll_uses_a_deadzone_around_the_anchor() {
        let delta = auto_scroll_delta(6.0, 0.016);

        assert_eq!(delta, 0.0);
    }

    #[test]
    fn middle_button_auto_scroll_exposes_anchor_indicator() {
        let mut controller = ScrollbarController::default();
        let scene = vec![
            cssimpler_core::RenderNode::container(LayoutBox::new(0.0, 0.0, 120.0, 80.0))
                .with_scrollbars(ScrollbarData::new(
                    OverflowMode::Hidden,
                    OverflowMode::Auto,
                    ScrollbarStyle::default(),
                    ScrollbarMetrics {
                        offset_x: 0.0,
                        offset_y: 0.0,
                        max_offset_x: 0.0,
                        max_offset_y: 200.0,
                        reserved_width: 0.0,
                        reserved_height: 0.0,
                    },
                )),
        ];

        assert!(controller.toggle_middle_button_auto_scroll(&scene, Some((44.0, 28.0))));
        assert_eq!(
            controller.auto_scroll_indicator(),
            Some(AutoScrollIndicator { x: 44.0, y: 28.0 })
        );
        assert!(controller.cancel_middle_button_auto_scroll());
        assert_eq!(controller.auto_scroll_indicator(), None);
    }
}
