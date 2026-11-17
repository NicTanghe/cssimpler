//! Investigation instrumentation for prepared native vector geometry (issue #22).
//!
//! Native vector paint performs several distinct kinds of work per path:
//! converting authored commands into a tiny-skia [`Path`], expanding a stroke
//! into an outline, projecting source coordinates into device space, and
//! rasterizing/blending coverage. Whether a prepared-geometry cache is worth its
//! memory depends on how those costs compare, so this module exposes an opt-in
//! probe that attributes nanoseconds to each stage.
//!
//! The probe is disabled by default. When disabled, each stage costs one relaxed
//! atomic load; no clock is read. When enabled, timings are accumulated by every
//! render worker into process-wide counters, so the totals are aggregate CPU
//! work, not wall time. Compare them with a separately measured frame time and
//! with [`VectorPreparationStats::paths_painted`] before attributing frame-level
//! shares.
//!
//! Diagnostic counters are reported as deltas since [`reset_vector_preparation_stats`].

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;

/// Per-stage clock selection for [`record`]. Kept `pub(crate)`: callers only
/// see the aggregated [`VectorPreparationStats`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stage {
    PathBuild,
    StrokePrep,
    Projection,
    CoverageRaster,
    CoverageBlend,
}

static ENABLED: AtomicBool = AtomicBool::new(false);
static PATHS_PAINTED: AtomicUsize = AtomicUsize::new(0);
static STROKE_OUTLINES: AtomicUsize = AtomicUsize::new(0);
static COVERAGE_TILES: AtomicUsize = AtomicUsize::new(0);
static PATH_BUILD_NS: AtomicU64 = AtomicU64::new(0);
static STROKE_PREP_NS: AtomicU64 = AtomicU64::new(0);
static PROJECTION_NS: AtomicU64 = AtomicU64::new(0);
static COVERAGE_RASTER_NS: AtomicU64 = AtomicU64::new(0);
static COVERAGE_BLEND_NS: AtomicU64 = AtomicU64::new(0);

/// Cumulative native vector work since the last reset.
///
/// `*_ns` values are per-thread sums across all render workers, not wall-clock
/// durations. `paths_painted`, `stroke_outlines` and `coverage_tiles` count the
/// work items those clocks were sampled around.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VectorPreparationStats {
    pub paths_painted: usize,
    pub stroke_outlines: usize,
    pub coverage_tiles: usize,
    pub path_build_ns: u64,
    pub stroke_prep_ns: u64,
    pub projection_ns: u64,
    pub coverage_raster_ns: u64,
    pub coverage_blend_ns: u64,
}

impl VectorPreparationStats {
    /// Sum of the stages that a prepared-geometry cache could remove on a hit
    /// with an unchanged projection: command conversion plus stroke expansion.
    pub fn geometry_prep_ns(&self) -> u64 {
        self.path_build_ns.saturating_add(self.stroke_prep_ns)
    }

    /// Total attributed vector work. Remaining frame time is scene traversal,
    /// clip/bounds checks, paint resolution and presentation.
    pub fn attributed_ns(&self) -> u64 {
        self.geometry_prep_ns()
            .saturating_add(self.projection_ns)
            .saturating_add(self.coverage_raster_ns)
            .saturating_add(self.coverage_blend_ns)
    }
}

/// Enable or disable the per-stage clocks. Prefer toggling between frames.
pub fn set_vector_preparation_probe_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

/// Whether the per-stage clocks currently read `Instant::now()`.
pub fn vector_preparation_probe_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Release all accumulated counters. Call before timing a frame or a loop.
pub fn reset_vector_preparation_stats() {
    for counter in [&PATHS_PAINTED, &STROKE_OUTLINES, &COVERAGE_TILES] {
        counter.store(0, Ordering::Relaxed);
    }
    for counter in [
        &PATH_BUILD_NS,
        &STROKE_PREP_NS,
        &PROJECTION_NS,
        &COVERAGE_RASTER_NS,
        &COVERAGE_BLEND_NS,
    ] {
        counter.store(0, Ordering::Relaxed);
    }
}

/// Read the accumulated counters without resetting them.
pub fn vector_preparation_stats() -> VectorPreparationStats {
    VectorPreparationStats {
        paths_painted: PATHS_PAINTED.load(Ordering::Relaxed),
        stroke_outlines: STROKE_OUTLINES.load(Ordering::Relaxed),
        coverage_tiles: COVERAGE_TILES.load(Ordering::Relaxed),
        path_build_ns: PATH_BUILD_NS.load(Ordering::Relaxed),
        stroke_prep_ns: STROKE_PREP_NS.load(Ordering::Relaxed),
        projection_ns: PROJECTION_NS.load(Ordering::Relaxed),
        coverage_raster_ns: COVERAGE_RASTER_NS.load(Ordering::Relaxed),
        coverage_blend_ns: COVERAGE_BLEND_NS.load(Ordering::Relaxed),
    }
}

/// Start a stage clock when the probe is enabled.
#[inline]
pub(crate) fn stage_start() -> Option<Instant> {
    ENABLED.load(Ordering::Relaxed).then(Instant::now)
}

/// Stop a stage clock started by [`stage_start`] and accumulate its duration.
#[inline]
pub(crate) fn record(stage: Stage, start: Option<Instant>) {
    if let Some(start) = start {
        add_time(stage_counter(stage), elapsed_ns(start.elapsed()));
    }
}

/// Count one path that reached path construction.
#[inline]
pub(crate) fn count_path() {
    if ENABLED.load(Ordering::Relaxed) {
        add(&PATHS_PAINTED, 1);
    }
}

/// Count one stroke outline produced by the stroker.
#[inline]
pub(crate) fn count_stroke_outline() {
    if ENABLED.load(Ordering::Relaxed) {
        add(&STROKE_OUTLINES, 1);
    }
}

/// Count one coverage tile that was rasterized and blended.
#[inline]
pub(crate) fn count_coverage_tile() {
    if ENABLED.load(Ordering::Relaxed) {
        add(&COVERAGE_TILES, 1);
    }
}

fn stage_counter(stage: Stage) -> &'static AtomicU64 {
    match stage {
        Stage::PathBuild => &PATH_BUILD_NS,
        Stage::StrokePrep => &STROKE_PREP_NS,
        Stage::Projection => &PROJECTION_NS,
        Stage::CoverageRaster => &COVERAGE_RASTER_NS,
        Stage::CoverageBlend => &COVERAGE_BLEND_NS,
    }
}

fn elapsed_ns(elapsed: std::time::Duration) -> u64 {
    elapsed.as_nanos().min(u64::MAX as u128) as u64
}

// Saturating so counters cannot wrap during very long measurement loops.
// `fetch_update` is deprecated in favour of `try_update` on newer nightlies but
// remains available on older supported toolchains; keep it for compatibility.
#[allow(deprecated)]
fn add(counter: &AtomicUsize, value: usize) {
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |old| {
        Some(old.saturating_add(value))
    });
}

#[allow(deprecated)]
fn add_time(counter: &AtomicU64, value: u64) {
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |old| {
        Some(old.saturating_add(value))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn probe_is_off_by_default_and_stages_stay_silent() {
        let _guard = TEST_LOCK.lock().unwrap();
        set_vector_preparation_probe_enabled(false);
        reset_vector_preparation_stats();
        let start = stage_start();
        assert!(start.is_none(), "disabled probe must not read the clock");
        record(Stage::PathBuild, start);
        count_path();
        assert_eq!(
            vector_preparation_stats(),
            VectorPreparationStats::default()
        );
    }

    #[test]
    fn enabled_probe_accumulates_every_stage_and_resets() {
        let _guard = TEST_LOCK.lock().unwrap();
        set_vector_preparation_probe_enabled(true);
        reset_vector_preparation_stats();
        for stage in [
            Stage::PathBuild,
            Stage::StrokePrep,
            Stage::Projection,
            Stage::CoverageRaster,
            Stage::CoverageBlend,
        ] {
            record(stage, stage_start());
        }
        count_path();
        count_stroke_outline();
        count_coverage_tile();
        let stats = vector_preparation_stats();
        // Other tests may paint concurrently while the probe is enabled, so
        // assert the work this test contributed rather than an exact total.
        assert!(stats.paths_painted >= 1);
        assert!(stats.stroke_outlines >= 1);
        assert!(stats.coverage_tiles >= 1);
        // Clocks measure real work; with the probe enabled each recorded stage
        // is a non-negative duration. Only assert the sum is coherent.
        assert_eq!(
            stats.attributed_ns(),
            stats
                .path_build_ns
                .saturating_add(stats.stroke_prep_ns)
                .saturating_add(stats.projection_ns)
                .saturating_add(stats.coverage_raster_ns)
                .saturating_add(stats.coverage_blend_ns)
        );
        // Disable before the final reset so a concurrent paint test cannot add
        // work between the reset and the assertion.
        set_vector_preparation_probe_enabled(false);
        reset_vector_preparation_stats();
        assert_eq!(
            vector_preparation_stats(),
            VectorPreparationStats::default()
        );
    }

    #[test]
    fn concurrent_workers_accumulate_without_loss() {
        let _guard = TEST_LOCK.lock().unwrap();
        set_vector_preparation_probe_enabled(true);
        reset_vector_preparation_stats();
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    for _ in 0..500 {
                        count_path();
                        record(Stage::CoverageBlend, stage_start());
                    }
                });
            }
        });
        assert!(vector_preparation_stats().paths_painted >= 2000);
        set_vector_preparation_probe_enabled(false);
        reset_vector_preparation_stats();
    }
}
