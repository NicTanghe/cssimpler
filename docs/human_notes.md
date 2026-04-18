right now all text is in braces aswell. waybe we can ad another entry to the child to allow plain text aswell. allthough that might be confusing.

ok i was wondering set interaction state seeminglty only has 1 thing but cant we hover 3 things at the same time ects?

implement dualog and autoclosures ects

popover hnts ?


scroll target group !


Likely Next Wins
If we were profiling this renderer, these would be the first things I’d investigate.

- Precompute subtree bounds for parallel full redraw too. Right now parallel full redraw uses CullMode::Subtree, which means node_intersects_clip() may recompute subtree_visual_bounds(node) repeatedly per worker band at renderer/src/lib.rs:1533. Reusing a cached bounds tree there could cut traversal overhead.
- Replace “clear the whole cache when full” with LRU-style eviction for shadow/text caches. Right now the caches just flush entirely at capacity, which is simple but can cause performance cliffs. See renderer/src/lib.rs:314 and renderer/src/fonts.rs:272.
- Cache rasterized gradient layers for static nodes if gradients become common. Gradients currently still sample per pixel each paint, which is correct but can become expensive on large animated surfaces.
- Add more span-based filling for alpha masks and rounded shapes. A lot of time in software rasterizers disappears into per-pixel bounds checks and indexing; row-span batching can help.
- Tighten damage for “own visuals changed, children unchanged” cases. Right now a node-level visual mismatch can dirty the union of previous/current subtree bounds at renderer/src/lib.rs:2645. That is safe, but sometimes larger than necessary.
- SIMD on mask blend loops. The text/shadow mask loops are classic candidates if CPU raster becomes a bottleneck.

- Long-term: a GPU backend. The current architecture is already clean enough that RenderNode is a decent scene representation for a future GPU painter.

Use Vello as renderer when transitioning to GPU also supporting lottie whould be neat.


something is wrong with the 3d transform animations,
THey seem unaturally slow.


gpu renderer has way way more banding this needs to be fixed.
