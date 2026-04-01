use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;
use cssimpler::app::{App, Invalidation};
use cssimpler::core::Node;
use cssimpler::fonts::register_font_file;
use cssimpler::renderer::{FrameInfo, WindowConfig};
use cssimpler::style::{Stylesheet, parse_stylesheet};
use cssimpler::ui;

const INITIAL_CARD_COUNT: u64 = 4;

const ACTION_TOGGLE_SIDEBAR: u64 = 1 << 0;
const ACTION_ADD_CARD: u64 = 1 << 1;

static ACTIONS: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
struct SidebarState {
    collapsed: bool,
    next_card_seed: u64,
    cards: Vec<QueueCard>,
}

impl Default for SidebarState {
    fn default() -> Self {
        let cards = (0..INITIAL_CARD_COUNT)
            .map(generate_card)
            .collect::<Vec<_>>();

        Self {
            collapsed: false,
            next_card_seed: INITIAL_CARD_COUNT,
            cards,
        }
    }
}

impl SidebarState {
    fn queue_count_label(&self) -> String {
        let noun = if self.cards.len() == 1 {
            "card"
        } else {
            "cards"
        };
        format!("{} {noun}", self.cards.len())
    }

    fn queue_status_label(&self) -> String {
        if self.collapsed {
            return format!(
                "{} hidden while the rail is collapsed.",
                self.queue_count_label()
            );
        }

        format!("{} in queue.", self.queue_count_label())
    }

    fn queue_interaction_label(&self) -> &'static str {
        "wheel, drag, or click track"
    }
}

#[derive(Clone, Debug)]
struct QueueCard {
    title: String,
    note: String,
    accent_class: &'static str,
}

#[derive(Clone, Copy)]
struct CardTemplate {
    title: &'static str,
    note: &'static str,
    accent_class: &'static str,
}

const CARD_LIBRARY: [CardTemplate; 8] = [
    CardTemplate {
        title: "Receipt audit",
        note: "Color pass for the receipts lane.",
        accent_class: "queue-card-sky",
    },
    CardTemplate {
        title: "Archive tidy",
        note: "Trim stale labels before handoff.",
        accent_class: "queue-card-mint",
    },
    CardTemplate {
        title: "Spec review",
        note: "Check spacing against the new rail size.",
        accent_class: "queue-card-indigo",
    },
    CardTemplate {
        title: "Patch notes",
        note: "Bundle the latest sidebar tweaks.",
        accent_class: "queue-card-amber",
    },
    CardTemplate {
        title: "Card ingest",
        note: "Queue up another content sample.",
        accent_class: "queue-card-rose",
    },
    CardTemplate {
        title: "Support mail",
        note: "Collect the next customer thread.",
        accent_class: "queue-card-teal",
    },
    CardTemplate {
        title: "Status pulse",
        note: "Mark the feed for the next refresh.",
        accent_class: "queue-card-lilac",
    },
    CardTemplate {
        title: "Shelf sync",
        note: "Prep the left rail for another card.",
        accent_class: "queue-card-sand",
    },
];

fn main() -> Result<()> {
    register_demo_fonts();
    let config = WindowConfig::new("cssimpler / collapsible sidebar", 1280, 760);

    App::new(SidebarState::default(), stylesheet(), update, build_ui)
        .run(config)
        .map_err(Into::into)
}

fn update(state: &mut SidebarState, _frame: FrameInfo) -> Invalidation {
    let actions = ACTIONS.swap(0, Ordering::Relaxed);
    if actions == 0 {
        return Invalidation::Clean;
    }

    if actions & ACTION_TOGGLE_SIDEBAR != 0 {
        state.collapsed = !state.collapsed;
    }

    if actions & ACTION_ADD_CARD != 0 {
        let seed = state.next_card_seed;
        state.cards.push(generate_card(seed));
        state.next_card_seed = seed + 1;
    }

    Invalidation::Layout
}

fn build_ui(state: &SidebarState) -> Node {
    Node::element("div")
        .with_id("app")
        .with_child(build_workspace(state))
        .into()
}

fn build_workspace(state: &SidebarState) -> Node {
    Node::element("section")
        .with_class("workspace")
        .with_child(build_sidebar(state))
        .with_child(build_content(state))
        .into()
}

fn build_sidebar(state: &SidebarState) -> Node {
    let mut sidebar = Node::element("aside").with_class("sidebar");
    if state.collapsed {
        sidebar = sidebar.with_class("sidebar-collapsed");
    }

    sidebar = sidebar
        .with_child(build_sidebar_header(state))
        .with_child(build_menu(state));

    if !state.collapsed {
        sidebar = sidebar.with_child(build_queue_panel(state));
    }

    sidebar.into()
}

fn build_sidebar_header(state: &SidebarState) -> Node {
    if state.collapsed {
        return Node::element("div")
            .with_class("sidebar-header-compact")
            .with_child(text_element("p", "rail-badge", "UI"))
            .with_child(
                Node::element("button")
                    .with_class("rail-toggle")
                    .on_click(toggle_sidebar)
                    .with_child(Node::text(">"))
                    .into(),
            )
            .into();
    }

    ui! {
        <div class="sidebar-header">
            <div class="sidebar-copy">
                <p class="sidebar-kicker">
                    {"Segoe UI demo"}
                </p>
                <h2 class="sidebar-title">
                    {"Collections"}
                </h2>
            </div>
            <button class="rail-toggle" onclick={toggle_sidebar}>
                {"<"}
            </button>
        </div>
    }
}

fn build_menu(state: &SidebarState) -> Node {
    let items = [
        ("Browse", "BR", "menu-item-active"),
        ("Pinned", "PI", "menu-item-soft"),
        ("Activity", "AC", "menu-item-soft"),
        ("Archive", "AR", "menu-item-soft"),
    ];

    let mut menu = Node::element("nav").with_class("menu-stack");
    for (label, glyph, accent_class) in items {
        menu = menu.with_child(build_menu_item(label, glyph, accent_class, state.collapsed));
    }

    menu.into()
}

fn build_menu_item(label: &str, glyph: &str, accent_class: &'static str, collapsed: bool) -> Node {
    let mut item = Node::element("div")
        .with_class("menu-item")
        .with_class(accent_class)
        .with_child(text_element("p", "menu-glyph", glyph));

    if !collapsed {
        item = item.with_child(text_element("p", "menu-label", label));
    }

    item.into()
}

fn build_queue_panel(state: &SidebarState) -> Node {
    Node::element("section")
        .with_class("queue-panel")
        .with_child(ui! {
            <div class="queue-header">
                <p class="queue-kicker">
                    {"Left panel queue"}
                </p>
                <p class="queue-meta">
                    {state.queue_count_label()}
                </p>
            </div>
        })
        .with_child(build_queue_shell(state))
        .into()
}

fn build_queue_shell(state: &SidebarState) -> Node {
    Node::element("div")
        .with_class("queue-shell")
        .with_child(build_queue_viewport(state))
        .into()
}

fn build_queue_viewport(state: &SidebarState) -> Node {
    let mut viewport = Node::element("div").with_class("queue-viewport");

    for card in &state.cards {
        viewport = viewport.with_child(build_queue_card(card));
    }

    viewport.into()
}

fn build_queue_card(card: &QueueCard) -> Node {
    Node::element("article")
        .with_class("queue-card")
        .with_class(card.accent_class)
        .with_child(text_element("p", "queue-card-title", &card.title))
        .with_child(text_element("p", "queue-card-note", &card.note))
        .into()
}

fn build_content(state: &SidebarState) -> Node {
    ui! {
        <section class="content">
            <div class="content-card">
                <p class="panel-kicker">
                    {"What this scene shows"}
                </p>
                <h2 class="panel-title">
                    {"A collapsible menu with a real engine-owned queue scrollbar"}
                </h2>
                <p class="panel-copy">
                    {"Add a card, scroll the queue, or collapse the rail. The sidebar now uses the built-in scrollbar instead of the old fake thumb."}
                </p>
                <div class="content-actions">
                    <button class="primary-button content-button" onclick={queue_random_card}>
                        {"Add random card"}
                    </button>
                </div>
                <div class="feature-row">
                    <p class="feature-chip feature-chip-a">{"collapse"}</p>
                    <p class="feature-chip feature-chip-b">{"add card"}</p>
                    <p class="feature-chip feature-chip-c">{"scrollbar"}</p>
                </div>
            </div>
            <div class="content-grid">
                <article class="detail-card detail-card-a">
                    <p class="detail-label">{"Queue status"}</p>
                    <p class="detail-value">
                        {state.queue_status_label()}
                    </p>
                </article>
                <article class="detail-card detail-card-b">
                    <p class="detail-label">{"Scroll interaction"}</p>
                    <p class="detail-value">
                        {state.queue_interaction_label()}
                    </p>
                </article>
                <article class="detail-card detail-card-c">
                    <p class="detail-label">{"Font note"}</p>
                    <p class="detail-value">
                        {"Uses Segoe UI when the Windows fonts are available."}
                    </p>
                </article>
            </div>
        </section>
    }
}

fn text_element(tag: &str, class_name: &str, text: impl Into<String>) -> Node {
    Node::element(tag)
        .with_class(class_name)
        .with_child(Node::text(text.into()))
        .into()
}

fn generate_card(seed: u64) -> QueueCard {
    let library_index =
        seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223) % CARD_LIBRARY.len() as u64;
    let template = CARD_LIBRARY[library_index as usize];

    QueueCard {
        title: format!("{} {}", template.title, seed + 1),
        note: template.note.to_string(),
        accent_class: template.accent_class,
    }
}

fn toggle_sidebar() {
    ACTIONS.fetch_or(ACTION_TOGGLE_SIDEBAR, Ordering::Relaxed);
}

fn queue_random_card() {
    ACTIONS.fetch_or(ACTION_ADD_CARD, Ordering::Relaxed);
}

fn register_demo_fonts() {
    #[cfg(target_os = "windows")]
    {
        let Some(fonts_dir) = windows_fonts_dir() else {
            return;
        };

        for file_name in ["segoeui.ttf", "seguisb.ttf", "segoeuib.ttf"] {
            let font_path = fonts_dir.join(file_name);
            if font_path.is_file() {
                let _ = register_font_file(&font_path);
            }
        }
    }
}

#[cfg(target_os = "windows")]
fn windows_fonts_dir() -> Option<PathBuf> {
    std::env::var_os("WINDIR")
        .map(PathBuf::from)
        .or_else(|| Some(PathBuf::from(r"C:\Windows")))
        .map(|path| path.join("Fonts"))
}

fn stylesheet() -> &'static Stylesheet {
    static STYLESHEET: OnceLock<Stylesheet> = OnceLock::new();

    STYLESHEET.get_or_init(|| {
        parse_stylesheet(include_str!("collapsible_sidebar.css"))
            .expect("collapsible sidebar stylesheet should stay valid")
    })
}
