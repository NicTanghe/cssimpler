//! A USD cube over CSSimpler's native glass, with no GPU background or UI panels.
//! cargo run --features gpu-viewport --example usd_glass_cube
use cssimpler::{
    app::{App, Invalidation},
    core::Color,
    renderer::{
        self, RedrawSchedule, WindowConfig,
        gpu::{
            GpuContext, GpuRedrawHandle, GpuViewportFrame, GpuViewportInfo, GpuViewportOptions,
            GpuViewportRenderer,
            usd::{UsdCameraMode, UsdRenderer, UsdScene},
        },
    },
    style::parse_stylesheet,
    ui,
};
use std::{cell::Cell, rc::Rc};

struct Cube {
    usd: UsdRenderer,
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
        self.usd.event(event)
    }
    fn render(&mut self, gpu: &GpuContext, frame: GpuViewportFrame<'_>) -> renderer::Result<()> {
        let started = *self.started.get_or_insert_with(std::time::Instant::now);
        if self.input_smoke {
            self.exercise_input();
        }
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
    let stylesheet = parse_stylesheet(
        r#"
        #scene {
            width:100vw; height:100vh;
            native-material:glass;
            glass-tint:rgba(18,25,36,0.35);
        }
    "#,
    )?;
    App::new(
        done,
        &stylesheet,
        |_, _| Invalidation::Clean,
        |_| ui! { <div id="scene"></div> },
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
            Ok(Box::new(Cube {
                usd: UsdRenderer::new(init, scene, UsdCameraMode::Orbit)?,
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
