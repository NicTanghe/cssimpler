use std::collections::BTreeMap;

use crate::Style;

pub type EventHandler = fn();

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
    pub on_click: Option<EventHandler>,
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
            on_click: None,
        }
    }

    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        let id = id.into();
        self.id = Some(id.clone());
        self.attributes.insert("id".to_string(), id);
        self
    }

    pub fn with_class(mut self, class_name: impl Into<String>) -> Self {
        self.classes.push(class_name.into());
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

    pub fn on_click(mut self, handler: EventHandler) -> Self {
        self.on_click = Some(handler);
        self
    }

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
