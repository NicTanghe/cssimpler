#!/usr/bin/env python3
"""Alternating A/B driver for the issue #17 vector-geometry harnesses.

Usage (from the repository root):

    # Build both revisions first, each with its OWN target directory.
    cargo build --release -p cssimpler --example vector_sharing_bench
    (cd /tmp/cssimpler-issue17-baseline && cargo build --release \
        --target-dir /tmp/cssimpler-issue17-target -p cssimpler \
        --example vector_sharing_bench)

    python3 scripts/vector_sharing_ab.py bench \
        --baseline /tmp/cssimpler-issue17-target/release/examples/vector_sharing_bench \
        --head target/release/examples/vector_sharing_bench --runs 5

    python3 scripts/vector_sharing_ab.py clone \
        --baseline /tmp/cssimpler-issue17-target/release/examples/vector_clone_measure \
        --head target/release/examples/vector_clone_measure --runs 5

Trials alternate baseline/head order every run so slow drift (thermal state,
background load) is spread across both revisions. The script prints the median
and min..max spread across trials, plus the load average observed around the
measurement window. Standard library only; jq is not required.
"""

import argparse
import json
import os
import statistics
import subprocess
import sys
import time

BENCH_SCENARIOS = [
    "small-icon",
    "curves",
    "transform-update",
    "paint-update",
    "repeated-edits",
]
CLONE_SCENARIOS = ["retained-clones"]
# Fields that describe the experiment rather than a measurement.
IGNORED_FIELDS = {
    "scenario",
    "label",
    "frames",
    "width",
    "height",
    "cached_layers",
    "clones",
    "commands",
}


def run_once(binary, scenario, label):
    command = [binary, "--scenario", scenario, "--label", label]
    result = subprocess.run(
        command, capture_output=True, text=True, check=False
    )
    if result.returncode != 0:
        raise RuntimeError(
            f"{' '.join(command)} failed ({result.returncode}):\n{result.stderr}"
        )
    lines = [line for line in result.stdout.splitlines() if line.strip().startswith("{")]
    if not lines:
        raise RuntimeError(
            f"{' '.join(command)} produced no JSON:\n{result.stdout}\n{result.stderr}"
        )
    return json.loads(lines[-1])


def summarize(values):
    return {
        "median": statistics.median(values),
        "min": min(values),
        "max": max(values),
        "n": len(values),
    }


def format_delta(baseline, head):
    if baseline == 0:
        return "n/a"
    return f"{(head - baseline) / baseline * 100.0:+.1f}%"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("kind", choices=["bench", "clone"])
    parser.add_argument("--baseline", required=True)
    parser.add_argument("--head", required=True)
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument(
        "--scenarios",
        default="",
        help="comma-separated subset (default: all for the chosen kind)",
    )
    parser.add_argument(
        "--markdown",
        default="",
        help="optional path to write the markdown summary",
    )
    args = parser.parse_args()

    if args.runs < 1:
        parser.error("--runs must be >= 1")
    scenarios = (
        [item for item in args.scenarios.split(",") if item]
        if args.scenarios
        else (BENCH_SCENARIOS if args.kind == "bench" else CLONE_SCENARIOS)
    )

    load_start = os.getloadavg()
    samples = {scenario: {"baseline": [], "head": []} for scenario in scenarios}
    raw = []
    for run in range(args.runs):
        # Alternate which revision goes first so drift is shared.
        order = [("baseline", args.baseline), ("head", args.head)]
        if run % 2 == 1:
            order.reverse()
        for scenario in scenarios:
            for label, binary in order:
                print(
                    f"run {run + 1}/{args.runs} {scenario} {label}...",
                    file=sys.stderr,
                    flush=True,
                )
                result = run_once(binary, scenario, label)
                samples[scenario][label].append(result)
                raw.append(result)
    load_end = os.getloadavg()

    output = []
    output.append(
        f"# Vector geometry A/B ({args.kind}), {args.runs} alternating runs"
    )
    output.append("")
    output.append(
        f"Machine: {os.cpu_count()} logical CPUs, load average at start "
        f"{load_start[0]:.2f} {load_start[1]:.2f} {load_start[2]:.2f}, "
        f"at end {load_end[0]:.2f} {load_end[1]:.2f} {load_end[2]:.2f}."
    )
    output.append("")
    header = (
        "| scenario | metric | baseline median (min..max) | head median (min..max) "
        "| head vs baseline |"
    )
    output.append(header)
    output.append("|---|---|---|---|---|")
    for scenario in scenarios:
        baseline_samples = samples[scenario]["baseline"]
        head_samples = samples[scenario]["head"]
        fields = []
        for sample in baseline_samples:
            for key in sample:
                if key not in IGNORED_FIELDS and key not in fields:
                    fields.append(key)
        for field in fields:
            baseline_values = [
                sample[field]
                for sample in baseline_samples
                if isinstance(sample.get(field), (int, float))
            ]
            head_values = [
                sample[field]
                for sample in head_samples
                if isinstance(sample.get(field), (int, float))
            ]
            if not baseline_values or not head_values:
                continue
            baseline = summarize(baseline_values)
            head = summarize(head_values)
            output.append(
                f"| {scenario} | {field} "
                f"| {baseline['median']:.4g} ({baseline['min']:.4g}..{baseline['max']:.4g}) "
                f"| {head['median']:.4g} ({head['min']:.4g}..{head['max']:.4g}) "
                f"| {format_delta(baseline['median'], head['median'])} |"
            )
    output.append("")
    text = "\n".join(output)
    print(text)
    if args.markdown:
        with open(args.markdown, "w", encoding="utf-8") as handle:
            handle.write(text + "\n")
    if args.markdown:
        raw_path = args.markdown + ".jsonl"
        with open(raw_path, "w", encoding="utf-8") as handle:
            for sample in raw:
                handle.write(json.dumps(sample, sort_keys=True) + "\n")
        print(f"raw samples: {raw_path}", file=sys.stderr)


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        sys.exit(130)
