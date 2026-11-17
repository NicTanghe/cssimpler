//! Measure picking a large native vector scene in release mode.
use cssimpler::{
    core::{LayoutBox, VectorPathBuilder, VectorPoint as P, VectorScene, VectorViewBox},
    renderer::hit_test_vector_scene,
};
use std::{hint::black_box, time::Instant};

fn main() {
    const SIDE: usize = 100;
    const QUERIES: usize = 2_000;
    let paths = (0..SIDE * SIDE)
        .map(|i| {
            let x = (i % SIDE) as f32 * 10.;
            let y = (i / SIDE) as f32 * 10.;
            VectorPathBuilder::new()
                .move_to(P::new(x + 1., y + 1.))
                .line_to(P::new(x + 8., y + 1.))
                .line_to(P::new(x + 8., y + 8.))
                .line_to(P::new(x + 1., y + 8.))
                .close()
                .build()
                .unwrap()
        })
        .collect();
    let scene = VectorScene::new(VectorViewBox::new(0., 0., 1000., 1000.), paths).unwrap();
    let layout = LayoutBox::new(0., 0., 1000., 1000.);
    for (label, point, expected) in [
        ("first candidate", P::new(994., 994.), Some(9999)),
        ("last candidate", P::new(4., 4.), Some(0)),
        ("no candidate", P::new(9., 9.), None),
    ] {
        assert_eq!(hit_test_vector_scene(&scene, layout, point, 0.), expected);
        let mut samples = Vec::with_capacity(7);
        for _ in 0..7 {
            let start = Instant::now();
            for _ in 0..QUERIES {
                black_box(hit_test_vector_scene(
                    black_box(&scene),
                    layout,
                    black_box(point),
                    0.,
                ));
            }
            samples.push(start.elapsed().as_secs_f64() * 1e6 / QUERIES as f64);
        }
        samples.sort_by(f64::total_cmp);
        println!("{label}: {:.2} µs/query (median)", samples[3]);
    }
}
