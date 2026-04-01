use std::time::Duration;

use crate::core::{
    Color, LinearRgba, RenderNode, TransitionEntry, TransitionPropertyName,
    TransitionTimingFunction,
};

#[derive(Clone)]
pub(crate) struct SceneTransition {
    from: Vec<RenderNode>,
    to: Vec<RenderNode>,
    elapsed_seconds: f32,
    duration_seconds: f32,
}

impl SceneTransition {
    pub(crate) fn new(from: Vec<RenderNode>, to: Vec<RenderNode>) -> Option<Self> {
        if !scene_structures_match(&from, &to) {
            return None;
        }

        let duration_seconds = max_scene_transition_duration(&from, &to);
        if duration_seconds <= f32::EPSILON {
            return None;
        }

        Some(Self {
            from,
            to,
            elapsed_seconds: 0.0,
            duration_seconds,
        })
    }

    pub(crate) fn sample(&self) -> Vec<RenderNode> {
        self.from
            .iter()
            .zip(&self.to)
            .map(|(from, to)| sample_render_node(from, to, self.elapsed_seconds, None))
            .collect()
    }

    pub(crate) fn advance(&mut self, delta: Duration) -> Vec<RenderNode> {
        self.elapsed_seconds =
            (self.elapsed_seconds + delta.as_secs_f32()).min(self.duration_seconds);
        self.sample()
    }

    pub(crate) fn is_active(&self) -> bool {
        self.elapsed_seconds + f32::EPSILON < self.duration_seconds
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
    from.iter()
        .zip(to)
        .map(|(from, to)| max_render_node_transition_duration(from, to))
        .fold(0.0_f32, f32::max)
}

fn max_render_node_transition_duration(from: &RenderNode, to: &RenderNode) -> f32 {
    let own_duration = transition_end_time_for_node(from, to);
    let child_duration = from
        .children
        .iter()
        .zip(&to.children)
        .map(|(from, to)| max_render_node_transition_duration(from, to))
        .fold(0.0_f32, f32::max);
    own_duration.max(child_duration)
}

fn transition_end_time_for_node(from: &RenderNode, to: &RenderNode) -> f32 {
    let mut max_duration = 0.0_f32;

    for property in ["color", "background-color", "background", "border-color"] {
        if let Some(entry) = matching_transition_entry(to, property)
            && property_transition_changes(from, to, property)
            && transition_entry_is_animating(&entry)
        {
            max_duration = max_duration.max(entry.delay_seconds + entry.duration_seconds);
        }
    }

    if layout_transition_for_node(from, to).is_some() {
        max_duration = max_duration.max(
            transition_entries_for(to)
                .into_iter()
                .filter(|entry| {
                    is_layout_transition_name(&entry.property) && transition_entry_is_animating(entry)
                })
                .map(|entry| entry.delay_seconds + entry.duration_seconds)
                .fold(0.0_f32, f32::max),
        );
    }

    max_duration
}

fn property_transition_changes(from: &RenderNode, to: &RenderNode, property: &str) -> bool {
    match property {
        "color" => from.style.foreground != to.style.foreground,
        "background-color" | "background" => from.style.background != to.style.background,
        "border-color" => from.style.border.color != to.style.border.color,
        _ => false,
    }
}

fn sample_render_node(
    from: &RenderNode,
    to: &RenderNode,
    elapsed_seconds: f32,
    inherited_layout_progress: Option<f32>,
) -> RenderNode {
    let mut node = to.clone();
    let layout_progress = layout_transition_for_node(from, to)
        .and_then(|entry| transition_progress(&entry, elapsed_seconds))
        .or(inherited_layout_progress);

    if let Some(progress) = layout_progress {
        node.layout.x = lerp(from.layout.x, to.layout.x, progress);
        node.layout.y = lerp(from.layout.y, to.layout.y, progress);
        node.layout.width = lerp(from.layout.width, to.layout.width, progress);
        node.layout.height = lerp(from.layout.height, to.layout.height, progress);
    }

    if let Some(entry) = matching_transition_entry(to, "color")
        && let Some(progress) = transition_progress(&entry, elapsed_seconds)
        && from.style.foreground != to.style.foreground
    {
        node.style.foreground = interpolate_color(from.style.foreground, to.style.foreground, progress);
    }

    if let Some(entry) = matching_transition_entry(to, "background-color")
        .or_else(|| matching_transition_entry(to, "background"))
        && let Some(progress) = transition_progress(&entry, elapsed_seconds)
        && from.style.background != to.style.background
    {
        node.style.background = if progress <= f32::EPSILON {
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
    }

    if let Some(entry) = matching_transition_entry(to, "border-color")
        && let Some(progress) = transition_progress(&entry, elapsed_seconds)
        && from.style.border.color != to.style.border.color
    {
        node.style.border.color = interpolate_color(from.style.border.color, to.style.border.color, progress);
    }

    node.children = from
        .children
        .iter()
        .zip(&to.children)
        .map(|(from, to)| sample_render_node(from, to, elapsed_seconds, layout_progress))
        .collect();
    node
}

fn transition_entries_for(node: &RenderNode) -> Vec<TransitionEntry> {
    node.transitions.entries()
}

fn matching_transition_entry(node: &RenderNode, property: &str) -> Option<TransitionEntry> {
    transition_entries_for(node)
        .into_iter()
        .find(|entry| transition_entry_matches(entry, property))
}

fn transition_entry_matches(entry: &TransitionEntry, property: &str) -> bool {
    match &entry.property {
        TransitionPropertyName::All => true,
        TransitionPropertyName::Property(name) => name.eq_ignore_ascii_case(property),
    }
}

fn transition_entry_is_animating(entry: &TransitionEntry) -> bool {
    entry.duration_seconds > f32::EPSILON
        && !matches!(entry.timing_function, TransitionTimingFunction::Unsupported)
}

fn layout_transition_for_node(from: &RenderNode, to: &RenderNode) -> Option<TransitionEntry> {
    if from.layout == to.layout {
        return None;
    }

    transition_entries_for(to)
        .into_iter()
        .find(|entry| {
            is_layout_transition_name(&entry.property) && transition_entry_is_animating(entry)
        })
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

fn transition_progress(entry: &TransitionEntry, elapsed_seconds: f32) -> Option<f32> {
    if !transition_entry_is_animating(entry) {
        return None;
    }

    if elapsed_seconds <= entry.delay_seconds {
        return Some(0.0);
    }
    if elapsed_seconds >= entry.delay_seconds + entry.duration_seconds {
        return Some(1.0);
    }

    let progress = ((elapsed_seconds - entry.delay_seconds) / entry.duration_seconds).clamp(0.0, 1.0);
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
    let from = from.map(Color::to_linear_rgba).unwrap_or(LinearRgba::TRANSPARENT);
    let to = to.map(Color::to_linear_rgba).unwrap_or(LinearRgba::TRANSPARENT);
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
        let half = transition.advance(Duration::from_millis(500));
        let current = half[0].style.foreground;

        assert_ne!(current, Color::BLACK);
        assert_ne!(current, Color::rgb(37, 99, 235));

        let final_scene = transition.advance(Duration::from_millis(500));
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
        let scene = scene_transition.advance(Duration::from_millis(500));

        assert!((scene[0].layout.width - 150.0).abs() < 0.01);
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
        RenderNode::text(layout, "label").with_style(VisualStyle {
            foreground,
            ..VisualStyle::default()
        })
        .with_transitions(transitions)
    }

    fn color_transition(duration_seconds: f32, timing_function: TransitionTimingFunction) -> TransitionStyle {
        TransitionStyle {
            properties: vec![TransitionPropertyName::Property("color".to_string())],
            durations_seconds: vec![duration_seconds],
            delays_seconds: vec![0.0],
            timing_functions: vec![timing_function],
        }
    }
}
