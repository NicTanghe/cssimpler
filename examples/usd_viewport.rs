//! cargo run --features gpu-viewport --example usd_viewport -- [file.usd] [--studio] [--transparent] [--continuous] [--smoke] [--wireframe=off|edges|silhouette|overlay]
use cssimpler::{
    app::{App, Invalidation},
    core::Node,
    renderer::{
        self, RedrawSchedule, WindowConfig,
        gpu::{
            GpuContext, GpuRedrawHandle, GpuViewportFrame, GpuViewportInfo, GpuViewportOptions,
            GpuViewportRenderer,
            usd::{
                UsdCameraMode, UsdMaterial, UsdRenderer, UsdScene, UsdStudioLighting, Wireframe,
                WireframeMode, WireframeStyle,
            },
        },
    },
    style::parse_stylesheet,
    ui,
};
use std::{
    cell::Cell,
    collections::HashMap,
    rc::Rc,
    sync::{Arc, Mutex},
};

const WIDTHS: [f32; 6] = [1.0, 2.0, 3.0, 4.0, 6.0, 8.0];

/// Name, core colour and outline colour. A second colour gives an outline on
/// both sides of the core; equal colours give one solid band.
const PALETTE: [(&str, [f32; 3], [f32; 3]); 5] = [
    ("black", [0.0, 0.0, 0.0], [0.0, 0.0, 0.0]),
    ("outlined", [0.82, 0.88, 0.95], [0.0, 0.0, 0.0]),
    ("cyan", [0.10, 0.85, 1.00], [0.0, 0.0, 0.0]),
    ("amber", [1.00, 0.72, 0.15], [0.0, 0.0, 0.0]),
    ("red", [1.00, 0.20, 0.20], [0.0, 0.0, 0.0]),
];

/// Viewport default plus per-mesh overrides keyed by USD prim path.
#[derive(Default)]
struct Wires {
    default: Wireframe,
    models: HashMap<String, Wireframe>,
}

impl Wires {
    fn get(&self, path: &str) -> Wireframe {
        self.models.get(path).copied().unwrap_or(self.default)
    }
}

fn cycle_width(wireframe: Wireframe) -> Wireframe {
    let index = WIDTHS
        .iter()
        .position(|width| *width >= wireframe.style.width_px)
        .unwrap_or(0);
    let width_px = WIDTHS[(index + 1) % WIDTHS.len()];
    Wireframe {
        style: WireframeStyle {
            width_px,
            ..wireframe.style
        },
        ..wireframe
    }
}

fn palette_index(wireframe: Wireframe) -> Option<usize> {
    PALETTE.iter().position(|(_, core, outline)| {
        *core == wireframe.style.core && *outline == wireframe.style.outline
    })
}

fn cycle_colour(wireframe: Wireframe) -> Wireframe {
    let index = palette_index(wireframe).map_or(0, |index| (index + 1) % PALETTE.len());
    let (_, core, outline) = PALETTE[index];
    Wireframe {
        style: WireframeStyle {
            core,
            outline,
            ..wireframe.style
        },
        ..wireframe
    }
}

fn colour_name(wireframe: Wireframe) -> &'static str {
    palette_index(wireframe).map_or("custom", |index| PALETTE[index].0)
}

fn edit(wires: &Mutex<Wires>, path: Option<&str>, apply: impl FnOnce(Wireframe) -> Wireframe) {
    let mut state = wires.lock().unwrap_or_else(|e| e.into_inner());
    match path {
        Some(path) => {
            let current = state.get(path);
            state.models.insert(path.to_owned(), apply(current));
        }
        None => state.default = apply(state.default),
    }
}

fn poke(redraws: &Mutex<Vec<GpuRedrawHandle>>) {
    let handles: Vec<_> = redraws.lock().unwrap_or_else(|e| e.into_inner()).clone();
    for handle in handles {
        handle.request_redraw();
    }
}

/// One control row: mode, thickness and colour for a single mesh, or for the
/// viewport default when `path` is `None`.
fn control_row(
    path: Option<String>,
    name: &str,
    wireframe: Wireframe,
    wires: &Arc<Mutex<Wires>>,
    redraws: &Arc<Mutex<Vec<GpuRedrawHandle>>>,
) -> Node {
    let handler = |apply: fn(Wireframe) -> Wireframe| {
        let wires = wires.clone();
        let redraws = redraws.clone();
        let path = path.clone();
        move || {
            edit(&wires, path.as_deref(), apply);
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
    let colour_label = colour_name(wireframe).to_owned();
    ui! {
        <div class="model">
            <span class="model-name">{name.to_owned()}</span>
            <button class="wire-button" onclick_shared={mode}>{mode_label}</button>
            <button class="wire-button" onclick_shared={width}>{width_label}</button>
            <button class="wire-button" onclick_shared={colour}>{colour_label}</button>
        </div>
    }
}

struct Preview {
    renderer: UsdRenderer,
    wires: Arc<Mutex<Wires>>,
    models: Arc<Mutex<Vec<String>>>,
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
        // Publish the model list for the CPU controls and mirror the shared
        // settings onto the renderer. Both calls are change-detected.
        let models = self.renderer.models().to_vec();
        {
            let mut published = self.models.lock().unwrap_or_else(|e| e.into_inner());
            if *published != models {
                *published = models.clone();
            }
        }
        let state = self.wires.lock().unwrap_or_else(|e| e.into_inner());
        self.renderer.set_wireframe_default(state.default);
        for path in &models {
            self.renderer.set_model_wireframe(path, state.get(path));
        }
        drop(state);
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

fn preview_renderer(
    init: renderer::gpu::GpuViewportInit<'_>,
    scene: UsdScene,
    camera: UsdCameraMode,
    studio: bool,
) -> renderer::Result<UsdRenderer> {
    let mut renderer = UsdRenderer::new(init, scene, camera)?;
    if studio {
        renderer.set_studio_lighting(Some(UsdStudioLighting {
            toward_key: [-0.55, 0.85, -0.70],
            ..Default::default()
        }))?;
        renderer.set_model_material("/Studio/Receiver", UsdMaterial::fabric([0.40, 0.11, 0.06]))?;
        renderer.set_model_material(
            "/Studio/Caster",
            UsdMaterial::brushed_metal([0.65, 0.72, 0.80]),
        )?;
    }
    Ok(renderer)
}

fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let studio = args.iter().any(|a| a == "--studio");
    let path = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .cloned()
        .unwrap_or_else(|| {
            let asset = if studio {
                "tests/fixtures/usd-studio-shadows.usda"
            } else {
                "examples/assets/usd-preview.usda"
            };
            format!("{}/{asset}", env!("CARGO_MANIFEST_DIR"))
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
    let initial = match args.iter().find_map(|a| a.strip_prefix("--wireframe=")) {
        Some(name) => WireframeMode::from_label(name)
            .ok_or_else(|| anyhow::anyhow!("unknown wireframe mode: {name}"))?,
        None => WireframeMode::default(),
    };
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
    let wires = Arc::new(Mutex::new(Wires {
        default: Wireframe {
            mode: initial,
            ..Default::default()
        },
        ..Default::default()
    }));
    let models: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    // Renderers are created lazily, so each factory registers its wake handle
    // here and the controls start driving the viewports once they exist.
    let redraws: Arc<Mutex<Vec<GpuRedrawHandle>>> = Arc::new(Mutex::new(Vec::new()));
    let primary_wires = wires.clone();
    let primary_models = models.clone();
    let primary_redraws = redraws.clone();
    let secondary_wires = wires.clone();
    let secondary_models = models.clone();
    let secondary_redraws = redraws.clone();
    let ui_wires = wires.clone();
    let ui_models = models.clone();
    let ui_redraws = redraws.clone();
    let stylesheet = parse_stylesheet(
        r#"
        .workspace { width:100vw; height:100vh; display:flex; background:#182638; color:#edf3ff; padding:24px; gap:24px; }
        .sidebar { width:260px; display:flex; flex-direction:column; gap:12px; }
        .viewports { flex:1; display:flex; gap:16px; }
        .viewport { flex:1; min-width:100px; min-height:100px; background:linear-gradient(135deg,#24445c,#82576d); }
        .wire-controls { display:flex; flex-direction:column; gap:6px; }
        .model { display:flex; align-items:center; gap:6px; }
        .model-name { width:110px; font-size:12px; color:#a9bdd4; }
        .wire-button { padding:5px 8px; font-size:12px; }
        h1 { font-size:24px; } p { font-size:15px; }
    "#,
    )?;
    let mut app = App::new(
        done,
        &stylesheet,
        |_, _| Invalidation::Clean,
        move |_| {
            let models = ui_models.lock().unwrap_or_else(|e| e.into_inner()).clone();
            let state = ui_wires.lock().unwrap_or_else(|e| e.into_inner());
            let mut rows = Vec::with_capacity(models.len() + 1);
            rows.push(control_row(
                None,
                "all models",
                state.default,
                &ui_wires,
                &ui_redraws,
            ));
            for path in &models {
                rows.push(control_row(
                    Some(path.clone()),
                    path,
                    state.get(path),
                    &ui_wires,
                    &ui_redraws,
                ));
            }
            drop(state);
            ui! {
                <div class="workspace">
                    <div class="sidebar">
                        <h1>USD viewport</h1>
                        <p>Hold the primary button to orbit or scroll to zoom.</p>
                        <input type="text" placeholder="CPU text input" />
                        <div class="wire-controls">{rows}</div>
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
            primary_redraws
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(init.redraw.clone());
            Ok(Box::new(Preview {
                renderer: preview_renderer(init, primary_scene, primary_camera, studio)?,
                wires: primary_wires,
                models: primary_models,
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
                secondary_redraws
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(init.redraw.clone());
                Ok(Box::new(Preview {
                    renderer: preview_renderer(init, scene, camera, studio)?,
                    wires: secondary_wires,
                    models: secondary_models,
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
