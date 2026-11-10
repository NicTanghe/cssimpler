# cssimpler

> A Rust UI library that brings things back to **HTML + CSS** — nothing more.

No signals.  
No virtual DOM.  
No JavaScript runtime.  

---

## Why

Frontend development got overengineered.

We abstracted CSS because it felt slow.  
Then we abstracted those abstractions with state systems, signals, and frameworks.

Now AI writes and adjusts CSS instantly — but the complexity stayed.

**cssimpler removes that complexity.**

---

## Philosophy

- Keep it simple  
- HTML is structure  
- CSS is styling  
- No JavaScript is a feature  
- No runtime magic  

If you know HTML and CSS, you already know this library.

---

## Features

- HTML-like UI via Rust macros  
- Real CSS (no custom styling DSL)  
- Taffy layout engine  
- CPU renderer (current)  
- Optional native GPU viewports for USD 3D scenes (`gpu-viewport`)
- Minimal runtime overhead  

---

## Example

~~~rust
ui! {
    <body style="background: var(--bg); color: var(--text);">
        <div class="container">
            <h1>Hello, world</h1>
            <p>This is just HTML and CSS.</p>
        </div>
    </body>
}
~~~

No signals.  
No indirection.  
What you write is what renders.

---

## Styling

Use actual CSS:

~~~css
:root {
    --bg: #0e0e0e;
    --text: #ffffff;
}

.container {
    padding: 16px;
}
~~~

Dynamic theming = changing variables. Nothing more.

---

## Counter Button Example

If you do want a button, it can stay simple too:

~~~rust
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;
use cssimpler::app::{App, Invalidation};
use cssimpler::core::Node;
use cssimpler::renderer::{FrameInfo, WindowConfig};
use cssimpler::style::{Stylesheet, parse_stylesheet};
use cssimpler::ui;

static CLICK_COUNT: AtomicU64 = AtomicU64::new(0);

fn main() -> Result<()> {
    let config = WindowConfig::new("cssimpler / counter", 480, 220);

    App::new((), stylesheet(), update, build_ui)
        .run(config)
        .map_err(Into::into)
}

fn update(_state: &mut (), _frame: FrameInfo) -> Invalidation {
    Invalidation::Clean
}

fn build_ui(_state: &()) -> Node {
    let count = CLICK_COUNT.load(Ordering::Relaxed);

    ui! {
        <div class="card">
            <p>{format!("count: {count}")}</p>
            <button class="button" type="button" onclick={increment}>
                Increment
            </button>
        </div>
    }
}

fn increment() {
    CLICK_COUNT.fetch_add(1, Ordering::Relaxed);
}

fn stylesheet() -> &'static Stylesheet {
    static STYLESHEET: OnceLock<Stylesheet> = OnceLock::new();

    STYLESHEET.get_or_init(|| {
        parse_stylesheet(
            r#"
            .card {
                min-height: 100vh;
                display: flex;
                gap: 12px;
                justify-content: center;
                align-items: center;
                background: #101218;
                color: #f5f7ff;
            }

            .button {
                padding: 10px 14px;
            }
            "#,
        )
        .expect("counter stylesheet should stay valid")
    })
}
~~~

No state struct.  
No signals.  
Just a counter and a button.

---



## What This Is Not

- ❌ No JavaScript execution  
- ❌ No reactive signal systems  
- ❌ No framework-specific APIs  
- ❌ No virtual DOM  

If you want a reactive framework — this is not it.

---

## Architecture

~~~text
core/      - DOM, layout bridge, style resolution  
style/     - CSS parsing (lightningcss), selectors  
renderer/  - CPU UI renderer and optional native GPU 3D viewports
macro/     - ui! macro (HTML-like → AST)  
app/       - entrypoint + render loop  
~~~

---

## Rendering

**Current**
- CPU renderer  
- Partial redraws  
- Deterministic output  

**Planned**
- Optional GPU backend for UI rasterization
- Same API  

---

## 3D Viewports (experimental)

Enable the opt-in `gpu-viewport` feature to embed GPU-rendered USD scenes in the
CPU-rendered UI. Native viewports use DirectComposition on Windows, Metal on
macOS, and X11 on Linux. The USD examples support orbit/zoom controls,
on-demand rendering, transparent composition over native glass, and 4× MSAA
when supported by the GPU. Native Wayland embedding is still pending.

```toml
[dependencies]
cssimpler = { path = "path/to/cssimpler", features = ["gpu-viewport"] }
```

Try the USD viewer or the transparent glass cube:

```sh
cargo run --release --features gpu-viewport --example usd_viewport
cargo run --release --features gpu-viewport --example usd_glass_cube
```

The feature is disabled by default; CPU-only builds do not include the GPU or
USD dependencies. The initial USD renderer supports mesh geometry and cameras,
with built-in studio shading; production PBR and arbitrary material networks
are still pending. We plan to migrate to a Hydra-based rendering backend in the
future, replacing the initial USD renderer.

See [native GPU viewport documentation](docs/gpu-viewports.md)
for registration APIs, platform requirements, supported USD features, and AA
performance and memory details.

---

## Design Principles

### CSS is no longer the bottleneck

Abstractions over CSS made sense before.  
Now they add complexity without solving a real problem.

---

### Don’t fight the web model

- Keep hierarchy  
- Keep layout semantics  
- Don’t reinvent CSS  

---

### No hidden systems

No:
- background schedulers  
- reactive graphs  
- invisible re-renders  

---

## When to Use

Use cssimpler if you want:

- Predictable performance  
- Full rendering control  
- Minimal abstraction  
- Rust-native UI  

Avoid it if you want:

- SPA frameworks  
- JS ecosystem tooling  

---

## Status

Early stage — APIs may change.

---

## Roadmap

- [ ] Expand CSS coverage  
- [ ] GPU renderer  
- [ ] Text layout improvements  
- [ ] Full HTML5 support  
- [ ] Profiling tools  

---

## Contributing

Keep it simple.

If you want to add something it probably has value so 
don`t be afraid to put in a request.


---

## License

MIT

## Text selection and frameless windows

Native text fields support drag selection, Shift+Arrow/Home/End, Shift+click,
Ctrl/Cmd+A, and clipboard copy/cut/paste. Password fields accept paste but do not
export their contents through copy or cut. Ordinary page text can be selected
across text blocks and copied; interactive controls are excluded. Selection
colours follow `::selection`. Connected highlights merge into one backdrop with
only exposed corners rounded. Outer ends have 2px horizontal and 1px vertical
padding; starts and ends within a bounded 20px range align within each block.
The union is composited once to avoid seams and doubled opacity at any display
scale. Manual render-tree clients can call `prepare_text_selection_backgrounds`
before painting; applications do this automatically. Selecting text
preserves the original full-run glyph positions, tracking, shaping, and wrapping.

For frameless windows, `WindowConfig::with_drag_region_height(height)` enables a
draggable strip at the top of the client area. Interactive content takes priority.
`WindowConfig::with_resize_border_width(width)` enables native resizing from all
edges and corners with matching pointer cursors. Both measurements use logical
pixels and default to zero. Resizing takes priority over dragging and is disabled
for decorated, maximized, fullscreen, or non-resizable windows.
