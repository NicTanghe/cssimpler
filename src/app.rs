use std::marker::PhantomData;

use crate::core::{Node, RenderNode};
use crate::renderer::{self, FrameInfo, SceneProvider, WindowConfig};
use crate::style::{Stylesheet, build_render_tree};

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
    render_mode: RenderMode,
    pending_refresh: Refresh,
    cached_scene: Option<Vec<RenderNode>>,
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
            render_mode: RenderMode::OnInvalidation,
            pending_refresh: Refresh::full(Invalidation::Structure),
            cached_scene: None,
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
    }

    fn needs_rerender(&self) -> bool {
        self.cached_scene.is_none()
            || matches!(self.render_mode, RenderMode::EveryFrame)
            || self.pending_refresh.needs_rerender()
    }

    fn rebuild_scene(&mut self) {
        let view = &mut self.view;
        let tree = view(&self.state);
        self.cached_scene = Some(vec![build_render_tree(&tree, self.stylesheet)]);
        self.pending_refresh = Refresh::clean();
    }

    fn scene(&self) -> &[RenderNode] {
        self.cached_scene
            .as_deref()
            .expect("app scene should be cached after the first frame")
    }
}

impl<'a, State, Update, View, Signal> SceneProvider for App<'a, State, Update, View, Signal>
where
    Update: FnMut(&mut State, FrameInfo) -> Signal,
    View: FnMut(&State) -> Node,
    Signal: Into<Refresh>,
{
    fn update(&mut self, frame: FrameInfo) {
        self.advance(frame);
    }

    fn scene(&self) -> &[RenderNode] {
        App::scene(self)
    }
}

pub struct FragmentApp<'a, State, Update, Signal = Invalidation> {
    state: State,
    stylesheet: &'a Stylesheet,
    update: Update,
    fragments: Vec<Fragment<'a, State>>,
    render_mode: RenderMode,
    pending_refresh: Refresh,
    cached_scene: Option<Vec<RenderNode>>,
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
            render_mode: RenderMode::OnInvalidation,
            pending_refresh: Refresh::full(Invalidation::Structure),
            cached_scene: None,
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
            .map(|fragment| build_render_tree(&fragment.render(&self.state), self.stylesheet))
            .collect();
        self.cached_scene = Some(scene);
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
            replacements.push((index, build_render_tree(&node, self.stylesheet)));
        }

        let scene = self
            .cached_scene
            .as_mut()
            .expect("cached scene existence was checked above");
        for (index, node) in replacements {
            scene[index] = node;
        }

        true
    }

    fn scene(&self) -> &[RenderNode] {
        self.cached_scene
            .as_deref()
            .expect("fragment app scene should be cached after the first frame")
    }
}

impl<'a, State, Update, Signal> SceneProvider for FragmentApp<'a, State, Update, Signal>
where
    Update: FnMut(&mut State, FrameInfo) -> Signal,
    Signal: Into<Refresh>,
{
    fn update(&mut self, frame: FrameInfo) {
        self.advance(frame);
    }

    fn scene(&self) -> &[RenderNode] {
        FragmentApp::scene(self)
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

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::time::Duration;

    use crate::core::{RenderKind, RenderNode};
    use crate::ui;

    use super::{App, Fragment, FragmentApp, Invalidation, Refresh, RefreshTarget, RenderMode};
    use crate::renderer::{FrameInfo, SceneProvider};
    use crate::style::Stylesheet;

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
