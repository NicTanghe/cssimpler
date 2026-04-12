use crate::{ElementNode, EventHandlers, IntoNode, Node};

/// Compile-time metadata derived from already-baked node descriptors.
///
/// This is a finalizer over structured data, not a parser. The intent is to let
/// proc-macro or build-step generated descriptors compute small metrics and flags
/// during compilation without relying on heap allocation or runtime-only logic.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StaticAttribute {
    pub name: &'static str,
    pub value: &'static str,
}

impl StaticAttribute {
    pub const fn new(name: &'static str, value: &'static str) -> Self {
        Self { name, value }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StaticTextRun {
    pub text: &'static str,
}

impl StaticTextRun {
    pub const fn new(text: &'static str) -> Self {
        Self { text }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct StaticElementNodeDesc {
    pub tag: &'static str,
    pub id: Option<&'static str>,
    pub classes: &'static [&'static str],
    pub attributes: &'static [StaticAttribute],
    pub children: &'static [StaticNodeDesc],
    pub handlers: EventHandlers,
}

impl StaticElementNodeDesc {
    pub const fn new(
        tag: &'static str,
        id: Option<&'static str>,
        classes: &'static [&'static str],
        attributes: &'static [StaticAttribute],
        children: &'static [StaticNodeDesc],
    ) -> Self {
        Self {
            tag,
            id,
            classes,
            attributes,
            children,
            handlers: EventHandlers::NONE,
        }
    }

    pub const fn with_handlers(mut self, handlers: EventHandlers) -> Self {
        self.handlers = handlers;
        self
    }

    pub fn to_node(&self) -> ElementNode {
        let mut node = ElementNode::new(self.tag);
        if let Some(id) = self.id {
            node = node.with_id(id);
        }
        for class_name in self.classes {
            node = node.with_class(*class_name);
        }
        for attribute in self.attributes {
            node = node.with_attribute(attribute.name, attribute.value);
        }
        node.handlers = self.handlers;
        for child in self.children {
            node = node.with_child(child.to_node());
        }
        node
    }
}

#[derive(Clone, Copy, Debug)]
pub enum StaticNodeDesc {
    Element(StaticElementNodeDesc),
    Text(StaticTextRun),
}

impl StaticNodeDesc {
    pub const fn element(element: StaticElementNodeDesc) -> Self {
        Self::Element(element)
    }

    pub const fn text(text: &'static str) -> Self {
        Self::Text(StaticTextRun::new(text))
    }

    pub fn to_node(&self) -> Node {
        match self {
            Self::Element(element) => Node::Element(element.to_node()),
            Self::Text(text) => Node::Text(text.text.to_string()),
        }
    }
}

impl IntoNode for StaticNodeDesc {
    fn into_node(self) -> Node {
        self.to_node()
    }
}

impl IntoNode for &StaticNodeDesc {
    fn into_node(self) -> Node {
        self.to_node()
    }
}

impl IntoNode for StaticTextRun {
    fn into_node(self) -> Node {
        Node::Text(self.text.to_string())
    }
}

impl IntoNode for &StaticTextRun {
    fn into_node(self) -> Node {
        Node::Text(self.text.to_string())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StaticPrefabFlags {
    pub interactive: bool,
    pub identified: bool,
    pub has_classes: bool,
    pub has_attributes: bool,
}

impl StaticPrefabFlags {
    pub const NONE: Self = Self {
        interactive: false,
        identified: false,
        has_classes: false,
        has_attributes: false,
    };

    const fn merge(self, other: Self) -> Self {
        Self {
            interactive: self.interactive || other.interactive,
            identified: self.identified || other.identified,
            has_classes: self.has_classes || other.has_classes,
            has_attributes: self.has_attributes || other.has_attributes,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StaticPrefabMetrics {
    pub node_count: usize,
    pub element_count: usize,
    pub text_count: usize,
    pub interactive_element_count: usize,
    pub max_depth: usize,
    pub flags: StaticPrefabFlags,
}

impl StaticPrefabMetrics {
    pub const EMPTY: Self = Self {
        node_count: 0,
        element_count: 0,
        text_count: 0,
        interactive_element_count: 0,
        max_depth: 0,
        flags: StaticPrefabFlags::NONE,
    };

    const fn merge(self, other: Self) -> Self {
        Self {
            node_count: self.node_count + other.node_count,
            element_count: self.element_count + other.element_count,
            text_count: self.text_count + other.text_count,
            interactive_element_count: self.interactive_element_count
                + other.interactive_element_count,
            max_depth: if self.max_depth >= other.max_depth {
                self.max_depth
            } else {
                other.max_depth
            },
            flags: self.flags.merge(other.flags),
        }
    }
}

/// Analyze a baked prefab in a compile-time context.
///
/// This helper intentionally operates on `StaticNodeDesc`. It does not parse raw
/// markup and it does not allocate a runtime `Node` tree.
pub const fn analyze_prefab(node: &StaticNodeDesc) -> StaticPrefabMetrics {
    analyze_prefab_at_depth(node, 1)
}

const fn analyze_prefab_at_depth(node: &StaticNodeDesc, depth: usize) -> StaticPrefabMetrics {
    match node {
        StaticNodeDesc::Element(element) => {
            let mut metrics = StaticPrefabMetrics {
                node_count: 1,
                element_count: 1,
                text_count: 0,
                interactive_element_count: if has_handlers(element.handlers) { 1 } else { 0 },
                max_depth: depth,
                flags: StaticPrefabFlags {
                    interactive: has_handlers(element.handlers),
                    identified: element.id.is_some(),
                    has_classes: !element.classes.is_empty(),
                    has_attributes: !element.attributes.is_empty(),
                },
            };
            let mut index = 0;
            while index < element.children.len() {
                metrics =
                    metrics.merge(analyze_prefab_at_depth(&element.children[index], depth + 1));
                index += 1;
            }
            metrics
        }
        StaticNodeDesc::Text(_) => StaticPrefabMetrics {
            node_count: 1,
            element_count: 0,
            text_count: 1,
            interactive_element_count: 0,
            max_depth: depth,
            flags: StaticPrefabFlags::NONE,
        },
    }
}

const fn has_handlers(handlers: EventHandlers) -> bool {
    handlers.click.is_some()
        || handlers.contextmenu.is_some()
        || handlers.dblclick.is_some()
        || handlers.mousedown.is_some()
        || handlers.mouseenter.is_some()
        || handlers.mouseleave.is_some()
        || handlers.mousemove.is_some()
        || handlers.mouseout.is_some()
        || handlers.mouseover.is_some()
        || handlers.mouseup.is_some()
}

#[cfg(test)]
mod tests {
    use crate::{EventHandlers, Node};

    use super::{
        StaticAttribute, StaticElementNodeDesc, StaticNodeDesc, StaticPrefabFlags,
        StaticPrefabMetrics, analyze_prefab,
    };

    fn increment() {}

    const CHILDREN: [StaticNodeDesc; 1] = [StaticNodeDesc::text("hello")];
    const ATTRIBUTES: [StaticAttribute; 1] = [StaticAttribute::new("data-kind", "greeting")];
    const ROOT: StaticNodeDesc = StaticNodeDesc::element(
        StaticElementNodeDesc::new("button", Some("cta"), &["primary"], &ATTRIBUTES, &CHILDREN)
            .with_handlers(EventHandlers {
                click: Some(increment),
                ..EventHandlers::NONE
            }),
    );
    const ROOT_METRICS: StaticPrefabMetrics = analyze_prefab(&ROOT);

    #[test]
    fn static_node_descriptors_lower_into_runtime_nodes() {
        let node = ROOT.to_node();
        let Node::Element(element) = node else {
            panic!("expected element node");
        };

        assert_eq!(element.tag, "button");
        assert_eq!(element.id.as_deref(), Some("cta"));
        assert_eq!(element.classes, vec!["primary".to_string()]);
        assert_eq!(element.attribute("data-kind"), Some("greeting"));
        assert!(element.handlers.click.is_some());
        assert_eq!(element.children.len(), 1);
    }

    #[test]
    fn const_prefab_analysis_computes_metrics_and_flags() {
        assert_eq!(
            ROOT_METRICS,
            StaticPrefabMetrics {
                node_count: 2,
                element_count: 1,
                text_count: 1,
                interactive_element_count: 1,
                max_depth: 2,
                flags: StaticPrefabFlags {
                    interactive: true,
                    identified: true,
                    has_classes: true,
                    has_attributes: true,
                },
            }
        );
    }
}
