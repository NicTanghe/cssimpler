#[allow(dead_code)]
#[path = "gui_effect_pressure.rs"]
mod shared;

use std::sync::OnceLock;

use anyhow::Result;
use cssimpler::app::{Fragment, FragmentApp, Refresh};
use cssimpler::renderer::{FrameInfo, WindowConfig};
use cssimpler::style::{Stylesheet, parse_stylesheet};
use cssimpler::ui_prefab;

fn main() -> Result<()> {
    let config = WindowConfig::new("cssimpler / gui effect pressure / prefab", 1440, 960);

    cssimpler::renderer::run_with_scene_provider(
        config,
        shared::PressureProvider::new(FragmentApp::new(
            shared::EffectStressState::default(),
            stylesheet(),
            update,
            fragments(),
        )),
    )
    .map_err(Into::into)
}

fn update(state: &mut shared::EffectStressState, frame: FrameInfo) -> Refresh {
    let actions = shared::take_actions();
    let previous = state.clone();
    let invalidation = shared::apply_frame(state, frame, actions);
    shared::maybe_log_perf(state, actions);
    shared::fragment_refresh(&previous, state, invalidation)
}

fn fragments() -> Vec<Fragment<'static, shared::EffectStressState>> {
    let mut fragments = vec![
        Fragment::new("backdrop", |_state: &shared::EffectStressState| {
            ui_prefab! {
                <div id="backdrop" class="fragment-backdrop"></div>
            }
        }),
        Fragment::new("wall-shell", |_state: &shared::EffectStressState| {
            ui_prefab! {
                <section id="wall-shell" class="wall-shell fragment-wall-shell"></section>
            }
        }),
        Fragment::new("hero", |state: &shared::EffectStressState| {
            shared::build_fragment_hero(state)
        }),
    ];
    fragments.extend((0..shared::MAX_TILE_COUNT).map(|tile_index| {
        let id = shared::tile_fragment_id(tile_index);
        Fragment::new(id, move |state: &shared::EffectStressState| {
            shared::build_fragment_tile(tile_index, state)
        })
    }));
    fragments
}

fn stylesheet() -> &'static Stylesheet {
    static STYLESHEET: OnceLock<Stylesheet> = OnceLock::new();

    STYLESHEET.get_or_init(|| {
        let css = format!(
            "{}\n{}",
            include_str!("gui_effect_pressure.css"),
            fragment_overlay_css()
        );
        parse_stylesheet(&css).expect("prefab pressure stylesheet should stay valid")
    })
}

fn fragment_overlay_css() -> String {
    let mut css = String::from(
        ".fragment-backdrop {
            position: absolute;
            left: 0px;
            top: 0px;
            width: 1440px;
            height: 960px;
            background:
              radial-gradient(circle at 12% 8%, rgba(56, 189, 248, 0.2) 0px, rgba(56, 189, 248, 0.0) 220px),
              radial-gradient(circle at 88% 0%, rgba(244, 63, 94, 0.12) 0px, rgba(244, 63, 94, 0.0) 240px),
              linear-gradient(180deg, #050914 0%, #08111d 48%, #030711 100%);
          }
          .fragment-hero {
            position: absolute;
            left: 24px;
            top: 24px;
            width: 1392px;
          }
          .fragment-wall-shell {
            position: absolute;
            left: 24px;
            top: 506px;
            width: 1392px;
            height: 430px;
          }
          .fragment-tile {
            position: absolute;
          }
          .fragment-hidden {
            display: none;
          }\n",
    );

    for tile_index in 0..shared::MAX_TILE_COUNT {
        let column = tile_index % 4;
        let row = tile_index / 4;
        let left = 42 + column * 324;
        let top = 524 + row * 288;
        css.push_str(&format!(
            ".fragment-tile-pos-{tile_index} {{ left: {left}px; top: {top}px; }}\n"
        ));
    }

    css
}
