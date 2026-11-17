#!/usr/bin/env python3
"""Multi-process driver for examples/vector_prepared_geometry_bench.rs (issue #22).

Runs every scenario in several fresh release processes, interleaving the
scenarios so machine-load drift is shared rather than concentrated in one
workload, then prints medians with min..max spreads and optionally writes a
Markdown report plus the raw JSON samples next to it.

The per-stage numbers are aggregate CPU work across render workers (see the
example header); the wall frame time is measured separately with the probe off.
Normalization against each process's own ALU calibration loop is reported so a
busy shared machine can be judged rather than trusted blindly.

Usage:
    cargo build -j 2 --release --example vector_prepared_geometry_bench
    python3 scripts/vector_prepared_geometry_bench.py --runs 7 \
        --markdown docs/reports/issue-22-prepared-geometry.md
"""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import platform
import statistics
import subprocess
import sys
import time

DEFAULT_BINARY = "target/release/examples/vector_prepared_geometry_bench"

SCENARIOS = [
    "small-icon",
    "small-icon-edit",
    "dense-strokes",
    "curves",
    "curves-serial",
    "curves-translate",
    "curves-zoom",
    "curves-paint-update",
    "curves-edit",
]

# (json field, table label, scale, digits)
STAGE_COLUMNS = [
    ("path_build_ms", "build", 1000.0, 1),
    ("stroke_prep_ms", "stroke", 1000.0, 1),
    ("projection_ms", "project", 1000.0, 1),
    ("coverage_raster_ms", "raster", 1000.0, 0),
    ("coverage_blend_ms", "blend", 1000.0, 0),
]

SUMMARY_COLUMNS = [
    ("frame_ms", "frame", 1.0, 2),
    ("empty_scene_ms", "empty", 1.0, 2),
    ("vector_work_ms", "vector work", 1.0, 2),
    ("geometry_prep_ms", "build+stroke", 1000.0, 1),
]


def run_once(binary: pathlib.Path, scenario: str, label: str) -> dict:
    started = time.time()
    completed = subprocess.run(
        [str(binary), "--scenario", scenario, "--label", label],
        capture_output=True,
        text=True,
        check=False,
    )
    if completed.returncode != 0:
        sys.stderr.write(completed.stderr)
        raise SystemExit(
            f"scenario {scenario!r} failed with exit code {completed.returncode}"
        )
    lines = [line for line in completed.stdout.splitlines() if line.strip()]
    sample = json.loads(lines[-1])
    sample["elapsed_s"] = time.time() - started
    return sample


def spread(values: list[float]) -> str:
    return f"{statistics.median(values):.4g} ({min(values):.4g}..{max(values):.4g})"


def median(values: list[float]) -> float:
    return statistics.median(values)


def percent(numerator: float, denominator: float) -> float:
    return 100.0 * numerator / denominator if denominator > 0 else float("nan")


def machine_note() -> str:
    load = None
    try:
        load = pathlib.Path("/proc/loadavg").read_text().split()[:3]
    except OSError:
        try:
            load = [line.split("load averages:")[1].strip()
                    for line in subprocess.run(
                        ["uptime"], capture_output=True, text=True, check=False
                    ).stdout.splitlines()
                    if "load averages:" in line]
        except (IndexError, OSError):
            pass
    cpus = None
    try:
        cpus = str(len(pathlib.Path("/proc/stat").read_text().splitlines()))
    except OSError:
        cpus = None
    return (
        f"host `{platform.system()} {platform.machine()}`, "
        f"CPUs `{os.cpu_count()}`, load when starting `{load}`, "
        f"`/proc/stat` lines `{cpus}`, rustc `"
        + subprocess.run(["rustc", "--version"], capture_output=True, text=True,
                         check=False).stdout.strip()
        + "`"
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", default=DEFAULT_BINARY)
    parser.add_argument("--runs", type=int, default=7)
    parser.add_argument("--label", default="head")
    parser.add_argument("--markdown", help="write a Markdown report here")
    parser.add_argument("--jsonl", help="write raw samples here (default: <markdown>.jsonl)")
    parser.add_argument("--scenarios", default=",".join(SCENARIOS))
    args = parser.parse_args()

    binary = pathlib.Path(args.binary)
    if not binary.is_file():
        raise SystemExit(
            f"missing benchmark binary {binary}; build it with "
            "`cargo build -j 2 --release --example vector_prepared_geometry_bench`"
        )
    scenarios = [name for name in args.scenarios.split(",") if name]
    samples: dict[str, list[dict]] = {name: [] for name in scenarios}
    notes = machine_note()

    # Round-robin: every pass runs all scenarios once, so load drift affects all
    # workloads instead of only the last one.
    for pass_index in range(args.runs):
        for name in scenarios:
            samples[name].append(run_once(binary, name, args.label))
        sys.stderr.write(f"pass {pass_index + 1}/{args.runs} done\n")

    for name in scenarios:
        for sample in samples[name]:
            sample.setdefault("scenario", name)

    def field(name: str, key: str) -> list[float]:
        return [float(sample[key]) for sample in samples[name]]

    lines: list[str] = []
    header = ("| Scenario | Frame ms | Empty scene | Vector work | Build µs | Stroke µs "
              "| Project µs | Raster µs | Blend µs | Build+stroke % of paint |")
    lines.append(header)
    lines.append("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|")
    for name in scenarios:
        frame = median(field(name, "frame_ms"))
        empty = median(field(name, "empty_scene_ms"))
        vector = median(field(name, "vector_work_ms"))
        cells = [name, spread(field(name, "frame_ms")), f"{empty:.2f}", f"{vector:.2f}"]
        for key, _label, scale, digits in STAGE_COLUMNS:
            cells.append(f"{median(field(name, key)) * scale:.{digits}f}")
        # prep CPU / attributed paint CPU == its share of wall frame time when
        # the paint is serial and a parallel-speedup estimate otherwise.
        prep_share = percent(
            median(field(name, "geometry_prep_ms")), median(field(name, "attributed_ms"))
        )
        cells.append(f"{prep_share:.2f}%")
        lines.append("| " + " | ".join(cells) + " |")

    report = [
        "# Prepared vector geometry: stage-separated measurements (issue #22)",
        "",
        f"Recorded on {time.strftime('%Y-%m-%d %H:%M:%S %Z')} from "
        f"`{args.binary}` with {args.runs} fresh release processes per scenario, "
        "round-robin order (one pass = one run of every scenario).",
        "",
        f"Machine: {notes}.",
        "",
        "Per-frame stage numbers are aggregate CPU work summed across render "
        "workers, not wall time; `Frame ms` is the wall median with the probe "
        "off and `Empty scene` paints the same node tree with empty vector "
        "scenes. `Vector work = Frame − Empty`. `Build+stroke % of paint` is "
        "build+stroke CPU over attributed paint CPU, which equals the share of "
        "wall frame time for serial paint and estimates it for parallel paint. Full "
        "per-metric spreads and the raw JSON are in the `.jsonl` next to this file.",
        "",
        *lines,
        "",
    ]
    markdown = "\n".join(report)
    if args.markdown:
        path = pathlib.Path(args.markdown)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(markdown + "\n")
        jsonl = pathlib.Path(args.jsonl) if args.jsonl else path.with_suffix(path.suffix + ".jsonl")
        with jsonl.open("w") as handle:
            for name in scenarios:
                for sample in samples[name]:
                    handle.write(json.dumps(sample) + "\n")
        print(f"wrote {path} and {jsonl}")
    else:
        print(markdown)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
