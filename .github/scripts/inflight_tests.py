#!/usr/bin/env python3
"""Report which tests were still running when a test binary died.

libtest's normal output prints a test when it *finishes*. That is why every
round of chasing the Windows crash has been reading the wrong names: the test
that crashed was still running, so it never printed, and the last line in the
log belongs to some unrelated test that happened to finish first.

libtest's JSON format also emits an event when a test *starts*:

    { "type": "test", "event": "started", "name": "..." }
    { "type": "test", "name": "...", "event": "ok" }

The tests that started and never reported a result are exactly the ones in
flight when the process died. At four threads that is at most four names out of
2586.

Usage:
    varda-<hash>.exe -Zunstable-options --format=json --test-threads=4 > events.jsonl
    python .github/scripts/inflight_tests.py events.jsonl

`--format=json` needs `-Zunstable-options`, so the run needs RUSTC_BOOTSTRAP=1.
Set it only for the run: it is part of the build fingerprint, and setting it
during compilation evicts the whole cargo cache.

See spec/roadmap.md, DEBT: Windows Heap Corruption Under Parallel Tests.
"""

import json
import sys


def in_flight(lines):
    """Return (tests started, names that never reported a result)."""
    running = []
    started = 0
    for line in lines:
        if not line.startswith("{"):
            # cargo's own progress lines, and anything else sharing the stream.
            continue
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            # A crash can cut the final line in half. Everything before it still
            # counts, so skip the fragment rather than giving up on the file.
            continue
        if event.get("type") != "test":
            continue
        name = event.get("name")
        if event.get("event") == "started":
            running.append(name)
            started += 1
        elif name in running:
            running.remove(name)
    return started, running


def main():
    if len(sys.argv) != 2:
        print(f"usage: {sys.argv[0]} <events.jsonl>", file=sys.stderr)
        return 2

    with open(sys.argv[1], encoding="utf-8", errors="replace") as f:
        started, running = in_flight(f)

    print(f"{started} tests started, {len(running)} still in flight")

    if started == 0:
        # Distinct from "crashed early", and easy to misread as one. No start
        # events at all means the process never reached the first test, so
        # whatever killed it happened before any Varda code ran and is not the
        # fault being hunted. Loader failures look like this: exit 0xC0000135
        # (STATUS_DLL_NOT_FOUND) is the one already seen here, from launching
        # the test binary directly instead of through `cargo test`, which sets
        # up the library search path.
        print()
        print("NOTE: no test ever started, so the process died before libtest")
        print("      got going. This says nothing about the crash under")
        print("      investigation. Check how the binary was launched.")
        return 0

    if not running:
        return 0

    print()
    print(f"==== IN FLIGHT WHEN IT DIED ({len(running)}) ====")
    for name in sorted(running):
        print(f"  {name}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
