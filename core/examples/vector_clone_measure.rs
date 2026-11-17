//! Retained-clone allocation and RSS comparison for native vector geometry.
//!
//! Run in a release build:
//! `cargo run --release -p cssimpler-core --example vector_clone_measure`
//!
//! The workload builds one native path with `--commands` commands, retains
//! `--clones` clones of it, and reports heap allocation calls/requested bytes
//! made while cloning plus process RSS before and after. It is a retained-storage
//! microbenchmark, not a frame-rate benchmark. The wrapping allocator counts
//! `alloc` calls only (like the original harness); it is portable across Linux
//! and macOS: Linux RSS comes from `/proc/self/status`, elsewhere from `ps`.

use cssimpler_core::{PathCommand, VectorPaint, VectorPath, VectorPoint};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    hint::black_box,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

struct CountingAllocator;
static ALLOC_CALLS: AtomicU64 = AtomicU64::new(0);
static ALLOC_BYTES: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOC_CALLS.fetch_add(1, Ordering::Relaxed);
        ALLOC_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        // SAFETY: forwarded unchanged to the system allocator.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: forwarded unchanged to the system allocator.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator;

fn rss_kib() -> Option<u64> {
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        return status.lines().find_map(|line| {
            line.strip_prefix("VmRSS:")
                .and_then(|value| value.split_whitespace().next())
                .and_then(|value| value.parse().ok())
        });
    }
    let pid = std::process::id().to_string();
    let output = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid])
        .output()
        .ok()?;
    String::from_utf8(output.stdout).ok()?.trim().parse().ok()
}

fn json(value: Option<u64>) -> String {
    value.map_or("null".to_string(), |value| value.to_string())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| {
        args.windows(2)
            .find(|pair| pair[0] == name)
            .map(|pair| pair[1].as_str())
    };
    let label = arg("--label").unwrap_or("unlabeled");
    let clones = arg("--clones")
        .unwrap_or("10000")
        .parse::<usize>()
        .expect("--clones must be an integer")
        .max(1);
    let command_count = arg("--commands")
        .unwrap_or("1000")
        .parse::<usize>()
        .expect("--commands must be an integer")
        .max(2);

    let commands = (0..command_count)
        .map(|index| {
            if index == 0 {
                PathCommand::MoveTo(VectorPoint::new(0., 0.))
            } else {
                PathCommand::LineTo(VectorPoint::new(index as f32, index as f32))
            }
        })
        .collect();
    let path = VectorPath::new(commands, VectorPaint::default()).unwrap();
    // Keep warmup allocations out of the measured window.
    let warmup: Vec<_> = (0..16).map(|_| black_box(path.clone())).collect();
    black_box(&warmup);
    drop(warmup);

    let rss_before = rss_kib();
    let allocs_before = ALLOC_CALLS.load(Ordering::Relaxed);
    let bytes_before = ALLOC_BYTES.load(Ordering::Relaxed);
    let start = Instant::now();
    let retained: Vec<_> = (0..clones).map(|_| black_box(path.clone())).collect();
    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
    let allocations = ALLOC_CALLS.load(Ordering::Relaxed) - allocs_before;
    let allocated_bytes = ALLOC_BYTES.load(Ordering::Relaxed) - bytes_before;
    let rss_after = rss_kib();
    black_box(&retained);
    drop(retained);
    let rss_after_drop = rss_kib();

    println!(
        "{{\"scenario\":\"retained-clones\",\"label\":\"{}\",\"clones\":{},\"commands\":{},\
\"elapsed_ms\":{:.5},\"allocations\":{},\"allocated_bytes\":{},\
\"rss_before_kib\":{},\"rss_after_kib\":{},\"rss_after_drop_kib\":{}}}",
        label.replace('"', ""),
        clones,
        command_count,
        elapsed_ms,
        allocations,
        allocated_bytes,
        json(rss_before),
        json(rss_after),
        json(rss_after_drop),
    );
}
