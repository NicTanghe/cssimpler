# todo

Platform support and remaining work for cssimpler. Reviewed against the source
and existing documentation on 2026-11-10. This is an implementation inventory,
not a claim that every OS has been tested on this date. Update the tables when
features land or platform verification changes.

## OS support sheet

**Implemented** means a code path exists; **partial** means there are restrictions;
**pending** means the feature is missing. Shared features still depend on the
window system, installed fonts, devices, and compositor.

| Capability | Windows | macOS | Linux / X11 (including XWayland) | Linux / native Wayland |
| --- | --- | --- | --- | --- |
| CPU UI rendering, partial redraws, idle redraw suppression | Implemented | Implemented | Implemented | Implemented |
| HTML-like macros, CSS subset, Taffy layout, viewport media queries | Implemented | Implemented | Implemented | Implemented |
| Physical-resolution rendering and logical-pixel input / DPI scaling | Implemented | Implemented | Implemented | Implemented |
| Mouse, wheel, primary-touch tap / swipe scrolling | Implemented | Mouse/wheel; native touch unavailable | Implemented; device dependent | Implemented; device dependent |
| Text/password editing, text selection, clipboard copy/cut/paste | Implemented | Implemented; Command shortcuts | Implemented; clipboard ownership retained | Partial; clipboard depends on compositor data-control support |
| IME text commit | Implemented | Implemented | Implemented | Implemented |
| IME preedit display / composition editing | Implemented; native verification pending | Implemented; native verification pending | Implemented; native verification pending | Implemented; native verification pending |
| Frameless drag region and edge/corner resizing | Implemented | Implemented | Implemented | Implemented; native move starts on pointer press |
| Native glass / desktop blur | Acrylic via native composition API | NSVisualEffectView with custom CPU presenter | Partial: transparent/tinted window; no compositor blur request | Partial: transparent/tinted window; no compositor blur request |
| Native GPU viewport (`gpu-viewport`, experimental) | DirectComposition | CAMetalLayer | X11 child window; X Shape 1.1 required | Pending; visible registrations return an error |
| Transparent GPU viewport | Implemented | Implemented | Requires ARGB visual, X Composite 0.4, and a compositing desktop | Pending |
| USD meshes, cameras, orbit/zoom, time sampling | Implemented subset | Implemented subset | Implemented subset | Blocked by native GPU host |
| GPU viewport 4x MSAA | Direct full-frame resolve when supported | Memoryless resolve on suitable Apple GPUs; strip fallback otherwise | Strip resolve when surface capabilities allow | Blocked by native GPU host |
| GPU rasterization of the ordinary CSS UI | Pending | Pending | Pending | Pending |

Android, iOS, browser/WASM, and other operating systems have no documented,
validated application support here. Dependency backend availability alone does
not establish cssimpler support; native GPU hosts explicitly target the desktop
platforms above.

Important limits:

- Native GPU views must be fully visible, rectangular, childless regions. They
  hide under partial clipping, later-painted UI, rounded corners, or unsupported
  transforms. CPU backgrounds remain visible while a viewport is hidden.
- GPU viewports do not make the CPU UI GPU-rendered. Backdrop effects cannot yet
  sample live 3D content; opaque GPU backgrounds assume a solid backdrop.
- MSAA is capability dependent; unsupported targets retain single-sample output.
- Glass appearance is OS/compositor dependent. Linux transparency/tint is not
  equivalent to requesting desktop blur.
- Keyboard traversal currently covers editable inputs. It does not provide
  complete keyboard navigation or an OS accessibility bridge for all controls.

Sources: [native application behavior](docs/native-touch-ui.md),
[GPU viewport behavior and validation](docs/gpu-viewports.md),
[native GPU hosts](renderer/src/gpu/native.rs),
[Linux GPU host](renderer/src/gpu/native/x11.rs),
[native glass](renderer/src/native_glass.rs),
[clipboard](src/app/input/clipboard.rs), and
[text input handling](src/app/input.rs).

## Features still to implement

Priorities below are suggested ordering, not committed delivery dates.
**High** closes a correctness or platform gap; **medium** expands functionality;
**later** is exploratory or depends on upstream work.

| Done | Priority | Feature / gap | Next concrete step / completion check |
| --- | --- | --- | --- |
| [ ] | High | Native Wayland GPU embedding | Implement the remaining native host; verify opaque/transparent views, resize, hide/show, input routing, and teardown on Wayland. See the [viewport spec](docs/native-gpu-viewport-specsheet.md). |
| [x] | High | IME composition implementation | Temporary preview, underline/selection/caret, commit/cancel, password masking, shortcut isolation, and native candidate positioning implemented with automated editor/rendering tests. See [IME behavior](docs/native-touch-ui.md#ime-composition). |
| [ ] | High | Native IME verification | Manually verify Japanese/Chinese/Korean composition and candidate positioning on Windows, macOS, X11, and Wayland, including focus changes and display scaling. Automated tests do not drive an OS IME. |
| [ ] | High | Keyboard access and accessibility | Extend focus/activation beyond editable inputs and add an OS accessibility bridge; verify buttons, modal focus, labels, and screen-reader behavior. Modal input confinement alone is not full accessibility support. |
| [ ] | High | CSS conditional-rule semantics | Evaluate conditions/scopes or reject unsupported behavior explicitly. `collect_style_rules` currently descends into `@supports`, `@container`, and `@scope` without enforcing their semantics; add cases where their declarations must not apply. |
| [ ] | Medium | CSS `@font-face` | Load declared font sources and integrate family/weight/style matching with existing font registration and fallback. The rule is currently ignored. |
| [ ] | Medium | CSS keyframe animations | Implement `@keyframes` evaluation, timing, invalidation, and idle scheduling. Existing transitions are a separate, already implemented feature. |
| [ ] | Medium | CSS / SVG coverage | Prioritize real example failures: gradient interpolation hints, broader filters, SVG gradient transforms/inheritance, and additional selectors. Keep unsupported forms explicit; maintain a tested subset rather than claiming full browser compatibility. |
| [ ] | Medium | GPU viewport clipping and stacking | Support partially clipped/rounded views and overlap with CPU UI; verify scrolling and transformed placement without hiding the entire viewport. |
| [ ] | Medium | Effects sampling live GPU content | Define composition support for backdrop blur and non-solid backgrounds without forcing unrelated CPU UI repaint. |
| [ ] | Medium | Broader USD rendering | Add authored normals/materials/textures/lights and geometry features incrementally. Subdivision, skinning, concave faces, arbitrary material networks, and production PBR remain outside the initial renderer's subset. |
| [ ] | Medium | Linux desktop blur integration | Investigate explicit compositor integration and fallbacks; verify each supported desktop separately rather than treating transparency as blur. |
| [ ] | Later | Optional GPU UI renderer | Evaluate a backend such as Vello while preserving CSS behavior, text quality, and the CPU path. Keep this distinct from embedded GPU viewports. |
| [ ] | Later | Dialog / popover helpers and Lottie | Scope the ideas in [human notes](docs/human_notes.md) into concrete APIs and examples before implementation. |
| [ ] | Later | Upstream Hydra-like USD backend | Migrate when a suitable upstream implementation exists, preserving stage ownership and public registration APIs. See [USD2](docs/native-gpu-viewport-specsheet.md#usd2-future-upstream-hydra-like-backend). |

CSS rule gaps above are visible in [style rule collection](style/src/lib.rs).
The reported slow 3D transform animations in [human notes](docs/human_notes.md)
still need a reproducible example and timing trace before being treated as a
confirmed current defect.

## Optimisations to investigate

These are profiling candidates, not claims of measured improvements. Start with
a repeatable release baseline and retain rendering and interaction correctness.

| Done | Priority | Candidate | Measurement / acceptance check |
| --- | --- | --- | --- |
| [ ] | High | Rounded translucent paint spans | Add thin/subpixel rounded workloads before changing span discovery; improve the rounded translucent case without losing fractional edge AA. See [performance results](agent_docs/performance-results.md). |
| [ ] | Medium | Repeated stacking and hit-test traversal | Profile repeated ordering/traversal during pointer movement; reuse prepared data with explicit invalidation if it reduces CPU work. |
| [ ] | Medium | Focused-input authored-root reconstruction | Measure `root_as_node` and scene decoration costs; avoid rebuilding/cloning unrelated authored trees during caret, selection, and editing updates. |
| [ ] | Medium | Compact runtime glass metadata | Measure retained scene data and redundant copies; retain only metadata required by native glass presentation and invalidation. |
| [ ] | Medium | macOS native-glass presenter buffers | Profile the two full-size alpha-image conversions/allocations per presentation; investigate buffer reuse or reduced conversion work while preserving alpha/tint composition. See [presenter](renderer/src/native_glass.rs). |
| [ ] | Medium | Tighter paint-only damage | Measure whether changes to a node's own visuals repaint unchanged descendants; tighten bounds only with coverage for shadows, transforms, blur, and overlap. |
| [ ] | Later | SIMD mask blending | Profile text/shadow/alpha-mask loops and compare vectorized spans against the scalar path with identical output. |
| [ ] | Medium | Cross-platform performance and smoke baselines | Record CPU frame time, allocations, idle work, RAM/VRAM, resize, DPI, and transparent composition on Windows, macOS, X11, and Wayland. Label hardware, build mode, and compositor. |

Use [the performance strategy](agent_docs/performance-strategy.md) and
[headless benchmark](examples/headless_render_bench.rs) for repeatable work;
use [GPU validation commands](docs/gpu-viewports.md) for native viewports.

## Already implemented — preserve during future work

Do not reopen these as missing features based only on older specs or notes:

- Scoped invalidation, dirty-region rendering, idle redraw suppression, and
  physical-resolution CPU rasterization.
- Text selection and clipboard support, password export protection, touch/DPI
  handling, viewport media queries, and deferred close handlers.
- Text/shadow LRU caches, cached gradient layers, and cached scene bounds.
- Direct full-redraw worker writes, reduced scene cloning, and fully covered
  pixel AA fast paths documented in [performance results](agent_docs/performance-results.md).
- Shared GPU device infrastructure, on-demand viewport redraw, camera/target
  reuse, and platform-specific MSAA paths documented in [GPU usage](docs/gpu-viewports.md).

When closing a row, link its implementation and relevant verification here.
Keep older specs as design references; this checklist should track remaining work.
