#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
TARGET_DIR="$ROOT/target"
SESSION_PREFIX="${SESSION_PREFIX:-fpy-startup-bench}"
TMUX_SIZE="${TMUX_SIZE:-120x40}"
WIDTH="${TMUX_SIZE%x*}"
HEIGHT="${TMUX_SIZE#*x}"
PYTHON_BIN="${PYTHON_BIN:-$ROOT/.venv/bin/python}"
FPY_BIN="${FPY_BIN:-$ROOT/target/release/fpy}"
FPY_CMD="${FPY_CMD:-$FPY_BIN run --python $PYTHON_BIN}"
IPYTHON_CMD="${IPYTHON_CMD:-$ROOT/.venv/bin/ipython}"
SAMPLES="${SAMPLES:-5}"
TIMEOUT_SECONDS="${TIMEOUT_SECONDS:-15}"
POLL_SECONDS="${POLL_SECONDS:-0.02}"
CAPTURE_LINES="${CAPTURE_LINES:-80}"

mkdir -p "$TARGET_DIR"

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    printf 'missing required command: %s\n' "$1" >&2
    exit 1
  fi
}

require_path() {
  if [ ! -e "$1" ]; then
    printf 'missing required path: %s\n' "$1" >&2
    exit 1
  fi
}

now_ns() {
  perl -MTime::HiRes=time -e 'printf "%.0f\n", time() * 1000000000'
}

session_name() {
  printf '%s-%s-%s' "$SESSION_PREFIX" "$1" "$2"
}

cleanup_session() {
  tmux kill-session -t "$1" >/dev/null 2>&1 || true
}

capture_pane() {
  tmux capture-pane -pt "$1" -S "-$CAPTURE_LINES"
}

pane_is_ready() {
  bench_name=$1
  pane_text=$2
  case "$bench_name" in
    fpy)
      printf '%s' "$pane_text" | grep -F "Ctrl-P palette" >/dev/null 2>&1 || return 1
      printf '%s' "$pane_text" | grep -F "Connecting to kernel..." >/dev/null 2>&1 && return 1
      printf '%s' "$pane_text" | grep -F "Kernel busy. Ctrl-C to interrupt" >/dev/null 2>&1 && return 1
      return 0
      ;;
    ipython)
      printf '%s' "$pane_text" | grep -E '(^|[[:space:]])In \[[0-9]+\]:' >/dev/null 2>&1
      return $?
      ;;
    *)
      printf 'unknown benchmark target: %s\n' "$bench_name" >&2
      exit 2
      ;;
  esac
}

pane_has_first_result() {
  bench_name=$1
  pane_text=$2
  case "$bench_name" in
    fpy|ipython)
      printf '%s' "$pane_text" | grep -F "Out[1]: 2" >/dev/null 2>&1
      return $?
      ;;
    *)
      printf 'unknown benchmark target: %s\n' "$bench_name" >&2
      exit 2
      ;;
  esac
}

run_sample() {
  bench_name=$1
  command_text=$2
  sample_index=$3
  session=$(session_name "$bench_name" "$sample_index")
  log_prefix="$TARGET_DIR/$session"
  timeout_deadline=$(($(now_ns) + TIMEOUT_SECONDS * 1000000000))

  cleanup_session "$session"
  tmux new-session -d -s "$session" -x "$WIDTH" -y "$HEIGHT" zsh
  tmux send-keys -t "$session" "cd $ROOT" Enter
  start_ns=$(now_ns)
  tmux send-keys -t "$session" "$command_text" Enter

  ready_ns=
  while :; do
    pane_text=$(capture_pane "$session")
    if pane_is_ready "$bench_name" "$pane_text"; then
      ready_ns=$(now_ns)
      break
    fi
    if [ "$(now_ns)" -ge "$timeout_deadline" ]; then
      printf '%s\n' "$pane_text" > "$log_prefix-timeout-ready.log"
      cleanup_session "$session"
      printf 'timed out waiting for %s prompt readiness; pane saved to %s-timeout-ready.log\n' \
        "$bench_name" "$log_prefix" >&2
      exit 1
    fi
    sleep "$POLL_SECONDS"
  done

  tmux send-keys -t "$session" "1+1" Enter

  result_ns=
  while :; do
    pane_text=$(capture_pane "$session")
    if pane_has_first_result "$bench_name" "$pane_text"; then
      result_ns=$(now_ns)
      break
    fi
    if [ "$(now_ns)" -ge "$timeout_deadline" ]; then
      printf '%s\n' "$pane_text" > "$log_prefix-timeout-result.log"
      cleanup_session "$session"
      printf 'timed out waiting for %s first result; pane saved to %s-timeout-result.log\n' \
        "$bench_name" "$log_prefix" >&2
      exit 1
    fi
    sleep "$POLL_SECONDS"
  done

  cleanup_session "$session"

  ready_ms=$(( (ready_ns - start_ns) / 1000000 ))
  first_result_ms=$(( (result_ns - start_ns) / 1000000 ))
  printf '%s %s %s\n' "$sample_index" "$ready_ms" "$first_result_ms"
}

summarize_samples() {
  bench_name=$1
  sample_file=$2
  awk -v name="$bench_name" '
    BEGIN {
      ready_sum = 0;
      result_sum = 0;
      ready_min = -1;
      ready_max = 0;
      result_min = -1;
      result_max = 0;
      count = 0;
    }
    {
      count += 1;
      ready = $2;
      result = $3;
      ready_sum += ready;
      result_sum += result;
      if (ready_min == -1 || ready < ready_min) ready_min = ready;
      if (ready > ready_max) ready_max = ready;
      if (result_min == -1 || result < result_min) result_min = result;
      if (result > result_max) result_max = result;
    }
    END {
      printf "%s %d %d %d %d %d %d %d\n",
        name,
        count,
        ready_sum / count,
        ready_min,
        ready_max,
        result_sum / count,
        result_min,
        result_max;
    }
  ' "$sample_file"
}

print_summary() {
  fpy_summary=$1
  ipython_summary=$2

  set -- $fpy_summary
  fpy_ready_avg=$3
  fpy_ready_min=$4
  fpy_ready_max=$5
  fpy_result_avg=$6
  fpy_result_min=$7
  fpy_result_max=$8

  set -- $ipython_summary
  ipython_ready_avg=$3
  ipython_ready_min=$4
  ipython_ready_max=$5
  ipython_result_avg=$6
  ipython_result_min=$7
  ipython_result_max=$8

  ready_ratio=$(awk -v a="$fpy_ready_avg" -v b="$ipython_ready_avg" 'BEGIN { printf "%.2f", a / b }')
  result_ratio=$(awk -v a="$fpy_result_avg" -v b="$ipython_result_avg" 'BEGIN { printf "%.2f", a / b }')

  printf '\nSummary\n'
  printf 'target   ready_avg_ms ready_min_ms ready_max_ms first_result_avg_ms first_result_min_ms first_result_max_ms\n'
  printf 'fpy      %12s %12s %12s %19s %19s %19s\n' \
    "$fpy_ready_avg" "$fpy_ready_min" "$fpy_ready_max" \
    "$fpy_result_avg" "$fpy_result_min" "$fpy_result_max"
  printf 'ipython  %12s %12s %12s %19s %19s %19s\n' \
    "$ipython_ready_avg" "$ipython_ready_min" "$ipython_ready_max" \
    "$ipython_result_avg" "$ipython_result_min" "$ipython_result_max"
  printf '\nRelative slowdown\n'
  printf 'fpy ready_avg vs ipython: %sx\n' "$ready_ratio"
  printf 'fpy first_result_avg vs ipython: %sx\n' "$result_ratio"
}

benchmark_target() {
  bench_name=$1
  command_text=$2
  sample_file="$TARGET_DIR/${bench_name}-startup-benchmark.txt"

  : > "$sample_file"
  printf 'Benchmarking %s\n' "$bench_name" >&2
  i=1
  while [ "$i" -le "$SAMPLES" ]; do
    sample=$(run_sample "$bench_name" "$command_text" "$i")
    printf '%s\n' "$sample" >> "$sample_file"
    set -- $sample
    printf '  sample %s: ready=%sms first_result=%sms\n' "$1" "$2" "$3" >&2
    i=$((i + 1))
  done
  summarize_samples "$bench_name" "$sample_file"
}

require_command tmux
require_command perl
require_path "$PYTHON_BIN"

if [ ! -x "$IPYTHON_CMD" ]; then
  IPYTHON_CMD="$PYTHON_BIN -m IPython"
fi

printf 'Building fpy release binary (excluded from timings)...\n'
(cd "$ROOT" && cargo build --release >/dev/null)
require_path "$FPY_BIN"

fpy_summary=$(benchmark_target fpy "$FPY_CMD")
ipython_summary=$(benchmark_target ipython "$IPYTHON_CMD")
print_summary "$fpy_summary" "$ipython_summary"
