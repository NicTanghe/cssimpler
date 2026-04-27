# CSSimpler Native Glass Specsheet

Roadmap for CSS-controlled native Windows glass regions.

This document extends the runtime, style, and rendering work described in:

- [specsheet.md](./specsheet.md)
- [specsheet_pt2.md](./specsheet_pt2.md)
- [specsheet_pt3.md](./specsheet_pt3.md)

The author-facing feature name is `glass`. Windows may use acrylic APIs internally, but CSS and public docs should not expose `acrylic` as the primary concept.

---

# Epic AC - CSS-native glass regions

## AC1. Glass style model

Depends: C2, C4, E1  
Status: proposed

Purpose:
- Let normal CSS request native platform glass on an element without coupling DOM, style, or render nodes to Windows-specific terminology.
- Carry the material request through the same resolved style and render tree path as other visual properties.

CSS API:

```css
.glass-panel {
    native-material: glass;
    glass-tint: rgba(84, 154, 205, 0.61);
}
```

Support:
- `native-material: none`
- `native-material: glass`
- `glass-tint: <color>`
- `glass-tint: var(...)` where variable resolution can produce a supported color

Core data:
- Add `NativeMaterial`.
- Add `NativeMaterial::None`.
- Add `NativeMaterial::Glass`.
- Add `native_material: NativeMaterial` to `VisualStyle`.
- Add `glass_tint: Option<Color>` to `VisualStyle`.

Rules:
- Default material is `none`.
- `glass-tint` does not by itself enable native glass.
- `native-material: glass` marks the element's border box as a native glass reveal region.
- Unsupported material names fail clearly.

Acceptance:
- Stylesheet parsing accepts `native-material: glass` and `glass-tint`.
- Resolved styles and render nodes carry the material and tint.
- Existing non-glass rendering remains unchanged when material is `none`.

---

## AC2. Scene extraction for glass regions

Depends: AC1, E1, G3, X3  
Status: proposed

Purpose:
- Make the runtime and renderer able to detect glass requirements from the extracted scene.
- Keep the native-window decision independent from CSS parsing internals.

Support:
- Extract glass region metadata from `RenderNode`.
- Preserve:
  - layout box
  - border radius
  - transform
  - overflow clip
  - element path
  - requested tint
  - stable sort key or paint order information

Scene-level queries:
- `scene_requires_native_glass`
- `scene_glass_regions`
- `scene_preferred_glass_tint`

Tint selection:
- The HWND can only have one native tint.
- If one glass region exists, use that region's tint for the native window tint.
- If multiple regions exist, pick a deterministic scene tint, then draw per-region tint overlays.
- First implementation may choose the first glass region in paint order.
- Later implementation may choose the largest region or an app/root-level default.

Acceptance:
- Extracted scenes report whether native glass is needed.
- Tests cover one region, multiple regions, and no regions.
- Tint selection is deterministic.

---

## AC3. Native Windows glass runtime

Depends: AC2, X1, X3  
Status: proposed

Purpose:
- Apply native Windows glass at the window composition layer when the scene requires it.
- Keep glass active even when the window or element is not hovered, focused, active, or otherwise targeted.

Implementation source:
- Port the proven behavior from the sibling `glassy` experiment.
- Use `SetWindowCompositionAttribute`.
- Use `WCA_ACCENT_POLICY`.
- Use `ACCENT_ENABLE_ACRYLICBLURBEHIND`.
- Pack the selected tint into `gradient_color`.

Runtime rules:
- Native glass is applied to the whole HWND.
- CSS regions decide where the renderer reveals that native backdrop.
- Focus changes, pointer changes, hover state, and active state must not clear glass.
- If scene tint changes, reapply the native composition attribute.
- If the scene no longer contains glass regions, clear or disable native glass where supported.

Window setup:
- The winit window must support transparency when native glass is enabled.
- Because window attributes are chosen at creation time, the runtime needs a policy for transparency:
  - simple v1: allow `WindowConfig` to request glass-capable transparent windows up front
  - smarter later: infer glass capability from app configuration before window creation

Acceptance:
- A glass-capable app applies Windows native glass when a scene contains `native-material: glass`.
- Glass remains active after focus changes.
- Runtime failures are logged and fall back without panicking.
- Non-Windows builds compile without Windows-only dependencies leaking into shared code.

---

## AC4. Renderer masking and reveal behavior

Depends: AC2, AC3, E3, P2, T1  
Status: proposed

Purpose:
- Since Windows applies glass to the whole window, the renderer must decide which pixels expose it.
- A CSS element should visually behave as if only that container has native glass.

Core constraint:
- Windows cannot apply acrylic/glass to an individual CSS node.
- CSSimpler simulates node-level native glass by making the window glass-backed and painting opaque pixels everywhere except glass reveal regions.

Support:
- Normal nodes remain opaque unless authored otherwise.
- Glass regions reveal the native window backdrop inside their border box.
- Border radius clips the reveal region.
- Overflow and ancestor clips are respected.
- Children draw normally above the reveal.
- Borders, shadows, and text draw normally.
- `glass-tint` draws as a per-region translucent overlay.

Ordering:
- Glass reveal happens before that element's children.
- Per-region tint is drawn over the revealed backdrop.
- Borders and foreground content draw over tint.

Transformed regions:
- v1 may support only untransformed or axis-aligned glass regions.
- Transform support should follow the same geometry path as transformed rounded rectangles.
- Unsupported transformed glass can fall back to a renderer-only tinted panel until transform-safe masking is implemented.

Acceptance:
- A marked panel reveals native glass while surrounding opaque UI hides it.
- Rounded glass panels do not reveal outside their radius.
- Children remain readable and draw above the glass surface.
- Multiple glass panels can coexist with deterministic tint overlays.

---

## AC5. Fallback rendering

Depends: AC1, AC4, T1, T2  
Status: proposed

Purpose:
- Keep glass-authored CSS usable on platforms or machines where native Windows glass is unavailable.

Fallback cases:
- Non-Windows platform.
- Windows native composition API unavailable.
- Window was not created as glass-capable.
- Runtime native glass call fails.
- Unsupported geometry for native reveal masking.

Fallback behavior:
- Draw a translucent tinted panel using `glass-tint`.
- Use renderer-owned backdrop blur if available.
- If backdrop blur is unavailable, draw a stable translucent color overlay.
- Keep layout, children, borders, and event behavior unchanged.

Acceptance:
- Glass CSS never makes the app unusable on unsupported platforms.
- Native failure logs a diagnostic and renders a fallback surface.
- Fallbacks are deterministic and covered by renderer tests.

---

## AC6. Authoring examples and diagnostics

Depends: AC1, AC2, AC3, AC4, AC5  
Status: proposed

Purpose:
- Make the feature easy to discover and hard to misuse.

Add examples:

```css
#app {
    width: 100%;
    height: 100%;
    background: rgb(12, 18, 28);
}

.glass-panel {
    native-material: glass;
    glass-tint: rgba(84, 154, 205, 0.61);
    border: 1px solid rgba(240, 248, 255, 0.28);
    border-radius: 14px;
}
```

Diagnostics:
- Unsupported `native-material` values name the invalid value.
- `glass-tint` values must use the same color diagnostics as other CSS color properties.
- Runtime logs should distinguish:
  - unsupported platform
  - missing Windows composition entry point
  - failed composition call
  - fallback due to window configuration

Acceptance:
- Example app visibly shows a glass container.
- Parser tests cover accepted and rejected values.
- Runtime diagnostics are clear enough to explain why fallback rendering was used.

---

# Suggested Implementation Order

1. Add `NativeMaterial` and `glass_tint` to core visual style.
2. Parse `native-material` and `glass-tint` in `style`.
3. Propagate material metadata through render nodes and extracted scene items.
4. Add scene queries for glass regions and preferred tint.
5. Add Windows-only native glass helper in the winit runtime.
6. Add `WindowConfig` support for glass-capable transparent windows.
7. Apply or clear native glass based on extracted scene state.
8. Implement first-pass glass reveal masking for axis-aligned rounded rectangles.
9. Draw per-region tint overlays.
10. Add fallback rendering for unsupported platforms and native failures.
11. Add an example app and regression tests.

---

# Open Questions

- Should the window be glass-capable by default, or only when `WindowConfig` opts in?
- Should the preferred native HWND tint be the first glass region, the largest glass region, or an explicit app-level tint?
- Should `native-material: glass` imply a default `glass-tint`, or should untinted glass be allowed?
- Should a custom `<glass>` tag exist as syntax sugar, or should CSS remain the only v1 API?
- Should transformed glass regions fall back initially, or block the feature until transform-safe masking exists?
- Should generated or baked UI assets preserve `native-material` as a first-class style descriptor?

---

# Final Model

Author-facing CSS:

```css
.panel {
    native-material: glass;
    glass-tint: rgba(84, 154, 205, 0.61);
}
```

Engine model:

- CSS marks regions.
- The scene reports glass requirements.
- The runtime enables native Windows glass on the HWND.
- The renderer reveals that native backdrop only inside marked regions.
- Per-region tint, children, borders, and shadows remain renderer-owned.

This gives authors container-level glass ergonomics while respecting the platform reality that native glass is window-level.
