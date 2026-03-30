# Rust UI Engine (HTML + CSS + Taffy)
Rust-native UI system with HTML-like syntax, real CSS parsing, explicit rendering (no signals)

---

# Epic A - Workspace & foundations (NON-NEGOTIABLE)

## A1. Workspace layout
Depends: -  
Status: todo  

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
Status: todo  

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
Status: todo  

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
Status: todo  

- Rectangle rendering  
- Text rendering  
- Color + background  

Acceptance  
- Basic UI visible on screen  

---

## E3. Visual features

### Shadows
Depends: E2  
Status: todo  

- `box-shadow` support  
- Blur + offset  

Acceptance  
- Shadow renders behind element  

---

### Borders & radius
Depends: E2  
Status: todo  

Acceptance  
- Rounded corners respected  

---

### Clipping / overflow
Depends: E2  
Status: todo  

Acceptance  
- Children clipped to parent bounds  

---

# Epic F - Event system & interaction

## F1. Hit testing
Depends: D3  
Status: todo  

- Map mouse → layout boxes  

Acceptance  
- Click selects correct node  

---

## F2. Event dispatch
Depends: F1, B3  
Status: todo  

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
Status: todo  

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
Status: todo  

- Rebuild UI tree every frame or on demand  

Acceptance  
- Deterministic output  

---

## G3. Invalidation & partial refresh
Depends: G2  
Status: todo  

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
Status: todo  

- diffing  
- partial updates  

Acceptance  
- Performance improves without changing API  

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
7. F1 + F2 (click handling)  
8. E3 (shadows, visuals)  
9. G1 + G2 (full loop)  
10. G3 
11. g4 
Polish: selectors, styling depth, performance  

---

# Final note

This architecture gives you:

- Rust-native UI  
- HTML-like ergonomics  
- CSS input without browser chaos  
- Full control over rendering  

You’re not wrapping a browser — you’re building a deterministic UI engine with familiar syntax.
