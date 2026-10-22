#!/usr/bin/env node
// Stand-in for opencode 2 in the smoke checks, installed as ~/bin/opencode
// in the demo home. It loads Impulse's plugin from where opencode would
// (~/.config/opencode/plugins/impulse/, put there by Agent Integration):
// server.js as opencode's background service does and tui.js as each
// terminal does, joined by an in-process stand-in for opencode's plugin
// RPC, and plays a turn through them, so the real plugin talks to the real
// Impulse. The service half sees another terminal's IMPULSE_PANE_TOKEN, as
// a shared service started elsewhere would. It never talks to any model.
// What the plugin hands back (the system prompt, tool output, a merge it
// asked about, a shell's pane, Impulse's tools) is saved in
// ~/opencode/<mode>.json, then it waits, like an agent at its prompt.
//
//   opencode waiting   a prompt, then a permission request left unanswered
//   opencode turn      a prompt, an edit of a file a task also changes,
//                      Impulse's list_tasks tool, done
//   opencode merge     a prompt, then git merge of a busy task's branch:
//                      asked about, approved, run

import { mkdirSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

const mode = process.argv[2] ?? "idle";
const home = homedir();
const directory = process.cwd();
const sessionID = `ses_smoke${mode}`;
const pause = (seconds) =>
  new Promise((done) => setTimeout(done, seconds * 1000));
const plugin = join(home, ".config/opencode/plugins/impulse");

// opencode's plugin RPC: the service registers, every terminal calls and
// listens.
const rpc = { handlers: {}, listeners: {} };
const emit = (name, data) => {
  for (const listener of rpc.listeners[name] ?? []) listener({ data });
};

// The service's half.
const hooks = {};
const transforms = [];
const tools = new Map();
const addHook = (domain) => async (name, callback) => {
  (hooks[`${domain}.${name}`] ??= []).push(callback);
  return { dispose: async () => {} };
};
const run = async (name, event) => {
  for (const callback of hooks[name] ?? []) await callback(event);
  return event;
};
// The terminal's event stream (opencode's data.on).
const listeners = {};
const tell = (type, data) => {
  for (const listener of listeners[type] ?? []) listener({ type, data });
};
const answers = [];
const server = {
  location: { directory },
  rpc: {
    register: async (definition, handlers) => {
      rpc.handlers = handlers;
      return {
        events: { emit: async (name, data) => emit(name, data) },
        dispose: async () => {},
      };
    },
  },
  tool: {
    transform: async (callback) => {
      transforms.push(callback);
      return { dispose: async () => {} };
    },
    reload: async () => {
      tools.clear();
      const editor = {
        namespace() {},
        add: (tool) => tools.set(tool.name, tool),
      };
      for (const callback of transforms) callback(editor);
    },
    hook: addHook("tool"),
  },
  session: { hook: addHook("session") },
  shell: { hook: addHook("shell") },
  permission: {
    hook: addHook("permission"),
    // Asks you when a hook says so: here, answered after a moment.
    assert: async (input) => {
      const event = await run("permission.evaluate", {
        ...input,
        effect: "allow",
      });
      if (event.effect !== "ask") return;
      answers.push(event.message);
      tell("permission.asked", {
        sessionID: input.sessionID,
        action: input.action,
        message: event.message,
      });
      await pause(4);
      tell("permission.replied", { sessionID: input.sessionID, reply: "once" });
    },
  },
};

// The terminal's half.
const terminal = {
  location: { directory },
  client: {
    rpc: () => ({
      attach: (input) => rpc.handlers.attach(input),
      detach: (input) => rpc.handlers.detach(input),
      show: (input) => rpc.handlers.show(input),
      answer: (input) => rpc.handlers.answer(input),
      events: {
        on: (name, listener) => {
          (rpc.listeners[name] ??= []).push(listener);
          return () => {};
        },
      },
    }),
  },
  data: {
    on: (type, listener) => {
      (listeners[type] ??= []).push(listener);
      return () => {};
    },
    session: { root: (id) => id },
  },
  ui: {
    router: { current: () => ({ type: "session", sessionID }) },
    tabs: { list: () => [] },
  },
};

// Let Impulse notice the process before the first hook arrives.
await pause(2.5);
const serverPlugin = (await import(join(plugin, "server.js"))).default;
const tuiPlugin = (await import(join(plugin, "tui.js"))).default;
await serverPlugin.setup(server);
await tuiPlugin.setup(terminal);
await server.tool.reload();
const result = { tools: [...tools.keys()] };
console.log(`opencode (stand-in): ${mode}`);

const tool = { sessionID, agent: "build", messageID: "msg_1", id: "call_1" };
if (mode !== "idle") {
  tell("session.execution.started", { sessionID });
  const context = await run("session.context", {
    sessionID,
    system: [{ type: "text", text: "You are opencode." }],
  });
  result.system = context.system.map((part) => part.text);
}
if (mode === "waiting") {
  tell("permission.asked", { sessionID, action: "edit" });
  console.log("Allow this edit to src/trails.ts?");
} else if (mode === "turn") {
  // Overlaps are worked out in the background: give them a moment.
  await pause(3);
  const edit = await run("tool.execute.after", {
    ...tool,
    tool: "edit",
    input: { filePath: "src/photos.ts" },
    status: "completed",
    result: { content: [{ type: "text", text: "Edit applied successfully." }] },
  });
  result.editOutput = edit.result.content.map((part) => part.text).join("\n\n");
  const listTasks = tools.get("list_tasks");
  result.listTasks = listTasks
    ? (await listTasks.execute({}, tool)).content
    : null;
  tell("session.execution.succeeded", { sessionID });
  console.log("Done.");
} else if (mode === "merge") {
  const command = "git merge add-trail-photos";
  await run("tool.execute.before", {
    ...tool,
    tool: "shell",
    input: { command },
  });
  result.asked = answers;
  // The command's shell, from a service another terminal started.
  const shell = await run("shell.create.before", {
    command,
    cwd: directory,
    env: { ...process.env, IMPULSE_PANE_TOKEN: "another-terminals-token" },
  });
  result.shellIsThisPane =
    shell.env.IMPULSE_PANE_TOKEN === process.env.IMPULSE_PANE_TOKEN;
  tell("session.execution.succeeded", { sessionID });
  console.log("Done.");
}

mkdirSync(join(home, "opencode"), { recursive: true });
writeFileSync(join(home, "opencode", `${mode}.json`), JSON.stringify(result));
setInterval(() => {}, 1000);
