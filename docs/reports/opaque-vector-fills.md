# Static rendering with opaque vector fills

The CPU renderer writes solid-color, fully covered runs directly into the framebuffer. It reuses its coverage tile and writes the corresponding alpha span when the target has a separate alpha plane.

The benchmark paints 12 curved paths with solid strokes in a 960 × 640 buffer. The opaque scene has fully opaque fills. The mixed scene alternates opaque fills with fills at alpha 128. SVG and native vector modes receive the same authored curves and colors. The SVG parser flattens curves during preparation, so framebuffer checksums compare before and after within each mode.

| Static scene | Input | Baseline ms/frame | Optimized ms/frame | Baseline peak RSS (KiB) | Optimized peak RSS (KiB) |
| --- | --- | ---: | ---: | ---: | ---: |
| Opaque | SVG | 11.903 | 3.995 | 7,556 | 6,980 |
| Opaque | Native paths | 11.354 | 3.407 | 6,408 | 6,236 |
| Mixed | SVG | 12.412 | 8.184 | 7,612 | 7,036 |
| Mixed | Native paths | 11.181 | 7.505 | 6,536 | 6,156 |

Each result is the median of seven fresh Linux processes, each measuring 31 release frames after a warm-up frame. The baseline was commit `5249a0d` with the same benchmark source. Both builds used Rust 1.100.0 on an Intel Xeon Platinum 8370C with a two-CPU container quota. The runs alternated baseline and optimized binaries and used the headless renderer with layer caching disabled. Render-tree construction and preparation were timed separately from painting. Peak RSS is the process-wide Linux `VmHWM` reading and includes the executable and allocator. The optimization adds no persistent buffer.

The framebuffer checksums matched their respective baselines for all four cases. Opaque-fill time fell by 66–70% and mixed-fill time fell by 33–34% in these scenes.

To reproduce, use the benchmark source from this change with each renderer revision and run each mode and scene in a fresh process.

```sh
cargo build --release --locked --example native_vector_bench
for mode in svg native; do
  for scene in opaque mixed; do
    target/release/examples/native_vector_bench --mode "$mode" --scene "$scene" --frames 31
  done
done
```
