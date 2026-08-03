#!/usr/bin/env python3
"""Run a release build with progress reporting and a hard child-process deadline."""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import pathlib
import platform
import re
import signal
import subprocess
import sys
import threading
import time
from dataclasses import dataclass


PROCESS_PATTERN = re.compile(
    r"^(rustc|cargo|clang(?:-cl)?|clang\+\+|lld(?:-link)?|ld(?:\.lld|64)?|link|mold|cc|c\+\+)$",
    re.IGNORECASE,
)
ANSI_ESCAPE = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]")


@dataclass(frozen=True)
class ProcessSnapshot:
    lines: list[str]
    cpu_seconds_by_pid: dict[int, float]
    rss_bytes: int
    cpu_percent: float


def parse_cpu_time(value: str) -> float:
    day_parts = value.split("-", 1)
    days = int(day_parts[0]) if len(day_parts) == 2 else 0
    clock = day_parts[-1].split(":")
    seconds = float(clock[-1])
    minutes = int(clock[-2]) if len(clock) >= 2 else 0
    hours = int(clock[-3]) if len(clock) >= 3 else 0
    return days * 86_400 + hours * 3_600 + minutes * 60 + seconds


def _windows_process_snapshot(root_pid: int) -> ProcessSnapshot:
    script = rf"""
$rootPid = {root_pid}
$processes = @(Get-CimInstance Win32_Process)
$ids = New-Object 'System.Collections.Generic.HashSet[int]'
[void]$ids.Add($rootPid)
do {{
  $added = $false
  foreach ($process in $processes) {{
    if ($ids.Contains([int]$process.ParentProcessId) -and $ids.Add([int]$process.ProcessId)) {{
      $added = $true
    }}
  }}
}} while ($added)
foreach ($process in $processes) {{
  if (-not $ids.Contains([int]$process.ProcessId)) {{ continue }}
  $live = Get-Process -Id $process.ProcessId -ErrorAction SilentlyContinue
  if ($null -eq $live) {{ continue }}
  '{{0}}|{{1}}|{{2}}|{{3}}|{{4}}' -f $process.ProcessId,$process.ParentProcessId,$process.Name,$live.CPU,$live.WorkingSet64
}}
"""
    completed = subprocess.run(
        ["powershell", "-NoProfile", "-Command", script],
        check=False,
        capture_output=True,
        text=True,
        timeout=15,
    )
    lines: list[str] = []
    cpu_seconds_by_pid: dict[int, float] = {}
    rss_bytes = 0
    for raw_line in completed.stdout.splitlines():
        try:
            pid_text, _parent_text, name, cpu_text, rss_text = raw_line.strip().split("|", 4)
            pid = int(pid_text)
            cpu_seconds = float(cpu_text or 0)
            process_rss = int(rss_text)
        except ValueError:
            continue
        cpu_seconds_by_pid[pid] = cpu_seconds
        rss_bytes += process_rss
        if PROCESS_PATTERN.match(pathlib.Path(name).stem) and len(lines) < 12:
            lines.append(
                f"pid={pid} process={name} cpu_seconds={cpu_seconds:.1f} "
                f"rss_mib={process_rss / 1_048_576:.1f}"
            )
    return ProcessSnapshot(
        lines or ["no compiler or linker process visible"],
        cpu_seconds_by_pid,
        rss_bytes,
        0.0,
    )


def _posix_process_snapshot(root_pid: int) -> ProcessSnapshot:
    completed = subprocess.run(
        ["ps", "-axo", "pid=,ppid=,pcpu=,rss=,time=,comm=,args="],
        check=False,
        capture_output=True,
        text=True,
        timeout=15,
    )
    rows: dict[int, tuple[int, float, int, float, str, str]] = {}
    for raw_line in completed.stdout.splitlines():
        fields = raw_line.strip().split(maxsplit=6)
        if len(fields) < 7:
            continue
        try:
            pid = int(fields[0])
            parent_pid = int(fields[1])
            cpu_percent = float(fields[2])
            rss_bytes = int(fields[3]) * 1024
            cpu_seconds = parse_cpu_time(fields[4])
        except ValueError:
            continue
        rows[pid] = (parent_pid, cpu_percent, rss_bytes, cpu_seconds, fields[5], fields[6])

    descendants = {root_pid}
    changed = True
    while changed:
        changed = False
        for pid, (parent_pid, *_rest) in rows.items():
            if parent_pid in descendants and pid not in descendants:
                descendants.add(pid)
                changed = True

    lines: list[str] = []
    cpu_seconds_by_pid: dict[int, float] = {}
    rss_bytes = 0
    cpu_percent = 0.0
    for pid in descendants:
        row = rows.get(pid)
        if row is None:
            continue
        _parent_pid, process_cpu_percent, process_rss, cpu_seconds, command, args = row
        cpu_seconds_by_pid[pid] = cpu_seconds
        rss_bytes += process_rss
        cpu_percent += process_cpu_percent
        if PROCESS_PATTERN.match(pathlib.Path(command).name) and len(lines) < 12:
            lines.append(
                f"pid={pid} cpu_percent={process_cpu_percent:.1f} "
                f"rss_mib={process_rss / 1_048_576:.1f} command={command} args={args}"
            )
    return ProcessSnapshot(
        lines or ["no compiler or linker process visible"],
        cpu_seconds_by_pid,
        rss_bytes,
        cpu_percent,
    )


def active_processes(root_pid: int) -> ProcessSnapshot:
    try:
        if platform.system() == "Windows":
            return _windows_process_snapshot(root_pid)
        return _posix_process_snapshot(root_pid)
    except (OSError, subprocess.TimeoutExpired) as error:
        return ProcessSnapshot([f"process snapshot unavailable: {error}"], {}, 0, 0.0)


def latest_cargo_unit(build_log: pathlib.Path) -> str:
    try:
        with build_log.open("rb") as handle:
            handle.seek(0, 2)
            size = handle.tell()
            handle.seek(max(0, size - 262_144))
            tail = handle.read().decode("utf-8", errors="replace")
    except OSError as error:
        return f"cargo_unit=unavailable error={error}"
    for line in reversed(tail.splitlines()):
        stripped = ANSI_ESCAPE.sub("", line).strip()
        if stripped.startswith(("Compiling ", "Building ", "Finished ")):
            return f"cargo_unit={stripped}"
    return "cargo_unit=not-yet-emitted"


def emit(message: str, output: pathlib.Path) -> None:
    timestamp = dt.datetime.now(dt.timezone.utc).isoformat()
    rendered = f"[{timestamp}] {message}"
    print(rendered, flush=True)
    with output.open("a", encoding="utf-8") as handle:
        handle.write(rendered)
        handle.write("\n")


def tee_output(process: subprocess.Popen[str], build_log: pathlib.Path) -> None:
    assert process.stdout is not None
    with build_log.open("w", encoding="utf-8", errors="replace") as handle:
        for line in process.stdout:
            sys.stdout.write(line)
            sys.stdout.flush()
            handle.write(line)
            handle.flush()


def terminate_process_tree(process: subprocess.Popen[str], output: pathlib.Path) -> None:
    if process.poll() is not None:
        return
    emit(f"terminating build process tree root_pid={process.pid}", output)
    if platform.system() == "Windows":
        try:
            subprocess.run(
                ["taskkill", "/PID", str(process.pid), "/T", "/F"],
                check=False,
                capture_output=True,
                text=True,
                timeout=30,
            )
        except (OSError, subprocess.TimeoutExpired) as error:
            emit(f"Windows process-tree termination failed: {error}", output)
    else:
        try:
            os.killpg(process.pid, signal.SIGTERM)
            process.wait(timeout=10)
            return
        except (OSError, subprocess.TimeoutExpired):
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except OSError:
                pass
    try:
        process.wait(timeout=30)
    except subprocess.TimeoutExpired:
        emit(f"build process tree did not reap root_pid={process.pid}", output)


def write_status(path: pathlib.Path, status: dict[str, object]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(status, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=pathlib.Path, required=True)
    parser.add_argument("--build-log", type=pathlib.Path, required=True)
    parser.add_argument("--status-file", type=pathlib.Path, required=True)
    parser.add_argument("--timeout-seconds", type=int, required=True)
    parser.add_argument("--interval-seconds", type=int, default=60)
    parser.add_argument("--warning-seconds", type=int, default=1_200)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not command:
        parser.error("a command is required after --")
    if args.timeout_seconds <= 0 or args.interval_seconds <= 0:
        parser.error("timeouts and intervals must be positive")

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text("", encoding="utf-8")
    started_wall = dt.datetime.now(dt.timezone.utc)
    started = time.monotonic()
    emit(f"release build wrapper started command={command!r}", args.output)

    popen_kwargs: dict[str, object] = {
        "stdout": subprocess.PIPE,
        "stderr": subprocess.STDOUT,
        "text": True,
        "bufsize": 1,
    }
    if platform.system() == "Windows":
        popen_kwargs["creationflags"] = subprocess.CREATE_NEW_PROCESS_GROUP
    else:
        popen_kwargs["start_new_session"] = True

    try:
        process = subprocess.Popen(command, **popen_kwargs)
    except OSError as error:
        message = f"failed to launch build command: {error}"
        args.build_log.write_text(message + "\n", encoding="utf-8")
        emit(message, args.output)
        write_status(
            args.status_file,
            {
                "command": command,
                "elapsed_seconds": 0,
                "outcome": "launch_failed",
                "process_exit_code": None,
                "timed_out": False,
                "wrapper_exit_code": 127,
            },
        )
        return 127

    reader = threading.Thread(
        target=tee_output,
        args=(process, args.build_log),
        name="build-output-tee",
    )
    reader.start()

    warned = False
    timed_out = False
    previous_cpu_seconds_by_pid: dict[int, float] = {}
    observed_cpu_seconds = 0.0
    peak_rss_bytes = 0
    peak_cpu_percent = 0.0
    peak_normalized_cpu_percent = 0.0
    logical_cpus = max(1, os.cpu_count() or 1)
    next_sample = started
    previous_sample_at = started

    while process.poll() is None:
        now = time.monotonic()
        elapsed = now - started
        if elapsed >= args.timeout_seconds:
            timed_out = True
            print(
                f"::error::Build exceeded its {args.timeout_seconds}-second deadline; terminating the process tree.",
                flush=True,
            )
            terminate_process_tree(process, args.output)
            break
        if args.warning_seconds > 0 and elapsed >= args.warning_seconds and not warned:
            print(
                "::warning::Cold release build exceeded the 20-minute target; "
                "continuing to collect compiler diagnostics.",
                flush=True,
            )
            warned = True
        if now >= next_sample:
            snapshot = active_processes(process.pid)
            cpu_delta = sum(
                max(0.0, cpu_seconds - previous_cpu_seconds_by_pid.get(pid, cpu_seconds))
                for pid, cpu_seconds in snapshot.cpu_seconds_by_pid.items()
            )
            sample_elapsed = max(0.001, now - previous_sample_at)
            derived_cpu_percent = cpu_delta / sample_elapsed * 100.0
            aggregate_cpu_percent = max(snapshot.cpu_percent, derived_cpu_percent)
            previous_cpu_seconds_by_pid = snapshot.cpu_seconds_by_pid
            previous_sample_at = now
            observed_cpu_seconds += cpu_delta
            peak_rss_bytes = max(peak_rss_bytes, snapshot.rss_bytes)
            peak_cpu_percent = max(peak_cpu_percent, aggregate_cpu_percent)
            normalized_cpu_percent = aggregate_cpu_percent / logical_cpus
            peak_normalized_cpu_percent = max(
                peak_normalized_cpu_percent, normalized_cpu_percent
            )
            emit(
                f"elapsed_seconds={int(elapsed)} "
                f"active_processes={len(snapshot.cpu_seconds_by_pid)} "
                f"active_rss_mib={snapshot.rss_bytes / 1_048_576:.1f} "
                f"aggregate_cpu_percent={aggregate_cpu_percent:.1f} "
                f"normalized_cpu_percent={normalized_cpu_percent:.1f} "
                f"active_cpu_seconds_delta={cpu_delta:.1f} "
                f"cpu_active={str(cpu_delta > 0.0 or aggregate_cpu_percent > 0.0).lower()}",
                args.output,
            )
            emit(latest_cargo_unit(args.build_log), args.output)
            for process_line in snapshot.lines:
                emit(process_line, args.output)
            next_sample = now + args.interval_seconds
        time.sleep(0.25)

    if process.poll() is None:
        terminate_process_tree(process, args.output)
    process_exit_code = process.wait()
    reader.join(timeout=30)
    if reader.is_alive():
        emit("build output reader did not stop cleanly", args.output)

    elapsed_seconds = int(time.monotonic() - started)
    if timed_out:
        outcome = "timed_out"
        wrapper_exit_code = 124
    elif process_exit_code == 0:
        outcome = "success"
        wrapper_exit_code = 0
    else:
        outcome = "failed"
        wrapper_exit_code = (
            process_exit_code if process_exit_code > 0 else 128 + abs(process_exit_code)
        )

    emit(
        f"release build wrapper stopped outcome={outcome} elapsed_seconds={elapsed_seconds} "
        f"peak_rss_mib={peak_rss_bytes / 1_048_576:.1f} "
        f"peak_aggregate_cpu_percent={peak_cpu_percent:.1f} "
        f"peak_normalized_cpu_percent={peak_normalized_cpu_percent:.1f} "
        f"observed_cpu_seconds={observed_cpu_seconds:.1f}",
        args.output,
    )
    write_status(
        args.status_file,
        {
            "command": command,
            "elapsed_seconds": elapsed_seconds,
            "finished_at": dt.datetime.now(dt.timezone.utc).isoformat(),
            "observed_cpu_seconds": observed_cpu_seconds,
            "outcome": outcome,
            "peak_aggregate_cpu_percent": peak_cpu_percent,
            "peak_normalized_cpu_percent": peak_normalized_cpu_percent,
            "peak_rss_bytes": peak_rss_bytes,
            "process_exit_code": process_exit_code,
            "started_at": started_wall.isoformat(),
            "timed_out": timed_out,
            "wrapper_exit_code": wrapper_exit_code,
        },
    )
    return wrapper_exit_code


if __name__ == "__main__":
    raise SystemExit(main())
