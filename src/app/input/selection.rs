use super::*;
use crate::core::{RenderKind, TextSelectionStyle};

#[derive(Clone)]
struct TextRun {
    path: ElementPath,
    text: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Endpoint {
    run: usize,
    byte: usize,
}

#[derive(Default)]
pub(super) struct DocumentSelection {
    runs: Vec<TextRun>,
    anchor: Option<Endpoint>,
    focus: Option<Endpoint>,
    dragging: bool,
}

impl DocumentSelection {
    pub fn clear(&mut self) -> bool {
        let changed = self.range().is_some();
        *self = Self::default();
        changed
    }

    pub fn reconcile(&mut self, scene: Option<&[RenderNode]>, world: &RuntimeWorld) -> bool {
        if self.runs.is_empty() {
            return false;
        }
        let current = collect_runs(scene.unwrap_or_default(), world);
        if current.len() != self.runs.len()
            || current
                .iter()
                .zip(&self.runs)
                .any(|(a, b)| a.path != b.path || a.text != b.text)
        {
            return self.clear();
        }
        false
    }

    pub fn begin(
        &mut self,
        scene: Option<&[RenderNode]>,
        world: &RuntimeWorld,
        position: Option<PointerPosition>,
    ) -> bool {
        let Some(scene) = scene else {
            return false;
        };
        let Some(position) = position else {
            return false;
        };
        let Some(path) = renderer::hit_test_element_path(scene, position.x, position.y) else {
            return false;
        };
        self.runs = collect_runs(scene, world);
        let Some(run) = self.runs.iter().position(|run| run.path == path) else {
            self.clear();
            return false;
        };
        let Some(node) = find_render_node_at_path(scene, &path) else {
            return false;
        };
        let endpoint = Endpoint {
            run,
            byte: hit_text(node, &self.runs[run].text, position),
        };
        self.anchor = Some(endpoint);
        self.focus = Some(endpoint);
        self.dragging = true;
        false
    }

    pub fn update(&mut self, scene: Option<&[RenderNode]>, position: PointerPosition) -> bool {
        if !self.dragging {
            return false;
        }
        let Some(scene) = scene else {
            return false;
        };
        let mut closest = None;
        let mut distance = f32::INFINITY;
        for (run, text) in self.runs.iter().enumerate() {
            let Some(node) = find_render_node_at_path(scene, &text.path) else {
                continue;
            };
            let dx = (node.layout.x - position.x).max(0.0)
                + (position.x - node.layout.x - node.layout.width).max(0.0);
            let dy = (node.layout.y - position.y).max(0.0)
                + (position.y - node.layout.y - node.layout.height).max(0.0);
            let d = dx * dx + dy * dy;
            if d < distance {
                distance = d;
                closest = Some(Endpoint {
                    run,
                    byte: hit_text(node, &text.text, position),
                });
            }
        }
        let changed = closest != self.focus;
        self.focus = closest;
        changed
    }

    pub fn finish(&mut self) {
        self.dragging = false;
    }

    pub fn select_all(&mut self, scene: Option<&[RenderNode]>, world: &RuntimeWorld) -> bool {
        self.runs = collect_runs(scene.unwrap_or_default(), world);
        let Some(last) = self.runs.last() else {
            return self.clear();
        };
        self.anchor = Some(Endpoint { run: 0, byte: 0 });
        self.focus = Some(Endpoint {
            run: self.runs.len() - 1,
            byte: last.text.len(),
        });
        self.dragging = false;
        true
    }

    fn range(&self) -> Option<(Endpoint, Endpoint)> {
        let a = self.anchor?;
        let b = self.focus?;
        if a == b {
            return None;
        }
        Some(if (a.run, a.byte) < (b.run, b.byte) {
            (a, b)
        } else {
            (b, a)
        })
    }

    pub fn selected_text(&self) -> Option<String> {
        let (a, b) = self.range()?;
        Some(
            (a.run..=b.run)
                .map(|i| {
                    let run = &self.runs[i];
                    let start = if i == a.run { a.byte } else { 0 };
                    let end = if i == b.run { b.byte } else { run.text.len() };
                    &run.text[start..end]
                })
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }

    pub fn decorate(
        &self,
        scene: &mut [RenderNode],
        stylesheet: &Stylesheet,
        world: &RuntimeWorld,
    ) {
        let Some((a, b)) = self.range() else {
            return;
        };
        for i in a.run..=b.run {
            let run = &self.runs[i];
            let start = if i == a.run { a.byte } else { 0 };
            let end = if i == b.run { b.byte } else { run.text.len() };
            let style = world
                .root_as_node(run.path.root)
                .and_then(|root| {
                    resolve_selection_style_at_path(
                        &root,
                        stylesheet,
                        world.interaction(),
                        &run.path,
                    )
                })
                .unwrap_or_else(TextSelectionStyle::default);
            decorate_run(scene, run, start, end, &style);
        }
    }
}

fn decorate_run(
    nodes: &mut [RenderNode],
    run: &TextRun,
    start: usize,
    end: usize,
    style: &TextSelectionStyle,
) {
    for node in nodes {
        if node.element_path.as_ref() == Some(&run.path)
            && matches!(&node.kind, RenderKind::Text(text) if text == &run.text)
        {
            node.text_edit = Some(TextEditDecoration {
                composition: None,
                backgrounds: None,
                caret: None,
                selection: Some(TextSelectionRange {
                    start,
                    end,
                    style: style.clone(),
                }),
            });
        }
        decorate_run(&mut node.children, run, start, end, style);
    }
}

fn collect_runs(scene: &[RenderNode], world: &RuntimeWorld) -> Vec<TextRun> {
    fn visit(
        node: &RenderNode,
        world: &RuntimeWorld,
        roots: &mut HashMap<usize, Option<Node>>,
        runs: &mut Vec<TextRun>,
    ) {
        if let Some(path) = &node.element_path {
            if let Some(root) = roots
                .entry(path.root)
                .or_insert_with(|| world.root_as_node(path.root))
                .as_ref()
            {
                if let Some((_, modal)) = last_modal(&root, &ElementPath::root(path.root)) {
                    if !path.children.starts_with(&modal.children)
                        && !modal.children.starts_with(&path.children)
                    {
                        return;
                    }
                }
                if let Some(element) = element_at_path(&root, path) {
                    if ["input", "textarea", "button", "select", "svg"]
                        .iter()
                        .any(|tag| element.tag.eq_ignore_ascii_case(tag))
                        || element.attribute("href").is_some()
                    {
                        return;
                    }
                }
            }
            if let RenderKind::Text(text) = &node.kind
                && !text.is_empty()
            {
                runs.push(TextRun {
                    path: path.clone(),
                    text: text.clone(),
                });
            }
        }
        for child in &node.children {
            visit(child, world, roots, runs);
        }
    }
    let mut runs = Vec::new();
    let mut roots = HashMap::new();
    for node in scene {
        visit(node, world, &mut roots, &mut runs);
    }
    runs
}

fn hit_text(node: &RenderNode, _text: &str, position: PointerPosition) -> usize {
    renderer::text_index_at_position(node, position.x, position.y).unwrap_or(0)
}
