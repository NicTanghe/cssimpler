# cssimpler upstream text layout fix specification

Date: 2026-06-23  
Upstream repository: `C:\Users\duplico\dev\cssimpler`  
Inspected upstream revision: `c6a2d25`

## Objective

Make ordinary CSS text controls behave like browser controls:

- text in flex buttons must honor `justify-content` and `align-items`;
- a word that fit during intrinsic measurement must not wrap after pixel rounding;
- final text layout dimensions must agree with the dimensions used by Taffy;
- words must not be split at arbitrary characters under default CSS wrapping;
- glyphs must be vertically centered inside their CSS line box;
- `text-align` and `white-space` must be represented rather than silently ignored.

The PrintCountPay application must no longer require label wrapper elements or defensive
`flex-shrink: 0` rules just to display short labels such as `Recording`, `REC`, and `Online`.

## Confirmed failure

The defect is reproducible in the computed render tree, not only in the screenshot.

For the `Recording` tab:

```text
Natural one-line text width: 43.140968px
Final button border box:      61px
Horizontal content inset:     9px + 9px
Final text content width:     43px
Taffy text-node height:       12px
Rebuilt text-layout height:   24px
Rebuilt lines:                ["Recordin", "g"]
```

The sequence is:

1. Taffy measures `Recording` as one line at approximately `43.140968px`.
2. Taffy's default final-layout rounding reduces the text content width to `43px`.
3. `render_node_from_layout` calls `text_layout_from_measure_context` using the rounded
   width.
4. `cached_text_layout` rebuilds the text as two lines.
5. Taffy is not rerun, so the node remains `12px` high while its prepared text is `24px`
   high.
6. The second line paints outside the control and overlaps the content below it.

This explains the isolated final letters visible in the screenshot.

There is also a separate alignment defect. An element containing only direct text is
collapsed into a Taffy leaf. Consequently:

```html
<button style="display:flex; justify-content:center; align-items:center">
  Recording
</button>
```

does not establish a flex formatting context in cssimpler. Its `justify-content` and
`align-items` declarations cannot affect the direct text.

## Root causes

### 1. Taffy rounding changes the wrapping width after measurement

Location: `style/src/render_tree.rs`

`layout_resolved_render_tree` creates `TaffyTree::new()`. Taffy 0.7 enables final-layout
rounding by default. cssimpler then uses the rounded box width to rebuild the prepared
text layout.

Relevant functions:

- `layout_resolved_render_tree`
- `render_node_from_layout`
- `text_layout_from_measure_context`
- `text_layout_wrap_width`
- `cached_text_layout`

The renderer already accepts floating-point `LayoutBox` coordinates and performs its own
pixel-bound calculations, so forcing integer layout at the Taffy layer is unnecessary and
destroys text-layout consistency.

### 2. Direct-text flex and grid elements are incorrectly converted to leaves

Location: `style/src/render_tree.rs`

`resolve_element_tree` returns a text-bearing `ResolvedElement` with no children whenever
an element has no element children. `build_layout_tree` then calls
`new_leaf_with_context`.

This optimization is invalid when the element establishes a flex or grid formatting
context. Direct text in such an element must become an anonymous flex/grid item, while the
element itself remains a container.

Relevant functions:

- `resolve_element_tree`
- `flush_text_child`
- `text_child_style`
- `build_layout_tree`

### 3. Default wrapping incorrectly splits long words

Location: `core/src/fonts.rs`

`wrap_source_line` unconditionally calls `wrap_long_word` when a word exceeds the
available width. Browser default behavior for:

```css
white-space: normal;
overflow-wrap: normal;
word-break: normal;
```

does not split an ordinary word at arbitrary characters. The unbroken word overflows its
line box. Character-level splitting is only valid when explicitly requested, for example
with `overflow-wrap: anywhere` or `word-break: break-all`.

Relevant functions:

- `layout_text_block`
- `wrap_text_lines`
- `wrap_source_line`
- `wrap_long_word`

### 4. Glyphs are top-aligned inside the line box

Location: `renderer/src/fonts.rs`

`positioned_glyphs` uses:

```rust
baseline_y = start_y + ascent + line_index * line_height
```

Any extra leading from `line-height` is placed entirely below the glyph. Bitmap text is
also drawn from the line box's top edge. This makes labels appear too high even when the
text node itself is correctly centered by flex layout.

Relevant functions:

- `positioned_glyphs`
- `rasterize_bitmap_text`

### 5. Important text properties are unsupported

No implementation was found for:

- `text-align`
- `white-space`
- `overflow-wrap`
- `word-break`

The PrintCountPay stylesheet currently contains `text-align: center`, but cssimpler does
not represent or paint this property.

Likely parser/style location:

- `style/src/fonts.rs`
- `core/src/fonts.rs`
- text style cache key in `renderer/src/fonts.rs`

## Required implementation

### P0. Keep layout and text measurement in the same coordinate system

Immediately disable Taffy's final integer rounding when constructing the layout tree:

```rust
let mut taffy = TaffyTree::<LeafMeasureContext>::new();
taffy.disable_rounding();
```

Use floating-point layouts through style extraction and rendering. Pixel snapping, where
needed for crisp one-pixel borders, should occur in the renderer for that visual primitive;
it must not alter text wrapping constraints.

Do not solve this by adding an arbitrary wrapping epsilon. An epsilon can hide this sample
while preserving the invalid state where the prepared text height differs from the height
Taffy measured.

Add a debug assertion for auto-sized text leaves after extraction:

```text
prepared wrap width equals the content width used during final Taffy measurement
```

If rounded layout remains configurable in the future, store the measured/unrounded text
content width separately and never rebuild wrapping from a different rounded width without
rerunning layout.

### P0. Preserve flex/grid containers with direct text

Change the direct-text fast path in `resolve_element_tree`.

The element may be collapsed to one measured text leaf only when it does not need to
establish a child formatting context. At minimum, never collapse elements whose resolved
display is `Flex` or `Grid`.

For flex/grid elements:

1. Keep the original element as a container.
2. Convert each contiguous direct-text run into an anonymous child using the existing
   `flush_text_child` path.
3. Ignore whitespace-only anonymous text runs in flex/grid containers.
4. Keep events, background, border, padding, dimensions, and interaction state on the
   original element.
5. Inherit text and foreground styles onto the anonymous child.

This must work without requiring:

```html
<button><span>Recording</span></button>
```

### P0. Implement browser-correct default word wrapping

Extend `TextStyle` with explicit wrapping behavior. A suitable minimal model is:

```rust
enum WhiteSpace {
    Normal,
    NoWrap,
    Pre,
    PreWrap,
}

enum OverflowWrap {
    Normal,
    Anywhere,
}

enum WordBreak {
    Normal,
    BreakAll,
}
```

Defaults must be `Normal`.

Under the defaults:

- collapse ordinary whitespace as currently done;
- wrap at valid whitespace opportunities;
- preserve explicit paragraph/newline boundaries as currently supported;
- never call `wrap_long_word` for an ordinary unbroken word;
- allow the line's measured width to exceed `max_width`.

Only split a word when the computed style requests `Anywhere` or `BreakAll`. Prefer Unicode
grapheme boundaries over Rust `char` boundaries so combining characters are not separated.

`white-space: nowrap` must suppress automatic wrapping. `pre` and `pre-wrap` may be added in
the same change or tracked as a clearly documented follow-up, but parsing `nowrap` is
required for controls.

### P0. Center real and bitmap glyphs within each line box

For resolved fonts, derive a baseline offset using half-leading:

```rust
let glyph_height = scaled_font.ascent() - scaled_font.descent();
let leading = line_height - glyph_height;
let baseline_offset = leading * 0.5 + scaled_font.ascent();
let baseline_y = start_y + baseline_offset + line_index as f32 * line_height;
```

Negative leading should be handled consistently rather than silently moved below the
glyph. Apply equivalent centering to the bitmap fallback using its actual bitmap-cell
height.

Prefer storing the baseline offset in `TextLayout` or a shared metrics structure so real
and bitmap rendering cannot diverge.

### P1. Implement `text-align`

Add inherited `text-align` state with at least:

- `start`
- `left`
- `center`
- `right`
- `end`

Until writing direction is implemented, `start` may map to left and `end` to right.

When painting each line, calculate its starting X coordinate from `line.width` and the
content-box width:

```text
left/start: layout.x
center:     layout.x + (layout.width - line.width) / 2
right/end:  layout.x + layout.width - line.width
```

Apply the same logic to real-font and bitmap-font paths. Include alignment in any cache key
whose output depends on glyph positions.

## Regression tests

Tests must use a bundled deterministic font where exact metrics matter.

### Test 1: direct text remains a flex child

Input:

```html
<div id="root">
  <button id="button">Recording</button>
</div>
```

```css
#button {
    display: flex;
    width: 120px;
    height: 32px;
    padding: 4px 8px;
    justify-content: center;
    align-items: center;
    font-size: 12px;
    line-height: 1;
}
```

Assertions:

- the button render/layout node is a container;
- it has one anonymous text child;
- text has one line;
- text center X equals button content-box center X within `0.01px`;
- text center Y equals button content-box center Y within `0.01px`.

### Test 2: fractional intrinsic width does not cause post-layout rewrap

Use a word with a fractional measured width, or specifically the deterministic-font
equivalent of `Recording`.

Assertions:

- intrinsic measurement produces one line;
- final prepared layout produces one line;
- final prepared layout height equals the height used by Taffy;
- no child width is reduced merely by final layout rounding.

This test must fail if `TaffyTree` default rounding is re-enabled.

### Test 3: default wrapping does not split a word

Measure a word in a box slightly narrower than the word.

Assertions:

- one line is returned;
- line width may exceed the requested wrap width;
- the text content is unchanged.

### Test 4: explicit anywhere wrapping splits a word

Use the same input with `overflow-wrap: anywhere`.

Assertions:

- more than one line is returned;
- concatenating line contents reproduces the original word;
- no Unicode grapheme cluster is split.

### Test 5: normal wrapping still wraps at spaces

Measure `alpha beta` at a width that fits each word but not both.

Assertions:

- two lines are returned;
- lines are `alpha` and `beta`.

### Test 6: line-height uses half-leading

Use a font size smaller than an explicit line height.

Assertions:

- top and bottom leading are equal within rasterization tolerance;
- real-font and bitmap fallback use equivalent vertical placement;
- increasing line-height does not pin glyphs to the top.

### Test 7: text-align center

Paint a short line into a fixed-width text box.

Assertions:

- left and right free space are equal within one raster pixel;
- cache reuse does not return a left-aligned mask for centered text.

### Test 8: cached-layout rebuild remains consistent

Build a scene, rebuild it through `rebuild_resolved_render_tree_with_cached_layout`, and
compare:

- line count;
- prepared wrap width;
- prepared layout width and height;
- glyph placement.

The cached and uncached paths must be equivalent.

## Acceptance criteria in PrintCountPay

After updating the cssimpler dependency and removing the temporary application workarounds:

- `Recording` remains on one line in the tab.
- `REC` remains on one line inside the circular badge.
- `Online` remains on one line.
- button text is centered horizontally and vertically with direct text children.
- no text from a tab or badge paints outside its control.
- `Advanced: Off` either remains one line when intrinsic space is available or wraps only
  at the space when genuinely constrained.
- reducing application padding changes spacing only; it does not change whether a short
  word is split.
- existing cssimpler renderer, style, and integration tests pass.

## Recommended implementation order

1. Disable Taffy final-layout rounding and add the fractional-width regression test.
2. Correct default word-breaking and add wrapping tests.
3. preserve flex/grid containers with anonymous direct-text children.
4. Correct line-box baseline placement for real and bitmap fonts.
5. Add `white-space: nowrap` and `text-align`.
6. Run the PrintCountPay cssimpler UI and remove its label-wrapper/flex-shrink workarounds
   only after upstream tests and visual verification pass.

## Non-goals

- Full Unicode line-breaking conformance in the first patch.
- Full bidirectional writing-mode support.
- Browser-complete inline formatting contexts.
- Reworking application colors, dimensions, or business logic.

The patch should establish correct invariants and browser-compatible defaults without
attempting the entire CSS Text specification.

## Suggested handoff prompt

```text
Implement the upstream cssimpler text-layout fixes described in
C:\Users\duplico\dev\PrintCountPay\PrintCountPay\docs\CSSIMPLER_TEXT_LAYOUT_UPSTREAM_FIX_SPEC.md
against C:\Users\duplico\dev\cssimpler.

Treat every P0 item and its regression tests as required. Do not modify PrintCountPay while
implementing the upstream patch. After cssimpler tests pass, report the changed files,
behavioral decisions, and any P1 work left incomplete.
```
