// Impulse's opencode plugin, the half in opencode's background service.
//
// The service is shared by every terminal and started with the environment
// of whichever terminal ran opencode first, so this half never reads
// Impulse's variables from its own. For anything only Impulse can answer
// (the other tasks, when a session starts; notes after an edit or a shell
// command, and an edit stopped before it overwrites another agent's;
// whether a merge needs asking about; Impulse's tools) it asks the
// terminal showing the session (tui.js), which asks Impulse. It also
// follows the subagents each session starts (opencode's subagent tool,
// child sessions): when each starts and stops, what it reports, and the
// ones still running in the background when the session's turn ends.
// Sessions no Impulse terminal shows get nothing, and their shell commands
// don't get another pane's identity.
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
const subagentTool = "subagent";
// The permission Impulse asks for before a merge it has a reason to stop.
const mergeAction = "impulse_merge";

// opencode loads the plugin once per folder it serves, in one process:
// everything the instances share lives here. (A newer plugin, reloaded
// into a running service, names its own if what's kept here changes.)
const shared = (globalThis.__impulsePlugin2 ??= {
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
  // other tasks (or, for a subagent's, the note about others working in its
  // checkout), or null for a session no Impulse terminal shows.
  summaries: new Map(),
  // Subagents' sessions, by id: the session that started each, its type,
  // what it was asked to do, whether it runs in the background, whether
  // it's still running, and whether its result reached that session.
  children: new Map(),
  // What to tell the agent after a subagent call, by the call's id.
  launches: new Map(),
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

// The session a subagent's belongs to: the one the user is in, which its
// terminal shows.
function top(sessionID) {
  let id = sessionID;
  const seen = new Set();
  while (shared.children.has(id) && !seen.has(id)) {
    seen.add(id);
    id = shared.children.get(id).parent;
  }
  return id;
}

// A subagent's session, kept up to date with what's known of it.
function child(id, details) {
  const known = shared.children.get(id) ?? {
    parent: details.parent,
    background: false,
    running: true,
    reported: false,
  };
  for (const [key, value] of Object.entries(details)) {
    if (value !== undefined) known[key] = value;
  }
  shared.children.set(id, known);
  if (shared.children.size > 200) {
    shared.children.delete(shared.children.keys().next().value);
  }
  return known;
}

// Where an ask about something `sessionID` does goes (its top session's
// terminal), and, for a subagent's, which subagent did it.
function about(sessionID) {
  return {
    session: top(sessionID),
    subagent: shared.children.has(sessionID) ? { agent_id: sessionID } : {},
  };
}

// A subagent call's input as Impulse reads it: what it's asked to do and
// the subagent's type.
function subagentInput(input) {
  return { description: input?.description, subagent_type: input?.agent };
}

// The files an edit tool changes: the one it names, or each one a patch
// adds, updates, deletes or moves to (from its result once it ran, else
// from its text).
function editedFiles(event) {
  const input = event.input ?? {};
  if (event.tool !== "patch") return [input.path ?? input.filePath];
  const files =
    event.result?.metadata?.files ?? event.result?.output?.files ?? undefined;
  if (Array.isArray(files)) {
    return files.map((f) => f.movePath ?? f.file ?? f.filePath ?? f.path);
  }
  const patch = String(input.patchText ?? input.patch ?? "");
  return [
    ...patch.matchAll(
      /^\*\*\* (?:(?:Add|Update|Delete) File|Move to): (.+)$/gm,
    ),
  ].map((match) => match[1].trim());
}

// The result a background subagent's message to its session carries:
// `<subagent sessionID=… state=…>result</subagent>`.
function subagentResult(message) {
  const found = /^<subagent[^>]*>\n?([\s\S]*?)\n?<\/subagent>\s*$/.exec(
    message ?? "",
  );
  return found ? found[1] : message;
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
      // The subagents a session left running in the background, as its
      // turn ends.
      subagents: async (input) => {
        const running = [];
        for (const [id, known] of shared.children) {
          if (!known.background || !known.running) continue;
          if (top(id) !== input.sessionID) continue;
          running.push({
            id,
            agent_type: known.agent ?? "",
            description: known.description ?? "",
          });
        }
        return { running };
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
    // session, and how to use Impulse's tools. A subagent's session, on its
    // first step, starts that subagent, which may hear about others
    // working in its checkout.
    const context = await ctx.session.hook("context", async (event) => {
      if (!shared.summaries.has(event.sessionID)) {
        const session = await ctx.session
          .get({ sessionID: event.sessionID })
          .catch(() => undefined);
        let answer;
        if (session?.parentID) {
          child(event.sessionID, {
            parent: session.parentID,
            agent: session.agent ?? event.agent,
            description: session.title,
          });
          answer = await ask(
            registration,
            top(event.sessionID),
            "subagent.start",
            {
              agent_id: event.sessionID,
              agent_type: session.agent ?? event.agent,
              cwd: session.location?.directory ?? directory,
            },
          );
        } else {
          answer = await ask(registration, event.sessionID, "session.start");
        }
        if (answer === undefined) return;
        shared.summaries.set(event.sessionID, answer.context ?? null);
      }
      const summary = shared.summaries.get(event.sessionID);
      if (summary) event.system.push({ type: "text", text: summary });
      if (shared.instructions && shared.tools.length > 0) {
        event.system.push({ type: "text", text: shared.instructions });
      }
    });

    // Before a subagent call: what Impulse tells the agent about it, kept
    // for after the call (what the agent reads next).
    const beforeSubagent = async (event) => {
      const { session, subagent } = about(event.sessionID);
      const answer = await ask(registration, session, "tool.execute.before", {
        tool_name: subagentTool,
        tool_input: subagentInput(event.input),
        ...subagent,
      });
      if (answer?.context) shared.launches.set(event.id, answer.context);
    };
    // Before an edit: stopped, with why, when it would overwrite a file
    // another agent in the same checkout changed meanwhile. The agent reads
    // it again, and its retry goes through.
    const beforeEdit = async (event) => {
      const { session, subagent } = about(event.sessionID);
      for (const file of editedFiles(event)) {
        if (typeof file !== "string") continue;
        const answer = await ask(registration, session, "tool.execute.before", {
          tool_name: event.tool,
          tool_input: { file_path: resolve(directory, file) },
          ...subagent,
        });
        if (answer === undefined) return;
        if (answer.deny) throw new Error(answer.deny);
      }
    };

    // Before a shell command: whether it merges a branch a task is still
    // working on (then opencode asks you, with Impulse's reason), and the
    // pane it's for.
    const before = await ctx.tool.hook("execute.before", async (event) => {
      if (event.tool === subagentTool) return beforeSubagent(event);
      if (editTools.has(event.tool)) return beforeEdit(event);
      const command = event.input?.command;
      if (event.tool !== "shell" || typeof command !== "string") return;
      const { session, subagent } = about(event.sessionID);
      const answer = await ask(registration, session, "tool.execute.before", {
        tool_name: "shell",
        tool_input: { command },
        ...subagent,
      });
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

    // A subagent call returned: the subagent's session, and whether it
    // went on in the background (opencode tells the agent when it
    // finishes) or finished, with its result.
    const afterSubagent = async (event) => {
      const output = event.result?.output ?? {};
      const id = output.sessionID;
      if (typeof id !== "string") return;
      const background = output.status === "running";
      child(id, {
        parent: event.sessionID,
        agent: event.input?.agent,
        description: event.input?.description,
        background,
        running: background,
      });
      const { session, subagent } = about(event.sessionID);
      const answer = await ask(registration, session, "tool.execute.after", {
        tool_name: subagentTool,
        tool_input: subagentInput(event.input),
        tool_response: { agentId: id, isAsync: background },
        ...subagent,
      });
      if (answer === undefined || background) return;
      await ask(registration, session, "subagent.stop", {
        agent_id: id,
        last_assistant_message: text(output.output),
      });
    };

    // Notes after an edit, a shell command or a subagent call go after
    // what the tool returned, which the agent reads next.
    const after = await ctx.tool.hook("execute.after", async (event) => {
      const notes = [];
      if (event.tool === subagentTool) {
        const launched = shared.launches.get(event.id);
        shared.launches.delete(event.id);
        if (event.status !== "completed") return;
        await afterSubagent(event);
        if (launched) notes.push(launched);
      }
      if (event.status !== "completed") return;
      const inputs = [];
      if (event.tool === "shell") {
        inputs.push({ command: event.input?.command });
      } else if (editTools.has(event.tool)) {
        for (const file of editedFiles(event)) {
          if (typeof file === "string") {
            inputs.push({ file_path: resolve(directory, file) });
          }
        }
      }
      const { session, subagent } = about(event.sessionID);
      for (const toolInput of inputs) {
        const answer = await ask(registration, session, "tool.execute.after", {
          tool_name: event.tool,
          tool_input: toolInput,
          ...subagent,
        });
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

    // Subagents' sessions ending, and a background one's result reaching
    // the session that started it (which brings it back for a turn). Every
    // instance hears these: the first to take one tells Impulse.
    const events = ctx.event?.subscribe?.();
    let listening = true;
    (async () => {
      for await (const event of events ?? []) {
        if (!listening) break;
        const data = event.data ?? event.properties ?? {};
        if (
          event.type === "session.execution.succeeded" ||
          event.type === "session.execution.failed" ||
          event.type === "session.execution.interrupted"
        ) {
          const known = shared.children.get(data.sessionID);
          if (!known?.running) continue;
          known.running = false;
          ask(registration, top(data.sessionID), "subagent.stop", {
            agent_id: data.sessionID,
          });
        } else if (event.type === "session.inbox.enqueued") {
          const payload = data.item?.payload;
          const id = payload?.metadata?.childID;
          if (payload?.metadata?.source !== "subagent") continue;
          const known = shared.children.get(id);
          if (!known || known.reported) continue;
          known.reported = true;
          known.running = false;
          ask(registration, top(id), "subagent.stop", {
            agent_id: id,
            last_assistant_message: subagentResult(payload.text),
            subagents_reported: [id],
          });
        }
      }
    })().catch(() => {});

    return async () => {
      listening = false;
      try {
        await events?.return?.();
      } catch {}
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
