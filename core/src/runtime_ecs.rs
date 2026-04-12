use std::collections::BTreeMap;

use crate::{
    ElementInteractionState, ElementNode, ElementPath, EventHandlers, LayoutBox, Node,
    ScrollbarData, Style,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Entity(usize);

impl Entity {
    pub const fn index(self) -> usize {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum RuntimeDirtyClass {
    #[default]
    Clean,
    Paint,
    Layout,
    Structure,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RuntimeDirtyFlags {
    pub paint: bool,
    pub layout: bool,
    pub structure: bool,
}

impl RuntimeDirtyFlags {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn mark(&mut self, class: RuntimeDirtyClass) {
        match class {
            RuntimeDirtyClass::Clean => {}
            RuntimeDirtyClass::Paint => {
                self.paint = true;
            }
            RuntimeDirtyClass::Layout => {
                self.paint = true;
                self.layout = true;
            }
            RuntimeDirtyClass::Structure => {
                self.paint = true;
                self.layout = true;
                self.structure = true;
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RuntimeElementInteraction {
    pub hovered: bool,
    pub active: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RuntimeScrollState {
    pub data: Option<ScrollbarData>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RuntimeViewport {
    pub width: usize,
    pub height: usize,
}

impl RuntimeViewport {
    pub const fn new(width: usize, height: usize) -> Self {
        Self {
            width: if width == 0 { 1 } else { width },
            height: if height == 0 { 1 } else { height },
        }
    }
}

#[derive(Clone, Debug)]
pub struct RuntimeElementData {
    pub tag: String,
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub attributes: BTreeMap<String, String>,
    pub style: Style,
    pub handlers: EventHandlers,
}

#[derive(Clone, Debug)]
pub enum RuntimeNodeKind {
    Element(RuntimeElementData),
    Text(String),
}

#[derive(Clone, Debug, Default)]
pub struct RuntimeComputedNode {
    pub resolved_style: Option<Style>,
    pub layout: Option<LayoutBox>,
    pub interaction: RuntimeElementInteraction,
    pub scroll: RuntimeScrollState,
    pub dirty: RuntimeDirtyFlags,
    pub element_path: Option<ElementPath>,
}

#[derive(Clone, Debug)]
pub struct RuntimeEntityData {
    pub parent: Option<Entity>,
    pub children: Vec<Entity>,
    pub authored: RuntimeNodeKind,
    pub computed: RuntimeComputedNode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeSyncPolicy {
    ForceRebuild,
    PreferPatch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeSyncAction {
    Rebuilt,
    Patched,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuntimeSyncResult {
    pub root: Entity,
    pub action: RuntimeSyncAction,
    pub reused_entities: usize,
    pub spawned_entities: usize,
    pub despawned_entities: usize,
}

#[derive(Clone, Debug, Default)]
pub struct RuntimeWorld {
    entities: Vec<Option<RuntimeEntityData>>,
    free_list: Vec<usize>,
    roots: Vec<Option<Entity>>,
    viewport: Option<RuntimeViewport>,
    interaction: ElementInteractionState,
}

impl RuntimeWorld {
    pub fn viewport(&self) -> Option<RuntimeViewport> {
        self.viewport
    }

    pub fn set_viewport(&mut self, viewport: Option<RuntimeViewport>) {
        self.viewport = viewport;
    }

    pub fn interaction(&self) -> &ElementInteractionState {
        &self.interaction
    }

    pub fn set_interaction(&mut self, interaction: ElementInteractionState) {
        self.interaction = interaction;
        self.refresh_interaction_components();
    }

    pub fn sync_interaction_components(&mut self) {
        self.refresh_interaction_components();
    }

    pub fn root_entity(&self, root_index: usize) -> Option<Entity> {
        self.roots.get(root_index).copied().flatten()
    }

    pub fn entity_count(&self) -> usize {
        self.entities.iter().filter(|slot| slot.is_some()).count()
    }

    pub fn entity(&self, entity: Entity) -> Option<&RuntimeEntityData> {
        self.entities.get(entity.index())?.as_ref()
    }

    pub fn entity_mut(&mut self, entity: Entity) -> Option<&mut RuntimeEntityData> {
        self.entities.get_mut(entity.index())?.as_mut()
    }

    pub fn root_dirty_flags(&self, root_index: usize) -> RuntimeDirtyFlags {
        self.root_entity(root_index)
            .and_then(|entity| self.entity(entity).map(|data| data.computed.dirty))
            .unwrap_or_default()
    }

    pub fn clear_dirty_flags(&mut self) {
        for entity in self.entities.iter_mut().flatten() {
            entity.computed.dirty.clear();
        }
    }

    pub fn root_as_node(&self, root_index: usize) -> Option<Node> {
        self.root_entity(root_index)
            .and_then(|entity| self.entity_as_node(entity))
    }

    pub fn entity_as_node(&self, entity: Entity) -> Option<Node> {
        let data = self.entity(entity)?;
        match &data.authored {
            RuntimeNodeKind::Element(element) => {
                let mut children = Vec::with_capacity(data.children.len());
                for child in &data.children {
                    children.push(self.entity_as_node(*child)?);
                }

                Some(Node::Element(ElementNode {
                    tag: element.tag.clone(),
                    id: element.id.clone(),
                    classes: element.classes.clone(),
                    attributes: element.attributes.clone(),
                    style: element.style.clone(),
                    children,
                    handlers: element.handlers,
                }))
            }
            RuntimeNodeKind::Text(text) => Some(Node::Text(text.clone())),
        }
    }

    pub fn sync_root(
        &mut self,
        root_index: usize,
        node: &Node,
        policy: RuntimeSyncPolicy,
        dirty_class: RuntimeDirtyClass,
    ) -> RuntimeSyncResult {
        self.ensure_root_slot(root_index);

        let existing_root = self.roots[root_index];
        if matches!(policy, RuntimeSyncPolicy::PreferPatch)
            && let Some(root) = existing_root
            && self.node_matches_shape(node, root)
        {
            let mut result = RuntimeSyncResult {
                root,
                action: RuntimeSyncAction::Patched,
                reused_entities: 0,
                spawned_entities: 0,
                despawned_entities: 0,
            };
            let element_path = element_path_for_root_node(node, root_index);
            self.patch_node(
                root,
                node,
                None,
                element_path.as_ref(),
                dirty_class,
                &mut result,
            );
            return result;
        }

        let despawned_entities = existing_root.map_or(0, |root| self.despawn_subtree(root));
        let mut result = RuntimeSyncResult {
            root: Entity(0),
            action: RuntimeSyncAction::Rebuilt,
            reused_entities: 0,
            spawned_entities: 0,
            despawned_entities,
        };
        let element_path = element_path_for_root_node(node, root_index);
        let root = self.spawn_node(node, None, element_path.as_ref(), dirty_class, &mut result);
        self.roots[root_index] = Some(root);
        result.root = root;
        result
    }

    fn ensure_root_slot(&mut self, root_index: usize) {
        if self.roots.len() <= root_index {
            self.roots.resize(root_index + 1, None);
        }
    }

    fn refresh_interaction_components(&mut self) {
        let interaction = self.interaction.clone();
        for entity in self.entities.iter_mut().flatten() {
            entity.computed.interaction =
                interaction_component(&interaction, entity.computed.element_path.as_ref());
        }
    }

    fn node_matches_shape(&self, node: &Node, entity: Entity) -> bool {
        let Some(data) = self.entity(entity) else {
            return false;
        };
        match (node, &data.authored) {
            (Node::Element(element), RuntimeNodeKind::Element(_)) => {
                if element.children.len() != data.children.len() {
                    return false;
                }
                element
                    .children
                    .iter()
                    .zip(&data.children)
                    .all(|(child_node, child_entity)| {
                        self.node_matches_shape(child_node, *child_entity)
                    })
            }
            (Node::Text(_), RuntimeNodeKind::Text(_)) => data.children.is_empty(),
            _ => false,
        }
    }

    fn patch_node(
        &mut self,
        entity: Entity,
        node: &Node,
        parent: Option<Entity>,
        element_path: Option<&ElementPath>,
        dirty_class: RuntimeDirtyClass,
        result: &mut RuntimeSyncResult,
    ) {
        let interaction = self.interaction.clone();
        let (children, authored_children) = {
            let data = self
                .entity_mut(entity)
                .expect("patched runtime entities should exist");
            data.parent = parent;
            data.authored = authored_from_node(node);
            data.computed.resolved_style = None;
            data.computed.layout = None;
            data.computed.scroll = RuntimeScrollState::default();
            data.computed.dirty.clear();
            data.computed.dirty.mark(dirty_class);
            data.computed.element_path = element_path.cloned();
            data.computed.interaction =
                interaction_component(&interaction, data.computed.element_path.as_ref());
            result.reused_entities += 1;

            let authored_children = match node {
                Node::Element(element) => Some(element.children.clone()),
                Node::Text(_) => None,
            };
            (data.children.clone(), authored_children)
        };

        let Some(authored_children) = authored_children else {
            return;
        };

        let mut element_child_index = 0;
        for (child_entity, child_node) in children.into_iter().zip(&authored_children) {
            let child_element_path = match child_node {
                Node::Element(_) => {
                    let path = element_path.map(|path| path.with_child(element_child_index));
                    element_child_index += 1;
                    path
                }
                Node::Text(_) => None,
            };
            self.patch_node(
                child_entity,
                child_node,
                Some(entity),
                child_element_path.as_ref(),
                dirty_class,
                result,
            );
        }
    }

    fn spawn_node(
        &mut self,
        node: &Node,
        parent: Option<Entity>,
        element_path: Option<&ElementPath>,
        dirty_class: RuntimeDirtyClass,
        result: &mut RuntimeSyncResult,
    ) -> Entity {
        let entity = self.allocate_entity();
        let interaction = interaction_component(&self.interaction, element_path);
        let computed = RuntimeComputedNode {
            resolved_style: None,
            layout: None,
            interaction,
            scroll: RuntimeScrollState::default(),
            dirty: {
                let mut dirty = RuntimeDirtyFlags::default();
                dirty.mark(dirty_class);
                dirty
            },
            element_path: element_path.cloned(),
        };
        self.entities[entity.index()] = Some(RuntimeEntityData {
            parent,
            children: Vec::new(),
            authored: authored_from_node(node),
            computed,
        });
        result.spawned_entities += 1;

        if let Node::Element(element) = node {
            let mut child_entities = Vec::with_capacity(element.children.len());
            let mut element_child_index = 0;
            for child in &element.children {
                let child_element_path = match child {
                    Node::Element(_) => {
                        let path = element_path.map(|path| path.with_child(element_child_index));
                        element_child_index += 1;
                        path
                    }
                    Node::Text(_) => None,
                };
                let child_entity = self.spawn_node(
                    child,
                    Some(entity),
                    child_element_path.as_ref(),
                    dirty_class,
                    result,
                );
                child_entities.push(child_entity);
            }
            self.entity_mut(entity)
                .expect("spawned entity should still exist")
                .children = child_entities;
        }

        entity
    }

    fn allocate_entity(&mut self) -> Entity {
        if let Some(index) = self.free_list.pop() {
            return Entity(index);
        }

        let index = self.entities.len();
        self.entities.push(None);
        Entity(index)
    }

    fn despawn_subtree(&mut self, entity: Entity) -> usize {
        let Some(data) = self.entities.get_mut(entity.index()).and_then(Option::take) else {
            return 0;
        };

        let mut removed = 1;
        for child in data.children {
            removed += self.despawn_subtree(child);
        }
        self.free_list.push(entity.index());
        removed
    }
}

fn authored_from_node(node: &Node) -> RuntimeNodeKind {
    match node {
        Node::Element(element) => RuntimeNodeKind::Element(RuntimeElementData {
            tag: element.tag.clone(),
            id: element.id.clone(),
            classes: element.classes.clone(),
            attributes: element.attributes.clone(),
            style: element.style.clone(),
            handlers: element.handlers,
        }),
        Node::Text(text) => RuntimeNodeKind::Text(text.clone()),
    }
}

fn element_path_for_root_node(node: &Node, root_index: usize) -> Option<ElementPath> {
    matches!(node, Node::Element(_)).then(|| ElementPath::root(root_index))
}

fn interaction_component(
    interaction: &ElementInteractionState,
    element_path: Option<&ElementPath>,
) -> RuntimeElementInteraction {
    let Some(element_path) = element_path else {
        return RuntimeElementInteraction::default();
    };

    RuntimeElementInteraction {
        hovered: interaction.is_hovered(element_path),
        active: interaction.is_active(element_path),
    }
}

#[cfg(test)]
mod tests {
    use crate::{Node, Style};

    use super::{
        RuntimeDirtyClass, RuntimeSyncAction, RuntimeSyncPolicy, RuntimeViewport, RuntimeWorld,
    };

    #[test]
    fn viewport_is_engine_owned_and_clamped() {
        let mut world = RuntimeWorld::default();
        world.set_viewport(Some(RuntimeViewport::new(0, 240)));
        assert_eq!(world.viewport(), Some(RuntimeViewport::new(1, 240)));
    }

    #[test]
    fn sync_root_builds_runtime_entities_from_dom_nodes() {
        let mut world = RuntimeWorld::default();
        let tree = Node::element("section")
            .with_id("card")
            .with_child(
                Node::element("p")
                    .with_style(Style::default())
                    .with_child(Node::text("hello"))
                    .into(),
            )
            .into();

        let result = world.sync_root(
            0,
            &tree,
            RuntimeSyncPolicy::ForceRebuild,
            RuntimeDirtyClass::Structure,
        );

        assert_eq!(result.action, RuntimeSyncAction::Rebuilt);
        assert_eq!(world.entity_count(), 3);
        let root = world.root_entity(0).expect("root entity should exist");
        let root_data = world.entity(root).expect("root data should exist");
        assert!(matches!(
            root_data.authored,
            super::RuntimeNodeKind::Element(_)
        ));
        assert_eq!(
            root_data.computed.element_path,
            Some(crate::ElementPath::root(0))
        );
    }

    #[test]
    fn prefer_patch_reuses_entities_when_structure_stays_stable() {
        let mut world = RuntimeWorld::default();
        let first = Node::element("div")
            .with_child(Node::element("span").with_child(Node::text("first")).into())
            .into();
        let second = Node::element("div")
            .with_child(
                Node::element("span")
                    .with_child(Node::text("second"))
                    .into(),
            )
            .into();

        world.sync_root(
            0,
            &first,
            RuntimeSyncPolicy::ForceRebuild,
            RuntimeDirtyClass::Structure,
        );
        let root = world.root_entity(0).expect("first root should exist");
        let child = world.entity(root).expect("root data should exist").children[0];
        let text = world
            .entity(child)
            .expect("child data should exist")
            .children[0];

        let result = world.sync_root(
            0,
            &second,
            RuntimeSyncPolicy::PreferPatch,
            RuntimeDirtyClass::Paint,
        );

        assert_eq!(result.action, RuntimeSyncAction::Patched);
        assert_eq!(world.root_entity(0), Some(root));
        assert_eq!(
            world
                .entity(root)
                .expect("patched root should exist")
                .children[0],
            child
        );
        assert_eq!(
            world
                .entity(child)
                .expect("patched child should exist")
                .children[0],
            text
        );
        let roundtrip = world.root_as_node(0).expect("root should roundtrip");
        let Node::Element(root_element) = roundtrip else {
            panic!("roundtripped root should stay an element");
        };
        let Node::Element(span) = &root_element.children[0] else {
            panic!("roundtripped child should stay an element");
        };
        let Node::Text(text) = &span.children[0] else {
            panic!("roundtripped grandchild should stay text");
        };
        assert_eq!(text, "second");
    }

    #[test]
    fn prefer_patch_falls_back_to_rebuild_when_shape_changes() {
        let mut world = RuntimeWorld::default();
        let first = Node::element("div")
            .with_child(Node::element("span").into())
            .into();
        let second = Node::element("div")
            .with_child(Node::element("span").into())
            .with_child(Node::element("strong").into())
            .into();

        world.sync_root(
            0,
            &first,
            RuntimeSyncPolicy::ForceRebuild,
            RuntimeDirtyClass::Structure,
        );
        assert!(world.root_entity(0).is_some(), "first root should exist");

        let result = world.sync_root(
            0,
            &second,
            RuntimeSyncPolicy::PreferPatch,
            RuntimeDirtyClass::Layout,
        );

        assert_eq!(result.action, RuntimeSyncAction::Rebuilt);
        assert_eq!(
            world.root_entity(0).expect("rebuilt root should exist"),
            result.root
        );
        assert!(result.despawned_entities > 0);
        assert!(result.spawned_entities > 0);
    }

    #[test]
    fn interaction_state_updates_runtime_entity_components() {
        let mut world = RuntimeWorld::default();
        let tree = Node::element("div")
            .with_child(
                Node::element("button")
                    .with_child(Node::element("span").into())
                    .into(),
            )
            .into();

        world.sync_root(
            0,
            &tree,
            RuntimeSyncPolicy::ForceRebuild,
            RuntimeDirtyClass::Structure,
        );
        world.set_interaction(crate::ElementInteractionState {
            hovered: Some(crate::ElementPath::root(0).with_child(0).with_child(0)),
            active: Some(crate::ElementPath::root(0).with_child(0)),
        });

        let root = world.root_entity(0).expect("root should exist");
        let button = world.entity(root).expect("root data should exist").children[0];
        let span = world
            .entity(button)
            .expect("button data should exist")
            .children[0];

        assert!(
            world
                .entity(root)
                .expect("root entity should exist")
                .computed
                .interaction
                .hovered
        );
        assert!(
            world
                .entity(button)
                .expect("button entity should exist")
                .computed
                .interaction
                .hovered
        );
        assert!(
            world
                .entity(button)
                .expect("button entity should exist")
                .computed
                .interaction
                .active
        );
        assert!(
            world
                .entity(span)
                .expect("span entity should exist")
                .computed
                .interaction
                .hovered
        );
        assert!(
            !world
                .entity(span)
                .expect("span entity should exist")
                .computed
                .interaction
                .active
        );
    }
}
