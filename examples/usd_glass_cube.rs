//! A USD cube over CSSimpler's native glass, with wireframe controls in a slim
//! toolbar. cargo run --features gpu-viewport --example usd_glass_cube
use cssimpler::{
    app::{App, Invalidation},
    core::Color,
    renderer::{
        self, RedrawSchedule, WindowConfig,
        gpu::{
            GpuContext, GpuRedrawHandle, GpuViewportFrame, GpuViewportInfo, GpuViewportOptions,
            GpuViewportRenderer,
            usd::{UsdCameraMode, UsdRenderer, UsdScene, Wireframe, WireframeStyle},
        },
    },
    style::parse_stylesheet,
    ui,
};
use std::{
    cell::Cell,
    rc::Rc,
    sync::{Arc, Mutex},
};

const WIDTHS: [f32; 4] = [1.0, 2.0, 4.0, 6.0];
const COLOURS: [(&str, [f32; 3], [f32; 3]); 3] = [
    ("black", [0.0, 0.0, 0.0], [0.0, 0.0, 0.0]),
    ("outlined", [0.82, 0.88, 0.95], [0.0, 0.0, 0.0]),
    ("cyan", [0.10, 0.85, 1.00], [0.0, 0.0, 0.0]),
];

fn cycle_width(wireframe: Wireframe) -> Wireframe {
    let index = WIDTHS
        .iter()
        .position(|width| *width >= wireframe.style.width_px)
        .unwrap_or(0);
    Wireframe {
        style: WireframeStyle {
            width_px: WIDTHS[(index + 1) % WIDTHS.len()],
            ..wireframe.style
        },
        ..wireframe
    }
}

fn colour_index(wireframe: Wireframe) -> Option<usize> {
    COLOURS.iter().position(|(_, core, outline)| {
        *core == wireframe.style.core && *outline == wireframe.style.outline
    })
}

fn cycle_colour(wireframe: Wireframe) -> Wireframe {
    let index = colour_index(wireframe).map_or(0, |index| (index + 1) % COLOURS.len());
    let (_, core, outline) = COLOURS[index];
    Wireframe {
        style: WireframeStyle {
            core,
            outline,
            ..wireframe.style
        },
        ..wireframe
    }
}

struct Cube {
    usd: UsdRenderer,
    wireframe: Arc<Mutex<Wireframe>>,
    frames: u64,
    smoke: bool,
    on_demand_smoke: bool,
    input_smoke: bool,
    redraw: GpuRedrawHandle,
    started: Option<std::time::Instant>,
    done: Rc<Cell<bool>>,
}
impl GpuViewportRenderer for Cube {
    fn resize(&mut self, gpu: &GpuContext, info: GpuViewportInfo) -> renderer::Result<()> {
        self.usd.resize(gpu, info)
    }
    fn event(&mut self, event: &renderer::EngineEvent) -> bool {
        // The stress test owns its synthetic press/release sequence. Real
        // mouse-up/focus events must not interrupt that sequence and turn a
        // manual interaction with the test window into a failed assertion.
        if self.input_smoke {
            return false;
        }
        self.usd.event(event)
    }
    fn render(&mut self, gpu: &GpuContext, frame: GpuViewportFrame<'_>) -> renderer::Result<()> {
        let started = *self.started.get_or_insert_with(std::time::Instant::now);
        if self.input_smoke {
            self.exercise_input();
        }
        let wireframe = *self.wireframe.lock().unwrap_or_else(|e| e.into_inner());
        self.usd.set_wireframe_default(wireframe);
        self.usd.render(gpu, frame)?;
        self.frames += 1;
        if self.smoke && self.frames == 120 {
            eprintln!(
                "Glass cube smoke: gpu_frames={} last_cpu_frame={} elapsed_ms={}",
                self.frames,
                renderer::latest_frame_timing_stats().frame_index,
                started.elapsed().as_millis(),
            );
            self.done.set(true);
        } else if self.on_demand_smoke {
            // Exercise the same invalidation path as orbit input, including
            // waking the event loop when the next GPU frame is cadence-limited.
            self.redraw.request_redraw();
        }
        Ok(())
    }
}

impl Cube {
    fn exercise_input(&mut self) {
        use renderer::{ButtonState, EngineEvent, PointerButton, PointerPosition, ScrollDelta};
        let moved = |x| EngineEvent::PointerMoved {
            position: PointerPosition { x, y: 40.0 },
            modifiers: Default::default(),
        };
        let held = self.frames < 90;
        if self.frames == 0 {
            self.usd.event(&moved(0.0));
        }
        if self.frames == 0 || self.frames == 90 {
            self.usd.event(&EngineEvent::PointerButton {
                button: PointerButton::Primary,
                state: if held {
                    ButtonState::Pressed
                } else {
                    ButtonState::Released
                },
                modifiers: Default::default(),
            });
        }
        // Stress input accumulation and redraw coalescing while native glass
        // presents, then feed hover/scroll samples after releasing the button.
        for sample in 1..=1000 {
            let consumed = self
                .usd
                .event(&moved(self.frames as f32 * 2.0 + sample as f32 * 0.002));
            assert_eq!(
                consumed, held,
                "camera must accept movement only while held"
            );
            if consumed {
                self.redraw.request_redraw();
            }
        }
        assert_eq!(
            self.usd.event(&EngineEvent::Scroll {
                delta: ScrollDelta::Pixels { x: 0.0, y: 0.4 },
                modifiers: Default::default(),
            }),
            held,
            "camera must accept scrolling only while held"
        );
    }
}

fn edit(wireframe: &Mutex<Wireframe>, apply: impl FnOnce(Wireframe) -> Wireframe) {
    let mut state = wireframe.lock().unwrap_or_else(|e| e.into_inner());
    *state = apply(*state);
}

fn poke(redraws: &Mutex<Vec<GpuRedrawHandle>>) {
    let handles: Vec<_> = redraws.lock().unwrap_or_else(|e| e.into_inner()).clone();
    for handle in handles {
        handle.request_redraw();
    }
}

fn main() -> anyhow::Result<()> {
    let input_smoke = std::env::args().any(|arg| arg == "--smoke-input");
    let trace_input = std::env::args().any(|arg| arg == "--trace-input");
    let on_demand_smoke = input_smoke || std::env::args().any(|arg| arg == "--smoke-on-demand");
    let smoke = on_demand_smoke || std::env::args().any(|arg| arg == "--smoke");
    let scene = UsdScene::open(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/examples/assets/usd-preview.usda"
    ))?;
    let done = Rc::new(Cell::new(false));
    let completed = done.clone();
    let exit = done.clone();
    let wireframe = Arc::new(Mutex::new(Wireframe::default()));
    // The renderer is created lazily, so the factory registers its wake handle
    // and the toolbar starts driving the viewport once it exists.
    let redraws: Arc<Mutex<Vec<GpuRedrawHandle>>> = Arc::new(Mutex::new(Vec::new()));
    let cube_wireframe = wireframe.clone();
    let cube_redraws = redraws.clone();
    let ui_wireframe = wireframe.clone();
    let ui_redraws = redraws.clone();
    let stylesheet = parse_stylesheet(
        r#"
        .workspace { width:100vw; height:100vh; display:flex; flex-direction:column; }
        #scene {
            flex:1;
            native-material:glass;
            glass-tint:rgba(18,25,36,0.35);
        }
        .toolbar {
            height:44px; display:flex; align-items:center; gap:8px; padding:0 16px;
            background:#131b26; color:#dfe9f5;
        }
        .wire-button { padding:6px 12px; font-size:13px; }
    "#,
    )?;
    App::new(
        done,
        &stylesheet,
        |_, _| Invalidation::Clean,
        move |_| {
            let wireframe = *ui_wireframe.lock().unwrap_or_else(|e| e.into_inner());
            let handler = |apply: fn(Wireframe) -> Wireframe| {
                let wireframe = ui_wireframe.clone();
                let redraws = ui_redraws.clone();
                move || {
                    edit(&wireframe, apply);
                    poke(&redraws);
                }
            };
            let mode = handler(|wireframe| Wireframe {
                mode: wireframe.mode.next(),
                ..wireframe
            });
            let width = handler(cycle_width);
            let colour = handler(cycle_colour);
            let mode_label = format!("wire: {}", wireframe.mode.label());
            let width_label = format!("{}px", wireframe.style.width_px);
            let colour_label = colour_index(wireframe).map_or("custom", |index| COLOURS[index].0);
            ui! {
                <div class="workspace">
                    <div id="scene"></div>
                    <div class="toolbar">
                        <button class="wire-button" onclick_shared={mode}>{mode_label}</button>
                        <button class="wire-button" onclick_shared={width}>{width_label}</button>
                        <button class="wire-button" onclick_shared={colour}>{colour_label}</button>
                    </div>
                </div>
            }
        },
    )
    .with_gpu_viewport(
        "scene",
        GpuViewportOptions {
            transparent: true,
            redraw: if smoke && !on_demand_smoke {
                RedrawSchedule::EveryFrame
            } else {
                RedrawSchedule::OnInvalidation
            },
        },
        move |init| {
            let redraw = init.redraw.clone();
            cube_redraws
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(redraw.clone());
            Ok(Box::new(Cube {
                usd: UsdRenderer::new(init, scene, UsdCameraMode::Orbit)?,
                wireframe: cube_wireframe,
                frames: 0,
                smoke,
                on_demand_smoke,
                input_smoke,
                redraw,
                started: None,
                done: exit,
            }))
        },
    )
    .with_exit_when(|done| done.get())
    .run(WindowConfig {
        clear_color: Color::rgba(0, 0, 0, 0),
        ..WindowConfig::new(
            if trace_input {
                "CSSimpler glass cube — input trace"
            } else {
                "CSSimpler glass cube"
            },
            800,
            600,
        )
        .with_glass_capable(true)
        .with_decorations(false)
        .with_drag_region_height(28.0)
        .with_resize_border_width(6.0)
    })?;
    if smoke {
        anyhow::ensure!(
            completed.get(),
            "glass viewport did not complete the smoke test"
        );
    }
    Ok(())
}
