mod scene_transition;

use std::marker::PhantomData;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use std::time::Instant;

use self::scene_transition::SceneTransition;
use crate::core::{ElementInteractionState, ElementPath, Node, RenderNode};
use crate::renderer::{self, FrameInfo, SceneProvider, ViewportSize, WindowConfig};
use crate::style::{
    Stylesheet, build_render_tree_in_viewport_with_interaction_at_root,
    build_render_tree_with_interaction_at_root, rebuild_render_tree_with_cached_layout,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RuntimeStats {
    pub view_us: u64,
    pub render_tree_us: u64,
    pub scene_swap_us: u64,
    pub transition_us: u64,
    pub rerendered: bool,
    pub transition_active: bool,
}

static RUNTIME_STATS: OnceLock<Mutex<RuntimeStats>> = OnceLock::new();

pub fn latest_runtime_stats() -> RuntimeStats {
    *runtime_stats_store()
        .lock()
        .expect("runtime stats mutex should not be poisoned")
}

fn record_runtime_stats(stats: RuntimeStats) {
    *runtime_stats_store()
        .lock()
        .expect("runtime stats mutex should not be poisoned") = stats;
}

fn runtime_stats_store() -> &'static Mutex<RuntimeStats> {
    RUNTIME_STATS.get_or_init(|| Mutex::new(RuntimeStats::default()))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Invalidation {
    #[default]
    Clean,
    Paint,
    Layout,
    Structure,
}

impl Invalidation {
    pub const fn needs_rerender(self) -> bool {
        !matches!(self, Self::Clean)
    }

    fn merge(self, other: Self) -> Self {
        self.max(other)
    }
}

impl From<crate::style::StyleInvalidation> for Invalidation {
    fn from(value: crate::style::StyleInvalidation) -> Self {
        match value {
            crate::style::StyleInvalidation::Clean => Self::Clean,
            crate::style::StyleInvalidation::Paint => Self::Paint,
            crate::style::StyleInvalidation::Layout => Self::Layout,
            crate::style::StyleInvalidation::Structure => Self::Structure,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum RefreshTarget {
    #[default]
    None,
    Full,
    Fragments(Vec<String>),
}

impl RefreshTarget {
    fn merge(self, other: Self) -> Self {
        match (self, other) {
            (Self::Full, _) | (_, Self::Full) => Self::Full,
            (Self::None, target) | (target, Self::None) => target,
            (Self::Fragments(mut left), Self::Fragments(right)) => {
                for id in right {
                    push_unique_id(&mut left, id);
                }
                Self::Fragments(left)
            }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Refresh {
    pub invalidation: Invalidation,
    pub target: RefreshTarget,
}

impl Refresh {
    pub const fn clean() -> Self {
        Self {
            invalidation: Invalidation::Clean,
            target: RefreshTarget::None,
        }
    }

    pub const fn full(invalidation: Invalidation) -> Self {
        if matches!(invalidation, Invalidation::Clean) {
            return Self::clean();
        }

        Self {
            invalidation,
            target: RefreshTarget::Full,
        }
    }

    pub fn fragment(id: impl Into<String>, invalidation: Invalidation) -> Self {
        Self::fragments([id.into()], invalidation)
    }

    pub fn fragments<I, S>(ids: I, invalidation: Invalidation) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        if matches!(invalidation, Invalidation::Clean) {
            return Self::clean();
        }

        let mut fragment_ids = Vec::new();
        for id in ids {
            push_unique_id(&mut fragment_ids, id.into());
        }

        if fragment_ids.is_empty() {
            Self::full(invalidation)
        } else {
            Self {
                invalidation,
                target: RefreshTarget::Fragments(fragment_ids),
            }
        }
    }

    pub const fn needs_rerender(&self) -> bool {
        self.invalidation.needs_rerender()
    }

    fn merge(self, other: Self) -> Self {
        let invalidation = self.invalidation.merge(other.invalidation);
        let target = if invalidation.needs_rerender() {
            self.target.merge(other.target)
        } else {
            RefreshTarget::None
        };

        Self {
            invalidation,
            target,
        }
    }
}

impl From<Invalidation> for Refresh {
    fn from(value: Invalidation) -> Self {
        Self::full(value)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RenderMode {
    EveryFrame,
    #[default]
    OnInvalidation,
}

pub struct Fragment<'a, State> {
    id: String,
    view: Box<dyn Fn(&State) -> Node + 'a>,
}

impl<'a, State> Fragment<'a, State> {
    pub fn new(id: impl Into<String>, view: impl Fn(&State) -> Node + 'a) -> Self {
        Self {
            id: id.into(),
            view: Box::new(view),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    fn render(&self, state: &State) -> Node {
        (self.view)(state)
    }
}

enum UniqueMatch<T> {
    None,
    Single(T),
    Multiple,
}

struct NodeBoundaryMatch<'a> {
    node: &'a Node,
    path: ElementPath,
}

struct RenderBoundaryMatch<'a> {
    node: &'a RenderNode,
    path: ElementPath,
}

pub struct App<'a, State, Update, View, Signal = Invalidation> {
    state: State,
    stylesheet: &'a Stylesheet,
    update: Update,
    view: View,
    viewport: Option<ViewportSize>,
    interaction: ElementInteractionState,
    render_mode: RenderMode,
    pending_refresh: Refresh,
    cached_scene: Option<Vec<RenderNode>>,
    scene_transition: Option<SceneTransition>,
    signal: PhantomData<Signal>,
}

impl<'a, State, Update, View, Signal> App<'a, State, Update, View, Signal>
where
    Update: FnMut(&mut State, FrameInfo) -> Signal,
    View: FnMut(&State) -> Node,
    Signal: Into<Refresh>,
{
    pub fn new(state: State, stylesheet: &'a Stylesheet, update: Update, view: View) -> Self {
        Self {
            state,
            stylesheet,
            update,
            view,
            viewport: None,
            interaction: ElementInteractionState::default(),
            render_mode: RenderMode::OnInvalidation,
            pending_refresh: Refresh::full(Invalidation::Structure),
            cached_scene: None,
            scene_transition: None,
            signal: PhantomData,
        }
    }

    pub fn with_render_mode(mut self, render_mode: RenderMode) -> Self {
        self.render_mode = render_mode;
        self
    }

    pub fn render_mode(&self) -> RenderMode {
        self.render_mode
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn set_viewport(&mut self, viewport: ViewportSize) {
        if self.viewport != Some(viewport) {
            self.viewport = Some(viewport);
            self.invalidate(Invalidation::Layout);
        }
    }

    pub fn invalidate(&mut self, refresh: impl Into<Refresh>) {
        self.pending_refresh = std::mem::take(&mut self.pending_refresh).merge(refresh.into());
    }

    pub fn frame(&mut self, frame: FrameInfo) -> Vec<RenderNode> {
        self.advance(frame);
        self.scene().to_vec()
    }

    pub fn run(self, config: WindowConfig) -> renderer::Result<()> {
        renderer::run_with_scene_provider(config, self)
    }

    fn advance(&mut self, frame: FrameInfo) {
        let mut stats = RuntimeStats::default();
        let update = &mut self.update;
        let state = &mut self.state;
        let refresh = update(state, frame).into();
        self.pending_refresh = std::mem::take(&mut self.pending_refresh).merge(refresh);

        if self.needs_rerender() {
            self.refresh_scene(&mut stats);
        }

        self.advance_scene_transition(frame.delta, &mut stats);
        stats.transition_active = self.scene_transition.is_some();
        record_runtime_stats(stats);
    }

    fn needs_rerender(&self) -> bool {
        self.cached_scene.is_none()
            || matches!(self.render_mode, RenderMode::EveryFrame)
            || self.pending_refresh.needs_rerender()
    }

    fn refresh_scene(&mut self, stats: &mut RuntimeStats) {
        let must_full_refresh = self.cached_scene.is_none()
            || matches!(self.render_mode, RenderMode::EveryFrame)
            || matches!(self.pending_refresh.target, RefreshTarget::Full)
            || !matches!(self.pending_refresh.invalidation, Invalidation::Paint);

        if must_full_refresh {
            self.rebuild_scene(stats);
            return;
        }

        let fragment_ids = match &self.pending_refresh.target {
            RefreshTarget::None => {
                self.pending_refresh = Refresh::clean();
                return;
            }
            RefreshTarget::Full => unreachable!("full refreshes return early"),
            RefreshTarget::Fragments(ids) => ids.clone(),
        };

        if !self.refresh_fragments(&fragment_ids, stats) {
            self.rebuild_scene(stats);
            return;
        }

        self.pending_refresh = Refresh::clean();
    }

    fn rebuild_scene(&mut self, stats: &mut RuntimeStats) {
        let view = &mut self.view;
        let view_start = Instant::now();
        let tree = view(&self.state);
        stats.view_us = duration_to_us(view_start.elapsed());

        let render_tree_start = Instant::now();
        let scene = vec![self.render_root(&tree)];
        stats.render_tree_us = duration_to_us(render_tree_start.elapsed());

        let scene_swap_start = Instant::now();
        self.replace_scene(scene);
        stats.scene_swap_us = duration_to_us(scene_swap_start.elapsed());
        stats.rerendered = true;
        self.pending_refresh = Refresh::clean();
    }

    fn refresh_fragments(&mut self, ids: &[String], stats: &mut RuntimeStats) -> bool {
        let Some(existing_scene) = self.cached_scene.as_ref() else {
            return false;
        };
        if existing_scene.len() != 1 {
            return false;
        }
        let existing_root = &existing_scene[0];

        let view = &mut self.view;
        let view_start = Instant::now();
        let tree = view(&self.state);
        stats.view_us = duration_to_us(view_start.elapsed());

        let render_tree_start = Instant::now();
        let mut replacements = Vec::with_capacity(ids.len());
        for id in ids {
            let UniqueMatch::Single(node_match) =
                find_unique_node_boundary(&tree, id, &ElementPath::root(0))
            else {
                return false;
            };
            let UniqueMatch::Single(render_match) = find_unique_render_boundary(existing_root, id)
            else {
                return false;
            };
            if node_match.path != render_match.path {
                return false;
            }
            let Some(node) = rebuild_render_tree_with_cached_layout(
                node_match.node,
                self.stylesheet,
                &self.interaction,
                &node_match.path,
                render_match.node,
            ) else {
                return false;
            };
            replacements.push((render_match.path, node));
        }
        stats.render_tree_us = duration_to_us(render_tree_start.elapsed());

        replacements.sort_by_key(|(path, _)| path.children.len());
        let mut filtered = Vec::with_capacity(replacements.len());
        for (path, node) in replacements {
            if filtered
                .iter()
                .any(|(existing_path, _): &(ElementPath, RenderNode)| {
                    existing_path.is_prefix_of(&path)
                })
            {
                continue;
            }
            filtered.push((path, node));
        }

        let mut scene = self
            .cached_scene
            .take()
            .expect("app scene should be cached while refreshing fragments");
        for (path, node) in filtered {
            let Some(target) = find_render_node_mut(&mut scene[0], &path) else {
                return false;
            };
            *target = node;
        }

        let scene_swap_start = Instant::now();
        self.replace_scene(scene);
        stats.scene_swap_us = duration_to_us(scene_swap_start.elapsed());
        stats.rerendered = true;
        true
    }

    fn render_root(&self, tree: &Node) -> RenderNode {
        if let Some(viewport) = self.viewport {
            build_render_tree_in_viewport_with_interaction_at_root(
                tree,
                self.stylesheet,
                viewport.width,
                viewport.height,
                &self.interaction,
                0,
            )
        } else {
            build_render_tree_with_interaction_at_root(tree, self.stylesheet, &self.interaction, 0)
        }
    }

    fn scene(&self) -> &[RenderNode] {
        self.cached_scene
            .as_deref()
            .expect("app scene should be cached after the first frame")
    }

    fn replace_scene(&mut self, scene: Vec<RenderNode>) {
        let previous = self.cached_scene.take();

        match previous {
            Some(p) if SceneTransition::should_create(&p, &scene) => {
                let transition = SceneTransition::new(p, scene)
                    .expect("SceneTransition::new failed despite should_create check");

                self.cached_scene = Some(transition.sample());
                self.scene_transition = Some(transition);
            }
            _ => {
                self.cached_scene = Some(scene);
                self.scene_transition = None;
            }
        }
    }

    fn advance_scene_transition(&mut self, delta: Duration, stats: &mut RuntimeStats) {
        let Some(transition) = self.scene_transition.as_mut() else {
            return;
        };
        let sample_start = Instant::now();
        self.cached_scene = Some(transition.advance(delta));
        stats.transition_us = duration_to_us(sample_start.elapsed());
        if !transition.is_active() {
            self.scene_transition = None;
        }
    }

    fn set_interaction_state(&mut self, interaction: ElementInteractionState) -> bool {
        if self.interaction == interaction {
            return false;
        }

        let refresh = self.interaction_refresh(&self.interaction, &interaction);
        self.interaction = interaction;
        let needs_rerender = refresh.needs_rerender();
        self.invalidate(refresh);
        needs_rerender
    }

    fn interaction_refresh(
        &self,
        previous: &ElementInteractionState,
        next: &ElementInteractionState,
    ) -> Refresh {
        let invalidation: Invalidation = self
            .stylesheet
            .interaction_invalidation(previous, next)
            .into();
        if !invalidation.needs_rerender() {
            return Refresh::clean();
        }
        if !matches!(invalidation, Invalidation::Paint) {
            return Refresh::full(invalidation);
        }

        let Some(root) = self.cached_scene.as_deref().and_then(|scene| scene.first()) else {
            return Refresh::full(invalidation);
        };

        let mut fragment_ids = Vec::new();
        for path in interaction_paths(previous)
            .into_iter()
            .chain(interaction_paths(next))
        {
            let Some(fragment_id) = stable_boundary_id_for_path(root, &path) else {
                return Refresh::full(invalidation);
            };
            push_unique_id(&mut fragment_ids, fragment_id);
        }

        if fragment_ids.is_empty() {
            Refresh::full(invalidation)
        } else {
            Refresh::fragments(fragment_ids, invalidation)
        }
    }
}

impl<'a, State, Update, View, Signal> SceneProvider for App<'a, State, Update, View, Signal>
where
    Update: FnMut(&mut State, FrameInfo) -> Signal,
    View: FnMut(&State) -> Node,
    Signal: Into<Refresh>,
{
    fn set_viewport(&mut self, viewport: ViewportSize) {
        App::set_viewport(self, viewport);
    }

    fn update(&mut self, frame: FrameInfo) {
        self.advance(frame);
    }

    fn scene(&self) -> &[RenderNode] {
        App::scene(self)
    }

    fn set_element_interaction(&mut self, interaction: ElementInteractionState) -> bool {
        App::set_interaction_state(self, interaction)
    }
}

pub struct FragmentApp<'a, State, Update, Signal = Invalidation> {
    state: State,
    stylesheet: &'a Stylesheet,
    update: Update,
    fragments: Vec<Fragment<'a, State>>,
    viewport: Option<ViewportSize>,
    interaction: ElementInteractionState,
    render_mode: RenderMode,
    pending_refresh: Refresh,
    cached_scene: Option<Vec<RenderNode>>,
    scene_transition: Option<SceneTransition>,
    signal: PhantomData<Signal>,
}

impl<'a, State, Update, Signal> FragmentApp<'a, State, Update, Signal>
where
    Update: FnMut(&mut State, FrameInfo) -> Signal,
    Signal: Into<Refresh>,
{
    pub fn new<I>(state: State, stylesheet: &'a Stylesheet, update: Update, fragments: I) -> Self
    where
        I: IntoIterator<Item = Fragment<'a, State>>,
    {
        Self {
            state,
            stylesheet,
            update,
            fragments: fragments.into_iter().collect(),
            viewport: None,
            interaction: ElementInteractionState::default(),
            render_mode: RenderMode::OnInvalidation,
            pending_refresh: Refresh::full(Invalidation::Structure),
            cached_scene: None,
            scene_transition: None,
            signal: PhantomData,
        }
    }

    pub fn with_render_mode(mut self, render_mode: RenderMode) -> Self {
        self.render_mode = render_mode;
        self
    }

    pub fn render_mode(&self) -> RenderMode {
        self.render_mode
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn set_viewport(&mut self, viewport: ViewportSize) {
        if self.viewport != Some(viewport) {
            self.viewport = Some(viewport);
            self.invalidate(Invalidation::Layout);
        }
    }

    pub fn invalidate(&mut self, refresh: impl Into<Refresh>) {
        self.pending_refresh = std::mem::take(&mut self.pending_refresh).merge(refresh.into());
    }

    pub fn frame(&mut self, frame: FrameInfo) -> Vec<RenderNode> {
        self.advance(frame);
        self.scene().to_vec()
    }

    pub fn run(self, config: WindowConfig) -> renderer::Result<()> {
        renderer::run_with_scene_provider(config, self)
    }

    fn advance(&mut self, frame: FrameInfo) {
        let mut stats = RuntimeStats::default();
        let update = &mut self.update;
        let state = &mut self.state;
        let refresh = update(state, frame).into();
        self.pending_refresh = std::mem::take(&mut self.pending_refresh).merge(refresh);

        if self.needs_rerender() {
            self.refresh_scene(&mut stats);
        }

        self.advance_scene_transition(frame.delta, &mut stats);
        stats.transition_active = self.scene_transition.is_some();
        record_runtime_stats(stats);
    }

    fn needs_rerender(&self) -> bool {
        self.cached_scene.is_none()
            || matches!(self.render_mode, RenderMode::EveryFrame)
            || self.pending_refresh.needs_rerender()
    }

    fn refresh_scene(&mut self, stats: &mut RuntimeStats) {
        let refresh_start = Instant::now();
        let must_full_refresh = self.cached_scene.is_none()
            || matches!(self.render_mode, RenderMode::EveryFrame)
            || matches!(self.pending_refresh.target, RefreshTarget::Full);

        if must_full_refresh {
            self.rebuild_all_fragments();
            stats.render_tree_us = duration_to_us(refresh_start.elapsed());
            stats.rerendered = true;
            self.pending_refresh = Refresh::clean();
            return;
        }

        let fragment_ids = match &self.pending_refresh.target {
            RefreshTarget::None => {
                stats.render_tree_us = duration_to_us(refresh_start.elapsed());
                self.pending_refresh = Refresh::clean();
                return;
            }
            RefreshTarget::Full => unreachable!("full refreshes return early"),
            RefreshTarget::Fragments(ids) => ids.clone(),
        };

        if !self.refresh_fragments(&fragment_ids) {
            self.rebuild_all_fragments();
        }

        stats.render_tree_us = duration_to_us(refresh_start.elapsed());
        stats.rerendered = true;
        self.pending_refresh = Refresh::clean();
    }

    fn rebuild_all_fragments(&mut self) {
        let scene = self
            .fragments
            .iter()
            .enumerate()
            .map(|(index, fragment)| self.render_fragment(index, fragment))
            .collect();
        self.replace_scene(scene);
    }

    fn refresh_fragments(&mut self, ids: &[String]) -> bool {
        let Some(existing_scene) = self.cached_scene.as_ref() else {
            return false;
        };
        if existing_scene.len() != self.fragments.len() {
            return false;
        }

        let mut replacements = Vec::with_capacity(ids.len());
        for id in ids {
            let Some(index) = self
                .fragments
                .iter()
                .position(|fragment| fragment.id() == id)
            else {
                return false;
            };
            let node = self.fragments[index].render(&self.state);
            replacements.push((index, self.render_node(&node, index)));
        }

        let mut scene = self
            .cached_scene
            .take()
            .expect("fragment app scene should be cached while refreshing fragments");
        for (index, node) in replacements {
            scene[index] = node;
        }

        self.replace_scene(scene);

        true
    }

    fn render_fragment(&self, index: usize, fragment: &Fragment<'a, State>) -> RenderNode {
        let node = fragment.render(&self.state);
        self.render_node(&node, index)
    }

    fn render_node(&self, node: &Node, root_index: usize) -> RenderNode {
        if let Some(viewport) = self.viewport {
            build_render_tree_in_viewport_with_interaction_at_root(
                node,
                self.stylesheet,
                viewport.width,
                viewport.height,
                &self.interaction,
                root_index,
            )
        } else {
            build_render_tree_with_interaction_at_root(
                node,
                self.stylesheet,
                &self.interaction,
                root_index,
            )
        }
    }

    fn scene(&self) -> &[RenderNode] {
        self.cached_scene
            .as_deref()
            .expect("fragment app scene should be cached after the first frame")
    }

    fn replace_scene(&mut self, scene: Vec<RenderNode>) {
        if let Some(previous) = self.cached_scene.take() {
            if SceneTransition::should_create(&previous, &scene) {
                let transition = SceneTransition::new(previous, scene)
                    .expect("scene transition should exist after precheck");
                self.cached_scene = Some(transition.sample());
                self.scene_transition = Some(transition);
                return;
            }
        }

        self.cached_scene = Some(scene);
        self.scene_transition = None;
    }

    fn advance_scene_transition(&mut self, delta: Duration, stats: &mut RuntimeStats) {
        let Some(transition) = self.scene_transition.as_mut() else {
            return;
        };
        let sample_start = Instant::now();
        self.cached_scene = Some(transition.advance(delta));
        stats.transition_us = duration_to_us(sample_start.elapsed());
        if !transition.is_active() {
            self.scene_transition = None;
        }
    }

    fn set_interaction_state(&mut self, interaction: ElementInteractionState) -> bool {
        if self.interaction == interaction {
            return false;
        }

        let refresh = self.interaction_refresh(&self.interaction, &interaction);
        self.interaction = interaction;
        let needs_rerender = refresh.needs_rerender();
        self.invalidate(refresh);
        needs_rerender
    }

    fn interaction_refresh(
        &self,
        previous: &ElementInteractionState,
        next: &ElementInteractionState,
    ) -> Refresh {
        let invalidation: Invalidation = self
            .stylesheet
            .interaction_invalidation(previous, next)
            .into();
        if !invalidation.needs_rerender() {
            return Refresh::clean();
        }

        let mut fragment_ids = Vec::new();
        for root in interaction_roots(previous)
            .into_iter()
            .chain(interaction_roots(next))
        {
            let Some(fragment) = self.fragments.get(root) else {
                return Refresh::full(invalidation);
            };
            push_unique_id(&mut fragment_ids, fragment.id().to_string());
        }

        if fragment_ids.is_empty() {
            Refresh::clean()
        } else {
            Refresh::fragments(fragment_ids, invalidation)
        }
    }
}

impl<'a, State, Update, Signal> SceneProvider for FragmentApp<'a, State, Update, Signal>
where
    Update: FnMut(&mut State, FrameInfo) -> Signal,
    Signal: Into<Refresh>,
{
    fn set_viewport(&mut self, viewport: ViewportSize) {
        FragmentApp::set_viewport(self, viewport);
    }

    fn update(&mut self, frame: FrameInfo) {
        self.advance(frame);
    }

    fn scene(&self) -> &[RenderNode] {
        FragmentApp::scene(self)
    }

    fn set_element_interaction(&mut self, interaction: ElementInteractionState) -> bool {
        FragmentApp::set_interaction_state(self, interaction)
    }
}

pub fn run<State, Update, View, Signal>(
    config: WindowConfig,
    state: State,
    stylesheet: &Stylesheet,
    update: Update,
    view: View,
) -> renderer::Result<()>
where
    Update: FnMut(&mut State, FrameInfo) -> Signal,
    View: FnMut(&State) -> Node,
    Signal: Into<Refresh>,
{
    App::new(state, stylesheet, update, view).run(config)
}

fn push_unique_id(ids: &mut Vec<String>, candidate: String) {
    if !ids.iter().any(|existing| existing == &candidate) {
        ids.push(candidate);
    }
}

fn interaction_roots(interaction: &ElementInteractionState) -> Vec<usize> {
    let mut roots = Vec::new();

    for root in interaction
        .hovered
        .as_ref()
        .map(|path| path.root)
        .into_iter()
        .chain(interaction.active.as_ref().map(|path| path.root))
    {
        if !roots.contains(&root) {
            roots.push(root);
        }
    }

    roots
}

fn interaction_paths(interaction: &ElementInteractionState) -> Vec<ElementPath> {
    let mut paths = Vec::new();
    for path in interaction
        .hovered
        .as_ref()
        .into_iter()
        .chain(interaction.active.as_ref())
    {
        if !paths.iter().any(|existing| existing == path) {
            paths.push(path.clone());
        }
    }
    paths
}

fn stable_boundary_id_for_path(root: &RenderNode, path: &ElementPath) -> Option<String> {
    let mut deepest = None;
    collect_boundary_id_for_path(root, path, &mut deepest);
    deepest
}

fn collect_boundary_id_for_path(
    node: &RenderNode,
    path: &ElementPath,
    deepest: &mut Option<String>,
) {
    let Some(node_path) = node.element_path.as_ref() else {
        return;
    };
    if !node_path.is_prefix_of(path) {
        return;
    }
    if let Some(element_id) = &node.element_id {
        *deepest = Some(element_id.clone());
    }
    for child in &node.children {
        collect_boundary_id_for_path(child, path, deepest);
    }
}

fn find_unique_node_boundary<'a>(
    node: &'a Node,
    id: &str,
    path: &ElementPath,
) -> UniqueMatch<NodeBoundaryMatch<'a>> {
    let Node::Element(element) = node else {
        return UniqueMatch::None;
    };

    let mut result = if element.id.as_deref() == Some(id) {
        UniqueMatch::Single(NodeBoundaryMatch {
            node,
            path: path.clone(),
        })
    } else {
        UniqueMatch::None
    };

    let mut child_index = 0;
    for child in &element.children {
        let Node::Element(_) = child else {
            continue;
        };
        let child_path = path.with_child(child_index);
        child_index += 1;
        result = merge_unique_match(result, find_unique_node_boundary(child, id, &child_path));
        if matches!(result, UniqueMatch::Multiple) {
            return result;
        }
    }

    result
}

fn find_unique_render_boundary<'a>(
    node: &'a RenderNode,
    id: &str,
) -> UniqueMatch<RenderBoundaryMatch<'a>> {
    let mut result = if node.element_id.as_deref() == Some(id) {
        node.element_path
            .clone()
            .map(|path| RenderBoundaryMatch { node, path })
            .map_or(UniqueMatch::None, UniqueMatch::Single)
    } else {
        UniqueMatch::None
    };

    for child in &node.children {
        result = merge_unique_match(result, find_unique_render_boundary(child, id));
        if matches!(result, UniqueMatch::Multiple) {
            return result;
        }
    }

    result
}

fn merge_unique_match<T>(left: UniqueMatch<T>, right: UniqueMatch<T>) -> UniqueMatch<T> {
    match (left, right) {
        (UniqueMatch::Multiple, _) | (_, UniqueMatch::Multiple) => UniqueMatch::Multiple,
        (UniqueMatch::None, other) | (other, UniqueMatch::None) => other,
        (UniqueMatch::Single(_), UniqueMatch::Single(_)) => UniqueMatch::Multiple,
    }
}

fn find_render_node_mut<'a>(
    node: &'a mut RenderNode,
    path: &ElementPath,
) -> Option<&'a mut RenderNode> {
    if node.element_path.as_ref() == Some(path) {
        return Some(node);
    }
    for child in &mut node.children {
        if let Some(found) = find_render_node_mut(child, path) {
            return Some(found);
        }
    }
    None
}

fn duration_to_us(duration: Duration) -> u64 {
    duration.as_micros().min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::time::Duration;

    use crate::core::{Color, ElementInteractionState, ElementPath, Node, RenderKind, RenderNode};
    use crate::ui;

    use super::{App, Fragment, FragmentApp, Invalidation, Refresh, RefreshTarget, RenderMode};
    use crate::renderer::{FrameInfo, SceneProvider, ViewportSize};
    use crate::style::{Stylesheet, parse_stylesheet};

    #[test]
    fn initial_frame_builds_the_scene() {
        let stylesheet = Stylesheet::default();
        let render_calls = Cell::new(0_u32);
        let mut app = App::new(
            3_u32,
            &stylesheet,
            |_state, _frame| Invalidation::Clean,
            |state| {
                render_calls.set(render_calls.get() + 1);
                ui! {
                    <div id="app">
                        <p class="label">
                            {format!("count {}", state)}
                        </p>
                    </div>
                }
            },
        );

        let scene = app.frame(frame(0));

        assert_eq!(render_calls.get(), 1);
        assert_eq!(text_nodes(&scene), vec!["count 3".to_string()]);
    }

    #[test]
    fn on_invalidation_mode_reuses_the_cached_scene_when_state_is_clean() {
        let stylesheet = Stylesheet::default();
        let render_calls = Cell::new(0_u32);
        let mut app = App::new(
            0_u32,
            &stylesheet,
            |state, frame| {
                if frame.frame_index == 1 {
                    *state = 7;
                    Invalidation::Paint
                } else {
                    Invalidation::Clean
                }
            },
            |state| {
                render_calls.set(render_calls.get() + 1);
                ui! {
                    <div id="app">
                        <p class="label">
                            {format!("count {}", state)}
                        </p>
                    </div>
                }
            },
        );

        let first = app.frame(frame(0));
        let second = app.frame(frame(1));
        let third = app.frame(frame(2));

        assert_eq!(render_calls.get(), 2);
        assert_eq!(text_nodes(&first), vec!["count 0".to_string()]);
        assert_eq!(text_nodes(&second), vec!["count 7".to_string()]);
        assert_eq!(text_nodes(&third), vec!["count 7".to_string()]);
    }

    #[test]
    fn every_frame_mode_rebuilds_even_without_new_invalidations() {
        let stylesheet = Stylesheet::default();
        let render_calls = Cell::new(0_u32);
        let mut app = App::new(
            1_u32,
            &stylesheet,
            |_state, _frame| Invalidation::Clean,
            |state| {
                render_calls.set(render_calls.get() + 1);
                ui! {
                    <div id="app">
                        <p class="label">
                            {format!("count {}", state)}
                        </p>
                    </div>
                }
            },
        )
        .with_render_mode(RenderMode::EveryFrame);

        app.frame(frame(0));
        app.frame(frame(1));

        assert_eq!(render_calls.get(), 2);
        assert_eq!(app.render_mode(), RenderMode::EveryFrame);
        assert_eq!(*app.state(), 1);
    }

    #[test]
    fn manual_invalidation_marks_the_cached_scene_dirty() {
        let stylesheet = Stylesheet::default();
        let render_calls = Cell::new(0_u32);
        let mut app = App::new(
            2_u32,
            &stylesheet,
            |_state, _frame| Invalidation::Clean,
            |state| {
                render_calls.set(render_calls.get() + 1);
                ui! {
                    <div id="app">
                        <p class="label">
                            {format!("count {}", state)}
                        </p>
                    </div>
                }
            },
        );

        app.frame(frame(0));
        app.invalidate(Invalidation::Layout);
        app.frame(frame(1));

        assert_eq!(render_calls.get(), 2);
    }

    #[test]
    fn refresh_merges_fragment_targets_without_duplicates() {
        let refresh = Refresh::fragment("sidebar", Invalidation::Paint).merge(Refresh::fragments(
            ["stats", "sidebar"],
            Invalidation::Layout,
        ));

        assert_eq!(refresh.invalidation, Invalidation::Layout);
        assert_eq!(
            refresh.target,
            RefreshTarget::Fragments(vec!["sidebar".to_string(), "stats".to_string()])
        );
    }

    #[test]
    fn fragment_app_rerenders_only_the_targeted_fragment() {
        let stylesheet = Stylesheet::default();
        let left_calls = Cell::new(0_u32);
        let right_calls = Cell::new(0_u32);
        let mut app = FragmentApp::new(
            (0_u32, 10_u32),
            &stylesheet,
            |state, frame| {
                if frame.frame_index == 1 {
                    state.0 = 7;
                    Refresh::fragment("left", Invalidation::Paint)
                } else {
                    Refresh::clean()
                }
            },
            [
                Fragment::new("left", |state: &(u32, u32)| {
                    left_calls.set(left_calls.get() + 1);
                    ui! {
                        <section id="left">
                            <p>{format!("left {}", state.0)}</p>
                        </section>
                    }
                }),
                Fragment::new("right", |state: &(u32, u32)| {
                    right_calls.set(right_calls.get() + 1);
                    ui! {
                        <section id="right">
                            <p>{format!("right {}", state.1)}</p>
                        </section>
                    }
                }),
            ],
        );

        let first = app.frame(frame(0));
        let second = app.frame(frame(1));
        let third = app.frame(frame(2));

        assert_eq!(left_calls.get(), 2);
        assert_eq!(right_calls.get(), 1);
        assert_eq!(
            text_nodes(&first),
            vec!["left 0".to_string(), "right 10".to_string()]
        );
        assert_eq!(
            text_nodes(&second),
            vec!["left 7".to_string(), "right 10".to_string()]
        );
        assert_eq!(
            text_nodes(&third),
            vec!["left 7".to_string(), "right 10".to_string()]
        );
    }

    #[test]
    fn fragment_app_falls_back_to_a_full_refresh_when_scope_is_unknown() {
        let stylesheet = Stylesheet::default();
        let left_calls = Cell::new(0_u32);
        let right_calls = Cell::new(0_u32);
        let mut app = FragmentApp::new(
            5_u32,
            &stylesheet,
            |state, frame| {
                if frame.frame_index == 1 {
                    *state = 9;
                    Refresh::fragment("missing", Invalidation::Layout)
                } else {
                    Refresh::clean()
                }
            },
            [
                Fragment::new("left", |state: &u32| {
                    left_calls.set(left_calls.get() + 1);
                    ui! {
                        <section id="left">
                            <p>{format!("left {}", state)}</p>
                        </section>
                    }
                }),
                Fragment::new("right", |state: &u32| {
                    right_calls.set(right_calls.get() + 1);
                    ui! {
                        <section id="right">
                            <p>{format!("right {}", state)}</p>
                        </section>
                    }
                }),
            ],
        );

        app.frame(frame(0));
        let refreshed = app.frame(frame(1));

        assert_eq!(left_calls.get(), 2);
        assert_eq!(right_calls.get(), 2);
        assert_eq!(
            text_nodes(&refreshed),
            vec!["left 9".to_string(), "right 9".to_string()]
        );
    }

    #[test]
    fn app_scene_provider_exposes_the_cached_scene_without_cloning_through_frame() {
        let stylesheet = Stylesheet::default();
        let mut app = App::new(
            4_u32,
            &stylesheet,
            |_state, _frame| Invalidation::Clean,
            |state| {
                ui! {
                    <div id="app">
                        <p>{format!("count {}", state)}</p>
                    </div>
                }
            },
        );

        SceneProvider::update(&mut app, frame(0));

        assert_eq!(
            text_nodes(SceneProvider::scene(&app)),
            vec!["count 4".to_string()]
        );
    }

    #[test]
    fn fragment_scene_provider_exposes_the_cached_scene_without_cloning_through_frame() {
        let stylesheet = Stylesheet::default();
        let mut app = FragmentApp::new(
            9_u32,
            &stylesheet,
            |_state, _frame| Refresh::clean(),
            [Fragment::new("stats", |state: &u32| {
                ui! {
                    <section id="stats">
                        <p>{format!("value {}", state)}</p>
                    </section>
                }
            })],
        );

        SceneProvider::update(&mut app, frame(0));

        assert_eq!(
            text_nodes(SceneProvider::scene(&app)),
            vec!["value 9".to_string()]
        );
    }

    #[test]
    fn viewport_changes_trigger_a_layout_rerender() {
        let stylesheet =
            parse_stylesheet("#app { width: 100%; height: 100%; background-color: #ffffff; }")
                .expect("viewport stylesheet should parse");
        let render_calls = Cell::new(0_u32);
        let mut app = App::new(
            (),
            &stylesheet,
            |_state, _frame| Invalidation::Clean,
            |_state| {
                render_calls.set(render_calls.get() + 1);
                ui! {
                    <div id="app"></div>
                }
            },
        );

        app.set_viewport(ViewportSize::new(320, 180));
        let first = app.frame(frame(0));
        let second = app.frame(frame(1));
        app.set_viewport(ViewportSize::new(640, 360));
        let third = app.frame(frame(2));

        assert_eq!(render_calls.get(), 2);
        assert_eq!(first[0].layout.width, 320.0);
        assert_eq!(first[0].layout.height, 180.0);
        assert_eq!(second[0].layout.width, 320.0);
        assert_eq!(third[0].layout.width, 640.0);
        assert_eq!(third[0].layout.height, 360.0);
    }

    #[test]
    fn app_rerenders_when_element_interaction_changes() {
        let stylesheet =
            parse_stylesheet(".button { color: #111111; } .button:hover { color: #2563eb; }")
                .expect("interactive stylesheet should parse");
        let render_calls = Cell::new(0_u32);
        let mut app = App::new(
            (),
            &stylesheet,
            |_state, _frame| Invalidation::Clean,
            |_state| {
                render_calls.set(render_calls.get() + 1);
                ui! {
                    <button class="button">{"hover me"}</button>
                }
            },
        );

        let first = app.frame(frame(0));
        assert!(SceneProvider::set_element_interaction(
            &mut app,
            ElementInteractionState {
                hovered: Some(ElementPath::root(0)),
                active: None,
            },
        ));
        let second = app.frame(frame(1));

        assert_eq!(render_calls.get(), 2);
        assert_eq!(
            first[0].style.foreground,
            crate::core::Color::rgb(17, 17, 17)
        );
        assert_eq!(
            second[0].style.foreground,
            crate::core::Color::rgb(37, 99, 235)
        );
    }

    #[test]
    fn app_descendant_hover_rules_target_the_nearest_stable_id_boundary() {
        let stylesheet = parse_stylesheet(".button:hover .hover-text { color: #2563eb; }")
            .expect("interactive stylesheet should parse");
        let mut app = App::new(
            (),
            &stylesheet,
            |_state, _frame| Refresh::clean(),
            |_state| {
                ui! {
                    <div id="app">
                        <section id="left">
                            <div class="button">
                                <span class="hover-text">{"left"}</span>
                            </div>
                        </section>
                        <section id="right">
                            <div class="button">
                                <span class="hover-text">{"right"}</span>
                            </div>
                        </section>
                    </div>
                }
            },
        );

        app.frame(frame(0));
        assert!(SceneProvider::set_element_interaction(
            &mut app,
            ElementInteractionState {
                hovered: Some(
                    ElementPath::root(0)
                        .with_child(1)
                        .with_child(0)
                        .with_child(0)
                ),
                active: None,
            },
        ));

        assert_eq!(
            app.pending_refresh,
            Refresh::fragment("right", Invalidation::Paint)
        );
    }

    #[test]
    fn app_manual_fragment_refresh_keeps_other_stable_regions_intact() {
        let stylesheet = Stylesheet::default();
        let mut app = App::new(
            (1_u32, 10_u32),
            &stylesheet,
            |state, frame| {
                if frame.frame_index == 1 {
                    state.1 = 42;
                    Refresh::fragment("right", Invalidation::Paint)
                } else {
                    Refresh::clean()
                }
            },
            |state| {
                ui! {
                    <div id="app">
                        <section id="left">
                            <p>{format!("left {}", state.0)}</p>
                        </section>
                        <section id="right">
                            <p>{format!("right {}", state.1)}</p>
                        </section>
                    </div>
                }
            },
        );

        let first = app.frame(frame(0));
        let second = app.frame(frame(1));

        assert_eq!(
            text_nodes(&first),
            vec!["left 1".to_string(), "right 10".to_string()]
        );
        assert_eq!(
            text_nodes(&second),
            vec!["left 1".to_string(), "right 42".to_string()]
        );
        assert_eq!(second[0].children[0].layout, first[0].children[0].layout);
        assert_eq!(second[0].children[1].layout, first[0].children[1].layout);
    }

    #[test]
    fn fragment_app_limits_interaction_rerenders_to_affected_roots() {
        let stylesheet = parse_stylesheet(".button:hover { color: #2563eb; }")
            .expect("interactive stylesheet should parse");
        let left_calls = Cell::new(0_u32);
        let right_calls = Cell::new(0_u32);
        let mut app = FragmentApp::new(
            (),
            &stylesheet,
            |_state, _frame| Refresh::clean(),
            [
                Fragment::new("left", |_state: &()| {
                    left_calls.set(left_calls.get() + 1);
                    ui! {
                        <button class="button">{"left"}</button>
                    }
                }),
                Fragment::new("right", |_state: &()| {
                    right_calls.set(right_calls.get() + 1);
                    ui! {
                        <button class="button">{"right"}</button>
                    }
                }),
            ],
        );

        app.frame(frame(0));
        assert!(SceneProvider::set_element_interaction(
            &mut app,
            ElementInteractionState {
                hovered: Some(ElementPath::root(1)),
                active: None,
            },
        ));
        let scene = app.frame(frame(1));

        assert_eq!(left_calls.get(), 1);
        assert_eq!(right_calls.get(), 2);
        assert_eq!(scene[0].style.foreground, crate::core::Color::BLACK);
        assert_eq!(
            scene[1].style.foreground,
            crate::core::Color::rgb(37, 99, 235)
        );
    }

    #[test]
    fn app_skips_interaction_rerenders_without_interactive_rules() {
        let stylesheet =
            parse_stylesheet(".button { color: #111111; }").expect("stylesheet should parse");
        let render_calls = Cell::new(0_u32);
        let mut app = App::new(
            (),
            &stylesheet,
            |_state, _frame| Invalidation::Clean,
            |_state| {
                render_calls.set(render_calls.get() + 1);
                ui! {
                    <button class="button">{"hover me"}</button>
                }
            },
        );

        app.frame(frame(0));
        assert!(!SceneProvider::set_element_interaction(
            &mut app,
            ElementInteractionState {
                hovered: Some(ElementPath::root(0)),
                active: None,
            },
        ));
        app.frame(frame(1));

        assert_eq!(render_calls.get(), 1);
        assert_eq!(app.pending_refresh, Refresh::clean());
    }

    #[test]
    fn fragment_app_skips_interaction_rerenders_without_interactive_rules() {
        let stylesheet =
            parse_stylesheet(".button { color: #111111; }").expect("stylesheet should parse");
        let render_calls = Cell::new(0_u32);
        let mut app = FragmentApp::new(
            (),
            &stylesheet,
            |_state, _frame| Refresh::clean(),
            [Fragment::new("button", |_state: &()| {
                render_calls.set(render_calls.get() + 1);
                ui! {
                    <button class="button">{"hover me"}</button>
                }
            })],
        );

        app.frame(frame(0));
        assert!(!SceneProvider::set_element_interaction(
            &mut app,
            ElementInteractionState {
                hovered: Some(ElementPath::root(0)),
                active: None,
            },
        ));
        app.frame(frame(1));

        assert_eq!(render_calls.get(), 1);
        assert_eq!(app.pending_refresh, Refresh::clean());
    }

    #[test]
    fn app_promotes_interaction_refresh_to_layout_when_needed() {
        let stylesheet =
            parse_stylesheet(".button:hover { width: 120px; }").expect("stylesheet should parse");
        let mut app = App::new(
            (),
            &stylesheet,
            |_state, _frame| Invalidation::Clean,
            |_state| {
                ui! {
                    <button class="button">{"hover me"}</button>
                }
            },
        );

        app.frame(frame(0));
        assert!(SceneProvider::set_element_interaction(
            &mut app,
            ElementInteractionState {
                hovered: Some(ElementPath::root(0)),
                active: None,
            },
        ));

        assert_eq!(app.pending_refresh, Refresh::full(Invalidation::Layout));
    }

    #[test]
    fn fragment_app_descendant_hover_rules_rerender_only_the_affected_root() {
        let stylesheet = parse_stylesheet(".button:hover .hover-text { color: #2563eb; }")
            .expect("interactive stylesheet should parse");
        let left_calls = Cell::new(0_u32);
        let right_calls = Cell::new(0_u32);
        let mut app = FragmentApp::new(
            (),
            &stylesheet,
            |_state, _frame| Refresh::clean(),
            [
                Fragment::new("left", |_state: &()| {
                    left_calls.set(left_calls.get() + 1);
                    ui! {
                        <div class="button">
                            <span class="hover-text">{"left"}</span>
                        </div>
                    }
                }),
                Fragment::new("right", |_state: &()| {
                    right_calls.set(right_calls.get() + 1);
                    ui! {
                        <div class="button">
                            <span class="hover-text">{"right"}</span>
                        </div>
                    }
                }),
            ],
        );

        app.frame(frame(0));
        assert!(SceneProvider::set_element_interaction(
            &mut app,
            ElementInteractionState {
                hovered: Some(ElementPath::root(1).with_child(0)),
                active: None,
            },
        ));

        assert_eq!(
            app.pending_refresh,
            Refresh::fragment("right", Invalidation::Paint)
        );

        let scene = app.frame(frame(1));

        assert_eq!(left_calls.get(), 1);
        assert_eq!(right_calls.get(), 2);
        assert_eq!(
            scene[0].children[0].style.foreground,
            crate::core::Color::BLACK
        );
        assert_eq!(
            scene[1].children[0].style.foreground,
            crate::core::Color::rgb(37, 99, 235)
        );
    }

    #[test]
    fn fragment_app_promotes_interaction_refresh_to_layout_when_needed() {
        let stylesheet =
            parse_stylesheet(".button:hover { width: 120px; }").expect("stylesheet should parse");
        let mut app = FragmentApp::new(
            (),
            &stylesheet,
            |_state, _frame| Refresh::clean(),
            [Fragment::new("right", |_state: &()| {
                ui! {
                    <button class="button">{"right"}</button>
                }
            })],
        );

        app.frame(frame(0));
        assert!(SceneProvider::set_element_interaction(
            &mut app,
            ElementInteractionState {
                hovered: Some(ElementPath::root(0)),
                active: None,
            },
        ));

        assert_eq!(
            app.pending_refresh,
            Refresh::fragment("right", Invalidation::Layout)
        );
    }

    #[test]
    fn app_advances_color_transitions_without_rebuilding_every_frame() {
        let stylesheet = parse_stylesheet(
            ".button { color: #111111; transition: color 32ms linear; }
             .button.hot { color: #2563eb; }",
        )
        .expect("transition stylesheet should parse");
        let render_calls = Cell::new(0_u32);
        let mut app = App::new(
            false,
            &stylesheet,
            |state, frame| {
                if frame.frame_index == 1 {
                    *state = true;
                    Invalidation::Paint
                } else {
                    Invalidation::Clean
                }
            },
            |state| {
                render_calls.set(render_calls.get() + 1);
                let mut button = Node::element("button").with_class("button");
                if *state {
                    button = button.with_class("hot");
                }
                button.with_child(Node::text("hover me")).into()
            },
        );

        let first = app.frame(frame(0));
        let second = app.frame(frame(1));
        let third = app.frame(frame(2));
        let fourth = app.frame(frame(3));

        assert_eq!(render_calls.get(), 2);
        assert_eq!(first[0].style.foreground, Color::rgb(17, 17, 17));
        assert_ne!(second[0].style.foreground, Color::rgb(17, 17, 17));
        assert_ne!(second[0].style.foreground, Color::rgb(37, 99, 235));
        assert_eq!(third[0].style.foreground, Color::rgb(37, 99, 235));
        assert_eq!(fourth[0].style.foreground, Color::rgb(37, 99, 235));
    }

    #[test]
    fn app_advances_layout_transitions_without_rebuilding_every_frame() {
        let stylesheet = parse_stylesheet(
            ".button { width: 80px; transition: width 32ms linear; }
             .button.hot { width: 160px; }",
        )
        .expect("layout transition stylesheet should parse");
        let render_calls = Cell::new(0_u32);
        let mut app = App::new(
            false,
            &stylesheet,
            |state, frame| {
                if frame.frame_index == 1 {
                    *state = true;
                    Invalidation::Layout
                } else {
                    Invalidation::Clean
                }
            },
            |state| {
                render_calls.set(render_calls.get() + 1);
                let mut button = Node::element("button").with_class("button");
                if *state {
                    button = button.with_class("hot");
                }
                button.with_child(Node::text("hover me")).into()
            },
        );

        let first = app.frame(frame(0));
        let second = app.frame(frame(1));
        let third = app.frame(frame(2));
        let fourth = app.frame(frame(3));

        assert_eq!(render_calls.get(), 2);
        assert_eq!(first[0].layout.width, 80.0);
        assert!((second[0].layout.width - 120.0).abs() < 0.01);
        assert_eq!(third[0].layout.width, 160.0);
        assert_eq!(fourth[0].layout.width, 160.0);
    }

    fn frame(frame_index: u64) -> FrameInfo {
        FrameInfo {
            frame_index,
            delta: Duration::from_millis(16),
        }
    }

    fn text_nodes(scene: &[RenderNode]) -> Vec<String> {
        let mut text = Vec::new();

        for node in scene {
            collect_text(node, &mut text);
        }

        text
    }

    fn collect_text(node: &RenderNode, text: &mut Vec<String>) {
        if let RenderKind::Text(content) = &node.kind {
            text.push(content.clone());
        }

        for child in &node.children {
            collect_text(child, text);
        }
    }
}
