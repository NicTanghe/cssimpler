mod scene_transition;

use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use std::time::Instant;

use self::scene_transition::SceneTransition;
use crate::core::{
    ElementInteractionState, ElementPath, Node, RenderNode, RuntimeDirtyClass, RuntimeDirtyFlags,
    RuntimeSyncAction, RuntimeSyncPolicy, RuntimeViewport, RuntimeWorld,
};
use crate::renderer::{self, FrameInfo, SceneProvider, ViewportSize, WindowConfig};
use crate::style::{
    Stylesheet, extract_render_tree, layout_resolved_render_tree_in_viewport,
    rebuild_resolved_render_tree_with_cached_layout, resolve_render_tree_with_interaction_at_path,
    resolve_render_tree_with_interaction_at_root,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimePhase {
    StructuralUpdate,
    InteractionSync,
    StyleResolution,
    LayoutSync,
    RenderExtraction,
    TransitionAdvance,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RuntimeStats {
    pub view_us: u64,
    pub render_tree_us: u64,
    pub scene_swap_us: u64,
    pub transition_us: u64,
    pub structural_update_us: u64,
    pub interaction_us: u64,
    pub style_resolution_us: u64,
    pub layout_sync_us: u64,
    pub render_extraction_us: u64,
    pub phase_order: Vec<RuntimePhase>,
    pub rerendered: bool,
    pub transition_active: bool,
}

impl RuntimeStats {
    fn record_phase(&mut self, phase: RuntimePhase, elapsed: Duration) {
        let micros = duration_to_us(elapsed);
        self.phase_order.push(phase);
        match phase {
            RuntimePhase::StructuralUpdate => {
                self.structural_update_us = self.structural_update_us.saturating_add(micros);
            }
            RuntimePhase::InteractionSync => {
                self.interaction_us = self.interaction_us.saturating_add(micros);
            }
            RuntimePhase::StyleResolution => {
                self.style_resolution_us = self.style_resolution_us.saturating_add(micros);
            }
            RuntimePhase::LayoutSync => {
                self.layout_sync_us = self.layout_sync_us.saturating_add(micros);
            }
            RuntimePhase::RenderExtraction => {
                self.render_extraction_us = self.render_extraction_us.saturating_add(micros);
            }
            RuntimePhase::TransitionAdvance => {
                self.transition_us = self.transition_us.saturating_add(micros);
            }
        }
        self.render_tree_us = self
            .structural_update_us
            .saturating_add(self.interaction_us)
            .saturating_add(self.style_resolution_us)
            .saturating_add(self.layout_sync_us)
            .saturating_add(self.render_extraction_us);
    }
}

static RUNTIME_STATS: OnceLock<Mutex<RuntimeStats>> = OnceLock::new();

pub fn latest_runtime_stats() -> RuntimeStats {
    runtime_stats_store()
        .lock()
        .expect("runtime stats mutex should not be poisoned")
        .clone()
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

enum RootRenderMode<'a> {
    FullLayout,
    CachedLayout(&'a RenderNode),
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

fn sync_runtime_world(
    runtime_world: &mut RuntimeWorld,
    root_index: usize,
    tree: &Node,
    policy: RuntimeSyncPolicy,
    dirty_class: RuntimeDirtyClass,
    stats: &mut RuntimeStats,
) -> RuntimeSyncAction {
    let structural_start = Instant::now();
    let sync = runtime_world.sync_root(root_index, tree, policy, dirty_class);
    stats.record_phase(RuntimePhase::StructuralUpdate, structural_start.elapsed());
    sync.action
}

fn sync_runtime_interaction(runtime_world: &mut RuntimeWorld, stats: &mut RuntimeStats) {
    let interaction_start = Instant::now();
    runtime_world.sync_interaction_components();
    stats.record_phase(RuntimePhase::InteractionSync, interaction_start.elapsed());
}

fn render_root_with_schedule(
    runtime_world: &RuntimeWorld,
    root_index: usize,
    stylesheet: &Stylesheet,
    viewport: Option<ViewportSize>,
    mode: RootRenderMode<'_>,
    stats: &mut RuntimeStats,
) -> Option<RenderNode> {
    let root = runtime_world.root_as_node(root_index)?;

    let style_start = Instant::now();
    let resolved = resolve_render_tree_with_interaction_at_root(
        &root,
        stylesheet,
        runtime_world.interaction(),
        root_index,
    );
    stats.record_phase(RuntimePhase::StyleResolution, style_start.elapsed());

    match mode {
        RootRenderMode::FullLayout => {
            let layout_start = Instant::now();
            let mut layout = layout_resolved_render_tree_in_viewport(
                &resolved,
                viewport.map(|viewport| (viewport.width, viewport.height)),
            );
            stats.record_phase(RuntimePhase::LayoutSync, layout_start.elapsed());

            let extract_start = Instant::now();
            let node = extract_render_tree(&mut layout);
            stats.record_phase(RuntimePhase::RenderExtraction, extract_start.elapsed());
            Some(node)
        }
        RootRenderMode::CachedLayout(template) => {
            let extract_start = Instant::now();
            let node = rebuild_resolved_render_tree_with_cached_layout(&resolved, template)?;
            stats.record_phase(RuntimePhase::RenderExtraction, extract_start.elapsed());
            Some(node)
        }
    }
}

fn render_boundary_with_schedule(
    resolved: &crate::style::ResolvedRenderTree,
    element_path: ElementPath,
    viewport: Option<ViewportSize>,
    mode: RootRenderMode<'_>,
    stats: &mut RuntimeStats,
) -> Option<RenderNode> {
    match mode {
        RootRenderMode::FullLayout => {
            let layout_start = Instant::now();
            let mut layout = layout_resolved_render_tree_in_viewport(
                resolved,
                viewport.map(|viewport| (viewport.width, viewport.height)),
            );
            stats.record_phase(RuntimePhase::LayoutSync, layout_start.elapsed());

            let extract_start = Instant::now();
            let node = extract_render_tree(&mut layout);
            stats.record_phase(RuntimePhase::RenderExtraction, extract_start.elapsed());
            Some(node)
        }
        RootRenderMode::CachedLayout(template) => {
            if template.element_path.as_ref() != Some(&element_path) {
                return None;
            }
            let extract_start = Instant::now();
            let node = rebuild_resolved_render_tree_with_cached_layout(resolved, template)?;
            stats.record_phase(RuntimePhase::RenderExtraction, extract_start.elapsed());
            Some(node)
        }
    }
}

fn root_render_mode<'a>(
    dirty_flags: RuntimeDirtyFlags,
    template: Option<&'a RenderNode>,
) -> RootRenderMode<'a> {
    if dirty_flags.layout || dirty_flags.structure {
        RootRenderMode::FullLayout
    } else if let Some(template) = template {
        RootRenderMode::CachedLayout(template)
    } else {
        RootRenderMode::FullLayout
    }
}

pub struct App<'a, State, Update, View, Signal = Invalidation> {
    state: State,
    stylesheet: &'a Stylesheet,
    update: Update,
    view: View,
    runtime_world: RuntimeWorld,
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
            runtime_world: RuntimeWorld::default(),
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

    pub fn needs_redraw(&self) -> bool {
        self.cached_scene.is_none()
            || self.pending_refresh.needs_rerender()
            || self.scene_transition.is_some()
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn runtime_world(&self) -> &RuntimeWorld {
        &self.runtime_world
    }

    pub fn set_viewport(&mut self, viewport: ViewportSize) {
        let viewport = RuntimeViewport::new(viewport.width, viewport.height);
        if self.runtime_world.viewport() != Some(viewport) {
            self.runtime_world.set_viewport(Some(viewport));
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

        let dirty_class = runtime_dirty_class(
            self.pending_refresh.invalidation,
            self.cached_scene.is_none(),
            matches!(self.render_mode, RenderMode::EveryFrame),
        );
        sync_runtime_world(
            &mut self.runtime_world,
            0,
            &tree,
            runtime_sync_policy(
                self.cached_scene.is_none(),
                self.pending_refresh.invalidation,
            ),
            dirty_class,
            stats,
        );
        sync_runtime_interaction(&mut self.runtime_world, stats);
        let dirty_flags = self.runtime_world.root_dirty_flags(0);
        let scene = vec![
            render_root_with_schedule(
                &self.runtime_world,
                0,
                self.stylesheet,
                self.viewport(),
                root_render_mode(
                    dirty_flags,
                    self.cached_scene.as_ref().and_then(|scene| scene.first()),
                ),
                stats,
            )
            .expect("runtime world should contain the app root before rendering"),
        ];

        let scene_swap_start = Instant::now();
        self.replace_scene(scene);
        stats.scene_swap_us = duration_to_us(scene_swap_start.elapsed());
        stats.rerendered = true;
        self.runtime_world.clear_dirty_flags();
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

        let sync = sync_runtime_world(
            &mut self.runtime_world,
            0,
            &tree,
            RuntimeSyncPolicy::PreferPatch,
            runtime_dirty_class(self.pending_refresh.invalidation, false, false),
            stats,
        );
        if matches!(sync, RuntimeSyncAction::Rebuilt) {
            return false;
        }
        sync_runtime_interaction(&mut self.runtime_world, stats);
        let dirty_flags = self.runtime_world.root_dirty_flags(0);
        let needs_layout = dirty_flags.layout || dirty_flags.structure;

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
            let style_start = Instant::now();
            let resolved = resolve_render_tree_with_interaction_at_path(
                node_match.node,
                self.stylesheet,
                self.runtime_world.interaction(),
                &node_match.path,
            );
            stats.record_phase(RuntimePhase::StyleResolution, style_start.elapsed());

            let mode = if needs_layout {
                RootRenderMode::FullLayout
            } else {
                RootRenderMode::CachedLayout(render_match.node)
            };
            let Some(node) = render_boundary_with_schedule(
                &resolved,
                node_match.path.clone(),
                self.viewport(),
                mode,
                stats,
            ) else {
                return false;
            };
            replacements.push((render_match.path, node));
        }

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
        self.runtime_world.clear_dirty_flags();
        true
    }

    fn viewport(&self) -> Option<ViewportSize> {
        self.runtime_world
            .viewport()
            .map(|viewport| ViewportSize::new(viewport.width, viewport.height))
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
        if let Some(scene) = self.cached_scene.as_mut() {
            transition.advance(delta, scene);
        }
        stats.record_phase(RuntimePhase::TransitionAdvance, sample_start.elapsed());
        if !transition.is_active() {
            self.scene_transition = None;
        }
    }

    fn set_interaction_state(&mut self, interaction: ElementInteractionState) -> bool {
        if self.runtime_world.interaction() == &interaction {
            return false;
        }

        let previous = self.runtime_world.interaction().clone();
        let refresh = self.interaction_refresh(&previous, &interaction);
        self.runtime_world.set_interaction(interaction);
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

    fn redraw_schedule(&self) -> renderer::RedrawSchedule {
        match self.render_mode {
            RenderMode::EveryFrame => renderer::RedrawSchedule::EveryFrame,
            RenderMode::OnInvalidation => renderer::RedrawSchedule::OnInvalidation,
        }
    }

    fn needs_redraw(&self) -> bool {
        App::needs_redraw(self)
    }
}

pub struct FragmentApp<'a, State, Update, Signal = Invalidation> {
    state: State,
    stylesheet: &'a Stylesheet,
    update: Update,
    fragments: Vec<Fragment<'a, State>>,
    fragment_indices: HashMap<String, usize>,
    runtime_world: RuntimeWorld,
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
        let fragments = fragments.into_iter().collect::<Vec<_>>();
        Self {
            state,
            stylesheet,
            update,
            fragment_indices: build_fragment_indices(&fragments),
            fragments,
            runtime_world: RuntimeWorld::default(),
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

    pub fn needs_redraw(&self) -> bool {
        self.cached_scene.is_none()
            || self.pending_refresh.needs_rerender()
            || self.scene_transition.is_some()
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn runtime_world(&self) -> &RuntimeWorld {
        &self.runtime_world
    }

    pub fn set_viewport(&mut self, viewport: ViewportSize) {
        let viewport = RuntimeViewport::new(viewport.width, viewport.height);
        if self.runtime_world.viewport() != Some(viewport) {
            self.runtime_world.set_viewport(Some(viewport));
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
            || matches!(self.pending_refresh.target, RefreshTarget::Full);

        if must_full_refresh {
            self.rebuild_all_fragments(stats);
            stats.rerendered = true;
            self.pending_refresh = Refresh::clean();
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
            self.rebuild_all_fragments(stats);
        }

        stats.rerendered = true;
        self.pending_refresh = Refresh::clean();
    }

    fn rebuild_all_fragments(&mut self, stats: &mut RuntimeStats) {
        let policy = runtime_sync_policy(
            self.cached_scene.is_none(),
            self.pending_refresh.invalidation,
        );
        let dirty_class = runtime_dirty_class(
            self.pending_refresh.invalidation,
            self.cached_scene.is_none(),
            matches!(self.render_mode, RenderMode::EveryFrame),
        );
        let mut scene = Vec::with_capacity(self.fragments.len());
        for index in 0..self.fragments.len() {
            let node = self.fragments[index].render(&self.state);
            sync_runtime_world(
                &mut self.runtime_world,
                index,
                &node,
                policy,
                dirty_class,
                stats,
            );
            sync_runtime_interaction(&mut self.runtime_world, stats);
            let dirty_flags = self.runtime_world.root_dirty_flags(index);
            scene.push(
                render_root_with_schedule(
                    &self.runtime_world,
                    index,
                    self.stylesheet,
                    self.viewport(),
                    root_render_mode(
                        dirty_flags,
                        self.cached_scene
                            .as_ref()
                            .and_then(|scene| scene.get(index)),
                    ),
                    stats,
                )
                .expect("runtime world should contain the fragment root before rendering"),
            );
        }
        self.replace_scene(scene);
        self.runtime_world.clear_dirty_flags();
    }

    fn refresh_fragments(&mut self, ids: &[String], stats: &mut RuntimeStats) -> bool {
        let Some(existing_scene) = self.cached_scene.as_ref() else {
            return false;
        };
        if existing_scene.len() != self.fragments.len() {
            return false;
        }

        let mut replacements = Vec::with_capacity(ids.len());
        for id in ids {
            let Some(&index) = self.fragment_indices.get(id) else {
                return false;
            };
            let node = self.fragments[index].render(&self.state);
            let sync = sync_runtime_world(
                &mut self.runtime_world,
                index,
                &node,
                RuntimeSyncPolicy::PreferPatch,
                runtime_dirty_class(self.pending_refresh.invalidation, false, false),
                stats,
            );
            if matches!(sync, RuntimeSyncAction::Rebuilt) {
                return false;
            }
            sync_runtime_interaction(&mut self.runtime_world, stats);
            let dirty_flags = self.runtime_world.root_dirty_flags(index);
            replacements.push((
                index,
                render_root_with_schedule(
                    &self.runtime_world,
                    index,
                    self.stylesheet,
                    self.viewport(),
                    root_render_mode(dirty_flags, existing_scene.get(index)),
                    stats,
                )
                .expect("runtime world should contain the fragment root before rendering"),
            ));
        }

        let mut scene = self
            .cached_scene
            .take()
            .expect("fragment app scene should be cached while refreshing fragments");
        for (index, node) in replacements {
            scene[index] = node;
        }

        self.replace_scene(scene);
        self.runtime_world.clear_dirty_flags();

        true
    }

    fn viewport(&self) -> Option<ViewportSize> {
        self.runtime_world
            .viewport()
            .map(|viewport| ViewportSize::new(viewport.width, viewport.height))
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
        if let Some(scene) = self.cached_scene.as_mut() {
            transition.advance(delta, scene);
        }
        stats.record_phase(RuntimePhase::TransitionAdvance, sample_start.elapsed());
        if !transition.is_active() {
            self.scene_transition = None;
        }
    }

    fn set_interaction_state(&mut self, interaction: ElementInteractionState) -> bool {
        if self.runtime_world.interaction() == &interaction {
            return false;
        }

        let previous = self.runtime_world.interaction().clone();
        let refresh = self.interaction_refresh(&previous, &interaction);
        self.runtime_world.set_interaction(interaction);
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

    fn redraw_schedule(&self) -> renderer::RedrawSchedule {
        match self.render_mode {
            RenderMode::EveryFrame => renderer::RedrawSchedule::EveryFrame,
            RenderMode::OnInvalidation => renderer::RedrawSchedule::OnInvalidation,
        }
    }

    fn needs_redraw(&self) -> bool {
        FragmentApp::needs_redraw(self)
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

fn build_fragment_indices<State>(fragments: &[Fragment<'_, State>]) -> HashMap<String, usize> {
    let mut indices = HashMap::with_capacity(fragments.len());
    for (index, fragment) in fragments.iter().enumerate() {
        indices.entry(fragment.id().to_string()).or_insert(index);
    }
    indices
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

fn runtime_sync_policy(initial_build: bool, invalidation: Invalidation) -> RuntimeSyncPolicy {
    if initial_build || matches!(invalidation, Invalidation::Structure) {
        RuntimeSyncPolicy::ForceRebuild
    } else {
        RuntimeSyncPolicy::PreferPatch
    }
}

fn runtime_dirty_class(
    invalidation: Invalidation,
    initial_build: bool,
    every_frame_refresh: bool,
) -> RuntimeDirtyClass {
    match invalidation {
        Invalidation::Clean if initial_build => RuntimeDirtyClass::Structure,
        Invalidation::Clean if every_frame_refresh => RuntimeDirtyClass::Paint,
        Invalidation::Clean => RuntimeDirtyClass::Clean,
        Invalidation::Paint => RuntimeDirtyClass::Paint,
        Invalidation::Layout => RuntimeDirtyClass::Layout,
        Invalidation::Structure => RuntimeDirtyClass::Structure,
    }
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

    use super::{
        App, Fragment, FragmentApp, Invalidation, Refresh, RefreshTarget, RenderMode, RuntimePhase,
        latest_runtime_stats,
    };
    use crate::renderer::{
        FrameInfo, SceneProvider, ViewportSize, render_scene_update, render_to_buffer,
    };
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
    fn app_runtime_stats_record_explicit_phase_order() {
        let stylesheet = Stylesheet::default();
        let mut app = App::new(
            (),
            &stylesheet,
            |_state, _frame| Invalidation::Clean,
            |_state| {
                ui! {
                    <div id="app">
                        <p>{"hello"}</p>
                    </div>
                }
            },
        );

        app.frame(frame(0));
        let stats = latest_runtime_stats();

        assert_eq!(
            stats.phase_order,
            vec![
                RuntimePhase::StructuralUpdate,
                RuntimePhase::InteractionSync,
                RuntimePhase::StyleResolution,
                RuntimePhase::LayoutSync,
                RuntimePhase::RenderExtraction,
            ]
        );
    }

    #[test]
    fn paint_only_refresh_skips_the_layout_phase() {
        let stylesheet = parse_stylesheet(
            ".button { color: #111111; }
             .button.hot { color: #2563eb; }",
        )
        .expect("paint-only stylesheet should parse");
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
                let mut button = Node::element("button").with_class("button");
                if *state {
                    button = button.with_class("hot");
                }
                button.with_child(Node::text("hover me")).into()
            },
        );

        app.frame(frame(0));
        app.frame(frame(1));
        let stats = latest_runtime_stats();

        assert!(!stats.phase_order.contains(&RuntimePhase::LayoutSync));
        assert!(stats.phase_order.contains(&RuntimePhase::RenderExtraction));
    }

    #[test]
    fn app_populates_the_runtime_world_from_view_output() {
        let stylesheet = Stylesheet::default();
        let mut app = App::new(
            3_u32,
            &stylesheet,
            |_state, _frame| Invalidation::Clean,
            |state| {
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

        assert_eq!(app.runtime_world().entity_count(), 3);
        let root = app
            .runtime_world()
            .root_entity(0)
            .expect("runtime world should contain the app root");
        let label = app
            .runtime_world()
            .entity(root)
            .expect("root entity should exist")
            .children[0];
        let text = app
            .runtime_world()
            .entity(label)
            .expect("label entity should exist")
            .children[0];
        assert_eq!(
            app.runtime_world()
                .entity(root)
                .expect("root entity should exist")
                .computed
                .element_path,
            Some(ElementPath::root(0))
        );

        let roundtrip = app
            .runtime_world()
            .root_as_node(0)
            .expect("app root should roundtrip through the runtime world");
        let Node::Element(root_element) = roundtrip else {
            panic!("runtime world root should stay an element");
        };
        let Node::Element(label_element) = &root_element.children[0] else {
            panic!("runtime world child should stay an element");
        };
        let Node::Text(text_content) = &label_element.children[0] else {
            panic!("runtime world grandchild should stay text");
        };

        assert_eq!(text_content, "count 3");
        assert!(app.runtime_world().entity(text).is_some());
    }

    #[test]
    fn app_reuses_runtime_entities_for_paint_only_refreshes() {
        let stylesheet = parse_stylesheet(
            ".button { color: #111111; }
             .button.hot { color: #2563eb; }",
        )
        .expect("stylesheet should parse");
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
                let mut button = Node::element("button").with_class("button");
                if *state {
                    button = button.with_class("hot");
                }
                button.with_child(Node::text("hover me")).into()
            },
        );

        let first = app.frame(frame(0));
        let root = app
            .runtime_world()
            .root_entity(0)
            .expect("runtime world should contain the app root");
        let text = app
            .runtime_world()
            .entity(root)
            .expect("root entity should exist")
            .children[0];

        let second = app.frame(frame(1));

        assert_eq!(app.runtime_world().root_entity(0), Some(root));
        assert_eq!(
            app.runtime_world()
                .entity(root)
                .expect("root entity should exist")
                .children[0],
            text
        );
        assert_eq!(first[0].style.foreground, Color::rgb(17, 17, 17));
        assert_eq!(second[0].style.foreground, Color::rgb(37, 99, 235));
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
    fn fragment_app_can_refresh_multiple_targeted_fragments_in_one_pass() {
        let stylesheet = Stylesheet::default();
        let left_calls = Cell::new(0_u32);
        let middle_calls = Cell::new(0_u32);
        let right_calls = Cell::new(0_u32);
        let mut app = FragmentApp::new(
            (1_u32, 2_u32, 3_u32),
            &stylesheet,
            |state, frame| {
                if frame.frame_index == 1 {
                    state.0 = 10;
                    state.2 = 30;
                    Refresh::fragments(["right", "left"], Invalidation::Paint)
                } else {
                    Refresh::clean()
                }
            },
            [
                Fragment::new("left", |state: &(u32, u32, u32)| {
                    left_calls.set(left_calls.get() + 1);
                    ui! {
                        <section id="left">
                            <p>{format!("left {}", state.0)}</p>
                        </section>
                    }
                }),
                Fragment::new("middle", |state: &(u32, u32, u32)| {
                    middle_calls.set(middle_calls.get() + 1);
                    ui! {
                        <section id="middle">
                            <p>{format!("middle {}", state.1)}</p>
                        </section>
                    }
                }),
                Fragment::new("right", |state: &(u32, u32, u32)| {
                    right_calls.set(right_calls.get() + 1);
                    ui! {
                        <section id="right">
                            <p>{format!("right {}", state.2)}</p>
                        </section>
                    }
                }),
            ],
        );

        let first = app.frame(frame(0));
        let second = app.frame(frame(1));

        assert_eq!(
            text_nodes(&first),
            vec![
                "left 1".to_string(),
                "middle 2".to_string(),
                "right 3".to_string()
            ]
        );
        assert_eq!(
            text_nodes(&second),
            vec![
                "left 10".to_string(),
                "middle 2".to_string(),
                "right 30".to_string()
            ]
        );
        assert_eq!(left_calls.get(), 2);
        assert_eq!(middle_calls.get(), 1);
        assert_eq!(right_calls.get(), 2);
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

    #[test]
    fn app_keeps_a_3d_card_near_its_center_during_transform_transitions() {
        let stylesheet = parse_stylesheet(
            r#"
            #app {
              width: 420px;
              height: 420px;
              padding: 60px;
            }

            .parent {
              position: relative;
              width: 290px;
              height: 300px;
              perspective: 1000px;
            }

            .card {
              position: relative;
              width: 100%;
              height: 100%;
              border-radius: 50px;
              background: #00ffd6;
              transform-style: preserve-3d;
              transition: transform 500ms linear;
            }

            .parent.hot .card {
              transform: rotateX(10deg) rotateY(-10deg) scale3d(1.02, 1.02, 1.02);
            }

            .glass {
              position: absolute;
              inset: 8px;
              border-radius: 55px;
              border-top-right-radius: 100%;
              background: rgba(255, 255, 255, 0.7);
              transform: translate3d(0px, 0px, 25px);
            }
            "#,
        )
        .expect("3d transition stylesheet should parse");
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
                let mut parent = Node::element("div").with_class("parent");
                if *state {
                    parent = parent.with_class("hot");
                }
                let card = Node::element("div")
                    .with_class("card")
                    .with_child(Node::element("div").with_class("glass").into());
                Node::element("div")
                    .with_id("app")
                    .with_child(parent.with_child(card.into()).into())
                    .into()
            },
        );

        let idle = app.frame(frame(0));
        let mid = app.frame(FrameInfo {
            frame_index: 1,
            delta: Duration::from_millis(250),
        });
        let final_scene = app.frame(FrameInfo {
            frame_index: 2,
            delta: Duration::from_millis(250),
        });

        let clear = Color::rgb(255, 0, 255);
        let clear_packed = ((clear.r as u32) << 16) | ((clear.g as u32) << 8) | clear.b as u32;
        let mut idle_buffer = vec![0_u32; 420 * 420];
        let mut mid_buffer = vec![0_u32; 420 * 420];
        let mut final_buffer = vec![0_u32; 420 * 420];

        render_to_buffer(&idle, &mut idle_buffer, 420, 420, clear);
        render_to_buffer(&mid, &mut mid_buffer, 420, 420, clear);
        render_to_buffer(&final_scene, &mut final_buffer, 420, 420, clear);

        let idle_bounds =
            visible_bounds(&idle_buffer, 420, 420, clear_packed).expect("idle card should render");
        let mid_bounds = visible_bounds(&mid_buffer, 420, 420, clear_packed).unwrap_or_else(|| {
            panic!(
                "mid-transition card should render; midpoint transform was {:?}",
                mid[0].children[0].children[0].style.transform.operations
            )
        });
        let final_bounds = visible_bounds(&final_buffer, 420, 420, clear_packed)
            .expect("final card should render");

        let idle_center_x = (idle_bounds.0 + idle_bounds.2) as f32 * 0.5;
        let idle_center_y = (idle_bounds.1 + idle_bounds.3) as f32 * 0.5;
        let mid_center_x = (mid_bounds.0 + mid_bounds.2) as f32 * 0.5;
        let mid_center_y = (mid_bounds.1 + mid_bounds.3) as f32 * 0.5;
        let final_center_x = (final_bounds.0 + final_bounds.2) as f32 * 0.5;
        let final_center_y = (final_bounds.1 + final_bounds.3) as f32 * 0.5;

        assert!((mid_center_x - idle_center_x).abs() < 18.0);
        assert!((mid_center_y - idle_center_y).abs() < 18.0);
        assert!((final_center_x - idle_center_x).abs() < 24.0);
        assert!((final_center_y - idle_center_y).abs() < 24.0);
    }

    #[test]
    fn incremental_render_matches_full_redraw_for_a_mid_transition_3d_card() {
        let stylesheet = parse_stylesheet(
            r#"
            #app {
              width: 420px;
              height: 420px;
              padding: 60px;
            }

            .parent {
              position: relative;
              width: 290px;
              height: 300px;
              perspective: 1000px;
            }

            .card {
              position: relative;
              width: 100%;
              height: 100%;
              border-radius: 50px;
              background: #00ffd6;
              transform-style: preserve-3d;
              transition: transform 500ms linear;
            }

            .parent.hot .card {
              transform: rotateX(10deg) rotateY(-10deg) scale3d(1.02, 1.02, 1.02);
            }

            .glass {
              position: absolute;
              inset: 8px;
              border-radius: 55px;
              border-top-right-radius: 100%;
              background: rgba(255, 255, 255, 0.7);
              transform: translate3d(0px, 0px, 25px);
            }
            "#,
        )
        .expect("3d transition stylesheet should parse");
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
                let mut parent = Node::element("div").with_class("parent");
                if *state {
                    parent = parent.with_class("hot");
                }
                let card = Node::element("div")
                    .with_class("card")
                    .with_child(Node::element("div").with_class("glass").into());
                Node::element("div")
                    .with_id("app")
                    .with_child(parent.with_child(card.into()).into())
                    .into()
            },
        );

        let idle = app.frame(frame(0));
        let mid = app.frame(FrameInfo {
            frame_index: 1,
            delta: Duration::from_millis(250),
        });

        let clear = Color::rgb(255, 0, 255);
        let mut incremental = vec![0_u32; 420 * 420];
        let mut full = vec![0_u32; 420 * 420];

        render_to_buffer(&idle, &mut incremental, 420, 420, clear);
        render_scene_update(&idle, &mid, &mut incremental, 420, 420, clear);
        render_to_buffer(&mid, &mut full, 420, 420, clear);

        assert_eq!(incremental, full);
    }

    fn frame(frame_index: u64) -> FrameInfo {
        FrameInfo {
            frame_index,
            delta: Duration::from_millis(16),
        }
    }

    fn visible_bounds(
        buffer: &[u32],
        width: usize,
        height: usize,
        clear_packed: u32,
    ) -> Option<(i32, i32, i32, i32)> {
        let mut x0 = width as i32;
        let mut y0 = height as i32;
        let mut x1 = 0_i32;
        let mut y1 = 0_i32;

        for y in 0..height as i32 {
            for x in 0..width as i32 {
                if buffer[y as usize * width + x as usize] == clear_packed {
                    continue;
                }
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }

        (x1 > x0 && y1 > y0).then_some((x0, y0, x1, y1))
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
