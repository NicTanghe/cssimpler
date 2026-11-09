# Native GPU viewport specsheet

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

## Scope and defaults

- Support Windows, macOS, Linux X11, and Linux Wayland.
- Provide generic GPU viewport registration and rendering callbacks. Applications
  remain responsible for their scene data, shaders, and rendering pipelines.
- Preserve existing `App`, `FragmentApp`, `ui!`, CSS, CPU rendering, and native-glass
  behavior around viewports and beneath transparent viewport pixels.
- Support rectangular viewport content regions with opaque presentation by default
  and an explicit transparent presentation option. CPU controls remain around
  them. Full CSS composition of CPU content over live GPU content is deferred.
- Support multiple nonoverlapping viewports sharing one GPU device per runtime.
- Keep USD loading, cloth simulation, production PBR, and Bevy integration outside
  the initial implementation. The generic API must permit later compute and render
  passes on the same device.
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
- Add `with_gpu_viewport(element_id, options, renderer_factory)` to both `App` and
  `FragmentApp`, binding a registration to an existing element ID. For example,
  `<div id="scene" class="viewport"></div>` can reserve the region without new
  macro syntax or a new core DOM variant.
- Add a feature-gated, defaulted registry hook to `SceneProvider` for custom
  providers. Existing provider implementations must continue to compile.
- Treat an absent matching element as an inactive registration. Reject duplicate
  registrations and duplicate matching element IDs explicitly.

Acceptance:
- Default builds do not compile `wgpu` or the new GPU-specific dependencies.
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

Depends: GPU1 through GPU6  
Status: planned

Purpose:
- Prove native embedding, CPU/GPU scheduling separation, and feature isolation
  before applications build USD or simulation integrations on this API.

Support:
- Add a feature-gated example with an animated depth-tested mesh, CPU sidebar
  controls, resizing, on-demand/continuous switching, and a covering modal. Include
  opaque and transparent backgrounds, with CSSimpler glass underneath where
  available. Require the feature explicitly for this example target.
- Add mock-backed tests for registration, content-box geometry, fractional DPI,
  visibility and overlap decisions, input capture, scheduling, and surface recovery.
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
- Cross-platform support has native execution evidence for all four backends.
  Cross-compilation or build-only checks must be recorded separately and must not
  be presented as native runtime verification.

## Implementation order and completion gate

Implement GPU1 and GPU2 first, then GPU3 for all four backends. Integrate GPU4 and
GPU5 with the existing runtime, apply GPU6 before exposing general application use,
and finish with GPU7's full validation matrix.

Every milestone remains planned until implemented and verified. This specsheet
does not authorize changing the selected architecture to a shared GPU presenter,
dropping a desktop backend, or adding USD, a cloth solver, or production PBR to the
initial scope.
