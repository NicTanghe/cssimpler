#!/usr/bin/env python3
"""Rebuild and rerun the PR #9 CPU benchmark in isolated temporary worktrees.

Usage: python3 docs/reports/pr9-paint-refresh-run.py --output /tmp/pr9-results.json
Two separate target directories prevent Cargo from reusing the wrong revision.
The toolchain and host may differ from the recorded report.
"""
import argparse
import json
import platform
import shutil
import subprocess
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
HARNESS = ROOT / "docs/reports/pr9-paint-refresh-benchmark.rs"
REVISIONS = {
    "before": "34472cb33a10debc900b942226b776599cf6ff2c",
    "after": "ac4c2eec6a43cd346138a7ef1d4e120b16eee439",
}


def command(*args, cwd=ROOT, **kwargs):
    return subprocess.run(args, cwd=cwd, check=True, **kwargs)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    result = {
        "revisions": REVISIONS,
        "machine": platform.platform(),
        "rust": command("rustc", "--version", text=True, capture_output=True).stdout.strip(),
        "warmup_frames": 32,
        "measured_frames": 240,
        "runs_per_case": 5,
        "results": [],
    }
    with tempfile.TemporaryDirectory(prefix="cssimpler-pr9-") as temporary:
        root = Path(temporary)
        worktrees = []
        try:
            for name, revision in REVISIONS.items():
                tree = root / name
                command("git", "worktree", "add", "--detach", str(tree), revision)
                worktrees.append(tree)
                shutil.copy2(HARNESS, tree / "examples/pr9_paint_refresh_bench.rs")
                command("cargo", "build", "--release", "--locked", "-j", "2", "--example", "pr9_paint_refresh_bench", cwd=tree)
            executable = "pr9_paint_refresh_bench.exe" if platform.system() == "Windows" else "pr9_paint_refresh_bench"
            for nodes in (100, 500):
                for run in range(5):
                    order = ("before", "after") if run % 2 == 0 else ("after", "before")
                    for name in order:
                        output = command(str(root / name / "target/release/examples" / executable), str(nodes), text=True, capture_output=True).stdout
                        for line in output.splitlines():
                            row = json.loads(line)
                            row.update(run=run + 1, revision=name)
                            result["results"].append(row)
                        print(f"completed {nodes} nodes, run {run + 1}, {name}", flush=True)
                        time.sleep(0.2)
        finally:
            for tree in reversed(worktrees):
                command("git", "worktree", "remove", "--force", str(tree))
    args.output.write_text(json.dumps(result, indent=2) + "\n")


if __name__ == "__main__":
    main()
