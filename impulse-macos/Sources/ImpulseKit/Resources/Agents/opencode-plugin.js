// Impulse's opencode plugin, installed by Impulse's Agent Integration.
//
// In an Impulse terminal it tells Impulse what opencode is doing (working,
// waiting for you, finished) by running `impulse hook opencode <event>`, and
// passes on what Impulse answers: a summary of the repository's other tasks
// when a session starts, notes after an edit or a shell command, and a stop
// before a command that merges a branch another task is still working on.
// Anywhere else (no IMPULSE_CLI) it does nothing.
//
// Impulse replaces or removes this file as a whole, so changes made to it
// here don't last.

import { spawn } from "node:child_process";
import { resolve } from "node:path";

const cli = process.env.IMPULSE_CLI;
const editTools = new Set(["edit", "write", "multiedit", "apply_patch"]);

// Calls go one at a time, in the order opencode reported them.
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
    // The CLI itself, by its path: no shell, so no alias or function
    // can stand in for it.
    const child = spawn(cli, ["hook", "opencode", event], {
      stdio: ["pipe", "pipe", "ignore"],
    });
    // Never keep opencode waiting long.
    const timer = setTimeout(() => child.kill(), 10000);
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
    child.stdin.end(JSON.stringify(payload));
  });
}

export const ImpulsePlugin = async ({ client, directory }) => {
  if (!cli) return {};
  // Sessions opencode's task tool started: their turns aren't yours, and
  // they aren't the session to resume.
  const subagents = new Set();
  // Your sessions this run, with what Impulse said about the other tasks.
  const summaries = new Map();
  // The last event sent for each session (opencode can report idle twice).
  const last = new Map();

  const isSubagent = async (sessionID) => {
    if (subagents.has(sessionID)) return true;
    if (summaries.has(sessionID)) return false;
    try {
      const session = await client.session.get({ path: { id: sessionID } });
      if (session?.data?.parentID) subagents.add(sessionID);
    } catch {}
    return subagents.has(sessionID);
  };

  const send = (event, sessionID, extra = {}) => {
    last.set(sessionID, event);
    return hook(event, {
      session_id: subagents.has(sessionID) ? undefined : sessionID,
      cwd: directory,
      ...extra,
    });
  };

  hook("start", { cwd: directory });

  return {
    event: async ({ event }) => {
      const properties = event.properties ?? {};
      const sessionID = properties.sessionID;
      switch (event.type) {
        case "session.created":
          if (properties.info?.parentID) subagents.add(properties.info.id);
          break;
        case "session.idle":
          if (subagents.has(sessionID) || last.get(sessionID) === event.type) {
            break;
          }
          await send(event.type, sessionID);
          break;
        // permission.updated: opencode before 1.0.
        case "permission.asked":
        case "permission.updated":
          await send("permission.asked", sessionID, {
            tool_name: properties.permission ?? properties.type,
          });
          break;
        case "permission.replied":
        case "question.asked":
        case "question.replied":
        case "question.rejected":
          await send(event.type, sessionID);
          break;
      }
    },

    "chat.message": async (input) => {
      if (await isSubagent(input.sessionID)) return;
      if (!summaries.has(input.sessionID)) {
        const answer = await send("session.start", input.sessionID);
        summaries.set(input.sessionID, answer.context);
      }
      await send("chat.message", input.sessionID);
    },

    // The summary sits beside opencode's own instructions for the whole
    // session.
    "experimental.chat.system.transform": async (input, output) => {
      const summary = summaries.get(input.sessionID);
      if (summary) output.system.push(summary);
    },

    // opencode's plugins can't ask you before a tool runs: stopping it, with
    // the reason, makes the agent ask instead.
    "tool.execute.before": async (input, output) => {
      const command = output.args?.command;
      if (input.tool !== "bash" || typeof command !== "string") return;
      const answer = await send("tool.execute.before", input.sessionID, {
        tool_name: input.tool,
        tool_input: { command },
      });
      if (answer.block) throw new Error(answer.block);
    },

    // Notes go after what the tool returned, which the agent reads next.
    "tool.execute.after": async (input, output) => {
      const notes = [];
      const report = async (toolInput) => {
        const answer = await send("tool.execute.after", input.sessionID, {
          tool_name: input.tool,
          tool_input: toolInput,
        });
        if (answer.context) notes.push(answer.context);
      };
      if (input.tool === "bash") {
        await report({ command: input.args?.command });
      } else if (editTools.has(input.tool)) {
        const files =
          input.tool === "apply_patch"
            ? (output.metadata?.files ?? []).map(
                (f) => f.movePath ?? f.filePath,
              )
            : [input.args?.filePath];
        for (const file of files) {
          if (typeof file === "string") {
            await report({ file_path: resolve(directory, file) });
          }
        }
      }
      if (notes.length) output.output = [output.output, ...notes].join("\n\n");
    },
  };
};
