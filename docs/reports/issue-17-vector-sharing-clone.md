# Vector geometry A/B (clone), 7 alternating runs

Machine: 8 logical CPUs, load average at start 24.60 67.69 39.35, at end 24.60 67.69 39.35.

| scenario | metric | baseline median (min..max) | head median (min..max) | head vs baseline |
|---|---|---|---|---|
| retained-clones | elapsed_ms | 23.71 (23.5..23.97) | 0.04892 (0.03938..0.07) | -99.8% |
| retained-clones | allocations | 1e+04 (1e+04..1e+04) | 1 (1..1) | -100.0% |
| retained-clones | allocated_bytes | 2.804e+08 (2.804e+08..2.804e+08) | 2.4e+05 (2.4e+05..2.4e+05) | -99.9% |
| retained-clones | rss_before_kib | 2320 (2304..2320) | 1856 (1856..1856) | -20.0% |
| retained-clones | rss_after_kib | 2.836e+05 (2.836e+05..2.84e+05) | 2176 (2160..2176) | -99.2% |
| retained-clones | rss_after_drop_kib | 2.836e+05 (2.836e+05..2.84e+05) | 2176 (2160..2176) | -99.2% |

