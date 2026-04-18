#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
ACTION="${1:-ctrl-d}"
SESSION="${SESSION:-fpy-repro-$$}"
TMUX_SIZE="${TMUX_SIZE:-120x40}"
WIDTH="${TMUX_SIZE%x*}"
HEIGHT="${TMUX_SIZE#*x}"
PYTHON_BIN="${PYTHON_BIN:-.venv/bin/python}"
FPY_CMD="${FPY_CMD:-cargo run -- run --python $PYTHON_BIN}"
PRE_INPUT="${PRE_INPUT-1+1}"
INPUTS="${INPUTS-$PRE_INPUT}"
STARTUP_WAIT="${STARTUP_WAIT:-2}"
PRE_INPUT_WAIT="${PRE_INPUT_WAIT:-1}"
EXIT_WAIT="${EXIT_WAIT:-1}"
CAPTURE_LINES="${CAPTURE_LINES:-40}"
BEFORE_LOG="${BEFORE_LOG:-$ROOT/target/fpy-tmux-repro.before.log}"
AFTER_LOG="${AFTER_LOG:-$ROOT/target/fpy-tmux-repro.after.log}"
PASTE_TEXT="${PASTE_TEXT:-x = 1
y = 2}"

mkdir -p "$ROOT/target"

cleanup() {
  tmux kill-session -t "$SESSION" >/dev/null 2>&1 || true
}
trap cleanup EXIT INT TERM

tmux new-session -d -s "$SESSION" -x "$WIDTH" -y "$HEIGHT" zsh
tmux send-keys -t "$SESSION" "cd $ROOT" Enter
tmux send-keys -t "$SESSION" "$FPY_CMD" Enter
sleep "$STARTUP_WAIT"

if [ -n "$INPUTS" ]; then
  old_ifs=$IFS
  IFS='
'
  for input in $INPUTS; do
    tmux send-keys -t "$SESSION" "$input" Enter
    sleep "$PRE_INPUT_WAIT"
  done
  IFS=$old_ifs
fi

tmux capture-pane -pt "$SESSION" -S "-$CAPTURE_LINES" > "$BEFORE_LOG"

case "$ACTION" in
  edit-left)
    tmux send-keys -t "$SESSION" -l "abcd"
    tmux send-keys -t "$SESSION" Left Left
    tmux send-keys -t "$SESSION" -l "X"
    tmux send-keys -t "$SESSION" Enter
    ;;
  history-up)
    tmux send-keys -t "$SESSION" Up Enter
    ;;
  vim-goto)
    tmux send-keys -t "$SESSION" -l "a"
    tmux send-keys -t "$SESSION" -H 1b -H 5b -H 31 -H 33 -H 3b -H 32 -H 75
    sleep 0.2
    tmux send-keys -t "$SESSION" -l "b"
    tmux send-keys -t "$SESSION" -H 1b -H 5b -H 31 -H 33 -H 3b -H 32 -H 75
    sleep 0.2
    tmux send-keys -t "$SESSION" -l "c"
    sleep 0.2
    tmux send-keys -t "$SESSION" Escape
    sleep 0.2
    tmux send-keys -t "$SESSION" 3 G
    sleep 0.2
    tmux send-keys -t "$SESSION" i
    sleep 0.2
    tmux send-keys -t "$SESSION" -l "X"
    ;;
  vim-normal)
    tmux send-keys -t "$SESSION" -l "abc"
    sleep 0.2
    tmux send-keys -t "$SESSION" Escape
    sleep 0.2
    tmux send-keys -t "$SESSION" 0
    sleep 0.2
    tmux send-keys -t "$SESSION" i
    sleep 0.2
    tmux send-keys -t "$SESSION" -l "X"
    tmux send-keys -t "$SESSION" Enter
    ;;
  shift-enter)
    tmux send-keys -t "$SESSION" -l "abc"
    tmux send-keys -t "$SESSION" -H 1b -H 5b -H 31 -H 33 -H 3b -H 32 -H 75
    ;;
  ctrl-c-multiline)
    tmux send-keys -t "$SESSION" -l "abc"
    tmux send-keys -t "$SESSION" -H 1b -H 5b -H 31 -H 33 -H 3b -H 32 -H 75
    sleep 0.2
    tmux send-keys -t "$SESSION" -l "def"
    sleep 0.2
    tmux send-keys -t "$SESSION" C-c
    ;;
  ctrl-c-multiline-bottom)
    tmux send-keys -t "$SESSION" -l "!ls -lah"
    tmux send-keys -t "$SESSION" Enter
    sleep 0.5
    tmux send-keys -t "$SESSION" -l "!ls -lah"
    tmux send-keys -t "$SESSION" Enter
    sleep 0.5
    tmux send-keys -t "$SESSION" -l "!ls -lah"
    tmux send-keys -t "$SESSION" Enter
    sleep 0.5
    tmux send-keys -t "$SESSION" -l "!ls -lah"
    tmux send-keys -t "$SESSION" Enter
    sleep 0.5
    tmux send-keys -t "$SESSION" -l "abc"
    tmux send-keys -t "$SESSION" -H 1b -H 5b -H 31 -H 33 -H 3b -H 32 -H 75
    sleep 0.2
    tmux send-keys -t "$SESSION" -l "def"
    sleep 0.2
    tmux send-keys -t "$SESSION" C-c
    ;;
  paste)
    tmux set-buffer -b fpy-repro "$PASTE_TEXT"
    tmux paste-buffer -p -t "$SESSION" -b fpy-repro
    ;;
  compose-while-busy)
    tmux send-keys -t "$SESSION" "import time; time.sleep(3); 42" Enter
    sleep 0.5
    tmux send-keys -t "$SESSION" -l "1+1"
    ;;
  stdin-reply)
    tmux send-keys -t "$SESSION" "name = input('Name: '); print(name)" Enter
    sleep 1
    tmux send-keys -t "$SESSION" -l "bob"
    tmux send-keys -t "$SESSION" Enter
    ;;
  stdin-prompt)
    tmux send-keys -t "$SESSION" "input()" Enter
    sleep 1
    ;;
  stdin-shift-enter)
    tmux send-keys -t "$SESSION" "input()" Enter
    sleep 1
    tmux send-keys -t "$SESSION" -H 1b -H 5b -H 31 -H 33 -H 3b -H 32 -H 75
    sleep 0.2
    tmux send-keys -t "$SESSION" -H 1b -H 5b -H 31 -H 33 -H 3b -H 32 -H 75
    ;;
  vim-open-below)
    tmux send-keys -t "$SESSION" Escape
    sleep 0.2
    tmux send-keys -t "$SESSION" o
    ;;
  vim-open-below-twice)
    tmux send-keys -t "$SESSION" Escape
    sleep 0.2
    tmux send-keys -t "$SESSION" o
    sleep 0.2
    tmux send-keys -t "$SESSION" Escape
    sleep 0.2
    tmux send-keys -t "$SESSION" o
    ;;
  ctrl-l)
    tmux send-keys -t "$SESSION" C-l
    ;;
  palette)
    tmux send-keys -t "$SESSION" C-p
    ;;
  palette-cycle)
    tmux send-keys -t "$SESSION" C-p
    sleep 0.2
    tmux send-keys -t "$SESSION" Escape
    sleep 0.2
    tmux send-keys -t "$SESSION" C-p
    ;;
  palette-move-cycle)
    tmux send-keys -t "$SESSION" C-p
    sleep 0.2
    tmux send-keys -t "$SESSION" Down
    sleep 0.2
    tmux send-keys -t "$SESSION" Escape
    sleep 0.2
    tmux send-keys -t "$SESSION" C-p
    ;;
  ctrl-d)
    tmux send-keys -t "$SESSION" C-d
    ;;
  exitpy)
    tmux send-keys -t "$SESSION" "exit()" Enter
    ;;
  quit)
    tmux send-keys -t "$SESSION" "quit()" Enter
    ;;
  none)
    ;;
  *)
    printf 'unknown action: %s\n' "$ACTION" >&2
    exit 2
    ;;
esac

sleep "$EXIT_WAIT"

tmux capture-pane -pt "$SESSION" -S "-$CAPTURE_LINES" > "$AFTER_LOG"

printf 'session: %s\n' "$SESSION"
printf 'before: %s\n' "$BEFORE_LOG"
printf 'after: %s\n' "$AFTER_LOG"
printf '\n== after ==\n'
cat "$AFTER_LOG"
