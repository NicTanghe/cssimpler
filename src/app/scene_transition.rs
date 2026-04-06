use std::time::Duration;

use crate::core::{
    Color, LinearRgba, RenderNode, TransitionPropertyName, TransitionStyle,
    TransitionTimingFunction,
};

#[derive(Clone)]
pub(crate) struct SceneTransition {
    from: Vec<RenderNode>,
    to: Vec<RenderNode>,
    plans: Vec<TransitionPlanNode>,
    elapsed_seconds: f32,
    duration_seconds: f32,
}

#[derive(Clone, Debug, Default)]
struct TransitionPlanNode {
    layout: Option<TransitionTimingPlan>,
    foreground: Option<TransitionTimingPlan>,
    background: Option<TransitionTimingPlan>,
    border_color: Option<TransitionTimingPlan>,
    children: Vec<TransitionPlanNode>,
}

#[derive(Clone, Copy, Debug)]
struct TransitionTimingPlan {
    duration_seconds: f32,
    delay_seconds: f32,
    timing_function: TransitionTimingFunction,
}

impl TransitionTimingPlan {
    fn end_time(self) -> f32 {
        self.delay_seconds + self.duration_seconds
    }
}

impl SceneTransition {
    pub(crate) fn should_create(from: &[RenderNode], to: &[RenderNode]) -> bool {
        if !scene_structures_match(from, to) {
            return false;
        }

        max_scene_transition_duration(from, to) > f32::EPSILON
    }

    pub(crate) fn new(from: Vec<RenderNode>, to: Vec<RenderNode>) -> Option<Self> {
        if !Self::should_create(&from, &to) {
            return None;
        }

        let (plans, duration_seconds) = build_scene_transition_plan(&from, &to);

        Some(Self {
            from,
            to,
            plans,
            elapsed_seconds: 0.0,
            duration_seconds,
        })
    }

    pub(crate) fn sample(&self) -> Vec<RenderNode> {
        let mut sampled = self.to.clone();
        self.sample_into(&mut sampled);
        sampled
    }

    pub(crate) fn advance(&mut self, delta: Duration, sampled: &mut [RenderNode]) {
        self.elapsed_seconds =
            (self.elapsed_seconds + delta.as_secs_f32()).min(self.duration_seconds);
        self.sample_into(sampled);
    }

    pub(crate) fn is_active(&self) -> bool {
        self.elapsed_seconds + f32::EPSILON < self.duration_seconds
    }

    fn sample_into(&self, sampled: &mut [RenderNode]) {
        for (((sampled_node, from), to), plan) in sampled
            .iter_mut()
            .zip(&self.from)
            .zip(&self.to)
            .zip(&self.plans)
        {
            sample_render_node_in_place(sampled_node, from, to, plan, self.elapsed_seconds, None);
        }
    }
}

fn scene_structures_match(left: &[RenderNode], right: &[RenderNode]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| render_node_structure_matches(left, right))
}

fn render_node_structure_matches(left: &RenderNode, right: &RenderNode) -> bool {
    std::mem::discriminant(&left.kind) == std::mem::discriminant(&right.kind)
        && left.children.len() == right.children.len()
        && left
            .children
            .iter()
            .zip(&right.children)
            .all(|(left, right)| render_node_structure_matches(left, right))
}

fn max_scene_transition_duration(from: &[RenderNode], to: &[RenderNode]) -> f32 {
    build_scene_transition_plan(from, to).1
}

fn sample_render_node_in_place(
    sampled: &mut RenderNode,
    from: &RenderNode,
    to: &RenderNode,
    plan: &TransitionPlanNode,
    elapsed_seconds: f32,
    inherited_layout_progress: Option<f32>,
) {
    let layout_progress = plan
        .layout
        .and_then(|entry| transition_progress(entry, elapsed_seconds))
        .or(inherited_layout_progress);

    if let Some(progress) = layout_progress {
        sampled.layout.x = lerp(from.layout.x, to.layout.x, progress);
        sampled.layout.y = lerp(from.layout.y, to.layout.y, progress);
        sampled.layout.width = lerp(from.layout.width, to.layout.width, progress);
        sampled.layout.height = lerp(from.layout.height, to.layout.height, progress);
    } else {
        sampled.layout = to.layout;
    }

    if let Some(entry) = plan.foreground
        && let Some(progress) = transition_progress(entry, elapsed_seconds)
    {
        sampled.style.foreground =
            interpolate_color(from.style.foreground, to.style.foreground, progress);
    } else {
        sampled.style.foreground = to.style.foreground;
    }

    if let Some(entry) = plan.background
        && let Some(progress) = transition_progress(entry, elapsed_seconds)
    {
        sampled.style.background = if progress <= f32::EPSILON {
            from.style.background
        } else if progress >= 1.0 - f32::EPSILON {
            to.style.background
        } else {
            Some(interpolate_optional_color(
                from.style.background,
                to.style.background,
                progress,
            ))
        };
    } else {
        sampled.style.background = to.style.background;
    }

    if let Some(entry) = plan.border_color
        && let Some(progress) = transition_progress(entry, elapsed_seconds)
    {
        sampled.style.border.color =
            interpolate_color(from.style.border.color, to.style.border.color, progress);
    } else {
        sampled.style.border.color = to.style.border.color;
    }

    for (((sampled_child, from_child), to_child), child_plan) in sampled
        .children
        .iter_mut()
        .zip(&from.children)
        .zip(&to.children)
        .zip(&plan.children)
    {
        sample_render_node_in_place(
            sampled_child,
            from_child,
            to_child,
            child_plan,
            elapsed_seconds,
            layout_progress,
        );
    }
}

fn build_scene_transition_plan(
    from: &[RenderNode],
    to: &[RenderNode],
) -> (Vec<TransitionPlanNode>, f32) {
    let mut max_duration = 0.0_f32;
    let plans = from
        .iter()
        .zip(to)
        .map(|(from, to)| {
            let (plan, duration) = build_transition_plan_node(from, to);
            max_duration = max_duration.max(duration);
            plan
        })
        .collect();
    (plans, max_duration)
}

fn build_transition_plan_node(from: &RenderNode, to: &RenderNode) -> (TransitionPlanNode, f32) {
    let layout = (from.layout != to.layout)
        .then(|| first_layout_transition_plan(&to.transitions))
        .flatten();
    let foreground = (from.style.foreground != to.style.foreground)
        .then(|| first_transition_plan_for_property(&to.transitions, "color"))
        .flatten();
    let background = (from.style.background != to.style.background)
        .then(|| {
            first_transition_plan_for_property(&to.transitions, "background-color")
                .or_else(|| first_transition_plan_for_property(&to.transitions, "background"))
        })
        .flatten();
    let border_color = (from.style.border.color != to.style.border.color)
        .then(|| first_transition_plan_for_property(&to.transitions, "border-color"))
        .flatten();

    let mut max_duration = 0.0_f32;
    for plan in [layout, foreground, background, border_color]
        .into_iter()
        .flatten()
    {
        max_duration = max_duration.max(plan.end_time());
    }

    let mut children = Vec::with_capacity(from.children.len());
    for (from_child, to_child) in from.children.iter().zip(&to.children) {
        let (child_plan, child_duration) = build_transition_plan_node(from_child, to_child);
        children.push(child_plan);
        max_duration = max_duration.max(child_duration);
    }

    (
        TransitionPlanNode {
            layout,
            foreground,
            background,
            border_color,
            children,
        },
        max_duration,
    )
}

fn first_transition_plan_for_property(
    style: &TransitionStyle,
    property: &str,
) -> Option<TransitionTimingPlan> {
    iter_transition_plans(style)
        .find(|(entry_property, plan)| {
            transition_property_matches(entry_property, property)
                && transition_plan_is_animating(*plan)
        })
        .map(|(_, plan)| plan)
}

fn first_layout_transition_plan(style: &TransitionStyle) -> Option<TransitionTimingPlan> {
    iter_transition_plans(style)
        .find(|(property, plan)| {
            is_layout_transition_name(property) && transition_plan_is_animating(*plan)
        })
        .map(|(_, plan)| plan)
}

fn iter_transition_plans(
    style: &TransitionStyle,
) -> impl Iterator<Item = (&TransitionPropertyName, TransitionTimingPlan)> {
    let duration_count = style.durations_seconds.len().max(1);
    let delay_count = style.delays_seconds.len().max(1);
    let timing_count = style.timing_functions.len().max(1);

    style
        .properties
        .iter()
        .enumerate()
        .map(move |(index, property)| {
            let duration_seconds = style
                .durations_seconds
                .get(index % duration_count)
                .copied()
                .unwrap_or(0.0);
            let delay_seconds = style
                .delays_seconds
                .get(index % delay_count)
                .copied()
                .unwrap_or(0.0);
            let timing_function = style
                .timing_functions
                .get(index % timing_count)
                .copied()
                .unwrap_or(TransitionTimingFunction::Ease);

            (
                property,
                TransitionTimingPlan {
                    duration_seconds,
                    delay_seconds,
                    timing_function,
                },
            )
        })
}

fn transition_property_matches(entry_property: &TransitionPropertyName, property: &str) -> bool {
    match entry_property {
        TransitionPropertyName::All => true,
        TransitionPropertyName::Property(name) => name.eq_ignore_ascii_case(property),
    }
}

fn transition_plan_is_animating(entry: TransitionTimingPlan) -> bool {
    entry.duration_seconds > f32::EPSILON
        && !matches!(entry.timing_function, TransitionTimingFunction::Unsupported)
}

fn is_layout_transition_name(property: &TransitionPropertyName) -> bool {
    match property {
        TransitionPropertyName::All => true,
        TransitionPropertyName::Property(name) => matches!(
            name.as_str(),
            "width"
                | "height"
                | "top"
                | "right"
                | "bottom"
                | "left"
                | "margin"
                | "margin-top"
                | "margin-right"
                | "margin-bottom"
                | "margin-left"
                | "padding"
                | "padding-top"
                | "padding-right"
                | "padding-bottom"
                | "padding-left"
                | "flex-basis"
                | "gap"
                | "row-gap"
                | "column-gap"
        ),
    }
}

fn transition_progress(entry: TransitionTimingPlan, elapsed_seconds: f32) -> Option<f32> {
    if !transition_plan_is_animating(entry) {
        return None;
    }

    if elapsed_seconds <= entry.delay_seconds {
        return Some(0.0);
    }
    if elapsed_seconds >= entry.delay_seconds + entry.duration_seconds {
        return Some(1.0);
    }

    let progress =
        ((elapsed_seconds - entry.delay_seconds) / entry.duration_seconds).clamp(0.0, 1.0);
    Some(apply_timing_function(entry.timing_function, progress))
}

fn apply_timing_function(function: TransitionTimingFunction, progress: f32) -> f32 {
    match function {
        TransitionTimingFunction::Linear => progress,
        TransitionTimingFunction::Ease => cubic_bezier(progress, 0.25, 0.1, 0.25, 1.0),
        TransitionTimingFunction::EaseIn => cubic_bezier(progress, 0.42, 0.0, 1.0, 1.0),
        TransitionTimingFunction::EaseOut => cubic_bezier(progress, 0.0, 0.0, 0.58, 1.0),
        TransitionTimingFunction::EaseInOut => cubic_bezier(progress, 0.42, 0.0, 0.58, 1.0),
        TransitionTimingFunction::Unsupported => 1.0,
    }
}

fn cubic_bezier(progress: f32, x1: f32, y1: f32, x2: f32, y2: f32) -> f32 {
    let progress = progress.clamp(0.0, 1.0);
    let mut lower = 0.0;
    let mut upper = 1.0;
    let mut t = progress;
    for _ in 0..10 {
        t = (lower + upper) * 0.5;
        let x = cubic_curve(t, x1, x2);
        if (x - progress).abs() <= 0.0005 {
            break;
        }
        if x < progress {
            lower = t;
        } else {
            upper = t;
        }
    }
    cubic_curve(t, y1, y2)
}

fn cubic_curve(t: f32, p1: f32, p2: f32) -> f32 {
    let inverse = 1.0 - t;
    3.0 * inverse * inverse * t * p1 + 3.0 * inverse * t * t * p2 + t * t * t
}

fn interpolate_optional_color(from: Option<Color>, to: Option<Color>, progress: f32) -> Color {
    let from = from
        .map(Color::to_linear_rgba)
        .unwrap_or(LinearRgba::TRANSPARENT);
    let to = to
        .map(Color::to_linear_rgba)
        .unwrap_or(LinearRgba::TRANSPARENT);
    Color::from_linear_rgba(from.lerp(to, progress))
}

fn interpolate_color(from: Color, to: Color, progress: f32) -> Color {
    Color::from_linear_rgba(from.to_linear_rgba().lerp(to.to_linear_rgba(), progress))
}

fn lerp(from: f32, to: f32, progress: f32) -> f32 {
    from + (to - from) * progress
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::core::{
        Color, LayoutBox, RenderNode, TransitionPropertyName, TransitionStyle,
        TransitionTimingFunction, VisualStyle,
    };

    use super::SceneTransition;

    #[test]
    fn scene_transition_interpolates_text_color() {
        let from = vec![text_node(
            LayoutBox::new(0.0, 0.0, 100.0, 20.0),
            Color::BLACK,
            TransitionStyle::default(),
        )];
        let to = vec![text_node(
            LayoutBox::new(0.0, 0.0, 100.0, 20.0),
            Color::rgb(37, 99, 235),
            color_transition(1.0, TransitionTimingFunction::Linear),
        )];

        let mut transition = SceneTransition::new(from, to).expect("color transition should exist");
        let mut half = transition.sample();
        transition.advance(Duration::from_millis(500), &mut half);
        let current = half[0].style.foreground;

        assert_ne!(current, Color::BLACK);
        assert_ne!(current, Color::rgb(37, 99, 235));

        transition.advance(Duration::from_millis(500), &mut half);
        let final_scene = half;
        assert_eq!(final_scene[0].style.foreground, Color::rgb(37, 99, 235));
        assert!(!transition.is_active());
    }

    #[test]
    fn scene_transition_interpolates_layout_for_supported_properties() {
        let transition = TransitionStyle {
            properties: vec![TransitionPropertyName::Property("width".to_string())],
            durations_seconds: vec![1.0],
            delays_seconds: vec![0.0],
            timing_functions: vec![TransitionTimingFunction::Linear],
        };
        let from = vec![text_node(
            LayoutBox::new(0.0, 0.0, 100.0, 20.0),
            Color::BLACK,
            TransitionStyle::default(),
        )];
        let to = vec![text_node(
            LayoutBox::new(0.0, 0.0, 200.0, 20.0),
            Color::BLACK,
            transition,
        )];

        let mut scene_transition =
            SceneTransition::new(from, to).expect("layout transition should exist");
        let mut scene = scene_transition.sample();
        scene_transition.advance(Duration::from_millis(500), &mut scene);

        assert!((scene[0].layout.width - 150.0).abs() < 0.01);
    }

    #[test]
    fn scene_transition_reuses_sampled_scene_storage_between_advances() {
        let from = vec![text_node(
            LayoutBox::new(0.0, 0.0, 100.0, 20.0),
            Color::BLACK,
            color_transition(1.0, TransitionTimingFunction::Linear),
        )];
        let to = vec![text_node(
            LayoutBox::new(0.0, 0.0, 100.0, 20.0),
            Color::rgb(37, 99, 235),
            color_transition(1.0, TransitionTimingFunction::Linear),
        )];

        let mut transition = SceneTransition::new(from, to).expect("color transition should exist");
        let mut sampled = transition.sample();
        let root_ptr = sampled.as_ptr();

        transition.advance(Duration::from_millis(250), &mut sampled);
        let first_sample = sampled[0].style.foreground;
        transition.advance(Duration::from_millis(250), &mut sampled);
        let second_sample = sampled[0].style.foreground;

        assert_eq!(sampled.as_ptr(), root_ptr);
        assert_ne!(first_sample, Color::BLACK);
        assert_ne!(second_sample, first_sample);
    }

    #[test]
    fn unsupported_transitions_snap_instead_of_creating_animation_state() {
        let from = vec![text_node(
            LayoutBox::new(0.0, 0.0, 100.0, 20.0),
            Color::BLACK,
            TransitionStyle::default(),
        )];
        let to = vec![text_node(
            LayoutBox::new(0.0, 0.0, 100.0, 20.0),
            Color::rgb(37, 99, 235),
            color_transition(1.0, TransitionTimingFunction::Unsupported),
        )];

        assert!(SceneTransition::new(from, to).is_none());
    }

    fn text_node(layout: LayoutBox, foreground: Color, transitions: TransitionStyle) -> RenderNode {
        RenderNode::text(layout, "label")
            .with_style(VisualStyle {
                foreground,
                ..VisualStyle::default()
            })
            .with_transitions(transitions)
    }

    fn color_transition(
        duration_seconds: f32,
        timing_function: TransitionTimingFunction,
    ) -> TransitionStyle {
        TransitionStyle {
            properties: vec![TransitionPropertyName::Property("color".to_string())],
            durations_seconds: vec![duration_seconds],
            delays_seconds: vec![0.0],
            timing_functions: vec![timing_function],
        }
    }
}
