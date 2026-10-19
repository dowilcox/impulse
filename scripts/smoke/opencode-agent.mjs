#!/usr/bin/env node
// Stand-in for opencode in the smoke checks, installed as ~/bin/opencode in
// the demo home. It loads Impulse's plugin from where opencode would
// (~/.config/opencode/plugins/impulse.js, put there by Agent Integration)
// and calls its hooks the way opencode does during a turn, so the real
// plugin talks to the real Impulse. It never talks to any model. What the
// plugin hands back (the system prompt, tool output, a stopped command) is
// saved in ~/opencode/<mode>.json, then it waits, like an agent at its
// prompt.
//
//   opencode waiting   a prompt, then a permission request left unanswered
//   opencode turn      a prompt, an edit of a file a task also changes, done
//   opencode merge     a prompt, then git merge of a busy task's branch, twice

import { mkdirSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

const mode = process.argv[2] ?? "idle";
const home = homedir();
const sessionID = `ses_smoke${mode}`;
const client = { session: { get: async () => ({ data: { id: sessionID } }) } };
const pause = (seconds) =>
  new Promise((done) => setTimeout(done, seconds * 1000));

// Let Impulse notice the process before the first hook arrives.
await pause(2.5);
const { ImpulsePlugin } = await import(
  join(home, ".config/opencode/plugins/impulse.js")
);
const hooks = await ImpulsePlugin({ client, directory: process.cwd() });
const result = {};
console.log(`opencode (stand-in): ${mode}`);

if (mode !== "idle") {
  await hooks["chat.message"]({ sessionID });
  const system = { system: ["You are opencode."] };
  await hooks["experimental.chat.system.transform"]({ sessionID }, system);
  result.system = system.system;
}
if (mode === "waiting") {
  await hooks.event({
    event: {
      type: "permission.asked",
      properties: { sessionID, permission: "edit" },
    },
  });
  console.log("Allow this edit to src/trails.ts?");
} else if (mode === "turn") {
  // Overlaps are worked out in the background: give them a moment.
  await pause(3);
  const edit = { output: "Edit applied successfully." };
  await hooks["tool.execute.after"](
    { tool: "edit", sessionID, args: { filePath: "src/photos.ts" } },
    edit,
  );
  result.editOutput = edit.output;
  await hooks.event({
    event: { type: "session.idle", properties: { sessionID } },
  });
  console.log("Done.");
} else if (mode === "merge") {
  result.attempts = [];
  for (let attempt = 0; attempt < 2; attempt++) {
    try {
      await hooks["tool.execute.before"](
        { tool: "bash", sessionID },
        { args: { command: "git merge add-trail-photos" } },
      );
      result.attempts.push("ran");
    } catch (error) {
      result.attempts.push(error.message);
    }
  }
  await hooks.event({
    event: { type: "session.idle", properties: { sessionID } },
  });
  console.log("Done.");
}

mkdirSync(join(home, "opencode"), { recursive: true });
writeFileSync(join(home, "opencode", `${mode}.json`), JSON.stringify(result));
setInterval(() => {}, 1000);
