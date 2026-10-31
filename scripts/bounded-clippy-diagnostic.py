#!/usr/bin/env python3
"""One bounded compiler diagnostic; stop its own process group before starvation."""
import argparse
import json
import os
from pathlib import Path
import signal
import subprocess
import time


def execute(command, memory_file, swap_file, report, timeout=2700,
            max_memory=11 * 1024**3, max_swap=2 * 1024**3):
    # Fail before starting if the worker cannot expose its own cgroup counters.
    int(memory_file.read_text())
    int(swap_file.read_text())
    start = time.monotonic()
    peak_memory = peak_swap = 0
    reason = None
    process = subprocess.Popen(command, start_new_session=True)
    def interrupted(signum, _frame):
        raise InterruptedError(f"monitor received signal {signum}")
    handlers = {sig: signal.signal(sig, interrupted) for sig in (signal.SIGTERM, signal.SIGINT)}
    try:
        while process.poll() is None:
            memory = int(memory_file.read_text())
            swap = int(swap_file.read_text())
            peak_memory = max(peak_memory, memory)
            peak_swap = max(peak_swap, swap)
            if memory >= max_memory:
                reason = 'memory safety cutoff before the 12-GiB pool cap'
            elif swap >= max_swap:
                reason = 'swap safety cutoff before heartbeat starvation'
            elif time.monotonic() - start >= timeout:
                reason = 'bounded diagnostic timeout'
            if reason:
                break
            time.sleep(1)
    except BaseException:
        reason = 'monitor interrupted or failed'
        raise
    finally:
        for sig in handlers:
            signal.signal(sig, signal.SIG_IGN)
        # Dedicated session/group contains only this invocation and its children.
        # Also handle an interrupted monitor after Cargo itself has exited.
        if process.poll() is None or reason:
            try:
                os.killpg(process.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                process.wait(timeout=5)
            # Cargo can exit on TERM while a compiler descendant remains.
            # Kill surviving members of ONLY our dedicated group in that case.
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        for sig, handler in handlers.items():
            signal.signal(sig, handler)
        summary = dict(command=command, exit_code=process.returncode,
                       stop_reason=reason, elapsed_seconds=round(time.monotonic() - start, 2),
                       peak_memory_bytes=peak_memory, peak_swap_bytes=peak_swap,
                       memory_cutoff_bytes=max_memory, swap_cutoff_bytes=max_swap,
                       guard_sample_seconds=1, guard_is_hard_cgroup_limit=False,
                       allocator_conf=os.environ.get('MALLOC_CONF'), candidate_only=True)
        report.write_text(json.dumps(summary, indent=2) + '\n')
        print(json.dumps(summary, indent=2), flush=True)
    return 1 if reason or process.returncode else 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--report', type=Path, required=True)
    parser.add_argument('command', nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command
    if command and command[0] == '--':
        command = command[1:]
    if not command:
        parser.error('a compiler command is required')
    return execute(command, Path('/sys/fs/cgroup/memory.current'),
                   Path('/sys/fs/cgroup/memory.swap.current'), args.report)


if __name__ == '__main__':
    raise SystemExit(main())
