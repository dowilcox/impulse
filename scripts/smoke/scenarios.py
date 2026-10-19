"""The smoke scenarios (see run.py).

Each scenario:
  name, about   shown by --list
  session       the window to start with (make_demo.session)
  setup         makes its files, after the demo world is built
  settings      keys merged into the demo's settings.json
  shell         the terminals' shell (default /bin/zsh)
  lsp           start language servers (off by default)
  persist       save settings and the session as a normal launch does
  actions       debug actions, run in order (MainWindowController+Debug.swift)
  relaunch      actions for a second launch that restores the saved session
  timeout       seconds before the app is stopped (default 240)
  check(run)    reads the dumps and files and calls run.expect(...)
"""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
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
# Editor: Impulse's copy of each buffer follows Monaco's edits exactly, saves
# write what's on screen, reloads from disk are in place and undoable, a
# change on disk never silently loses typing, and files the editor can't
# show are refused with a toast.

UNICODE_TEXT = "Trail notes 🥾\n日本語の道\ncafé ñ — naïve\nend\n"
CRLF_TEXT = "first\r\nsecond\r\nthird\r\n"
BIG_NAME = "big log (copy).log"

# Typing, multi-cursor typing, a paste, one edit made of several changes,
# deleting across an emoji, a new line, then undo/undo/redo.
EDIT_UNICODE = r"""
const m = editor.getModel();
const end = (n) => ({ lineNumber: n, column: m.getLineMaxColumn(n) });
editor.setPosition(end(1));
editor.trigger("keyboard", "type", { text: " ⛰️ day 1" });
editor.setSelections([1, 2, 3].map((i) => new monaco.Selection(i + 1, 1, i + 1, 1)));
editor.trigger("keyboard", "type", { text: "» " });
editor.setPosition(end(4));
editor.trigger("keyboard", "paste", { text: "\npasted 🧭 line\nand another" });
editor.executeEdits("smoke", [
  { range: new monaco.Range(2, 3, 2, 4), text: "道🌲" },
  { range: new monaco.Range(3, 1, 3, 1), text: "👣" },
  { range: new monaco.Range(5, 1, 5, 7), text: "" },
]);
const boot = m.getLineContent(1).indexOf("🥾");
editor.executeEdits("smoke", [{ range: new monaco.Range(1, boot - 1, 1, boot + 3), text: "" }]);
editor.setPosition({ lineNumber: 2, column: 4 });
editor.trigger("keyboard", "type", { text: "\n" });
editor.trigger("smoke", "undo");
editor.trigger("smoke", "undo");
editor.trigger("smoke", "redo");
return editor.getValue();
"""

EDIT_CRLF = r"""
const m = editor.getModel();
editor.setPosition({ lineNumber: 2, column: 4 });
editor.trigger("keyboard", "type", { text: "\ninserted" });
editor.setPosition({ lineNumber: m.getLineCount(), column: 1 });
editor.trigger("keyboard", "paste", { text: "pasted one\npasted two\n" });
return JSON.stringify({ eol: m.getEOL() === "\r\n" ? "crlf" : "lf", value: editor.getValue() });
"""

TYPE_AT_START = r"""
editor.setPosition({ lineNumber: 1, column: 1 });
editor.trigger("keyboard", "type", { text: "edited " });
return editor.getValue();
"""


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
    "js=unicode:" + EDIT_UNICODE,
    "sleep=1", "dump=unicode-edited",
    "save", "until=result:save:unicode.txt", "sleep=1", "dump=unicode-saved",
    # CRLF line endings stay CRLF.
    "open=smoke/crlf.txt", "until=editor",
    "js=crlf:" + EDIT_CRLF,
    "sleep=1", "save", "until=result:save:crlf.txt", "sleep=1", "dump=crlf-saved",
    # A clean buffer follows the file on disk, and the reload can be undone.
    "open=smoke/reload.txt", "until=editor",
    "write-file=smoke/reload.txt|version two\nfrom disk\n",
    "until=15:js:editor.getValue() === 'version two\\nfrom disk\\n'",
    "sleep=1", "dump=reloaded",
    "js=undo-reload:editor.trigger('smoke', 'undo'); return editor.getValue();",
    "sleep=1", "dump=reload-undone",
    # A dirty buffer isn't replaced by a change on disk; saving then asks.
    "open=smoke/dirty.txt", "until=editor",
    "js=dirty:" + TYPE_AT_START,
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
    def same_copy(dump: dict, name: str, when: str) -> dict | None:
        found = editor(dump, name)
        if not run.expect(found is not None, f"{name} is open ({when})"):
            return None
        run.expect(found["swift"] == found["monaco"], f"{name}: Impulse's copy matches Monaco ({when})",
                   {"swift": found["swift"], "monaco": found["monaco"]})
        return found

    # Unicode edits, then save.
    edited = same_copy(run.dump("unicode-edited"), "unicode.txt", "after edits")
    if edited:
        run.expect(edited["modified"], "unicode.txt is marked modified after edits")
        run.expect(edited["monaco"] != UNICODE_TEXT, "the edits changed unicode.txt's text")
    saved = same_copy(run.dump("unicode-saved"), "unicode.txt", "after save")
    if saved:
        disk = (SMOKE / "unicode.txt").read_text()
        run.expect(disk == saved["monaco"], "unicode.txt on disk is exactly what Monaco shows",
                   {"disk": disk, "monaco": saved["monaco"]})
        run.expect(not saved["modified"], "unicode.txt isn't modified after saving")

    # CRLF.
    crlf = same_copy(run.dump("crlf-saved"), "crlf.txt", "after save")
    if crlf:
        disk = (SMOKE / "crlf.txt").read_bytes().decode()
        run.expect(disk == crlf["monaco"], "crlf.txt on disk is exactly what Monaco shows",
                   {"disk": disk, "monaco": crlf["monaco"]})
        run.expect(re.search(r"(?<!\r)\n", disk) is None, "crlf.txt keeps CRLF on every line", repr(disk))
        run.expect("inserted" in disk and "pasted two" in disk, "crlf.txt has the typed and pasted lines", repr(disk))

    # Reload from disk.
    reloaded = same_copy(run.dump("reloaded"), "reload.txt", "after reload")
    if reloaded:
        run.expect(reloaded["monaco"] == "version two\nfrom disk\n", "reload.txt shows the new file on disk",
                   reloaded["monaco"])
        run.expect(not reloaded["modified"], "reload.txt isn't modified after a reload")
    undone = same_copy(run.dump("reload-undone"), "reload.txt", "after undoing the reload")
    if undone:
        run.expect(undone["monaco"] == "version one\n", "undo brings back the text from before the reload",
                   undone["monaco"])

    # Dirty buffer and a change on disk.
    dirty = same_copy(run.dump("dirty-after-disk"), "dirty.txt", "after the disk changed")
    if dirty:
        run.expect(dirty["monaco"] == "edited keep me\n", "typing isn't replaced by a change on disk",
                   dirty["monaco"])
    sheets = run.dump("conflict-sheet")["sheets"]
    texts = " ".join(t for sheet in sheets for t in sheet["texts"])
    run.expect("changed on disk" in texts, "saving over a changed file asks first", sheets)
    cancelled = run.dump("conflict-cancelled")["results"]
    run.expect(cancelled.get("save:dirty.txt") is False, "Cancel doesn't save", cancelled.get("save:dirty.txt"))
    run.expect(cancelled.get("after-cancel") == "changed elsewhere\n", "Cancel leaves the file on disk alone",
               cancelled.get("after-cancel"))
    final = same_copy(run.dump("conflict-saved"), "dirty.txt", "after Save Anyway")
    if final:
        run.expect((SMOKE / "dirty.txt").read_text() == final["monaco"] == "edited keep me\n",
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
MANAGED = HOME / "Library" / "Application Support" / "impulse" / "lsp"
PENDING = {"Hover": "pendingHovers", "Completion": "pendingCompletions", "Definition": "pendingDefinitions",
           "Rename": "pendingRename"}
SHAPES = {
    "Hover": "answer && answer.contents.map((c) => c.value)",
    "Completion": "answer && answer.suggestions.map((s) => (typeof s.label === 'string' ? s.label : s.label.label))",
    "Definition": "answer && { uri: String(answer.uri), line: answer.range.startLineNumber }",
    "Rename": "answer",
}


def lsp_request(kind: str, line: int, character: int, extra: dict | None = None) -> str:
    """What Monaco's provider for `kind` sends Impulse, answered the same way."""
    pending = PENDING[kind]
    store = "{ resolve: settle, reject: (e) => settle({ error: String(e) }) }" if kind == "Rename" else "settle"
    message = {"type": f"{kind}Requested", "line": line, "character": character, **(extra or {})}
    return f"""
const id = ++requestSeq;
const answer = await new Promise((resolve) => {{
  const settle = (value) => resolve(value === undefined ? null : value);
  {pending}.set(id, {store});
  sendToHost(Object.assign({json.dumps(message)}, {{ request_id: id }}));
  setTimeout(() => {{ if ({pending}.has(id)) {{ {pending}.delete(id); settle(null); }} }}, 15000);
}});
return JSON.stringify({SHAPES[kind]});
"""


HOVER = lsp_request("Hover", 5, 14)


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
        env = dict(os.environ, HOME=str(HOME), CFFIXED_USER_HOME=str(HOME))
        app = Path(capture.APP) / "Contents" / "MacOS" / "Impulse Dev"
        result = subprocess.run([str(app), "--install-lsp-servers"], env=env, capture_output=True, text=True,
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
    # The server needs a moment to load the project.
    "until=120:js:JSON.parse(await (async () => {" + HOVER + "})()) !== null",
    "js=hover:" + HOVER,
    "js=completion:" + lsp_request("Completion", 4, 18),
    "js=definition:" + lsp_request("Definition", 5, 14),
    # Go to Definition as the editor runs it: geo.ts opens.
    # Opening the definition in another file, through the editor opener
    # Monaco calls for ⌘-click and Go to Definition.
    "js=open-definition:await editor._codeEditorService.openCodeEditor({ resource: monaco.Uri.file("
    + json.dumps(str(TS_TRUSTED / "src" / "geo.ts"))
    + "), options: { selection: { startLineNumber: 6, startColumn: 17, endLineNumber: 6, endColumn: 17 } } },"
    " editor); return true;",
    "until=15:js:currentFilePath.endsWith('/smoke-ts/src/geo.ts')", "until=editor",
    # Renaming the declaration reaches main.ts too (applied by Impulse, not Monaco).
    "js=rename:" + lsp_request("Rename", 5, 16, {"new_name": "distanceInKm"}),
    "until=20:js:editor.getValue().includes('distanceInKm')", "sleep=2", "dump=lsp",
    f"workspace={TS_RESTRICTED}", "until=10:sheet", "sheet-button=Stay Restricted", "until=no-sheet",
    f"open={TS_RESTRICTED}/src/main.ts", "until=editor", "sleep=8",
    "js=hover-restricted:" + HOVER, "sleep=1", "dump=restricted",
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
        run.expect(main["monaco"].count("distanceInKm") == 2, "rename changes both uses in main.ts", main["monaco"])
        run.expect(main["swift"] == main["monaco"], "Impulse's copy of main.ts matches Monaco after the rename")
    geo = editor(dump, "smoke-ts/src/geo.ts")
    geo_text = geo["monaco"] if geo else (TS_TRUSTED / "src" / "geo.ts").read_text()
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
            "title": "From the agent", "brief": "Say hello from the task.\nThen stop."}}),
    ]
    write(MCP_DIR / "in.jsonl", "\n".join(lines) + "\n")


MCP_ACTIONS = [
    "until=prompt", "trust-project-settings",
    # Standard input stays open while create_task waits for the sheet.
    f"run=(cat {MCP_DIR / 'in.jsonl'}; sleep 120) | impulse mcp > {MCP_DIR / 'out.jsonl'} &",
    "until=30:sheet", "sleep=2", "dump=asks",
    "sheet-button=Create Task", f"until=60:file:{WORKTREES / 'from-the-agent'}", "sleep=4", "dump=created",
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

    asks = run.dump("asks")["sheets"]
    texts = " ".join(t for sheet in asks for t in sheet["texts"])
    run.expect("wants to start this task" in texts, "create_task shows New Task, saying an agent asked", asks)

    created = answers.get(5, {}).get("result", {})
    run.expect(created.get("isError") is False, "create_task answers once the task is created", created)
    run.expect((WORKTREES / "from-the-agent").exists(), "the task's worktree is there")
    registry = json.loads((REPO_ROOT / ".git" / "impulse" / "tasks.json").read_text())
    record = next((t for t in registry["tasks"] if t["branch"] == "from-the-agent"), {})
    run.expect(record.get("createdBy") == "agent", "the task is recorded as an agent's", record)
    brief = REPO_ROOT / ".git" / "impulse" / "tasks" / "from-the-agent" / "brief.md"
    run.expect(brief.exists() and "Say hello" in brief.read_text(), "the brief is kept with the task")
    status = git(REPO_ROOT, "status", "--porcelain")
    run.expect(status == "", "nothing is written into the main checkout", status)


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
    {"name": "agents", "about": "agent states from hooks, checkpoints, review last turn, impulse tasks",
     "session": ONE, "actions": AGENT_ACTIONS, "check": check_agents},
    {"name": "lsp", "about": "server install, the trust question, TypeScript features, restricted folders",
     "session": ONE, "setup": setup_lsp, "settings": {"workspace_trust": True}, "lsp": True,
     "actions": LSP_ACTIONS, "check": check_lsp, "timeout": 420},
    {"name": "tasks", "about": "new task, Finish Task (merge), archive and Undo, nested repositories",
     "session": folder_session(REPO_ROOT), "setup": setup_tasks, "actions": TASK_ACTIONS, "check": check_tasks,
     "timeout": 360},
    {"name": "mcp", "about": "impulse mcp: initialize, tools/list, list_tasks, create_task through the New Task sheet",
     "session": folder_session(REPO_ROOT), "setup": setup_mcp, "actions": MCP_ACTIONS, "check": check_mcp,
     "timeout": 240},
    {"name": "session", "about": "workspaces, tabs, splits, editors and output survive a relaunch",
     "session": ONE, "persist": True, "actions": SESSION_ACTIONS, "relaunch": ["sleep=4", "dump=after"],
     "check": check_session},
]
