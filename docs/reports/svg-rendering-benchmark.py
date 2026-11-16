#!/usr/bin/env python3
"""Prepare the portable baseline example or compare isolated renderer processes."""
import argparse
import hashlib
import json
import os
import platform
import statistics
import subprocess
from pathlib import Path


def prepare(source, destination):
    # Both revisions compile the exact same benchmark source.
    Path(destination).parent.mkdir(parents=True, exist_ok=True)
    Path(destination).write_bytes(Path(source).read_bytes())


def compare(args):
    records = []
    modes = [("original", args.baseline), ("tiny-skia", args.current)]
    for case in ["icons", "panels"]:
        for run in range(args.runs):
            # Rotate ordering to avoid always favoring the same implementation.
            for mode, executable in modes[run % len(modes):] + modes[:run % len(modes)]:
                command = [str(Path(executable).resolve()), "--case", case, "--frames", str(args.frames)]
                result = json.loads(subprocess.check_output(command, text=True))
                records.append({"mode": mode, "run": run + 1, **result})
                print(f'{case} {mode} run {run + 1}: {result["median_ms"]:.3f} ms, peak RSS {result["peak_rss_kib"]} KiB', flush=True)
    summary = []
    for case in ["icons", "panels"]:
        for mode, _ in modes:
            group = [r for r in records if r["case"] == case and r["mode"] == mode]
            row = {"case": case, "mode": mode}
            for key in ["warmup_ms", "median_ms", "p95_ms", "rss_before_kib", "rss_warm_kib", "rss_after_kib", "peak_rss_kib"]:
                values = [r[key] for r in group if r[key] is not None]
                row[key] = statistics.median(values) if values else None
            summary.append(row)
    cpuinfo = Path("/proc/cpuinfo")
    cpu = next((line.split(":", 1)[1].strip() for line in cpuinfo.read_text().splitlines() if line.startswith("model name")), "unknown") if cpuinfo.exists() else platform.processor()
    quota = Path("/sys/fs/cgroup/cpu.max")
    metadata = {
        "platform": platform.platform(), "cpu": cpu, "logical_cpus": os.cpu_count(),
        "cgroup_cpu_max": quota.read_text().strip() if quota.exists() else None,
        "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(),
        "baseline_commit": args.baseline_commit, "runs": args.runs, "frames_per_run": args.frames,
        "benchmark_sha256": hashlib.sha256(Path(args.source).read_bytes()).hexdigest(),
        "method": "Fresh process per run; one cold paint then timed full repaints; summaries are medians of per-run values; Linux VmRSS/VmHWM in KiB.",
        "layer_caching": "Off in both revisions; ordinary static SVG repaint only.",
        "build": "Cargo release profile, default features, repository x86 SSE4.1/SSE4.2 rustflags; same toolchain and geometry for both revisions.",
    }
    Path(args.output).write_text(json.dumps({"metadata": metadata, "summary": summary, "runs": records}, indent=2) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    prep = sub.add_parser("prepare-baseline")
    prep.add_argument("source")
    prep.add_argument("destination")
    run = sub.add_parser("compare")
    run.add_argument("--baseline", required=True)
    run.add_argument("--current", required=True)
    run.add_argument("--baseline-commit", required=True)
    run.add_argument("--source", default="renderer/examples/svg_raster_bench.rs")
    run.add_argument("--runs", type=int, default=3)
    run.add_argument("--frames", type=int, default=12)
    run.add_argument("--output", required=True)
    args = parser.parse_args()
    if args.command == "prepare-baseline":
        prepare(args.source, args.destination)
    else:
        compare(args)
