use anyhow::Result;
use cssimpler::app::{App, Invalidation, Refresh, RuntimeStats, latest_runtime_stats};
use cssimpler::core::Node;
use cssimpler::renderer::{
    FrameInfo, FramePaintMode, FramePaintReason, FrameTimingStats, WindowConfig,
    latest_frame_timing_stats,
};
use cssimpler::style::{Stylesheet, parse_stylesheet};
use cssimpler::ui;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

const ACTION_GENERATE_TEXT: u64 = 1 << 0;
const INITIAL_SEED: u64 = 0x9E37_79B9_7F4A_7C15;

const TITLE_MODIFIERS: &[&str] = &[
    "elastic",
    "layered",
    "dense",
    "incremental",
    "measured",
    "overscanned",
    "glassy",
    "luminous",
    "spiraling",
    "wide-wrap",
];

const TITLE_NOUNS: &[&str] = &[
    "annotations",
    "captions",
    "margins",
    "footnotes",
    "headings",
    "ledgers",
    "fragments",
    "baselines",
    "callouts",
    "chapters",
];

const SUBJECTS: &[&str] = &[
    "the reading lane",
    "each margin note",
    "the stacked viewport",
    "this long article",
    "every caption block",
    "the scrolling ledger",
    "the layout pass",
    "each glyph ribbon",
    "the measured paragraph",
    "every soft card",
];

const VERBS: &[&str] = &[
    "threads",
    "measures",
    "reflows",
    "stretches",
    "packs",
    "folds",
    "layers",
    "carries",
    "stacks",
    "fans out",
];

const ADJECTIVES: &[&str] = &[
    "microtypographic",
    "high-contrast",
    "counter-rotating",
    "narrow-guttered",
    "soft-edged",
    "wide-wrap",
    "signal-heavy",
    "ledger-like",
    "multi-clause",
    "ink-dense",
    "slow-blooming",
    "precision-tuned",
];

const OBJECTS: &[&str] = &[
    "annotations",
    "line breaks",
    "baseline hints",
    "footnote rails",
    "chapter labels",
    "wrap boundaries",
    "render queues",
    "measure strips",
    "index cards",
    "callout stacks",
    "caption clusters",
    "numeric markers",
];

const QUALIFIERS: &[&str] = &[
    "under a colder palette",
    "inside a padded shell",
    "beside a drifting ribbon of digits",
    "with little breathing room",
    "through a tall scrolling viewport",
    "while the card stack keeps widening",
    "inside a split dashboard",
    "with a narrow sidebar watching",
];

const CONTINUATIONS: &[&str] = &[
    "before the next paint lands",
    "while the scroll range keeps growing",
    "so the next rerender has more to absorb",
    "without losing the headline rhythm",
    "while every paragraph keeps wrapping",
    "so the long body stays easy to scan",
    "before the footer locks into place",
    "while the viewport keeps collecting text",
];

const CONNECTORS: &[&str] = &[
    "meanwhile",
    "in practice",
    "under load",
    "for the next pass",
    "by comparison",
    "at the same time",
    "in the wider sheet",
    "across the full pane",
];

static ACTIONS: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
struct TextStressState {
    generation: u64,
    seed: u64,
    document: GeneratedDocument,
    last_frame_ms: u128,
    renderer_stats: FrameTimingStats,
    app_stats: RuntimeStats,
    pending_perf_refresh: bool,
}

impl Default for TextStressState {
    fn default() -> Self {
        let generation = 1;
        let seed = INITIAL_SEED;

        Self {
            generation,
            seed,
            document: generate_document(seed, generation),
            last_frame_ms: 0,
            renderer_stats: FrameTimingStats::default(),
            app_stats: RuntimeStats::default(),
            pending_perf_refresh: false,
        }
    }
}

impl TextStressState {
    fn regenerate(&mut self) {
        self.generation = self.generation.saturating_add(1);
        self.seed = next_seed(self.seed, self.generation);
        self.document = generate_document(self.seed, self.generation);
        self.pending_perf_refresh = true;
    }
}

#[derive(Clone, Debug)]
struct GeneratedDocument {
    title: String,
    sections: Vec<GeneratedSection>,
}

#[derive(Clone, Debug)]
struct GeneratedSection {
    heading: String,
    paragraphs: Vec<String>,
}

fn main() -> Result<()> {
    let config = WindowConfig::new("cssimpler / text render stress", 1440, 920);

    App::new(TextStressState::default(), stylesheet(), update, build_ui)
        .run(config)
        .map_err(Into::into)
}

fn update(state: &mut TextStressState, frame: FrameInfo) -> Refresh {
    state.last_frame_ms = frame.delta.as_millis();
    state.renderer_stats = latest_frame_timing_stats();
    state.app_stats = latest_runtime_stats();

    let actions = ACTIONS.swap(0, Ordering::Relaxed);
    if actions & ACTION_GENERATE_TEXT != 0 {
        state.regenerate();
        return Refresh::full(Invalidation::Layout);
    }

    if state.pending_perf_refresh {
        state.pending_perf_refresh = false;
        return Refresh::fragment("hud", Invalidation::Layout);
    }

    Refresh::clean()
}

fn build_ui(state: &TextStressState) -> Node {
    ui! {
        <div id="app">
            {build_hud(state)}
            {build_story_shell(state)}
        </div>
    }
}

fn build_hud(state: &TextStressState) -> Node {
    ui! {
        <section id="hud" class="hud">
            <div class="control-row">
                <button class="generate-button" type="button" onclick={generate_random_text}>
                    Generate random text
                </button>
            </div>
            {build_metric_row(state)}
        </section>
    }
}

fn build_metric_row(state: &TextStressState) -> Node {
    ui! {
        <div class="metric-row">
            {stat_chip("dt", format!("{} ms", state.last_frame_ms))}
            {stat_chip("app view", format_us(state.app_stats.view_us))}
            {stat_chip("tree build", format_us(state.app_stats.render_tree_us))}
            {stat_chip("scene swap", format_us(state.app_stats.scene_swap_us))}
            {stat_chip("transition", format_us(state.app_stats.transition_us))}
            {stat_chip("scene prep", format_us(state.renderer_stats.scene_prep_us))}
            {stat_chip("paint", format_us(state.renderer_stats.paint_us))}
            {stat_chip("present", format_us(state.renderer_stats.present_us))}
            {stat_chip("frame total", format_us(state.renderer_stats.total_us))}
            {stat_chip("paint mode", paint_mode_label(state.renderer_stats))}
            {stat_chip(
                "paint reason",
                paint_reason_label(state.renderer_stats.paint_reason).to_string(),
            )}
            {stat_chip("dirty regions", state.renderer_stats.dirty_regions.to_string())}
            {stat_chip("dirty jobs", state.renderer_stats.dirty_jobs.to_string())}
            {stat_chip("damage", format_pixels(state.renderer_stats.damage_pixels))}
            {stat_chip("painted", format_pixels(state.renderer_stats.painted_pixels))}
            {stat_chip("scene passes", state.renderer_stats.scene_passes.to_string())}
            {stat_chip("workers", state.renderer_stats.render_workers.to_string())}
        </div>
    }
}

fn stat_chip(label: impl Into<String>, value: impl Into<String>) -> Node {
    let label = label.into();
    let value = value.into();

    ui! {
        <div class="metric-chip">
            <p class="metric-label">
                {label}
            </p>
            <p class="metric-value">
                {value}
            </p>
        </div>
    }
}

fn build_story_shell(state: &TextStressState) -> Node {
    ui! {
        <section class="story-shell">
            <div class="story-viewport">
                {build_document(&state.document)}
            </div>
        </section>
    }
}

fn build_document(document: &GeneratedDocument) -> Node {
    let mut story = Node::element("article")
        .with_class("story")
        .with_child(build_story_intro(document));

    for (index, section) in document.sections.iter().enumerate() {
        story = story.with_child(build_story_section(index, section));
    }

    story.into()
}

fn build_story_intro(document: &GeneratedDocument) -> Node {
    Node::element("section")
        .with_class("story-intro")
        .with_child(text_block("h1", "story-title", &document.title))
        .into()
}

fn build_story_section(index: usize, section: &GeneratedSection) -> Node {
    let mut card = Node::element("section")
        .with_class("story-section")
        .with_class(section_variant_class(index))
        .with_child(text_block("h2", "section-title", &section.heading));

    for paragraph in &section.paragraphs {
        card = card.with_child(text_block("p", "section-copy", paragraph));
    }

    card.into()
}

fn text_block(tag: &str, class_name: &str, text: impl Into<String>) -> Node {
    Node::element(tag)
        .with_class(class_name)
        .with_child(Node::text(text.into()))
        .into()
}

fn generate_random_text() {
    ACTIONS.fetch_or(ACTION_GENERATE_TEXT, Ordering::Relaxed);
}

fn stylesheet() -> &'static Stylesheet {
    static STYLESHEET: OnceLock<Stylesheet> = OnceLock::new();

    STYLESHEET.get_or_init(|| {
        parse_stylesheet(include_str!("text_render_stress.css"))
            .expect("text render stress stylesheet should stay valid")
    })
}

fn generate_document(seed: u64, generation: u64) -> GeneratedDocument {
    let mut rng = TextRng::new(seed ^ generation.wrapping_mul(0xD6E8_FEB8_6659_FD93));
    let section_count = rng.range_usize(5, 8);
    let mut sections = Vec::with_capacity(section_count);

    for _ in 0..section_count {
        sections.push(generate_section(&mut rng));
    }

    let title = format!(
        "{} {} and {} {}",
        title_word(&mut rng),
        rng.choose(TITLE_NOUNS),
        title_word(&mut rng),
        rng.choose(TITLE_NOUNS),
    );

    GeneratedDocument { title, sections }
}

fn generate_section(rng: &mut TextRng) -> GeneratedSection {
    let paragraph_count = rng.range_usize(4, 7);
    let mut paragraphs = Vec::with_capacity(paragraph_count);

    for _ in 0..paragraph_count {
        paragraphs.push(build_paragraph(rng));
    }

    GeneratedSection {
        heading: format!("{} {}", title_word(rng), rng.choose(TITLE_NOUNS)),
        paragraphs,
    }
}

fn build_paragraph(rng: &mut TextRng) -> String {
    let sentence_count = rng.range_usize(4, 8);
    let mut paragraph = String::new();

    for sentence_index in 0..sentence_count {
        if sentence_index > 0 {
            paragraph.push(' ');
        }
        paragraph.push_str(&build_sentence(rng));
    }

    paragraph
}

fn build_sentence(rng: &mut TextRng) -> String {
    match rng.range_usize(0, 4) {
        0 => format!(
            "{} {} {} {} {}, {}.",
            capitalize(rng.choose(SUBJECTS)),
            rng.choose(VERBS),
            rng.choose(ADJECTIVES),
            rng.choose(OBJECTS),
            rng.choose(QUALIFIERS),
            build_tail_clause(rng),
        ),
        1 => format!(
            "{} {} {} in a {} pattern; {}, {} {} {}.",
            capitalize(rng.choose(SUBJECTS)),
            rng.choose(VERBS),
            rng.choose(OBJECTS),
            rng.choose(ADJECTIVES),
            rng.choose(CONNECTORS),
            rng.choose(SUBJECTS),
            rng.choose(VERBS),
            rng.choose(OBJECTS),
        ),
        2 => format!(
            "When {} {} {}, {} {} {} {}.",
            rng.choose(SUBJECTS),
            rng.choose(VERBS),
            rng.choose(OBJECTS),
            rng.choose(SUBJECTS),
            rng.choose(VERBS),
            rng.choose(ADJECTIVES),
            rng.choose(OBJECTS),
        ),
        _ => format!(
            "{} {} {} across {} columns, {} markers, and {} wrapped clauses.",
            capitalize(rng.choose(SUBJECTS)),
            rng.choose(VERBS),
            rng.choose(OBJECTS),
            rng.range_usize(2, 8),
            rng.range_usize(12, 80),
            rng.range_usize(3, 16),
        ),
    }
}

fn build_tail_clause(rng: &mut TextRng) -> String {
    format!("{} {}", rng.choose(CONTINUATIONS), rng.choose(QUALIFIERS))
}

fn title_word(rng: &mut TextRng) -> String {
    capitalize(rng.choose(TITLE_MODIFIERS))
}

fn capitalize(value: &str) -> String {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };

    let mut result = String::with_capacity(value.len());
    result.push(first.to_ascii_uppercase());
    result.push_str(chars.as_str());
    result
}

fn next_seed(current: u64, generation: u64) -> u64 {
    current
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407)
        .wrapping_add(generation.wrapping_mul(0x9E37_79B9_7F4A_7C15))
}

fn format_us(duration_us: u64) -> String {
    format!("{:.2} ms", duration_us as f64 / 1000.0)
}

fn paint_mode_label(stats: FrameTimingStats) -> String {
    match stats.paint_mode {
        FramePaintMode::Idle => "idle".to_string(),
        FramePaintMode::Full => {
            if stats.render_workers > 1 {
                format!("full x{}", stats.render_workers)
            } else {
                "full".to_string()
            }
        }
        FramePaintMode::Incremental => {
            format!("incremental {}r/{}j", stats.dirty_regions, stats.dirty_jobs)
        }
    }
}

fn paint_reason_label(reason: FramePaintReason) -> &'static str {
    match reason {
        FramePaintReason::Idle => "idle",
        FramePaintReason::FullRedraw => "full redraw",
        FramePaintReason::DirtyRegionLimit => "dirty-region limit",
        FramePaintReason::DirtyAreaLimit => "dirty-area limit",
        FramePaintReason::FragmentedDamage => "fragmented damage",
        FramePaintReason::IncrementalDamage => "small damage",
    }
}

fn format_pixels(pixels: usize) -> String {
    if pixels >= 1_000_000 {
        format!("{:.2}M px", pixels as f64 / 1_000_000.0)
    } else if pixels >= 1_000 {
        format!("{:.1}K px", pixels as f64 / 1_000.0)
    } else {
        format!("{pixels} px")
    }
}

fn section_variant_class(index: usize) -> &'static str {
    match index % 4 {
        0 => "story-section-amber",
        1 => "story-section-sky",
        2 => "story-section-mint",
        _ => "story-section-rose",
    }
}

#[derive(Clone, Copy, Debug)]
struct TextRng {
    state: u64,
}

impl TextRng {
    fn new(seed: u64) -> Self {
        Self { state: seed | 1 }
    }

    fn next_u64(&mut self) -> u64 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 7;
        self.state ^= self.state << 17;
        self.state
    }

    fn range_usize(&mut self, start: usize, end: usize) -> usize {
        assert!(start < end, "range start must be smaller than range end");
        start + (self.next_u64() as usize % (end - start))
    }

    fn choose<'a>(&mut self, values: &'a [&'a str]) -> &'a str {
        values[self.range_usize(0, values.len())]
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ACTIONS, GeneratedDocument, Invalidation, Refresh, TextStressState, generate_document,
        generate_random_text, update,
    };
    use cssimpler::renderer::FrameInfo;
    use std::sync::atomic::Ordering;
    use std::sync::{Mutex, MutexGuard, OnceLock};
    use std::time::Duration;

    #[test]
    fn generated_document_is_large_enough_to_pressure_wrapping() {
        let document = generate_document(0x1234_5678_9ABC_DEF0, 7);
        let stats = document_stats(&document);

        assert!(stats.section_count >= 5);
        assert!(stats.paragraph_count >= 20);
        assert!(stats.word_count > 700);
        assert!(stats.char_count > 4_000);
    }

    #[test]
    fn generator_is_seeded_deterministically() {
        let first = generate_document(0xCAFE_BABE_F00D_F00D, 3);
        let second = generate_document(0xCAFE_BABE_F00D_F00D, 3);

        assert_eq!(first.title, second.title);
        assert_eq!(first.sections[0].heading, second.sections[0].heading);
        assert_eq!(
            first.sections[0].paragraphs[0],
            second.sections[0].paragraphs[0]
        );
    }

    #[test]
    fn update_regenerates_the_document_when_the_button_action_is_queued() {
        let _guard = action_lock();
        ACTIONS.store(0, Ordering::Relaxed);
        let mut state = TextStressState::default();
        let previous_seed = state.seed;
        let previous_title = state.document.title.clone();

        generate_random_text();
        let refresh = update(&mut state, frame(1));

        assert_eq!(refresh, Refresh::full(Invalidation::Layout));
        assert_ne!(state.seed, previous_seed);
        assert_ne!(state.document.title, previous_title);
        assert!(state.pending_perf_refresh);
        assert_eq!(ACTIONS.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn update_emits_a_follow_up_hud_refresh_for_perf_stats() {
        let _guard = action_lock();
        ACTIONS.store(0, Ordering::Relaxed);
        let mut state = TextStressState::default();

        generate_random_text();
        let _ = update(&mut state, frame(1));
        let refresh = update(&mut state, frame(2));

        assert_eq!(refresh, Refresh::fragment("hud", Invalidation::Layout));
        assert!(!state.pending_perf_refresh);
    }

    #[derive(Debug)]
    struct DocumentStats {
        section_count: usize,
        paragraph_count: usize,
        word_count: usize,
        char_count: usize,
    }

    fn document_stats(document: &GeneratedDocument) -> DocumentStats {
        let mut paragraph_count = 0;
        let mut word_count = count_words(&document.title);
        let mut char_count = document.title.chars().count();

        for section in &document.sections {
            word_count += count_words(&section.heading);
            char_count += section.heading.chars().count();

            for paragraph in &section.paragraphs {
                paragraph_count += 1;
                word_count += count_words(paragraph);
                char_count += paragraph.chars().count();
            }
        }

        DocumentStats {
            section_count: document.sections.len(),
            paragraph_count,
            word_count,
            char_count,
        }
    }

    fn count_words(value: &str) -> usize {
        value.split_whitespace().count()
    }

    fn action_lock() -> MutexGuard<'static, ()> {
        static TEST_ACTION_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

        TEST_ACTION_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("test action lock should not be poisoned")
    }

    fn frame(frame_index: u64) -> FrameInfo {
        FrameInfo {
            frame_index,
            delta: Duration::from_millis(16),
        }
    }
}
