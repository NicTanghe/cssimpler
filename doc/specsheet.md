# Rust UI Engine (HTML + CSS + Taffy)
Rust-native UI system with HTML-like syntax, real CSS parsing, explicit rendering (no signals)

---

# Epic A - Workspace & foundations (NON-NEGOTIABLE)

## A1. Workspace layout
Depends: -  
Status: done  

Cargo workspace:

- `core/` – DOM, style system, layout bridge (Taffy), event system  
- `renderer/` – rendering backend (wgpu / Iced adapter)  
- `macro/` – `ui!` procedural macro (HTML-like syntax → AST)  
- `app/` – application entrypoint (state + render loop)  
- `style/` – CSS parsing + selector system (using lightningcss)  
- `docs/` – specs and architecture notes  

Acceptance  
- `cargo run` launches app (via workspace default)  
- `core` has zero renderer dependencies  
- `renderer` does not know about parsing internals  
- `macro` outputs only `core` types  

---

## A2. Minimal runtime app
Depends: A1  
Status: done 

- Basic window (via renderer backend)  
- Single render loop  
- Hardcoded UI tree for testing  

Acceptance  
- Window opens and renders a rectangle + text  
- Render loop runs deterministically  

---

## A3. Core / Renderer contract
Depends: A1  
Status: done  

Core exposes:
- `Node` (DOM)
- `Style`
- `LayoutBox`
- `RenderNode`

Renderer consumes:
- `RenderNode` only

Acceptance  
- `core` builds without renderer  
- `renderer` builds using only public `core` types  

---

# Epic B - DOM & UI definition

## B1. Node system
Depends: A3  
Status: done  

```rust
enum Node {
    Element(ElementNode),
    Text(String),
}
```

Acceptance  
- Tree structure supports nesting  
- No renderer types inside nodes  

---

## B2. UI macro (`ui!`)
Depends: B1  
Status: done  

```rust
ui! {
    <div class="card">
        {state.count}
    </div>
}
```

Acceptance  
- Expands to `Node` tree  
- Supports `{}` Rust expressions  
- Supports attributes (`class`, `id`, events)  

---

## B3. Event binding
Depends: B1  
Status: done  

```rust
<button onclick={increment}>
```

Acceptance  
- Events map to Rust function pointers  
- No string-based handlers  
- No async required  

---

# Epic C - CSS system (real input, controlled execution)

## C1. CSS parsing
Depends: A1  
Status: done  

Use:
- lightningcss

Acceptance  
- Parse `.class`, `#id`, `tag` selectors  
- Extract declarations into intermediate form  

---

## C2. Style representation
Depends: C1  
Status: done  

```rust
struct Style {
    layout: LayoutStyle,
    visual: VisualStyle,
}
```

Acceptance  
- Clean separation: layout vs visual  
- No renderer types inside style  

---

## C3. Selector system
Depends: C1  
Status: done  

Supported:
- `.class`
- `#id`
- `tag`

Acceptance  
- Deterministic matching  
- No full CSS cascade complexity  

---

## C4. Style resolution
Depends: C2, C3  
Status: done  

```rust
fn resolve(node, stylesheet) -> Style
```

Acceptance  
- Styles applied per node  
- Predictable override rules  

---

# Epic D - Layout (Taffy integration)

## D1. Layout mapping
Depends: C2  
Status: done  

```rust
fn to_taffy(style: &LayoutStyle) -> taffy::Style
```

Acceptance  
- Supports flexbox + grid basics  
- Margin / padding respected  

---

## D2. Layout computation
Depends: D1  
Status: done  

```rust
taffy.compute_layout(root)
```

Acceptance  
- Each node gets absolute layout box  
- Stable results across frames  

---

## D3. Layout output model
Depends: D2  
Status: done  

```rust
struct LayoutBox {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}
```

Acceptance  
- All renderable nodes have layout  

---

# Epic E - Rendering system

## E1. Render tree construction
Depends: D2, C4  
Status: done  

```rust
struct RenderNode {
    layout: LayoutBox,
    style: VisualStyle,
    children: Vec<RenderNode>,
}
```

Acceptance  
- Fully detached from DOM  
- Ready for renderer consumption  

---

## E2. Renderer backend
Depends: E1  
Status: done  

- Rectangle rendering  
- Text rendering  
- Color + background  

Acceptance  
- Basic UI visible on screen  

---

## E3. Visual features

### Shadows
Depends: E2  
Status: done  

- `box-shadow` support  
- Blur + offset  

Acceptance  
- Shadow renders behind element  

---

### Borders & radius
Depends: E2  
Status: done  

Acceptance  
- Rounded corners respected  

---

### Clipping / overflow
Depends: E2  
Status: done  

Acceptance  
- Children clipped to parent bounds  

---

# Epic H - Typography & font system

## H1. Typography model
Depends: A3, C2  
Status: planned  

Purpose:
- Represent text styling as first-class engine data instead of assuming one built-in bitmap font

Add:
- `TextStyle` or equivalent typography data on resolved styles / render nodes
- `font-family`
- `font-size`
- `font-weight`
- `font-style`
- `line-height`

Where this lands:
- `core/` owns the font-related data carried by `Style` and `RenderNode`
- `macro/` does not need new syntax

Acceptance
- A render node can describe which font family stack it wants
- Text styling is detached from any specific renderer implementation

---

## H2. Font source resolution
Depends: H1, E2  
Status: planned  

Purpose:
- Resolve CSS family requests to actual fonts from either the host system or explicit user-provided files

Support:
- System font lookup by family name
- Arbitrary font registration from file path or embedded bytes
- Family fallback stacks

Where this lands:
- `renderer/` owns the platform-facing font database / loader
- `app/` or a small shared text module may expose registration APIs for custom fonts

Acceptance
- `font-family: "Segoe UI", Arial, sans-serif` can resolve against installed fonts
- A demo can register and use a `.ttf` or `.otf` font that is not installed system-wide
- Missing families fall back deterministically

---

## H3. Text measurement for layout
Depends: H1, H2, D2  
Status: planned  

Purpose:
- Replace fixed glyph constants with real font metrics so layout matches the font actually drawn

Where this lands:
- `style/` parses typography declarations and resolves them into style data
- `style/` replaces `GLYPH_WIDTH`, `GLYPH_HEIGHT`, and text wrapping heuristics with font-aware measurement
- `LeafMeasureContext` carries both text content and resolved text style

Acceptance
- Text width and height come from font metrics, not hardcoded character cells
- Wrapping changes when `font-size`, `line-height`, or family changes
- Layout and paint agree on line breaks

---

## H4. Text shaping and rasterization
Depends: H2, H3, E2  
Status: planned  

Purpose:
- Draw the resolved font instead of always using the baked-in `font8x8` glyph set

Where this lands:
- `renderer/` replaces the current `draw_text` path with a font-backed text renderer
- Renderer keeps glyph caching / atlasing concerns local

Acceptance
- Rendered text visually changes when the chosen font family changes
- Font size affects the final pixels on screen
- Unicode coverage is no longer limited to the bitmap font table

---

## H5. Arbitrary font demos and validation
Depends: H2, H4  
Status: planned  

Purpose:
- Prove both system fonts and custom fonts work in real examples

Acceptance
- One example uses a Windows system font such as `Segoe UI`
- One example loads a project-local font asset
- Screenshots or golden tests confirm that changing the font changes both layout and paint

---

# Epic F - Event system & interaction

## F1. Hit testing
Depends: D3  
Status: done  

- Map mouse → layout boxes  

Acceptance  
- Click selects correct node  

---

## F2. Event dispatch
Depends: F1, B3  
Status: done  

Flow:
1. hit test  
2. call handler  
3. mutate state  
4. rerender  

Acceptance  
- Events trigger Rust logic directly  

---

# Epic G - Render loop (explicit, no signals)

## G1. Application model
Depends: A2  
Status: done  

```rust
fn update(state: &mut State)
fn render(state: &State) -> Node
```

Acceptance  
- No signals  
- No implicit updates  

---

## G2. Full rerender
Depends: G1  
Status: done  

- Rebuild UI tree every frame or on demand  

Acceptance  
- Deterministic output  

---

## G3. Invalidation & partial refresh
Depends: G2  
Status: done  

Purpose:
- Rerender on interaction, state mutation, or external data change
- Refresh only the smallest safe affected subtree
- Fall back to full rerender when the impact is unclear

Triggers:
- Hover / pointer enter / pointer leave
- Click / input / focus changes
- State updates
- Data updates from outside the UI tree

Invalidation classes:
- `paint` - visual-only changes such as color, background, border, shadow
- `layout` - changes that can affect size or position
- `structure` - insert / remove / reorder nodes or change matching attributes / classes

Rules:
- Every change marks nodes as dirty
- The engine computes the smallest safe invalidation root
- `paint` changes should avoid layout work when possible
- `layout` changes may recompute parent and sibling layout as needed
- `structure` changes may expand to a larger subtree or full rerender
- Correctness wins over partial refresh

Acceptance  
- Hover-only style changes repaint without rebuilding unrelated UI  
- Local state changes rerender only the affected subtree when safe  
- Layout-affecting changes update all impacted layout boxes  
- Engine can always fall back to full rerender deterministically  

---

## G4. Optional optimization (later)
Depends: G2  
Status: done  

- diffing  
- partial updates  

Acceptance  
- Performance improves without changing API  

---

# Epic I - Scrollbars (engine-owned, CSS-styled)

## I1. Scrollbar style model
Depends: C2, C4, E3  
Status: done  

Purpose:
- Add first-class scrollbar support without ever falling back to platform-default visuals
- Make scrollbar appearance an explicit CSS concern instead of an implicit renderer detail

Support:
- `overflow`
- `overflow-x`
- `overflow-y`
- `scrollbar-width`
- `scrollbar-color`
- Dedicated scrollbar parts for track, thumb, and corner
- Familiar CSS entry points for scrollbar styling, even if the engine keeps the supported subset intentionally small

File ownership note:
- Scrollbar work should live in dedicated `scrollbar.rs` files, not be implemented inline in crate `lib.rs` files
- Expected ownership:
- `core/src/scrollbar.rs` for scrollbar state and shared data
- `style/src/visual/scrollbar.rs` for parsed / resolved visual scrollbar styles
- `renderer/src/scrollbar.rs` for painting and interaction-facing render data
- Crate `lib.rs` files should only wire modules / exports

Acceptance
- A scrollable container never shows an unstyled default scrollbar
- Resolved CSS fully determines the scrollbar's visible look
- Scrollbar styling data stays separate from top-level crate glue

---

## I2. Scroll container model and layout reservation
Depends: I1, D2, D3, E3  
Status: done  

Purpose:
- Model scrollable content, viewport size, scroll offsets, and scrollbar gutter reservation

Where this lands:
- `core/` owns scroll state, viewport metrics, content metrics, and axis visibility decisions
- Layout integration computes when scrollbars appear and how much space they reserve

Acceptance
- Scrollbars appear only when content overflows or CSS explicitly requests them
- Viewport, content size, and scroll offset are deterministic engine data
- Gutter reservation is stable and does not depend on host platform behavior

---

## I3. Scrollbar rendering
Depends: I1, I2, E2  
Status: done  

Purpose:
- Render scrollbar track, thumb, and corner as engine-owned draw primitives

Support:
- Background color
- Border
- Radius
- Hover / active visual states
- Thumb sizing based on visible content ratio

Acceptance
- Scrollbar track and thumb render with CSS-resolved visuals
- Thumb size reflects viewport-to-content ratio
- Scrollbar painting respects clipping and element bounds

---

## I4. Scroll input and interaction
Depends: I2, I3, F2  
Status: done  

Purpose:
- Allow scrollbars to behave like real interactive controls instead of decorative overlays

Support:
- Mouse wheel / trackpad scrolling
- Thumb dragging
- Optional track click paging
- Hover / pressed state transitions

Acceptance
- Pointer wheel input updates scroll offset on the correct scroll container
- Dragging the thumb updates the scroll position deterministically
- Interactive state changes can restyle the scrollbar without falling back to platform widgets

---

# Constraints (explicit)

- Rust-only (no JS, no webview)  
- No signals / reactive systems  
- Explicit render loop  
- CSS is parsed but not fully browser-accurate  
- Layout handled by Taffy  
- Rendering fully owned by project  
- Clear separation:
  - core ≠ renderer ≠ macro ≠ style  

---

# Suggested implementation order (fastest to “feels real”)

1. A1 + A2 + A3  
2. B1 + B2  
3. C1 (basic CSS parsing)  
4. C2 + C4 (apply styles)  
5. D1 + D2 (layout working)  
6. E2 (basic rendering)  
7. H1 + H2 + H3 + H4 (real typography, system fonts, arbitrary fonts)  
8. F1 + F2 (click handling)  
9. E3 (shadows, visuals)  
10. I1 + I2 + I3 + I4 (engine-owned CSS scrollbars)  
11. G1 + G2 (full loop)  
12. G3  
13. G4  
Polish: selectors, styling depth, performance  

---

# Final note

This architecture gives you:

- Rust-native UI  
- HTML-like ergonomics  
- CSS input without browser chaos  
- Full control over rendering  

You’re not wrapping a browser — you’re building a deterministic UI engine with familiar syntax.
