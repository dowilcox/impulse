#!/bin/bash
# Stand-in for an agent CLI in Impulse's documentation screenshots. Installed
# as ~/bin/claude and ~/bin/codex in the demo home: Impulse recognizes the
# agent by its name, and this script reports its state through `impulse hook`
# the way the real agents' hooks do. It never talks to any model.
#
#   claude waiting   asks to approve an edit (the agent needs you)
#   claude working   keeps working on a prompt
#   claude turn      makes one edit (a checkpointed turn), then waits
#   claude done      finishes a turn without editing anything
#   claude finish    mid-turn, asks Impulse to finish its own task (as the
#                    MCP server's finish_task does), saving the answer in
#                    ~/mcp/finish.txt, then keeps working
#   claude subagents starts two subagents in the background that both edit
#                    src/photos.ts, ends its turn while they run, and lets
#                    them finish once ~/mcp/subagents-go exists, both before
#                    the first of the turns Claude Code takes to report them;
#                    after that turn it waits for ~/mcp/subagents-go-on.
#                    Impulse's answers go to ~/mcp/subagent-notes.txt
#   claude subagent-worktrees
#                    starts two subagents in the background, each in a
#                    worktree of its own as Claude Code makes them
#                    (.claude/worktrees/<id>), ends its turn while they run,
#                    and once ~/mcp/subagent-worktrees-go exists lets them
#                    finish: the first with a change, the second with none,
#                    so its worktree is removed as Claude Code removes it
#   claude mcp …     what Agent Integration runs to install Impulse's MCP
#                    server: recorded in ~/mcp/<agent>-mcp.txt, one call a
#                    line, as JSON
#   claude           sits at an empty prompt

agent=$(basename "$0")
mode=${1:-idle}

if [ "$mode" = mcp ]; then
  mkdir -p "$HOME/mcp"
  python3 -c 'import json, sys; print(json.dumps(sys.argv[1:]))' "$@" >> "$HOME/mcp/$agent-mcp.txt"
  echo "Added stdio MCP server to user config (demo agent)"
  exit 0
fi
dim=$'\e[2m' bold=$'\e[1m' green=$'\e[32m' yellow=$'\e[33m' cyan=$'\e[36m' reset=$'\e[0m'

hook() { command -v impulse >/dev/null && impulse hook "$agent" "$1" "${2:-{\}}" >/dev/null 2>&1; }

header() {
  printf '%s╭──────────────────────────────────────────────────────────╮%s\n' "$dim" "$reset"
  printf '%s│%s %s%s%s  %s(demo agent)%s%*s%s│%s\n' "$dim" "$reset" "$bold" "$agent" "$reset" "$dim" "$reset" \
    $((43 - ${#agent})) "" "$dim" "$reset"
  # ~ for the home folder (bash 5 would expand a ~ in ${PWD/#$HOME/~}).
  local here=$PWD
  case $here in "$HOME"/*) here="~${here#"$HOME"}" ;; esac
  printf '%s│%s %s%-56s%s %s│%s\n' "$dim" "$reset" "$dim" "$here" "$reset" "$dim" "$reset"
  printf '%s╰──────────────────────────────────────────────────────────╯%s\n\n' "$dim" "$reset"
}

prompt_line() { printf '%s>%s %s\n\n' "$cyan" "$reset" "$1"; }
step() { printf '%s●%s %s %s%s%s\n' "$green" "$reset" "$1" "$dim" "$2" "$reset"; }
detail() { printf '  %s⎿  %s%s\n' "$dim" "$1" "$reset"; }

# An interactive program, as far as the terminal can tell (bracketed paste),
# so Impulse gives it the keyboard and shows its toolbelt.
printf '\e[?2004h'
# Let Impulse notice the process before the first hook arrives.
sleep 2.5
hook SessionStart '{"session_id":"demo-session"}'
header

case "$mode" in
  waiting)
    hook UserPromptSubmit '{"session_id":"demo-session"}'
    prompt_line "Some trails have no trailhead elevation, so the forecast page shows
  \"undefined m\". Fall back to the nearest known trailhead in the same region."
    sleep 0.4; step "Read" "src/trails.ts (41 lines)"
    sleep 0.4; step "Search" "\"trailheadElevationM\" in src (3 matches)"
    sleep 0.4; step "Update" "src/trails.ts"
    detail "Add trailheadElevation(trail) with a same-region fallback"
    echo
    printf '%sAllow this edit to src/trails.ts?%s\n' "$yellow" "$reset"
    printf '  %s❯ 1. Yes%s\n' "$bold" "$reset"
    printf '    2. Yes, and allow edits for the rest of this session\n'
    printf '    3. No, and say what to do instead\n'
    hook Notification '{"message":"Needs your permission to edit src/trails.ts","session_id":"demo-session"}'
    ;;
  finish)
    hook UserPromptSubmit '{"session_id":"demo-session"}'
    prompt_line "Finish this task."
    sleep 1
    step "Finish" "${PWD##*/}"
    mkdir -p "$HOME/mcp"
    impulse tasks finish "${PWD##*/}" > "$HOME/mcp/finish.tmp" 2>&1
    echo "exit $?" >> "$HOME/mcp/finish.tmp"
    mv "$HOME/mcp/finish.tmp" "$HOME/mcp/finish.txt"
    detail "$(head -1 "$HOME/mcp/finish.txt")"
    while true; do sleep 1; done
    ;;
  working)
    hook UserPromptSubmit '{"session_id":"demo-session"}'
    prompt_line "Add a photos endpoint: GET /trails/:id/photos, newest first, with tests."
    step "Read" "src/server.ts, src/trails.ts"
    step "Write" "src/photos.ts"
    detail "photosFor(trailId) with an in-memory store"
    step "Update" "src/server.ts"
    echo
    spin='⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏'
    i=0
    while true; do
      printf '\r%s%s%s Writing test/photos.test.ts… %s(%ds)%s ' "$yellow" "${spin:i%10:1}" "$reset" "$dim" $((i / 5)) "$reset"
      i=$((i + 1))
      sleep 0.2
    done
    ;;
  turn)
    hook UserPromptSubmit '{"session_id":"demo-session"}'
    prompt_line "Some trails have no trailhead elevation, so the forecast page shows
  \"undefined m\". Fall back to the nearest known trailhead in the same region."
    sleep 1.5
    step "Read" "src/trails.ts (41 lines)"
    fixed="$HOME/../agent/trails.fixed.ts"
    [ -f "$fixed" ] && cp "$fixed" src/trails.ts
    step "Update" "src/trails.ts"
    detail "Added trailheadElevation(trail): the trail's own elevation, else the"
    detail "first known one in the same region"
    sleep 1
    echo
    echo "Done. Trails without an elevation now borrow one from their region;"
    echo "Annette Lake now gets 205 m from Mount Si's trailhead."
    hook Stop '{"session_id":"demo-session"}'
    ;;
  subagents)
    mkdir -p "$HOME/mcp"
    note() { impulse hook "$agent" "$1" "$2" >> "$HOME/mcp/subagent-notes.txt" 2>&1; }
    s='"session_id":"demo-session"'
    file="$PWD/src/photos.ts"
    hook UserPromptSubmit "{$s}"
    prompt_line "Add the photos endpoint and its tests, in parallel."
    start() { # id, description
      note PreToolUse "{$s,\"tool_name\":\"Agent\",\"tool_input\":{\"description\":\"$2\"}}"
      note SubagentStart "{$s,\"agent_id\":\"$1\",\"agent_type\":\"general-purpose\",\"cwd\":\"$PWD\"}"
      note PostToolUse "{$s,\"tool_name\":\"Agent\",\"tool_input\":{\"description\":\"$2\"},\"tool_response\":{\"isAsync\":true,\"agentId\":\"$1\"}}"
      step "Agent" "$2 (running in the background)"
    }
    edit() { # id
      note PostToolUse "{$s,\"agent_id\":\"$1\",\"agent_type\":\"general-purpose\",\"tool_name\":\"Write\",\"tool_input\":{\"file_path\":\"$file\"},\"cwd\":\"$PWD\"}"
    }
    start sub-a "Write the photos endpoint"
    start sub-b "Write the photo tests"
    edit sub-a
    edit sub-b
    running='{"id":"sub-a","type":"subagent","status":"running","description":"Write the photos endpoint","agent_type":"general-purpose"},{"id":"sub-b","type":"subagent","status":"running","description":"Write the photo tests","agent_type":"general-purpose"}'
    hook Stop "{$s,\"background_tasks\":[$running]}"
    echo; echo "Both subagents are working; I'll check their results when they finish."
    touch "$HOME/mcp/subagents-ready"
    for _ in $(seq 300); do [ -e "$HOME/mcp/subagents-go" ] && break; sleep 0.2; done
    # As Claude Code does it: both stop, then a turn reports each one.
    note SubagentStop "{$s,\"agent_id\":\"sub-a\",\"agent_type\":\"general-purpose\",\"last_assistant_message\":\"Added GET /trails/:id/photos\"}"
    note SubagentStop "{$s,\"agent_id\":\"sub-b\",\"agent_type\":\"general-purpose\",\"last_assistant_message\":\"Added test/photos.test.ts\"}"
    reported() { # id
      hook UserPromptSubmit "{$s,\"prompt\":\"<task-notification>\\n<task-id>$1</task-id>\\n<status>completed</status>\\n</task-notification>\"}"
    }
    reported sub-a
    hook Stop "{$s,\"background_tasks\":[]}"
    echo "The endpoint is in; the tests are next."
    touch "$HOME/mcp/subagents-one-reported"
    for _ in $(seq 300); do [ -e "$HOME/mcp/subagents-go-on" ] && break; sleep 0.2; done
    reported sub-b
    hook Stop "{$s,\"background_tasks\":[]}"
    echo "Both finished: the endpoint and its tests are in."
    touch "$HOME/mcp/subagents-finished"
    ;;
  subagent-worktrees)
    mkdir -p "$HOME/mcp"
    s='"session_id":"demo-session"'
    hook UserPromptSubmit "{$s}"
    prompt_line "Try both fixes for the elevation units, each in its own worktree."
    start() { # id, description, the folder its SubagentStart names
      git worktree add -q -b "worktree-$1" ".claude/worktrees/$1" HEAD
      input="{\"description\":\"$2\",\"isolation\":\"worktree\"}"
      hook PreToolUse "{$s,\"tool_name\":\"Agent\",\"tool_input\":$input}"
      # The call returns before a subagent with its own worktree starts.
      hook PostToolUse "{$s,\"tool_name\":\"Agent\",\"tool_input\":$input,\"tool_response\":{\"isAsync\":true,\"agentId\":\"$1\"}}"
      hook SubagentStart "{$s,\"agent_id\":\"$1\",\"agent_type\":\"general-purpose\",\"cwd\":\"$3\"}"
      step "Agent" "$2 (running in the background)"
    }
    start agent-w1 "Use metres throughout" "$PWD/.claude/worktrees/agent-w1"
    # Named only by the folder its tools run in.
    start agent-w2 "Convert at the edges" "$PWD"
    hook PreToolUse "{$s,\"agent_id\":\"agent-w2\",\"agent_type\":\"general-purpose\",\"tool_name\":\"Bash\",\"tool_input\":{\"command\":\"ls\"},\"cwd\":\"$PWD/.claude/worktrees/agent-w2\"}"
    changed="$PWD/.claude/worktrees/agent-w1/src/units.ts"
    mkdir -p "$(dirname "$changed")" && echo 'export const METRES = true;' > "$changed"
    hook PostToolUse "{$s,\"agent_id\":\"agent-w1\",\"agent_type\":\"general-purpose\",\"tool_name\":\"Write\",\"tool_input\":{\"file_path\":\"$changed\"},\"cwd\":\"$PWD/.claude/worktrees/agent-w1\"}"
    running='{"id":"agent-w1","type":"subagent","status":"running","description":"Use metres throughout","agent_type":"general-purpose"},{"id":"agent-w2","type":"subagent","status":"running","description":"Convert at the edges","agent_type":"general-purpose"}'
    hook Stop "{$s,\"background_tasks\":[$running]}"
    echo; echo "Both subagents are working in their own worktrees."
    touch "$HOME/mcp/subagent-worktrees-ready"
    for _ in $(seq 300); do [ -e "$HOME/mcp/subagent-worktrees-go" ] && break; sleep 0.2; done
    hook SubagentStop "{$s,\"agent_id\":\"agent-w1\",\"agent_type\":\"general-purpose\",\"last_assistant_message\":\"Elevations are metres everywhere\"}"
    hook SubagentStop "{$s,\"agent_id\":\"agent-w2\",\"agent_type\":\"general-purpose\",\"last_assistant_message\":\"Nothing to change\"}"
    # Claude Code removes the worktree of a subagent that changed nothing.
    git worktree remove --force .claude/worktrees/agent-w2 && git branch -q -D worktree-agent-w2
    for id in agent-w1 agent-w2; do
      hook UserPromptSubmit "{$s,\"prompt\":\"<task-notification>\\n<task-id>$id</task-id>\\n<status>completed</status>\\n</task-notification>\"}"
      hook Stop "{$s,\"background_tasks\":[]}"
    done
    echo "One fix is in its worktree; the other needed nothing."
    touch "$HOME/mcp/subagent-worktrees-finished"
    ;;
  done)
    hook UserPromptSubmit '{"session_id":"demo-session"}'
    prompt_line "Why does the forecast endpoint return 502 for Skyline Trail?"
    sleep 1
    step "Read" "src/forecast.ts, src/server.ts"
    step "Bash" "curl -s localhost:3000/trails/skyline/forecast"
    echo
    echo "The weather service rejects elevations above 1500 m without an API key,"
    echo "and FORECAST_KEY isn't set in this worktree. Nothing to change in code."
    hook Stop '{"session_id":"demo-session"}'
    ;;
esac

echo
printf '%s>%s ' "$cyan" "$reset"
# Wait like an interactive session would.
read -r _
