#!/usr/bin/env python3
"""Launch time and memory of database apps on this Mac, measured the same way for each.

    swiftc -O dev/bench/winwait.swift -o /tmp/winwait
    python3 dev/bench/apps.py /tmp/winwait out.json "Kiyi=/Applications/Kiyi.app"

For every app: one warm-up launch (first-run setup, Gatekeeper checks), then RUNS launches.
 - Launch time: from starting the binary to its first on-screen window of at least 700x440
   points, so splash screens don't count.
 - Memory: after IDLE seconds with no connection open, the physical footprint (what Activity
   Monitor shows as "Memory") summed over the app and every process working for it: child
   processes (Electron helpers, JVM) and WebKit processes started on its behalf (Tauri).
 - Size: the installed .app bundle.
"""
import json
import os
import re
import signal
import statistics
import subprocess
import sys
import time

RUNS = 3
IDLE = 15


def ps():
    out = subprocess.run(["ps", "-axo", "pid=,ppid=,comm="], capture_output=True, text=True).stdout
    procs = {}
    for line in out.splitlines():
        pid, ppid, comm = line.strip().split(None, 2)
        procs[int(pid)] = (int(ppid), comm)
    return procs


def family(root, before):
    """The app, its descendants, and WebKit/XPC helpers that appeared since launch."""
    procs = ps()
    pids = {root}
    changed = True
    while changed:
        changed = False
        for pid, (ppid, _) in procs.items():
            if ppid in pids and pid not in pids:
                pids.add(pid)
                changed = True
    for pid, (_, comm) in procs.items():
        if pid not in before and "com.apple.WebKit" in comm:
            pids.add(pid)
    return [p for p in pids if p in procs]


def footprint_mb(pid):
    out = subprocess.run(["footprint", "-p", str(pid)], capture_output=True, text=True).stdout
    m = re.search(r"Footprint: ([\d.]+) (KB|MB|GB)", out)
    if not m:
        return 0.0
    n, unit = float(m.group(1)), m.group(2)
    return n / 1024 if unit == "KB" else n * 1024 if unit == "GB" else n


def stop(pids):
    for p in pids:
        try:
            os.kill(p, signal.SIGTERM)
        except ProcessLookupError:
            pass
    time.sleep(3)
    for p in pids:
        try:
            os.kill(p, signal.SIGKILL)
        except ProcessLookupError:
            pass
    time.sleep(2)


def executable(app):
    name = subprocess.run(["defaults", "read", f"{app}/Contents/Info", "CFBundleExecutable"], capture_output=True, text=True).stdout.strip()
    return f"{app}/Contents/MacOS/{name}"


def launch(winwait, app, measure):
    before = set(ps())
    out = subprocess.run([winwait, executable(app)], capture_output=True, text=True).stdout.split()
    pid, secs = int(out[0]), out[1]
    mem, procs = None, None
    if measure and secs != "timeout":
        time.sleep(IDLE)
        fam = family(pid, before)
        procs = len(fam)
        mem = sum(footprint_mb(p) for p in fam)
    stop(family(pid, before))
    return (None if secs == "timeout" else float(secs)), mem, procs


def main():
    winwait, out = sys.argv[1], sys.argv[2]
    results = []
    for spec in sys.argv[3:]:
        name, app = spec.split("=", 1)
        version = subprocess.run(["defaults", "read", f"{app}/Contents/Info", "CFBundleShortVersionString"], capture_output=True, text=True).stdout.strip()
        size = int(subprocess.run(["du", "-sk", app], capture_output=True, text=True).stdout.split()[0]) / 1024
        launch(winwait, app, measure=False)  # warm-up
        starts, mems, procs = [], [], 0
        for _ in range(RUNS):
            s, m, n = launch(winwait, app, measure=True)
            if s is not None:
                starts.append(s)
            if m is not None:
                mems.append(m)
                procs = n
        r = {
            "app": name,
            "version": version,
            "launchSec": round(statistics.median(starts), 2) if starts else None,
            "memoryMb": round(statistics.median(mems)) if mems else None,
            "processes": procs,
            "sizeMb": round(size),
        }
        print(json.dumps(r), flush=True)
        results.append(r)
    with open(out, "w") as f:
        json.dump(results, f, indent=2)


if __name__ == "__main__":
    main()
