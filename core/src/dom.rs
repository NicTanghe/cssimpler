use std::collections::BTreeMap;

use crate::Style;

pub type EventHandler = fn();
pub type InputEventHandler = fn(&str);

// Plain function handlers are stored inline. Captured handlers are allocated only
// for nodes that need them and shared when a node is cloned.
#[derive(Clone, Default)]
pub struct EventHandlers {
    pub click: Option<EventHandler>,
    pub contextmenu: Option<EventHandler>,
    pub dblclick: Option<EventHandler>,
    pub input: Option<InputEventHandler>,
    pub enter: Option<EventHandler>,
    pub mousedown: Option<EventHandler>,
    pub mouseenter: Option<EventHandler>,
    pub mouseleave: Option<EventHandler>,
    pub mousemove: Option<EventHandler>,
    pub mouseout: Option<EventHandler>,
    pub mouseover: Option<EventHandler>,
    pub mouseup: Option<EventHandler>,
    shared: Option<std::sync::Arc<SharedEventHandlers>>,
}

#[derive(Clone, Default)]
struct SharedEventHandlers {
    click: Option<std::sync::Arc<dyn Fn() + Send + Sync>>,
    contextmenu: Option<std::sync::Arc<dyn Fn() + Send + Sync>>,
    dblclick: Option<std::sync::Arc<dyn Fn() + Send + Sync>>,
    input: Option<std::sync::Arc<dyn Fn(&str) + Send + Sync>>,
    enter: Option<std::sync::Arc<dyn Fn() + Send + Sync>>,
    mousedown: Option<std::sync::Arc<dyn Fn() + Send + Sync>>,
    mouseenter: Option<std::sync::Arc<dyn Fn() + Send + Sync>>,
    mouseleave: Option<std::sync::Arc<dyn Fn() + Send + Sync>>,
    mousemove: Option<std::sync::Arc<dyn Fn() + Send + Sync>>,
    mouseout: Option<std::sync::Arc<dyn Fn() + Send + Sync>>,
    mouseover: Option<std::sync::Arc<dyn Fn() + Send + Sync>>,
    mouseup: Option<std::sync::Arc<dyn Fn() + Send + Sync>>,
}

macro_rules! event_handler_methods {
    ($name:ident, $set:ident, $set_shared:ident, $handler:ty, ($($arg:ty),*)) => {
        pub fn $name(&self) -> Option<&(dyn Fn($($arg),*) + Send + Sync)> {
            if let Some(handler) = &self.$name {
                Some(handler)
            } else {
                self.shared.as_ref().and_then(|shared| shared.$name.as_deref())
            }
        }

        pub fn $set(&mut self, handler: $handler) {
            self.$name = Some(handler);
            if let Some(shared) = &mut self.shared {
                std::sync::Arc::make_mut(shared).$name = None;
            }
        }

        pub fn $set_shared(&mut self, handler: impl Fn($($arg),*) + Send + Sync + 'static) {
            self.$name = None;
            std::sync::Arc::make_mut(self.shared.get_or_insert_with(Default::default)).$name =
                Some(std::sync::Arc::new(handler));
        }
    };
}

impl EventHandlers {
    event_handler_methods!(click, set_click, set_click_shared, EventHandler, ());
    event_handler_methods!(
        contextmenu,
        set_contextmenu,
        set_contextmenu_shared,
        EventHandler,
        ()
    );
    event_handler_methods!(
        dblclick,
        set_dblclick,
        set_dblclick_shared,
        EventHandler,
        ()
    );
    event_handler_methods!(
        input,
        set_input,
        set_input_shared,
        InputEventHandler,
        (&str)
    );
    event_handler_methods!(enter, set_enter, set_enter_shared, EventHandler, ());
    event_handler_methods!(
        mousedown,
        set_mousedown,
        set_mousedown_shared,
        EventHandler,
        ()
    );
    event_handler_methods!(
        mouseenter,
        set_mouseenter,
        set_mouseenter_shared,
        EventHandler,
        ()
    );
    event_handler_methods!(
        mouseleave,
        set_mouseleave,
        set_mouseleave_shared,
        EventHandler,
        ()
    );
    event_handler_methods!(
        mousemove,
        set_mousemove,
        set_mousemove_shared,
        EventHandler,
        ()
    );
    event_handler_methods!(
        mouseout,
        set_mouseout,
        set_mouseout_shared,
        EventHandler,
        ()
    );
    event_handler_methods!(
        mouseover,
        set_mouseover,
        set_mouseover_shared,
        EventHandler,
        ()
    );
    event_handler_methods!(mouseup, set_mouseup, set_mouseup_shared, EventHandler, ());
}

impl std::fmt::Debug for EventHandlers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventHandlers").finish_non_exhaustive()
    }
}

/// Values accepted in a dynamic HTML child slot, including lists and optional nodes.
pub trait IntoNodes {
    fn into_nodes(self) -> Vec<Node>;
}
impl<T: IntoNode> IntoNodes for T {
    fn into_nodes(self) -> Vec<Node> {
        vec![self.into_node()]
    }
}
impl<T: IntoNode> IntoNodes for Vec<T> {
    fn into_nodes(self) -> Vec<Node> {
        self.into_iter().map(IntoNode::into_node).collect()
    }
}
impl<T: IntoNode> IntoNodes for Option<T> {
    fn into_nodes(self) -> Vec<Node> {
        self.into_iter().map(IntoNode::into_node).collect()
    }
}

pub trait IntoNode {
    fn into_node(self) -> Node;
}

#[derive(Clone, Debug)]
pub enum Node {
    Element(ElementNode),
    Text(String),
}

impl Node {
    pub fn element(tag: impl Into<String>) -> ElementNode {
        ElementNode::new(tag)
    }

    pub fn text(text: impl Into<String>) -> Self {
        Self::Text(text.into())
    }
}

pub fn into_node(value: impl IntoNode) -> Node {
    value.into_node()
}

#[derive(Clone, Debug)]
pub struct ElementNode {
    pub tag: String,
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub attributes: BTreeMap<String, String>,
    pub style: Style,
    pub children: Vec<Node>,
    pub handlers: EventHandlers,
}

impl ElementNode {
    pub fn new(tag: impl Into<String>) -> Self {
        Self {
            tag: tag.into(),
            id: None,
            classes: Vec::new(),
            attributes: BTreeMap::new(),
            style: Style::default(),
            children: Vec::new(),
            handlers: EventHandlers::default(),
        }
    }

    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        let id = id.into();
        self.id = Some(id.clone());
        self.attributes.insert("id".to_string(), id);
        self
    }

    /// Append a whitespace-separated class list, preserving existing classes.
    /// This also handles class values supplied by dynamic `ui!` expressions.
    pub fn with_class(mut self, class_names: impl Into<String>) -> Self {
        self.classes
            .extend(class_names.into().split_whitespace().map(str::to_owned));
        self.sync_class_attribute();
        self
    }

    pub fn with_attribute(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.set_attribute(name, value);
        self
    }

    pub fn with_style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    pub fn with_child(mut self, child: Node) -> Self {
        self.children.push(child);
        self
    }

    pub fn with_children(mut self, children: impl IntoIterator<Item = Node>) -> Self {
        self.children.extend(children);
        self
    }

    pub fn with_nodes(mut self, nodes: impl IntoNodes) -> Self {
        self.children.extend(nodes.into_nodes());
        self
    }

    event_builder_methods!(
        on_click,
        on_click_shared,
        set_click,
        set_click_shared,
        EventHandler,
        ()
    );
    event_builder_methods!(
        on_contextmenu,
        on_contextmenu_shared,
        set_contextmenu,
        set_contextmenu_shared,
        EventHandler,
        ()
    );
    event_builder_methods!(
        on_dblclick,
        on_dblclick_shared,
        set_dblclick,
        set_dblclick_shared,
        EventHandler,
        ()
    );
    event_builder_methods!(
        on_input,
        on_input_shared,
        set_input,
        set_input_shared,
        InputEventHandler,
        (&str)
    );
    event_builder_methods!(
        on_enter,
        on_enter_shared,
        set_enter,
        set_enter_shared,
        EventHandler,
        ()
    );
    event_builder_methods!(
        on_mousedown,
        on_mousedown_shared,
        set_mousedown,
        set_mousedown_shared,
        EventHandler,
        ()
    );
    event_builder_methods!(
        on_mouseenter,
        on_mouseenter_shared,
        set_mouseenter,
        set_mouseenter_shared,
        EventHandler,
        ()
    );
    event_builder_methods!(
        on_mouseleave,
        on_mouseleave_shared,
        set_mouseleave,
        set_mouseleave_shared,
        EventHandler,
        ()
    );
    event_builder_methods!(
        on_mousemove,
        on_mousemove_shared,
        set_mousemove,
        set_mousemove_shared,
        EventHandler,
        ()
    );
    event_builder_methods!(
        on_mouseout,
        on_mouseout_shared,
        set_mouseout,
        set_mouseout_shared,
        EventHandler,
        ()
    );
    event_builder_methods!(
        on_mouseover,
        on_mouseover_shared,
        set_mouseover,
        set_mouseover_shared,
        EventHandler,
        ()
    );
    event_builder_methods!(
        on_mouseup,
        on_mouseup_shared,
        set_mouseup,
        set_mouseup_shared,
        EventHandler,
        ()
    );

    pub fn set_attribute(&mut self, name: impl Into<String>, value: impl Into<String>) {
        let name = name.into();
        let value = value.into();

        match name.as_str() {
            "id" => {
                self.id = Some(value.clone());
                self.attributes.insert(name, value);
            }
            "class" => {
                self.classes = value.split_whitespace().map(str::to_string).collect();
                self.sync_class_attribute();
            }
            _ => {
                self.attributes.insert(name, value);
            }
        }
    }

    pub fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes.get(name).map(String::as_str)
    }

    pub fn attributes(&self) -> &BTreeMap<String, String> {
        &self.attributes
    }

    fn sync_class_attribute(&mut self) {
        if self.classes.is_empty() {
            self.attributes.remove("class");
        } else {
            self.attributes
                .insert("class".to_string(), self.classes.join(" "));
        }
    }
}

impl From<ElementNode> for Node {
    fn from(value: ElementNode) -> Self {
        Self::Element(value)
    }
}

impl IntoNode for Node {
    fn into_node(self) -> Node {
        self
    }
}

impl IntoNode for ElementNode {
    fn into_node(self) -> Node {
        self.into()
    }
}

impl IntoNode for String {
    fn into_node(self) -> Node {
        Node::Text(self)
    }
}

impl IntoNode for &str {
    fn into_node(self) -> Node {
        Node::Text(self.to_string())
    }
}

impl IntoNode for &String {
    fn into_node(self) -> Node {
        Node::Text(self.clone())
    }
}

impl IntoNode for char {
    fn into_node(self) -> Node {
        Node::Text(self.to_string())
    }
}

impl IntoNode for bool {
    fn into_node(self) -> Node {
        Node::Text(self.to_string())
    }
}

macro_rules! impl_into_node_via_to_string {
    ($($ty:ty),* $(,)?) => {
        $(
            impl IntoNode for $ty {
                fn into_node(self) -> Node {
                    Node::Text(self.to_string())
                }
            }
        )*
    };
}

impl_into_node_via_to_string!(i8, i16, i32, i64, i128, isize);
impl_into_node_via_to_string!(u8, u16, u32, u64, u128, usize);
impl_into_node_via_to_string!(f32, f64);

#[cfg(test)]
mod tests {
    use super::Node;

    #[test]
    fn nested_dom_trees_are_supported() {
        let tree = Node::element("div")
            .with_class("card")
            .with_child(Node::element("p").with_child(Node::text("hello")).into())
            .into();

        match tree {
            Node::Element(element) => {
                assert_eq!(element.tag, "div");
                assert_eq!(element.classes, vec!["card".to_string()]);
                assert_eq!(element.children.len(), 1);
            }
            Node::Text(_) => panic!("expected an element node"),
        }
    }

    #[test]
    fn class_builders_and_attributes_share_tokenization() {
        for input in [
            "card",
            "card selected",
            "  card\tselected\nactive  ",
            "",
            " \t\n",
        ] {
            let appended = Node::element("button").with_class(input);
            let assigned = Node::element("button").with_attribute("class", input);
            assert_eq!(appended.classes, assigned.classes, "{input:?}");
            assert_eq!(appended.attribute("class"), assigned.attribute("class"));
        }
        let node = Node::element("button")
            .with_class("base")
            .with_class("card selected")
            .with_class(" ");
        assert_eq!(node.classes, ["base", "card", "selected"]);
        assert_eq!(node.attribute("class"), Some("base card selected"));
    }

    #[test]
    fn generic_attributes_are_preserved_deterministically() {
        let element = Node::element("div")
            .with_attribute("data-text", "uiverse")
            .with_attribute("aria-hidden", "true")
            .with_id("hero")
            .with_class("card")
            .with_class("selected");

        assert_eq!(element.attribute("data-text"), Some("uiverse"));
        assert_eq!(element.attribute("aria-hidden"), Some("true"));
        assert_eq!(element.attribute("id"), Some("hero"));
        assert_eq!(element.attribute("class"), Some("card selected"));
        assert_eq!(
            element
                .attributes()
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["aria-hidden", "class", "data-text", "id"]
        );
    }
}
