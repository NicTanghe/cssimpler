//! Headless transition stress test. This measures scene preparation, not painting.
//!
//! Run with `cargo run --release --example transition_stress -- 1000 180`.
//! Arguments are the number of animated elements and measured frames.

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use cssimpler::app::{App, Invalidation};
use cssimpler::core::Node;
use cssimpler::renderer::FrameInfo;
use cssimpler::style::parse_stylesheet;

struct CountingAllocator;
static LIVE_BYTES: AtomicUsize = AtomicUsize::new(0);
static PEAK_BYTES: AtomicUsize = AtomicUsize::new(0);
static ALLOCATED_BYTES: AtomicUsize = AtomicUsize::new(0);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

fn record_allocation(bytes: usize) {
    ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
    ALLOCATED_BYTES.fetch_add(bytes, Ordering::Relaxed);
    let live = LIVE_BYTES.fetch_add(bytes, Ordering::Relaxed) + bytes;
    PEAK_BYTES.fetch_max(live, Ordering::Relaxed);
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            record_allocation(layout.size());
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            record_allocation(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE_BYTES.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) };
    }

    unsafe fn realloc(&self, ptr: *mut u8, old: Layout, new_size: usize) -> *mut u8 {
        let result = unsafe { System.realloc(ptr, old, new_size) };
        if !result.is_null() {
            if new_size >= old.size() {
                record_allocation(new_size - old.size());
            } else {
                LIVE_BYTES.fetch_sub(old.size() - new_size, Ordering::Relaxed);
            }
        }
        result
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[derive(Default)]
struct StressState {
    alternate: bool,
    extra_node: bool,
    structural_changes: bool,
}

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let elements = args.next().map(|n| n.parse()).transpose()?.unwrap_or(1000);
    let frames = args.next().map(|n| n.parse()).transpose()?.unwrap_or(180);
    anyhow::ensure!(
        elements > 0 && frames > 0,
        "element and frame counts must be positive"
    );
    anyhow::ensure!(
        args.next().is_none(),
        "usage: transition_stress [elements] [frames]"
    );

    println!("Headless scene preparation only; no pixels are painted or presented.");
    println!("Simulated frame interval: 16 ms. Every 30 frames, all elements change color.");
    println!("In the structural case, an unrelated child is inserted or removed every 5 frames.");
    run_scenario(elements, frames, false)?;
    run_scenario(elements, frames, true)?;
    Ok(())
}

fn run_scenario(elements: usize, frames: usize, structural_changes: bool) -> anyhow::Result<()> {
    let stylesheet = parse_stylesheet(
        ".item { width: 10px; height: 10px; background-color: #000; \
         transition: background-color 200ms linear; } \
         .item.on { background-color: #fff; }",
    )?;
    let state = StressState {
        structural_changes,
        ..StressState::default()
    };
    let mut app = App::new(
        state,
        &stylesheet,
        |state: &mut StressState, frame: FrameInfo| {
            let toggle = frame.frame_index % 30 == 1;
            let change_structure = state.structural_changes && frame.frame_index % 5 == 2;
            if toggle {
                state.alternate = !state.alternate;
            }
            if change_structure {
                state.extra_node = !state.extra_node;
                Invalidation::Structure
            } else if toggle {
                Invalidation::Paint
            } else {
                Invalidation::Clean
            }
        },
        move |state: &StressState| {
            let mut animated = Node::element("section");
            for _ in 0..elements {
                let mut item = Node::element("div").with_class("item");
                if state.alternate {
                    item = item.with_class("on");
                }
                animated = animated.with_child(item.into());
            }
            let mut unrelated = Node::element("aside");
            if state.extra_node {
                unrelated = unrelated.with_child(Node::element("span").into());
            }
            Node::element("main")
                .with_child(animated.into())
                .with_child(unrelated.into())
                .into()
        },
    );

    const WARMUP: u64 = 30;
    let frame = |index| FrameInfo {
        frame_index: index,
        delta: Duration::from_millis(16),
    };
    for index in 0..WARMUP {
        black_box(app.frame(frame(index)));
    }

    let mut samples = Vec::with_capacity(frames);
    let start_live = LIVE_BYTES.load(Ordering::Relaxed);
    PEAK_BYTES.store(start_live, Ordering::Relaxed);
    ALLOCATED_BYTES.store(0, Ordering::Relaxed);
    ALLOCATIONS.store(0, Ordering::Relaxed);
    let mut active_frames = 0;
    for index in 0..frames {
        let started = Instant::now();
        black_box(app.frame(frame(WARMUP + index as u64)));
        samples.push(started.elapsed().as_micros());
        active_frames += usize::from(app.latest_stats().transition_active);
    }
    let peak = PEAK_BYTES.load(Ordering::Relaxed);
    let allocated = ALLOCATED_BYTES.load(Ordering::Relaxed);
    let allocations = ALLOCATIONS.load(Ordering::Relaxed);
    samples.sort_unstable();
    let percentile = |p: usize| samples[(samples.len() - 1) * p / 100];
    println!(
        "scenario={} elements={} frames={} active_frames={} median_us={} p95_us={} max_us={} peak_live_mib={:.2} extra_peak_mib={:.2} allocated_mib={:.2} allocations={}",
        if structural_changes {
            "structural"
        } else {
            "stable"
        },
        elements,
        frames,
        active_frames,
        percentile(50),
        percentile(95),
        samples[samples.len() - 1],
        peak as f64 / 1_048_576.0,
        peak.saturating_sub(start_live) as f64 / 1_048_576.0,
        allocated as f64 / 1_048_576.0,
        allocations,
    );
    Ok(())
}
