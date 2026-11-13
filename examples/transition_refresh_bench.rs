//! CPU-only scene preparation benchmark for cssimpler PR #9.
use cssimpler::app::{App, Invalidation, Refresh};
use cssimpler::core::Node;
use cssimpler::renderer::FrameInfo;
use cssimpler::style::parse_stylesheet;
use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

struct Count;
static CALLS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for Count {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            CALLS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        unsafe { System.dealloc(p, layout) }
    }
    unsafe fn realloc(&self, p: *mut u8, layout: Layout, n: usize) -> *mut u8 {
        let r = unsafe { System.realloc(p, layout, n) };
        if !r.is_null() {
            CALLS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(n, Ordering::Relaxed);
        }
        r
    }
}
#[global_allocator]
static ALLOCATOR: Count = Count;
#[derive(Default)]
struct State {
    hot: bool,
    count: u64,
}
fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .unwrap_or("100".into())
        .parse()
        .unwrap();
    let frames = 240u64;
    for mode in ["idle", "full", "partial"] {
        let sheet = parse_stylesheet("#root { width: 1100px; height: 900px; position: relative; } #thumb { position: absolute; left: 3px; width: 16px; height: 16px; transition: left 400ms linear; } #root.hot #thumb { left: 27px; } .item { width: 12px; height: 2px; background-color: #598; }").unwrap();
        let mut app = App::new(
            State::default(),
            &sheet,
            move |state: &mut State, frame: FrameInfo| -> Refresh {
                if frame.frame_index == 0 {
                    return Refresh::clean();
                }
                state.count += 1;
                if mode != "idle" && frame.frame_index % 32 == 1 {
                    state.hot = !state.hot;
                    return Invalidation::Layout.into();
                }
                if mode == "partial" {
                    Refresh::fragment("status", Invalidation::Paint)
                } else {
                    Invalidation::Paint.into()
                }
            },
            move |state: &State| -> Node {
                let mut root = Node::element("div").with_id("root");
                if state.hot {
                    root = root.with_class("hot");
                }
                root = root
                    .with_child(Node::element("div").with_id("thumb").into())
                    .with_child(
                        Node::element("span")
                            .with_id("status")
                            .with_child(Node::text(format!("{}", state.count)))
                            .into(),
                    );
                for _ in 0..n {
                    root = root.with_child(Node::element("div").with_class("item").into());
                }
                root.into()
            },
        );
        app.frame(FrameInfo {
            frame_index: 0,
            delta: Duration::ZERO,
        });
        let mut times = Vec::with_capacity(frames as usize);
        let mut allocs = 0usize;
        let mut bytes = 0usize;
        let mut reached = 0u64;
        let mut checks = 0u64;
        for i in 1..=(frames + 32) {
            let before_c = CALLS.load(Ordering::Relaxed);
            let before_b = BYTES.load(Ordering::Relaxed);
            let start = Instant::now();
            black_box(app.frame(FrameInfo {
                frame_index: i,
                delta: Duration::from_millis(16),
            }));
            let elapsed = start.elapsed();
            if i > 32 {
                times.push(elapsed.as_nanos() as u64);
                allocs += CALLS.load(Ordering::Relaxed) - before_c;
                bytes += BYTES.load(Ordering::Relaxed) - before_b;
            }
            if mode != "idle" && i > 32 && i % 32 == 31 {
                checks += 1;
                let x = app.find_element_bounds_by_id("#thumb").unwrap().x;
                let expected = if app.state().hot { 27.0 } else { 3.0 };
                if (x - expected).abs() < 0.01 {
                    reached += 1;
                }
            }
        }
        times.sort_unstable();
        println!(
            "{{\"mode\":\"{}\",\"nodes\":{},\"frames\":{},\"median_us\":{:.3},\"p95_us\":{:.3},\"alloc_calls_per_frame\":{:.1},\"alloc_bytes_per_frame\":{:.1},\"endpoints_reached\":{},\"endpoint_checks\":{}}}",
            mode,
            n,
            frames,
            times[times.len() / 2] as f64 / 1000.0,
            times[(times.len() * 95).div_ceil(100) - 1] as f64 / 1000.0,
            allocs as f64 / frames as f64,
            bytes as f64 / frames as f64,
            reached,
            checks
        );
    }
}
