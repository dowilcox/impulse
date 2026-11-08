#!/usr/bin/env node
// Stand-in for Pi in the smoke checks, installed as ~/bin/pi in the demo
// home. It loads Impulse's extension from where Pi would
// (~/.pi/agent/extensions/impulse.js, put there by Agent Integration) and
// calls it as Pi's extension runner does (docs/extensions.md in Pi's
// package: the events in their order, a tool_call handler's block, a
// tool_result handler's patch, a tool's execute, ui_prompt_start/end
// around a dialog), so the real extension talks to the real Impulse. It
// never talks to any model. What the extension hands back (its trust
// answer, the system prompt's section, tool output, a merge it asked
// about, Impulse's tools) is saved in ~/pi/<mode>.json, then it waits,
// like an agent at its prompt.
//
//   pi turn     the folder's trust, a prompt, an edit of a file a task also
//               changes, Impulse's list_tasks tool, done
//   pi merge    a prompt, then git merge of a busy task's branch: asked
//               about, approved, run
//   pi failed   a prompt whose run ends on the provider's error
//   pi aborted  a prompt stopped with Esc
//   pi nested   a Pi started by the pane's Pi: tools, and no reports

import { mkdirSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

const mode = process.argv[2] ?? "idle";
const home = homedir();
const cwd = process.cwd();
const sessionId = `0193f5a2-smoke-${mode}`;
const pause = (seconds) =>
  new Promise((done) => setTimeout(done, seconds * 1000));
const extension = join(home, ".pi/agent/extensions/impulse.js");

// Pi's ExtensionAPI, as much of it as the extension uses.
const handlers = {};
const tools = new Map();
const pi = {
  on(event, handler) {
    (handlers[event] ??= []).push(handler);
    return () => {};
  },
  registerTool(tool) {
    tools.set(tool.name, tool);
  },
};
const emit = async (type, event = {}) => {
  let answer;
  for (const handler of handlers[type] ?? []) {
    const result = await handler({ type, ...event }, ctx);
    if (result !== undefined) answer = result;
  }
  return answer;
};
// A dialog, as Pi shows one: answered after a moment.
const asked = [];
const ctx = {
  cwd,
  mode: "tui",
  hasUI: true,
  sessionManager: {
    getSessionFile: () => join(home, ".pi/agent/sessions", `${sessionId}.jsonl`),
    getSessionId: () => sessionId,
  },
  ui: {
    confirm: async (title, message) => {
      asked.push({ title, message });
      await emit("ui_prompt_start", { reason: "ui_prompt", kind: "confirm", title });
      await pause(4);
      await emit("ui_prompt_end", { reason: "ui_prompt", kind: "confirm", title });
      return true;
    },
  },
};
// tool_call: the first handler that blocks wins.
const toolCall = async (toolName, input) => {
  for (const handler of handlers.tool_call ?? []) {
    const result = await handler(
      { type: "tool_call", toolCallId: "call_1", toolName, input },
      ctx,
    );
    if (result?.block) return result;
  }
  return undefined;
};
// tool_result: each handler patches what the one before left.
const toolResult = async (toolName, input, text) => {
  let event = {
    type: "tool_result",
    toolCallId: "call_1",
    toolName,
    input,
    content: [{ type: "text", text }],
    isError: false,
  };
  for (const handler of handlers.tool_result ?? []) {
    const patch = await handler(event, ctx);
    if (patch) event = { ...event, ...patch };
  }
  return event.content.map((part) => part.text).join("\n\n");
};
// A run that ends with the model's last message.
const settle = async (message, aborted = false) => {
  await emit("agent_end", { messages: [message] });
  await emit("agent_settled", { aborted });
};

if (mode === "nested") {
  // The pane's Pi started this one.
  process.env.IMPULSE_PI_PID = String(process.ppid || 1);
}
// Let Impulse notice the process before the first hook arrives.
await pause(2.5);
const factory = (await import(extension)).default;
await factory(pi);
const result = {};
if (mode !== "nested") {
  result.trust = await emit("project_trust", { cwd });
}
await emit("session_start", { reason: "startup" });
result.tools = [...tools.keys()];
console.log(`Pi (stand-in): ${mode}`);

if (mode !== "idle") {
  const options = { sections: {}, selectedTools: [] };
  await emit("before_agent_start", {
    messageType: "user",
    prompt: "Add photos",
    systemPrompt: "You are Pi.",
    systemPromptOptions: options,
  });
  result.section = options.sections.impulse ?? null;
  await emit("agent_start");
}

if (mode === "turn" || mode === "nested") {
  // Overlaps are worked out in the background: give them a moment.
  await pause(3);
  result.editOutput = await toolResult(
    "edit",
    { path: "src/photos.ts", edits: [] },
    "Successfully replaced 1 block in src/photos.ts.",
  );
  const listTasks = tools.get("impulse_list_tasks");
  result.listTasks = listTasks
    ? (await listTasks.execute("call_2", {}, undefined, undefined, ctx)).content
        .map((part) => part.text)
        .join("\n")
    : null;
  await settle({ role: "assistant", stopReason: "stop" });
  console.log("Done.");
} else if (mode === "merge") {
  result.blocked = (await toolCall("bash", { command: "git merge add-trail-photos" })) ?? null;
  result.asked = asked;
  await settle({ role: "assistant", stopReason: "stop" });
  console.log("Done.");
} else if (mode === "failed") {
  await settle({
    role: "assistant",
    stopReason: "error",
    errorMessage: "429 Too Many Requests: rate limited",
  });
  console.log("Error: 429 Too Many Requests: rate limited");
} else if (mode === "aborted") {
  // Pi's own words for a run stopped with Esc.
  await pause(2);
  await settle({ role: "assistant", stopReason: "aborted", errorMessage: "Operation aborted" }, true);
  console.log("Operation aborted");
}

mkdirSync(join(home, "pi"), { recursive: true });
writeFileSync(join(home, "pi", `${mode}.json`), JSON.stringify(result));
setInterval(() => {}, 1000);
