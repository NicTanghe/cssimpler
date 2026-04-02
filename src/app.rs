mod scene_transition;

use std::marker::PhantomData;
use std::time::Duration;

use self::scene_transition::SceneTransition;
use crate::core::{ElementInteractionState, Node, RenderNode};
use crate::renderer::{self, FrameInfo, SceneProvider, ViewportSize, WindowConfig};
use crate::style::{
    Stylesheet, build_render_tree_in_viewport_with_interaction_at_root,
    build_render_tree_with_interaction_at_root,
};

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
        let update = &mut self.update;
        let state = &mut self.state;
        let refresh = update(state, frame).into();
        self.pending_refresh = std::mem::take(&mut self.pending_refresh).merge(refresh);

        if self.needs_rerender() {
            self.rebuild_scene();
        }

        self.advance_scene_transition(frame.delta);
    }

    fn needs_rerender(&self) -> bool {
        self.cached_scene.is_none()
            || matches!(self.render_mode, RenderMode::EveryFrame)
            || self.pending_refresh.needs_rerender()
    }

    fn rebuild_scene(&mut self) {
        let view = &mut self.view;
        let tree = view(&self.state);
        let scene = vec![if let Some(viewport) = self.viewport {
            build_render_tree_in_viewport_with_interaction_at_root(
                &tree,
                self.stylesheet,
                viewport.width,
                viewport.height,
                &self.interaction,
                0,
            )
        } else {
            build_render_tree_with_interaction_at_root(&tree, self.stylesheet, &self.interaction, 0)
        }];
        self.replace_scene(scene);
        self.pending_refresh = Refresh::clean();
    }

    fn scene(&self) -> &[RenderNode] {
        self.cached_scene
            .as_deref()
            .expect("app scene should be cached after the first frame")
    }

    fn replace_scene(&mut self, scene: Vec<RenderNode>) {
        if let Some(previous) = self.cached_scene.clone()
            && let Some(transition) = SceneTransition::new(previous, scene.clone())
        {
            self.cached_scene = Some(transition.sample());
            self.scene_transition = Some(transition);
            return;
        }

        self.cached_scene = Some(scene);
        self.scene_transition = None;
    }

    fn advance_scene_transition(&mut self, delta: Duration) {
        let Some(transition) = self.scene_transition.as_mut() else {
            return;
        };
        self.cached_scene = Some(transition.advance(delta));
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
        invalidation.into()
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
        let update = &mut self.update;
        let state = &mut self.state;
        let refresh = update(state, frame).into();
        self.pending_refresh = std::mem::take(&mut self.pending_refresh).merge(refresh);

        if self.needs_rerender() {
            self.refresh_scene();
        }

        self.advance_scene_transition(frame.delta);
    }

    fn needs_rerender(&self) -> bool {
        self.cached_scene.is_none()
            || matches!(self.render_mode, RenderMode::EveryFrame)
            || self.pending_refresh.needs_rerender()
    }

    fn refresh_scene(&mut self) {
        let must_full_refresh = self.cached_scene.is_none()
            || matches!(self.render_mode, RenderMode::EveryFrame)
            || matches!(self.pending_refresh.target, RefreshTarget::Full);

        if must_full_refresh {
            self.rebuild_all_fragments();
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

        if !self.refresh_fragments(&fragment_ids) {
            self.rebuild_all_fragments();
        }

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
        let Some(existing_scene) = self.cached_scene.clone() else {
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

        let mut scene = existing_scene;
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
        if let Some(previous) = self.cached_scene.clone()
            && let Some(transition) = SceneTransition::new(previous, scene.clone())
        {
            self.cached_scene = Some(transition.sample());
            self.scene_transition = Some(transition);
            return;
        }

        self.cached_scene = Some(scene);
        self.scene_transition = None;
    }

    fn advance_scene_transition(&mut self, delta: Duration) {
        let Some(transition) = self.scene_transition.as_mut() else {
            return;
        };
        self.cached_scene = Some(transition.advance(delta));
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
