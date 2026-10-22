// Impulse's opencode plugin, the half in opencode's background service.
//
// The service is shared by every terminal and started with the environment
// of whichever terminal ran opencode first, so this half never reads
// Impulse's variables from its own. For anything only Impulse can answer
// (the other tasks, when a session starts; notes after an edit or a shell
// command; whether a merge needs asking about; Impulse's tools) it asks
// the terminal showing the session (tui.js), which asks Impulse. Sessions
// no Impulse terminal shows get nothing, and their shell commands don't
// get another pane's identity.
//
// Impulse replaces or removes this folder as a whole, so changes made to
// it here don't last.

import { resolve } from "node:path";
import { Impulse } from "./rpc.js";

// The variables that tell `impulse` which pane it's in: a shell command
// gets those of the terminal showing its session, or none.
const paneVariables = [
  "IMPULSE_SOCKET",
  "IMPULSE_PANE_TOKEN",
  "IMPULSE_CLI",
  "IMPULSE_TASK",
  "IMPULSE_TASK_SLOT",
  "IMPULSE_REPO_ROOT",
];
const editTools = new Set(["edit", "write", "patch"]);
// The permission Impulse asks for before a merge it has a reason to stop.
const mergeAction = "impulse_merge";

// opencode loads the plugin once per folder it serves, in one process:
// everything the instances share lives here. (A newer plugin, reloaded
// into a running service, names its own if what's kept here changes.)
const shared = (globalThis.__impulsePlugin1 ??= {
  // Terminals in Impulse, by id, with when each last said it's there.
  clients: new Map(),
  // The terminal each session is in: the last one to show it.
  owners: new Map(),
  // Asks waiting for an answer, by id.
  asks: new Map(),
  lastID: 0,
  // Impulse's tools and what to tell the agent about them (from the last
  // terminal that attached).
  tools: [],
  instructions: undefined,
  // Each instance's tool reload, run when the tools change.
  reloads: new Set(),
  // What each session's terminal said when it started: the summary of the
  // other tasks, or null for a session no Impulse terminal shows.
  summaries: new Map(),
  // Shell commands about to start, with their terminal's variables.
  shells: [],
  // Why a merge needs asking about, by command.
  merges: new Map(),
});

// A terminal says it's there every 30 seconds (tui.js); one silent for
// longer has gone without saying so.
const silence = 90_000;

// What the terminal showing `sessionID` answers; undefined when no Impulse
// terminal shows it. Every terminal sees the ask and says whether the
// session (or, for a subagent's, the one that started it) is its own.
// Tool calls (`patient`) may wait on you in Impulse, up to an hour.
function ask(registration, sessionID, event, payload, patient = false) {
  const now = Date.now();
  const waiting = new Set();
  for (const [client, seen] of shared.clients) {
    if (now - seen < silence) waiting.add(client);
    else shared.clients.delete(client);
  }
  if (waiting.size === 0) return Promise.resolve(undefined);
  return new Promise((done) => {
    const id = `${process.pid}-${++shared.lastID}`;
    const finish = (answer) => {
      if (!shared.asks.delete(id)) return;
      clearTimeout(timer);
      done(answer);
    };
    shared.asks.set(id, (client, mine, answer) => {
      waiting.delete(client);
      if (mine) finish(answer ?? {});
      else if (waiting.size === 0) finish(undefined);
    });
    const timer = setTimeout(() => finish(undefined), patient ? 3_600_000 : 10_000);
    registration.events
      .emit("ask", { id, sessionID, event, payload: payload ?? {} })
      .catch(() => finish(undefined));
  });
}

function text(content) {
  if (typeof content === "string") return content;
  return (content ?? [])
    .filter((part) => part.type === "text")
    .map((part) => part.text)
    .join("\n");
}

export default {
  id: "impulse",
  async setup(ctx) {
    const directory = ctx.location?.directory ?? process.cwd();
    const registration = await ctx.rpc.register(Impulse, {
      attach: async (input) => {
        shared.clients.set(input.client, Date.now());
        const tools = Array.isArray(input.tools) ? input.tools : [];
        if (JSON.stringify(tools) !== JSON.stringify(shared.tools)) {
          shared.tools = tools;
          for (const reload of shared.reloads) reload();
        }
        shared.instructions = input.instructions;
        return { owners: Object.fromEntries(shared.owners) };
      },
      detach: async (input) => {
        shared.clients.delete(input.client);
        for (const settle of shared.asks.values()) settle(input.client, false);
        for (const [sessionID, client] of shared.owners) {
          if (client !== input.client) continue;
          shared.owners.delete(sessionID);
          await registration.events
            .emit("owner", { sessionID, client: "" })
            .catch(() => {});
        }
        return {};
      },
      // A terminal shows a session: it's that terminal's now.
      show: async (input) => {
        if (shared.owners.get(input.sessionID) === input.client) return {};
        shared.owners.set(input.sessionID, input.client);
        await registration.events
          .emit("owner", { sessionID: input.sessionID, client: input.client })
          .catch(() => {});
        return {};
      },
      answer: async (input) => {
        shared.asks.get(input.id)?.(input.client, input.mine, input.answer);
        return {};
      },
    });

    // Impulse's tools, as opencode's own (an MCP server would run in the
    // service, where it can't tell which terminal is asking). Each call
    // goes to the terminal showing the session, which runs it.
    const tools = await ctx.tool.transform((editor) => {
      if (shared.tools.length === 0) return;
      editor.namespace({
        name: "impulse",
        description:
          "Impulse, the terminal IDE you're running in: tasks, review comments, problems, terminals' output, unsaved files and project scripts.",
      });
      for (const tool of shared.tools) {
        editor.add({
          name: tool.name,
          description: tool.description ?? "",
          input: tool.inputSchema ?? { type: "object" },
          options: { namespace: "impulse" },
          execute: async (input, context) => {
            const answer = await ask(
              registration,
              context.sessionID,
              "tool.call",
              { name: tool.name, arguments: input ?? {} },
              true,
            );
            if (answer === undefined) {
              return {
                content:
                  "Impulse's tools only work in a session opened in an Impulse terminal.",
              };
            }
            const output = text(answer.content);
            if (answer.isError) throw new Error(output || "Impulse refused.");
            return { content: output };
          },
        });
      }
    });
    const reload = () => ctx.tool.reload().catch(() => {});
    shared.reloads.add(reload);

    // The other tasks, beside opencode's instructions for the whole
    // session, and how to use Impulse's tools.
    const context = await ctx.session.hook("context", async (event) => {
      if (!shared.summaries.has(event.sessionID)) {
        const answer = await ask(
          registration,
          event.sessionID,
          "session.start",
        );
        if (answer === undefined) return;
        shared.summaries.set(event.sessionID, answer.context ?? null);
      }
      const summary = shared.summaries.get(event.sessionID);
      if (summary) event.system.push({ type: "text", text: summary });
      if (shared.instructions && shared.tools.length > 0) {
        event.system.push({ type: "text", text: shared.instructions });
      }
    });

    // Before a shell command: whether it merges a branch a task is still
    // working on (then opencode asks you, with Impulse's reason), and the
    // pane it's for.
    const before = await ctx.tool.hook("execute.before", async (event) => {
      const command = event.input?.command;
      if (event.tool !== "shell" || typeof command !== "string") return;
      const answer = await ask(
        registration,
        event.sessionID,
        "tool.execute.before",
        { tool_name: "shell", tool_input: { command } },
      );
      shared.shells.push({ command, env: answer?.env ?? {} });
      if (shared.shells.length > 50) shared.shells.shift();
      if (!answer?.ask) return;
      shared.merges.set(command, answer.ask);
      try {
        await ctx.permission.assert({
          sessionID: event.sessionID,
          agent: event.agent,
          action: mergeAction,
          resources: [command],
          source: { type: "tool", messageID: event.messageID, id: event.id },
        });
      } finally {
        shared.merges.delete(command);
      }
    });
    const evaluate = await ctx.permission.hook("evaluate", (event) => {
      if (event.action !== mergeAction) return;
      const reason = shared.merges.get(event.resources?.[0]);
      if (!reason) return;
      event.effect = "ask";
      event.message = reason;
    });
    const shell = await ctx.shell.hook("create.before", (event) => {
      for (const name of paneVariables) delete event.env[name];
      const index = shared.shells.findIndex((s) => s.command === event.command);
      if (index < 0) return;
      const [{ env }] = shared.shells.splice(index, 1);
      for (const name of paneVariables) {
        if (typeof env[name] === "string") event.env[name] = env[name];
      }
    });

    // Notes after an edit or a shell command go after what the tool
    // returned, which the agent reads next.
    const after = await ctx.tool.hook("execute.after", async (event) => {
      if (event.status !== "completed") return;
      const inputs = [];
      if (event.tool === "shell") {
        inputs.push({ command: event.input?.command });
      } else if (editTools.has(event.tool)) {
        const files =
          event.tool === "patch"
            ? (event.result?.metadata?.files ?? []).map(
                (f) => f.movePath ?? f.filePath,
              )
            : [event.input?.filePath];
        for (const file of files) {
          if (typeof file === "string") {
            inputs.push({ file_path: resolve(directory, file) });
          }
        }
      } else {
        return;
      }
      const notes = [];
      for (const toolInput of inputs) {
        const answer = await ask(
          registration,
          event.sessionID,
          "tool.execute.after",
          { tool_name: event.tool, tool_input: toolInput },
        );
        if (answer === undefined) return;
        if (answer.context) notes.push(answer.context);
      }
      if (notes.length === 0) return;
      const parts = notes.map((note) => ({ type: "text", text: note }));
      if (Array.isArray(event.result.content)) {
        event.result.content.push(...parts);
      } else {
        const output = event.result.content;
        event.result = {
          ...event.result,
          content: [
            ...(output ? [{ type: "text", text: output }] : []),
            ...parts,
          ],
        };
      }
    });

    return async () => {
      shared.reloads.delete(reload);
      for (const done of [
        registration,
        tools,
        context,
        before,
        evaluate,
        shell,
        after,
      ]) {
        await done.dispose().catch(() => {});
      }
    };
  },
};
