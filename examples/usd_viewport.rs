//! cargo run --features gpu-viewport --example usd_viewport -- [file.usd] [--transparent] [--continuous] [--smoke]
use cssimpler::{
    app::{App, Invalidation},
    renderer::{
        self, RedrawSchedule, WindowConfig,
        gpu::{
            GpuContext, GpuViewportFrame, GpuViewportInfo, GpuViewportOptions, GpuViewportRenderer,
            usd::{UsdCameraMode, UsdRenderer, UsdScene},
        },
    },
    style::parse_stylesheet,
    ui,
};
use std::{cell::Cell, rc::Rc};

struct Preview {
    renderer: UsdRenderer,
    frames: Rc<Cell<u64>>,
    done: Rc<Cell<bool>>,
    smoke: bool,
}
impl GpuViewportRenderer for Preview {
    fn resize(&mut self, gpu: &GpuContext, info: GpuViewportInfo) -> renderer::Result<()> {
        self.renderer.resize(gpu, info)
    }
    fn event(&mut self, event: &renderer::EngineEvent) -> bool {
        self.renderer.event(event)
    }
    fn render(&mut self, gpu: &GpuContext, frame: GpuViewportFrame<'_>) -> renderer::Result<()> {
        self.renderer.render(gpu, frame)?;
        self.frames.set(self.frames.get() + 1);
        if self.smoke && self.frames.get() == 120 {
            let cpu = renderer::latest_frame_timing_stats();
            eprintln!(
                "GPU smoke: gpu_frames={} last_cpu_frame={} cpu_paint_us={} cpu_present_us={}",
                self.frames.get(),
                cpu.frame_index,
                cpu.paint_us,
                cpu.present_us
            );
            self.done.set(true);
        }
        Ok(())
    }
}
fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let path = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .cloned()
        .unwrap_or_else(|| {
            format!(
                "{}/examples/assets/usd-preview.usda",
                env!("CARGO_MANIFEST_DIR")
            )
        });
    let transparent = args.iter().any(|a| a == "--transparent");
    let smoke = args.iter().any(|a| a == "--smoke");
    let continuous = smoke || args.iter().any(|a| a == "--continuous");
    let two_views = args.iter().any(|a| a == "--two-views");
    let camera = args
        .iter()
        .find_map(|a| a.strip_prefix("--camera="))
        .map(|path| UsdCameraMode::Authored(path.to_owned()))
        .unwrap_or_default();
    let scene = UsdScene::open(path)?;
    let primary_scene = scene.clone();
    let primary_camera = camera.clone();
    let done = Rc::new(Cell::new(false));
    let exit = done.clone();
    let primary_frames = Rc::new(Cell::new(0));
    let secondary_frames = Rc::new(Cell::new(0));
    let primary_counter = primary_frames.clone();
    let secondary_counter = secondary_frames.clone();
    let secondary_exit = done.clone();
    let stylesheet = parse_stylesheet(
        r#"
        .workspace { width:100vw; height:100vh; display:flex; background:#182638; color:#edf3ff; padding:24px; gap:24px; }
        .sidebar { width:220px; display:flex; flex-direction:column; gap:14px; }
        .viewports { flex:1; display:flex; gap:16px; }
        .viewport { flex:1; min-width:100px; min-height:100px; background:linear-gradient(135deg,#24445c,#82576d); }
        h1 { font-size:24px; } p { font-size:15px; }
    "#,
    )?;
    let mut app = App::new(
        done,
        &stylesheet,
        |_, _| Invalidation::Clean,
        move |_| {
            ui! {
                <div class="workspace">
                    <div class="sidebar">
                        <h1>USD viewport</h1>
                        <p>CPU-rendered CSSimpler UI</p>
                        <p>Hold the primary button to orbit or scroll to zoom.</p>
                        <input type="text" placeholder="CPU text input" />
                        <p>The native GPU surface renders independently.</p>
                    </div>
                    <div class="viewports">
                        <div id="scene" class="viewport"></div>
                        {two_views.then(|| ui! { <div id="scene2" class="viewport"></div> })}
                    </div>
                </div>
            }
        },
    )
    .with_gpu_viewport(
        "scene",
        GpuViewportOptions {
            transparent,
            redraw: if continuous {
                RedrawSchedule::EveryFrame
            } else {
                RedrawSchedule::OnInvalidation
            },
        },
        move |init| {
            Ok(Box::new(Preview {
                renderer: UsdRenderer::new(init, primary_scene, primary_camera)?,
                frames: primary_counter,
                done: exit,
                smoke,
            }))
        },
    )
    .with_exit_when(|done| done.get());
    if two_views {
        app = app.with_gpu_viewport(
            "scene2",
            GpuViewportOptions {
                transparent,
                ..Default::default()
            },
            move |init| {
                Ok(Box::new(Preview {
                    renderer: UsdRenderer::new(init, scene, camera)?,
                    frames: secondary_counter,
                    done: secondary_exit,
                    smoke: false,
                }))
            },
        );
    }
    app.run(WindowConfig::new("CSSimpler USD viewport", 1000, 640))?;
    if smoke {
        anyhow::ensure!(
            primary_frames.get() >= 120,
            "primary viewport did not complete the smoke test"
        );
        if two_views {
            anyhow::ensure!(
                secondary_frames.get() >= 1,
                "secondary viewport did not render"
            );
            eprintln!("GPU smoke: secondary_gpu_frames={}", secondary_frames.get());
        }
    }
    Ok(())
}
