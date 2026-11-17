# Prepared vector geometry: stage-separated measurements (issue #22)

Recorded on 2026-11-17 22:07:43 CET from `target/release/examples/vector_prepared_geometry_bench` with 7 fresh release processes per scenario, round-robin order (one pass = one run of every scenario).

Machine: host `Darwin arm64`, CPUs `8`, load when starting `['5.26 6.16 10.53']`, `/proc/stat` lines `None`, rustc `rustc 1.101.0-nightly (32dba69d6 2026-10-09)`.

Per-frame stage numbers are aggregate CPU work summed across render workers, not wall time; `Frame ms` is the wall median with the probe off and `Empty scene` paints the same node tree with empty vector scenes. `Vector work = Frame − Empty`. `Build+stroke % of paint` is build+stroke CPU over attributed paint CPU, which equals the share of wall frame time for serial paint and estimates it for parallel paint. Full per-metric spreads and the raw JSON are in the `.jsonl` next to this file.

| Scenario | Frame ms | Empty scene | Vector work | Build µs | Stroke µs | Project µs | Raster µs | Blend µs | Build+stroke % of paint |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| small-icon | 5.93 (5.889..5.978) | 0.01 | 5.92 | 22.2 | 209.6 | 9.6 | 2238 | 3394 | 3.95% |
| small-icon-edit | 5.895 (5.885..6.068) | 0.02 | 5.88 | 21.0 | 209.3 | 9.7 | 2241 | 3400 | 3.94% |
| dense-strokes | 13.98 (13.89..14.13) | 0.01 | 13.96 | 23.3 | 349.0 | 4.6 | 8807 | 4696 | 2.68% |
| curves | 3.414 (3.353..3.605) | 0.07 | 3.34 | 50.1 | 343.2 | 38.2 | 8383 | 12558 | 1.84% |
| curves-serial | 4.511 (4.45..4.638) | 0.01 | 4.50 | 1.4 | 16.9 | 0.7 | 802 | 3678 | 0.41% |
| curves-translate | 3.504 (3.492..3.595) | 0.07 | 3.43 | 51.6 | 357.0 | 43.3 | 8814 | 12606 | 1.88% |
| curves-zoom | 3.836 (3.801..3.89) | 0.07 | 3.77 | 52.5 | 383.0 | 44.4 | 9309 | 14988 | 1.73% |
| curves-paint-update | 3.39 (3.335..3.53) | 0.07 | 3.32 | 49.1 | 385.6 | 41.4 | 8341 | 12685 | 2.03% |
| curves-edit | 3.401 (3.346..3.535) | 0.07 | 3.33 | 54.2 | 339.2 | 36.9 | 8354 | 12758 | 1.85% |

