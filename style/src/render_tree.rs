use cssimpler_core::{
    CustomProperties, ElementInteractionState, ElementNode, ElementPath, EventHandlers,
    LayoutBox, Node, RenderNode, ScrollbarData, Style, TransitionStyle,
    fonts::{TextStyle, layout_text_block},
};
use taffy::geometry::Size as TaffySize;
use taffy::prelude::{
    AvailableSpace, Dimension, LengthPercentage as TaffyLengthPercentage, NodeId,
    Style as TaffyStyle, TaffyTree,
};

use crate::{ElementRef, PseudoElementKind, Stylesheet};

#[derive(Clone, Debug)]
pub(crate) struct ResolvedElement {
    pub(crate) element_id: Option<String>,
    pub(crate) style: Style,
    pub(crate) text: String,
    pub(crate) element_path: ElementPath,
    pub(crate) handlers: EventHandlers,
    pub(crate) children: Vec<ResolvedElement>,
}

#[derive(Clone, Debug)]
struct LayoutTree {
    node_id: NodeId,
    element_id: Option<String>,
    style: Style,
    text: String,
    element_path: ElementPath,
    handlers: EventHandlers,
    children: Vec<LayoutTree>,
}

#[derive(Clone, Debug, Default)]
struct LeafMeasureContext {
    text: String,
    text_style: TextStyle,
}

pub fn build_render_tree(root: &Node, stylesheet: &Stylesheet) -> RenderNode {
    build_render_tree_with_interaction(root, stylesheet, &ElementInteractionState::default())
}

pub fn build_render_tree_with_interaction(
    root: &Node,
    stylesheet: &Stylesheet,
    interaction: &ElementInteractionState,
) -> RenderNode {
    build_render_tree_with_interaction_at_root(root, stylesheet, interaction, 0)
}

pub fn build_render_tree_with_interaction_at_root(
    root: &Node,
    stylesheet: &Stylesheet,
    interaction: &ElementInteractionState,
    root_index: usize,
) -> RenderNode {
    build_render_tree_with_available_space(root, stylesheet, None, interaction, root_index)
}

pub fn rebuild_render_tree_with_cached_layout(
    root: &Node,
    stylesheet: &Stylesheet,
    interaction: &ElementInteractionState,
    element_path: &ElementPath,
    template: &RenderNode,
) -> Option<RenderNode> {
    let Node::Element(root_element) = root else {
        return None;
    };
    let resolved = resolve_element_tree(
        root_element,
        stylesheet,
        None,
        None,
        &[],
        interaction,
        element_path,
    );
    render_node_with_cached_layout(&resolved, template)
}

pub fn build_render_tree_in_viewport(
    root: &Node,
    stylesheet: &Stylesheet,
    viewport_width: usize,
    viewport_height: usize,
) -> RenderNode {
    build_render_tree_in_viewport_with_interaction(
        root,
        stylesheet,
        viewport_width,
        viewport_height,
        &ElementInteractionState::default(),
    )
}

pub fn build_render_tree_in_viewport_with_interaction(
    root: &Node,
    stylesheet: &Stylesheet,
    viewport_width: usize,
    viewport_height: usize,
    interaction: &ElementInteractionState,
) -> RenderNode {
    build_render_tree_in_viewport_with_interaction_at_root(
        root,
        stylesheet,
        viewport_width,
        viewport_height,
        interaction,
        0,
    )
}

pub fn build_render_tree_in_viewport_with_interaction_at_root(
    root: &Node,
    stylesheet: &Stylesheet,
    viewport_width: usize,
    viewport_height: usize,
    interaction: &ElementInteractionState,
    root_index: usize,
) -> RenderNode {
    let viewport = TaffySize {
        width: AvailableSpace::Definite(viewport_width.max(1) as f32),
        height: AvailableSpace::Definite(viewport_height.max(1) as f32),
    };
    build_render_tree_with_available_space(
        root,
        stylesheet,
        Some(viewport),
        interaction,
        root_index,
    )
}

pub(crate) fn resolve_element_tree(
    element: &ElementNode,
    stylesheet: &Stylesheet,
    inherited_text: Option<&TextStyle>,
    inherited_custom_properties: Option<&CustomProperties>,
    ancestors: &[ElementRef<'_>],
    interaction: &ElementInteractionState,
    element_path: &ElementPath,
) -> ResolvedElement {
    let style = crate::resolve_style_target(
        element,
        stylesheet,
        element.style.clone(),
        inherited_text,
        inherited_custom_properties,
        ancestors,
        interaction,
        element_path,
        None,
    );
    let mut child_ancestors = Vec::with_capacity(ancestors.len() + 1);
    child_ancestors.push(ElementRef::from(element));
    child_ancestors.extend_from_slice(ancestors);
    let mut child_index = 0;
    let before = resolve_pseudo_element_tree(
        element,
        stylesheet,
        &style,
        &child_ancestors,
        interaction,
        element_path,
        PseudoElementKind::Before,
    );
    let after = resolve_pseudo_element_tree(
        element,
        stylesheet,
        &style,
        &child_ancestors,
        interaction,
        element_path,
        PseudoElementKind::After,
    );
    let has_element_children = element
        .children
        .iter()
        .any(|child| matches!(child, Node::Element(_)));
    let direct_text = direct_text_content(element);

    if before.is_none() && after.is_none() && !has_element_children {
        return ResolvedElement {
            element_id: element.id.clone(),
            style,
            text: direct_text,
            element_path: element_path.clone(),
            handlers: element.handlers,
            children: Vec::new(),
        };
    }

    let mut children = Vec::new();
    if let Some(before) = before {
        children.push(before);
    }

    let mut pending_text = String::new();
    for child in &element.children {
        match child {
            Node::Text(text) => pending_text.push_str(text),
            Node::Element(child) => {
                flush_text_child(&mut children, &mut pending_text, &style, element_path);
                let child_path = element_path.with_child(child_index);
                child_index += 1;
                children.push(resolve_element_tree(
                    child,
                    stylesheet,
                    Some(&style.visual.text),
                    Some(&style.custom_properties),
                    &child_ancestors,
                    interaction,
                    &child_path,
                ));
            }
        }
    }
    flush_text_child(&mut children, &mut pending_text, &style, element_path);

    if let Some(after) = after {
        children.push(after);
    }

    ResolvedElement {
        element_id: element.id.clone(),
        style,
        text: String::new(),
        element_path: element_path.clone(),
        handlers: element.handlers,
        children,
    }
}

fn build_render_tree_with_available_space(
    root: &Node,
    stylesheet: &Stylesheet,
    available_space_override: Option<TaffySize<AvailableSpace>>,
    interaction: &ElementInteractionState,
    root_index: usize,
) -> RenderNode {
    let Node::Element(root_element) = root else {
        panic!("render tree roots must be elements");
    };

    let resolved = resolve_element_tree(
        root_element,
        stylesheet,
        None,
        None,
        &[],
        interaction,
        &ElementPath::root(root_index),
    );
    let mut taffy = TaffyTree::<LeafMeasureContext>::new();
    let layout_tree = build_layout_tree(&resolved, &mut taffy);
    let available_space = available_space_override
        .unwrap_or_else(|| available_space_from_root(&layout_tree.style.layout.taffy));
    taffy
        .compute_layout_with_measure(
            layout_tree.node_id,
            available_space,
            |known_dimensions, available_space, _, context, _| {
                context.map_or(
                    TaffySize {
                        width: 0.0,
                        height: 0.0,
                    },
                    |context| {
                        measure_text(
                            &context.text,
                            &context.text_style,
                            known_dimensions,
                            available_space,
                        )
                    },
                )
            },
        )
        .expect("resolved layout should be valid for taffy");

    render_node_from_layout(&layout_tree, &taffy, 0.0, 0.0)
}

fn build_layout_tree(
    resolved: &ResolvedElement,
    taffy: &mut TaffyTree<LeafMeasureContext>,
) -> LayoutTree {
    let children: Vec<_> = resolved
        .children
        .iter()
        .map(|child| build_layout_tree(child, taffy))
        .collect();
    let child_ids: Vec<_> = children.iter().map(|child| child.node_id).collect();
    let node_id = if child_ids.is_empty() {
        taffy
            .new_leaf_with_context(
                crate::to_taffy(&resolved.style.layout),
                LeafMeasureContext {
                    text: resolved.text.clone(),
                    text_style: resolved.style.visual.text.clone(),
                },
            )
            .expect("leaf style should be accepted by taffy")
    } else {
        taffy
            .new_with_children(crate::to_taffy(&resolved.style.layout), &child_ids)
            .expect("container style should be accepted by taffy")
    };

    LayoutTree {
        node_id,
        element_id: resolved.element_id.clone(),
        style: resolved.style.clone(),
        text: resolved.text.clone(),
        element_path: resolved.element_path.clone(),
        handlers: resolved.handlers,
        children,
    }
}

fn available_space_from_root(style: &TaffyStyle) -> TaffySize<AvailableSpace> {
    TaffySize {
        width: available_space_from_dimension(style.size.width),
        height: available_space_from_dimension(style.size.height),
    }
}

fn available_space_from_dimension(dimension: Dimension) -> AvailableSpace {
    match dimension {
        Dimension::Length(value) => AvailableSpace::Definite(value),
        _ => AvailableSpace::MaxContent,
    }
}

fn render_node_from_layout(
    tree: &LayoutTree,
    taffy: &TaffyTree<LeafMeasureContext>,
    parent_x: f32,
    parent_y: f32,
) -> RenderNode {
    let layout = taffy
        .layout(tree.node_id)
        .expect("computed layouts should be readable");
    let x = parent_x + layout.location.x;
    let y = parent_y + layout.location.y;
    let layout_box = LayoutBox::new(x, y, layout.size.width, layout.size.height);
    let child_nodes: Vec<_> = tree
        .children
        .iter()
        .map(|child| render_node_from_layout(child, taffy, x, y))
        .collect();
    let content_inset = content_inset_from_taffy(&tree.style.layout.taffy);
    let scrollbars = crate::visual::scrollbars_from_layout(&tree.style, layout);

    if child_nodes.is_empty() && !tree.text.is_empty() {
        let mut node = RenderNode::text(layout_box, tree.text.clone())
            .with_style(tree.style.visual.clone())
            .with_transitions(tree.style.transitions.clone())
            .with_element_path(tree.element_path.clone())
            .with_content_inset(content_inset);
        if let Some(element_id) = &tree.element_id {
            node = node.with_element_id(element_id.clone());
        }
        if let Some(scrollbars) = scrollbars {
            node = node.with_scrollbars(scrollbars);
        }
        apply_layout_tree_handlers(node, tree)
    } else {
        let mut node = RenderNode::container(layout_box)
            .with_style(tree.style.visual.clone())
            .with_transitions(tree.style.transitions.clone())
            .with_element_path(tree.element_path.clone())
            .with_content_inset(content_inset)
            .with_children(child_nodes);
        if let Some(element_id) = &tree.element_id {
            node = node.with_element_id(element_id.clone());
        }
        if let Some(scrollbars) = scrollbars {
            node = node.with_scrollbars(scrollbars);
        }
        apply_layout_tree_handlers(node, tree)
    }
}

fn resolve_pseudo_element_tree(
    element: &ElementNode,
    stylesheet: &Stylesheet,
    inherited_style: &Style,
    ancestors: &[ElementRef<'_>],
    interaction: &ElementInteractionState,
    element_path: &ElementPath,
    pseudo_element: PseudoElementKind,
) -> Option<ResolvedElement> {
    let style = crate::resolve_style_target(
        element,
        stylesheet,
        Style::default(),
        Some(&inherited_style.visual.text),
        Some(&inherited_style.custom_properties),
        ancestors,
        interaction,
        element_path,
        Some(pseudo_element),
    );
    let text = match style.generated_text.as_ref() {
        Some(cssimpler_core::GeneratedTextSource::Literal(value)) => value.clone(),
        Some(cssimpler_core::GeneratedTextSource::Attribute(name)) => {
            element.attribute(name).unwrap_or_default().to_string()
        }
        None => return None,
    };

    Some(ResolvedElement {
        element_id: None,
        style,
        text,
        element_path: element_path.clone(),
        handlers: EventHandlers::default(),
        children: Vec::new(),
    })
}

fn direct_text_content(element: &ElementNode) -> String {
    let mut content = String::new();
    for child in &element.children {
        if let Node::Text(text) = child {
            content.push_str(text);
        }
    }
    content
}

fn flush_text_child(
    children: &mut Vec<ResolvedElement>,
    pending_text: &mut String,
    parent_style: &Style,
    element_path: &ElementPath,
) {
    if pending_text.is_empty() {
        return;
    }

    children.push(ResolvedElement {
        element_id: None,
        style: text_child_style(parent_style),
        text: std::mem::take(pending_text),
        element_path: element_path.clone(),
        handlers: EventHandlers::default(),
        children: Vec::new(),
    });
}

fn render_node_with_cached_layout(
    resolved: &ResolvedElement,
    template: &RenderNode,
) -> Option<RenderNode> {
    if resolved.element_id != template.element_id {
        return None;
    }
    if template.element_path.as_ref() != Some(&resolved.element_path) {
        return None;
    }

    let content_inset = content_inset_from_taffy(&resolved.style.layout.taffy);
    let scrollbars = scrollbars_with_cached_metrics(&resolved.style, template)?;

    if resolved.children.is_empty() && !resolved.text.is_empty() {
        if !matches!(template.kind, cssimpler_core::RenderKind::Text(_))
            || !template.children.is_empty()
        {
            return None;
        }

        let mut node = RenderNode::text(template.layout, resolved.text.clone())
            .with_style(resolved.style.visual.clone())
            .with_transitions(resolved.style.transitions.clone())
            .with_element_path(resolved.element_path.clone())
            .with_content_inset(content_inset);
        if let Some(element_id) = &resolved.element_id {
            node = node.with_element_id(element_id.clone());
        }
        if let Some(scrollbars) = scrollbars {
            node = node.with_scrollbars(scrollbars);
        }
        return Some(apply_resolved_handlers(node, resolved));
    }

    if !matches!(template.kind, cssimpler_core::RenderKind::Container) {
        return None;
    }
    if resolved.children.len() != template.children.len() {
        return None;
    }

    let children = resolved
        .children
        .iter()
        .zip(&template.children)
        .map(|(child, child_template)| render_node_with_cached_layout(child, child_template))
        .collect::<Option<Vec<_>>>()?;

    let mut node = RenderNode::container(template.layout)
        .with_style(resolved.style.visual.clone())
        .with_transitions(resolved.style.transitions.clone())
        .with_element_path(resolved.element_path.clone())
        .with_content_inset(content_inset)
        .with_children(children);
    if let Some(element_id) = &resolved.element_id {
        node = node.with_element_id(element_id.clone());
    }
    if let Some(scrollbars) = scrollbars {
        node = node.with_scrollbars(scrollbars);
    }
    Some(apply_resolved_handlers(node, resolved))
}

fn scrollbars_with_cached_metrics(
    style: &Style,
    template: &RenderNode,
) -> Option<Option<ScrollbarData>> {
    match (
        style.visual.overflow.allows_scrolling(),
        template.scrollbars,
    ) {
        (false, _) => Some(None),
        (true, Some(previous)) => Some(Some(ScrollbarData::new(
            style.visual.overflow.x,
            style.visual.overflow.y,
            style.visual.scrollbar,
            previous.metrics,
        ))),
        (true, None) => None,
    }
}

fn text_child_style(parent_style: &Style) -> Style {
    let mut style = Style {
        transitions: TransitionStyle::default(),
        ..Style::default()
    };
    style.visual.foreground = parent_style.visual.foreground;
    style.visual.text = parent_style.visual.text.clone();
    style.visual.text_stroke = parent_style.visual.text_stroke;
    style.visual.text_shadows = parent_style.visual.text_shadows.clone();
    style.visual.filter_drop_shadows = parent_style.visual.filter_drop_shadows.clone();
    style
}

fn apply_layout_tree_handlers(node: RenderNode, tree: &LayoutTree) -> RenderNode {
    node.with_handlers(tree.handlers)
}

fn apply_resolved_handlers(node: RenderNode, resolved: &ResolvedElement) -> RenderNode {
    node.with_handlers(resolved.handlers)
}

fn measure_text(
    text: &str,
    text_style: &TextStyle,
    known_dimensions: TaffySize<Option<f32>>,
    available_space: TaffySize<AvailableSpace>,
) -> TaffySize<f32> {
    if text.is_empty() {
        return TaffySize {
            width: 0.0,
            height: 0.0,
        };
    }

    let wrap_width = known_dimensions
        .width
        .or_else(|| match available_space.width {
            AvailableSpace::Definite(width) => Some(width.max(1.0)),
            AvailableSpace::MinContent | AvailableSpace::MaxContent => None,
        });
    let layout = layout_text_block(text, text_style, wrap_width);

    TaffySize {
        width: known_dimensions.width.unwrap_or(layout.width),
        height: known_dimensions.height.unwrap_or(layout.height),
    }
}

fn content_inset_from_taffy(style: &TaffyStyle) -> cssimpler_core::Insets {
    cssimpler_core::Insets {
        top: resolved_length(style.padding.top) + resolved_length(style.border.top),
        right: resolved_length(style.padding.right) + resolved_length(style.border.right),
        bottom: resolved_length(style.padding.bottom) + resolved_length(style.border.bottom),
        left: resolved_length(style.padding.left) + resolved_length(style.border.left),
    }
}

fn resolved_length(value: TaffyLengthPercentage) -> f32 {
    match value {
        TaffyLengthPercentage::Length(value) => value,
        TaffyLengthPercentage::Percent(_) => 0.0,
    }
}
