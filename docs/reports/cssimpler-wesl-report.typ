#let ink = rgb("#203037")
#let muted = rgb("#526269")
#let teal = rgb("#12645C")
#let tint = rgb("#EAF4F1")
#let rule = rgb("#CBDAD6")
#let code-bg = rgb("#F3F6F5")
#let sans = ("Noto Sans", "DejaVu Sans")
#let serif = "Libertinus Serif"

#set document(
  title: "WESL in the cssimpler USD viewport",
  description: "Engineering report on build-time shader compilation in cssimpler PR 59.",
)
#set page(
  paper: "a4",
  margin: (x: 20mm, top: 26mm, bottom: 22mm),
  header: [
    #set text(font: sans, size: 8pt, fill: muted)
    #text(weight: "bold", fill: teal)[CSSIMPLER]
    #h(1fr)
    ENGINEERING REPORT
    #v(2mm)
    #line(length: 100%, stroke: 0.55pt + rule)
  ],
  footer: context [
    #line(length: 100%, stroke: 0.55pt + rule)
    #v(1.5mm)
    #set text(font: sans, size: 8pt, fill: muted)
    USD viewport · WESL pilot
    #h(1fr)
    #counter(page).display("1")
  ],
)
#set text(font: sans, size: 10.3pt, fill: ink, lang: "en")
#set par(leading: 0.66em, spacing: 0.85em)
#set heading(numbering: none)
#show heading.where(level: 1): set text(font: serif, size: 26pt)
#show heading.where(level: 2): set text(font: sans, size: 12.5pt, fill: teal)
#show heading.where(level: 2): set block(above: 1.25em, below: 0.65em)
#show link: set text(fill: teal)
#show raw: set text(font: "DejaVu Sans Mono", size: 0.9em)
#show raw.where(block: true): it => block(
  width: 100%,
  fill: code-bg,
  stroke: (left: 1.5pt + rule),
  inset: (x: 9pt, y: 8pt),
  radius: 2pt,
  breakable: false,
  it,
)

#let repo = "https://github.com/NicTanghe/cssimpler"
#let commit = "c0d31b7f6da22cd2f6087b9ac4d1f296a76278db"
#let pr = link(repo + "/pull/59")[cssimpler PR \#59]
#let source(path, label) = link(repo + "/blob/" + commit + "/" + path, label)
#let stage(title, description) = block(
  width: 100%,
  height: 25mm,
  fill: tint,
  radius: 3pt,
  inset: 8pt,
  [
    #text(size: 9.2pt, weight: "bold", fill: teal, title)
    #v(0.7mm)
    #text(size: 9pt, description)
  ],
)
#let arrow = align(center + horizon, text(size: 12pt, fill: teal)[→])

#text(font: serif, size: 35pt, weight: "bold")[WESL in the USD viewport]
#v(1mm)
#text(size: 11.6pt, fill: muted)[Build-time shader compilation for cssimpler]
#v(4mm)
#text(size: 9pt, fill: muted)[23 November 2026] #h(8pt) #text(size: 9pt)[·] #h(8pt) #text(size: 9pt)[#pr]
#v(4mm)

PR \#59 moves shader assembly into the Cargo build for cssimpler’s built-in USD
viewport. Rust previously joined the main WGSL shader with either the shadow
shader or an inline fallback. WESL adds imports and conditional translation to
WGSL, so the shader files now declare that dependency directly. The pilot reuses
the existing studio lighting equations, shadow filter, and uniform layout.
#link(repo + "/pull/59/files")[PR diff]

== Shader compilation and selection

#grid(
  columns: (1fr, 15pt, 1fr, 15pt, 1fr),
  align: horizon,
  stage([WESL source], [The viewport, studio uniform, and shadow sampling live in shader modules.]),
  arrow,
  stage([Cargo build], [WESL emits two WGSL variants. Naga parses and validates both.]),
  arrow,
  stage([wgpu runtime], [The renderer chooses embedded WGSL for the GPU adapter and render pass.]),
)

wgpu still uses Naga to process WGSL for the selected backend. cssimpler also
calls Naga during the build, so both variants are validated before an adapter
is available. #source("renderer/build.rs", [Build script])

== Advantages for shader development

- The viewport and shadow modules *share one studio uniform definition*. A
  layout edit goes in that module and should match Rust’s #raw("StudioUniform").
- *One viewport source produces both variants.* The fallback resolves key-light
  visibility to 1.0 and omits the depth texture and comparison sampler, so its
  pipeline can run without those bindings.
- *Shader errors can stop the Cargo build.* Both variants are checked in the
  same build, and Cargo tracks changes to imported shader files. At runtime,
  the renderer uses embedded shader text without assembling strings.

The optional #raw("gpu-viewport") feature activates the shader build dependencies.
WESL 0.6 requires Rust 1.97.1 or newer.
#source("renderer/Cargo.toml", [Renderer manifest])
· #source("docs/gpu-viewports.md", [Viewport documentation])

#pagebreak()

= Authoring and building the shader

== WESL imports and the fallback

The viewport imports the studio uniform in both builds and the shadow visibility
function when #raw("shadow_maps") is enabled. The fallback has the same function
signature, keeping the shading call site shared. This excerpt omits the camera
declarations between the imports and fallback.
#source("renderer/src/gpu/usd/viewport.wesl", [Viewport source])

```wgsl
import package::studio_uniform::studio;
@if(shadow_maps) import package::shadow::key_visibility;

@if(!shadow_maps)
fn key_visibility(
    position: vec3<f32>, normal: vec3<f32>, n_dot_l: f32
) -> f32 {
    return 1.0;
}
```

The shadow module imports the same studio uniform and owns the two group 2
bindings. Studio stays at group 0, binding 1 in the generated WGSL.
#source("renderer/src/gpu/usd/shadow.wesl", [Shadow module])
· #source("renderer/src/gpu/usd/studio_uniform.wesl", [Uniform module])

== Cargo generation and Naga validation

The build script sets #raw("shadow_maps") for each artifact, compiles the same WESL
entry module twice, and writes validated WGSL to Cargo’s #raw("OUT_DIR"). This excerpt
shortens error handling. The implementation reports which variant failed and
provides compiler diagnostics.
#source("renderer/build.rs", [Build script])

```rust
use naga::valid::{Capabilities, ValidationFlags, Validator};
use wesl::{Compiler, Feature};

let mut compiler = Compiler::default();
compiler.options.features.default = Feature::Error;
for (name, enabled) in [("usd_shadows", true), ("usd_no_shadows", false)] {
    compiler.options.features.set("shadow_maps", enabled);
    let shader = compiler.compile("src/gpu/usd/viewport.wesl").unwrap();
    shader.emit_rerun_if_changed();

    let wgsl = shader.to_string();
    let module = naga::front::wgsl::parse_str(&wgsl).unwrap();
    Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .unwrap();
    shader.write_artifact(name);
}
```

Edit the WESL sources and rerun the GPU build to regenerate both artifacts.
Changes to a Rust-side studio setting such as shadow strength still flow
through the uniform at runtime. #source("renderer/src/gpu/usd.rs", [USD renderer])

#pagebreak()

= Runtime selection and verification

== Choosing the shader variant

The mesh pipeline uses #raw("usd_shadows.wgsl") when the adapter supports the depth map
and comparison sampler. The fallback #raw("usd_no_shadows.wgsl") also serves the
wireframe and shadow depth passes, which don’t sample the shadow map. Both
artifacts preserve the original entry points. #source("renderer/src/gpu/usd.rs", [USD renderer])
· #source("renderer/src/gpu/usd/studio.rs", [Studio adapter checks])

```rust
fn usd_shader(gpu: &GpuContext, shadows_supported: bool) -> wgpu::ShaderModule {
    let source = if shadows_supported {
        include_str!(concat!(env!("OUT_DIR"), "/usd_shadows.wgsl"))
    } else {
        include_str!(concat!(env!("OUT_DIR"), "/usd_no_shadows.wgsl"))
    };
    gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("USD studio shading"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    })
}
```

== Reproducing the build checks

Run these from the cssimpler checkout. The first command builds and validates
both generated shaders without a GPU. The second checks the renderer without
the GPU feature.

```bash
cargo check --locked -p cssimpler --all-targets --features gpu-viewport
cargo check --locked -p cssimpler-renderer --no-default-features
```

== Verification results

Both build checks passed. The generated shaders had the expected entry points
and shared bindings, with the group 2 shadow resources present only in the
shadow variant. On Linux software Vulkan, 18 USD tests passed, including studio
shadow and wireframe rendering. Three strip versus full-frame MSAA comparisons
failed with identical pixel differences on the PR branch and #raw("main").
#link(repo + "/pull/59")[PR verification]

== ClothCraft dependency

ClothCraft’s desktop feature enables cssimpler’s #raw("gpu-viewport") feature and
tracks cssimpler #raw("main"). A desktop build picks up the WESL change when
ClothCraft’s lockfile resolves cssimpler to a revision that includes it.
#link("https://github.com/NicTanghe/ClothCraft/blob/main/Cargo.toml")[ClothCraft manifest]
