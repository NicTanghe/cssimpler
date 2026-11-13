# IME test lab

Use the dedicated example to test native text composition while developing IME
support. It has plain, password, and transformed/centered inputs, a scrollable
case, committed-value/callback counters, and a recent-event log.

## Automated replay

No installed input method, window, or desktop session is needed:

```sh
cargo run --example ime_composition -- --smoke
```

The same scenario runs as an example test:

```sh
cargo test --example ime_composition
```

The replay sends engine events in the same sequence as winit's preedit/commit
protocol. It checks temporary display, selection replacement, callback counts,
commit after empty preedit, direct commit, cancellation, password masking,
composition-key isolation, ordinary Enter after composition, unchanged rebuilds,
controlled replacement, readonly/removal, focus loss, and scaled rendering.
It fails with a nonzero exit code on an assertion failure.

This verifies the editor integration. It does **not** exercise native IME event
delivery, OS session resets, candidate-popup placement, or a particular language's
input method. Those require the interactive mode below.

For lower-level Unicode, geometry and pixel-comparison regressions:

```sh
cargo test -p cssimpler --lib ime_
cargo test -p cssimpler-renderer --lib ime_
```

## Native input method

```sh
cargo run --example ime_composition
```

Switch to an installed Japanese, Chinese, or Korean input method using the OS's
usual input-language controls. Click the first field, select some or all of its
text, and begin composing. Use the candidate picker supplied by that input method;
exact keystrokes differ between languages and input methods.

The committed-value labels and `oninput` counts should remain unchanged while
temporary text appears inside the input. Logs appear both in the window and on
stderr. They record lengths/cursor byte offsets, not typed text. The password
value is never shown by the diagnostics.

| Case | Action | Expected result |
| --- | --- | --- |
| Preview | Type without confirming a candidate | Temporary text is underlined; IME caret/selection is visible when provided. Committed value and `oninput` count stay unchanged. |
| Commit | Confirm a candidate | Temporary text becomes committed once; `oninput` increments once for that commit. No duplicate insertion. |
| Replacement | Select existing text, then compose and confirm | Only the original selection is replaced. |
| Cancel | Use the IME's cancellation action | Original committed text and callback count remain unchanged; temporary text disappears. |
| Enter isolation | Use Enter to confirm a candidate | `onenter` and application Enter counters stay unchanged during composition. An ordinary Enter afterwards increments both. |
| Password | Compose in the password input | Both temporary and committed text are masked. Diagnostics show only committed character count and callback count. |
| Rebuild | Keep composing for at least four seconds | A periodic unchanged-value rebuild is logged; composition stays visible and the committed value stays unchanged. |
| Window focus | Switch to another window while composing | Pending preview is discarded; returning permits fresh composition. |
| Field focus | Click another input while composing | The old field loses its preview; the next IME session belongs to the new field. |
| Positioning | Use the third input; scroll, move/resize the window, or change display scale | Candidate popup follows the rendered caret, including centered text, padding, transforms, scroll offsets, and DPI changes. |
| Readonly/removal | Compose in plain text, then use the corresponding button | Preview is cancelled; readonly or removed inputs do not retain IME focus. |
| Controlled replacement | Use Replace plain value | Plain text becomes the external value with no stale preview. |

Clicking any button cancels composition through pointer handling. Therefore,
use the automatic four-second rebuild to test preservation without a pointer
event. The automated replay separately tests controlled changes during an active
composition without clicking.

Test each desktop backend separately. XWayland results do not establish native
Wayland behavior. Repeat with the optional GPU event-routing path when relevant:

```sh
cargo run --features gpu-viewport --example ime_composition
```

This switches application event routing; the lab itself still uses CPU-rendered
inputs and does not register a 3D viewport.

## Recording a result

Record OS/version, X11/Wayland where applicable, input-method name/language,
display scale, default/GPU feature mode, tested commit, and each case's pass/fail
result. For a failure, include the event sequence, counter changes, and whether
the preview, committed value, or popup position was wrong. Use sample text when
sharing screenshots; stderr contains only event lengths and offsets.

Passing replay tests alone must not close the native-verification item in
[todo](../todo.md).
