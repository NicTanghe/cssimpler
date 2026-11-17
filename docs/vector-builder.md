# Fluent builder for native vector paths

`VectorPathBuilder` (`cssimpler::core::VectorPathBuilder`) is an ergonomic,
validated front end for the same [`VectorPath`](./native-vector-paths.md) used by
`Node::vector`, SVG export, and the renderer. It is an authoring convenience, not
a new geometry format and not a rasterization speed claim.

## Construct a path

```rust
use cssimpler::core::{Color, Node, VectorPath, VectorPoint as P, VectorScene, VectorViewBox};

let path = VectorPath::builder()
    .reserve(5) // reserve the command buffer up front
    .move_to(P::new(40.0, 240.0))
    .cubic_to(P::new(20.0, 30.0), P::new(260.0, 10.0), P::new(350.0, 100.0))
    .quad_to(P::new(390.0, 260.0), P::new(250.0, 250.0))
    .line_to(P::new(40.0, 240.0))
    .close()
    .fill(Color::rgb(200, 220, 240))
    .stroke(Color::rgba(30, 60, 90, 255))
    .stroke_width(2.0)
    .build()?;

let scene = VectorScene::new(
    VectorViewBox::new(0.0, 0.0, 400.0, 300.0),
    vec![path],
)?;
let element: Node = Node::vector(scene).with_id("artwork").into();
```

Builder methods:

| Method | Effect |
| --- | --- |
| `VectorPath::builder()` / `VectorPathBuilder::new()` | Empty path with default paint. |
| `VectorPathBuilder::with_capacity(n)` | Empty path whose buffer reserves `n` commands. |
| `reserve(additional)` | Reserves room for more commands inside a fluent chain (`Vec::reserve` semantics). |
| `move_to(point)` | Starts a new contour. |
| `line_to(point)` | Straight segment. |
| `quad_to(control, end)` | One-control quadratic curve. |
| `cubic_to(control1, control2, end)` | Two-control cubic curve. |
| `close()` | Closes the current contour. |
| `fill(color)` / `no_fill()` | Enables a solid fill / disables the fill. |
| `stroke(color)` / `no_stroke()` | Enables a solid stroke / disables the stroke. |
| `stroke_width(width)` | Sets the stroke width. |
| `with_paint(VectorPaint)` | Reuses a complete paint value, e.g. from another path. |
| `build()` | Validates and returns `Result<VectorPath, VectorError>`. |

Defaults match `VectorPaint::default` and therefore direct construction: black
fill, no stroke, and stroke width `1.0`. `no_fill` and `no_stroke` only change the
paint they name. An empty path is valid and paints nothing.

## Validation

Commands are buffered as they are chained; nothing is validated until `build()`.
`build()` calls `VectorPath::new`, so a built path is exactly the value direct
construction would produce and is accepted/rejected by the same rules:

- every coordinate and control point must be finite;
- the path must start with `move_to`; later `move_to` calls start additional
  contours;
- the stroke width must be finite and nonnegative.

As in direct construction and SVG, a command immediately after `close()` is
allowed and continues from the closed contour's initial point; the builder does
not add a stricter ordering rule.

Malformed input returns a `VectorError` from `build()` with the existing messages
(`"path coordinates must be finite"`, `"a contour must start with MoveTo"`,
`"stroke width must be finite and nonnegative"`); chaining and `build()` do not
panic. Validation errors are reported against the whole path, not the individual
call, because the builder deliberately allows partial contours while authoring.

The builder never parses or serializes SVG, never flattens curves, and never
rounds coordinates. `to_svg_path_data()`, `VectorScene::to_svg()`, rendering,
bounds, and invalidation all operate on the resulting `VectorPath` exactly as
they do for directly constructed commands.

## Construction and allocation cost

- The builder holds one `Vec<PathCommand>` plus one `VectorPaint`. There is no
  second geometry representation and no retained copy of the path.
- Chained methods take `self` and return `Self`, moving the small builder value
  (Vec header + paint) between calls. They push into the existing buffer; they do
  not clone the accumulated commands.
- `with_capacity(n)` (or `reserve(n)` in a chain) preallocates the buffer.
  Without it the `Vec` grows amortized as with any `Vec` push.
- `build()` validates in one linear pass and moves the buffer into `VectorPath`.
  Validation performs no allocation; the command storage is the same `Vec` that
  direct construction would pass to `VectorPath::new`.
- No raster, layer-cache, DOM, or global allocation is added by the builder.

Memory is therefore approximately the same as direct `Vec<PathCommand>`
construction (plus the `VectorPaint` value that direct construction also needs).
Any spare buffer capacity from over-reserving remains with the `Vec`, just as it
would for direct construction. This is not a claim of faster rasterization or
painting; the benefit is readable authoring with the same validated output.

## When to construct commands directly

Prefer `VectorPath::new(Vec<PathCommand>, VectorPaint)` when:

- geometry already exists as a `Vec<PathCommand>` (for example loaded, generated,
  transformed, or spliced from another path);
- a function maps one input to one command and command order is data-driven;
- you want to append, sort, or otherwise manipulate the raw command list before
  validation;
- the path is assembled once at a single call site where a fluent chain would add
  no clarity.

The builder is most useful for handwritten artwork and fixed UI shapes where a
linear chain reads better than nested enum literals. Both routes return the same
`VectorPath` type, so they can be mixed in one `VectorScene`.

## Example

```sh
cargo run -j 2 --example vector_builder
cargo run -j 2 --example vector_builder -- --export work/curves.svg
cargo run -j 2 --example vector_builder -- --show
```

The example builds one closed filled contour and one open stroke-only contour,
renders them through `Node::vector`, and, by default, exports the scene's SVG
without opening a window.
