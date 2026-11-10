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

Layout, pointer coordinates, and pixel-based wheel deltas use logical CSS pixels. The renderer rasterizes the scene and text outlines at the native surface’s physical resolution, including Retina and fractional scaling, without enlarging a lower-resolution frame. Damage tracking also uses physical pixels. Direct/headless callers can use `render_to_buffer_at_scale` with a logical scene and a physical-size buffer. Minimizing a window suspends drawing.

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


`App::with_event_handler(fn(&mut State, &EngineEvent) -> bool)` and the equivalent `FragmentApp` hook handle application shortcuts or open-menu keyboard navigation before native text editing. Return true to consume an event and invalidate the view; return false to keep default input behavior. Window/event dispatch remains in CSSimpler.

## IME composition

Native editable inputs in `App` and `FragmentApp` display temporary IME text
inline with an underline, the IME's selected range, and its caret. A missing IME
cursor hides the caret. Composition replaces the selected text visually, but
the field's `value` and `oninput` remain unchanged until commit. Commit replaces
the original selection once, including when winit first sends an empty preedit.
Password fields mask composing text as well as committed text.

While composing, the OS owns keyboard editing and navigation: Enter, Escape,
arrows, Tab, and shortcuts do not reach field editing, `onenter`, or application
keyboard handlers. Empty preedit cancels the preview. IME disable, window focus
loss, suspension, pointer repositioning, input removal/editability changes, and
focus changes discard pending composition. A changed controlled value also
discards it; rebuilding with the same value preserves it.

The native runtime enables IME for the focused input and updates the candidate
popup anchor from the rendered caret using logical coordinates, including text
alignment, padding, scrolling, and CSS transforms. Custom `SceneProvider`
implementations using native editing should implement `ime_target` and
`ime_composing` and provide the matching `TextEditDecoration`. Headless hosts
can forward `TextInputEvent::Preedit`, `Commit`, and `Disabled` themselves.

Native IME behavior still needs manual validation on each desktop backend. Run
`cargo run --example input_fields`, focus a text field, compose Japanese/Chinese
or Korean text, change candidates, confirm and cancel, replace a selection, and
switch fields/windows during composition. Check that the candidate popup follows
the caret after moving, resizing, scrolling, and changing display scale. Repeat
in a password field to verify the preview remains masked. Automated tests cover
the editor state, callbacks, candidate geometry, and full/incremental painting;
they do not drive an OS candidate popup.
