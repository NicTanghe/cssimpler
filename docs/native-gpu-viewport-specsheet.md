# Native GPU and USD viewport specsheet

Date: 2026-11-09  
Status: **Planned — not implemented**

This document records the agreed implementation plan. It does not introduce a
Cargo feature, add dependencies, or implement a renderer. The milestones and
acceptance criteria below describe future work.

## Objective

Allow an application to embed GPU-rendered content inside CSSimpler's CPU-rendered
UI without adopting another UI engine or moving UI rasterization to the GPU.

An optional `gpu-viewport` Cargo feature will enable separate native surfaces for
viewport elements. CSSimpler will continue to render and present the surrounding
UI through its existing CPU path. Each GPU viewport will present through `wgpu`,
and the operating system will compose it with the parent UI.

Transparent viewport pixels must reveal the CSSimpler UI beneath the native
surface, including CSSimpler's existing glass background where available. Glass
remains owned by CSSimpler and its existing native presenter; the 3D renderer does
not implement glass or sample it. This is background composition beneath 3D, not a
requirement to blur live 3D through glass placed above it.

```text
CSSimpler layout and input
    |
    +-- ordinary UI --> CPU rasterizer --> existing parent presenter --+
    |                                                               |
    +-- viewport element --> native child surface --> wgpu -----------+--> OS composition
```

GPU animation must not force CPU scene rebuilding, rasterization, or parent
presentation. Neither GPU readback nor uploading the CPU UI into GPU textures is
part of this design.

Native USD rendering is also part of the planned feature: an application supplies
a USD file and a camera mode, and CSSimpler loads and renders the scene. Use the
native Rust [`openusd` crate](https://docs.rs/openusd/latest/openusd/) for file
loading and the internal scene representation, with its matching
[`openusd-schemas` crate](https://docs.rs/openusd-schemas/latest/openusd_schemas/)
for standard geometry, camera, material, and lighting schemas. USD is not merely an
import format that gets converted into a second CSSimpler scene format.

## Scope and defaults

- Support Windows, macOS, Linux X11, and Linux Wayland.
- Provide both generic GPU viewport callbacks and a built-in USD viewport renderer.
  The generic path permits application-defined rendering; the USD path must render
  a file without application-supplied mesh conversion, shaders, or render callbacks.
- Use USD stages, prims, attributes, and standard schemas as the canonical internal
  representation for geometry, transforms, cameras, lights, materials, bindings,
  and scene animation. Do not introduce a parallel scene graph or serialized format.
- Preserve existing `App`, `FragmentApp`, `ui!`, CSS, CPU rendering, and native-glass
  behavior around viewports and beneath transparent viewport pixels.
- Support rectangular viewport content regions with opaque presentation by default
  and an explicit transparent presentation option. CPU controls remain around
  them. Full CSS composition of CPU content over live GPU content is deferred.
- Support multiple nonoverlapping viewports sharing one GPU device per runtime.
- Include native USD loading and rendering in the initial implementation. Keep
  cloth simulation, production PBR, and Bevy integration outside it. The generic
  API must permit later compute and render passes on the same device.
- Start with a built-in `wgpu` USD rendering backend. Plan to replace it with a
  suitable upstream Hydra-like renderer when that becomes available in the Rust
  USD stack; the application's USD scene representation must not change.
- These requirements concern native desktop applications; web and mobile embedding
  are outside this version.

The existing [native runtime documentation](./native-touch-ui.md) describes the
CPU window, input, and DPI behavior this extension must preserve. The GPU viewport
feature complements the broader [runtime roadmap](../doc/specsheet_pt3.md); it is
not a replacement GPU rasterizer for CSSimpler's UI.

---

## GPU1. Feature gating and viewport registration

Depends: existing `App`, `FragmentApp`, `SceneProvider`, and element IDs  
Status: planned

Purpose:
- Make GPU embedding an explicit opt-in while retaining the existing CPU-only
  dependency graph and runtime behavior by default.

Support:
- Add the root feature `gpu-viewport = ["cssimpler-renderer/gpu-viewport"]`, disabled
  by default. Gate GPU implementation and GPU-specific platform dependencies in
  the renderer crate behind its corresponding feature.
- Use `wgpu` 30.0 for the planned implementation and re-export it through
  `cssimpler::renderer::gpu` so consumers can use compatible GPU types.
- Gate `openusd` 0.8 and matching `openusd-schemas` 0.8 dependencies behind the same
  optional feature. Enable the `geom`, `shade`, and `lux` schema families and
  install their schema registry when opening or creating a stage.
- Add `with_gpu_viewport(element_id, options, renderer_factory)` to both `App` and
  `FragmentApp`, binding a registration to an existing element ID. For example,
  `<div id="scene" class="viewport"></div>` can reserve the region without new
  macro syntax or a new core DOM variant.
- Add a feature-gated, defaulted registry hook to `SceneProvider` for custom
  providers. Existing provider implementations must continue to compile.
- Treat an absent matching element as an inactive registration. Reject duplicate
  registrations and duplicate matching element IDs explicitly.

Acceptance:
- Default builds do not compile `wgpu`, `openusd`, `openusd-schemas`, or the new
  GPU-specific dependencies.
- Enabling the feature without registering a viewport does not initialize a GPU
  device or replace the parent presenter.
- Registration works through both application types and custom scene providers.

---

## GPU2. Shared GPU context and renderer callbacks

Depends: GPU1  
Status: planned

Purpose:
- Give application renderers access to a reusable GPU context while keeping native
  handles, surface acquisition, and presentation inside CSSimpler.

Support:
- Initialize one shared GPU device and queue lazily when a registered viewport
  first becomes visible. Allow optional required device features and limits for
  later GPU simulation use.
- Expose a `GpuViewportRenderer` trait with factory-based initialization, resize,
  input, and render callbacks.
- Supply the device and queue, target format, logical content bounds, physical
  dimensions, scale factor, and configured alpha mode through the callback context.
- Add a transparent presentation option to viewport configuration. Select a
  supported non-opaque surface alpha mode and define the renderer's output-alpha
  convention accordingly. Clear transparent backgrounds to zero alpha and ensure
  partially transparent pixels blend correctly without dark fringes.
- Supply an acquired texture view and command encoder to rendering callbacks so
  applications can encode compute and render passes. CSSimpler owns submission,
  surface configuration, acquisition, and presentation.
- Keep application GPU resources reusable across frames and temporary hiding.

Acceptance:
- An application renders without receiving or managing OS window handles.
- Compute output can be consumed by rendering on the shared device without a CPU
  round trip.
- Each native surface is checked for compatibility with the selected adapter and
  configured with a supported format. Unsupported configurations report errors.
- Requested transparency is checked against both GPU surface capabilities and
  native composition support. An alpha channel in the render target alone is not
  sufficient; unsupported transparent configurations report explicit errors rather
  than silently presenting an opaque background.

---

## GPU3. Native surface backends

Depends: GPU1, GPU2, existing window and native-glass lifecycle  
Status: planned

Purpose:
- Embed a separately presented GPU region while retaining the existing CPU
  presenter for the surrounding window.

Support:
- Windows: a child HWND host, with clipping for opaque presentation so CPU updates
  do not overwrite the child surface. Transparent presentation requires an
  alpha-capable native composition path that preserves the underlying CPU/glass
  background rather than leaving an unpainted hole.
- macOS: a child NSView backed by a Metal layer, coordinated with the existing
  native-glass view and layer hierarchy.
- X11: an embedded child window.
- Wayland: a `wl_surface` and `wl_subsurface` on the existing connection, using a
  separate guest event queue. Use desynchronized buffer commits for independent
  GPU animation and coordinate geometry changes with parent commits. Preserve
  winit's object ownership and connection lifetime.
- Make native hosts input-transparent so pointer and keyboard handling remains
  centralized in CSSimpler's parent runtime.
- Derive native geometry from the registered element's content box after layout
  and scrolling. Update it after layout, window resize, and DPI changes.
- Configure native host opacity and composition to honor the selected surface
  alpha mode. Keep the CSSimpler background beneath transparent viewports painted
  and presented when it changes. The 3D path has no glass-rendering responsibility.

Acceptance:
- Each of the four backends displays a viewport inside the intended content box.
- Parent CPU presentation remains active and does not erase the GPU image.
- Movement, resize, and fractional DPI do not produce stale positions or sizes.
- Existing native-glass setup, changes, and teardown do not hide, detach, or leak
  viewport hosts.
- Transparent GPU pixels reveal the underlying CSSimpler background, including
  existing glass where supported. Partially transparent pixels blend with that
  background correctly; opaque 3D pixels cover it normally.

---

## GPU4. Independent scheduling, lifecycle, and errors

Depends: GPU2, GPU3, existing invalidation and redraw scheduling  
Status: planned

Purpose:
- Update GPU content independently while preserving the CPU renderer's idle and
  incremental rendering behavior.

Support:
- Offer on-demand and continuous viewport redraw modes, defaulting to on-demand.
  Provide a cloneable redraw handle that wakes the existing event loop.
- Separate GPU-only redraws from application scene rebuilding, CPU rasterization,
  and parent presentation. Reuse cached layout and viewport eligibility until
  relevant UI state changes.
- Pause rendering while a viewport is hidden, its window is minimized or occluded,
  or the application is suspended. Avoid configuring zero-sized surfaces.
- Retain renderer resources while temporarily hidden; restore presentation when
  visible. Release GPU surfaces before native hosts, and hosts before their parent
  window, during removal and teardown.
- Reconfigure lost or outdated surfaces and skip temporary acquisition timeouts.
  Report initialization failures, device loss, and fatal rendering failures
  explicitly. Do not silently switch to a shared GPU presenter or CPU readback.

Acceptance:
- With static UI and continuous GPU animation, GPU frame counts advance while CPU
  paint and parent presentation counts remain unchanged after initialization.
- On-demand viewports sleep until invalidated instead of continuously polling.
- Suspend/resume, minimize/restore, element removal, and window closing leave no
  live native surface referencing a destroyed host.
- Recoverable acquisition failures do not close the application; fatal errors are
  returned or reported through the renderer's error path.

---

## GPU5. Input and focus routing

Depends: GPU1, GPU3, existing engine events and text input  
Status: planned

Purpose:
- Let applications interact with the GPU viewport without creating a competing
  native input loop or disturbing surrounding controls.

Support:
- Route pointer, wheel, keyboard, and existing primary-touch events to eligible
  viewports. Translate pointer coordinates into viewport-local logical pixels.
- Preserve application shortcuts and route keyboard input according to viewport
  focus. Coordinate focus changes with existing CPU text editing.
- Capture a viewport drag until release or cancellation, including movement
  outside its bounds. Cancel capture when hidden, removed, or unfocused.
- Prevent consumed viewport events from also scrolling or editing CPU controls.

Acceptance:
- Pointer coordinates remain correct after moving, scrolling, and DPI changes.
- Dragging outside a viewport delivers its final release or cancellation.
- Switching between viewport input and CPU text fields preserves focus behavior.
- A wheel event consumed by the viewport does not scroll the surrounding panel.

---

## GPU6. Reserved-rectangle composition policy

Depends: GPU3, GPU4, existing paint order and clipping information  
Status: planned

Purpose:
- Define predictable behavior for separate native surfaces without promising full
  CSS compositing support in the first version.

Support:
- Viewport content is rectangular and childless, with optional per-pixel alpha.
  CPU-rendered borders and padding remain outside the native content region.
- In transparent mode, compose GPU output over the existing CSSimpler background.
  CSSimpler glass beneath the viewport is allowed and is not an unsupported effect
  on the GPU surface. Per-pixel output alpha is distinct from CSS element opacity.
- Hide the entire native surface while partially clipped or covered by UI painted
  above it. Determine overlap from CSSimpler's paint order, not DOM order alone.
- Also hide a viewport affected by unsupported transforms, rounded clipping,
  opacity, or filters. Restore it when its content rectangle becomes eligible.
- Hide surfaces before presenting covering CPU UI so menus and dialogs cannot
  appear behind a native viewport. Restore surfaces after the covering UI clears.
- Keep the element's CPU-rendered background available while its surface is
  hidden, and beneath transparent pixels while it is visible. Do not read back or
  freeze the GPU image into that background.
- Defer full CPU/GPU CSS stacking, partially clipped native viewports, and backdrop
  effects that sample live GPU content. This does not defer showing CSSimpler's
  existing glass beneath transparent GPU pixels.

Acceptance:
- A modal or menu covering a viewport hides the native surface and remains usable.
- Partly scrolled-out viewports do not escape their intended clip bounds.
- Unsupported visual effects hide the viewport rather than display it incorrectly.
- Multiple nonoverlapping viewports render independently using the shared device.
- Transparent regions show the underlying UI/glass without a glass pass in the
  3D renderer, CPU readback, or CPU-UI texture uploads.

---

## GPU7. Example, tests, and acceptance evidence

Depends: GPU1 through GPU6, USD1

Status: planned

Purpose:
- Prove native embedding, built-in USD rendering, CPU/GPU scheduling separation,
  and feature isolation before applications build simulation integrations on it.

Support:
- Add a feature-gated example with an animated depth-tested mesh, CPU sidebar
  controls, resizing, on-demand/continuous switching, and a covering modal. Include
  opaque and transparent backgrounds, with CSSimpler glass underneath where
  available. Require the feature explicitly for this example target.
- Add mock-backed tests for registration, content-box geometry, fractional DPI,
  visibility and overlap decisions, input capture, scheduling, and surface recovery.
- Add a USD example that accepts a file and camera mode without custom render
  callbacks. Include a scene with geometry, authored cameras, materials, and lights,
  plus an orbit view and time-sampled mesh playback. Show the same stage in two
  widgets with independent cameras and reusable GPU resources.
- Test `.usda`, `.usdc`, and `.usdz` inputs, referenced assets, stage transforms and
  units, camera selection, stage edits, and animation-time changes. Verify missing
  files, invalid camera paths, missing dependencies, and unsupported rendering
  features produce useful diagnostics. Check that runtime camera changes do not
  modify the source file.
- Run existing workspace tests and example checks with the feature disabled and
  enabled. Inspect the default dependency graph for GPU dependency leakage.
- Record native smoke tests on Windows, macOS, X11, and Wayland, covering input,
  focus, resize/DPI, scrolling, minimize/restore, native glass where available, and
  teardown. Validate fully transparent, partially transparent, and opaque GPU
  pixels over a changing CPU background; verify existing glass separately on
  platforms where it is available.
- Instrument CPU paint/presentation counts and GPU frame counts to demonstrate
  scheduling separation rather than infer it from visual smoothness.

Acceptance:
- Existing CPU-only tests and examples continue to pass.
- The animated example demonstrates GPU activity without CPU repaint or parent
  presentation on GPU-only frames.
- No per-frame GPU readback or CPU-UI texture upload is introduced.
- Transparent presentation reveals the live CPU background without black/opaque
  replacement pixels or blend fringes. Glass remains entirely in CSSimpler's
  existing UI/native presentation path.
- A supported USD scene renders from a file and camera mode using the built-in
  renderer. Geometry, cameras, materials, and lights retain their USD identities;
  changing USD attributes updates the corresponding rendering state.
- Cross-platform support has native execution evidence for all four backends.
  Cross-compilation or build-only checks must be recorded separately and must not
  be presented as native runtime verification.

---

## USD1. Native USD scene model and built-in rendering

Depends: GPU1, GPU2, GPU4, GPU5

Status: planned

Purpose:
- Make USD the native scene model and provide a direct file-to-viewport experience
  without requiring applications to assemble a renderer themselves.

Support:
- Load USD through the requested `openusd` crate, using its composed `usd::Stage`
  rather than parsing layers into an application-specific geometry format. Accept
  an existing in-memory stage as well as a file so programmatically created scenes
  use the same rendering path.
- Add a high-level `with_usd_viewport(element_id, file_or_stage, camera_mode)`
  registration API to both `App` and `FragmentApp`. Expose optional settings for
  shading, animation time, background alpha, and redraw scheduling while keeping
  file plus camera mode sufficient for a visible first render.
- Offer an interactive orbit camera mode that frames scene bounds by default, and
  an authored-camera mode selecting a USD camera prim by path. Honor the selected
  camera's USD projection and optical attributes. Represent generated camera poses
  and projection in USD camera prims in a transient session layer, without saving
  changes to the input file. Give each view its own camera identity.
- Read geometry, transforms, visibility, normals, primvars, and time-sampled values
  through USD. Respect stage axis/unit metadata and camera conventions when
  deriving GPU transforms. Start with mesh-based scenes and cloth cache playback;
  do not imply every USD primitive or extension is renderable in the first backend.
- Use `UsdShade` materials and bindings and `UsdLux` lights for internal material
  and lighting representation. Light types, transforms, colors, intensities, and
  animation remain USD data; do not add a parallel CSSimpler lighting format.
- Provide built-in matcap shading for the initial file-and-camera experience,
  including scenes without authored lighting. A matcap is a viewport display
  choice; it does not replace or rewrite the scene's USD materials and lights.
  Document the initial backend's supported geometry, material, and lighting
  subset, report unsupported features, and preserve their USD data for a later
  renderer. Full production shading remains deferred.
- Derive vertex/index buffers, evaluated matrices, textures, material uniforms,
  lighting buffers, and draw commands as disposable rendering caches keyed by
  stage/prim identity. These caches are necessary GPU execution data, not a second
  authoritative scene graph, editable scene model, or asset format.
- Reuse a stage across logical views and reuse compatible GPU assets on the shared
  device. Keep camera, time, and shading choices view-specific. Give the renderer
  a target rectangle so logical views are not inherently tied to one native
  surface each; later surface grouping must not require another scene model.
- Invalidate derived render data when relevant USD changes or view-time changes
  occur. Such changes redraw GPU content without forcing unrelated CPU UI repaint.
- Keep stage ownership and mutation on its owning thread. The inspected crate's
  `Stage` handle is neither `Send` nor `Sync`; sharing between views must not rely
  on moving it across worker threads or assuming that `Arc` makes it thread-safe.

Acceptance:
- A file path and camera mode render a supported USD scene with built-in shading.
- Geometry, cameras, lights, and materials are queried and edited as USD data;
  there is no required conversion to glTF or a custom CSSimpler scene format.
- A stage created in memory renders through the same API as a file-backed stage.
- Two logical widgets can render the same stage with independent cameras without
  reparsing its file or duplicating an authoritative scene representation.
- Runtime camera changes leave the source asset unchanged. Unsupported features
  are diagnosed while their authored data remains in the stage.

---

## USD2. Future upstream Hydra-like backend

Depends: USD1 and a suitable upstream Rust USD rendering implementation

Status: deferred until upstream support exists

Purpose:
- Allow adoption of upstream scene-to-renderer integration without migrating
  application scenes or coupling the public viewport API to the first backend.

Support:
- Keep a private renderer-backend boundary accepting USD stage/prim identities,
  camera selection, evaluation time, viewport display settings, and a GPU target.
  The first implementation evaluates USD and issues `wgpu` commands itself.
- Replace that backend with an upstream Hydra-like implementation when available
  and suitable for the native surfaces and GPU resource requirements. This is an
  intended migration, not a claim that the current crate already supplies Hydra
  or has committed to a particular future rendering API.
- Preserve the file/stage-plus-camera entry point, native surface lifecycle,
  transparent presentation, CPU UI separation, and canonical USD representation.
  Do not introduce a custom intermediate scene format to emulate future Hydra.

Acceptance for the future migration:
- Applications keep their USD assets, stage-based scene editing, and viewport
  registration APIs when the backend changes.
- The USD rendering and native composition acceptance suite can exercise either
  backend. Backend-specific GPU resources remain derived caches.
- Upstream renderer availability does not block completion of the initial `wgpu`
  backend and USD1; USD2 is a later migration milestone.

## Implementation order and completion gate

Implement GPU1 and GPU2 first, then GPU3 for all four backends. Integrate GPU4 and
GPU5 with the existing runtime, apply GPU6 before exposing general application use,
implement USD1 on that infrastructure, and finish with GPU7's full validation
matrix. USD2 remains a future upstream-dependent migration.

Every milestone remains planned until implemented and verified. This specsheet
does not authorize changing the selected architecture to a shared GPU presenter,
dropping a desktop backend, or adding a cloth solver or production PBR to the
initial scope. Native USD loading, scene representation, and built-in rendering
are explicitly included in the initial scope.

## USD API references

- [`openusd` crate](https://docs.rs/openusd/latest/openusd/): requested native Rust
  USD implementation and file-format entry point.
- [`openusd::usd::Stage`](https://docs.rs/openusd/latest/openusd/usd/struct.Stage.html):
  composed scene access, edits, session layers, change notifications, and ownership
  constraints.
- [`openusd-schemas`](https://docs.rs/openusd-schemas/latest/openusd_schemas/): typed
  geometry, camera, material, and lighting schema views and registry setup.
