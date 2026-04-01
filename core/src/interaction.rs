#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ElementPath {
    pub root: usize,
    pub children: Vec<usize>,
}

impl ElementPath {
    pub fn root(root: usize) -> Self {
        Self {
            root,
            children: Vec::new(),
        }
    }

    pub fn with_child(&self, child_index: usize) -> Self {
        let mut children = self.children.clone();
        children.push(child_index);
        Self {
            root: self.root,
            children,
        }
    }

    pub fn ancestor(&self, levels: usize) -> Option<Self> {
        if self.children.len() < levels {
            return None;
        }

        let mut children = self.children.clone();
        children.truncate(children.len() - levels);
        Some(Self {
            root: self.root,
            children,
        })
    }

    pub fn is_prefix_of(&self, candidate: &Self) -> bool {
        self.root == candidate.root && candidate.children.starts_with(&self.children)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ElementInteractionState {
    pub hovered: Option<ElementPath>,
    pub active: Option<ElementPath>,
}

impl ElementInteractionState {
    pub fn is_hovered(&self, path: &ElementPath) -> bool {
        self.hovered
            .as_ref()
            .is_some_and(|hovered| path.is_prefix_of(hovered))
    }

    pub fn is_active(&self, path: &ElementPath) -> bool {
        self.active
            .as_ref()
            .is_some_and(|active| path.is_prefix_of(active))
    }
}

#[cfg(test)]
mod tests {
    use super::{ElementInteractionState, ElementPath};

    #[test]
    fn element_paths_track_children_and_ancestors() {
        let root = ElementPath::root(2);
        let child = root.with_child(1);
        let grandchild = child.with_child(3);

        assert_eq!(grandchild.root, 2);
        assert_eq!(grandchild.children, vec![1, 3]);
        assert_eq!(grandchild.ancestor(1), Some(child.clone()));
        assert_eq!(grandchild.ancestor(2), Some(root));
        assert_eq!(grandchild.ancestor(3), None);
    }

    #[test]
    fn interaction_state_treats_ancestor_paths_as_hovered_and_active() {
        let button = ElementPath::root(0).with_child(1);
        let label = button.with_child(0);
        let interaction = ElementInteractionState {
            hovered: Some(label.clone()),
            active: Some(label),
        };

        assert!(interaction.is_hovered(&button));
        assert!(interaction.is_active(&button));
        assert!(!interaction.is_hovered(&ElementPath::root(1)));
    }
}
