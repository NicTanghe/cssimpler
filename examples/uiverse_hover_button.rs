use std::sync::OnceLock;
use std::time::Duration;

use anyhow::Result;
use cssimpler::app::{App, Invalidation, Refresh, RuntimeStats, latest_runtime_stats};
use cssimpler::core::Node;
use cssimpler::renderer::{
    FrameInfo, FramePaintMode, FramePaintReason, FrameTimingStats, WindowConfig,
    latest_frame_timing_stats,
};
use cssimpler::style::{Stylesheet, parse_stylesheet};
use cssimpler::ui;

const BUTTON_TEXT: &str = "uiverse";
const PERF_LOG_INTERVAL: Duration = Duration::from_secs(1);
const HUD_UPDATE_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HoverDemoState {
    pub frame_index: u64,
    pub last_frame_ms: u128,
    pub log_elapsed: Duration,
    pub last_logged_frame: u64,
    pub last_spike_frame: u64,
    pub last_animation_stall_frame: u64,
    pub last_deferred_frame: u64,
    pub hud_elapsed: Duration,
    pub hud_frame_ms: u128,
    pub hud_renderer_stats: FrameTimingStats,
    pub hud_app_stats: RuntimeStats,
    pub renderer_stats: FrameTimingStats,
    pub app_stats: RuntimeStats,
}

fn main() -> Result<()> {
    let config = WindowConfig::new("cssimpler / uiverse hover button", 1440, 920);

    App::new(HoverDemoState::default(), stylesheet(), update, build_ui)
        .with_continuous_updates(true)
        .run(config)
        .map_err(Into::into)
}

fn update(state: &mut HoverDemoState, frame: FrameInfo) -> Refresh {
    state.frame_index = frame.frame_index;
    state.renderer_stats = latest_frame_timing_stats();
    state.app_stats = latest_runtime_stats();
    state.last_frame_ms = if state.renderer_stats.total_us > 0 {
        u128::from(state.renderer_stats.frame_delta_us.div_ceil(1_000))
    } else {
        frame.delta.as_millis()
    };
    state.log_elapsed += frame.delta;
    state.hud_elapsed += frame.delta;

    let refresh_hud = (state.hud_elapsed >= HUD_UPDATE_INTERVAL || state.frame_index <= 1)
        && !state.app_stats.transition_active;
    if refresh_hud {
        state.hud_elapsed = Duration::ZERO;
        state.hud_frame_ms = state.last_frame_ms;
        state.hud_renderer_stats = state.renderer_stats;
        state.hud_app_stats = state.app_stats.clone();
    }

    maybe_log_perf(state);
    if refresh_hud {
        Refresh::fragment("hud", Invalidation::Paint)
    } else {
        Refresh::clean()
    }
}

pub fn maybe_log_perf(state: &mut HoverDemoState) {
    if state.renderer_stats.transition_deferred
        && state.renderer_stats.frame_index != state.last_deferred_frame
    {
        state.last_deferred_frame = state.renderer_stats.frame_index;
        eprintln!(
            "[uiverse_hover][deferred] frame={} dt={} anim_delta={} elapsed={} duration={} reason=zero_progress_transition",
            state.renderer_stats.frame_index,
            format_us(state.renderer_stats.frame_delta_us),
            format_us(state.app_stats.transition_delta_us),
            format_us(state.app_stats.transition_elapsed_us),
            format_us(state.app_stats.transition_duration_us),
        );
    }

    if state.renderer_stats.paint_mode == FramePaintMode::Idle
        && state.renderer_stats.painted_pixels == 0
        && !state.app_stats.transition_active
    {
        return;
    }

    if state.frame_index == state.last_logged_frame {
        return;
    }

    // Keep rare tail-latency events even when the normal one-second sampling
    // window would hide them.
    if state.renderer_stats.total_us >= 45_000
        && state.renderer_stats.frame_index != state.last_spike_frame
    {
        state.last_spike_frame = state.renderer_stats.frame_index;
        eprintln!(
            "[uiverse_hover][spike] frame={} dt={} total={} paint={} mode={} reason={} viewport={}x{} resized={} workers={} passes={} helped={} wait={} main_task={} slowest_task={} dirty={}r/{}j damage={} shadow_cache={} shadow_raster={} shadow_draw={} text_raster={}/{} text_effect={}/{}",
            state.renderer_stats.frame_index,
            format_us(state.renderer_stats.frame_delta_us),
            format_us(state.renderer_stats.total_us),
            format_us(state.renderer_stats.paint_us),
            paint_mode_label(state.renderer_stats),
            paint_reason_label(state.renderer_stats.paint_reason),
            state.renderer_stats.viewport_width,
            state.renderer_stats.viewport_height,
            state.renderer_stats.buffer_resized,
            state.renderer_stats.render_workers,
            state.renderer_stats.scene_passes,
            state.renderer_stats.worker_main_helped,
            format_us(state.renderer_stats.worker_wait_us as u64),
            format_us(state.renderer_stats.worker_main_task_us as u64),
            format_us(state.renderer_stats.worker_slowest_task_us as u64),
            state.renderer_stats.dirty_regions,
            state.renderer_stats.dirty_jobs,
            format_pixels(state.renderer_stats.damage_pixels),
            state.renderer_stats.shadow_cache_misses,
            format_us(state.renderer_stats.shadow_raster_us as u64),
            format_us(state.renderer_stats.shadow_draw_us as u64),
            state.renderer_stats.text_raster_cache_misses,
            format_us(state.renderer_stats.text_raster_build_us as u64),
            state.renderer_stats.text_effect_cache_misses,
            format_us(state.renderer_stats.text_effect_build_us as u64),
        );
    }

    if state.app_stats.transition_active
        && state.app_stats.transition_delta_us >= 45_000
        && state.app_stats.transition_elapsed_us >= state.app_stats.transition_delta_us
        && state.renderer_stats.frame_index != state.last_animation_stall_frame
    {
        state.last_animation_stall_frame = state.renderer_stats.frame_index;
        eprintln!(
            "[uiverse_hover][animation-stall] frame={} anim_delta={} elapsed={} duration={} previous_paint={} previous_total={}",
            state.renderer_stats.frame_index,
            format_us(state.app_stats.transition_delta_us),
            format_us(state.app_stats.transition_elapsed_us),
            format_us(state.app_stats.transition_duration_us),
            format_us(state.renderer_stats.paint_us),
            format_us(state.renderer_stats.total_us),
        );
    }

    if state.log_elapsed < PERF_LOG_INTERVAL {
        return;
    }

    state.last_logged_frame = state.frame_index;
    while state.log_elapsed >= PERF_LOG_INTERVAL {
        state.log_elapsed = state.log_elapsed.saturating_sub(PERF_LOG_INTERVAL);
    }

    let fps = if state.renderer_stats.frame_delta_us > 0 {
        (1_000_000 / state.renderer_stats.frame_delta_us).min(999)
    } else {
        60
    };

    eprintln!(
        "[uiverse_hover] frame={} fps={:<2} dt={:>3}ms update={:>7} tree={:>7} prep={:>7} paint={:>7} present={:>7} total={:>7} anim={} mode={} reason={} dirty={}r/{}j damage={} painted={} passes={} workers={} main_task={:>7} slowest_task={:>7} wait={:>7} shadow_cache={} shadow_raster={:>7} shadow_draw={:>7}",
        state.renderer_stats.frame_index,
        fps,
        state.last_frame_ms,
        format_us(state.renderer_stats.update_us),
        format_us(state.app_stats.render_tree_us),
        format_us(state.renderer_stats.scene_prep_us),
        format_us(state.renderer_stats.paint_us),
        format_us(state.renderer_stats.present_us),
        format_us(state.renderer_stats.total_us),
        format_animation_clock(&state.app_stats),
        paint_mode_label(state.renderer_stats),
        paint_reason_label(state.renderer_stats.paint_reason),
        state.renderer_stats.dirty_regions,
        state.renderer_stats.dirty_jobs,
        format_pixels(state.renderer_stats.damage_pixels),
        format_pixels(state.renderer_stats.painted_pixels),
        state.renderer_stats.scene_passes,
        state.renderer_stats.render_workers,
        format_us(state.renderer_stats.worker_main_task_us as u64),
        format_us(state.renderer_stats.worker_slowest_task_us as u64),
        format_us(state.renderer_stats.worker_wait_us as u64),
        state.renderer_stats.shadow_cache_misses,
        format_us(state.renderer_stats.shadow_raster_us as u64),
        format_us(state.renderer_stats.shadow_draw_us as u64),
    );
}

fn build_ui(state: &HoverDemoState) -> Node {
    ui! {
        <div id="app">
            {build_hud(state)}
            <div class="demo-stage">
                <section class="spotlight">
                    <p class="kicker">
                        Uiverse-inspired hover reveal
                    </p>
                    {build_button()}
                    <p class="note">
                        The outlined label stays centered while the neon fill sweeps across on hover.
                    </p>
                    <p class="kicker">
                        Uiverse-inspired glass card
                    </p>
                    {build_card()}
                    <p class="note">
                        A frosted panel, floating badge stack, and compact social actions sit on top of a mint neon base.
                    </p>
                </section>
            </div>
        </div>
    }
}

fn build_hud(state: &HoverDemoState) -> Node {
    ui! {
        <section id="hud" class="hud">
            <div class="hud-header">
                <p class="hud-title">Performance Metrics</p>
            </div>
            {build_metric_row(state)}
        </section>
    }
}

fn build_metric_row(state: &HoverDemoState) -> Node {
    let (paint_mode_main, paint_mode_sub) = paint_mode_lines(state.hud_renderer_stats);
    ui! {
        <div class="metric-row">
            {stat_chip("dt", format!("{} ms", state.hud_frame_ms))}
            {stat_chip("app view", format_us(state.hud_app_stats.view_us))}
            {stat_chip("tree build", format_us(state.hud_app_stats.render_tree_us))}
            {stat_chip("scene swap", format_us(state.hud_app_stats.scene_swap_us))}
            {stat_chip("transition", format_us(state.hud_app_stats.transition_us))}
            {stat_chip("animation clock", format_animation_clock(&state.hud_app_stats))}
            {stat_chip("scene prep", format_us(state.hud_renderer_stats.scene_prep_us))}
            {stat_chip("paint", format_us(state.hud_renderer_stats.paint_us))}
            {stat_chip("present", format_us(state.hud_renderer_stats.present_us))}
            {stat_chip("frame total", format_us(state.hud_renderer_stats.total_us))}
            {two_line_stat_chip("paint mode", paint_mode_main, paint_mode_sub)}
            {stat_chip("paint reason", paint_reason_label(state.hud_renderer_stats.paint_reason).to_string())}
            {stat_chip("dirty regions", state.hud_renderer_stats.dirty_regions.to_string())}
            {stat_chip("dirty jobs", state.hud_renderer_stats.dirty_jobs.to_string())}
            {stat_chip("damage", format_pixels(state.hud_renderer_stats.damage_pixels))}
            {stat_chip("painted", format_pixels(state.hud_renderer_stats.painted_pixels))}
            {stat_chip("scene passes", state.hud_renderer_stats.scene_passes.to_string())}
            {stat_chip("workers", state.hud_renderer_stats.render_workers.to_string())}
            {two_line_stat_chip(
                "worker timing",
                format!("main {}", format_us(state.hud_renderer_stats.worker_main_task_us as u64)),
                format!("slow {}", format_us(state.hud_renderer_stats.worker_slowest_task_us as u64)),
            )}
            {two_line_stat_chip(
                "text cache",
                format!("raster {}", state.hud_renderer_stats.text_raster_cache_misses),
                format!("effect {}", state.hud_renderer_stats.text_effect_cache_misses),
            )}
        </div>
    }
}

fn stat_chip(label: impl Into<String>, value: impl Into<String>) -> Node {
    let label = label.into();
    let value = value.into();

    ui! {
        <div class="stat-chip">
            <p class="stat-label">
                {label}
            </p>
            <p class="stat-value">
                {value}
            </p>
        </div>
    }
}

fn two_line_stat_chip(
    label: impl Into<String>,
    line1: impl Into<String>,
    line2: impl Into<String>,
) -> Node {
    let label = label.into();
    let line1 = line1.into();
    let line2 = line2.into();

    ui! {
        <div class="stat-chip">
            <p class="stat-label">
                {label}
            </p>
            <p class="stat-value">
                {line1}
            </p>
            <p class="stat-subvalue">
                {line2}
            </p>
        </div>
    }
}

fn format_us(duration_us: u64) -> String {
    format!("{:.2} ms", duration_us as f64 / 1000.0)
}

fn format_animation_clock(stats: &RuntimeStats) -> String {
    if stats.transition_duration_us == 0 {
        return "idle".to_string();
    }

    let progress = stats.transition_elapsed_us as f64 / stats.transition_duration_us as f64;
    format!(
        "{:.0}% {}/{}ms (+{}ms)",
        progress.clamp(0.0, 1.0) * 100.0,
        stats.transition_elapsed_us.div_ceil(1_000),
        stats.transition_duration_us.div_ceil(1_000),
        stats.transition_delta_us.div_ceil(1_000),
    )
}

fn paint_mode_lines(stats: FrameTimingStats) -> (String, String) {
    match stats.paint_mode {
        FramePaintMode::Idle => ("idle".to_string(), "-".to_string()),
        FramePaintMode::Full => {
            let sub = if stats.render_workers > 1 {
                format!("x{} workers", stats.render_workers)
            } else {
                "1 worker".to_string()
            };
            ("full".to_string(), sub)
        }
        FramePaintMode::Incremental => (
            "incremental".to_string(),
            format!("{}r / {}j", stats.dirty_regions, stats.dirty_jobs),
        ),
    }
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

fn build_button() -> Node {
    ui! {
        <button id="reveal-button" class="button" type="button">
            <span class="actual-text">
                <span class="actual-label">
                    <span class="actual-label-text">
                        {BUTTON_TEXT}
                    </span>
                </span>
            </span>
            <span class="hover-text">
                <span class="hover-fill" aria-hidden="true">
                    <span class="hover-label">
                        <span class="hover-label-text">
                            {BUTTON_TEXT}
                        </span>
                    </span>
                </span>
            </span>
        </button>
    }
}

fn build_card() -> Node {
    ui! {
        <div class="uiverse-card-demo">
            <div id="card-demo" class="parent">
                <div class="card">
                    <div class="logo">
                        <span class="circle circle1"></span>
                        <span class="circle circle2"></span>
                        <span class="circle circle3"></span>
                        <span class="circle circle4"></span>
                        <span class="circle circle5">
                            <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 29.667 31.69" class="svg">
                                <path d="M12.827,1.628A1.561,1.561,0,0,1,14.31,0h2.964a1.561,1.561,0,0,1,1.483,1.628v11.9a9.252,9.252,0,0,1-2.432,6.852q-2.432,2.409-6.963,2.409T2.4,20.452Q0,18.094,0,13.669V1.628A1.561,1.561,0,0,1,1.483,0h2.98A1.561,1.561,0,0,1,5.947,1.628V13.191a5.635,5.635,0,0,0,.85,3.451,3.153,3.153,0,0,0,2.632,1.094,3.032,3.032,0,0,0,2.582-1.076,5.836,5.836,0,0,0,.816-3.486Z" transform="translate(0 0)"></path>
                                <path d="M75.207,20.857a1.561,1.561,0,0,1-1.483,1.628h-2.98a1.561,1.561,0,0,1-1.483-1.628V1.628A1.561,1.561,0,0,1,70.743,0h2.98a1.561,1.561,0,0,1,1.483,1.628Z" transform="translate(-45.91 0)"></path>
                                <path d="M0,80.018A1.561,1.561,0,0,1,1.483,78.39h26.7a1.561,1.561,0,0,1,1.483,1.628v2.006a1.561,1.561,0,0,1-1.483,1.628H1.483A1.561,1.561,0,0,1,0,82.025Z" transform="translate(0 -51.963)"></path>
                            </svg>
                        </span>
                    </div>
                    <div class="glass"></div>
                    <div class="content">
                        <span class="title">UIVERSE (3D UI)</span>
                        <span class="text">
                            Create, share, and use beautiful custom elements made with CSS
                        </span>
                    </div>
                    <div class="bottom">
                        <div class="social-buttons-container">
                            <button class="social-button social-button1" type="button">
                                <svg viewBox="0 0 30 30" xmlns="http://www.w3.org/2000/svg" class="svg">
                                    <path d="M 9.9980469 3 C 6.1390469 3 3 6.1419531 3 10.001953 L 3 20.001953 C 3 23.860953 6.1419531 27 10.001953 27 L 20.001953 27 C 23.860953 27 27 23.858047 27 19.998047 L 27 9.9980469 C 27 6.1390469 23.858047 3 19.998047 3 L 9.9980469 3 z M 22 7 C 22.552 7 23 7.448 23 8 C 23 8.552 22.552 9 22 9 C 21.448 9 21 8.552 21 8 C 21 7.448 21.448 7 22 7 z M 15 9 C 18.309 9 21 11.691 21 15 C 21 18.309 18.309 21 15 21 C 11.691 21 9 18.309 9 15 C 9 11.691 11.691 9 15 9 z M 15 11 A 4 4 0 0 0 11 15 A 4 4 0 0 0 15 19 A 4 4 0 0 0 19 15 A 4 4 0 0 0 15 11 z"></path>
                                </svg>
                            </button>
                            <button class="social-button social-button2" type="button">
                                <svg viewBox="0 0 512 512" xmlns="http://www.w3.org/2000/svg" class="svg">
                                    <path d="M459.37 151.716c.325 4.548.325 9.097.325 13.645 0 138.72-105.583 298.558-298.558 298.558-59.452 0-114.68-17.219-161.137-47.106 8.447.974 16.568 1.299 25.34 1.299 49.055 0 94.213-16.568 130.274-44.832-46.132-.975-84.792-31.188-98.112-72.772 6.498.974 12.995 1.624 19.818 1.624 9.421 0 18.843-1.3 27.614-3.573-48.081-9.747-84.143-51.98-84.143-102.985v-1.299c13.969 7.797 30.214 12.67 47.431 13.319-28.264-18.843-46.781-51.005-46.781-87.391 0-19.492 5.197-37.36 14.294-52.954 51.655 63.675 129.3 105.258 216.365 109.807-1.624-7.797-2.599-15.918-2.599-24.04 0-57.828 46.782-104.934 104.934-104.934 30.213 0 57.502 12.67 76.67 33.137 23.715-4.548 46.456-13.32 66.599-25.34-7.798 24.366-24.366 44.833-46.132 57.827 21.117-2.273 41.584-8.122 60.426-16.243-14.292 20.791-32.161 39.308-52.628 54.253z"></path>
                                </svg>
                            </button>
                            <button class="social-button social-button3" type="button">
                                <svg viewBox="0 0 640 512" xmlns="http://www.w3.org/2000/svg" class="svg">
                                    <path d="M524.531,69.836a1.5,1.5,0,0,0-.764-.7A485.065,485.065,0,0,0,404.081,32.03a1.816,1.816,0,0,0-1.923.91,337.461,337.461,0,0,0-14.9,30.6,447.848,447.848,0,0,0-134.426,0,309.541,309.541,0,0,0-15.135-30.6,1.89,1.89,0,0,0-1.924-.91A483.689,483.689,0,0,0,116.085,69.137a1.712,1.712,0,0,0-.788.676C39.068,183.651,18.186,294.69,28.43,404.354a2.016,2.016,0,0,0,.765,1.375A487.666,487.666,0,0,0,176.02,479.918a1.9,1.9,0,0,0,2.063-.676A348.2,348.2,0,0,0,208.12,430.4a1.86,1.86,0,0,0-1.019-2.588,321.173,321.173,0,0,1-45.868-21.853,1.885,1.885,0,0,1-.185-3.126c3.082-2.309,6.166-4.711,9.109-7.137a1.819,1.819,0,0,1,1.9-.256c96.229,43.917,200.41,43.917,295.5,0a1.812,1.812,0,0,1,1.924.233c2.944,2.426,6.027,4.851,9.132,7.16a1.884,1.884,0,0,1-.162,3.126,301.407,301.407,0,0,1-45.89,21.83,1.875,1.875,0,0,0-1,2.611,391.055,391.055,0,0,0,30.014,48.815,1.864,1.864,0,0,0,2.063.7A486.048,486.048,0,0,0,610.7,405.729a1.882,1.882,0,0,0,.765-1.352C623.729,277.594,590.933,167.465,524.531,69.836ZM222.491,337.58c-28.972,0-52.844-26.587-52.844-59.239S193.056,219.1,222.491,219.1c29.665,0,53.306,26.82,52.843,59.239C275.334,310.993,251.924,337.58,222.491,337.58Zm195.38,0c-28.971,0-52.843-26.587-52.843-59.239S388.437,219.1,417.871,219.1c29.667,0,53.307,26.82,52.844,59.239C470.715,310.993,447.538,337.58,417.871,337.58Z"></path>
                                </svg>
                            </button>
                        </div>
                        <div class="view-more">
                            <button class="view-more-button" type="button">View more</button>
                            <svg class="svg" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" stroke-linecap="round" stroke-linejoin="round">
                                <path d="m6 9 6 6 6-6"></path>
                            </svg>
                        </div>
                    </div>
                </div>
            </div>
        </div>
    }
}

fn stylesheet() -> &'static Stylesheet {
    static STYLESHEET: OnceLock<Stylesheet> = OnceLock::new();

    STYLESHEET.get_or_init(|| {
        let source = format!(
            "{}\n{}",
            include_str!("uiverse_hover_button.css"),
            include_str!("uiverse_card.css")
        );
        parse_stylesheet(&source).expect("uiverse hover button stylesheet should stay valid")
    })
}

#[cfg(test)]
mod tests {
    use super::{BUTTON_TEXT, HoverDemoState, build_card, build_ui, stylesheet};
    use cssimpler::app::{App, Invalidation, latest_runtime_stats};
    use cssimpler::core::fonts::layout_text_block;
    use cssimpler::core::{ElementInteractionState, ElementPath, Node, RenderKind, RenderNode};
    use cssimpler::renderer::{
        FrameInfo, SceneProvider, ViewportSize, render_scene_update, render_to_buffer,
    };
    use cssimpler::style::{build_render_tree_in_viewport_with_interaction, parse_stylesheet};
    use cssimpler::ui;
    use std::time::Duration;

    #[test]
    fn hover_mask_expands_to_cover_the_button() {
        let tree = build_ui(&HoverDemoState::default());
        let idle = build_render_tree_in_viewport_with_interaction(
            &tree,
            stylesheet(),
            1280,
            720,
            &ElementInteractionState::default(),
        );
        let hovered = build_render_tree_in_viewport_with_interaction(
            &tree,
            stylesheet(),
            1280,
            720,
            &ElementInteractionState {
                hovered: Some(
                    ElementPath::root(0)
                        .with_child(1)
                        .with_child(0)
                        .with_child(1),
                ),
                active: None,
            },
        );

        let idle_mask = hover_mask(&idle);
        let hovered_mask = hover_mask(&hovered);
        let hovered_button = button(&hovered);

        assert_eq!(idle_mask.layout.width, idle_mask.style.border.widths.right);
        assert!((hovered_mask.layout.width - hovered_button.layout.width).abs() < 0.01);
        assert_eq!(hovered_mask.style.border.widths.right, 4.0);
        assert_eq!(hovered_mask.children.len(), 1);
        assert!((hovered_mask.children[0].layout.width - hovered_button.layout.width).abs() < 0.01);
        assert!(matches!(
            &hovered_mask.children[0].children[0].children[0].kind,
            RenderKind::Text(content) if content == BUTTON_TEXT
        ));
    }

    fn button(root: &RenderNode) -> &RenderNode {
        &root.children[1].children[0].children[1]
    }

    fn hover_mask(root: &RenderNode) -> &RenderNode {
        &button(root).children[1]
    }

    fn actual_text_node(root: &RenderNode) -> &RenderNode {
        &button(root).children[0].children[0].children[0]
    }

    fn hover_fill_text_node(root: &RenderNode) -> &RenderNode {
        &hover_mask(root).children[0].children[0].children[0]
    }

    #[test]
    fn hover_reveal_keeps_the_text_node_layouts_stable() {
        let tree = build_ui(&HoverDemoState::default());
        let idle = build_render_tree_in_viewport_with_interaction(
            &tree,
            stylesheet(),
            1280,
            720,
            &ElementInteractionState::default(),
        );
        let hovered = build_render_tree_in_viewport_with_interaction(
            &tree,
            stylesheet(),
            1280,
            720,
            &ElementInteractionState {
                hovered: Some(
                    ElementPath::root(0)
                        .with_child(1)
                        .with_child(0)
                        .with_child(1),
                ),
                active: None,
            },
        );

        assert_eq!(
            actual_text_node(&idle).layout,
            actual_text_node(&hovered).layout
        );
        assert_eq!(
            hover_fill_text_node(&idle).layout,
            hover_fill_text_node(&hovered).layout
        );
    }

    #[test]
    fn reveal_transition_keeps_the_hover_label_on_one_line() {
        let stylesheet = parse_stylesheet(
            r#"
            .button {
              width: 320px;
              height: 88px;
              display: flex;
              justify-content: center;
              align-items: center;
              position: relative;
              font-size: 44px;
              font-weight: 700;
              line-height: 1;
              letter-spacing: 2px;
              text-transform: uppercase;
            }

            .actual-text {
              display: flex;
              width: 320px;
              height: 88px;
              justify-content: center;
              align-items: center;
            }

            .actual-label {
              display: flex;
              width: 320px;
              height: 88px;
              justify-content: center;
              align-items: center;
              flex-shrink: 0;
            }

            .actual-label-text {
              display: block;
              width: 252px;
              flex-shrink: 0;
            }

            .hover-text {
              width: 0px;
              height: 88px;
              position: absolute;
              inset: 0;
              overflow: hidden;
              border-right: 4px solid #37ff8b;
              transition: width 32ms linear;
            }

            .button.hot .hover-text {
              width: 100%;
            }

            .hover-fill {
              display: flex;
              width: 320px;
              height: 88px;
              justify-content: center;
              align-items: center;
            }

            .hover-label {
              display: flex;
              width: 320px;
              height: 88px;
              justify-content: center;
              align-items: center;
              flex-shrink: 0;
            }

            .hover-label-text {
              display: block;
              width: 252px;
              flex-shrink: 0;
            }
            "#,
        )
        .expect("stylesheet should parse");

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
                if *state {
                    ui! {
                        <div>
                            {build_test_button(true)}
                        </div>
                    }
                } else {
                    ui! {
                        <div>
                            {build_test_button(false)}
                        </div>
                    }
                }
            },
        );

        let _first = app.frame(frame(0));
        let second = app.frame(frame(1));
        let _third = app.frame(frame(2));

        let mid_button = &second[0].children[0];
        let mid_mask = &mid_button.children[1];
        let hover_fill = &mid_mask.children[0];
        let hover_label = &hover_fill.children[0];
        let hover_label_text = &hover_label.children[0];

        assert!(mid_mask.layout.width > 4.0);
        assert!(mid_mask.layout.width < mid_button.layout.width);
        assert!((hover_fill.layout.width - mid_button.layout.width).abs() < 0.01);
        assert!((hover_label.layout.width - mid_button.layout.width).abs() < 0.01);
        assert!(matches!(
            &hover_label_text.kind,
            RenderKind::Text(content) if content == BUTTON_TEXT
        ));

        let text_layout = layout_text_block(
            BUTTON_TEXT,
            &hover_label_text.style.text,
            Some(hover_label_text.layout.width.max(1.0)),
        );
        assert_eq!(text_layout.lines.len(), 1);
    }

    #[test]
    fn hovered_card_full_render_spans_a_large_visible_area() {
        let tree = ui! {
            <div>
                {build_card()}
            </div>
        };
        let hovered = build_render_tree_in_viewport_with_interaction(
            &tree,
            stylesheet(),
            480,
            480,
            &ElementInteractionState {
                hovered: Some(ElementPath::root(0).with_child(0).with_child(0)),
                active: None,
            },
        );
        let clear = cssimpler::core::Color::rgb(255, 0, 255);
        let clear_packed = ((clear.r as u32) << 16) | ((clear.g as u32) << 8) | clear.b as u32;
        let mut buffer = vec![0_u32; 480 * 480];

        render_to_buffer(&[hovered], &mut buffer, 480, 480, clear);

        let mut x0 = 480_i32;
        let mut y0 = 480_i32;
        let mut x1 = 0_i32;
        let mut y1 = 0_i32;
        for y in 0..480_i32 {
            for x in 0..480_i32 {
                if buffer[y as usize * 480 + x as usize] == clear_packed {
                    continue;
                }
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }

        assert!(x1 - x0 > 180, "hovered card should cover a wide area");
        assert!(y1 - y0 > 180, "hovered card should cover a tall area");
    }

    #[test]
    fn hovered_card_incremental_render_matches_a_full_redraw() {
        let tree = ui! {
            <div>
                {build_card()}
            </div>
        };
        let idle = build_render_tree_in_viewport_with_interaction(
            &tree,
            stylesheet(),
            480,
            480,
            &ElementInteractionState::default(),
        );
        let hovered = build_render_tree_in_viewport_with_interaction(
            &tree,
            stylesheet(),
            480,
            480,
            &ElementInteractionState {
                hovered: Some(ElementPath::root(0).with_child(0).with_child(0)),
                active: None,
            },
        );
        let clear = cssimpler::core::Color::rgb(255, 0, 255);
        let mut incremental = vec![0_u32; 480 * 480];
        let mut full = vec![0_u32; 480 * 480];

        render_to_buffer(
            std::slice::from_ref(&idle),
            &mut incremental,
            480,
            480,
            clear,
        );
        render_scene_update(
            std::slice::from_ref(&idle),
            std::slice::from_ref(&hovered),
            &mut incremental,
            480,
            480,
            clear,
        );
        render_to_buffer(std::slice::from_ref(&hovered), &mut full, 480, 480, clear);

        assert_render_buffers_match(&incremental, &full, 480);
    }

    #[test]
    fn hovered_card_stays_near_its_idle_center() {
        let tree = ui! {
            <div>
                {build_card()}
            </div>
        };
        let idle = build_render_tree_in_viewport_with_interaction(
            &tree,
            stylesheet(),
            480,
            480,
            &ElementInteractionState::default(),
        );
        let hovered = build_render_tree_in_viewport_with_interaction(
            &tree,
            stylesheet(),
            480,
            480,
            &ElementInteractionState {
                hovered: Some(ElementPath::root(0).with_child(0).with_child(0)),
                active: None,
            },
        );
        let clear = cssimpler::core::Color::rgb(255, 0, 255);
        let clear_packed = ((clear.r as u32) << 16) | ((clear.g as u32) << 8) | clear.b as u32;
        let mut idle_buffer = vec![0_u32; 480 * 480];
        let mut hovered_buffer = vec![0_u32; 480 * 480];

        render_to_buffer(
            std::slice::from_ref(&idle),
            &mut idle_buffer,
            480,
            480,
            clear,
        );
        render_to_buffer(
            std::slice::from_ref(&hovered),
            &mut hovered_buffer,
            480,
            480,
            clear,
        );

        let idle_bounds = visible_bounds(&idle_buffer, 480, 480, clear_packed)
            .expect("idle card should render visible pixels");
        let hovered_bounds = visible_bounds(&hovered_buffer, 480, 480, clear_packed)
            .expect("hovered card should render visible pixels");
        let idle_center_x = (idle_bounds.0 + idle_bounds.2) as f32 * 0.5;
        let idle_center_y = (idle_bounds.1 + idle_bounds.3) as f32 * 0.5;
        let hovered_center_x = (hovered_bounds.0 + hovered_bounds.2) as f32 * 0.5;
        let hovered_center_y = (hovered_bounds.1 + hovered_bounds.3) as f32 * 0.5;

        assert!((hovered_center_x - idle_center_x).abs() < 40.0);
        assert!((hovered_center_y - idle_center_y).abs() < 40.0);
    }

    #[test]
    fn hover_transition_midpoint_keeps_the_real_card_near_its_idle_center() {
        let mut app = App::new(
            (),
            stylesheet(),
            |_state, _frame| Invalidation::Clean,
            |_state| {
                ui! {
                    <div>
                        {build_card()}
                    </div>
                }
            },
        );
        app.set_viewport(ViewportSize {
            width: 480,
            height: 480,
        });

        let idle = app.frame(frame(0));
        assert!(SceneProvider::set_element_interaction(
            &mut app,
            ElementInteractionState {
                hovered: Some(ElementPath::root(0).with_child(0).with_child(0)),
                active: None,
            },
        ));
        let mid = app.frame(FrameInfo {
            frame_index: 1,
            delta: Duration::from_millis(250),
        });
        let final_scene = app.frame(FrameInfo {
            frame_index: 2,
            delta: Duration::from_millis(250),
        });

        let clear = cssimpler::core::Color::rgb(255, 0, 255);
        let clear_packed = ((clear.r as u32) << 16) | ((clear.g as u32) << 8) | clear.b as u32;
        let mut idle_buffer = vec![0_u32; 480 * 480];
        let mut mid_buffer = vec![0_u32; 480 * 480];
        let mut final_buffer = vec![0_u32; 480 * 480];

        render_to_buffer(&idle, &mut idle_buffer, 480, 480, clear);
        render_to_buffer(&mid, &mut mid_buffer, 480, 480, clear);
        render_to_buffer(&final_scene, &mut final_buffer, 480, 480, clear);

        let idle_bounds =
            visible_bounds(&idle_buffer, 480, 480, clear_packed).expect("idle card should render");
        let mid_bounds = visible_bounds(&mid_buffer, 480, 480, clear_packed)
            .expect("mid-transition card should render");
        let final_bounds = visible_bounds(&final_buffer, 480, 480, clear_packed)
            .expect("final card should render");

        let idle_center_x = (idle_bounds.0 + idle_bounds.2) as f32 * 0.5;
        let idle_center_y = (idle_bounds.1 + idle_bounds.3) as f32 * 0.5;
        let mid_center_x = (mid_bounds.0 + mid_bounds.2) as f32 * 0.5;
        let mid_center_y = (mid_bounds.1 + mid_bounds.3) as f32 * 0.5;
        let final_center_x = (final_bounds.0 + final_bounds.2) as f32 * 0.5;
        let final_center_y = (final_bounds.1 + final_bounds.3) as f32 * 0.5;
        assert!(
            (mid_center_x - idle_center_x).abs() < 14.0,
            "mid x drift too large: idle={idle_center_x}, mid={mid_center_x}, final={final_center_x}"
        );
        assert!(
            (mid_center_y - idle_center_y).abs() < 18.0,
            "mid y drift too large: idle={idle_center_y}, mid={mid_center_y}, final={final_center_y}"
        );
        assert!(
            (final_center_x - idle_center_x).abs() < 24.0,
            "final x drift too large: idle={idle_center_x}, mid={mid_center_x}, final={final_center_x}"
        );
        assert!(
            (final_center_y - idle_center_y).abs() < 32.0,
            "final y drift too large: idle={idle_center_y}, mid={mid_center_y}, final={final_center_y}"
        );
    }

    #[test]
    fn hover_transition_zero_elapsed_sample_matches_idle_scene() {
        let mut app = App::new(
            (),
            stylesheet(),
            |_state, _frame| Invalidation::Clean,
            |_state| {
                ui! {
                    <div>
                        {build_card()}
                    </div>
                }
            },
        );
        app.set_viewport(ViewportSize {
            width: 480,
            height: 480,
        });

        let idle = app.frame(FrameInfo {
            frame_index: 0,
            delta: Duration::ZERO,
        });
        assert!(SceneProvider::set_element_interaction(
            &mut app,
            ElementInteractionState {
                hovered: Some(ElementPath::root(0).with_child(0).with_child(0)),
                active: None,
            },
        ));
        let start = app.frame(FrameInfo {
            frame_index: 1,
            delta: Duration::ZERO,
        });

        let clear = cssimpler::core::Color::rgb(255, 0, 255);
        let mut idle_buffer = vec![0_u32; 480 * 480];
        let mut start_buffer = vec![0_u32; 480 * 480];
        render_to_buffer(&idle, &mut idle_buffer, 480, 480, clear);
        render_to_buffer(&start, &mut start_buffer, 480, 480, clear);
        assert_render_buffers_match(&start_buffer, &idle_buffer, 480);
    }

    #[test]
    fn hover_transition_midpoint_incremental_render_matches_full_redraw() {
        let mut app = App::new(
            (),
            stylesheet(),
            |_state, _frame| Invalidation::Clean,
            |_state| {
                ui! {
                    <div>
                        {build_card()}
                    </div>
                }
            },
        );
        app.set_viewport(ViewportSize {
            width: 480,
            height: 480,
        });

        let idle = app.frame(frame(0));
        assert!(SceneProvider::set_element_interaction(
            &mut app,
            ElementInteractionState {
                hovered: Some(ElementPath::root(0).with_child(0).with_child(0)),
                active: None,
            },
        ));
        let mid = app.frame(FrameInfo {
            frame_index: 1,
            delta: Duration::from_millis(250),
        });

        let clear = cssimpler::core::Color::rgb(255, 0, 255);
        let mut incremental = vec![0_u32; 480 * 480];
        let mut full = vec![0_u32; 480 * 480];

        render_to_buffer(&idle, &mut incremental, 480, 480, clear);
        render_scene_update(&idle, &mid, &mut incremental, 480, 480, clear);
        render_to_buffer(&mid, &mut full, 480, 480, clear);

        assert_render_buffers_match(&incremental, &full, 480);
    }

    #[test]
    fn hover_cursor_sweep_across_children_advances_transition_continuously() {
        let mut app = App::new(
            (),
            stylesheet(),
            |_state, _frame| Invalidation::Clean,
            |_state| {
                ui! {
                    <div>
                        {build_card()}
                    </div>
                }
            },
        );
        app.set_viewport(ViewportSize {
            width: 480,
            height: 480,
        });

        // Frame 0: idle baseline
        let _idle = app.frame(FrameInfo {
            frame_index: 0,
            delta: Duration::ZERO,
        });

        let parent_path = ElementPath::root(0).with_child(0).with_child(0);
        let card_path = parent_path.clone().with_child(0);
        let glass_path = card_path.clone().with_child(1);
        let content_path = card_path.clone().with_child(2);
        let title_path = content_path.clone().with_child(0);

        // Hover enters .parent
        assert!(SceneProvider::set_element_interaction(
            &mut app,
            ElementInteractionState {
                hovered: Some(parent_path),
                active: None,
            },
        ));
        // Settle interaction with follow_up (delta = 0)
        SceneProvider::update(
            &mut app,
            FrameInfo {
                frame_index: 0,
                delta: Duration::ZERO,
            },
        );
        // Initial transition created at t = 0 is zero-progress
        assert!(SceneProvider::transition_is_zero_progress(&app));

        // Frame 1: 16ms elapses
        let _f1 = app.frame(FrameInfo {
            frame_index: 1,
            delta: Duration::from_millis(16),
        });
        let stats1 = latest_runtime_stats();
        assert!(stats1.transition_active);
        assert!(stats1.transition_elapsed_us >= 16_000);
        assert!(!SceneProvider::transition_is_zero_progress(&app));

        // Frame 2: cursor sweeps into .card child
        SceneProvider::set_element_interaction(
            &mut app,
            ElementInteractionState {
                hovered: Some(card_path),
                active: None,
            },
        );
        SceneProvider::update(
            &mut app,
            FrameInfo {
                frame_index: 1,
                delta: Duration::ZERO,
            },
        );
        // Active transition must NOT be reset to zero progress!
        assert!(!SceneProvider::transition_is_zero_progress(&app));

        let _f2 = app.frame(FrameInfo {
            frame_index: 2,
            delta: Duration::from_millis(16),
        });
        let stats2 = latest_runtime_stats();
        assert!(stats2.transition_active);
        assert!(stats2.transition_elapsed_us > stats1.transition_elapsed_us);
        assert!(stats2.transition_elapsed_us >= 32_000);
        assert!(!SceneProvider::transition_is_zero_progress(&app));

        // Frame 3: cursor sweeps into .glass child
        SceneProvider::set_element_interaction(
            &mut app,
            ElementInteractionState {
                hovered: Some(glass_path),
                active: None,
            },
        );
        SceneProvider::update(
            &mut app,
            FrameInfo {
                frame_index: 2,
                delta: Duration::ZERO,
            },
        );
        assert!(!SceneProvider::transition_is_zero_progress(&app));

        let _f3 = app.frame(FrameInfo {
            frame_index: 3,
            delta: Duration::from_millis(16),
        });
        let stats3 = latest_runtime_stats();
        assert!(stats3.transition_active);
        assert!(stats3.transition_elapsed_us > stats2.transition_elapsed_us);
        assert!(stats3.transition_elapsed_us >= 48_000);
        assert!(!SceneProvider::transition_is_zero_progress(&app));

        // Frame 4: cursor sweeps into .content child
        SceneProvider::set_element_interaction(
            &mut app,
            ElementInteractionState {
                hovered: Some(content_path),
                active: None,
            },
        );
        SceneProvider::update(
            &mut app,
            FrameInfo {
                frame_index: 3,
                delta: Duration::ZERO,
            },
        );
        assert!(!SceneProvider::transition_is_zero_progress(&app));

        let _f4 = app.frame(FrameInfo {
            frame_index: 4,
            delta: Duration::from_millis(16),
        });
        let stats4 = latest_runtime_stats();
        assert!(stats4.transition_active);
        assert!(stats4.transition_elapsed_us > stats3.transition_elapsed_us);
        assert!(stats4.transition_elapsed_us >= 64_000);
        assert!(!SceneProvider::transition_is_zero_progress(&app));

        // Frame 5: cursor sweeps into .title child
        SceneProvider::set_element_interaction(
            &mut app,
            ElementInteractionState {
                hovered: Some(title_path),
                active: None,
            },
        );
        SceneProvider::update(
            &mut app,
            FrameInfo {
                frame_index: 4,
                delta: Duration::ZERO,
            },
        );
        assert!(!SceneProvider::transition_is_zero_progress(&app));

        let _f5 = app.frame(FrameInfo {
            frame_index: 5,
            delta: Duration::from_millis(16),
        });
        let stats5 = latest_runtime_stats();
        assert!(stats5.transition_active);
        assert!(stats5.transition_elapsed_us > stats4.transition_elapsed_us);
        assert!(stats5.transition_elapsed_us >= 80_000);
        assert!(!SceneProvider::transition_is_zero_progress(&app));
    }

    fn build_test_button(is_hot: bool) -> Node {
        if is_hot {
            ui! {
                <button class="button hot">
                    <span class="actual-text">
                        <span class="actual-label">
                            <span class="actual-label-text">
                                {BUTTON_TEXT}
                            </span>
                        </span>
                    </span>
                    <span class="hover-text">
                        <span class="hover-fill">
                            <span class="hover-label">
                                <span class="hover-label-text">
                                    {BUTTON_TEXT}
                                </span>
                            </span>
                        </span>
                    </span>
                </button>
            }
        } else {
            ui! {
                <button class="button">
                    <span class="actual-text">
                        <span class="actual-label">
                            <span class="actual-label-text">
                                {BUTTON_TEXT}
                            </span>
                        </span>
                    </span>
                    <span class="hover-text">
                        <span class="hover-fill">
                            <span class="hover-label">
                                <span class="hover-label-text">
                                    {BUTTON_TEXT}
                                </span>
                            </span>
                        </span>
                    </span>
                </button>
            }
        }
    }

    fn frame(frame_index: u64) -> FrameInfo {
        FrameInfo {
            frame_index,
            delta: Duration::from_millis(16),
        }
    }

    fn assert_render_buffers_match(actual: &[u32], expected: &[u32], width: usize) {
        assert_eq!(actual.len(), expected.len());
        let Some((index, (&actual_pixel, &expected_pixel))) = actual
            .iter()
            .zip(expected)
            .enumerate()
            .find(|(_, (actual_pixel, expected_pixel))| actual_pixel != expected_pixel)
        else {
            return;
        };
        let mismatch_count = actual
            .iter()
            .zip(expected)
            .filter(|(actual_pixel, expected_pixel)| actual_pixel != expected_pixel)
            .count();
        panic!(
            "render buffers differ at ({}, {}): actual={actual_pixel:#08x}, expected={expected_pixel:#08x}; mismatched pixels={mismatch_count}",
            index % width,
            index / width,
        );
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
}
