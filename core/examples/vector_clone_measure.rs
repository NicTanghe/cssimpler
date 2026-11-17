//! Measures retained path-clone allocations; run with `cargo run --release -p cssimpler-core --example vector_clone_measure`.
use cssimpler_core::{PathCommand, VectorPaint, VectorPath, VectorPoint};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    hint::black_box,
    sync::atomic::{AtomicUsize, Ordering},
    time::Instant,
};
struct Count;
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for Count {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        unsafe { System.dealloc(p, layout) }
    }
}
#[global_allocator]
static ALLOCATOR: Count = Count;
fn rss() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find_map(|line| {
            line.strip_prefix("VmRSS:")
                .and_then(|s| s.split_whitespace().next())
                .and_then(|s| s.parse().ok())
        })
        .unwrap()
}
fn main() {
    let commands = (0..1000)
        .map(|i| {
            if i == 0 {
                PathCommand::MoveTo(VectorPoint::new(0., 0.))
            } else {
                PathCommand::LineTo(VectorPoint::new(i as f32, i as f32))
            }
        })
        .collect();
    let path = VectorPath::new(commands, VectorPaint::default()).unwrap();
    let before_rss = rss();
    let before_count = ALLOCS.load(Ordering::Relaxed);
    let before_bytes = BYTES.load(Ordering::Relaxed);
    let start = Instant::now();
    let copies: Vec<_> = (0..10000).map(|_| black_box(path.clone())).collect();
    let us = start.elapsed().as_micros();
    let count = ALLOCS.load(Ordering::Relaxed) - before_count;
    let bytes = BYTES.load(Ordering::Relaxed) - before_bytes;
    let after_rss = rss();
    black_box(copies);
    println!(
        "clones=10000 commands_per_path=1000 allocations={count} allocated_bytes={bytes} elapsed_us={us} rss_before_kib={before_rss} rss_after_kib={after_rss}"
    );
}
