#!/usr/bin/env python3
"""Check real sensor bounds and verify interval CPU accounting with a bounded load."""
import json
from pathlib import Path
import subprocess

root = Path(__file__).resolve().parents[1]
load = subprocess.Popen(["/usr/bin/yes"], stdout=subprocess.DEVNULL)
try:
    result = subprocess.run([str(root / "build/msutop-mac"), "--probe", "4"], text=True, capture_output=True, check=True, timeout=15)
    samples = [json.loads(line) for line in result.stdout.splitlines()]
    for sample in samples[1:]:
        r = sample["reading"]
        assert r["cpuAvailable"] and 0 <= r["cpuUser"] + r["cpuSystem"] <= 100
        assert len(r["cores"]) > 0 and all(0 <= c <= 100 for c in r["cores"])
        assert r["memoryAvailable"] and 0 < r["memoryUsed"] <= r["memoryTotal"]
        assert r["download"] >= 0 and r["upload"] >= 0
        assert not r["gpuAvailable"] or 0 <= r["gpu"] <= 100
        assert sample["volumes"] and all(0 <= v["used"] <= v["total"] for v in sample["volumes"])
    load_usage = [p["cpu"] for s in samples[1:] for p in s["processes"] if p["pid"] == load.pid]
    assert load_usage and max(load_usage) > 50, f"Single-core CPU accounting is wrong: {load_usage}"
    print(f"PASS: {len(samples)} live samples; {len(samples[-1]['reading']['cores'])} CPU cores; {len(samples[-1]['volumes'])} volumes; single-core load {max(load_usage):.1f}%")
finally:
    load.terminate()
    load.wait(timeout=5)
