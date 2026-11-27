"""Runs the explicit-QEMU ARM64 Linux smoke container and waits for its result."""

from __future__ import annotations

import json
import re
import time

from release import Failure, complete

FINAL_STATES = ("exited", "dead")
WAITING_STATES = ("created", "running")


def fail(message: str):
    raise Failure(f"fjx package: {message}")


def format_duration(seconds: float) -> str:
    """Formats a duration the way Nushell prints one, for example 2ms or 1min 30sec."""
    remaining = round(seconds * 1_000_000_000)
    if remaining == 0:
        return "0sec"
    units = [("wk", 604_800_000_000_000), ("day", 86_400_000_000_000), ("hr", 3_600_000_000_000),
             ("min", 60_000_000_000), ("sec", 1_000_000_000), ("ms", 1_000_000), ("µs", 1_000), ("ns", 1)]
    sign = "-" if remaining < 0 else ""
    remaining = abs(remaining)
    parts = []
    for unit, size in units:
        count, remaining = divmod(remaining, size)
        if count:
            parts.append(f"{count}{unit}")
    return sign + " ".join(parts)


def remove_container(runtime: str, container: str) -> None:
    if container:
        complete([runtime, "rm", "--force", container])


def output(result) -> str:
    return "\n".join(text for text in (result.stdout.strip(), result.stderr.strip()) if text)


def nu_string(value) -> str:
    if value is None:
        return ""
    if isinstance(value, bool):
        return "true" if value else "false"
    return str(value)


def exit_code(value, platform: str) -> int:
    if isinstance(value, bool):
        return int(value)
    if isinstance(value, int):
        return value
    if isinstance(value, float) and value.is_integer():
        return int(value)
    if isinstance(value, str) and re.fullmatch(r"[+-]?[0-9]+", value):
        return int(value)
    fail(f"container runtime returned a nonnumeric ARM64 Linux smoke exit code for {platform}")


def run_arm64_smoke_container(runtime: str, smoke_container: str, platform: str, qemu: str, binary: str,
                              timeout: float, poll_interval: float) -> None:
    """Copies QEMU and fjx into a created container, starts it and waits for exit 0.

    timeout and poll_interval are in seconds. The container is always removed.
    """
    if timeout <= 0:
        fail("ARM64 Linux smoke timeout must be positive")
    if poll_interval <= 0:
        fail("ARM64 Linux smoke poll interval must be positive")
    try:
        for source, destination in ((qemu, f"{smoke_container}:/usr/local/bin/qemu-aarch64"),
                                    (binary, f"{smoke_container}:/usr/local/bin/fjx")):
            copied = complete([runtime, "cp", source, destination])
            if copied.exit_code != 0:
                fail(f"could not prepare explicit-QEMU ARM64 Linux smoke for {platform}: {copied.stderr.strip()}")
        started = complete([runtime, "start", smoke_container])
        if started.exit_code != 0:
            fail(f"could not start explicit-QEMU ARM64 Linux smoke for {platform}: {started.stderr.strip()}")
        deadline = time.monotonic() + timeout
        while True:
            inspected = complete([runtime, "inspect", smoke_container])
            if inspected.exit_code != 0:
                fail(f"could not inspect explicit-QEMU ARM64 Linux smoke for {platform}: {inspected.stderr.strip()}")
            invalid = f"container runtime returned invalid ARM64 Linux smoke state for {platform}"
            try:
                inspections = json.loads(inspected.stdout)
            except ValueError:
                fail(invalid)
            if not isinstance(inspections, list):
                fail(invalid)
            if len(inspections) != 1:
                fail(f"container runtime returned an unexpected ARM64 Linux smoke state count for {platform}")
            state = inspections[0].get("State") if isinstance(inspections[0], dict) else None
            if not isinstance(state, dict) or "Status" not in state:
                fail(invalid)
            status = nu_string(state["Status"])
            if status in FINAL_STATES:
                logs = complete([runtime, "logs", smoke_container])
                if logs.exit_code != 0:
                    fail(f"could not read explicit-QEMU ARM64 Linux smoke logs for {platform}: {logs.stderr.strip()}")
                code = exit_code(state.get("ExitCode"), platform)
                if status != "exited" or code != 0:
                    fail(f"explicit-QEMU ARM64 Linux runtime smoke failed for {platform} with status {status} "
                         f"and exit code {code}; logs: {output(logs)}")
                break
            if status not in WAITING_STATES:
                fail(f"explicit-QEMU ARM64 Linux runtime smoke entered unexpected status {status} for {platform}")
            if time.monotonic() >= deadline:
                timed_out_logs = complete([runtime, "logs", smoke_container])
                fail(f"explicit-QEMU ARM64 Linux runtime smoke timed out after {format_duration(timeout)} "
                     f"for {platform} in status {status}; logs: {output(timed_out_logs)}")
            time.sleep(poll_interval)
    finally:
        remove_container(runtime, smoke_container)
