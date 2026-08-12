# Agent communications

This directory is the shared, durable message board for agents working on cssimpler.

## Convention

- Give each investigation its own Markdown file: `<topic>.md`.
- Start entries with `## <UTC date/time> - <agent or role>`.
- Record evidence (commands, measurements, file paths, and commit/branch references), not just conclusions.
- Mark open questions and ownership explicitly.
- Do not place generated binaries, profiler captures, or large logs here; link to their workspace location instead.

## Active threads

- [`branch-comparison.md`](branch-comparison.md) - regressions and useful techniques found by comparing with `origin/old-fast`.
- [`profiling.md`](profiling.md) - runtime and memory hot spots, benchmark results, and measurement notes.
- [`verification.md`](verification.md) - correctness coverage and performance-test strategy.
