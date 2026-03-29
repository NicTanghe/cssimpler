# Rust UI Engine (HTML + CSS + Taffy)
Rust-native UI system with HTML-like syntax, real CSS parsing, explicit rendering (no signals)

---

# Epic A - Workspace & foundations (NON-NEGOTIABLE)

## A1. Workspace layout
Depends: -
Status: in progress

Cargo workspace:

- `core/` - DOM, style system, layout bridge (Taffy), event system
- `renderer/` - rendering backend
- `macro/` - `ui!` procedural macro (bootstrap)
- `app/` - application entrypoint (state + render loop)
- `style/` - stylesheet + selector primitives
- `docs/` - specs and architecture notes

Acceptance
- `cargo run` launches app (pending validation)
- `core` has zero renderer dependencies
- `renderer` does not know about parsing internals
- `macro` outputs only `core` types

---

# Source spec

The original planning document remains in `doc/specsheet.md`.
