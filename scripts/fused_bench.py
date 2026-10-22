#!/usr/bin/env python3
"""Interleave prebuilt FusedIntersect candidates with --no-decomp.

Example: python3 scripts/fused_bench.py --binary base=/tmp/base \
    --binary candidate=/tmp/candidate --output /tmp/results.json
The default inputs are the BENCHMARKS literal in the local bench.py.
"""

import argparse
import ast
import hashlib
import json
import os
from pathlib import Path
import statistics
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", action="append", required=True, metavar="LABEL=PATH")
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--warmups", type=int, default=1)
    parser.add_argument("--timeout", type=float, default=300)
    parser.add_argument("--threads", type=int, default=1)
    parser.add_argument("--cpu", type=int)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("inputs", nargs="*", type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    binaries = {label: str(Path(path).resolve()) for label, path in
                (entry.split("=", 1) for entry in args.binary)}
    inputs = args.inputs
    if not inputs:
        tree = ast.parse((root / "bench.py").read_text())
        names = next(ast.literal_eval(node.value) for node in tree.body
                     if isinstance(node, ast.Assign)
                     and any(isinstance(t, ast.Name) and t.id == "BENCHMARKS"
                             for t in node.targets))
        inputs = [root / "tests" / name for name in names]
    if args.cpu is not None:
        os.sched_setaffinity(0, {args.cpu})
    result = {
        "binaries": {label: {"path": path, "sha256": hashlib.sha256(Path(path).read_bytes()).hexdigest()}
                     for label, path in binaries.items()},
        "options": {key: str(value) if isinstance(value, Path) else value
                    for key, value in vars(args).items() if key != "inputs"},
        "environment": {key: value for key, value in os.environ.items()
                        if key.startswith(("EGGLOG_", "RAYON_"))},
        "affinity": sorted(os.sched_getaffinity(0)),
        "benchmarks": {},
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    for src in inputs:
        records = result["benchmarks"][str(src)] = {label: [] for label in binaries}
        for iteration in range(-args.warmups, args.runs):
            order = list(binaries)
            if iteration % 2:
                order.reverse()
            for label in order:
                command = [binaries[label], "--no-decomp", "-j", str(args.threads), str(src.resolve())]
                started = time.perf_counter()
                try:
                    proc = subprocess.run(command, cwd=root, capture_output=True, timeout=args.timeout)
                    record = {"seconds": time.perf_counter() - started, "returncode": proc.returncode,
                              "stdout_sha256": hashlib.sha256(proc.stdout).hexdigest()}
                    if proc.returncode:
                        record["stderr"] = proc.stderr.decode(errors="replace")[-4000:]
                except subprocess.TimeoutExpired:
                    record = {"seconds": time.perf_counter() - started, "returncode": "timeout"}
                if iteration >= 0 or record["returncode"] != 0:
                    records[label].append(record)
                args.output.write_text(json.dumps(result, indent=2) + "\n")
                if record["returncode"] != 0:
                    raise SystemExit(f"{label} failed on {src}: {record}")
        means = {label: statistics.mean(r["seconds"] for r in rows) for label, rows in records.items()}
        baseline = next(iter(means.values()))
        print(src.name + ": " + "; ".join(
            f"{label} {mean:.4f}s ({(mean / baseline - 1) * 100:+.2f}%)"
            for label, mean in means.items()), flush=True)


if __name__ == "__main__":
    main()
