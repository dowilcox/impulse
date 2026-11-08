// Impulse's Pi extension.
//
// In an Impulse terminal it tells Impulse what Pi is doing (working,
// waiting for you, finished, stopped by an error) by running `impulse hook
// pi <event>`, and reads what Impulse answers: the repository's other
// tasks when a session starts (a section of Pi's system prompt), notes
// after an edit or a shell command (after the tool's output, which the
// model reads next), a question before a command that merges a branch a
// task is still working on (Pi asks you, with Impulse's reason), and
// whether to load the folder's own Pi extensions and settings, as Impulse
// trusts the folder. Impulse's tools are Pi's own here, run through this
// pane's `impulse mcp`. Pi runs in the pane, as the pane, so Impulse knows
// which terminal is asking. Anywhere else (no IMPULSE_CLI) it does nothing.
//
// A Pi started by this one (a subagent extension's, `pi -p` run by the
// bash tool) isn't the pane's agent: it gets Impulse's tools, and reports
// nothing.
//
// Impulse replaces or removes this file as a whole, so changes made to it
// here don't last.

import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import { resolve } from "node:path";

const cli = process.env.IMPULSE_CLI;

// The tools whose results Impulse may have a note for.
const editTools = new Set(["edit", "write"]);
const shellTool = "bash";

// Calls go one at a time, in the order Pi reported them.
let queue = Promise.resolve();

// Impulse's answer to an event ({} when it has none, or can't be reached).
function hook(event, payload) {
  const call = queue.then(() => run(event, payload));
  queue = call;
  return call;
}

function run(event, payload) {
  return new Promise((done) => {
    let output = "";
    // The CLI itself, by its path: no shell, so no alias or function can
    // stand in for it.
    const child = spawn(cli, ["hook", "pi", event], {
      stdio: ["pipe", "pipe", "ignore"],
    });
    // Never keep Pi waiting long.
    const timer = setTimeout(() => child.kill(), 10_000);
    child.stdout.on("data", (chunk) => (output += chunk));
    child.stdin.on("error", () => {});
    child.on("error", () => {
      clearTimeout(timer);
      done({});
    });
    child.on("close", () => {
      clearTimeout(timer);
      try {
        const answer = JSON.parse(output);
        done(answer && typeof answer === "object" ? answer : {});
      } catch {
        done({});
      }
    });
    child.stdin.end(JSON.stringify(payload ?? {}));
  });
}

// Impulse's MCP server, started from this pane: its tools, and the calls
// to them.
function mcpServer() {
  const child = spawn(cli, ["mcp"], { stdio: ["pipe", "pipe", "ignore"] });
  const waiting = new Map();
  let lastID = 0;
  let stopped = false;
  const stop = () => {
    stopped = true;
    for (const done of waiting.values()) {
      done({ error: { message: "Impulse's MCP server stopped." } });
    }
    waiting.clear();
  };
  child.on("error", stop);
  child.on("exit", stop);
  child.stdin.on("error", () => {});
  createInterface({ input: child.stdout }).on("line", (line) => {
    try {
      const message = JSON.parse(line);
      waiting.get(message.id)?.(message);
      waiting.delete(message.id);
    } catch {}
  });
  const send = (message) => {
    if (!stopped) child.stdin.write(JSON.stringify(message) + "\n");
  };
  const request = (method, params, signal) =>
    new Promise((done) => {
      if (stopped) {
        return done({ error: { message: "Impulse's MCP server stopped." } });
      }
      const id = ++lastID;
      waiting.set(id, done);
      // Esc: the server stops waiting (for you in Impulse, say) too.
      signal?.addEventListener(
        "abort",
        () => {
          if (!waiting.delete(id)) return;
          send({
            jsonrpc: "2.0",
            method: "notifications/cancelled",
            params: { requestId: id, reason: "Cancelled in Pi" },
          });
          done({ error: { message: "Cancelled." } });
        },
        { once: true },
      );
      send({ jsonrpc: "2.0", id, method, params });
    });
  return {
    async start() {
      const init = await request("initialize", {
        protocolVersion: "2025-11-25",
        capabilities: {},
        clientInfo: { name: "Pi (Impulse's extension)", version: "1" },
      });
      send({ jsonrpc: "2.0", method: "notifications/initialized" });
      const list = await request("tools/list", {});
      return {
        instructions: init.result?.instructions,
        tools: list.result?.tools ?? [],
      };
    },
    async call(name, args, signal) {
      const reply = await request(
        "tools/call",
        { name, arguments: args ?? {} },
        signal,
      );
      if (reply.error) {
        return { isError: true, text: reply.error.message };
      }
      return {
        isError: reply.result?.isError === true,
        text: text(reply.result?.content),
        structured: reply.result?.structuredContent,
      };
    },
    stop() {
      stopped = true;
      child.kill();
    },
  };
}

function text(content) {
  if (typeof content === "string") return content;
  return (Array.isArray(content) ? content : [])
    .filter((part) => part?.type === "text")
    .map((part) => part.text)
    .join("\n");
}

// Pi's tool names for Impulse's: `impulse_` in front, so they never take
// the place of another extension's tool.
const piName = (name) => `impulse_${name}`;

// `words` with each of Impulse's tool names as Pi knows it. A name like
// list_tasks wherever it stands; one that's an ordinary word too (notify)
// only as code, `notify`, so a sentence or `impulse notify` keeps it.
function renamed(words, tools) {
  let out = words ?? "";
  for (const tool of tools) {
    const name = tool.name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    const pattern = tool.name.includes("_") ? `\\b${name}\\b` : `(?<=\`)${name}(?=\`)`;
    out = out.replace(new RegExp(pattern, "g"), piName(tool.name));
  }
  return out;
}

// The file an edit or write names, as an absolute path (models sometimes
// put an @ in front, which Pi's own tools drop).
function editedFile(input, cwd) {
  const path = input?.path ?? input?.file_path;
  if (typeof path !== "string" || path === "") return undefined;
  return resolve(cwd, path.replace(/^@/, ""));
}

// The error the last of a run's messages ended on, if it did.
function failure(messages) {
  const last = [...(messages ?? [])]
    .reverse()
    .find((message) => message?.role === "assistant");
  if (last?.stopReason !== "error") return undefined;
  return last.errorMessage || "Pi stopped on an error";
}

export default function (pi) {
  if (!cli) return;
  // The first Pi in the pane is its agent; the ones it starts aren't.
  const owner = process.env.IMPULSE_PI_PID;
  const nested = owner !== undefined && owner !== String(process.pid);
  if (!nested) process.env.IMPULSE_PI_PID = String(process.pid);

  // The session to resume this pane's Pi with (none for one that isn't
  // saved), and the folder it works in.
  let session;
  let cwd = process.cwd();
  const send = (event, extra = {}) =>
    hook(event, { session_id: session, cwd, ...extra });

  // The repository's other tasks (from when the session started) and how
  // to use Impulse's tools, beside Pi's system prompt.
  let summary;
  let instructions;
  // Whether a turn runs (agent_start until agent_settled), and the error
  // the last run ended on.
  let working = false;
  let failed;
  // Impulse's MCP server and its tools, for this session.
  let mcp;
  let tools = [];

  const startTools = async () => {
    mcp?.stop();
    mcp = mcpServer();
    // Pi waits for this to start: never long, as with the hooks.
    const server = mcp;
    const catalog = await Promise.race([
      server.start(),
      new Promise((done) => setTimeout(done, 10_000, { tools: [] })),
    ]).catch(() => ({ tools: [] }));
    if (catalog.tools.length === 0) {
      tools = [];
      server.stop();
      if (mcp === server) mcp = undefined;
      return;
    }
    tools = catalog.tools.filter((tool) => typeof tool?.name === "string");
    if (tools.length === 0) return;
    instructions = renamed(catalog.instructions, tools);
    const namespace = {
      name: "impulse",
      description:
        "Impulse, the terminal IDE Pi is running in: tasks, review comments, problems, terminals' output, unsaved files and project scripts.",
      instructions,
    };
    for (const tool of tools) {
      const { title, ...annotations } = tool.annotations ?? {};
      pi.registerTool({
        name: piName(tool.name),
        label: tool.title ?? title ?? tool.name,
        description: renamed(tool.description, tools),
        parameters: tool.inputSchema ?? { type: "object", properties: {} },
        annotations,
        namespace,
        async execute(_id, params, signal) {
          const answer = await server.call(tool.name, params, signal);
          if (answer.isError) throw new Error(answer.text || "Impulse refused.");
          const result = {
            content: [{ type: "text", text: answer.text || "Done." }],
            details: undefined,
          };
          if (answer.structured && typeof answer.structured === "object") {
            result.structuredContent = answer.structured;
          }
          return result;
        },
      });
    }
  };

  pi.on("session_start", async (event, ctx) => {
    cwd = ctx.cwd ?? cwd;
    await startTools();
    if (nested) return;
    session = ctx.sessionManager.getSessionFile()
      ? ctx.sessionManager.getSessionId()
      : undefined;
    working = false;
    const answer = await send("SessionStart", { source: event.reason });
    summary =
      typeof answer.context === "string" && answer.context
        ? renamed(answer.context, tools)
        : undefined;
  });

  pi.on("session_shutdown", async () => {
    mcp?.stop();
    mcp = undefined;
  });

  // The folder's own extensions, settings and skills: loaded when Impulse
  // trusts the folder, never in one it keeps restricted (a task made from
  // someone else's branch); otherwise Pi asks, as it does.
  pi.on("project_trust", async (event) => {
    if (nested) return { trusted: "undecided" };
    const answer = await hook("ProjectTrust", { cwd: event.cwd });
    if (answer.trusted === "yes" || answer.trusted === "no") {
      return { trusted: answer.trusted };
    }
    return { trusted: "undecided" };
  });

  pi.on("before_agent_start", async (event) => {
    const parts = [summary, tools.length > 0 ? instructions : undefined];
    const section = parts.filter(Boolean).join("\n\n");
    if (section) event.systemPromptOptions.sections.impulse = section;
    else delete event.systemPromptOptions.sections.impulse;
  });

  if (nested) return;

  // A run starts; retries and queued messages start more before it settles.
  pi.on("agent_start", async () => {
    failed = undefined;
    if (working) return;
    working = true;
    await send("UserPromptSubmit");
  });

  pi.on("agent_end", async (event) => {
    failed = failure(event.messages);
  });

  pi.on("agent_settled", async (event) => {
    if (!working) return;
    working = false;
    // Stopped with Esc: back to idle, as Claude Code's turns end then.
    if (event.aborted) {
      await send("Interrupted");
    } else if (failed) {
      await send("StopFailure", { error: failed, last_assistant_message: failed });
    } else {
      await send("Stop");
    }
  });

  // A dialog (an extension's question, the merge question below) waits for
  // you mid-turn.
  pi.on("ui_prompt_start", async (event) => {
    if (!working) return;
    await send("prompt.start", { message: event.title ?? "" });
  });
  pi.on("ui_prompt_end", async () => {
    if (!working) return;
    await send("prompt.end");
  });

  // Before a shell command: whether it merges a branch a task is still
  // working on. Then you're asked, with Impulse's reason; without a UI to
  // ask in, it doesn't run.
  pi.on("tool_call", async (event, ctx) => {
    if (event.toolName !== shellTool) return;
    const command = event.input?.command;
    if (typeof command !== "string") return;
    const answer = await send("PreToolUse", {
      tool_name: shellTool,
      tool_input: { command },
    });
    if (typeof answer.ask !== "string" || !answer.ask) return;
    const reason = answer.ask;
    if (!ctx.hasUI) return { block: true, reason };
    const go = await ctx.ui.confirm("Impulse asks before this merge", reason);
    if (!go) return { block: true, reason: `You declined: ${reason}` };
  });

  // After an edit or a shell command: Impulse's notes (another task
  // changes this file; dependency files changed) after what the tool
  // returned, which the model reads next.
  pi.on("tool_result", async (event, ctx) => {
    if (event.isError) return;
    let input;
    if (event.toolName === shellTool) {
      input = { command: event.input?.command };
    } else if (editTools.has(event.toolName)) {
      const file = editedFile(event.input, ctx.cwd ?? cwd);
      if (!file) return;
      input = { file_path: file };
    } else {
      return;
    }
    const answer = await send("PostToolUse", {
      tool_name: event.toolName,
      tool_input: input,
    });
    if (typeof answer.context !== "string" || !answer.context) return;
    return {
      content: [
        ...(event.content ?? []),
        { type: "text", text: renamed(answer.context, tools) },
      ],
      structuredContent: event.structuredContent,
    };
  });
}
