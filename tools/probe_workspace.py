"""Bounded real-PTY resource probe; measurements are host-specific, not fleet proof."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import select
import statistics
import subprocess
import time

from test_native_cli import BINARY, Terminal


def cpu_seconds(value):
    parts = value.split(":")
    return sum(float(part) * 60 ** index for index, part in enumerate(reversed(parts)))


def sample(child):
    row = subprocess.check_output(
        ["ps", "-p", str(child.pid), "-o", "time=,rss="], text=True
    ).split()
    return {"cpu_seconds": cpu_seconds(row[0]), "rss_kib": int(row[1])}


def drain(terminal, seconds):
    deadline = time.monotonic() + seconds
    count = 0
    while time.monotonic() < deadline:
        if terminal.child.poll() is not None:
            raise RuntimeError("workspace process exited during the idle probe")
        if select.select([terminal.master], [], [], min(1, max(0, deadline - time.monotonic())))[0]:
            chunk = os.read(terminal.master, 65536)
            count += len(chunk)
            if count > 1024 * 1024:
                raise RuntimeError("idle output exceeded the 1 MiB bound")
    return count


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--seconds", type=int, default=300)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if not 5 <= args.seconds <= 300:
        parser.error("seconds must be between 5 and 300")
    started = time.monotonic()
    terminal = Terminal()
    try:
        terminal.expect("Everything else stays quiet")
        launch_ms = (time.monotonic() - started) * 1000
        drain(terminal, 0.1)
        latencies = []
        for _ in range(20):
            started = time.monotonic()
            terminal.send("\r")
            terminal.expect("Awaiting Moo")
            latencies.append((time.monotonic() - started) * 1000)
            drain(terminal, 0.05)
            terminal.send("\x1b")
            terminal.expect("Everything else stays quiet")
            drain(terminal, 0.05)
        initial = sample(terminal.child)
        samples = [initial]
        idle_bytes = 0
        idle_started = time.monotonic()
        for elapsed in range(0, args.seconds, 30):
            idle_bytes += drain(terminal, min(30, args.seconds - elapsed))
            samples.append(sample(terminal.child))
            print(f"idle sample {min(elapsed + 30, args.seconds)}/{args.seconds}s: {samples[-1]}", flush=True)
        idle_duration = time.monotonic() - idle_started
        started = time.monotonic()
        terminal.send("q")
        exit_code = terminal.finish()
        close_ms = (time.monotonic() - started) * 1000
        result = {
            "scope": "macOS actual debug binary, 120x32 PTY, synthetic data, disconnected Moo/model",
            "binary_sha256": hashlib.sha256(BINARY.read_bytes()).hexdigest(),
            "first_paint_ms": launch_ms,
            "input_samples_ms": latencies,
            "input_p50_ms": statistics.median(latencies),
            "input_p95_ms": sorted(latencies)[18],
            "idle_seconds": idle_duration,
            "idle_cpu_percent": (samples[-1]["cpu_seconds"] - initial["cpu_seconds"]) / idle_duration * 100,
            "sampled_max_rss_kib": max(row["rss_kib"] for row in samples),
            "idle_output_bytes": idle_bytes,
            "resource_samples": samples,
            "quit_to_exit_ms": close_ms,
            "exit_code": exit_code,
            "child_reaped": terminal.child.poll() is not None,
            "unknown": ["physical Ryzen3", "other OS/architecture", "peak RSS between samples",
                        "main-thread task duration", "network trace", "fleet load", "human acceptance", "matched alternatives"],
        }
        args.output.parent.mkdir(parents=True, exist_ok=True)
        with args.output.open("x", encoding="utf-8") as target:
            json.dump(result, target, indent=2)
            target.write("\n")
        print(json.dumps(result), flush=True)
        return exit_code
    finally:
        terminal.close()


if __name__ == "__main__":
    raise SystemExit(main())
