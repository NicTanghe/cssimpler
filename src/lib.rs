pub use cssimpler_core as core;
pub use cssimpler_macro::ui;
pub use cssimpler_renderer as renderer;
pub use cssimpler_style as style;

#[cfg(test)]
mod tests {
    use cssimpler_core::Node;

    use crate::ui;

    fn increment() {}

    #[test]
    fn ui_macro_builds_nested_nodes_from_html_like_input() {
        let count = 7_u32;
        let tree = ui! {
            <div id="app" class="card shell">
                <p class="label">
                    {count}
                </p>
            </div>
        };

        match tree {
            Node::Element(root) => {
                assert_eq!(root.id.as_deref(), Some("app"));
                assert_eq!(root.classes, vec!["card".to_string(), "shell".to_string()]);
                assert_eq!(root.children.len(), 1);
            }
            Node::Text(_) => panic!("expected root element"),
        }
    }

    #[test]
    fn ui_macro_supports_event_binding() {
        let tree = ui! {
            <button onclick={increment}>
                {"click"}
            </button>
        };

        match tree {
            Node::Element(button) => {
                assert!(button.on_click.is_some());
                assert_eq!(button.children.len(), 1);
            }
            Node::Text(_) => panic!("expected button element"),
        }
    }
}
