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
- Optional GPU renderer (planned)  
- Minimal runtime overhead  

---

## Example

~~~rust
ui! {
    <body style="background: var(--bg); color: var(--text);">
        <div class="container">
            <h1>"Hello, world"</h1>
            <p>"This is just HTML and CSS."</p>
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
renderer/  - CPU renderer (current), GPU (planned)  
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
- Optional GPU backend  
- Same API  

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

TBD
