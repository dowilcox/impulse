// Impulse's opencode plugin, the half in each terminal running opencode.
//
// In an Impulse terminal it tells Impulse what opencode is doing (working,
// waiting for you, finished) by running `impulse hook opencode <event>`,
// and answers the half in opencode's service (server.js) for the sessions
// it shows: the summary of the repository's other tasks when a session
// starts, notes after an edit or a shell command and whether an edit is
// stopped, whether a merge needs asking about, the subagents its sessions
// start and stop, and Impulse's tools, run through its own `impulse mcp`.
// Everything runs here, in the pane, as the pane: Impulse knows which
// terminal is asking. Anywhere else (no IMPULSE_CLI) it does nothing.
//
// Impulse replaces or removes this folder as a whole, so changes made to
// it here don't last.

import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import { Impulse } from "./rpc.js";

const cli = process.env.IMPULSE_CLI;
const paneVariables = [
  "IMPULSE_SOCKET",
  "IMPULSE_PANE_TOKEN",
  "IMPULSE_CLI",
  "IMPULSE_TASK",
  "IMPULSE_TASK_SLOT",
  "IMPULSE_REPO_ROOT",
];

// Calls go one at a time, in the order opencode reported them.
let queue = Promise.resolve();

// Impulse's answer to an event ({} when it has none, or can't be reached).
// `payload` may still be on its way (a promise): the calls keep their order.
function hook(event, payload) {
  const call = queue.then(async () => run(event, await payload));
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

// Impulse's MCP server, started from this pane: its tools, and the calls
// to them.
function mcpServer() {
  const child = spawn(cli, ["mcp"], { stdio: ["pipe", "pipe", "ignore"] });
  const waiting = new Map();
  let lastID = 0;
  child.on("error", () => {});
  child.stdin.on("error", () => {});
  child.on("exit", () => {
    for (const done of waiting.values())
      done({ error: { message: "Impulse's MCP server stopped." } });
    waiting.clear();
  });
  createInterface({ input: child.stdout }).on("line", (line) => {
    try {
      const message = JSON.parse(line);
      waiting.get(message.id)?.(message);
      waiting.delete(message.id);
    } catch {}
  });
  const send = (message) => child.stdin.write(JSON.stringify(message) + "\n");
  const request = (method, params) =>
    new Promise((done) => {
      const id = ++lastID;
      waiting.set(id, done);
      send({ jsonrpc: "2.0", id, method, params });
    });
  return {
    async start() {
      const init = await request("initialize", {
        protocolVersion: "2025-11-25",
        capabilities: {},
        clientInfo: { name: "opencode (Impulse's plugin)", version: "1" },
      });
      send({ jsonrpc: "2.0", method: "notifications/initialized" });
      const list = await request("tools/list", {});
      return {
        instructions: init.result?.instructions,
        tools: (list.result?.tools ?? []).map((t) => ({
          name: t.name,
          description: t.description,
          inputSchema: t.inputSchema,
        })),
      };
    },
    async call(name, args) {
      const reply = await request("tools/call", { name, arguments: args });
      if (reply.error) {
        return {
          isError: true,
          content: [{ type: "text", text: reply.error.message }],
        };
      }
      return {
        isError: reply.result?.isError === true,
        content: reply.result?.content ?? [],
      };
    },
    stop() {
      child.kill();
    },
  };
}

export default {
  id: "impulse",
  async setup(ctx) {
    if (!cli) return;
    const client = `${process.pid}-${Date.now()}`;
    const env = {};
    for (const name of paneVariables) {
      if (process.env[name]) env[name] = process.env[name];
    }
    const rpc = ctx.client.rpc(Impulse);

    // A subagent's session belongs to the one that started it.
    const root = (sessionID) => {
      try {
        return ctx.data.session.root(sessionID) || sessionID;
      } catch {
        return sessionID;
      }
    };
    const isSubagent = (sessionID) => root(sessionID) !== sessionID;

    // The terminal each session is in: the last to show it on screen
    // (opencode's tabs are the same in every terminal, so they don't say).
    const owners = new Map();
    const onScreen = () => {
      const route = ctx.ui.router.current();
      return route?.type === "session" ? route.sessionID : undefined;
    };
    let claimed;
    const claim = () => {
      const sessionID = onScreen();
      if (!sessionID || sessionID === claimed) return;
      claimed = sessionID;
      owners.set(sessionID, client);
      rpc.show({ client, sessionID }).catch(() => {});
    };
    // Whether a session is this terminal's: shown here last, or on screen
    // here and nobody else's yet.
    const mine = (sessionID) => {
      const top = root(sessionID);
      const owner = owners.get(top);
      if (owner) return owner === client;
      if (onScreen() !== top) return false;
      claimed = undefined;
      claim();
      return true;
    };

    // The last event sent for each session (no repeats of the same one).
    // Events about a subagent (`agent_id`, its session) are its agent's,
    // but don't name that agent's session.
    const last = new Map();
    const send = (event, sessionID, extra = {}) => {
      last.set(sessionID, event);
      return hook(
        event,
        Promise.resolve(extra).then((extra) => ({
          session_id:
            isSubagent(sessionID) || extra.agent_id ? undefined : sessionID,
          cwd: ctx.location?.directory ?? process.cwd(),
          ...extra,
        })),
      );
    };

    hook("start", { cwd: ctx.location?.directory ?? process.cwd() });

    // What opencode is doing, from its events. A turn's end says which
    // subagents it leaves running in the background: they bring the agent
    // back when they finish, so it isn't done yet.
    const ends = new Set([
      "session.execution.succeeded",
      "session.execution.interrupted",
    ]);
    const turn = (type) => (event) => {
      const sessionID = event.data?.sessionID;
      if (!sessionID || isSubagent(sessionID) || !mine(sessionID)) return;
      if (last.get(sessionID) === type) return;
      const extra = {
        message: event.data?.error?.message ?? event.data?.message,
      };
      if (!ends.has(type)) return send(type, sessionID, extra);
      const running = rpc
        .subagents({ sessionID })
        .then((reply) => ({
          ...extra,
          background_tasks: (reply?.running ?? []).map((task) => ({
            type: "subagent",
            ...task,
          })),
        }))
        .catch(() => extra);
      send(type, sessionID, running);
    };
    const needs = (type) => (event) => {
      const sessionID = event.data?.sessionID;
      if (!sessionID || !mine(sessionID)) return;
      const action = event.data?.action;
      send(type, sessionID, {
        tool_name: action === "impulse_merge" ? "shell" : action,
      });
    };
    const stops = [
      ctx.data.on(
        "session.execution.started",
        turn("session.execution.started"),
      ),
      ctx.data.on(
        "session.execution.succeeded",
        turn("session.execution.succeeded"),
      ),
      ctx.data.on("session.execution.failed", turn("session.execution.failed")),
      ctx.data.on(
        "session.execution.interrupted",
        turn("session.execution.interrupted"),
      ),
      ctx.data.on("permission.asked", needs("permission.asked")),
      ctx.data.on("permission.replied", needs("permission.replied")),
      ctx.data.on("form.created", needs("form.created")),
      ctx.data.on("form.replied", needs("form.replied")),
      ctx.data.on("form.cancelled", needs("form.cancelled")),
    ];

    // Who has which session, as terminals show them.
    stops.push(
      rpc.events.on("owner", (event) => {
        const { sessionID, client: owner } = event.data;
        if (owner) owners.set(sessionID, owner);
        else owners.delete(sessionID);
        // Another terminal let go of the session on screen here.
        if (!owner && onScreen() === sessionID) {
          claimed = undefined;
          claim();
        }
      }),
    );
    const watch = setInterval(claim, 500);
    stops.push(() => clearInterval(watch));

    // Impulse's tools, from this pane's own MCP server.
    const mcp = mcpServer();
    const catalog = await mcp.start().catch(() => ({ tools: [] }));

    // The service's questions about sessions this terminal shows.
    const answer = (id, mine, reply) =>
      rpc.answer({ id, client, mine, answer: reply }).catch(() => {});
    stops.push(
      rpc.events.on("ask", async (event) => {
        const { id, sessionID, event: name, payload } = event.data;
        if (!mine(sessionID)) return answer(id, false);
        switch (name) {
          case "session.start": {
            // Subagents don't need the summary: their session does.
            if (isSubagent(sessionID)) return answer(id, true, {});
            const reply = await send("session.start", sessionID);
            return answer(id, true, { context: reply.context });
          }
          case "tool.execute.before":
          case "tool.execute.after": {
            const reply = await send(name, sessionID, payload);
            return answer(id, true, { ...reply, env });
          }
          case "subagent.start":
          case "subagent.stop": {
            const reply = await send(name, sessionID, payload);
            return answer(id, true, reply);
          }
          case "tool.call": {
            const reply = await mcp.call(payload.name, payload.arguments);
            return answer(id, true, reply);
          }
          default:
            return answer(id, true, {});
        }
      }),
    );
    const attach = () =>
      rpc
        .attach({
          client,
          tools: catalog.tools,
          instructions: catalog.instructions,
        })
        .catch(() => ({}));
    const attached = await attach();
    // Still here (the service forgets a terminal that goes quiet).
    const alive = setInterval(attach, 30_000);
    stops.push(() => clearInterval(alive));
    for (const [sessionID, owner] of Object.entries(attached?.owners ?? {})) {
      if (!owners.has(sessionID)) owners.set(sessionID, owner);
    }
    claim();

    return async () => {
      for (const stop of stops) stop();
      await rpc.detach({ client }).catch(() => {});
      mcp.stop();
    };
  },
};
