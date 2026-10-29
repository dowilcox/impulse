"""The smoke scenarios (see run.py).

Each scenario:
  name, about   shown by --list
  session       the window to start with (make_demo.session)
  setup         makes its files, after the demo world is built
  settings      keys merged into the demo's settings.json
  shell         the terminals' shell (default /bin/zsh)
  lsp           start language servers (off by default)
  persist       save settings and the session as a normal launch does
  actions       debug actions, run in order (impulse-app/src/debug_snapshot.rs)
  relaunch      actions for a second launch that restores the saved session
  timeout       seconds before the app is stopped (default 240)
  check(run)    reads the dumps and files and calls run.expect(...)
"""

from __future__ import annotations

import base64
import json
import os
import re
import shutil
import struct
import subprocess
import zlib
from pathlib import Path

import capture
import make_demo
from dumps import editor, plain, surfaces, toast_messages
from make_demo import HOME, PROJECT, session

SMOKE = PROJECT / "smoke"
ONE = session([{"root": "trailhead", "tabs": [("terminal", "trailhead")]}])


def write(path: Path, content: str | bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    if isinstance(content, bytes):
        path.write_bytes(content)
    else:
        path.write_text(content)


# ---------------------------------------------------------------------------
# Editor: saves write exactly what's on screen (emoji, CJK, combining
# characters, CRLF), reloads from disk are in place and undoable, a change
# on disk never silently loses typing, and files the editor can't show are
# refused with a toast.

UNICODE_TEXT = "Trail notes 🥾\n日本語の道\ncafé ñ — naïve\nend\n"
CRLF_TEXT = "first\r\nsecond\r\nthird\r\n"
BIG_NAME = "big log (copy).log"

# `edit=` commands (EditorTab::debug_edit), columns in characters: typing,
# typing at the start of three lines, a paste, a replacement, deleting across
# an emoji, a new line, then undo/undo/redo.
EDIT_UNICODE = r"""
cursor 1:15
type  ✓
cursor 2:1
type » 
cursor 3:1
type » 
cursor 4:1
type » 
cursor 5:4
paste \npasted 🧭 line\nand another
select 2:3-2:4
type 道🌲
cursor 1:13
select 1:13-1:15
type 
cursor 2:4
type \n
undo
undo
redo
"""

EDIT_CRLF = r"""
cursor 2:7
type \ninserted
cursor 5:1
paste pasted one\npasted two\n
"""


def text(found: dict) -> str:
    """An open editor's text in a dump (the key is the one the Swift app's
    dumps used)."""
    return found["monaco"]


def setup_editor() -> None:
    write(SMOKE / "unicode.txt", UNICODE_TEXT)
    write(SMOKE / "crlf.txt", CRLF_TEXT.encode())
    write(SMOKE / "reload.txt", "version one\n")
    write(SMOKE / "dirty.txt", "keep me\n")
    write(SMOKE / BIG_NAME, ("x" * 99 + "\n") * 110_000)
    write(SMOKE / "blob.bin", b"\x00\x01\x02binary\x00\xff")
    write(SMOKE / "latin1.txt", b"caf\xe9 au lait\n")


EDITOR_ACTIONS = [
    "until=prompt",
    # Edits in a file with emoji, CJK and combining characters.
    "open=smoke/unicode.txt", "until=editor",
    "edit=unicode:" + EDIT_UNICODE,
    "sleep=1", "dump=unicode-edited",
    "save", "until=result:save:unicode.txt", "sleep=1", "dump=unicode-saved",
    # CRLF line endings stay CRLF.
    "open=smoke/crlf.txt", "until=editor",
    "edit=crlf:" + EDIT_CRLF,
    "sleep=1", "save", "until=result:save:crlf.txt", "sleep=1", "dump=crlf-saved",
    # A clean buffer follows the file on disk, and the reload can be undone.
    "open=smoke/reload.txt", "until=editor",
    "write-file=smoke/reload.txt|version two\nfrom disk\n",
    "until=15:text:version two\\nfrom disk\\n",
    "sleep=1", "dump=reloaded",
    "edit=undo-reload:undo",
    "sleep=1", "dump=reload-undone",
    # A dirty buffer isn't replaced by a change on disk; saving then asks.
    "open=smoke/dirty.txt", "until=editor",
    "edit=dirty:cursor 1:1\ntype edited ",
    "write-file=smoke/dirty.txt|changed elsewhere\n",
    "sleep=3", "dump=dirty-after-disk",
    "save", "until=10:sheet", "dump=conflict-sheet",
    "sheet-button=Cancel", "until=no-sheet", "sleep=1", "read-file=after-cancel|smoke/dirty.txt",
    "dump=conflict-cancelled",
    "save", "until=10:sheet", "sheet-button=Save Anyway", "until=no-sheet",
    "until=result:save:dirty.txt", "sleep=1", "dump=conflict-saved",
    # Files the editor refuses.
    f"open=smoke/{BIG_NAME}", "open=smoke/blob.bin", "open=smoke/latin1.txt",
    "sleep=1", "dump=refused",
    # The too-large toast's View in Terminal runs less on it, quoted for the shell.
    "toast-action=too large", "sleep=3", "dump=pager",
]


def check_editor(run) -> None:
    def opened(dump: dict, name: str, when: str) -> dict | None:
        found = editor(dump, name)
        return found if run.expect(found is not None, f"{name} is open ({when})") else None

    # Unicode edits, then save.
    edited = opened(run.dump("unicode-edited"), "unicode.txt", "after edits")
    if edited:
        run.expect(edited["modified"], "unicode.txt is marked modified after edits")
        run.expect(text(edited) != UNICODE_TEXT, "the edits changed unicode.txt's text")
    saved = opened(run.dump("unicode-saved"), "unicode.txt", "after save")
    if saved:
        disk = (SMOKE / "unicode.txt").read_text()
        run.expect(disk == text(saved), "unicode.txt on disk is exactly what the editor shows",
                   {"disk": disk, "editor": text(saved)})
        run.expect(not saved["modified"], "unicode.txt isn't modified after saving")

    # CRLF.
    crlf = opened(run.dump("crlf-saved"), "crlf.txt", "after save")
    if crlf:
        disk = (SMOKE / "crlf.txt").read_bytes().decode()
        run.expect(disk == text(crlf), "crlf.txt on disk is exactly what the editor shows",
                   {"disk": disk, "editor": text(crlf)})
        run.expect(re.search(r"(?<!\r)\n", disk) is None, "crlf.txt keeps CRLF on every line", repr(disk))
        run.expect("inserted" in disk and "pasted two" in disk, "crlf.txt has the typed and pasted lines", repr(disk))

    # Reload from disk.
    reloaded = opened(run.dump("reloaded"), "reload.txt", "after reload")
    if reloaded:
        run.expect(text(reloaded) == "version two\nfrom disk\n", "reload.txt shows the new file on disk",
                   text(reloaded))
        run.expect(not reloaded["modified"], "reload.txt isn't modified after a reload")
    undone = opened(run.dump("reload-undone"), "reload.txt", "after undoing the reload")
    if undone:
        run.expect(text(undone) == "version one\n", "undo brings back the text from before the reload",
                   text(undone))

    # Dirty buffer and a change on disk.
    dirty = opened(run.dump("dirty-after-disk"), "dirty.txt", "after the disk changed")
    if dirty:
        run.expect(text(dirty) == "edited keep me\n", "typing isn't replaced by a change on disk",
                   text(dirty))
    sheets = run.dump("conflict-sheet")["sheets"]
    texts = " ".join(t for sheet in sheets for t in sheet["texts"])
    run.expect("changed on disk" in texts, "saving over a changed file asks first", sheets)
    cancelled = run.dump("conflict-cancelled")["results"]
    run.expect(cancelled.get("save:dirty.txt") is False, "Cancel doesn't save", cancelled.get("save:dirty.txt"))
    run.expect(cancelled.get("after-cancel") == "changed elsewhere\n", "Cancel leaves the file on disk alone",
               cancelled.get("after-cancel"))
    final = opened(run.dump("conflict-saved"), "dirty.txt", "after Save Anyway")
    if final:
        run.expect((SMOKE / "dirty.txt").read_text() == text(final) == "edited keep me\n",
                   "Save Anyway writes the editor's text", (SMOKE / "dirty.txt").read_text())

    # Refused files.
    refused = run.dump("refused")
    for name in (BIG_NAME, "blob.bin", "latin1.txt"):
        run.expect(editor(refused, name) is None, f"{name} doesn't open in the editor")
    messages = toast_messages(refused)
    for expected in ("too large for the editor", "binary file the editor can't show", "isn't UTF-8 text"):
        run.expect(any(expected in m for m in messages), f"a toast says “{expected}”", messages)

    # The pager on the big file, quoted for zsh.
    pager = [t for t in surfaces(run.dump("pager"), "terminal") if "less" in plain(t["transcript"])
             or any("less" in (b["command"] or "") for b in t["blocks"])]
    if run.expect(pager, "View in Terminal opens a terminal running less"):
        transcript = plain(pager[-1]["transcript"])
        run.expect("No such file" not in transcript,
                   "less found the file with spaces and parentheses in its name", transcript[-400:])


# ---------------------------------------------------------------------------
# Terminal, in each shell: command blocks with their exit codes and folder,
# paths quoted for the shell (as drops and Impulse's own commands do), and
# shell integration marks that output can't fake. In zsh also the impulse
# CLI, including the commands only a pane's own shell may use.

QUOTING = SMOKE / "quoting"
QUOTING_NAMES = [
    "two words.txt", "it's.txt", 'say "hi".txt', "$HOME.txt", "back\\slash.txt", "star*.txt",
    "brace{a,b}.txt", "semi;colon.txt", "~lead.txt", "-dash.txt", "ünïcödé 🥾.txt", "hash#.txt",
    "paren(1).txt", "amp&.txt", "pipe|.txt", "lt<gt>.txt", "q?.txt", "excl!.txt", "percent%.txt",
    "back`tick.txt", "caret^.txt", "  spaced  .txt",
]
FORGED = [
    # The terminal's folder, a block's end with exit 7, a command for history,
    # a new prompt and command start: none carry the terminal's nonce.
    ("printf '\\033]7;file://localhost/private/tmp\\007'", "folder"),
    ("printf '\\033]133;D;7\\007'", "end"),
    ("printf '\\033]6973;Command=rm -rf /\\007'", "command"),
    ("printf '\\033]133;A\\007\\033]133;C\\007'", "prompt"),
]


def setup_terminal() -> None:
    for name in QUOTING_NAMES:
        write(QUOTING / name, "quoted\n")
    write(SMOKE / "cli-open.txt", "opened by impulse open\n")


def terminal_actions(cli: bool) -> list[str]:
    actions = ["until=prompt"]
    expected = 0

    def run(command: str, quoted: str | None = None) -> None:
        nonlocal expected
        expected += 1
        actions.append(f"run-quoted={command}|{quoted}" if quoted is not None else f"run={command}")
        actions.append(f"until=20:blocks:{expected}")

    run("echo smoke-start")
    run("false")
    run("cd smoke/quoting")
    for name in QUOTING_NAMES:
        run("test -e", name)
    for name in QUOTING_NAMES:
        run("test -e", str(QUOTING / name))
    for command, _ in FORGED:
        run(command)
    actions += ["sleep=1", "dump=shell"]
    if cli:
        actions += [
            # From a process that isn't the pane's shell or its child (as in
            # tmux): refused.
            f"run=( (sleep 1; impulse split; echo \"status=$?\") > {HOME / 'split-detached.out'} 2>&1 & )",
            f"until=file:{HOME / 'split-detached.out'}",
            f"run=( (sleep 1; impulse edit {SMOKE / 'detached-create.txt'}; echo \"status=$?\") "
            f"> {HOME / 'edit-detached.out'} 2>&1 & )",
            f"until=file:{HOME / 'edit-detached.out'}",
            "sleep=3", "dump=cli-detached",
            # From the pane's own shell: allowed.
            "run=impulse split", "until=panes:2", "until=prompt",
            "run=impulse tab", "until=tabs:2", "sleep=1", "select-terminal", "until=prompt",
            f"run=impulse open {SMOKE / 'cli-open.txt'}", "until=tabs:3", "sleep=1", "select-terminal",
            "until=prompt",
            f"run=impulse edit {SMOKE / 'created-by-cli.txt'} &", f"until=file:{SMOKE / 'created-by-cli.txt'}",
            "sleep=2", "dump=cli",
        ]
    return actions


def check_terminal(run, shell: str, cli: bool) -> None:
    dump = run.dump("shell")
    terminals = [t for t in surfaces(dump, "terminal")]
    if not run.expect(len(terminals) == 1, "one terminal before the CLI steps", len(terminals)):
        return
    terminal = terminals[0]
    run.expect(terminal["shell"] == shell, f"the terminal runs {shell}", terminal["shell"])
    blocks = [b for b in terminal["blocks"] if b["finished"]]
    commands = (["echo smoke-start", "false", "cd smoke/quoting"]
                + ["test -e"] * (2 * len(QUOTING_NAMES)) + [c for c, _ in FORGED])
    run.expect(len(blocks) == len(commands), "one block per command (forged marks add none)",
               [b["command"] for b in blocks])
    for index, (block, command) in enumerate(zip(blocks, commands)):
        exit_code = 1 if command == "false" else 0
        what = command if command != "test -e" else f"test -e {QUOTING_NAMES[index - 3] if index < 3 + len(QUOTING_NAMES) else QUOTING_NAMES[index - 3 - len(QUOTING_NAMES)]}"
        run.expect((block["command"] or "").startswith(command.split("'")[0].strip()),
                   f"block {index + 1} is “{what}”", block["command"])
        run.expect(block["exit"] == exit_code, f"“{what}” ended with {exit_code}", block["exit"])
    run.expect(terminal["cwd"] == str(QUOTING), "the folder is the one cd went to, not the forged one",
               terminal["cwd"])
    run.expect(not any((b["command"] or "") == "rm -rf /" for b in terminal["blocks"]),
               "a forged command isn't recorded")
    run.expect("smoke-start" in plain(terminal["transcript"]), "the output shows in the terminal")
    if not cli:
        return

    detached = run.dump("cli-detached")
    split_out = (HOME / "split-detached.out").read_text() if (HOME / "split-detached.out").exists() else ""
    edit_out = (HOME / "edit-detached.out").read_text() if (HOME / "edit-detached.out").exists() else ""
    run.expect("only works from the terminal's own shell" in split_out and "status=1" in split_out,
               "impulse split from outside the pane's shell is refused", split_out)
    run.expect("status=1" in edit_out and not (SMOKE / "detached-create.txt").exists(),
               "impulse edit can't create a file from outside the pane's shell", edit_out)
    run.expect(len(detached["tabs"]) == 1 and len(detached["tabs"][0]["surfaces"]) == 1,
               "the refused commands changed nothing", [t["title"] for t in detached["tabs"]])

    cli_dump = run.dump("cli")
    panes = max((len(t["surfaces"]) for t in cli_dump["tabs"]), default=0)
    run.expect(panes == 2, "impulse split adds a pane", [len(t["surfaces"]) for t in cli_dump["tabs"]])
    run.expect(len([t for t in cli_dump["tabs"] if t["surfaces"][0]["kind"] == "terminal"]) == 2,
               "impulse tab adds a terminal tab", [t["title"] for t in cli_dump["tabs"]])
    run.expect(editor(cli_dump, "cli-open.txt") is not None, "impulse open opens the file in an editor")
    run.expect((SMOKE / "created-by-cli.txt").exists() and editor(cli_dump, "created-by-cli.txt") is not None,
               "impulse edit creates the file and opens it")


def terminal_scenario(shell: str, path: str, cli: bool = False) -> dict:
    return {
        "name": f"terminal-{shell}",
        "about": f"blocks, quoting and forged marks in {shell}" + (", and the impulse CLI" if cli else ""),
        "session": ONE, "shell": path, "setup": setup_terminal, "actions": terminal_actions(cli),
        "check": lambda run: check_terminal(run, shell, cli),
    }


# ---------------------------------------------------------------------------
# Images: a script prints a PNG by the kitty graphics protocol, once placed
# directly and once shown by Unicode placeholder cells; `clear` takes the
# direct placement away.

IMAGES = SMOKE / "images"


def tiny_png(width: int, height: int) -> bytes:
    def chunk(kind: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    rows = b"".join(b"\0" + bytes([200, 80, 40, 255]) * width for _ in range(height))
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))


def setup_images() -> None:
    data = base64.b64encode(tiny_png(24, 16)).decode()
    # Two placeholder cells (U+10EEEE, the first with its row and column
    # marks, U+0305 twice) in image 2's color, 2 (a 256-color index).
    placeholders = "\\364\\216\\273\\256\\314\\205\\314\\205\\364\\216\\273\\256"
    write(IMAGES / "show.sh", (
        "#!/bin/sh\n"
        f"printf '\\033_Ga=T,f=100,i=1,q=2;{data}\\033\\\\'\n"
        f"printf '\\033_Ga=t,f=100,i=2,q=2;{data}\\033\\\\'\n"
        "printf '\\033_Ga=p,U=1,i=2,c=2,r=1,q=2\\033\\\\'\n"
        f"printf '\\n\\033[38;5;2m{placeholders}\\033[0m\\n'\n"
    ))


IMAGE_ACTIONS = [
    "until=prompt",
    f"run=sh {IMAGES / 'show.sh'}", "until=20:blocks:1", "sleep=1", "dump=shown",
    "run=clear", "until=20:blocks:2", "sleep=1", "dump=cleared",
]


def check_images(run) -> None:
    def images(name: str) -> dict:
        terminals = list(surfaces(run.dump(name), "terminal"))
        return terminals[0]["images"] if terminals else {}

    shown = images("shown")
    placements = shown.get("placements", [])
    run.expect(len(placements) == 1 and placements[0]["image"] == 1, "the PNG is placed in the terminal",
               placements)
    run.expect(shown.get("virtual") == 1, "the placeholder image has its virtual placement", shown)
    run.expect(shown.get("decoded") == 2, "both images decode", shown)
    cleared = images("cleared")
    run.expect(cleared.get("placements") == [], "clear takes the placed image away", cleared)


# ---------------------------------------------------------------------------
# Agents: the stand-in agent CLI (scripts/docs/agent.sh, as `claude`)
# reports its state through `impulse hook`; a turn that edits is
# checkpointed and can be reviewed; `impulse tasks` answers.


def check_agents(run) -> None:
    def agent(dump: dict) -> dict | None:
        found = [t for t in surfaces(dump, "terminal") if t["agent"]]
        return found[-1] if found else None

    for name, state in (("agent-waiting", "needsInput"), ("agent-turn", "done"), ("agent-working", "working")):
        found = agent(run.dump(name))
        if run.expect(found is not None, f"the agent is recognized ({name})"):
            run.expect(found["agent"] == "claude", "it's recognized as Claude Code", found["agent"])
            run.expect(found["agentState"] == state, f"its state is {state} ({name})", found["agentState"])
    refs = make_demo.git("for-each-ref", "--format=%(refname)", "refs/impulse/checkpoints/")
    run.expect(bool(refs.strip()), "the agent's turn is checkpointed under refs/impulse/checkpoints/", refs)
    review = run.dump("agent-review")
    run.expect(any(s["kind"] == "review" for s in surfaces(review)), "Review Last Turn opens a review")
    def subagents(name: str) -> dict | None:
        found = [t for t in surfaces(run.dump(name), "terminal") if t.get("subagents")]
        return found[0] if found else None

    waiting = subagents("subagents-waiting")
    if run.expect(waiting is not None, "the agent's subagents are listed"):
        run.expect(waiting["agentState"] == "working",
                   "a turn ended with subagents running keeps the agent working", waiting["agentState"])
        run.expect([(s["label"], s["running"]) for s in waiting["subagents"]]
                   == [("Write the photos endpoint", True), ("Write the photo tests", True)],
                   "both subagents run, named by their Agent calls", waiting["subagents"])
    halfway = subagents("subagents-one-reported")
    if run.expect(halfway is not None, "the subagents are listed while their reports arrive"):
        run.expect(halfway["agentState"] == "working",
                   "a turn that reports one finished subagent, with another still to report, keeps it working",
                   halfway["agentState"])
    finished = subagents("subagents-done")
    if run.expect(finished is not None, "the subagents are still listed once done"):
        run.expect(finished["agentState"] == "done", "the agent finishes with its subagents",
                   finished["agentState"])
        run.expect([s["result"] for s in finished["subagents"]]
                   == ["Added GET /trails/:id/photos", "Added test/photos.test.ts"],
                   "each subagent's report is kept", finished["subagents"])
    def subagent_workspaces(name: str) -> tuple[list, dict]:
        dump = run.dump(name)
        listed = [w for w in dump["workspaces"] if w.get("subagent")]
        active = next(w for w in dump["workspaces"] if w["active"])
        return listed, active

    listed, active = subagent_workspaces("subagent-worktrees-running")
    run.expect([(w["name"], w["subagent"]["running"]) for w in listed]
               == [("Use metres throughout", True), ("Convert at the edges", True)],
               "each subagent's worktree is listed as a workspace, named by its Agent call", listed)
    run.expect(all(w["task"] and w["tabs"] == 0 for w in listed),
               "they're tasks, not opened (no terminals yet)", listed)
    run.expect(not active.get("subagent"), "the agent's own workspace stays in front", active)
    listed, _ = subagent_workspaces("subagent-worktrees-done")
    run.expect([(w["name"], w["subagent"]["running"]) for w in listed] == [("Use metres throughout", False)],
               "a finished subagent's worktree stays, and one Claude Code removed goes with it", listed)
    notes_file = HOME / "mcp" / "subagent-notes.txt"
    notes = notes_file.read_text() if notes_file.exists() else ""
    for text, what in (
        ("shares the checkout with 1 other subagent still running", "the agent is told as it starts a second one"),
        ("other subagents are working in this same checkout", "the second subagent is told as it starts"),
        ('also edited by another subagent (\\"Write the photos endpoint\\")',
         "the second subagent is told the first changed the file"),
    ):
        run.expect(text in notes, what, notes[-600:])
    tasks = [t for t in surfaces(run.dump("final"), "terminal")
             if any((b["command"] or "").startswith("impulse tasks") for b in t["blocks"])]
    if run.expect(tasks, "impulse tasks ran"):
        block = next(b for b in tasks[0]["blocks"] if (b["command"] or "").startswith("impulse tasks"))
        run.expect(block["exit"] == 0, "impulse tasks succeeds", plain(tasks[0]["transcript"])[-400:])


AGENT_ACTIONS = [
    "until=prompt", "run=claude waiting", "until=30:agent:needsInput", "dump=agent-waiting",
    "newtab", "until=prompt", "run=claude turn", "until=40:agent:done", "sleep=1", "dump=agent-turn",
    "review-agent", "until=20:tabs:3", "sleep=1", "dump=agent-review",
    "newtab", "until=prompt", "run=claude working", "until=30:agent:working", "dump=agent-working",
    "newtab", "until=prompt", "run=claude subagents", "until=30:file:~/mcp/subagents-ready", "sleep=1",
    "dump=subagents-waiting",
    "newtab", "until=prompt", "run=touch ~/mcp/subagents-go", "until=30:file:~/mcp/subagents-one-reported",
    "sleep=1", "dump=subagents-one-reported",
    "run=touch ~/mcp/subagents-go-on", "until=30:file:~/mcp/subagents-finished", "sleep=1", "dump=subagents-done",
    "newtab", "until=prompt", "run=claude subagent-worktrees", "until=30:file:~/mcp/subagent-worktrees-ready",
    "sleep=1", "dump=subagent-worktrees-running",
    "newtab", "until=prompt", "run=touch ~/mcp/subagent-worktrees-go",
    "until=30:file:~/mcp/subagent-worktrees-finished", "sleep=1", "dump=subagent-worktrees-done",
    "newtab", "until=prompt", "run=impulse tasks", "until=20:blocks:1",
]


# ---------------------------------------------------------------------------
# Trust and language servers: a fresh install of the managed servers (the
# pinned versions, with --ignore-scripts), the trust question when a folder
# opens, TypeScript hover, completion, definition and a rename across files
# in a trusted folder, and nothing from servers in a restricted one.

TS_TRUSTED = HOME / "Code" / "smoke-ts"
TS_RESTRICTED = HOME / "Code" / "untrusted-ts"
TS_LOOSE = HOME / "Code" / "loose-ts"
GEO_TS = """export interface Point {
  lat: number;
  lon: number;
}

export function distanceKm(a: Point, b: Point): number {
  const dx = a.lat - b.lat;
  const dy = a.lon - b.lon;
  return Math.sqrt(dx * dx + dy * dy) * 111;
}
"""
MAIN_TS = """import { distanceKm, Point } from "./geo";

const start: Point = { lat: 47.5, lon: -121.7 };
const end: Point = { lat: 47.6, lon: -121.8 };
const lat = start.lat;
console.log(distanceKm(start, end), lat);
"""
MANAGED = make_demo.data_dir("impulse") / "lsp"


def setup_lsp() -> None:
    for root in (TS_TRUSTED, TS_RESTRICTED, TS_LOOSE):
        write(root / "package.json", json.dumps({"name": root.name, "private": True, "type": "module"}, indent=2))
        write(root / "tsconfig.json", json.dumps(
            {"compilerOptions": {"strict": True, "target": "ES2022", "module": "ESNext",
                                 "moduleResolution": "bundler"}, "include": ["src"]}, indent=2))
        write(root / "src" / "geo.ts", GEO_TS)
        write(root / "src" / "main.ts", MAIN_TS)
    # Install Impulse's managed servers the way the palette command does
    # (the command itself needs a key window, which a headless run has none of).
    # SMOKE_LSP_CACHE=1 reuses the last install (for working on the checks).
    cache = make_demo.REPO / "target" / "smoke" / "lsp-cache"
    if os.environ.get("SMOKE_LSP_CACHE") and cache.exists():
        shutil.copytree(cache, MANAGED, symlinks=True)
        (HOME / "lsp-install.log").write_text("(from target/smoke/lsp-cache)\n")
    else:
        env = dict(os.environ, HOME=str(HOME), XDG_DATA_HOME=str(HOME / ".local" / "share"))
        result = subprocess.run([str(capture.APP), "--install-lsp-servers"], env=env, capture_output=True, text=True,
                                timeout=900)
        (HOME / "lsp-install.log").write_text(result.stdout + result.stderr)
        if cache.exists():
            shutil.rmtree(cache)
        if MANAGED.exists():
            shutil.copytree(MANAGED, cache, symlinks=True)
    # Impulse looks on PATH before its managed folder, and a Mac may have its
    # own typescript-language-server there (Homebrew's PATH is always
    # included): name the managed one, so it's what gets tested.
    write(HOME / ".config" / "impulse" / "lsp.json", json.dumps({
        "servers": {"typescript-language-server": {
            "command": str(MANAGED / "node_modules" / ".bin" / "typescript-language-server"), "args": ["--stdio"]}},
        "language_servers": {"typescript": ["typescript-language-server"]},
    }, indent=2))


LSP_ACTIONS = [
    "trust-on",
    f"workspace={TS_TRUSTED}", "until=10:sheet", "dump=trust-sheet",
    "sheet-button=Trust Folder", "until=no-sheet",
    f"open={TS_TRUSTED}/src/main.ts", "until=editor",
    # `lsp=` asks the editor's server directly (EditorTab::debug_lsp), once
    # it has had a moment to load the project; a rename is applied as Rename
    # Symbol applies it, reaching main.ts too.
    "lsp-wait=ready:hover:5:14",
    "lsp=hover:hover:5:14",
    "lsp=completion:completion:4:18",
    "lsp=definition:definition:5:14",
    f"open={TS_TRUSTED}/src/geo.ts", "until=editor",
    "lsp=rename:rename:5:16:distanceInKm",
    "until=20:has:distanceInKm", "sleep=2", "dump=lsp",
    f"workspace={TS_RESTRICTED}", "until=10:sheet", "sheet-button=Stay Restricted", "until=no-sheet",
    f"open={TS_RESTRICTED}/src/main.ts", "until=editor", "sleep=8",
    "lsp=hover-restricted:hover:5:14",
    "sleep=1", "dump=restricted",
    # A file from a folder nobody was asked about (from a trusted workspace:
    # after Stay Restricted the notice keeps quiet): a notice says why.
    "select-workspace=smoke-ts",
    f"open={TS_LOOSE}/src/main.ts", "until=editor", "until=10:toast:Language servers are off",
]


def check_lsp(run) -> None:
    log = (HOME / "lsp-install.log").read_text() if (HOME / "lsp-install.log").exists() else ""
    server = MANAGED / "node_modules" / ".bin" / "typescript-language-server"
    run.expect(server.exists(), "the managed servers install", log[-600:])
    typescript = MANAGED / "node_modules" / "typescript" / "package.json"
    version = json.loads(typescript.read_text())["version"] if typescript.exists() else None
    run.expect(bool(version) and version.startswith("6."), "TypeScript is held to 6.x", version)

    sheets = run.dump("trust-sheet")["sheets"]
    texts = " ".join(t for sheet in sheets for t in sheet["texts"])
    run.expect("Do you trust the files in “smoke-ts”?" in texts, "opening a folder asks whether to trust it", sheets)

    dump = run.dump("lsp")
    results = dump["results"]
    workspace = next((w for w in dump["workspaces"] if w["root"] == str(TS_TRUSTED)), None)
    run.expect(workspace is not None and workspace["trusted"], "Trust Folder trusts it", workspace)

    def parsed(name: str):
        value = results.get(name)
        try:
            return json.loads(value) if isinstance(value, str) else value
        except ValueError:
            return value

    hover = parsed("hover")
    run.expect(bool(hover) and any("distanceKm" in h for h in hover), "hover describes distanceKm", hover)
    completion = parsed("completion")
    run.expect(bool(completion) and {"lat", "lon"} <= set(completion), "completion offers lat and lon", completion)
    definition = parsed("definition")
    run.expect(bool(definition) and str(definition.get("uri", "")).endswith("/smoke-ts/src/geo.ts"),
               "go to definition finds geo.ts", definition)
    run.expect(editor(dump, "smoke-ts/src/geo.ts") is not None, "go to definition opens geo.ts")
    main = editor(dump, "smoke-ts/src/main.ts")
    if run.expect(main is not None, "main.ts is open"):
        run.expect(text(main).count("distanceInKm") == 2, "rename changes both uses in main.ts", text(main))
    geo = editor(dump, "smoke-ts/src/geo.ts")
    geo_text = text(geo) if geo else (TS_TRUSTED / "src" / "geo.ts").read_text()
    run.expect("function distanceInKm" in geo_text, "rename reaches geo.ts, the other file", geo_text)

    restricted = run.dump("restricted")
    workspace = next((w for w in restricted["workspaces"] if w["root"] == str(TS_RESTRICTED)), None)
    run.expect(workspace is not None and not workspace["trusted"], "Stay Restricted leaves it untrusted", workspace)
    run.expect(parsed_value(restricted["results"].get("hover-restricted")) is None,
               "no hover from a server in a restricted folder", restricted["results"].get("hover-restricted"))
    messages = toast_messages(run.dump("final"))
    run.expect(any("Language servers are off" in m for m in messages),
               "a file from an untrusted folder says its language servers are off", messages)
    run.expect(not any("LSP error" in m for m in messages), "no language server errors", messages)


def parsed_value(value):
    try:
        return json.loads(value) if isinstance(value, str) else value
    except ValueError:
        return value


# ---------------------------------------------------------------------------
# Tasks: a task worktree is made, committed in and finished (a --no-ff merge
# pushed to the base without touching the main checkout, the branch and
# worktree removed); archiving can be undone; a task holding a nested
# repository isn't archived.

REPO_ROOT = HOME / "Code" / "smoke-repo"
REPO_REMOTE = make_demo.DEMO / "remotes" / "smoke-repo.git"
WORKTREES = HOME / "Code" / "smoke-repo.worktrees"
PROJECT_TOML = """[scripts]
check = "test -f README.md"
"""


def folder_session(root: Path) -> dict:
    return {
        "version": 2, "active_window_index": 0,
        "windows": [{
            "workspaces": [{"kind": "folder", "root": str(root), "active_tab_index": 0,
                            "tabs": [{"pinned": False, "panes": [{"kind": "terminal", "cwd": str(root)}]}]}],
            "active_workspace_index": 0, "sidebar_visible": True, "sidebar_width": 260,
        }],
    }


def git(cwd: Path, *args: str) -> str:
    return make_demo.git(*args, cwd=cwd)


def setup_tasks() -> None:
    REPO_REMOTE.parent.mkdir(parents=True, exist_ok=True)
    git(REPO_REMOTE.parent, "init", "-q", "--bare", "-b", "main", str(REPO_REMOTE))
    REPO_ROOT.mkdir(parents=True)
    git(REPO_ROOT, "init", "-q", "-b", "main")
    write(REPO_ROOT / "README.md", "# smoke-repo\n")
    write(REPO_ROOT / ".impulse" / "project.toml", PROJECT_TOML)
    make_demo.commit("Start", "maya", 1, cwd=REPO_ROOT)
    git(REPO_ROOT, "remote", "add", "origin", str(REPO_REMOTE))
    git(REPO_ROOT, "push", "-q", "-u", "origin", "main")


TASK_ACTIONS = [
    "until=prompt", "trust-project-settings",
    # Make a task, commit in it, finish it.
    "task=Smoke change", f"until=60:file:{WORKTREES / 'smoke-change'}", "until=20:prompt",
    "run=echo 'from the task' >> README.md && git commit -qam 'Change from the task'", "until=20:blocks:2",
    "finish-task", "until=15:tool:Finish", "sleep=2", "dump=finish-open",
    "finish-run=merge", f"until=120:no-file:{WORKTREES / 'smoke-change'}", "sleep=3", "dump=finished",
    # Archive a task, then Undo.
    "select-workspace=smoke-repo", "task=Archive me", f"until=60:file:{WORKTREES / 'archive-me'}",
    "until=20:prompt", "command=archive_task", "until=10:sheet", "dump=archive-asks",
    "sheet-button=Archive", f"until=30:no-file:{WORKTREES / 'archive-me'}", "until=10:toast:Archived",
    "sleep=1", "dump=archived",
    "toast-action=Archived", f"until=30:file:{WORKTREES / 'archive-me'}", "sleep=2", "dump=unarchived",
    # A task with a repository of its own inside isn't archived.
    "task=Nested repo", f"until=60:file:{WORKTREES / 'nested-repo'}", "until=20:prompt",
    "run=git init -q inner", "until=20:blocks:2", "command=archive_task", "sleep=3", "dump=nested",
]


def check_tasks(run) -> None:
    run.dump("finished")
    log = git(REPO_REMOTE, "log", "main", "--format=%s", "-3")
    run.expect("Change from the task" in log, "the task's commit reached origin/main", log)
    run.expect(any(line.startswith("Merge") and "smoke-change" in line for line in log.splitlines()),
               "it landed as a merge commit", log)
    run.expect(not git(REPO_ROOT, "branch", "--list", "smoke-change"), "the landed branch is deleted")
    run.expect(not (WORKTREES / "smoke-change").exists(), "the finished task's worktree is removed")
    status = git(REPO_ROOT, "status", "--porcelain")
    run.expect(status == "", "the main checkout has no changes", status)
    head, landed = git(REPO_ROOT, "rev-parse", "HEAD"), git(REPO_REMOTE, "rev-parse", "main")
    run.expect(head == landed, "the clean main checkout is fast-forwarded to what landed", (head, landed))

    asks = run.dump("archive-asks")["sheets"]
    texts = " ".join(t for sheet in asks for t in sheet["texts"])
    run.expect("Archive archive-me?" in texts and "branch archive-me is kept" in texts,
               "Archive Task asks first, and says the branch is kept", asks)
    archived = run.dump("archived")
    run.expect(not any(w["name"] == "archive-me" for w in archived["workspaces"]),
               "the archived task's workspace closes", [w["name"] for w in archived["workspaces"]])
    run.expect(bool(git(REPO_ROOT, "branch", "--list", "archive-me")), "archiving keeps the branch")
    archived_toast = next((t for t in archived["toasts"] if "Archived" in t["message"]), None)
    run.expect(archived_toast is not None and archived_toast["action"] == "Undo", "the Archived toast offers Undo",
               archived_toast)
    unarchived = run.dump("unarchived")
    run.expect((WORKTREES / "archive-me").exists() or any(w["name"] == "archive-me" for w in unarchived["workspaces"]),
               "Undo brings the task back", [w["name"] for w in unarchived["workspaces"]])
    worktrees = git(REPO_ROOT, "worktree", "list")
    run.expect("archive-me" in worktrees, "it's a worktree again", worktrees)

    nested = run.dump("nested")
    run.expect((WORKTREES / "nested-repo" / "inner" / ".git").exists(),
               "a task holding a repository of its own isn't archived")
    messages = toast_messages(nested)
    run.expect(any("nested-repo wasn't archived" in m for m in messages), "a toast says it wasn't archived", messages)


# ---------------------------------------------------------------------------
# Agent Integration installs Impulse's MCP server: a table in Codex's
# config.toml (read back as installed), and `claude mcp add-json` for
# Claude Code (the stand-in agent records the call).


AGENT_INTEGRATION_ACTIONS = [
    "agent-sheet=mcp:codex", "sleep=1", "dump=codex-preview", "agent-sheet-apply", "sleep=1",
    "agent-sheet=mcp:codex", "sleep=1", "dump=codex-installed", "agent-sheet-close", "sleep=1",
    "agent-sheet=mcp:claude", "sleep=2", "dump=claude-preview", "agent-sheet-apply", "sleep=1", "dump=claude-installed",
]


def check_agent_integration(run) -> None:
    preview = run.dump("codex-preview").get("agentSheet") or {}
    run.expect(preview.get("part") == "MCP Server" and not preview.get("installed"),
               "the sheet opens on the MCP server for Codex, not installed", preview)
    run.expect(any(line.startswith("+") and "[mcp_servers.impulse]" in line for line in preview.get("diff", [])),
               "its preview adds the [mcp_servers.impulse] table", preview.get("diff"))
    config = (HOME / ".codex" / "config.toml").read_text() if (HOME / ".codex" / "config.toml").exists() else ""
    run.expect('[mcp_servers.impulse]\ncommand = "sh"' in config, "Install writes the table to ~/.codex/config.toml",
               config)
    run.expect('env_vars = ["IMPULSE_SOCKET", "IMPULSE_PANE_TOKEN", "IMPULSE_CLI"]' in config,
               "it forwards the pane's variables (Codex clears the environment)", config)
    run.expect(str(capture.CLI) in config, "it falls back to the app's own impulse", config)
    installed = run.dump("codex-installed").get("agentSheet") or {}
    run.expect(installed.get("installed") is True, "reopened, the sheet says it's installed", installed)

    claude = run.dump("claude-preview").get("agentSheet") or {}
    run.expect(any(line.startswith("$ claude mcp add-json --scope user impulse") for line in claude.get("diff", [])),
               "for Claude Code the sheet shows the claude command it runs", claude)
    calls = [json.loads(line) for line in (HOME / "mcp" / "claude-mcp.txt").read_text().splitlines()] \
        if (HOME / "mcp" / "claude-mcp.txt").exists() else []
    run.expect(len(calls) == 1 and calls[0][:5] == ["mcp", "add-json", "--scope", "user", "impulse"],
               "Install runs claude mcp add-json --scope user impulse", calls)
    if calls and len(calls[0]) == 6:
        entry = json.loads(calls[0][5])
        run.expect(entry.get("command") == "sh" and entry.get("type") == "stdio", "the entry runs sh", entry)
        run.expect(entry.get("env", {}).get("IMPULSE_SOCKET") == "${IMPULSE_SOCKET:-}",
                   "the entry passes the pane's socket through", entry)
    run.expect(run.dump("claude-installed")["results"].get("agent-sheet") == "written", "the install succeeds",
               run.dump("claude-installed")["results"])


# ---------------------------------------------------------------------------
# opencode: Agent Integration replaces the plugin and MCP entry an older
# Impulse installed for opencode 1 with the plugin folder opencode 2 loads,
# taking the MCP entry out of an opencode.json with comments (which stay).
# A stand-in opencode (opencode-agent.mjs) loads that plugin's two halves as
# opencode's service and a terminal would, and plays turns through them, so
# the states, the other tasks' summary, a note about a shared file, the
# merge it asks about, the shell's pane, Impulse's tools and the session to
# resume all come from the real plugin talking to the real app.

OPENCODE_CONFIG = HOME / ".config" / "opencode"
OPENCODE_OUT = HOME / "opencode"
OPENCODE_JSON = """{
  // The user's own settings: Impulse leaves them as they are.
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "docs": { "type": "remote", "url": "https://docs.example.com/mcp" },
  },
}
"""
# What an older Impulse added for opencode 1.
OPENCODE_OLD_ENTRY = """    "impulse": {
      "type": "local",
      "command": ["sh", "-c", "exec \\"${IMPULSE_CLI:-/Applications/Impulse.app/Contents/Resources/bin/impulse}\\" mcp"]
    },
"""
OPENCODE_OLD_PLUGIN = "// Impulse's opencode plugin, installed by Impulse's Agent Integration.\nexport const ImpulsePlugin = async () => ({})\n"
OPENCODE_PLUGIN_SOURCE = (Path(__file__).resolve().parents[2] / "impulse-kit" / "resources" / "agents"
                          / "opencode-plugin")


def setup_opencode() -> None:
    write(OPENCODE_CONFIG / "opencode.json", OPENCODE_JSON.replace("  },\n}", OPENCODE_OLD_ENTRY + "  },\n}"))
    write(OPENCODE_CONFIG / "plugins" / "impulse.js", OPENCODE_OLD_PLUGIN)
    stand_in = HOME / "bin" / "opencode"
    write(stand_in, (Path(__file__).parent / "opencode-agent.mjs").read_text())
    stand_in.chmod(0o755)


OPENCODE_ACTIONS = [
    "agent-sheet=hooks:opencode", "sleep=1", "dump=oc-hooks-preview", "agent-sheet-apply", "sleep=1",
    "agent-sheet=mcp:opencode", "sleep=1", "dump=oc-mcp-preview", "agent-sheet-apply", "sleep=1",
    "agent-sheet=mcp:opencode", "sleep=1", "dump=oc-mcp-removed", "agent-sheet-close", "sleep=1",
    "until=prompt", "run=opencode waiting", "until=30:agent:needsInput", "dump=oc-waiting",
    "newtab", "until=prompt", "run=opencode turn", "until=40:agent:done", "sleep=1", "dump=oc-turn",
    "newtab", "until=prompt", "run=opencode merge", "until=30:agent:needsInput", "dump=oc-merge-asked",
    f"until=40:file:{OPENCODE_OUT / 'merge.json'}", "until=20:agent:done", "dump=oc-merge",
]


def check_opencode(run) -> None:
    hooks = run.dump("oc-hooks-preview").get("agentSheet") or {}
    run.expect(hooks.get("agent") == "opencode" and hooks.get("outdated")
               and hooks.get("path", "").endswith("/.config/opencode/plugins/impulse"),
               "the sheet offers opencode's plugin folder, over opencode 1's file", hooks)
    run.expect("older Impulse added" in (hooks.get("notice") or ""),
               "it points out the MCP entry opencode 1 used", hooks.get("notice"))
    folder = OPENCODE_CONFIG / "plugins" / "impulse"
    for name in ("rpc.js", "server.js", "tui.js"):
        run.expect((folder / name).exists() and (folder / name).read_text() == (OPENCODE_PLUGIN_SOURCE / name).read_text(),
                   f"Install writes the plugin's {name} as shipped")
    run.expect(not (OPENCODE_CONFIG / "plugins" / "impulse.js").exists(), "and removes opencode 1's single file")
    mcp = run.dump("oc-mcp-preview").get("agentSheet") or {}
    run.expect(mcp.get("installed") and any(line.startswith("- ") and '"impulse": {' in line for line in mcp.get("diff", [])),
               "the MCP side offers to take out the old entry", mcp.get("diff"))
    config = (OPENCODE_CONFIG / "opencode.json").read_text()
    run.expect(config == OPENCODE_JSON, "only the old entry goes: comments, servers and trailing commas stay", config)
    removed = run.dump("oc-mcp-removed").get("agentSheet") or {}
    run.expect(removed.get("installed") is False and "install it under Hooks" in (removed.get("notice") or ""),
               "reopened, the sheet says the tools come with the plugin", removed)

    def stand_in(dump: dict, mode: str) -> dict | None:
        return next((t for t in surfaces(dump, "terminal")
                     if f"opencode (stand-in): {mode}" in plain(t["transcript"])), None)

    def output(mode: str) -> dict:
        file = OPENCODE_OUT / f"{mode}.json"
        return json.loads(file.read_text()) if file.exists() else {}

    for name, mode, state in (("oc-waiting", "waiting", "needsInput"), ("oc-turn", "turn", "done"),
                              ("oc-merge-asked", "merge", "needsInput"), ("oc-merge", "merge", "done")):
        found = stand_in(run.dump(name), mode)
        if run.expect(found is not None and found["agent"] == "opencode", f"it's recognized as opencode ({name})",
                      found and found["agent"]):
            run.expect(found["agentState"] == state, f"its plugin reports {state} ({name})", found["agentState"])
    turn = stand_in(run.dump("oc-turn"), "turn") or {}
    run.expect(turn.get("resume") == "opencode --session ses_smoketurn", "the session to resume comes from the plugin",
               turn.get("resume"))
    system = "\n".join(output("turn").get("system", []))
    run.expect("main checkout of trailhead" in system and "add-trail-photos" in system,
               "a new session gets the other tasks' summary beside its instructions", system)
    edit = output("turn").get("editOutput", "")
    run.expect("src/photos.ts is also changed in the task add-trail-photos" in edit,
               "an edit of a file a task also changes gets a note after the tool's output", edit)
    run.expect("list_tasks" in output("turn").get("tools", []), "Impulse's tools come with the plugin",
               output("turn").get("tools"))
    tasks = output("turn").get("listTasks") or ""
    run.expect("add-trail-photos" in tasks, "a tool call runs in the session's own pane", tasks)
    merge = output("merge")
    asked = merge.get("asked", [])
    run.expect(len(asked) == 1 and "add-trail-photos" in asked[0],
               "a merge of a busy task's branch is asked about, with Impulse's reason", asked)
    run.expect(merge.get("shellIsThisPane") is True,
               "the command's shell gets its own pane's identity, not the service's", merge)


# ---------------------------------------------------------------------------
# The MCP server: an agent's tool calls, through `impulse mcp` in a pane,
# list the tasks, read the review comments and ask for a task, which waits
# for the New Task sheet.


MCP_DIR = HOME / "mcp"


def mcp_call(id: int, method: str, params: dict | None = None) -> str:
    return json.dumps({"jsonrpc": "2.0", "id": id, "method": method, "params": params or {}})


def setup_mcp() -> None:
    setup_tasks()
    lines = [
        mcp_call(1, "initialize", {"protocolVersion": "2025-11-25", "capabilities": {},
                                   "clientInfo": {"name": "smoke", "version": "1"}}),
        json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        mcp_call(2, "tools/list"),
        mcp_call(3, "tools/call", {"name": "list_tasks", "arguments": {}}),
        mcp_call(4, "tools/call", {"name": "get_project_config", "arguments": {}}),
        mcp_call(5, "tools/call", {"name": "create_task", "arguments": {
            "title": "From the agent", "brief": "- Say hello from the task.\n- Then stop."}}),
        # Answered while create_task waits for the sheet.
        mcp_call(6, "tools/call", {"name": "notify", "arguments": {"title": "From the smoke check"}}),
        mcp_call(7, "tools/call", {"name": "list_command_blocks", "arguments": {}}),
        mcp_call(8, "tools/call", {"name": "list_unsaved_files", "arguments": {}}),
    ]
    write(MCP_DIR / "in.jsonl", "\n".join(lines) + "\n")
    # How work lands is settled, so an agent's Finish runs up to Land.
    write(REPO_ROOT / ".git" / "impulse" / "project.toml", '[finish]\nland = "merge"\n')


MCP_ACTIONS = [
    "until=prompt", "trust-project-settings",
    # Standard input stays open while create_task waits for the sheet.
    f"run=(cat {MCP_DIR / 'in.jsonl'}; sleep 120) | impulse mcp > {MCP_DIR / 'out.jsonl'} &",
    "until=30:sheet", "sleep=2", "dump=asks",
    "task-sheet-create", f"until=60:file:{WORKTREES / 'from-the-agent'}", "sleep=4", "dump=created",
    # The task's agent commits, then, mid-turn, asks to finish its own task:
    # Finish doesn't wait for it, and stops at Land for the user.
    "until=20:prompt",
    f"run=echo 'from the agent' >> README.md && git commit -qam 'Change from the agent' && touch {MCP_DIR / 'committed'}",
    f"until=20:file:{MCP_DIR / 'committed'}", "until=20:prompt",
    "run=claude finish", "until=30:agent:working", f"until=180:file:{MCP_DIR / 'finish.txt'}", "sleep=1",
    "dump=agent-finish",
]


def check_mcp(run) -> None:
    answers = {}
    out = MCP_DIR / "out.jsonl"
    for line in (out.read_text().splitlines() if out.exists() else []):
        message = json.loads(line)
        if "id" in message:
            answers[message["id"]] = message
    run.expect(answers.get(1, {}).get("result", {}).get("serverInfo", {}).get("name") == "impulse",
               "initialize names the server", answers.get(1))
    tools = [t["name"] for t in answers.get(2, {}).get("result", {}).get("tools", [])]
    run.expect({"list_tasks", "create_task", "get_diagnostics", "list_review_comments"} <= set(tools),
               "tools/list offers Impulse's tools", tools)
    listing = answers.get(3, {}).get("result", {})
    run.expect(listing.get("structuredContent", {}).get("repository") == "smoke-repo",
               "list_tasks answers with the repository's workspaces", listing)
    project = answers.get(4, {}).get("result", {})
    run.expect("check" in json.dumps(project.get("structuredContent", {})), "get_project_config lists the trusted scripts",
               project)

    sheet = run.dump("asks").get("taskSheet") or {}
    run.expect(sheet.get("title") == "From the agent" and "smoke-repo" in (sheet.get("requestedBy") or ""),
               "create_task shows New Task filled in, saying an agent asked", sheet)
    run.expect(sheet.get("brief", "").startswith("- Say hello"), "the sheet shows the brief", sheet)

    created = answers.get(5, {}).get("result", {})
    run.expect(created.get("isError") is False, "create_task answers once the task is created", created)
    run.expect(created.get("structuredContent", {}).get("workspace", {}).get("name") == "from-the-agent",
               "create_task answers with the new task", created)
    run.expect(created.get("structuredContent", {}).get("workspace", {}).get("isCaller") is False,
               "create_task's new task isn't the caller's workspace", created)
    notified = answers.get(6, {}).get("result", {})
    run.expect(notified.get("isError") is False and "structuredContent" not in notified,
               "a text-only answer (notify) is a plain result", notified)
    blocks = answers.get(7, {}).get("result", {}).get("structuredContent", {}).get("terminals", [])
    run.expect(any(t.get("isCaller") for t in blocks), "list_command_blocks lists the caller's terminal", blocks)
    unsaved = answers.get(8, {}).get("result", {})
    run.expect(unsaved.get("isError") is False and unsaved.get("structuredContent", {}).get("files") == [],
               "list_unsaved_files answers", unsaved)
    run.expect((WORKTREES / "from-the-agent").exists(), "the task's worktree is there")
    registry = json.loads((REPO_ROOT / ".git" / "impulse" / "tasks.json").read_text())
    record = next((t for t in registry["tasks"] if t["branch"] == "from-the-agent"), {})
    run.expect(record.get("createdBy") == "agent" and "smoke-repo" in (record.get("requestedBy") or ""),
               "the task is recorded as an agent's, with who asked", record)
    brief = REPO_ROOT / ".git" / "impulse" / "tasks" / "from-the-agent" / "brief.md"
    run.expect(brief.exists() and brief.read_text().startswith("- Say hello"), "the brief is kept with the task")
    status = git(REPO_ROOT, "status", "--porcelain")
    run.expect(status == "", "nothing is written into the main checkout", status)

    finish = (MCP_DIR / "finish.txt").read_text() if (MCP_DIR / "finish.txt").exists() else ""
    run.expect("Ready to land" in finish and "exit 0" in finish,
               "an agent finishing its own task, mid-turn, gets to Land", finish)
    run.expect("is working in this task" not in finish, "Finish doesn't wait for the agent that asked", finish)
    landed = git(REPO_REMOTE, "log", "main", "--format=%s")
    run.expect("Change from the agent" not in landed, "landing waits for the user", landed)


# ---------------------------------------------------------------------------
# Scratch: Scratch Workspace opens it (once), it closes beside others, Undo
# brings it back, and it's kept as a window's last workspace.


SCRATCH_ACTIONS = [
    "until=prompt", "dump=start",
    "scratch-workspace", "until=prompt", "dump=opened",
    "scratch-workspace", "sleep=1", "dump=again",
    "close-workspace", "sleep=1", "dump=closed",
    "reopen", "sleep=1", "dump=reopened",
    f"workspace={PROJECT}", "sleep=1", "close-workspace", "sleep=1", "dump=folder-closed",
    "close-workspace", "sleep=1", "dump=kept",
]


def check_scratch(run) -> None:
    def scratches(name: str) -> list[dict]:
        return [w for w in run.dump(name)["workspaces"] if w["kind"] == "scratch"]

    def scratch_tabs(name: str) -> int:
        return sum(1 for tab in run.dump(name)["tabs"] if tab["workspace"] == "Scratch")

    run.expect(scratches("start") == [], "the session starts without Scratch", scratches("start"))
    opened = scratches("opened")
    run.expect(len(opened) == 1 and opened[0]["active"], "Scratch Workspace adds Scratch and shows it", opened)
    run.expect(scratch_tabs("opened") == 1, "with a terminal in it", scratch_tabs("opened"))
    run.expect(len(scratches("again")) == 1 and scratch_tabs("again") == 1,
               "a second time shows the same Scratch", run.dump("again")["workspaces"])
    run.expect(scratches("closed") == [], "Scratch closes beside a folder", run.dump("closed")["workspaces"])
    run.expect(len(scratches("reopened")) == 1 and scratch_tabs("reopened") == 1,
               "Reopen brings Scratch back with its terminal", run.dump("reopened")["workspaces"])
    left = run.dump("folder-closed")["workspaces"]
    run.expect([w["kind"] for w in left] == ["scratch"], "closing the folder leaves Scratch", left)
    kept = run.dump("kept")["workspaces"]
    run.expect([w["kind"] for w in kept] == ["scratch"], "a window's last workspace stays", kept)


# ---------------------------------------------------------------------------
# Session restore: workspaces, tabs, a split, an editor and terminal output
# saved by one launch come back in the next.


SESSION_ACTIONS = [
    "until=prompt", "run=echo restore-marker-42", "until=blocks:1",
    "run=impulse split", "until=panes:2", "until=prompt",
    f"open={PROJECT / 'README.md'}", "until=editor",
    f"workspace={HOME / 'Code' / 'trail-ui'}", "sleep=2",
    "save-session", "sleep=1", "dump=before",
]


def check_session(run) -> None:
    before, after = run.dump("before"), run.dump("after", launch=2)

    def shape(dump: dict) -> list:
        return [(tab["workspace"], [(s["kind"], s.get("path")) for s in tab["surfaces"]]) for tab in dump["tabs"]]

    run.expect([w["root"] for w in after["workspaces"]] == [w["root"] for w in before["workspaces"]],
               "the same workspaces come back", [w["root"] for w in after["workspaces"]])
    run.expect(shape(after) == shape(before), "the same tabs, splits and files come back",
               {"before": shape(before), "after": shape(after)})
    restored = [t for t in surfaces(after, "terminal") if "restore-marker-42" in plain(t["transcript"])]
    run.expect(bool(restored), "terminal output comes back")


# ---------------------------------------------------------------------------

SCENARIOS = [
    {"name": "editor", "about": "editing, saving, reloading and refusing files",
     "session": ONE, "setup": setup_editor, "actions": EDITOR_ACTIONS, "check": check_editor},
    terminal_scenario("zsh", "/bin/zsh", cli=True),
    terminal_scenario("bash", "/bin/bash"),
    terminal_scenario("fish", "/opt/homebrew/bin/fish"),
    {"name": "images", "about": "a PNG by the kitty graphics protocol, placed and by placeholders, then cleared",
     "session": ONE, "setup": setup_images, "actions": IMAGE_ACTIONS, "check": check_images},
    {"name": "scratch", "about": "Scratch Workspace, closing Scratch and bringing it back",
     "session": ONE, "actions": SCRATCH_ACTIONS, "check": check_scratch},
    {"name": "agents", "about": "agent states from hooks, subagents, checkpoints, review last turn, impulse tasks",
     "session": ONE, "actions": AGENT_ACTIONS, "check": check_agents},
    {"name": "lsp", "about": "server install, the trust question, TypeScript features, restricted folders",
     "session": ONE, "setup": setup_lsp, "settings": {"workspace_trust": True}, "lsp": True,
     "actions": LSP_ACTIONS, "check": check_lsp, "timeout": 420},
    {"name": "tasks", "about": "new task, Finish Task (merge), archive and Undo, nested repositories",
     "session": folder_session(REPO_ROOT), "setup": setup_tasks, "actions": TASK_ACTIONS, "check": check_tasks,
     "timeout": 360},
    {"name": "agent-integration", "about": "Agent Integration installs the MCP server for Codex and Claude Code",
     "session": ONE, "actions": AGENT_INTEGRATION_ACTIONS,
     "check": check_agent_integration},
    {"name": "opencode", "about": "Agent Integration replaces opencode 1's plugin and MCP entry with opencode 2's "
                                  "plugin folder, which drives states, the summary, notes, the merge question, "
                                  "shells' pane, Impulse's tools and resume",
     "session": ONE, "setup": setup_opencode, "actions": OPENCODE_ACTIONS, "check": check_opencode},
    {"name": "mcp", "about": "impulse mcp: initialize, tools/list, list_tasks, create_task through the New Task sheet, "
                             "an agent finishing its own task",
     "session": folder_session(REPO_ROOT), "setup": setup_mcp, "actions": MCP_ACTIONS, "check": check_mcp,
     "timeout": 360},
    {"name": "session", "about": "workspaces, tabs, splits, editors and output survive a relaunch",
     "session": ONE, "persist": True, "actions": SESSION_ACTIONS, "relaunch": ["sleep=4", "dump=after"],
     "check": check_session},
]
