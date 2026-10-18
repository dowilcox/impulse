#!/usr/bin/env python3
"""Smoke checks before a release: drive the dev build through what people
do (edit and save, run commands in each shell, the impulse CLI, agents,
trust and language servers, tasks, session restore) and check the results
on disk, in git and in the app's own state.

Each scenario rebuilds the screenshots' demo world (scripts/docs/make_demo.py)
plus files of its own, launches Impulse Dev headless the way
scripts/docs/capture.py does (invisible windows, a stand-in home, events
delivered inside the app, so it runs while you work), runs its actions
(MainWindowController+Debug.swift, "Smoke Checks") and checks the JSON state
they dump. Nothing it runs talks to the network except the language server
install.

usage: run.py [--list] [NAME_OR_PREFIX ...]

Build the app first: ./impulse-macos/build.sh --dev
Each scenario's actions, dumps and final window images are left in
target/smoke/<name>/.
"""

from __future__ import annotations

import json
import os
import shutil
import signal
import subprocess
import sys
import time
import traceback
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
sys.path.insert(0, str(REPO / "scripts" / "docs"))
import capture  # noqa: E402  (launch environment and preference pinning)
import make_demo  # noqa: E402

from scenarios import SCENARIOS  # noqa: E402

WORK = REPO / "target" / "smoke"
DATA = make_demo.HOME / "Library" / "Application Support" / "impulse-dev"


class Run:
    """One scenario's outcome: its dumps, and what passed or failed."""

    def __init__(self, name: str, folder: Path):
        self.name = name
        self.folder = folder
        self.passed: list[str] = []
        self.failed: list[str] = []

    def dump(self, name: str, launch: int = 1) -> dict:
        path = self.folder / f"run{launch}" / f"{name}.json"
        if not path.exists():
            self.fail(f"no {name} dump (run{launch})")
            return {"tabs": [], "workspaces": [], "toasts": [], "sheets": [], "results": {}, "failures": []}
        return json.loads(path.read_text())

    def expect(self, ok: bool, what: str, detail: object = None) -> bool:
        if ok:
            self.passed.append(what)
        else:
            self.fail(what if detail is None else f"{what}\n      {str(detail)[:600]}")
        return ok

    def fail(self, what: str) -> None:
        self.failed.append(what)


# ---------------------------------------------------------------------------
# Running the app


def stop_leftovers() -> None:
    """Stop whatever a run left in the demo (shells, agents, language
    servers, a less): they'd hold files and ports for the next one."""
    demo = str(make_demo.DEMO)
    pids = set()
    listing = subprocess.run(["ps", "-axo", "pid=,command="], capture_output=True, text=True).stdout
    for line in listing.splitlines():
        pid, _, command = line.strip().partition(" ")
        if demo in command or str(WORK) in command:
            pids.add(int(pid))
    cwds = subprocess.run(["lsof", "-a", "-d", "cwd", "-Fpn"], capture_output=True, text=True).stdout
    pid = None
    for line in cwds.splitlines():
        if line.startswith("p"):
            pid = int(line[1:])
        elif line.startswith("n") and pid and line[1:].startswith(demo):
            pids.add(pid)
    for pid in pids - {os.getpid()}:
        try:
            os.kill(pid, signal.SIGKILL)
        except ProcessLookupError:
            pass


def launch(scenario: dict, out: Path, actions: list[str], session_file: Path) -> bool:
    """Runs the app until its actions are done. False if it had to be stopped."""
    out.mkdir(parents=True)
    # Always end with the state, so waits that timed out are reported.
    (out / "actions.json").write_text(json.dumps([*actions, "dump=final"], indent=1, ensure_ascii=False))
    pinned = [part for key, value in capture.PINNED.items() for part in ("-" + key, capture.argument(value))]
    args = [
        "open", "-g", "-n", "-W", "-a", str(capture.APP),
        "--env", f"CFFIXED_USER_HOME={make_demo.HOME}", "--env", f"HOME={make_demo.HOME}",
        "--env", "USER=impulse-docs", "--env", f"SHELL={scenario.get('shell', '/bin/zsh')}",
        "--env", "PATH=/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin",
        "--args",
        *pinned,
        "--impulse-snapshot", str(out),
        "--impulse-snapshot-delay", "2",
        "--impulse-snapshot-session", str(session_file),
        "--impulse-snapshot-actions-file", str(out / "actions.json"),
    ]
    if not scenario.get("lsp"):
        args.append("--impulse-snapshot-no-lsp")
    if scenario.get("persist"):
        args.append("--impulse-snapshot-persist")
    try:
        subprocess.run(args, check=True, timeout=scenario.get("timeout", 240))
        return True
    except subprocess.TimeoutExpired:
        subprocess.run(["pkill", "-9", "-f", str(out)], capture_output=True)
        return False


def prepare(scenario: dict) -> None:
    make_demo.main()
    settings_file = DATA / "settings.json"
    settings = json.loads(settings_file.read_text())
    settings.update(scenario.get("settings", {}))
    settings_file.write_text(json.dumps(settings, indent=2))
    if "setup" in scenario:
        scenario["setup"]()


def run_scenario(scenario: dict) -> Run:
    folder = WORK / scenario["name"]
    if folder.exists():
        shutil.rmtree(folder)
    folder.mkdir(parents=True)
    run = Run(scenario["name"], folder)
    stop_leftovers()
    prepare(scenario)
    session_file = folder / "session.json"
    session_file.write_text(json.dumps(scenario["session"], indent=2))
    launches = [(scenario["actions"], session_file)]
    if "relaunch" in scenario:
        # The second launch restores the session the first one saved.
        launches.append((scenario["relaunch"], DATA / "session-state.json"))
    for number, (actions, session) in enumerate(launches, start=1):
        if not launch(scenario, folder / f"run{number}", actions, session):
            run.fail(f"run{number} didn't finish in {scenario.get('timeout', 240)} s (stopped)")
        for failure in run.dump("final", number).get("failures", []):
            run.fail(f"run{number}: {failure}")
        if number < len(launches):
            stop_leftovers()
    try:
        scenario["check"](run)
    except Exception:
        run.fail("check raised:\n" + traceback.format_exc())
    stop_leftovers()
    return run


def main() -> None:
    args = sys.argv[1:]
    if args[:1] == ["--list"]:
        for scenario in SCENARIOS:
            print(f"{scenario['name']:<16} {scenario['about']}")
        return
    if not capture.APP.exists():
        sys.exit(f"{capture.APP} is missing: run ./impulse-macos/build.sh --dev first")
    selected = [s for s in SCENARIOS if not args or any(s["name"].startswith(a) for a in args)]
    if not selected:
        sys.exit("no scenarios match")
    capture.WORK.mkdir(parents=True, exist_ok=True)
    saved = capture.read_preferences()
    results = []
    try:
        for scenario in selected:
            started = time.time()
            print(f"{scenario['name']} …", flush=True)
            run = run_scenario(scenario)
            results.append(run)
            status = "ok" if not run.failed else f"{len(run.failed)} FAILED"
            print(f"  {len(run.passed)} passed, {status}  ({time.time() - started:.0f}s)", flush=True)
            for failure in run.failed:
                print(f"  ✗ {failure}", flush=True)
    finally:
        capture.restore_preferences(saved)
    failed = [run for run in results if run.failed]
    total = sum(len(run.passed) for run in results)
    print(f"\n{total} checks passed; " + (f"{len(failed)} scenario(s) failed: {', '.join(r.name for r in failed)}"
                                         if failed else "every scenario passed."))
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
