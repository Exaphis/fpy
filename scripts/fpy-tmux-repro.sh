#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
ACTION="${1:-ctrl-d}"
SESSION="${SESSION:-fpy-repro-$$}"
TMUX_SIZE="${TMUX_SIZE:-120x40}"
WIDTH="${TMUX_SIZE%x*}"
HEIGHT="${TMUX_SIZE#*x}"
if [ -z "${PYTHON_BIN+x}" ]; then
  if [ -x "$ROOT/.venv/bin/python" ]; then
    PYTHON_BIN="$ROOT/.venv/bin/python"
  else
    PYTHON_BIN="python3"
  fi
fi
if [ -z "${FPY_BIN+x}" ] && [ -x "$ROOT/target/debug/fpy" ]; then
  FPY_BIN="$ROOT/target/debug/fpy"
fi
if [ -z "${FPY_CMD+x}" ]; then
  if [ -n "${FPY_BIN:-}" ]; then
    FPY_CMD="$FPY_BIN run --python $PYTHON_BIN"
  else
    FPY_CMD="cargo run -- run --python $PYTHON_BIN"
  fi
fi
PRE_INPUT="${PRE_INPUT-1+1}"
INPUTS="${INPUTS-$PRE_INPUT}"
CAPTURE_LINES="${CAPTURE_LINES:-40}"
CAPTURE_VISIBLE_ONLY="${CAPTURE_VISIBLE_ONLY:-0}"
BEFORE_LOG="${BEFORE_LOG:-$ROOT/target/fpy-tmux-repro.before.log}"
AFTER_LOG="${AFTER_LOG:-$ROOT/target/fpy-tmux-repro.after.log}"
PASTE_TEXT="${PASTE_TEXT:-x = 1
y = 2}"
SEARCH_QUERY="${SEARCH_QUERY:-}"
SEARCH_DOWN_COUNT="${SEARCH_DOWN_COUNT:-0}"
TIMEOUT_SECONDS="${TIMEOUT_SECONDS:-20}"
POLL_SECONDS="${POLL_SECONDS:-0.05}"
EXIT_WAIT="${EXIT_WAIT:-0.2}"

mkdir -p "$ROOT/target"

cleanup() {
  tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true
}
trap cleanup EXIT INT TERM

now_ns() {
  perl -MTime::HiRes=time -e 'printf "%.0f\n", time() * 1000000000'
}

capture_pane() {
  if [ "$CAPTURE_VISIBLE_ONLY" = "1" ]; then
    tmux capture-pane -pt "$1"
  else
    tmux capture-pane -pt "$1" -S "-$CAPTURE_LINES"
  fi
}

pane_has_usable_input() {
  pane_text=$1
  printf '%s' "$pane_text" | grep -F "Ctrl-P palette" >/dev/null 2>&1
}

pane_is_submit_ready() {
  pane_text=$1
  prompt_line=$(printf '%s\n' "$pane_text" | grep -F "Ctrl-P palette" | tail -n 1 || true)
  [ -n "$prompt_line" ] || return 1
  printf '%s' "$prompt_line" | grep -F "Connecting to kernel..." >/dev/null 2>&1 && return 1
  printf '%s' "$prompt_line" | grep -F "Kernel busy. Ctrl-C to interrupt" >/dev/null 2>&1 && return 1
  return 0
}

pane_has_text() {
  pane_text=$1
  needle=$2
  printf '%s' "$pane_text" | grep -F "$needle" >/dev/null 2>&1
}

wait_for_predicate() {
  session=$1
  predicate=$2
  description=$3
  streak_required=${4:-1}
  deadline=$(($(now_ns) + TIMEOUT_SECONDS * 1000000000))
  streak=0

  while :; do
    pane_text=$(capture_pane "$session")
    if "$predicate" "$pane_text"; then
      streak=$((streak + 1))
      if [ "$streak" -ge "$streak_required" ]; then
        return 0
      fi
    else
      streak=0
    fi

    if [ "$(now_ns)" -ge "$deadline" ]; then
      log_file="$ROOT/target/$SESSION-$description.timeout.log"
      printf '%s\n' "$pane_text" > "$log_file"
      printf 'timed out waiting for %s; pane saved to %s\n' "$description" "$log_file" >&2
      return 1
    fi

    sleep "$POLL_SECONDS"
  done
}

wait_for_text() {
  session=$1
  needle=$2
  description=$3
  deadline=$(($(now_ns) + TIMEOUT_SECONDS * 1000000000))

  while :; do
    pane_text=$(capture_pane "$session")
    if pane_has_text "$pane_text" "$needle"; then
      return 0
    fi

    if [ "$(now_ns)" -ge "$deadline" ]; then
      log_file="$ROOT/target/$SESSION-$description.timeout.log"
      printf '%s\n' "$pane_text" > "$log_file"
      printf 'timed out waiting for %s; pane saved to %s\n' "$description" "$log_file" >&2
      return 1
    fi

    sleep "$POLL_SECONDS"
  done
}

wait_for_usable_input() {
  wait_for_predicate "$1" pane_has_usable_input usable-input 2
}

wait_for_submit_ready() {
  wait_for_predicate "$1" pane_is_submit_ready submit-ready 2
}

last_prompt_line() {
  pane_text=$1
  printf '%s\n' "$pane_text" | grep -F "Ctrl-P palette" | tail -n 1 || true
}

pane_prompt_line_differs() {
  pane_text=$1
  expected=$2
  prompt_line=$(last_prompt_line "$pane_text")
  [ -n "$prompt_line" ] || return 1
  [ "$prompt_line" != "$expected" ]
}

wait_for_prompt_line_change() {
  session=$1
  expected=$2
  description=$3
  deadline=$(($(now_ns) + TIMEOUT_SECONDS * 1000000000))

  while :; do
    pane_text=$(capture_pane "$session")
    if pane_prompt_line_differs "$pane_text" "$expected"; then
      return 0
    fi

    if [ "$(now_ns)" -ge "$deadline" ]; then
      log_file="$ROOT/target/$SESSION-$description.timeout.log"
      printf '%s\n' "$pane_text" > "$log_file"
      printf 'timed out waiting for %s; pane saved to %s\n' "$description" "$log_file" >&2
      return 1
    fi

    sleep "$POLL_SECONDS"
  done
}

submit_cell() {
  session=$1
  text=$2
  wait_for_submit_ready "$session"
  before_pane=$(capture_pane "$session")
  before_prompt=$(last_prompt_line "$before_pane")
  tmux send-keys -t "$session" -l "$text"
  tmux send-keys -t "$session" Enter
  if [ -n "$before_prompt" ]; then
    wait_for_prompt_line_change "$session" "$before_prompt" submit-started
  fi
  wait_for_submit_ready "$session"
}

submit_lines() {
  session=$1
  inputs=$2
  if [ -z "$inputs" ]; then
    return 0
  fi

  old_ifs=$IFS
  IFS='
'
  for input in $inputs; do
    submit_cell "$session" "$input"
  done
  IFS=$old_ifs
}

if [ -n "${FPY_HISTORY_DIR+x}" ] && [ -n "${XDG_DATA_HOME+x}" ]; then
  tmux new-session -d -s "$SESSION" -x "$WIDTH" -y "$HEIGHT" env FPY_HISTORY_DIR="$FPY_HISTORY_DIR" XDG_DATA_HOME="$XDG_DATA_HOME" zsh
elif [ -n "${FPY_HISTORY_DIR+x}" ]; then
  tmux new-session -d -s "$SESSION" -x "$WIDTH" -y "$HEIGHT" env FPY_HISTORY_DIR="$FPY_HISTORY_DIR" zsh
elif [ -n "${XDG_DATA_HOME+x}" ]; then
  tmux new-session -d -s "$SESSION" -x "$WIDTH" -y "$HEIGHT" env XDG_DATA_HOME="$XDG_DATA_HOME" zsh
else
  tmux new-session -d -s "$SESSION" -x "$WIDTH" -y "$HEIGHT" zsh
fi
tmux send-keys -t "$SESSION" "cd $ROOT" Enter
tmux send-keys -t "$SESSION" "$FPY_CMD" Enter
wait_for_usable_input "$SESSION"

submit_lines "$SESSION" "$INPUTS"

capture_pane "$SESSION" > "$BEFORE_LOG"

case "$ACTION" in
  edit-left)
    wait_for_submit_ready "$SESSION"
    tmux send-keys -t "$SESSION" -l "abcd"
    tmux send-keys -t "$SESSION" Left Left
    tmux send-keys -t "$SESSION" -l "X"
    tmux send-keys -t "$SESSION" Enter
    ;;
  history-up)
    wait_for_submit_ready "$SESSION"
    tmux send-keys -t "$SESSION" Up Enter
    wait_for_submit_ready "$SESSION"
    ;;
  history-ctrl-k)
    wait_for_submit_ready "$SESSION"
    tmux send-keys -t "$SESSION" -H 1b -H 5b -H 31 -H 30 -H 37 -H 3b -H 35 -H 75
    tmux send-keys -t "$SESSION" Enter
    wait_for_submit_ready "$SESSION"
    ;;
  history-ctrl-k-ctrl-j)
    wait_for_submit_ready "$SESSION"
    tmux send-keys -t "$SESSION" -H 1b -H 5b -H 31 -H 30 -H 37 -H 3b -H 35 -H 75
    sleep 0.1
    tmux send-keys -t "$SESSION" -H 1b -H 5b -H 31 -H 30 -H 36 -H 3b -H 35 -H 75
    sleep 0.1
    tmux send-keys -t "$SESSION" -l "3+3"
    tmux send-keys -t "$SESSION" Enter
    wait_for_submit_ready "$SESSION"
    ;;
  vim-goto)
    wait_for_submit_ready "$SESSION"
    tmux send-keys -t "$SESSION" -l "a"
    tmux send-keys -t "$SESSION" -H 1b -H 5b -H 31 -H 33 -H 3b -H 32 -H 75
    wait_for_text "$SESSION" "2" "shift-enter-a"
    tmux send-keys -t "$SESSION" -l "b"
    tmux send-keys -t "$SESSION" -H 1b -H 5b -H 31 -H 33 -H 3b -H 32 -H 75
    wait_for_text "$SESSION" "3" "shift-enter-b"
    tmux send-keys -t "$SESSION" -l "c"
    sleep 0.1
    tmux send-keys -t "$SESSION" Escape
    sleep 0.1
    tmux send-keys -t "$SESSION" 3 G
    sleep 0.1
    tmux send-keys -t "$SESSION" i
    sleep 0.1
    tmux send-keys -t "$SESSION" -l "X"
    ;;
  vim-normal)
    wait_for_submit_ready "$SESSION"
    tmux send-keys -t "$SESSION" -l "abc"
    sleep 0.1
    tmux send-keys -t "$SESSION" Escape
    sleep 0.1
    tmux send-keys -t "$SESSION" 0
    sleep 0.1
    tmux send-keys -t "$SESSION" i
    sleep 0.1
    tmux send-keys -t "$SESSION" -l "X"
    tmux send-keys -t "$SESSION" Enter
    ;;
  shift-enter)
    wait_for_submit_ready "$SESSION"
    tmux send-keys -t "$SESSION" -l "abc"
    tmux send-keys -t "$SESSION" -H 1b -H 5b -H 31 -H 33 -H 3b -H 32 -H 75
    ;;
  ctrl-c-multiline)
    wait_for_submit_ready "$SESSION"
    tmux send-keys -t "$SESSION" -l "abc"
    tmux send-keys -t "$SESSION" -H 1b -H 5b -H 31 -H 33 -H 3b -H 32 -H 75
    wait_for_text "$SESSION" "2" "multiline-second-line"
    tmux send-keys -t "$SESSION" -l "def"
    sleep 0.1
    tmux send-keys -t "$SESSION" C-c
    ;;
  ctrl-c-multiline-bottom)
    submit_cell "$SESSION" "!ls -lah"
    submit_cell "$SESSION" "!ls -lah"
    submit_cell "$SESSION" "!ls -lah"
    submit_cell "$SESSION" "!ls -lah"
    tmux send-keys -t "$SESSION" -l "abc"
    tmux send-keys -t "$SESSION" -H 1b -H 5b -H 31 -H 33 -H 3b -H 32 -H 75
    wait_for_text "$SESSION" "2" "multiline-bottom-second-line"
    tmux send-keys -t "$SESSION" -l "def"
    sleep 0.1
    tmux send-keys -t "$SESSION" C-c
    ;;
  paste)
    buffer_name="$SESSION-paste"
    tmux set-buffer -b "$buffer_name" "$PASTE_TEXT"
    tmux paste-buffer -p -t "$SESSION" -b "$buffer_name"
    tmux delete-buffer -b "$buffer_name" >/dev/null 2>&1 || true
    ;;
  compose-while-busy)
    wait_for_submit_ready "$SESSION"
    tmux send-keys -t "$SESSION" "import time; time.sleep(3); 42" Enter
    wait_for_text "$SESSION" "Kernel busy. Ctrl-C to interrupt" "kernel-busy"
    tmux send-keys -t "$SESSION" -l "1+1"
    ;;
  stdin-reply)
    wait_for_submit_ready "$SESSION"
    tmux send-keys -t "$SESSION" "name = input('Name: '); print(name)" Enter
    wait_for_submit_ready "$SESSION"
    tmux send-keys -t "$SESSION" -l "bob"
    tmux send-keys -t "$SESSION" Enter
    wait_for_text "$SESSION" "bob" "stdin-reply-output"
    ;;
  stdin-empty-reply)
    wait_for_submit_ready "$SESSION"
    tmux send-keys -t "$SESSION" "repr(input())" Enter
    wait_for_submit_ready "$SESSION"
    tmux send-keys -t "$SESSION" Enter
    wait_for_text "$SESSION" "Out[" "stdin-empty-reply-output"
    ;;
  stdin-prompt)
    wait_for_submit_ready "$SESSION"
    tmux send-keys -t "$SESSION" "input()" Enter
    wait_for_submit_ready "$SESSION"
    ;;
  stdin-shift-enter)
    wait_for_submit_ready "$SESSION"
    tmux send-keys -t "$SESSION" "input()" Enter
    wait_for_submit_ready "$SESSION"
    tmux send-keys -t "$SESSION" -H 1b -H 5b -H 31 -H 33 -H 3b -H 32 -H 75
    sleep 0.1
    tmux send-keys -t "$SESSION" -H 1b -H 5b -H 31 -H 33 -H 3b -H 32 -H 75
    ;;
  stdin-ctrl-d)
    wait_for_submit_ready "$SESSION"
    tmux send-keys -t "$SESSION" "input()" Enter
    wait_for_submit_ready "$SESSION"
    tmux send-keys -t "$SESSION" C-d
    sleep 0.1
    tmux send-keys -t "$SESSION" -l "x"
    tmux send-keys -t "$SESSION" Enter
    wait_for_text "$SESSION" "Out[" "stdin-ctrl-d-output"
    ;;
  stdin-ctrl-c)
    wait_for_submit_ready "$SESSION"
    tmux send-keys -t "$SESSION" "input()" Enter
    wait_for_submit_ready "$SESSION"
    tmux send-keys -t "$SESSION" C-c
    wait_for_text "$SESSION" "KeyboardInterrupt" "stdin-ctrl-c-output"
    ;;
  pdb-basic)
    wait_for_submit_ready "$SESSION"
    tmux send-keys -t "$SESSION" 'import pdb; pdb.set_trace(); print("after")' Enter
    wait_for_text "$SESSION" "(Pdb)" "pdb-prompt-initial" || wait_for_text "$SESSION" "ipdb>" "ipdb-prompt-initial"
    tmux send-keys -t "$SESSION" -l 'where'
    tmux send-keys -t "$SESSION" Enter
    wait_for_text "$SESSION" "where" "pdb-where"
    tmux send-keys -t "$SESSION" -l 'p 1+1'
    tmux send-keys -t "$SESSION" Enter
    wait_for_text "$SESSION" "2" "pdb-print"
    tmux send-keys -t "$SESSION" -l 'c'
    tmux send-keys -t "$SESSION" Enter
    wait_for_text "$SESSION" "after" "pdb-continue"
    ;;
  vim-open-below)
    wait_for_submit_ready "$SESSION"
    tmux send-keys -t "$SESSION" Escape
    sleep 0.1
    tmux send-keys -t "$SESSION" o
    ;;
  vim-open-below-twice)
    wait_for_submit_ready "$SESSION"
    tmux send-keys -t "$SESSION" Escape
    sleep 0.1
    tmux send-keys -t "$SESSION" o
    sleep 0.1
    tmux send-keys -t "$SESSION" Escape
    sleep 0.1
    tmux send-keys -t "$SESSION" o
    ;;
  ctrl-l)
    wait_for_submit_ready "$SESSION"
    tmux send-keys -t "$SESSION" C-l
    ;;
  palette)
    wait_for_usable_input "$SESSION"
    tmux send-keys -t "$SESSION" C-p
    ;;
  palette-cycle)
    wait_for_usable_input "$SESSION"
    tmux send-keys -t "$SESSION" C-p
    sleep 0.1
    tmux send-keys -t "$SESSION" Escape
    sleep 0.1
    tmux send-keys -t "$SESSION" C-p
    ;;
  palette-move-cycle)
    wait_for_usable_input "$SESSION"
    tmux send-keys -t "$SESSION" C-p
    sleep 0.1
    tmux send-keys -t "$SESSION" Down
    sleep 0.1
    tmux send-keys -t "$SESSION" Escape
    sleep 0.1
    tmux send-keys -t "$SESSION" C-p
    ;;
  history-search-open)
    wait_for_usable_input "$SESSION"
    tmux send-keys -t "$SESSION" C-r
    sleep 0.1
    if [ -n "$SEARCH_QUERY" ]; then
      tmux send-keys -t "$SESSION" -l "$SEARCH_QUERY"
      wait_for_text "$SESSION" "$SEARCH_QUERY" "history-search-query"
    fi
    if [ "$SEARCH_DOWN_COUNT" -gt 0 ]; then
      i=0
      while [ "$i" -lt "$SEARCH_DOWN_COUNT" ]; do
        tmux send-keys -t "$SESSION" Down
        i=$((i + 1))
        sleep 0.05
      done
    fi
    ;;
  history-search-load)
    wait_for_usable_input "$SESSION"
    tmux send-keys -t "$SESSION" C-r
    sleep 0.1
    if [ -n "$SEARCH_QUERY" ]; then
      tmux send-keys -t "$SESSION" -l "$SEARCH_QUERY"
      wait_for_text "$SESSION" "$SEARCH_QUERY" "history-search-query"
    fi
    tmux send-keys -t "$SESSION" Enter
    ;;
  history-search-load-ctrl-k)
    wait_for_usable_input "$SESSION"
    tmux send-keys -t "$SESSION" C-r
    sleep 0.1
    if [ -n "$SEARCH_QUERY" ]; then
      tmux send-keys -t "$SESSION" -l "$SEARCH_QUERY"
      wait_for_text "$SESSION" "$SEARCH_QUERY" "history-search-query"
    fi
    tmux send-keys -t "$SESSION" Enter
    sleep 0.1
    tmux send-keys -t "$SESSION" -H 1b -H 5b -H 31 -H 30 -H 37 -H 3b -H 35 -H 75
    tmux send-keys -t "$SESSION" Enter
    wait_for_submit_ready "$SESSION"
    ;;
  history-search-load-ctrl-k-ctrl-j)
    wait_for_usable_input "$SESSION"
    tmux send-keys -t "$SESSION" C-r
    sleep 0.1
    if [ -n "$SEARCH_QUERY" ]; then
      tmux send-keys -t "$SESSION" -l "$SEARCH_QUERY"
      wait_for_text "$SESSION" "$SEARCH_QUERY" "history-search-query"
    fi
    tmux send-keys -t "$SESSION" Enter
    sleep 0.1
    tmux send-keys -t "$SESSION" -H 1b -H 5b -H 31 -H 30 -H 37 -H 3b -H 35 -H 75
    sleep 0.1
    tmux send-keys -t "$SESSION" -H 1b -H 5b -H 31 -H 30 -H 36 -H 3b -H 35 -H 75
    tmux send-keys -t "$SESSION" Enter
    wait_for_submit_ready "$SESSION"
    ;;
  ctrl-d)
    wait_for_submit_ready "$SESSION"
    tmux send-keys -t "$SESSION" C-d
    ;;
  exitpy)
    wait_for_submit_ready "$SESSION"
    tmux send-keys -t "$SESSION" "exit()" Enter
    wait_for_text "$SESSION" "Kernel exited unexpectedly" "kernel-exit"
    ;;
  quit)
    wait_for_submit_ready "$SESSION"
    tmux send-keys -t "$SESSION" "quit()" Enter
    wait_for_text "$SESSION" "Kernel exited unexpectedly" "kernel-quit"
    ;;
  none)
    wait_for_submit_ready "$SESSION"
    ;;
  *)
    printf 'unknown action: %s\n' "$ACTION" >&2
    exit 2
    ;;
esac

sleep "$EXIT_WAIT"

capture_pane "$SESSION" > "$AFTER_LOG"

printf 'session: %s\n' "$SESSION"
printf 'before: %s\n' "$BEFORE_LOG"
printf 'after: %s\n' "$AFTER_LOG"
printf '\n== after ==\n'
cat "$AFTER_LOG"
