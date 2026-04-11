use std::time::Duration;

use crate::core::{
    Color, LayoutBox, LengthPercentageValue, LinearRgba, RenderNode, Transform2D,
    TransformMatrix3d, TransformOperation, TransformOrigin, TransitionPropertyName,
    TransitionStyle, TransitionTimingFunction,
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
    transform: Option<TransitionTimingPlan>,
    foreground: Option<TransitionTimingPlan>,
    background: Option<TransitionTimingPlan>,
    border_color: Option<TransitionTimingPlan>,
    has_sampling_work: bool,
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
    if inherited_layout_progress.is_none() && !plan.has_sampling_work {
        return;
    }

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

    if let Some(entry) = plan.transform
        && let Some(progress) = transition_progress(entry, elapsed_seconds)
    {
        sampled.style.transform = if progress <= f32::EPSILON {
            from.style.transform.clone()
        } else if progress >= 1.0 - f32::EPSILON {
            to.style.transform.clone()
        } else {
            interpolate_transform(
                &from.style.transform,
                &to.style.transform,
                sampled.layout,
                progress,
            )
        };
    } else {
        sampled.style.transform = to.style.transform.clone();
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
        if layout_progress.is_none() && !child_plan.has_sampling_work {
            continue;
        }

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
    let transform = (from.style.transform != to.style.transform)
        .then(|| first_transition_plan_for_property(&to.transitions, "transform"))
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
    for plan in [layout, transform, foreground, background, border_color]
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

    let has_sampling_work = layout.is_some()
        || transform.is_some()
        || foreground.is_some()
        || background.is_some()
        || border_color.is_some()
        || children.iter().any(|child| child.has_sampling_work);

    (
        TransitionPlanNode {
            layout,
            transform,
            foreground,
            background,
            border_color,
            has_sampling_work,
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

fn interpolate_transform(
    from: &Transform2D,
    to: &Transform2D,
    layout: LayoutBox,
    progress: f32,
) -> Transform2D {
    let origin = TransformOrigin {
        x: from.origin.x.lerp(to.origin.x, progress),
        y: from.origin.y.lerp(to.origin.y, progress),
    };
    let operations =
        interpolate_transform_operations(&from.operations, &to.operations, layout, progress);

    Transform2D { origin, operations }
}

fn interpolate_transform_operations(
    from: &[TransformOperation],
    to: &[TransformOperation],
    layout: LayoutBox,
    progress: f32,
) -> Vec<TransformOperation> {
    let count = from.len().max(to.len());
    let mut operations = Vec::with_capacity(count);

    for index in 0..count {
        let to_operation = to.get(index).copied();
        let from_operation = from
            .get(index)
            .copied()
            .or_else(|| to_operation.map(identity_transform_operation));
        let to_operation =
            to_operation.or_else(|| from_operation.map(identity_transform_operation));

        let (Some(from_operation), Some(to_operation)) = (from_operation, to_operation) else {
            continue;
        };

        let Some(operation) =
            interpolate_transform_operation(from_operation, to_operation, layout, progress)
        else {
            let from_matrix = transform_operations_matrix(layout, from);
            let to_matrix = transform_operations_matrix(layout, to);
            let matrix = lerp_transform_matrix(from_matrix, to_matrix, progress);
            return if matrix.is_identity() {
                Vec::new()
            } else {
                vec![TransformOperation::Matrix3d { matrix }]
            };
        };

        if !transform_operation_is_identity(operation) {
            operations.push(operation);
        }
    }

    operations
}

fn interpolate_transform_operation(
    from: TransformOperation,
    to: TransformOperation,
    _layout: LayoutBox,
    progress: f32,
) -> Option<TransformOperation> {
    match (from, to) {
        (
            TransformOperation::Translate {
                x: from_x,
                y: from_y,
            },
            TransformOperation::Translate { x: to_x, y: to_y },
        ) => Some(TransformOperation::Translate {
            x: from_x.lerp(to_x, progress),
            y: from_y.lerp(to_y, progress),
        }),
        (
            TransformOperation::TranslateZ { z: from_z },
            TransformOperation::TranslateZ { z: to_z },
        ) => Some(TransformOperation::TranslateZ {
            z: lerp(from_z, to_z, progress),
        }),
        (
            TransformOperation::Scale {
                x: from_x,
                y: from_y,
            },
            TransformOperation::Scale { x: to_x, y: to_y },
        ) => Some(TransformOperation::Scale {
            x: lerp(from_x, to_x, progress),
            y: lerp(from_y, to_y, progress),
        }),
        (
            TransformOperation::Rotate {
                degrees: from_degrees,
            },
            TransformOperation::Rotate {
                degrees: to_degrees,
            },
        ) => Some(TransformOperation::Rotate {
            degrees: lerp(from_degrees, to_degrees, progress),
        }),
        (
            TransformOperation::RotateX {
                degrees: from_degrees,
            },
            TransformOperation::RotateX {
                degrees: to_degrees,
            },
        ) => Some(TransformOperation::RotateX {
            degrees: lerp(from_degrees, to_degrees, progress),
        }),
        (
            TransformOperation::RotateY {
                degrees: from_degrees,
            },
            TransformOperation::RotateY {
                degrees: to_degrees,
            },
        ) => Some(TransformOperation::RotateY {
            degrees: lerp(from_degrees, to_degrees, progress),
        }),
        (
            TransformOperation::RotateZ {
                degrees: from_degrees,
            },
            TransformOperation::RotateZ {
                degrees: to_degrees,
            },
        ) => Some(TransformOperation::RotateZ {
            degrees: lerp(from_degrees, to_degrees, progress),
        }),
        (
            TransformOperation::Matrix3d {
                matrix: from_matrix,
            },
            TransformOperation::Matrix3d { matrix: to_matrix },
        ) => Some(TransformOperation::Matrix3d {
            matrix: lerp_transform_matrix(from_matrix, to_matrix, progress),
        }),
        _ => None,
    }
}

fn identity_transform_operation(operation: TransformOperation) -> TransformOperation {
    match operation {
        TransformOperation::Translate { .. } => TransformOperation::Translate {
            x: LengthPercentageValue::ZERO,
            y: LengthPercentageValue::ZERO,
        },
        TransformOperation::TranslateZ { .. } => TransformOperation::TranslateZ { z: 0.0 },
        TransformOperation::Scale { .. } => TransformOperation::Scale { x: 1.0, y: 1.0 },
        TransformOperation::Rotate { .. } => TransformOperation::Rotate { degrees: 0.0 },
        TransformOperation::RotateX { .. } => TransformOperation::RotateX { degrees: 0.0 },
        TransformOperation::RotateY { .. } => TransformOperation::RotateY { degrees: 0.0 },
        TransformOperation::RotateZ { .. } => TransformOperation::RotateZ { degrees: 0.0 },
        TransformOperation::Matrix3d { .. } => TransformOperation::Matrix3d {
            matrix: TransformMatrix3d::IDENTITY,
        },
    }
}

fn transform_operation_is_identity(operation: TransformOperation) -> bool {
    match operation {
        TransformOperation::Translate { x, y } => {
            x == LengthPercentageValue::ZERO && y == LengthPercentageValue::ZERO
        }
        TransformOperation::TranslateZ { z } => z.abs() <= f32::EPSILON,
        TransformOperation::Scale { x, y } => {
            (x - 1.0).abs() <= f32::EPSILON && (y - 1.0).abs() <= f32::EPSILON
        }
        TransformOperation::Rotate { degrees }
        | TransformOperation::RotateX { degrees }
        | TransformOperation::RotateY { degrees }
        | TransformOperation::RotateZ { degrees } => degrees.abs() <= f32::EPSILON,
        TransformOperation::Matrix3d { matrix } => matrix.is_identity(),
    }
}

fn transform_operations_matrix(
    layout: LayoutBox,
    operations: &[TransformOperation],
) -> TransformMatrix3d {
    operations
        .iter()
        .fold(TransformMatrix3d::IDENTITY, |matrix, operation| {
            transform_operation_matrix(layout, *operation).multiply(matrix)
        })
}

fn transform_operation_matrix(
    layout: LayoutBox,
    operation: TransformOperation,
) -> TransformMatrix3d {
    match operation {
        TransformOperation::Translate { x, y } => {
            TransformMatrix3d::translate(x.resolve(layout.width), y.resolve(layout.height), 0.0)
        }
        TransformOperation::TranslateZ { z } => TransformMatrix3d::translate(0.0, 0.0, z),
        TransformOperation::Scale { x, y } => TransformMatrix3d::scale(x, y, 1.0),
        TransformOperation::Rotate { degrees } | TransformOperation::RotateZ { degrees } => {
            TransformMatrix3d::rotate(0.0, 0.0, 1.0, degrees)
        }
        TransformOperation::RotateX { degrees } => {
            TransformMatrix3d::rotate(1.0, 0.0, 0.0, degrees)
        }
        TransformOperation::RotateY { degrees } => {
            TransformMatrix3d::rotate(0.0, 1.0, 0.0, degrees)
        }
        TransformOperation::Matrix3d { matrix } => matrix,
    }
}

fn lerp_transform_matrix(
    from: TransformMatrix3d,
    to: TransformMatrix3d,
    progress: f32,
) -> TransformMatrix3d {
    TransformMatrix3d {
        m11: lerp(from.m11, to.m11, progress),
        m12: lerp(from.m12, to.m12, progress),
        m13: lerp(from.m13, to.m13, progress),
        m14: lerp(from.m14, to.m14, progress),
        m21: lerp(from.m21, to.m21, progress),
        m22: lerp(from.m22, to.m22, progress),
        m23: lerp(from.m23, to.m23, progress),
        m24: lerp(from.m24, to.m24, progress),
        m31: lerp(from.m31, to.m31, progress),
        m32: lerp(from.m32, to.m32, progress),
        m33: lerp(from.m33, to.m33, progress),
        m34: lerp(from.m34, to.m34, progress),
        m41: lerp(from.m41, to.m41, progress),
        m42: lerp(from.m42, to.m42, progress),
        m43: lerp(from.m43, to.m43, progress),
        m44: lerp(from.m44, to.m44, progress),
    }
}

fn lerp(from: f32, to: f32, progress: f32) -> f32 {
    from + (to - from) * progress
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::core::{
        Color, LayoutBox, LengthPercentageValue, RenderNode, Transform2D, TransformOperation,
        TransitionPropertyName, TransitionStyle, TransitionTimingFunction, VisualStyle,
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
        let transition = layout_transition("width", 1.0, TransitionTimingFunction::Linear);
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
    fn layout_transitions_propagate_progress_to_descendants_without_child_plans() {
        let transition = layout_transition("width", 1.0, TransitionTimingFunction::Linear);
        let from = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 100.0, 40.0))
                .with_transitions(transition.clone())
                .with_child(text_node(
                    LayoutBox::new(10.0, 0.0, 50.0, 20.0),
                    Color::BLACK,
                    TransitionStyle::default(),
                )),
        ];
        let to = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 200.0, 40.0))
                .with_transitions(transition)
                .with_child(text_node(
                    LayoutBox::new(20.0, 0.0, 100.0, 20.0),
                    Color::BLACK,
                    TransitionStyle::default(),
                )),
        ];

        let mut scene_transition =
            SceneTransition::new(from, to).expect("layout transition should exist");
        let mut scene = scene_transition.sample();
        scene_transition.advance(Duration::from_millis(500), &mut scene);

        assert!((scene[0].layout.width - 150.0).abs() < 0.01);
        assert!((scene[0].children[0].layout.x - 15.0).abs() < 0.01);
        assert!((scene[0].children[0].layout.width - 75.0).abs() < 0.01);
    }

    #[test]
    fn transform_transition_creates_animation_state_and_interpolates_midpoint() {
        let from = vec![transform_node(
            LayoutBox::new(0.0, 0.0, 100.0, 100.0),
            Transform2D::default(),
            transform_transition(1.0, TransitionTimingFunction::Linear),
        )];
        let to = vec![transform_node(
            LayoutBox::new(0.0, 0.0, 100.0, 100.0),
            Transform2D {
                operations: vec![TransformOperation::Translate {
                    x: LengthPercentageValue::from_px(20.0),
                    y: LengthPercentageValue::from_px(10.0),
                }],
                ..Transform2D::default()
            },
            transform_transition(1.0, TransitionTimingFunction::Linear),
        )];

        let mut transition =
            SceneTransition::new(from, to).expect("transform transition should exist");
        let mut scene = transition.sample();
        transition.advance(Duration::from_millis(500), &mut scene);

        assert_eq!(
            scene[0].style.transform.operations,
            vec![TransformOperation::Translate {
                x: LengthPercentageValue::from_px(10.0),
                y: LengthPercentageValue::from_px(5.0),
            }]
        );
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

    fn transform_node(
        layout: LayoutBox,
        transform: Transform2D,
        transitions: TransitionStyle,
    ) -> RenderNode {
        RenderNode::container(layout)
            .with_style(VisualStyle {
                transform,
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

    fn transform_transition(
        duration_seconds: f32,
        timing_function: TransitionTimingFunction,
    ) -> TransitionStyle {
        layout_transition("transform", duration_seconds, timing_function)
    }

    fn layout_transition(
        property_name: &str,
        duration_seconds: f32,
        timing_function: TransitionTimingFunction,
    ) -> TransitionStyle {
        TransitionStyle {
            properties: vec![TransitionPropertyName::Property(property_name.to_string())],
            durations_seconds: vec![duration_seconds],
            delays_seconds: vec![0.0],
            timing_functions: vec![timing_function],
        }
    }
}
