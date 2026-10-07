#!/usr/bin/env python3
"""Take the documentation screenshots (docs/images/*.png).

Each shot rebuilds the demo world (make_demo.py), launches the dev build in
Impulse's headless snapshot mode inside the demo home, runs the shot's named
debug actions (MainWindowController+Debug.swift), and composes the captured
windows into one framed PNG with imagetool.swift. Nothing shows on screen.

usage: capture.py [--list] [--keep] [--recompose] [NAME_OR_PREFIX ...]
  --keep       also leave each shot's raw windows in target/docs-shots/<name>/
  --recompose  don't take new snapshots: compose again from the kept windows
               (after changing a crop)

Build the app first: ./impulse-macos/build.sh --dev
"""

from __future__ import annotations

import json
import os
import plistlib
import re
import signal
import shutil
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
sys.path.insert(0, str(HERE))
import make_demo  # noqa: E402

APP = REPO / "dist" / "Impulse Dev.app"
OUT = REPO / "docs" / "images"
WORK = REPO / "target" / "docs-shots"
TOOL = WORK / "imagetool"

# The dev build's preferences are yours. Snapshots must not show them or
# leave anything behind: the global keys below are pinned for each run with
# argument-domain overrides (which win over stored values) and put back
# afterwards, and keys about the demo's folders are removed.
BUNDLE_ID = "dev.impulse.Impulse.Devel"
PINNED = {
    "recentWorkspaceFolders": [],
    "paletteRecentCommands": [],
    "historyGraphHeight": 0,
    "projectTrust": b"",
}

# System helper windows that are never part of a shot.
IGNORED_CLASSES = ("NSCampoLightweightUIHostWindow",)

KEEP = False

FRAME = re.compile(
    r"frame window-(\d+) x=(\S+) y=(\S+) w=(\S+) h=(\S+) level=(\S+) sheet=(\S+) class=(\S+)"
)


def read_preferences() -> dict:
    out = subprocess.run(["defaults", "export", BUNDLE_ID, "-"], capture_output=True).stdout
    return plistlib.loads(out) if out else {}


def restore_preferences(saved: dict) -> None:
    """Put the pinned keys back as they were and drop keys about the demo.
    (`defaults import` only adds keys, so removals are explicit.)"""
    demo = str(make_demo.DEMO)
    for key in read_preferences():
        if demo in key or (key in PINNED and key not in saved):
            subprocess.run(["defaults", "delete", BUNDLE_ID, key], capture_output=True)
    restore = {key: saved[key] for key in PINNED if key in saved}
    if restore:
        backup = WORK / "preferences.plist"
        backup.write_bytes(plistlib.dumps(restore, fmt=plistlib.FMT_XML))
        subprocess.run(["defaults", "import", BUNDLE_ID, str(backup)], check=True)
        backup.unlink()


def argument(value) -> str:
    """A value in the old-style property list syntax of argument overrides."""
    if isinstance(value, bytes):
        return "<" + value.hex() + ">"
    if isinstance(value, list):
        return "(" + ", ".join(json.dumps(str(v)) for v in value) + ")"
    return str(value)


def build_tool() -> None:
    source = HERE / "imagetool.swift"
    if TOOL.exists() and TOOL.stat().st_mtime > source.stat().st_mtime:
        return
    WORK.mkdir(parents=True, exist_ok=True)
    subprocess.run(["swiftc", "-O", "-o", str(TOOL), str(source)], check=True)


def cleanup() -> None:
    """Stop whatever a shot left running in the demo (a backgrounded dev
    server, a stand-in agent): it would hold ports and files for the next."""
    found = subprocess.run(
        ["lsof", "-a", "-d", "cwd", "-t", "+D", str(make_demo.DEMO)], capture_output=True, text=True
    ).stdout.split()
    found += subprocess.run(["pgrep", "-f", str(make_demo.HOME / "bin")], capture_output=True, text=True).stdout.split()
    for pid in {int(p) for p in found} - {os.getpid()}:
        try:
            os.kill(pid, signal.SIGKILL)
        except ProcessLookupError:
            pass


def prepare(shot: dict, folder: Path) -> None:
    """The demo world plus the shot's settings and input files."""
    make_demo.main(shot.get("variant"))
    settings_dir = make_demo.HOME / "Library" / "Application Support" / "impulse-dev"
    if "settings_text" in shot:
        (settings_dir / "settings.json").write_text(shot["settings_text"])
    elif "settings" in shot:
        settings = json.loads((settings_dir / "settings.json").read_text())
        settings.update(shot["settings"])
        (settings_dir / "settings.json").write_text(json.dumps(settings, indent=2))
    for name, content in shot.get("files", {}).items():
        path = folder / name
        path.write_text(content if isinstance(content, str) else json.dumps(content, indent=2))


def read_windows(folder: Path) -> list[dict]:
    windows = []
    for match in FRAME.finditer((folder / "snapshot.log").read_text()):
        index, x, y, w, h, level, sheet, cls = match.groups()
        windows.append({
            "path": str(folder / f"window-{index}.png"), "x": float(x), "y": float(y), "w": float(w),
            "h": float(h), "level": int(level), "sheet": sheet == "true", "class": cls,
        })
    return [w for w in windows if Path(w["path"]).exists() and w["class"] not in IGNORED_CLASSES]


def snapshot(shot: dict, folder: Path) -> list[dict]:
    session_file = folder / "session.json"
    session_file.write_text(json.dumps(shot["session"], indent=2))
    width, height = shot.get("size", (1280, 800))
    # Input files are referenced as {files}/<name> in actions.
    actions = ",".join(a.replace("{files}", str(folder)) for a in shot.get("actions", []))
    # Pinned preferences, plus the shot's own (paths under ~ are demo paths).
    overrides = dict(PINNED)
    for key, value in shot.get("defaults", {}).items():
        overrides[key] = [str(v).replace("~", str(make_demo.HOME), 1) for v in value]
    pinned = [part for key, value in overrides.items() for part in ("-" + key, argument(value))]
    args = [
        "open", "-g", "-n", "-W", "-a", str(APP),
        "--env", f"CFFIXED_USER_HOME={make_demo.HOME}", "--env", f"HOME={make_demo.HOME}",
        "--env", "USER=impulse-docs", "--env", "SHELL=/bin/zsh",
        # Not yours: tools found on PATH show up in Settings.
        "--env", "PATH=/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin",
        "--args",
        *pinned,
        "--impulse-snapshot", str(folder),
        "--impulse-snapshot-no-lsp",
        "--impulse-snapshot-delay", str(shot.get("delay", 8)),
        "--impulse-snapshot-size", f"{width}x{height}",
        "--impulse-snapshot-session", str(session_file),
    ]
    if actions:
        args += ["--impulse-snapshot-actions", actions]
    subprocess.run(args, check=True, timeout=240)
    log = folder / "snapshot.log"
    for _ in range(50):
        if log.exists():
            break
        time.sleep(0.2)
    return read_windows(folder)


def output(name: str) -> Path:
    """Shots whose names start with "_" are parts of a combined image."""
    return WORK / f"{name}.png" if name.startswith("_") else OUT / f"{name}.png"


def combine(shot: dict) -> None:
    spec = {"scale": 2, "beside": [str(output(n)) for n in shot["combine"]], "gap": shot.get("gap", 0),
            "out": str(output(shot["name"]))}
    spec_file = WORK / f"{shot['name']}-combine.json"
    spec_file.write_text(json.dumps(spec))
    subprocess.run([str(TOOL), str(spec_file)], check=True)


def compose(shot: dict, windows: list[dict], folder: Path) -> None:
    width, height = shot.get("size", (1280, 800))
    if shot.get("base"):
        # A window other than the main one (e.g. the theme gallery).
        main = next(w for w in windows if w["class"] == shot["base"])
    else:
        mains = [w for w in windows if w["w"] == width and w["h"] == height and not w["sheet"]]
        if not mains:
            sys.exit(f"{shot['name']}: no main window in {folder}")
        main = mains[0]
    width, height = main["w"], main["h"]
    # Screen coordinates are y-up; the spec is top-left.
    overlays = []
    only = shot.get("overlays")  # None: every other window; else class names
    for w in windows:
        if w is main or (only is not None and w["class"] not in only):
            continue
        overlays.append({
            "path": w["path"], "x": w["x"] - main["x"],
            "y": (main["y"] + main["h"]) - (w["y"] + w["h"]),
            "w": w["w"], "h": w["h"],
            "radius": 14 if w["sheet"] else 10, "shadow": True,
        })
    crop = shot.get("crop")
    if crop == "overlay":
        # The first floating window plus a margin of the window around it.
        if not overlays:
            sys.exit(f"{shot['name']}: no floating window to crop to in {folder}")
        o = overlays[0]
        pad = shot.get("pad", 24)
        crop = (o["x"] - pad, o["y"] - pad, o["w"] + 2 * pad, o["h"] + 2 * pad)
    style = shot.get("style", "window" if crop is None else "crop")
    spec = {
        "scale": 2,
        "base": main["path"],
        "overlays": [{k: v for k, v in o.items() if k not in ("w", "h")} for o in overlays],
        "radius": {"window": 16, "crop": 10}.get(style, 10),
        "shadow": style == "window",
        "border": True,
        "out": str(output(shot["name"])),
    }
    if crop:
        x, y, w, h = crop
        x, y = max(0, x), max(0, y)
        spec["crop"] = {"x": x, "y": y, "w": min(w, width - x), "h": min(h, height - y)}
    spec_file = folder / "spec.json"
    spec_file.write_text(json.dumps(spec, indent=2))
    subprocess.run([str(TOOL), str(spec_file)], check=True)


def main() -> None:
    from shots import SHOTS

    args = sys.argv[1:]
    global KEEP
    KEEP = "--keep" in args
    recompose = "--recompose" in args
    args = [a for a in args if a not in ("--keep", "--recompose")]
    if args[:1] == ["--list"]:
        for shot in SHOTS:
            print(shot["name"])
        return
    if not APP.exists():
        sys.exit(f"{APP} is missing: run ./impulse-macos/build.sh --dev first")
    selected = [s for s in SHOTS if not args or any(s["name"].startswith(a) for a in args)]
    if not selected:
        sys.exit("no shots match")
    build_tool()
    OUT.mkdir(parents=True, exist_ok=True)
    saved = read_preferences()
    try:
        run(selected, recompose)
    finally:
        if not recompose:
            restore_preferences(saved)


def run(selected: list, recompose: bool) -> None:
    for shot in selected:
        started = time.time()
        if "combine" in shot:
            combine(shot)
            print(f"{shot['name']}.png  (combined)", flush=True)
            continue
        folder = WORK / shot["name"]
        if recompose:
            compose(shot, read_windows(folder), folder)
            print(f"{shot['name']}.png  (recomposed)", flush=True)
            continue
        if folder.exists():
            shutil.rmtree(folder)
        folder.mkdir(parents=True)
        cleanup()
        prepare(shot, folder)
        windows = snapshot(shot, folder)
        cleanup()
        compose(shot, windows, folder)
        if not KEEP:
            for png in folder.glob("window-*.png"):
                png.unlink()
        print(f"{shot['name']}.png  ({time.time() - started:.0f}s)", flush=True)


if __name__ == "__main__":
    main()
