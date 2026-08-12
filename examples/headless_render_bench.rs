use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use cssimpler::core::{
    BackgroundLayer, Color, CornerRadius, GradientDirection, GradientInterpolation, GradientStop,
    LayoutBox, LengthPercentageValue, LinearGradient, RenderNode,
};
use cssimpler::renderer::render_to_buffer;

struct CountingAllocator;

static LIVE_BYTES: AtomicUsize = AtomicUsize::new(0);
static PEAK_BYTES: AtomicUsize = AtomicUsize::new(0);
static ALLOCATED_BYTES: AtomicUsize = AtomicUsize::new(0);
static ALLOCATION_COUNT: AtomicUsize = AtomicUsize::new(0);

fn record_allocation(bytes: usize) {
    ALLOCATED_BYTES.fetch_add(bytes, Ordering::Relaxed);
    ALLOCATION_COUNT.fetch_add(1, Ordering::Relaxed);
    let live = LIVE_BYTES.fetch_add(bytes, Ordering::Relaxed) + bytes;
    PEAK_BYTES.fetch_max(live, Ordering::Relaxed);
}

fn record_deallocation(bytes: usize) {
    LIVE_BYTES.fetch_sub(bytes, Ordering::Relaxed);
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            record_allocation(layout.size());
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            record_allocation(layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        record_deallocation(layout.size());
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let resized = unsafe { System.realloc(pointer, layout, new_size) };
        if !resized.is_null() {
            if new_size >= layout.size() {
                record_allocation(new_size - layout.size());
            } else {
                record_deallocation(layout.size() - new_size);
            }
        }
        resized
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    let scenario = args.get(1).map(String::as_str).unwrap_or("mixed");
    let iterations = parse_arg(&args, 2, 30usize);
    let width = parse_arg(&args, 3, 1920usize);
    let height = parse_arg(&args, 4, 1080usize);
    assert!(iterations > 0 && width > 0 && height > 0);

    let scene = build_scene(width, height, scenario);
    let mut buffer = vec![0u32; width.saturating_mul(height)];

    let cold_live_before = LIVE_BYTES.load(Ordering::Relaxed);
    PEAK_BYTES.store(cold_live_before, Ordering::Relaxed);
    render_to_buffer(&scene, &mut buffer, width, height, Color::rgb(5, 8, 14));
    let cold_live_after = LIVE_BYTES.load(Ordering::Relaxed);
    let cold_peak = PEAK_BYTES.load(Ordering::Relaxed);

    for _ in 0..8 {
        render_to_buffer(&scene, &mut buffer, width, height, Color::rgb(5, 8, 14));
    }

    let timed_live_before = LIVE_BYTES.load(Ordering::Relaxed);
    PEAK_BYTES.store(timed_live_before, Ordering::Relaxed);
    ALLOCATED_BYTES.store(0, Ordering::Relaxed);
    ALLOCATION_COUNT.store(0, Ordering::Relaxed);
    let mut samples_ns = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let started = Instant::now();
        render_to_buffer(&scene, &mut buffer, width, height, Color::rgb(5, 8, 14));
        samples_ns.push(started.elapsed().as_nanos() as u64);
    }
    let timed_live_after = LIVE_BYTES.load(Ordering::Relaxed);
    let timed_peak = PEAK_BYTES.load(Ordering::Relaxed);

    samples_ns.sort_unstable();
    let median_ns = percentile(&samples_ns, 50);
    let p95_ns = percentile(&samples_ns, 95);
    let checksum = buffer
        .iter()
        .step_by((buffer.len() / 4096).max(1))
        .fold(0u64, |sum, &pixel| sum.wrapping_add(u64::from(pixel)));
    black_box(checksum);

    println!(
        "scenario={scenario} size={width}x{height} iterations={iterations} median_us={} p95_us={} min_us={} cold_peak_extra_bytes={} cold_retained_bytes={} timed_peak_extra_bytes={} timed_retained_bytes={} allocated_bytes={} allocations={} checksum={checksum}",
        median_ns / 1_000,
        p95_ns / 1_000,
        samples_ns[0] / 1_000,
        cold_peak.saturating_sub(cold_live_before),
        signed_difference(cold_live_after, cold_live_before),
        timed_peak.saturating_sub(timed_live_before),
        signed_difference(timed_live_after, timed_live_before),
        ALLOCATED_BYTES.load(Ordering::Relaxed),
        ALLOCATION_COUNT.load(Ordering::Relaxed),
    );
}

fn parse_arg(args: &[String], index: usize, default: usize) -> usize {
    args.get(index)
        .map(|value| value.parse().expect("numeric benchmark argument expected"))
        .unwrap_or(default)
}

fn percentile(sorted: &[u64], percentile: usize) -> u64 {
    let index = (sorted.len() - 1).saturating_mul(percentile) / 100;
    sorted[index]
}

fn signed_difference(after: usize, before: usize) -> i128 {
    after as i128 - before as i128
}

fn build_scene(width: usize, height: usize, scenario: &str) -> Vec<RenderNode> {
    let mut root = RenderNode::container(LayoutBox::new(0.0, 0.0, width as f32, height as f32));
    root.style.background = Some(Color::rgb(12, 18, 30));

    if scenario == "gradient" {
        // At the default 1920x1080 size this raster is larger than the
        // renderer's single-static-gradient cache limit. Keeping it uncached
        // makes every timed frame exercise direct gradient sampling and shape
        // coverage rather than measuring only the cached-layer blit path.
        root.style.background_layers = vec![BackgroundLayer::LinearGradient(LinearGradient {
            direction: GradientDirection::Angle(135.0),
            interpolation: GradientInterpolation::Oklab,
            repeating: false,
            stops: vec![
                GradientStop {
                    color: Color::rgb(15, 23, 42),
                    position: LengthPercentageValue::from_fraction(0.0),
                },
                GradientStop {
                    color: Color::rgb(14, 165, 233),
                    position: LengthPercentageValue::from_fraction(0.32),
                },
                GradientStop {
                    color: Color::rgb(168, 85, 247),
                    position: LengthPercentageValue::from_fraction(0.68),
                },
                GradientStop {
                    color: Color::rgb(244, 63, 94),
                    position: LengthPercentageValue::from_fraction(1.0),
                },
            ],
        })];
        return vec![root];
    }

    if scenario == "hairline" {
        let line_count = (height / 2).clamp(1, 512);
        let line_spacing = height as f32 / line_count as f32;
        for line in 0..line_count {
            let mut hairline = RenderNode::container(LayoutBox::new(
                0.35,
                line as f32 * line_spacing + 0.35,
                (width as f32 - 0.7).max(0.05),
                0.45,
            ));
            hairline.style.background = Some(Color::rgba(
                32 + (line % 5) as u8 * 22,
                72 + (line % 7) as u8 * 11,
                112 + (line % 4) as u8 * 28,
                192,
            ));
            hairline.style.corner_radius = CornerRadius::all(0.225);
            root.children.push(hairline);
        }
        return vec![root];
    }

    let columns = 24usize;
    let rows = 14usize;
    let cell_width = width as f32 / columns as f32;
    let cell_height = height as f32 / rows as f32;
    for row in 0..rows {
        for column in 0..columns {
            let index = row * columns + column;
            let mut tile = RenderNode::container(LayoutBox::new(
                column as f32 * cell_width + 2.0,
                row as f32 * cell_height + 2.0,
                cell_width + 8.0,
                cell_height + 8.0,
            ));
            tile.style.background = Some(if scenario == "opaque" {
                Color::rgb(
                    24 + (index % 5) as u8 * 18,
                    52 + (index % 7) as u8 * 12,
                    96 + (index % 4) as u8 * 24,
                )
            } else {
                Color::rgba(
                    32 + (index % 5) as u8 * 22,
                    72 + (index % 7) as u8 * 11,
                    112 + (index % 4) as u8 * 28,
                    168 + (index % 3) as u8 * 24,
                )
            });
            if scenario == "rounded" {
                tile.style.corner_radius = CornerRadius::all(18.0);
            }
            root.children.push(tile);
        }
    }

    vec![root]
}
