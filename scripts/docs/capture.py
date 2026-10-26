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

The Rust app (impulse-app) runs the same snapshots: set IMPULSE_SNAPSHOT_APP
to its binary (target/debug/impulse-app, the default on Linux, where the
Swift app doesn't exist). Build it first with `cargo build -p impulse-app
-p impulse-cli`, on the Mac with `--features impulse-app/snapshot` added
(its window images come from GPUI's test-support; without it the app writes
none and says so in snapshot.log). On the Mac its windows never show either:
they stay transparent and are rendered off screen, without the window's
buttons, so the main window's are composited in from traffic-lights@2x.png
(the inactive ones, as in the Swift shots). On Linux it needs an X
server in DISPLAY (Xvfb), and the shots are composed by imagetool.py
(Pillow) instead of imagetool.swift.
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

MAC = sys.platform == "darwin"
# The Swift dev build, or the Rust app's binary.
APP = Path(os.environ.get("IMPULSE_SNAPSHOT_APP")
           or (REPO / "dist" / "Impulse Dev.app" if MAC else REPO / "target" / "debug" / "impulse-app"))
RUST = APP.suffix != ".app"
# How to build the Rust app for snapshots (on the Mac its window images need
# the snapshot feature).
RUST_BUILD = "cargo build -p impulse-app -p impulse-cli" + (" --features impulse-app/snapshot" if MAC else "")
# The `impulse` the app's terminals get, and the MCP entries fall back to.
CLI = APP.parent / "impulse" if RUST else APP / "Contents" / "Resources" / "bin" / "impulse"
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
# The Rust app draws dialogs and the palette inside its window: where one is,
# inside its frame, from the window's top left.
OVERLAY = re.compile(r"overlay window-(\d+) x=(\S+) y=(\S+) w=(\S+) h=(\S+)")

# The Rust app's window images on the Mac hold only what GPUI draws, not the
# window's buttons, so its main window gets the inactive ones the Swift
# shots show: cropped at 2x from a Swift shot (getting-started-window-tour)
# and taken off its background into alpha. Where they go, in points from the
# window's top left: main.rs puts the close button's 14-point frame at
# (14, 13), and the image starts 1 point left of and 1.5 above a button.
TRAFFIC_LIGHTS = HERE / "traffic-lights@2x.png"
TRAFFIC_LIGHTS_AT = (13, 11.5)


def read_preferences() -> dict:
    if RUST:
        # The Rust app keeps these in files in the demo home's data folder.
        return {}
    out = subprocess.run(["defaults", "export", BUNDLE_ID, "-"], capture_output=True).stdout
    return plistlib.loads(out) if out else {}


def restore_preferences(saved: dict) -> None:
    """Put the pinned keys back as they were and drop keys about the demo.
    (`defaults import` only adds keys, so removals are explicit.)"""
    if RUST:
        return
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
    if not MAC:
        return
    source = HERE / "imagetool.swift"
    if TOOL.exists() and TOOL.stat().st_mtime > source.stat().st_mtime:
        return
    WORK.mkdir(parents=True, exist_ok=True)
    subprocess.run(["swiftc", "-O", "-o", str(TOOL), str(source)], check=True)


def compose_image(spec_file: Path) -> None:
    """imagetool.swift on the Mac, its Pillow port elsewhere."""
    if MAC:
        subprocess.run([str(TOOL), str(spec_file)], check=True)
    else:
        subprocess.run([sys.executable, str(HERE / "imagetool.py"), str(spec_file)], check=True)


def environment(shell: str = "/bin/zsh") -> dict:
    """The stand-in home's environment, for a binary run directly."""
    home = make_demo.HOME
    runtime = make_demo.DEMO / "runtime"
    runtime.mkdir(mode=0o700, parents=True, exist_ok=True)
    env = {
        "HOME": str(home), "USER": "impulse-docs", "SHELL": shell,
        "XDG_CONFIG_HOME": str(home / ".config"), "XDG_DATA_HOME": str(home / ".local" / "share"),
        "XDG_CACHE_HOME": str(home / ".cache"), "XDG_STATE_HOME": str(home / ".local" / "state"),
        "XDG_RUNTIME_DIR": str(runtime),
        "PATH": "/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin" + ("" if MAC else ":/usr/local/bin"),
        "RUST_LOG": os.environ.get("RUST_LOG", "warn"),
    }
    for key in ("DISPLAY", "WAYLAND_DISPLAY", "TMPDIR", "RUST_BACKTRACE"):
        if key in os.environ:
            env[key] = os.environ[key]
    return env


def launch_command(args: list[str], env: dict, pinned: list[str]) -> tuple[list[str], dict | None]:
    """How to start the app with `args` in the demo home: the Swift app
    through `open` (hidden, its preferences pinned), the Rust app directly."""
    if RUST:
        return [str(APP), *args], env
    command = ["open", "-g", "-n", "-W", "-a", str(APP), "--env", f"CFFIXED_USER_HOME={make_demo.HOME}"]
    for key in ("HOME", "USER", "SHELL", "XDG_CONFIG_HOME", "PATH"):
        if key in env:
            command += ["--env", f"{key}={env[key]}"]
    return [*command, "--args", *pinned, *args], None


def run_app(args: list[str], env: dict, pinned: list[str], timeout: float) -> bool:
    """Runs the app until it quits. False if it had to be stopped."""
    command, environ = launch_command(args, env, pinned)
    process = subprocess.Popen(command, env=environ)
    try:
        process.wait(timeout=timeout)
        return True
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()
        if not RUST:
            # `open` has stopped; the app it started is named by the run's folder.
            subprocess.run(["pkill", "-9", "-f", args[args.index("--impulse-snapshot") + 1]], capture_output=True)
        return False


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
    settings_dir = make_demo.data_dir("impulse-dev")
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
    for match in OVERLAY.finditer((folder / "snapshot.log").read_text()):
        index, *rect = match.groups()
        for w in windows:
            if w["path"].endswith(f"window-{index}.png"):
                w["overlay"] = tuple(float(v) for v in rect)
    return [w for w in windows if Path(w["path"]).exists() and w["class"] not in IGNORED_CLASSES]


def drawn_overlay(window: dict, scale: int) -> tuple[float, float, float, float]:
    """The frame of a dialog or palette the Rust app drew inside `window`:
    from the rectangle it reported, outward to where the frame's own
    background ends, along lines through its padding, then out over its
    one-point border (`scale` pixels). The top and bottom are found 16
    points in from the left, past the corners' curve (radii up to 14)."""
    from PIL import Image
    image = Image.open(window["path"]).convert("RGB")
    x, y, w, h = (round(v * scale) for v in window["overlay"])
    left, top, right, bottom = x, y, x + w - 1, y + h - 1
    inset = 4 * scale
    background = image.getpixel((left + inset, top + inset))

    def same(px: tuple[int, int]) -> bool:
        return sum(abs(a - b) for a, b in zip(image.getpixel(px), background)) < 24

    def walk(px: int, py: int, dx: int, dy: int) -> tuple[int, int]:
        while 0 < px < image.width - 1 and 0 < py < image.height - 1 and same((px + dx, py + dy)):
            px, py = px + dx, py + dy
        return px, py

    middle = (top + bottom) // 2
    column = min(x + 16 * scale, (left + right) // 2)
    left = walk(left + inset, middle, -1, 0)[0] - scale
    right = walk(right - inset, middle, 1, 0)[0] + scale
    top = walk(column, top + inset, 0, -1)[1] - scale
    bottom = walk(column, bottom - inset, 0, 1)[1] + scale
    return (left / scale, top / scale, (right - left + 1) / scale, (bottom - top + 1) / scale)


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
    if RUST and "recentWorkspaceFolders" in overrides:
        recents = make_demo.data_dir("impulse-dev") / "recent-workspaces.json"
        recents.write_text(json.dumps(overrides["recentWorkspaceFolders"], indent=2))
    # Not yours: tools found on PATH show up in Settings.
    env = environment(shutil.which("zsh") or "/bin/zsh")
    args = [
        "--impulse-snapshot", str(folder),
        "--impulse-snapshot-no-lsp",
        "--impulse-snapshot-delay", str(shot.get("delay", 8)),
        "--impulse-snapshot-size", f"{width}x{height}",
        "--impulse-snapshot-session", str(session_file),
    ]
    if actions:
        args += ["--impulse-snapshot-actions", actions]
    if not run_app(args, env, pinned, 240):
        sys.exit(f"{shot['name']}: the app didn't finish in 240 s")
    log = folder / "snapshot.log"
    for _ in range(50):
        if log.exists():
            break
        time.sleep(0.2)
    if RUST and log.exists() and "snapshot feature" in log.read_text():
        sys.exit(f"{shot['name']}: no window images: build with {RUST_BUILD}")
    return read_windows(folder)


def output(name: str) -> Path:
    """Shots whose names start with "_" are parts of a combined image."""
    return WORK / f"{name}.png" if name.startswith("_") else OUT / f"{name}.png"


def combine(shot: dict) -> None:
    spec = {"scale": 2 if MAC else 1, "beside": [str(output(n)) for n in shot["combine"]], "gap": shot.get("gap", 0),
            "out": str(output(shot["name"]))}
    spec_file = WORK / f"{shot['name']}-combine.json"
    spec_file.write_text(json.dumps(spec))
    compose_image(spec_file)


def compose(shot: dict, windows: list[dict], folder: Path) -> None:
    width, height = shot.get("size", (1280, 800))
    if shot.get("base"):
        # A window other than the main one (e.g. the theme gallery).
        # The Rust app's other windows are all class "Window".
        base = "Window" if RUST and shot["base"] == "NSWindow" else shot["base"]
        main = next(w for w in windows if w["class"] == base)
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
    shadows = []
    if crop == "overlay":
        # The first floating window plus a margin of the window around it.
        pad = shot.get("pad", 24)
        if overlays:
            o = overlays[0]
            crop = (o["x"] - pad, o["y"] - pad, o["w"] + 2 * pad, o["h"] + 2 * pad)
        elif RUST and "overlay" in main:
            with open(main["path"], "rb") as png:
                scale = round(int.from_bytes(png.read(24)[16:20], "big") / width) or 1
            x, y, w, h = drawn_overlay(main, scale)
            # The Swift app's floating window can be larger than what it
            # draws (the toast panel's margin): the crop keeps that room.
            pad += shot.get("panel", 0)
            crop = (x - pad, y - pad, w + 2 * pad, h + 2 * pad)
            # And it gets the shadow the Swift app's windows get as overlays.
            shadows.append({"x": x, "y": y, "w": w, "h": h, "radius": 10})
        else:
            sys.exit(f"{shot['name']}: no floating window to crop to in {folder}")
    style = shot.get("style", "window" if crop is None else "crop")
    # Pixels per point, as the window was captured (2 on a Retina Mac).
    with open(main["path"], "rb") as png:
        pixels = int.from_bytes(png.read(24)[16:20], "big")
    scale = round(pixels / width) or 1
    if RUST and MAC and not shot.get("base") and scale == 2:
        # Under everything floating over the window, as the real ones are.
        x, y = TRAFFIC_LIGHTS_AT
        overlays.insert(0, {"path": str(TRAFFIC_LIGHTS), "x": x, "y": y, "radius": 0, "shadow": False})
    spec = {
        "scale": scale,
        "base": main["path"],
        "overlays": [{k: v for k, v in o.items() if k not in ("w", "h")} for o in overlays],
        "shadows": shadows,
        "radius": {"window": 16, "crop": 10}.get(style, 10),
        "shadow": style == "window",
        "border": True,
        "out": str(output(shot["name"])),
    }
    if crop:
        x, y, w, h = crop
        if RUST and shot.get("base") == "NSWindow":
            # Crops of an AppKit window count its 28-point titlebar; the
            # Rust app's window images start below the system's.
            y -= 28
        x, y = max(0, x), max(0, y)
        spec["crop"] = {"x": x, "y": y, "w": min(w, width - x), "h": min(h, height - y)}
    spec_file = folder / "spec.json"
    spec_file.write_text(json.dumps(spec, indent=2))
    compose_image(spec_file)


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
        sys.exit(f"{APP} is missing: run " + (RUST_BUILD if RUST else "./impulse-macos/build.sh --dev") + " first")
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
