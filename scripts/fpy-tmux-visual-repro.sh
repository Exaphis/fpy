#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
SCENARIO="${1:-startup-anchor}"
SESSION="${SESSION:-fpy-visual-$SCENARIO-$$}"
BASE="$ROOT/target/fpy-visual.$SCENARIO"

mkdir -p "$ROOT/target"

run_repro() {
  SESSION="$SESSION" \
  BEFORE_LOG="$BASE.before.log" \
  AFTER_LOG="$BASE.after.log" \
  AFTER_ANSI_LOG="$BASE.after.ansi.log" \
  AFTER_META_LOG="$BASE.after.meta" \
  "$ROOT/scripts/fpy-tmux-repro.sh" "$@"
}

require_meta_gt() {
  left_key=$1
  right_key=$2
  left=$(awk -F= -v key="$left_key" '$1 == key { print $2 }' "$BASE.after.meta")
  right=$(awk -F= -v key="$right_key" '$1 == key { print $2 }' "$BASE.after.meta")
  if [ -z "$left" ] || [ -z "$right" ] || [ "$left" -le "$right" ]; then
    printf 'FAIL: expected %s (%s) > %s (%s)\n' "$left_key" "${left:-missing}" "$right_key" "${right:-missing}" >&2
    printf 'plain capture: %s\n' "$BASE.after.log" >&2
    printf 'meta: %s\n' "$BASE.after.meta" >&2
    exit 1
  fi
}

require_ansi() {
  pattern=$1
  if ! perl -0ne "\$ok = 1 if /$pattern/; END { exit(\$ok ? 0 : 1) }" "$BASE.after.ansi.log"; then
    printf 'FAIL: expected ANSI pattern %s in %s\n' "$pattern" "$BASE.after.ansi.log" >&2
    exit 1
  fi
}

case "$SCENARIO" in
  startup-anchor)
    VISUAL_SENTINEL="__FPY_VISUAL_SENTINEL__" \
    PRE_INPUT="" \
    INPUTS="" \
    CAPTURE_VISIBLE_ONLY=1 \
    EXIT_WAIT=1 \
    run_repro none
    require_meta_gt fpy_row sentinel_row
    ;;
  footer-styling)
    PRE_INPUT="" \
    INPUTS="" \
    EXIT_WAIT=1 \
    run_repro none
    require_ansi '\e\[30m\e\[46m INS '
    require_ansi '\e\[90mCtrl-P palette'
    ;;
  bottom-prompt)
    TMUX_SIZE="${TMUX_SIZE:-120x20}" \
    PRE_INPUT="print('\n'.join(f'line {i}' for i in range(30)))" \
    INPUTS="print('\n'.join(f'line {i}' for i in range(30)))" \
    CAPTURE_LINES=80 \
    EXIT_WAIT=1 \
    run_repro none
    ;;
  *)
    printf 'usage: %s [startup-anchor|footer-styling|bottom-prompt]\n' "$0" >&2
    exit 2
    ;;
esac

printf '\nvisual scenario: %s\n' "$SCENARIO"
printf 'plain: %s\n' "$BASE.after.log"
printf 'ansi:  %s\n' "$BASE.after.ansi.log"
printf 'meta:  %s\n' "$BASE.after.meta"
