# Native touch applications

Use `cssimpler::app::App` with the `ui!` HTML macro and CSS. The native runtime owns the window, rendering, RGB10A2 presentation, mouse/touch input, scrolling, and DPI conversion. Application crates do not need winit, softbuffer, or rendering loops.

```rust
use cssimpler::{app::{App, Invalidation}, renderer::WindowConfig, style::parse_stylesheet, ui};

let stylesheet = parse_stylesheet(".panel { padding: 24px; } button { min-height: 52px; }").unwrap();
let app = App::new((), &stylesheet, |_, _| Invalidation::Clean, |_| {
    let rows = vec![ui! { <p>First</p> }, ui! { <p>Second</p> }];
    ui! { <section class="panel">{rows}{Some(ui! { <button>Continue</button> })}</section> }
});
// app.run(WindowConfig::new("Register", 1360, 900))?;
```

Dynamic expression slots accept individual nodes, text, vectors, and optional nodes. Lists are flattened without an extra layout element. Consumers only need the `cssimpler` dependency; macro expansion uses its core re-export.

## Touch and DPI

The first touch contact becomes the primary pointer. A tap produces one click; a swipe scrolls the nearest scrollable ancestor and cancels the pending click. Cancelled contacts and extra fingers do not trigger clicks. Mouse events synthesized after touch are briefly suppressed to prevent duplicate actions. Quick taps preserve both press and release even between redraw notifications.

Layout, pointer coordinates, and pixel-based wheel deltas use logical CSS pixels. The native surface remains at physical resolution; presentation scales the logical render buffer to fill that surface, including Retina and fractional scaling. Scaled presentation currently repaints the full surface; equal-size presentation retains damage-region updates. Minimizing a window suspends drawing.

## Responsive CSS

Normal App rendering evaluates viewport media queries and reevaluates them after resizing. Width/height queries support exact values, min/max, ranges, intervals, px/absolute lengths, and em/rem using the initial 16px size. Orientation, screen/all/print, and/or/not, comma alternatives, and nested queries are supported. Unimplemented media features do not make their declarations apply unconditionally.

```css
.workspace { display: flex; }
@media (max-width: 1150px) {
    .workspace { flex-direction: column; }
}
```

For direct style-engine use, call `Stylesheet::for_viewport(width, height)` before style resolution, or use the viewport-aware render-tree builders. Non-viewport parsing/resolution retains the existing API behavior.

## Deferred window closing

`App` and `FragmentApp` offer `with_close_handler(fn(&mut State) -> bool)` and `with_exit_when(fn(&State) -> bool)`. Return false from the close handler to show a confirmation or finish pending work; set application state so the exit predicate becomes true when closing is approved. The default behavior remains immediate closing.

## Mouse and keyboard

Mouse clicks and touch taps preserve press/release edges between frames. Native `<input type="text">` and `<input type="password">` support typing, mouse positioning/selection, Backspace/Delete, arrows, Home/End, Ctrl+A or Command+A, and Tab/Shift+Tab between editable inputs. Password rendering masks characters and maps caret/selection offsets back to the original UTF-8 text; `oninput` receives the original value.

Use `autofocus="autofocus"` to focus a field when it appears. The last autofocus input takes precedence for modal editors. An `aria-modal="true"` container confines input traversal and autofocus to its descendants. An `onenter={handler}` callback runs on a non-repeating Enter press. Put `data-preserve-input-focus="true"` on on-screen keypad buttons so clicking them keeps physical typing directed at the current input. Disabled/readonly inputs are excluded from native editing and input traversal. Input traversal currently covers editable fields; other controls remain mouse/touch operated.
