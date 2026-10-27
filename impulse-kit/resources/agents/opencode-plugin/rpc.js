// Impulse's opencode plugin: how its two halves talk.
//
// opencode runs one background service for every terminal, so the half in
// the service (server.js) can't tell which Impulse pane a session is in.
// The half in each terminal (tui.js) can: it runs in that pane, with its
// environment. Each terminal says which session it shows (`show`: the last
// to show one has it, and every terminal hears who does, from `owner`).
// The service half asks with an `ask` event every terminal sees; the one
// the session is in answers through `answer`, and the others say it isn't
// theirs.
//
// Impulse replaces or removes this folder as a whole, so changes made to
// it here don't last.

export const Impulse = {
  id: "impulse",
  methods: {
    // A terminal in Impulse is here, with Impulse's tools.
    attach: {
      input: {
        type: "object",
        properties: {
          client: { type: "string" },
          tools: { type: "array" },
          instructions: { type: "string" },
        },
        required: ["client"],
      },
      output: { type: "object" },
    },
    // It's gone.
    detach: {
      input: {
        type: "object",
        properties: { client: { type: "string" } },
        required: ["client"],
      },
      output: { type: "object" },
    },
    // It shows a session (the last terminal to show one has it).
    show: {
      input: {
        type: "object",
        properties: {
          client: { type: "string" },
          sessionID: { type: "string" },
        },
        required: ["client", "sessionID"],
      },
      output: { type: "object" },
    },
    // Its answer to an ask: `mine` false when the session isn't in it.
    answer: {
      input: {
        type: "object",
        properties: {
          id: { type: "string" },
          client: { type: "string" },
          mine: { type: "boolean" },
          answer: { type: "object" },
        },
        required: ["id", "client", "mine"],
      },
      output: { type: "object" },
    },
  },
  events: {
    // Which terminal a session is in now ("" when none).
    owner: {
      schema: {
        type: "object",
        properties: {
          sessionID: { type: "string" },
          client: { type: "string" },
        },
        required: ["sessionID", "client"],
      },
    },
    ask: {
      schema: {
        type: "object",
        properties: {
          id: { type: "string" },
          sessionID: { type: "string" },
          event: { type: "string" },
          payload: { type: "object" },
        },
        required: ["id", "sessionID", "event"],
      },
    },
  },
};
