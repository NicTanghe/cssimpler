# Rust UI Engine (HTML + CSS + Taffy)
Rust-native UI system with HTML-like syntax, real CSS parsing, explicit rendering (no signals)

---

# Epic A - Workspace & foundations (NON-NEGOTIABLE)

## A1. Workspace layout
Depends: -
Status: done

Cargo workspace:

- `core/` - DOM, style system, layout bridge (Taffy), event system
- `renderer/` - rendering backend
- `macro/` - `ui!` procedural macro (bootstrap)
- `style/` - stylesheet + selector primitives
- `examples/` - demo applications and integration samples
- `docs/` - specs and architecture notes

Acceptance
- `cargo run --example demo` launches the demo app
- `core` has zero renderer dependencies
- `renderer` does not know about parsing internals
- `macro` outputs only `core` types

Status sync
- `A1` through `G4` are implemented in the workspace
- `G4` currently uses renderer-side dirty-region diffing and partial repaint while preserving the public API
- `H1` through `H5` are planned for real typography, system fonts, and arbitrary font loading

---

# Source spec

The original planning document remains in `doc/specsheet.md`.
