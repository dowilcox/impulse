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
#   claude           sits at an empty prompt

agent=$(basename "$0")
mode=${1:-idle}
dim=$'\e[2m' bold=$'\e[1m' green=$'\e[32m' yellow=$'\e[33m' cyan=$'\e[36m' reset=$'\e[0m'

hook() { command -v impulse >/dev/null && impulse hook "$agent" "$1" "${2:-{\}}" >/dev/null 2>&1; }

header() {
  printf '%s╭──────────────────────────────────────────────────────────╮%s\n' "$dim" "$reset"
  printf '%s│%s %s%s%s  %s(demo agent)%s%*s%s│%s\n' "$dim" "$reset" "$bold" "$agent" "$reset" "$dim" "$reset" \
    $((43 - ${#agent})) "" "$dim" "$reset"
  printf '%s│%s %s%-56s%s %s│%s\n' "$dim" "$reset" "$dim" "${PWD/#$HOME/~}" "$reset" "$dim" "$reset"
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
