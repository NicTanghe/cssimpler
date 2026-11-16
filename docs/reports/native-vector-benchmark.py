#!/usr/bin/env python3
"""Compare static native/SVG construction, preparation, paint, and process RSS."""
import argparse
import hashlib
import json
import platform
import statistics
import subprocess
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("executable")
parser.add_argument("--output", default="docs/reports/native-vector-results.json")
args = parser.parse_args()
records = []
for run in range(3):
    for mode in (["native", "svg"] if run % 2 == 0 else ["svg", "native"]):
        result = json.loads(subprocess.check_output([str(Path(args.executable).resolve()), "--mode", mode, "--frames", "20"], text=True))
        records.append({"run": run + 1, **result})
        print(json.dumps(records[-1]), flush=True)
summary = []
for mode in ["svg", "native"]:
    group = [r for r in records if r["mode"] == mode]
    row = {"mode": mode}
    for key in ["construct_ms", "prepare_ms", "paint_ms", "rss_before_paint_kib", "rss_after_paint_kib", "peak_rss_kib"]:
        values = [r[key] for r in group if r[key] is not None]
        row[key] = statistics.median(values) if values else None
    summary.append(row)
quota = Path("/sys/fs/cgroup/cpu.max")
metadata = {
    "platform": platform.platform(),
    "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(),
    "cgroup_cpu_max": quota.read_text().strip() if quota.exists() else None,
    "example_sha256": hashlib.sha256(Path("examples/native_vector_bench.rs").read_bytes()).hexdigest(),
    "method": "Three fresh processes per mode, alternating order; 20 iterations per stage; medians of per-process medians; same authored curves; no layer requests; peak RSS includes all stages.",
    "build": "Cargo release profile, default features, repository SSE4.1/SSE4.2 flags on x86.",
}
Path(args.output).write_text(json.dumps({"metadata": metadata, "summary": summary, "runs": records}, indent=2) + "\n")
