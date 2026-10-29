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
//   opencode subagents a prompt starting two subagents in the background
//                      that both edit src/photos.ts (the second's first try
//                      stopped), a turn ended while they run, then, once
//                      ~/opencode/subagents-go exists, both finishing and
//                      their results bringing the session back for a turn

import { existsSync, mkdirSync, writeFileSync } from "node:fs";
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
// The service's (event.subscribe), and an event both see.
const streams = [];
const publish = (type, data) => {
  for (const stream of streams) stream({ type, data });
};
const both = (type, data) => {
  tell(type, data);
  publish(type, data);
};
// Sessions, and the one that started each subagent's.
const sessions = new Map([[sessionID, { id: sessionID }]]);
const parents = new Map();
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
  session: {
    hook: addHook("session"),
    get: async (input) => sessions.get(input.sessionID),
  },
  event: {
    subscribe: () => {
      const waiting = [];
      let wake = () => {};
      streams.push((event) => {
        waiting.push(event);
        wake();
      });
      return (async function* () {
        while (true) {
          while (waiting.length > 0) yield waiting.shift();
          await new Promise((done) => (wake = done));
        }
      })();
    },
  },
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
      subagents: (input) => rpc.handlers.subagents(input),
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
    session: { root: (id) => parents.get(id) ?? id },
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
    input: { path: "src/photos.ts" },
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

if (mode === "subagents") {
  const calls = [
    ["ses_suba", "call_a", "Write the photos endpoint"],
    ["ses_subb", "call_b", "Write the photo tests"],
  ];
  const input = (description) => ({
    agent: "general",
    description,
    prompt: description,
    background: true,
  });
  // As opencode makes two subagent calls in one message: both calls
  // before, then both return with their subagents in the background, and
  // then each subagent's first step.
  for (const [, id, description] of calls) {
    await run("tool.execute.before", {
      ...tool,
      id,
      tool: "subagent",
      input: input(description),
    });
  }
  result.launchOutput = [];
  for (const [child, id, description] of calls) {
    const call = await run("tool.execute.after", {
      ...tool,
      id,
      tool: "subagent",
      input: input(description),
      status: "completed",
      result: {
        output: { sessionID: child, status: "running", output: "running" },
        content: [{ type: "text", text: "The subagent is working." }],
      },
    });
    result.launchOutput.push(
      call.result.content.map((part) => part.text).join("\n\n"),
    );
  }
  result.childSystem = [];
  for (const [child, , description] of calls) {
    parents.set(child, sessionID);
    sessions.set(child, {
      id: child,
      parentID: sessionID,
      agent: "general",
      title: description,
      location: { directory },
    });
    const context = await run("session.context", {
      sessionID: child,
      agent: "general",
      system: [{ type: "text", text: "You are a subagent." }],
    });
    result.childSystem.push(context.system.map((part) => part.text));
  }
  // Both write src/photos.ts: the second's first try is stopped, as the
  // first changed it meanwhile; its retry, after reading it, goes through.
  const write = (child) => ({
    sessionID: child,
    agent: "general",
    messageID: "msg_2",
    id: `write_${child}`,
    tool: "write",
    input: { path: "src/photos.ts", content: "export {};\n" },
  });
  const edit = async (child) => {
    await run("tool.execute.before", write(child));
    const done = await run("tool.execute.after", {
      ...write(child),
      status: "completed",
      result: { content: [{ type: "text", text: "Wrote file successfully." }] },
    });
    return done.result.content.map((part) => part.text).join("\n\n");
  };
  await edit("ses_suba");
  result.stopped = await run("tool.execute.before", write("ses_subb")).then(
    () => null,
    (error) => error.message,
  );
  result.retryOutput = await edit("ses_subb");
  // The session's turn ends with both still running.
  tell("session.execution.succeeded", { sessionID });
  console.log("Both subagents are working; I'll check their results.");
  await pause(2);
  mkdirSync(join(home, "opencode"), { recursive: true });
  writeFileSync(join(home, "opencode", "subagents-ready"), "");
  const go = join(home, "opencode", "subagents-go");
  while (!existsSync(go)) await pause(0.2);
  // Both finish, and their results reach the session, which takes a turn
  // to read them.
  for (const [child, , description] of calls) {
    publish("session.execution.succeeded", { sessionID: child });
    publish("session.inbox.enqueued", {
      sessionID,
      item: {
        type: "synthetic",
        payload: {
          text: `<subagent sessionID="${child}" state="completed" description="${description}">\nDone: ${description}\n</subagent>`,
          metadata: { source: "subagent", childID: child, state: "completed" },
        },
      },
    });
  }
  await pause(2);
  both("session.execution.started", { sessionID });
  await pause(1);
  tell("session.execution.succeeded", { sessionID });
  console.log("Both finished.");
  await pause(1);
}

mkdirSync(join(home, "opencode"), { recursive: true });
writeFileSync(join(home, "opencode", `${mode}.json`), JSON.stringify(result));
setInterval(() => {}, 1000);
