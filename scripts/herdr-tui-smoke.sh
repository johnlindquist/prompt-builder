#!/usr/bin/env bash
# Smoke-tests prompt-builder's keyboard shortcuts by driving the built binary
# in a fresh herdr pane. Run from inside a herdr session (HERDR_PANE_ID set):
#
#   scripts/herdr-tui-smoke.sh
#
# The pane is split off the current one and zoomed so the app has room, then
# closed on exit. The app runs inside tmux within that pane: herdr's
# send-keys drops modifiers like Shift+Tab, so keys are injected through
# tmux (which encodes BTab, M-b, C-u, ... correctly) while the pane keeps
# the run visible in the workspace.
set -u

cd "$(dirname "$0")/.." || exit 1

BIN="${BIN:-./target/debug/prompt-builder}"
BASE_PANE="${HERDR_PANE_ID:?not inside a herdr pane}"
DELAY="${DELAY:-0.5}"
TMUX_SESSION="pb-smoke-$$"

PANE=""
PASS=0
FAIL=0
FAILURES=()

cleanup() {
  tmux kill-session -t "$TMUX_SESSION" 2>/dev/null
  if [[ -n "$PANE" ]]; then
    herdr pane zoom "$PANE" --off >/dev/null 2>&1
    herdr pane close "$PANE" >/dev/null 2>&1
  fi
}
trap cleanup EXIT

keys() {
  tmux send-keys -t "$TMUX_SESSION" "$@"
  sleep "$DELAY"
}

text() {
  tmux send-keys -t "$TMUX_SESSION" -l "$1"
  sleep "$DELAY"
}

snap() {
  tmux capture-pane -t "$TMUX_SESSION" -p 2>/dev/null
}

check() { # check <description> <expected fixed string on screen>
  if snap | grep -qF -- "$2"; then
    PASS=$((PASS + 1))
    printf 'PASS  %s\n' "$1"
  else
    FAIL=$((FAIL + 1))
    FAILURES+=("$1")
    printf 'FAIL  %s (expected on screen: %s)\n' "$1" "$2"
  fi
}

check_absent() { # check_absent <description> <string that must NOT be on screen>
  if snap | grep -qF -- "$2"; then
    FAIL=$((FAIL + 1))
    FAILURES+=("$1")
    printf 'FAIL  %s (unexpected on screen: %s)\n' "$1" "$2"
  else
    PASS=$((PASS + 1))
    printf 'PASS  %s\n' "$1"
  fi
}

wait_for() {
  local tries=40
  while ((tries--)); do
    snap | grep -qF -- "$1" && return 0
    sleep 0.25
  done
  return 1
}

cargo build 2>&1 | tail -1
[[ -x "$BIN" ]] || { echo "missing binary: $BIN"; exit 1; }

PANE=$(herdr pane split --pane "$BASE_PANE" --direction right --no-focus 2>/dev/null |
  sed -n 's/.*"pane_id":"\([^"]*\)".*/\1/p' | head -1)
[[ -n "$PANE" ]] || { echo "failed to split pane"; exit 1; }
echo "test pane: $PANE (tmux session: $TMUX_SESSION)"
herdr pane zoom "$PANE" --on >/dev/null
herdr pane run "$PANE" "tmux new-session -A -s $TMUX_SESSION $BIN" >/dev/null

wait_for "Compose new task" || { echo "app did not start"; snap; exit 1; }

# --- Name field: focus-aware hints + readline editing ---
check "launch: Name focused with its own hint row" "Shift+Tab back"
text "hello brave world"
keys C-w
check "name: Ctrl+W deletes the word before the cursor" "hello brave"
check_absent "name: Ctrl+W removed the last word" "world"
keys M-b M-d
check_absent "name: Alt+B then Alt+D deletes the word under the cursor" "brave"
keys C-u
check_absent "name: Ctrl+U kills back to the start" "hello"
keys C-k
check "name: Ctrl+K kills to the end (field empty again)" "Optional conversation name"

# --- Focus cycling ---
keys Tab
check "tab: focus moves to Prompt (hint row swaps)" "Esc clear"
keys BTab
check "shift+tab: focus cycles backwards to Name" "Shift+Tab back"
keys Escape
check "esc: from Name returns focus to Prompt" "Esc clear"

# --- Skill search: $ token popup ---
text '$fus'
check "skills: typing \$fus opens the search" 'Skills $fus'
check "skills: best match is selected" '> $fusion Fusion'
keys C-n
check "skills: Ctrl+N moves selection down" '> $fusion-max'
keys BTab
check "skills: Shift+Tab moves selection back up" '> $fusion Fusion'
keys Tab
check "skills: Tab accepts the mention into the prompt" '$fusion'
check_absent "skills: popup closes after accept" 'Skills $'

# --- Bonus: deleting into a mention re-triggers the search ---
keys BSpace BSpace
check "bonus: backspacing into the mention reopens search" 'Skills $fusio'
keys Escape
check_absent "bonus: Esc dismisses the reopened search" 'Skills $fusio'
check "bonus: partial mention text is kept" '$fusio'

# --- Esc clears the draft, history recovers it ---
keys Escape
check "esc: clears the prompt draft" "Compose new task"
keys Up
check "up: recalls the Esc-cleared draft from history" '$fusio'

# --- Undo / redo (Ctrl+U undo, Alt+R redo) ---
keys Escape
text "abcdef"
keys C-u
check_absent "undo: Ctrl+U removes typed text" "abcdef"
keys M-r
check "redo: Alt+R restores the undone text" "abcdef"

# --- Target selector ---
keys Tab
check "tab: reaches target selector (hint row swaps)" "←/→ target"
keys Escape
check "esc: from target selector returns to Prompt" "Esc clear"

# --- Exit: Ctrl+C clears, second Ctrl+C quits ---
keys C-c
keys C-c
sleep 1
if tmux has-session -t "$TMUX_SESSION" 2>/dev/null; then
  FAIL=$((FAIL + 1))
  FAILURES+=("ctrl+c: app exits")
  echo "FAIL  ctrl+c: app exits (tmux session still alive)"
else
  PASS=$((PASS + 1))
  echo "PASS  ctrl+c: app exits"
fi

echo
echo "passed: $PASS  failed: $FAIL"
if ((FAIL > 0)); then
  printf 'failed tests:\n'
  printf '  - %s\n' "${FAILURES[@]}"
  exit 1
fi
