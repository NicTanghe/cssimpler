## 2026-11-17 20:40 UTC - issue-21 agent

Issue: #21 "Add a fluent builder for native vector paths"
Branch: `agent/issue-21-vector-builder`
Base: `origin/main` @ `6d49b79` (unchanged when work started)
Feature commit: `045dc52 Add fluent builder for native vector paths`

### What changed

- `core/src/vector_builder.rs` (new): `VectorPathBuilder` plus
  `VectorPath::builder()`. One `Vec<PathCommand>` + one `VectorPaint`; consuming
  (`self -> Self`) chain methods; `with_capacity` / `reserve`; `move_to`,
  `line_to`, `quad_to`, `cubic_to`, `close`; `fill`, `no_fill`, `stroke`,
  `no_stroke`, `stroke_width`, `with_paint`; `build()` delegates to
  `VectorPath::new`, so validation and the resulting value are identical to direct
  construction. No SVG parsing/serialization or curve flattening.
- `core/src/lib.rs`: 2 added lines (`pub mod vector_builder;` and re-export).
  `core/src/vector.rs` was deliberately not touched to stay rebase-friendly with
  #17 (ownership design).
- `tests/issue_21_vector_builder.rs` (new): 9 integration tests.
- `examples/vector_builder.rs` (new): builder artwork rendered through
  `Node::vector`; headless by default (writes SVG), `--show` opens the app.
- `docs/vector-builder.md` (new) + one link line in `docs/README.md`: API table,
  validation, cost/allocation note, when direct `VectorPath::new` is preferable.

### Acceptance criteria evidence

| AC | Status | Evidence |
| --- | --- | --- |
| MoveTo/LineTo/QuadTo/CubicTo/Close + multiple contours | done | `builder_covers_every_command_and_multiple_contours`, `open_contours_keep_order_and_f32_precision` |
| fill/stroke colors+alpha, stroke width, disable fill/stroke, preserve defaults | done | `paint_settings_are_applied_and_can_be_disabled`, `defaults_match_direct_construction`; default = `VectorPaint::default` (black fill, no stroke, width 1.0) |
| Validate finite coords, contour ordering, paint; useful errors, no panics | done | `invalid_input_returns_errors_instead_of_panicking`; errors are the existing `VectorError` messages, returned from `build()` |
| Preserve order/control points/f32; no SVG parse/serialize/flatten | done | `open_contours_keep_order_and_f32_precision`, `built_path_is_identical_to_directly_authored_path_and_svg`; builder only pushes `PathCommand`s |
| Produce existing `VectorPath` for rendering/bounds/invalidation/SVG | done | `built_path_is_identical_to_directly_authored_path_and_svg` compares `VectorPath`, path data, full scene SVG, and `into_render_scene()` incl. control-point-hull bounds; `node_vector_accepts_a_builder_scene` |
| One command buffer + optional capacity, no per-call copy | done | `command_buffer_is_reserved_and_moved_without_copying` asserts capacity and `as_ptr()` are unchanged across 8 chained calls and after `build()` |
| Examples for `Node::vector`; document when direct construction is preferable | done | `examples/vector_builder.rs` (`view` uses `Node::vector`), `docs/vector-builder.md` section "When to construct commands directly" |
| Tests for contours/invalid input/paint/direct+SVG equivalence | done | 9 tests listed above |
| Check construction/allocation costs (usability, not speed claim) | done (analytic + structural test; no timings) | One `Vec` + one paint; no second representation; `build` validation is O(n) and allocates nothing; pointer/capacity test proves no per-call copy or reallocation; memory ~ direct `Vec` construction. No benchmark run: this is not a speed claim, and the machine constraint forbids starting benchmarks while another agent builds. |

### Validation

Environment: Darwin arm64, rustc 1.101.0-nightly (32dba69d6 2026-10-09), all
commands run with `-j 2`.

```
cargo check --workspace -j 2
  Finished dev profile; 1 pre-existing manifest warning: unused dependency `anyhow`
  (root Cargo.toml, not touched by this issue; lib target does not use it)

cargo test -j 2 -p cssimpler-core
  49 passed, 0 failed (unit tests)
  1 passed, 0 failed (doctest for VectorPathBuilder)

cargo test -j 2 -p cssimpler --lib
  81 passed, 0 failed

cargo test -j 2 -p cssimpler --test issue_21_vector_builder
  9 passed, 0 failed

cargo run -j 2 --example vector_builder
  built Node::vector <vector> with id Some("builder-artwork")
  wrote work/vector_builder.svg with 2 paths; pass --show to open the UI
```

`work/vector_builder.svg` was inspected (2 paths, correct M/C/Q/L/Z data, fill and
stroke alpha, stroke-width) and then removed; it is not committed. No release
build or benchmark was started, per the concurrent-build constraint. Full
`cargo test --workspace` was not run (core, root lib, and the new integration test
were); renderer/style tests are untouched by this change.

### Notes / limitations

- The existing validator only requires the *path* to start with `MoveTo`; a
  command after `Close` is accepted (SVG implicit subpath semantics). The builder
  delegates to `VectorPath::new`, so it intentionally matches that behavior rather
  than adding a stricter rule. Docs state this explicitly.
- `build()` is all-or-nothing and returns the existing `VectorError`; per-call
  errors are not produced because setters never validate or panic.
- No new dependencies, no public API changed, no SVG parsing/serialization, no
  curve flattening.
- `core/src/vector.rs` untouched (only `core/src/lib.rs` gained 2 lines), so
  overlap with #17 is limited to that file's re-export block.

### PR

Branch pushed as `agent/issue-21-vector-builder`; one PR opened against `main`
(see session report).
