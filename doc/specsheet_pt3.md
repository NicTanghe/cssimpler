# Rust UI Engine Specsheet Part 3
Continuation roadmap for platform/runtime rearchitecture: `winit`, ECS, an optional GPU renderer, and compile-time baked UI assets.

This document continues [specsheet_pt2.md](./specsheet_pt2.md).

Planning note:

- Part 3 is more architectural than parts 1 and 2.
- The goal is to modernize the runtime without giving up the deterministic explicit render model.
- The current CPU renderer should remain a correctness reference and fallback while the new path lands.
- The existing `App`, `ui!`, CSS pipeline, and examples should stay usable through the migration instead of being rewritten all at once.

---

# Epic X - `winit` platform runtime

## X1. Window/event loop migration to `winit`
Depends: A2, E2, F2, G1  
Status: planned

Purpose:
- Replace `minifb` windowing with `winit` so the engine has a modern event loop, better platform coverage, and window handles suitable for GPU surfaces

Should have been an extension:
- This would ideally have extended A2 and E2 because it replaces the runtime shell beneath the current app and renderer loop

Support:
- Window creation and lifecycle through `winit`
- Resize, scale-factor, close, focus, mouse, wheel, keyboard, and text-input events
- Deterministic frame timing forwarded into the existing explicit `App` update model
- A renderer-agnostic platform layer so CPU and GPU backends do not each decode raw window events separately

Acceptance:
- Current examples launch through `winit`
- Resize and DPI changes update viewport size deterministically
- The platform loop is no longer tied to the software renderer implementation

---

## X2. Engine-owned input and text event model
Depends: X1, F2, H4  
Status: planned

Purpose:
- Normalize `winit` input into engine-owned events so interaction, text editing, and future widgets are not backend-specific

Should have been an extension:
- This would ideally have extended F2 and H4 because it moves interaction and text input out of ad hoc renderer-owned event handling

Support:
- Pointer movement, button state, wheel and touchpad scroll
- Keyboard key events kept distinct from text input events
- Modifier tracking
- Focus transitions
- Optional later support for IME composition, clipboard, and richer text input APIs

Acceptance:
- Hover, active, click, and scroll behavior continue to work after the platform switch
- Text entry can be expressed without tying it to a particular window backend
- Backends receive normalized engine events instead of raw `winit` event enums

---

## X3. Redraw scheduling and surface lifecycle
Depends: X1, G1, G3, Z1  
Status: planned

Purpose:
- Keep the current explicit invalidation model while making the frame lifecycle safe for swapchain or surface-backed renderers

Should have been an extension:
- This would ideally have extended G1 and G3 because it refines when frames run and what happens on resize, suspend, or surface loss

Support:
- `request_redraw` driven rendering for `OnInvalidation`
- Continuous rendering support for `EveryFrame`
- Surface resize and recreation hooks
- Minimized or suspended window handling
- Stable frame begin/end hooks shared by CPU and GPU backends

Acceptance:
- `OnInvalidation` apps do not spin unnecessarily
- Backend surface loss or resize does not force app-level architecture changes
- CPU and GPU backends share the same frame scheduling contract

---

# Epic Y - ECS runtime model

## Y1. ECS world for runtime UI state
Depends: A3, B1, G3  
Status: planned

Purpose:
- Move runtime UI state into an ECS world so style, layout, interaction, transitions, and rendering can operate over explicit component data instead of one monolithic recursive structure

Should have been an extension:
- This would ideally have extended B1, E1, and G3 because it changes how node state is stored and how invalidation flows through the engine

Support:
- Entities representing UI nodes
- Components for parent/child relations, text, attributes, classes, id, resolved style, layout box, interaction state, scroll state, and dirty flags
- Stable identity for subtree reuse and event routing
- A strict separation between authoring data, computed data, and render-extracted data

Non-goals:
- Do not force application authors to write ECS code directly
- Do not expose renderer-only resource handles inside core ECS components

Acceptance:
- A UI tree can be represented deterministically as ECS entities and components
- Interaction and invalidation data live in ECS-owned storage instead of being scattered across app and renderer layers
- ECS identity is stable enough to preserve the current subtree refresh guarantees

---

## Y2. DOM-to-ECS lowering and migration path
Depends: Y1, B2, J1  
Status: planned

Purpose:
- Keep `ui!` and `Node` as the authoring model at first, while lowering them into ECS so the architecture can evolve without a full public API reset

Should have been an extension:
- This would ideally have extended B2 and J1 because it changes what the macro and node system ultimately feed at runtime

Support:
- Lowering from `Node` trees into ECS entities
- Stable mapping from authored nodes to runtime entities
- Incremental respawn, patch, or rebuild policies based on invalidation class
- A path for future direct macro-to-prefab generation without requiring it on day one

Acceptance:
- Existing `ui!`-based examples can run through the ECS path
- The engine can rebuild or patch ECS entities without losing deterministic output
- Public authoring ergonomics do not need to change immediately

---

## Y3. ECS schedules for style, layout, interaction, and transitions
Depends: Y1, Y2, C4, D2, G3  
Status: planned

Purpose:
- Replace ad hoc cross-crate runtime passes with explicit ECS schedules so the work graph becomes measurable, reorderable, and backend-independent

Should have been an extension:
- This would ideally have extended C4, D2, F2, and G3 because it reorganizes the same work into explicit stages

Support:
- Structural update phase
- Selector matching and style resolution phase
- Layout sync phase
- Interaction and hit-testing phase
- Transition advancement phase
- Render extraction phase
- Dirty propagation based on `paint`, `layout`, and `structure` invalidation classes

Acceptance:
- Paint-only changes do not force layout systems to run
- System ordering is explicit and testable
- Runtime stats can attribute cost to named ECS phases instead of one combined rerender block

---

## Y4. Render extraction world and backend-facing scene data
Depends: Y3, E1, Z1  
Status: planned

Purpose:
- Extract renderer-facing data from ECS into a backend-neutral scene or draw list so CPU and GPU backends can share the same upstream runtime model

Should have been an extension:
- This would ideally have extended E1 because it changes how the renderer receives prepared scene data

Support:
- Extraction of paint primitives, clips, transforms, text runs, gradients, shadows, and scrollbar visuals
- Stable sort or batch keys for deterministic painter order
- A clean split between long-lived runtime world data and short-lived render-extracted data

Acceptance:
- Render backends consume extracted primitives instead of traversing the full runtime world directly
- CPU and GPU backends can share the same extracted scene definition
- Extracted ordering stays deterministic across frames

---

# Epic Z - Optional GPU renderer

## Z1. Backend abstraction above extracted scene data
Depends: X3, Y4, E2  
Status: planned

Purpose:
- Define a shared renderer contract so the software renderer and GPU renderer can coexist without duplicating app/runtime logic

Should have been an extension:
- This would ideally have extended E2 because it formalizes what a renderer backend is allowed to consume and own

Support:
- Common frame lifecycle
- Shared extracted scene input
- Backend-owned device resources and caches
- Capability flags for features that may temporarily differ between CPU and GPU implementations

Acceptance:
- The app runtime can select CPU or GPU backend without changing the authored UI
- Backends own their own caches and device state without leaking those types into `core`
- Missing GPU parity falls back clearly instead of silently changing behavior

---

## Z2. Baseline GPU backend
Depends: Z1, X3  
Status: planned

Purpose:
- Add a practical first GPU renderer for rectangles, borders, rounded corners, text, and clipping so the engine is no longer locked to CPU raster

Should have been an extension:
- This would ideally have extended E2 and H4 because it replaces the underlying raster path for common UI drawing

Support:
- A GPU backend built around window surfaces created from the `winit` platform layer
- Solid fills, borders, rounded rectangles, text, and scissored clipping
- Viewport resize and surface recreation
- Optional initial implementation on `wgpu` or an equivalent Rust-native graphics layer

Acceptance:
- A subset of existing examples renders correctly through the GPU path
- Resize, present, and lost-surface recovery work without app changes
- The baseline GPU renderer already matches the explicit frame model used by the CPU backend

---

## Z3. GPU scene batching, atlasing, and effect pipeline
Depends: Z2, H4, E3, S2, T2  
Status: planned

Purpose:
- Move beyond a naive GPU port and actually exploit the extracted scene model for batching and resource reuse

Should have been an extension:
- This would ideally have extended Z2 because it is the performance and feature-completeness pass over the first GPU renderer

Support:
- Instanced or batched draws for repeated primitives
- Glyph atlas or equivalent text resource management
- Clip stack or clip-mask strategy
- GPU-friendly transforms
- Controlled support for gradients, shadows, SVG paint, and backdrop-style effects

Acceptance:
- Common UI scenes do not degrade into one expensive draw call per node
- Text and effect resources are cached in backend-owned GPU structures
- The GPU path becomes materially faster than the CPU path on larger or more animated scenes

---

## Z4. CPU/GPU parity and backend fallback policy
Depends: Z2, Z3  
Status: planned

Purpose:
- Keep the optional GPU path honest by defining what "same engine" means when two raster backends coexist

Should have been an extension:
- This would ideally have extended E2 and Z2 because it is a release-quality contract, not a brand-new feature

Support:
- Golden tests or reference-image comparisons with tolerances where needed
- A documented fallback policy for unsupported GPU features
- The CPU backend kept as the deterministic reference path until parity is proven

Acceptance:
- Backend changes do not silently alter layout, hit testing, or scene ordering
- Feature gaps are reported explicitly
- The project can ship an optional GPU backend without turning the CPU path into dead code

---

# Epic AA - Compile-time baked UI assets and const evaluation

## AA1. Optional build-time CSS and UI compilation
Depends: B2, C1, C4, Y2  
Status: planned

Purpose:
- Allow selected UI and stylesheet inputs to be compiled ahead of time so startup does not have to parse them at runtime

Should have been an extension:
- This would ideally have extended B2 and C1 because it reuses the same authoring syntax but moves parsing into the build pipeline

Support:
- Procedural macro or `build.rs` driven compilation of selected HTML-like and CSS inputs
- Generation of Rust modules or binary blobs included in the final build
- Opt-in usage per example, screen, or crate feature
- Coexistence with the current runtime parser path for dynamic or dev-mode workflows

Acceptance:
- A demo can ship with no runtime CSS parsing for its baked assets
- Raw source strings do not need to exist at runtime for baked bundles
- The baked path stays optional instead of replacing dynamic authoring completely


human notes:

const fn compute<T: Trait>() was recently added to rust.
I beleave this whould be the correct way to do this so it also keeps variables intact and fucntions working.
---

## AA2. Const-friendly static node and style descriptors
Depends: AA1, Y1  
Status: planned

Purpose:
- Define a baked data format that can live in `.rodata` and be consumed without heap-driven parsing or runtime normalization

Should have been an extension:
- This would ideally have extended A3 and E1 because it introduces a second, more compact form of engine-owned scene input

Support:
- `StaticNodeDesc`, `StaticStyleDesc`, `StaticTextRun`, or equivalent baked descriptors
- Fixed-capacity arrays, slices, interned identifiers, compact enums, and compact style tables
- No requirement that baked data use `String`, `Vec`, or other heap-owned runtime forms
- Lowering from baked descriptors either into `Node`, directly into ECS, or into extracted render data where appropriate

Acceptance:
- Baked UI data can be embedded as static read-only data
- Startup does not require reparsing that baked data
- The baked representation is clearly more compact than keeping CSS source plus parsed runtime state at once

---

## AA3. Baked prefab spawning into ECS
Depends: AA2, Y2, Y3  
Status: planned

Purpose:
- Let precompiled UI instantiate directly into runtime ECS storage so startup and hot paths do not need a temporary DOM or stylesheet representation

Should have been an extension:
- This would ideally have extended Y2 because it is a lower-overhead source for the same ECS world

Support:
- Prefab-style spawning of entities and components from static descriptors
- Shared style tables and interned strings where that reduces duplication
- Stable prefab identity for event routing and subtree reuse
- Mixing baked and dynamic subtrees in the same app

Acceptance:
- A fully baked example can start with minimal allocations and no runtime CSS parse
- Baked and dynamic views can coexist in one runtime
- The ECS path can instantiate a precompiled subtree without first building an owned `Node` tree

---

## AA4. Const trait based finalization and its real limits
Depends: AA2  
Status: planned

Purpose:
- Use const-friendly traits for compile-time specialization once the parser or macro has already produced structured data

Should have been an extension:
- This would ideally have extended AA2 because it is the final compile-time optimization layer over already-baked descriptors

Support:
- Trait-provided constants or other const-safe trait behavior for themes, defaults, and layout hints
- `const fn` helpers that finalize already-structured descriptors into a more resolved static form
- Compile-time computation of selector-independent defaults, metrics, flags, and small lookup tables
- Explicit non-goals for raw CSS parsing, heap allocation, and arbitrary runtime-only logic inside `const fn`

Acceptance:
- The docs clearly explain that `const fn compute<T: Trait>()` is a compile-time finalizer, not a CSS parser
- A small baked descriptor can be specialized at compile time from a type-level config or theme
- The compile-time path degrades gracefully on toolchains where the newest const-trait surface is still incomplete

---

# Suggested implementation order (part 3)

1. X1 + X2 + X3  
2. Y1 + Y2  
3. Y3 + Y4  
4. AA1 + AA2  
5. AA3  
6. AA4  
7. Z1 + Z2  
8. Z3 + Z4  

---

# Outcome

If part 3 lands, the engine should gain:

- A modern platform shell through `winit`
- A runtime model that is easier to schedule, measure, and optimize through ECS
- An optional GPU backend without abandoning the current deterministic CPU path
- A realistic compile-time asset pipeline for selected static UI
- Lower startup work and lower runtime allocation pressure for baked screens

Important caveat:

- Compile-time baking can reduce runtime work and RAM, but it does not automatically reduce binary size
- Binary size may go down if raw CSS, parsing code, and dynamic runtime machinery can be feature-gated out
- Binary size may also go up if generated descriptors duplicate data or force too much monomorphized code
- Size and memory wins should be measured, not assumed

---

# Appendix - What `const fn compute<T: Trait>()` actually means

Mental model:

- `const fn` means the function can run in a compile-time context
- `T: Trait` means behavior is selected from a type that satisfies a trait bound
- Combined, it means "compute a constant from type-provided behavior"

In other words:

- It is compile-time specialization over structured data
- It is not a parser
- It is not a general replacement for runtime logic

Small example:

```rust
trait ThemeSpec {
    const PADDING: u16;
    const BG: u32;
}

struct Primary;

impl ThemeSpec for Primary {
    const PADDING: u16 = 12;
    const BG: u32 = 0x2563eb;
}

struct StaticNodeDesc {
    padding: u16,
    bg: u32,
}

const fn build_button<T: ThemeSpec>() -> StaticNodeDesc {
    StaticNodeDesc {
        padding: T::PADDING,
        bg: T::BG,
    }
}

const PRIMARY_BUTTON: StaticNodeDesc = build_button::<Primary>();
```

What happened there:

- The compiler picked `Primary`
- It read the trait-provided constants
- It evaluated `build_button::<Primary>()` during compilation
- The final baked value is stored directly in the binary

What this is good for in this engine:

- Finalizing already-generated static style descriptors
- Picking theme or mode specific constants without runtime branching
- Precomputing small layout hints, flags, and lookup tables
- Building static prefabs once parsing has already happened in a macro or build step

What this is not good for:

- Parsing raw CSS text in `const fn`
- Running `lightningcss` at const-eval time
- Allocating arbitrary dynamic node trees inside const evaluation
- Replacing the build step or macro that turns source text into structured data

Practical guidance:

- Use a build step or proc macro to parse CSS and markup into typed descriptors
- Use `const fn` and const-friendly traits only for the final normalization and specialization stage
- Prefer designs that work first with associated constants and compact static data
- Treat newer const-trait features as an accelerator, not the only possible implementation path
